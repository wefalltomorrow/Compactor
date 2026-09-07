use std::collections::VecDeque;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf, Prefix};
use std::thread;
use std::time::{Duration, Instant};

use compresstimator::Compresstimator;
use crossbeam_channel::{unbounded, RecvTimeoutError};
use filesize::PathExt;
use globset::GlobSet;
use serde_derive::Serialize;
use walkdir::WalkDir;
use winapi::shared::minwindef::DWORD;
use winapi::shared::ntdef::PVOID;
use winapi::um::ioapiset::DeviceIoControl;
use winapi::um::winioctl::{
    PropertyStandardQuery, StorageDeviceSeekPenaltyProperty, IOCTL_STORAGE_QUERY_PROPERTY,
    STORAGE_PROPERTY_QUERY,
};
use winapi::um::winnt::{
    BOOLEAN, FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_ENCRYPTED, FILE_ATTRIBUTE_OFFLINE,
    FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SPARSE_FILE,
    FILE_ATTRIBUTE_SYSTEM, FILE_ATTRIBUTE_TEMPORARY, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, HANDLE,
};

use crate::background::{Background, ControlToken};
use crate::persistence::{incompressible_key, pathdb};

const AUTO_MAX_THREADS: usize = 6;
const MAX_CONFIGURED_THREADS: usize = 16;
const HDD_SAMPLE_WINDOWS: u64 = 4;
const HDD_SAMPLE_CHUNK: usize = 128 * 1024;
const ESTIMATOR_BLOCK_SIZE: usize = 8192;

#[derive(Debug, Clone, Serialize)]
pub struct FileInfo {
    pub path: PathBuf,
    pub logical_size: u64,
    pub physical_size: u64,
    pub estimated_physical_size: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct GroupInfo {
    pub files: VecDeque<FileInfo>,
    pub logical_size: u64,
    pub physical_size: u64,
    pub estimated_physical_size: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FolderInfo {
    pub path: PathBuf,
    pub logical_size: u64,
    pub physical_size: u64,
    pub compressible: GroupInfo,
    pub compressed: GroupInfo,
    pub skipped: GroupInfo,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct FolderSummary {
    pub logical_size: u64,
    pub physical_size: u64,
    pub compressible: GroupSummary,
    pub compressed: GroupSummary,
    pub skipped: GroupSummary,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct GroupSummary {
    pub count: usize,
    pub logical_size: u64,
    pub physical_size: u64,
    pub estimated_physical_size: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum FileKind {
    Compressed,
    Compressible,
    Skipped,
}

impl FolderInfo {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_owned(),
            logical_size: 0,
            physical_size: 0,
            compressible: GroupInfo::default(),
            compressed: GroupInfo::default(),
            skipped: GroupInfo::default(),
        }
    }

    pub fn summary(&self) -> FolderSummary {
        FolderSummary {
            logical_size: self.logical_size,
            physical_size: self.physical_size,
            compressible: self.compressible.summary(),
            compressed: self.compressed.summary(),
            skipped: self.skipped.summary(),
        }
    }

    pub fn len(&self, kind: FileKind) -> usize {
        match kind {
            FileKind::Compressible => self.compressible.files.len(),
            FileKind::Compressed => self.compressed.files.len(),
            FileKind::Skipped => self.skipped.files.len(),
        }
    }

    pub fn pop(&mut self, kind: FileKind) -> Option<FileInfo> {
        let ret = match kind {
            FileKind::Compressible => self.compressible.pop(),
            FileKind::Compressed => self.compressed.pop(),
            FileKind::Skipped => self.skipped.pop(),
        };

        if let Some(fi) = ret {
            self.logical_size = self.logical_size.saturating_sub(fi.logical_size);
            self.physical_size = self.physical_size.saturating_sub(fi.physical_size);
            Some(fi)
        } else {
            None
        }
    }

    pub fn push(&mut self, kind: FileKind, fi: FileInfo) {
        self.logical_size = self.logical_size.saturating_add(fi.logical_size);
        self.physical_size = self.physical_size.saturating_add(fi.physical_size);

        match kind {
            FileKind::Compressible => self.compressible.push(fi),
            FileKind::Compressed => self.compressed.push(fi),
            FileKind::Skipped => self.skipped.push(fi),
        }
    }
}

impl GroupInfo {
    pub fn summary(&self) -> GroupSummary {
        GroupSummary {
            count: self.files.len(),
            logical_size: self.logical_size,
            physical_size: self.physical_size,
            estimated_physical_size: self.estimated_physical_size,
        }
    }

    fn pop(&mut self) -> Option<FileInfo> {
        let ret = self.files.pop_front();

        if let Some(fi) = ret {
            self.logical_size = self.logical_size.saturating_sub(fi.logical_size);
            self.physical_size = self.physical_size.saturating_sub(fi.physical_size);
            self.estimated_physical_size = self
                .estimated_physical_size
                .saturating_sub(fi.estimated_physical_size);
            Some(fi)
        } else {
            None
        }
    }

    fn push(&mut self, fi: FileInfo) {
        self.logical_size = self.logical_size.saturating_add(fi.logical_size);
        self.physical_size = self.physical_size.saturating_add(fi.physical_size);
        self.estimated_physical_size = self
            .estimated_physical_size
            .saturating_add(fi.estimated_physical_size);
        self.files.push_back(fi);
    }
}

#[derive(Debug)]
pub struct FolderScan {
    path: PathBuf,
    excludes: GlobSet,
    ratio_limit: f32,
    max_threads: usize,
    hdd_single_thread: bool,
}

impl FolderScan {
    pub fn new<P: AsRef<Path>>(
        path: P,
        excludes: GlobSet,
        ratio_limit: f32,
        max_threads: usize,
        hdd_single_thread: bool,
    ) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            excludes,
            ratio_limit,
            max_threads,
            hdd_single_thread,
        }
    }
}

#[derive(Debug)]
struct EstimateCandidate {
    file: FileInfo,
    content_len: u64,
}

#[repr(C)]
struct DeviceSeekPenaltyDescriptor {
    version: DWORD,
    size: DWORD,
    incurs_seek_penalty: BOOLEAN,
}

fn estimate_physical_size(logical_size: u64, ratio: f32) -> u64 {
    if !ratio.is_finite() {
        return logical_size;
    }

    ((logical_size as f64) * (ratio.clamp(0.0, 1.0) as f64)).round() as u64
}

fn volume_device_path(path: &Path) -> Option<PathBuf> {
    match path.components().next()? {
        Component::Prefix(prefix) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                Some(PathBuf::from(format!(r"\\.\{}:", letter as char)))
            }
            _ => None,
        },
        _ => None,
    }
}

fn volume_incurs_seek_penalty(path: &Path) -> Option<bool> {
    let volume_path = volume_device_path(path)?;
    let volume = std::fs::OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(volume_path)
        .ok()?;

    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceSeekPenaltyProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut descriptor = DeviceSeekPenaltyDescriptor {
        version: 0,
        size: 0,
        incurs_seek_penalty: 0,
    };
    let mut bytes_returned: DWORD = 0;

    let ret = unsafe {
        DeviceIoControl(
            volume.as_raw_handle() as HANDLE,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &query as *const _ as PVOID,
            std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as DWORD,
            &mut descriptor as *mut _ as PVOID,
            std::mem::size_of::<DeviceSeekPenaltyDescriptor>() as DWORD,
            &mut bytes_returned,
            std::ptr::null_mut(),
        )
    };

    if ret != 0 {
        Some(descriptor.incurs_seek_penalty != 0)
    } else {
        None
    }
}

fn configured_thread_count(cpus: usize, max_threads: usize) -> usize {
    if max_threads == 0 {
        cpus.max(1).min(AUTO_MAX_THREADS)
    } else {
        max_threads.clamp(1, MAX_CONFIGURED_THREADS)
    }
}

fn worker_count_for_storage(
    cpus: usize,
    incurs_seek_penalty: Option<bool>,
    max_threads: usize,
    hdd_single_thread: bool,
) -> usize {
    let configured = configured_thread_count(cpus, max_threads);

    match incurs_seek_penalty {
        Some(true) if hdd_single_thread => 1,
        Some(_) => configured,
        None => 1,
    }
}

pub fn worker_count_for_path(path: &Path, max_threads: usize, hdd_single_thread: bool) -> usize {
    let cpus = thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1);
    worker_count_for_storage(
        cpus,
        volume_incurs_seek_penalty(path),
        max_threads,
        hdd_single_thread,
    )
}

fn compresstimate_hdd(
    estimator: &Compresstimator,
    handle: &mut std::fs::File,
    len: u64,
    buffer: &mut Vec<u8>,
) -> io::Result<f32> {
    let chunk = HDD_SAMPLE_CHUNK.min(len as usize);

    if chunk == 0 {
        return Ok(1.0);
    }

    if len <= (HDD_SAMPLE_CHUNK as u64) * HDD_SAMPLE_WINDOWS {
        return estimator.compresstimate(handle, len);
    }

    buffer.resize(chunk, 0);
    let max_start = len.saturating_sub(chunk as u64);
    let mut weighted_ratio = 0.0f64;
    let mut total_sampled = 0u64;

    for sample in 0..HDD_SAMPLE_WINDOWS {
        let mut offset = max_start.saturating_mul(sample) / (HDD_SAMPLE_WINDOWS - 1);
        offset = (offset / ESTIMATOR_BLOCK_SIZE as u64) * ESTIMATOR_BLOCK_SIZE as u64;

        handle.seek(SeekFrom::Start(offset))?;
        handle.read_exact(&mut buffer[..chunk])?;

        let ratio = estimator.compresstimate(Cursor::new(&buffer[..chunk]), chunk as u64)?;
        weighted_ratio += ratio as f64 * chunk as f64;
        total_sampled = total_sampled.saturating_add(chunk as u64);
    }

    if total_sampled == 0 {
        Ok(1.0)
    } else {
        Ok((weighted_ratio / total_sampled as f64) as f32)
    }
}

fn estimate_file(
    estimator: &Compresstimator,
    path: &Path,
    len: u64,
    hdd: bool,
    buffer: &mut Vec<u8>,
) -> io::Result<f32> {
    let mut handle = std::fs::File::open(path)?;

    if hdd {
        compresstimate_hdd(estimator, &mut handle, len, buffer)
    } else {
        estimator.compresstimate(&mut handle, len)
    }
}

fn apply_estimate(
    ds: &mut FolderInfo,
    mut fi: FileInfo,
    result: io::Result<f32>,
    ratio_limit: f32,
) {
    match result {
        Ok(ratio) if ratio < ratio_limit => {
            fi.estimated_physical_size =
                estimate_physical_size(fi.logical_size, ratio).min(fi.physical_size);
            ds.push(FileKind::Compressible, fi);
        }
        Ok(_) | Err(_) => ds.push(FileKind::Skipped, fi),
    }
}

fn estimate_candidates(
    path: &Path,
    mut candidates: VecDeque<EstimateCandidate>,
    workers: usize,
    hdd: bool,
    ratio_limit: f32,
    control: &ControlToken<(PathBuf, FolderSummary)>,
    ds: &mut FolderInfo,
    last_status: &mut Instant,
) -> bool {
    if candidates.is_empty() {
        return true;
    }

    let workers = workers.min(candidates.len()).max(1);
    let (job_tx, job_rx) = unbounded::<EstimateCandidate>();
    let (result_tx, result_rx) = unbounded::<(FileInfo, io::Result<f32>)>();
    let mut handles = Vec::with_capacity(workers);

    for _ in 0..workers {
        let jobs = job_rx.clone();
        let results = result_tx.clone();
        let root = path.to_path_buf();

        handles.push(thread::spawn(move || {
            let estimator = Compresstimator::with_block_size(ESTIMATOR_BLOCK_SIZE);
            let mut buffer = Vec::new();

            while let Ok(candidate) = jobs.recv() {
                let file_path = root.join(&candidate.file.path);
                let result = estimate_file(
                    &estimator,
                    &file_path,
                    candidate.content_len,
                    hdd,
                    &mut buffer,
                );

                if results.send((candidate.file, result)).is_err() {
                    break;
                }
            }
        }));
    }

    drop(job_rx);
    drop(result_tx);

    let mut in_flight = 0usize;
    for _ in 0..workers {
        if let Some(candidate) = candidates.pop_front() {
            if job_tx.send(candidate).is_err() {
                break;
            }
            in_flight += 1;
        }
    }

    let mut cancelled = false;

    while in_flight > 0 {
        if control.is_cancelled_with_pause() {
            cancelled = true;
            break;
        }

        match result_rx.recv_timeout(Duration::from_millis(25)) {
            Ok((fi, result)) => {
                in_flight -= 1;
                let status_path = fi.path.clone();
                apply_estimate(ds, fi, result, ratio_limit);

                if last_status.elapsed() >= Duration::from_millis(50) {
                    *last_status = Instant::now();
                    control.set_status((status_path, ds.summary()));
                }

                if let Some(candidate) = candidates.pop_front() {
                    if job_tx.send(candidate).is_ok() {
                        in_flight += 1;
                    } else {
                        cancelled = true;
                        break;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                cancelled = true;
                break;
            }
        }
    }

    drop(job_tx);

    for handle in handles {
        let _ = handle.join();
    }

    if cancelled {
        for (fi, result) in result_rx.try_iter() {
            apply_estimate(ds, fi, result, ratio_limit);
        }

        for candidate in candidates {
            ds.push(FileKind::Skipped, candidate.file);
        }

        false
    } else {
        true
    }
}

impl Background for FolderScan {
    type Output = Result<FolderInfo, FolderInfo>;
    type Status = (PathBuf, FolderSummary);

    fn run(self, control: &ControlToken<Self::Status>) -> Self::Output {
        let FolderScan {
            path,
            excludes,
            ratio_limit,
            max_threads,
            hdd_single_thread,
        } = self;
        let mut ds = FolderInfo::new(&path);
        let cpus = thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1);
        let seek_penalty = volume_incurs_seek_penalty(&path);
        let analysis_workers =
            worker_count_for_storage(cpus, seek_penalty, max_threads, hdd_single_thread);
        let is_hdd = matches!(seek_penalty, Some(true));
        let mut candidates: VecDeque<EstimateCandidate> = VecDeque::new();
        let inline_estimator = if analysis_workers == 1 {
            Some(Compresstimator::with_block_size(ESTIMATOR_BLOCK_SIZE))
        } else {
            None
        };
        let mut inline_buffer = Vec::new();
        let incompressible = pathdb();
        let mut incompressible = incompressible.write().unwrap();
        let _ = incompressible.load();

        let mut last_status = Instant::now();

        let walker = WalkDir::new(&path)
            .into_iter()
            .filter_entry(|e| e.file_type().is_file() || !excludes.is_match(e.path()))
            .filter_map(|e| e.map_err(|e| eprintln!("Error: {:?}", e)).ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().map(|md| (e, md)).ok())
            .filter_map(|(e, md)| e.path().size_on_disk().map(|s| (e, md, s)).ok())
            .enumerate();

        for (count, (entry, metadata, physical)) in walker {
            let shortname = entry
                .path()
                .strip_prefix(&path)
                .unwrap_or_else(|_e| entry.path())
                .to_path_buf();

            let logical_size = metadata.len().max(physical);
            let fi = FileInfo {
                path: shortname,
                logical_size,
                physical_size: physical,
                estimated_physical_size: physical,
            };

            if count % 8 == 0 {
                if control.is_cancelled_with_pause() {
                    for candidate in candidates {
                        ds.push(FileKind::Skipped, candidate.file);
                    }
                    return Err(ds);
                }

                if last_status.elapsed() >= Duration::from_millis(50) {
                    last_status = Instant::now();
                    control.set_status((fi.path.clone(), ds.summary()));
                }
            }

            let attributes = metadata.file_attributes();
            let special_attributes = FILE_ATTRIBUTE_COMPRESSED
                | FILE_ATTRIBUTE_ENCRYPTED
                | FILE_ATTRIBUTE_SPARSE_FILE
                | FILE_ATTRIBUTE_REPARSE_POINT
                | FILE_ATTRIBUTE_OFFLINE;

            if attributes & special_attributes != 0 {
                ds.push(FileKind::Skipped, fi);
            } else if fi.physical_size < fi.logical_size {
                ds.push(FileKind::Compressed, fi);
            } else if fi.logical_size <= 4096
                || attributes
                    & (FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_SYSTEM | FILE_ATTRIBUTE_TEMPORARY)
                    != 0
                || incompressible.contains(incompressible_key(entry.path(), &metadata))
                || excludes.is_match(entry.path())
            {
                ds.push(FileKind::Skipped, fi);
            } else if let Some(estimator) = inline_estimator.as_ref() {
                let result = estimate_file(
                    estimator,
                    entry.path(),
                    metadata.len(),
                    is_hdd,
                    &mut inline_buffer,
                );
                apply_estimate(&mut ds, fi, result, ratio_limit);
            } else {
                candidates.push_back(EstimateCandidate {
                    file: fi,
                    content_len: metadata.len(),
                });
            }
        }

        drop(incompressible);

        if analysis_workers == 1
            || estimate_candidates(
                &path,
                candidates,
                analysis_workers,
                is_hdd,
                ratio_limit,
                control,
                &mut ds,
                &mut last_status,
            )
        {
            Ok(ds)
        } else {
            Err(ds)
        }
    }
}

#[test]
fn analysis_workers_are_storage_aware() {
    assert_eq!(1, worker_count_for_storage(16, Some(true), 0, true));
    assert_eq!(1, worker_count_for_storage(16, None, 0, true));
    assert_eq!(1, worker_count_for_storage(1, Some(false), 0, true));
    assert_eq!(6, worker_count_for_storage(16, Some(false), 0, true));
    assert_eq!(4, worker_count_for_storage(16, Some(false), 4, true));
    assert_eq!(6, worker_count_for_storage(16, Some(true), 0, false));
    assert_eq!(16, worker_count_for_storage(4, Some(false), 16, true));
}

#[test]
fn it_walks() {
    use crate::background::BackgroundHandle;
    use crate::config::Config;

    let config = Config::default();
    let gs = config.globset().unwrap();
    let scanner = FolderScan::new(
        "C:\\Games",
        gs,
        config.ratio_limit(),
        config.max_threads,
        config.hdd_single_thread,
    );

    let task = BackgroundHandle::spawn(scanner);

    let deadline = Instant::now() + Duration::from_millis(2000);

    loop {
        let ret = task.wait_timeout(Duration::from_millis(100));

        if ret.is_some() {
            println!("Scanned: {:?}", ret);
            break;
        } else {
            println!("Status: {:?}", task.status());
        }

        if Instant::now() > deadline {
            task.cancel();
        }
    }
}
