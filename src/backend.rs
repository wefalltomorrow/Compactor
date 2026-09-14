use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, RecvTimeoutError};
use filesize::PathExt;
use winapi::um::winbase::SetThreadExecutionState;
use winapi::um::winnt::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED};

use crate::background::BackgroundHandle;
use crate::compact::{self, Compression};
use crate::compression::{BackgroundCompactor, CompressionJob};
use crate::directstorage::{discover_direct_storage_roots, is_under};
use crate::folder::{
    compression_worker_count_for_path, validate_target_path, FileInfo, FileKind, FolderInfo,
    FolderScan,
};
use crate::gui::{CompressedViewItem, GuiRequest, GuiResponse, GuiWrapper};
use crate::persistence::{config, incompressible_key, pathdb};

const COMPRESSED_VIEW_PAGE_SIZE: usize = 100;
const AUTO_MAX_DECOMPRESSION_THREADS: usize = 8;
const WOF_PREFLIGHT_SAMPLE_FILES: usize = 8;

pub struct Backend<T> {
    gui: GuiWrapper<T>,
    msg: Receiver<GuiRequest>,
    info: Option<FolderInfo>,
}

struct SystemAwakeGuard;

impl SystemAwakeGuard {
    fn new() -> Self {
        // Keep the system awake during file transformations, but deliberately
        // do not request ES_DISPLAY_REQUIRED: the monitor may still turn off.
        unsafe {
            let _ = SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
        }
        Self
    }
}

impl Drop for SystemAwakeGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = SetThreadExecutionState(ES_CONTINUOUS);
        }
    }
}

fn format_size(size: u64, decimal: bool) -> String {
    use humansize::{file_size_opts as options, FileSize};

    size.file_size(if decimal {
        options::DECIMAL
    } else {
        options::BINARY
    })
    .expect("file size")
}

fn progress(done_bytes: u64, total_bytes: u64) -> f32 {
    if total_bytes == 0 {
        1.0
    } else {
        (done_bytes as f64 / total_bytes as f64).min(1.0) as f32
    }
}

fn decompression_worker_count_for_path(
    path: &Path,
    max_threads: usize,
    hdd_single_thread: bool,
) -> usize {
    let storage_workers = compression_worker_count_for_path(
        path,
        Compression::Xpress16k,
        max_threads,
        hdd_single_thread,
    );

    if max_threads == 0 {
        storage_workers.min(AUTO_MAX_DECOMPRESSION_THREADS).max(1)
    } else {
        storage_workers.max(1)
    }
}

fn protect_direct_storage_candidates(folder: &mut FolderInfo, roots: &[PathBuf]) -> usize {
    if roots.is_empty() {
        return 0;
    }

    let candidates = folder.len(FileKind::Compressible);
    let mut protected = 0usize;

    for _ in 0..candidates {
        let Some(fi) = folder.pop(FileKind::Compressible) else {
            break;
        };
        let absolute = folder.path.join(&fi.path);

        if roots.iter().any(|root| is_under(&absolute, root)) {
            protected += 1;
            folder.push(FileKind::Skipped, fi);
        } else {
            folder.push(FileKind::Compressible, fi);
        }
    }

    protected
}

fn add_folder_exclusion(path: &Path) -> Result<bool, String> {
    let display = path.to_string_lossy().to_string();
    let c = config();
    let mut c = c.write().unwrap();
    let mut current = c.current();

    if current
        .excludes
        .iter()
        .any(|existing| existing.trim().eq_ignore_ascii_case(&display))
    {
        return Ok(false);
    }

    current.excludes.push(display);
    current.validate()?;
    c.replace(current);
    c.save()
        .map_err(|err| format!("Unable to save exclusions: {}", err))?;
    Ok(true)
}

#[derive(Default)]
struct CompressedFolderTotals {
    count: usize,
    logical_size: u64,
    physical_size: u64,
}

fn path_matches_query(path: &Path, query: &str) -> bool {
    query.is_empty() || path.to_string_lossy().to_lowercase().contains(query)
}

fn compressed_file_items(folder: &FolderInfo, query: &str) -> Vec<CompressedViewItem> {
    let mut files: Vec<&FileInfo> = folder
        .compressed
        .files
        .iter()
        .filter(|fi| path_matches_query(&fi.path, query))
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));

    files
        .into_iter()
        .map(|fi| CompressedViewItem::File {
            path: fi.path.clone(),
            logical_size: fi.logical_size,
            physical_size: fi.physical_size,
        })
        .collect()
}

fn compressed_folder_items(folder: &FolderInfo, query: &str) -> Vec<CompressedViewItem> {
    let mut folders: BTreeMap<PathBuf, CompressedFolderTotals> = BTreeMap::new();

    for fi in &folder.compressed.files {
        let parent = fi
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let totals = folders.entry(parent).or_default();
        totals.count += 1;
        totals.logical_size = totals.logical_size.saturating_add(fi.logical_size);
        totals.physical_size = totals.physical_size.saturating_add(fi.physical_size);
    }

    folders
        .into_iter()
        .filter(|(path, _)| path_matches_query(path, query))
        .map(|(path, totals)| CompressedViewItem::Folder {
            path,
            count: totals.count,
            logical_size: totals.logical_size,
            physical_size: totals.physical_size,
        })
        .collect()
}

fn paginate_compressed_items(
    items: Vec<CompressedViewItem>,
    page: usize,
) -> (usize, usize, usize, Vec<CompressedViewItem>) {
    let total = items.len();
    let pages = if total == 0 {
        1
    } else {
        (total + COMPRESSED_VIEW_PAGE_SIZE - 1) / COMPRESSED_VIEW_PAGE_SIZE
    };
    let page = page.min(pages - 1);
    let start = page.saturating_mul(COMPRESSED_VIEW_PAGE_SIZE);
    let page_items = items
        .into_iter()
        .skip(start)
        .take(COMPRESSED_VIEW_PAGE_SIZE)
        .collect();

    (page, pages, total, page_items)
}

impl<T> Backend<T> {
    pub fn new(gui: GuiWrapper<T>, msg: Receiver<GuiRequest>) -> Self {
        Self {
            gui,
            msg,
            info: None,
        }
    }

    fn target_is_valid(&self, path: &Path) -> bool {
        match validate_target_path(path) {
            Ok(()) => true,
            Err(message) => {
                self.gui.error("Folder not supported", &message);
                false
            }
        }
    }

    fn wof_preflight(&self, folder: &FolderInfo, kind: FileKind) -> bool {
        match compact::system_supports_compression() {
            Ok(true) => {}
            Ok(false) => {
                self.gui.error(
                    "WOF compression unavailable",
                    "This version of Windows does not report WOF filesystem compression support.",
                );
                return false;
            }
            Err(err) => {
                self.gui.error(
                    "WOF compression unavailable",
                    format!("Unable to initialise Windows WOF support: {}", err),
                );
                return false;
            }
        }

        let files = match kind {
            FileKind::Compressible => &folder.compressible.files,
            FileKind::Compressed => &folder.compressed.files,
            FileKind::Skipped => return true,
        };

        // Locked files can make an individual probe fail. Try a handful and
        // only block the operation when Windows explicitly says WOF is not
        // attached/supported on an accessible file from this volume.
        for fi in files.iter().take(WOF_PREFLIGHT_SAMPLE_FILES) {
            let path = folder.path.join(&fi.path);
            match compact::file_supports_compression(&path) {
                Ok(true) => return true,
                Ok(false) => {
                    self.gui.error(
                        "WOF unavailable on this drive",
                        "Windows Overlay Filter (Wof.sys) is not available for this NTFS volume, so WOF compression cannot be used here.",
                    );
                    return false;
                }
                Err(_) => continue,
            }
        }

        // If every sample happened to be locked, let the normal per-file path
        // handle it rather than refusing the whole operation on an uncertain
        // preflight result.
        true
    }

    pub fn run(&mut self) {
        loop {
            match self.msg.recv() {
                Ok(GuiRequest::ChooseFolder) => {
                    let path = self.gui.choose_folder().recv().ok().flatten();

                    if let Some(path) = path {
                        if self.target_is_valid(&path) {
                            self.gui.folder(&path);
                            self.scan_loop(path);
                        }
                    }
                }
                Ok(GuiRequest::Analyse) if self.info.is_some() => {
                    let path = self.info.as_ref().unwrap().path.clone();
                    if self.target_is_valid(&path) {
                        self.info = None;
                        self.gui.folder(&path);
                        self.scan_loop(path);
                    }
                }
                Ok(GuiRequest::ViewCompressed { view, query, page }) if self.info.is_some() => {
                    self.view_compressed(view, query, page);
                }
                Ok(GuiRequest::Compress) if self.info.is_some() => {
                    let path = self.info.as_ref().unwrap().path.clone();
                    if self.target_is_valid(&path) {
                        self.compress_loop();
                    }
                }
                Ok(GuiRequest::Decompress) if self.info.is_some() => {
                    let path = self.info.as_ref().unwrap().path.clone();
                    if self.target_is_valid(&path) {
                        self.uncompress_loop(false);
                    }
                }
                Ok(GuiRequest::DecompressAndExclude) if self.info.is_some() => {
                    let path = self.info.as_ref().unwrap().path.clone();
                    if self.target_is_valid(&path) {
                        self.uncompress_loop(true);
                    }
                }
                Ok(msg) => {
                    eprintln!("Backend: Ignored message: {:?}", msg);
                }
                Err(_) => {
                    eprintln!("Backend: exit run loop");
                    break;
                }
            }
        }
    }

    fn scan_loop(&mut self, path: PathBuf) {
        let current = config().read().unwrap().current();
        let excludes = current.globset().expect("globs");
        let ratio_limit = current.ratio_limit();

        let scanner = FolderScan::new(
            path,
            excludes,
            ratio_limit,
            current.compression,
            current.max_threads,
            current.hdd_single_thread,
        );
        let task = BackgroundHandle::spawn(scanner);
        let start = Instant::now();

        self.gui.status("Analysing", None);
        loop {
            let msg = self.msg.recv_timeout(Duration::from_millis(25));

            match msg {
                Ok(GuiRequest::Pause) => {
                    task.pause();
                    self.gui.status("Paused", Some(0.5));
                    self.gui.paused();
                }
                Ok(GuiRequest::Resume) => {
                    task.resume();
                    self.gui.status("Analysing", None);
                    self.gui.resumed();
                }
                Ok(GuiRequest::Stop) | Err(RecvTimeoutError::Disconnected) => {
                    task.cancel();
                }
                Ok(msg) => {
                    eprintln!("Ignored message: {:?}", msg);
                }
                Err(RecvTimeoutError::Timeout) => (),
            }

            match task.wait_timeout(Duration::from_millis(25)) {
                Some(Ok(info)) => {
                    self.gui
                        .status(format!("Analysed in {:.2?}", start.elapsed()), Some(1.0));
                    self.gui.summary(info.summary());
                    self.gui.scanned();
                    self.info = Some(info);
                    break;
                }
                Some(Err(info)) => {
                    self.gui.status(
                        format!("Analysis stopped after {:.2?}", start.elapsed()),
                        Some(0.5),
                    );
                    self.gui.summary(info.summary());
                    self.gui.stopped();
                    self.info = Some(info);
                    break;
                }
                None => {
                    if let Some(status) = task.status() {
                        self.gui
                            .status(format!("Analysing: {}", status.0.display()), None);
                        self.gui.summary(status.1);
                    }
                }
            }
        }
    }

    fn view_compressed(&self, view: String, query: String, page: usize) {
        let Some(folder) = self.info.as_ref() else {
            return;
        };

        let query = query.trim().to_string();
        let query_match = query.to_lowercase();
        let view = if view.eq_ignore_ascii_case("folders") {
            "folders"
        } else {
            "files"
        };
        let items = if view == "folders" {
            compressed_folder_items(folder, &query_match)
        } else {
            compressed_file_items(folder, &query_match)
        };
        let (page, pages, total, items) = paginate_compressed_items(items, page);
        let summary = folder.compressed.summary();

        self.gui.send(&GuiResponse::CompressedView {
            root: folder.path.clone(),
            view: view.to_string(),
            query,
            page,
            pages,
            total,
            compressed_count: summary.count,
            logical_size: summary.logical_size,
            physical_size: summary.physical_size,
            items,
        });
    }

    fn compress_loop(&mut self) {
        let current = config().read().unwrap().current();
        let compression = Some(current.compression);
        let mut folder = self.info.take().expect("fileinfo");

        if current.protect_direct_storage && folder.direct_storage {
            self.gui.status("Checking DirectStorage protection", None);
            let roots = discover_direct_storage_roots(&folder.path);
            let protected = protect_direct_storage_candidates(&mut folder, &roots);
            if protected > 0 {
                self.gui.status(
                    format!(
                        "Protected {} files in {} DirectStorage game{}",
                        protected,
                        roots.len(),
                        if roots.len() == 1 { "" } else { "s" }
                    ),
                    Some(0.0),
                );
                self.gui.summary(folder.summary());
            }
        }

        if folder.len(FileKind::Compressible) == 0 {
            self.gui.status("Nothing to compact", Some(1.0));
            self.gui.summary(folder.summary());
            self.gui.scanned();
            self.info = Some(folder);
            return;
        }

        if !self.wof_preflight(&folder, FileKind::Compressible) {
            self.gui.summary(folder.summary());
            self.gui.scanned();
            self.info = Some(folder);
            return;
        }

        let _awake = SystemAwakeGuard::new();
        let worker_count = compression_worker_count_for_path(
            &folder.path,
            current.compression,
            current.max_threads,
            current.hdd_single_thread,
        );

        let (send_file, send_file_rx) = bounded::<CompressionJob>(worker_count);
        let (recv_result_tx, recv_result) = bounded::<(PathBuf, io::Result<bool>)>(worker_count);
        let mut tasks = Vec::with_capacity(worker_count);

        for _ in 0..worker_count {
            let compactor = BackgroundCompactor::new(
                compression,
                current.ratio_limit(),
                send_file_rx.clone(),
                recv_result_tx.clone(),
            );
            tasks.push(BackgroundHandle::spawn(compactor));
        }

        drop(send_file_rx);
        drop(recv_result_tx);

        let start = Instant::now();
        let summary = folder.summary();
        let total_files = folder.len(FileKind::Compressible);
        let total_bytes = summary.compressible.logical_size;
        let compressible_size = summary.compressible.physical_size;
        let mut done_files = 0usize;
        let mut done_bytes = 0u64;
        let mut pending: HashMap<PathBuf, FileInfo> = HashMap::with_capacity(worker_count);
        let mut no_more_files = false;

        let mut last_update = Instant::now();
        let mut last_write = Instant::now();
        let mut paused = false;
        let mut stopped = false;

        let old_size = folder.physical_size;

        let incompressible = pathdb();
        let mut incompressible = incompressible.write().unwrap();
        let _ = incompressible.load();

        self.gui.compacting();
        self.gui.status("Compacting", Some(0.0));

        loop {
            while !paused && !stopped && pending.len() < worker_count && !no_more_files {
                if let Some(fi) = folder.pop(FileKind::Compressible) {
                    let path = folder.path.join(&fi.path);
                    let job = CompressionJob {
                        path: path.clone(),
                        content_len: fi.content_len,
                        modified_time: fi.modified_time,
                        estimate_valid: fi.estimate_valid,
                        estimated_ratio: fi.estimated_ratio,
                    };

                    if send_file.send(job).is_err() {
                        folder.push(FileKind::Compressible, fi);
                        stopped = true;
                        break;
                    }

                    pending.insert(path, fi);
                } else {
                    no_more_files = true;
                }
            }

            if (no_more_files || stopped) && pending.is_empty() {
                break;
            }

            if last_write.elapsed() > Duration::from_secs(60) {
                let _ = incompressible.save();
                last_write = Instant::now();
            }

            loop {
                match self.msg.try_recv() {
                    Ok(GuiRequest::Pause) if !paused && !stopped => {
                        paused = true;
                        self.gui.paused();
                        self.gui.status(
                            if pending.is_empty() {
                                "Paused"
                            } else {
                                "Pausing after active files finish"
                            },
                            Some(progress(done_bytes, total_bytes)),
                        );
                    }
                    Ok(GuiRequest::Resume) if paused && !stopped => {
                        paused = false;
                        self.gui.resumed();
                        self.gui
                            .status("Compacting", Some(progress(done_bytes, total_bytes)));
                    }
                    Ok(GuiRequest::Stop) if !stopped => {
                        stopped = true;
                        self.gui.status(
                            if pending.is_empty() {
                                "Stopping"
                            } else {
                                "Stopping after active files finish"
                            },
                            Some(progress(done_bytes, total_bytes)),
                        );
                    }
                    Ok(_) => (),
                    Err(_) => break,
                }
            }

            match recv_result.recv_timeout(Duration::from_millis(25)) {
                Ok((path, result)) => {
                    let Some(mut fi) = pending.remove(&path) else {
                        continue;
                    };
                    let logical_size = fi.logical_size;
                    let display_path = fi.path.clone();
                    done_files += 1;
                    done_bytes = done_bytes.saturating_add(logical_size);

                    match result {
                        Ok(true) => {
                            fi.physical_size = path.size_on_disk().unwrap_or(fi.physical_size);
                            fi.estimated_physical_size = fi.physical_size;
                            fi.estimate_valid = false;
                            if let Ok(metadata) = std::fs::metadata(&path) {
                                use std::os::windows::fs::MetadataExt;
                                fi.content_len = metadata.len();
                                fi.logical_size = metadata.len();
                                fi.modified_time = metadata.last_write_time();
                            }

                            if fi.physical_size >= fi.logical_size {
                                if let Ok(metadata) = std::fs::metadata(&path) {
                                    incompressible.insert(incompressible_key(&path, &metadata));
                                }
                                folder.push(FileKind::Skipped, fi);
                            } else {
                                folder.push(FileKind::Compressed, fi);
                            }
                        }
                        Ok(false) => {
                            fi.estimated_physical_size = fi.physical_size;
                            fi.estimate_valid = false;
                            if let Ok(metadata) = std::fs::metadata(&path) {
                                incompressible.insert(incompressible_key(&path, &metadata));
                            }
                            folder.push(FileKind::Skipped, fi);
                        }
                        Err(err) => {
                            fi.estimated_physical_size = fi.physical_size;
                            self.gui.status(
                                format!("Error: {}, {}", err, display_path.display()),
                                Some(progress(done_bytes, total_bytes)),
                            );
                            folder.push(FileKind::Skipped, fi);
                        }
                    }

                    if last_update.elapsed() > Duration::from_millis(50) {
                        last_update = Instant::now();
                        self.gui.status(
                            if paused {
                                if pending.is_empty() {
                                    "Paused".to_string()
                                } else {
                                    "Pausing after active files finish".to_string()
                                }
                            } else if stopped {
                                if pending.is_empty() {
                                    "Stopping".to_string()
                                } else {
                                    "Stopping after active files finish".to_string()
                                }
                            } else {
                                format!("Compacting: {}", display_path.display())
                            },
                            Some(progress(done_bytes, total_bytes)),
                        );

                        if pending.is_empty() {
                            self.gui.summary(folder.summary());
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if paused
                        && pending.is_empty()
                        && last_update.elapsed() > Duration::from_millis(50)
                    {
                        last_update = Instant::now();
                        self.gui
                            .status("Paused", Some(progress(done_bytes, total_bytes)));
                        self.gui.summary(folder.summary());
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    stopped = true;
                    break;
                }
            }
        }

        drop(send_file);
        for task in tasks {
            task.wait();
        }
        let _ = incompressible.save();

        let new_size = folder.physical_size;
        let decimal = config().read().unwrap().current().decimal;
        let saved = old_size.saturating_sub(new_size);

        let msg = if stopped {
            format!(
                "Stopped after {} of {} files, saving {} in {:.2?}",
                done_files,
                total_files,
                format_size(saved, decimal),
                start.elapsed()
            )
        } else {
            format!(
                "Compacted {} in {} files, saving {} in {:.2?}",
                format_size(compressible_size, decimal),
                done_files,
                format_size(saved, decimal),
                start.elapsed()
            )
        };

        self.gui
            .status(msg, Some(progress(done_bytes, total_bytes)));
        self.gui.summary(folder.summary());
        if stopped {
            self.gui.stopped();
        } else {
            self.gui.scanned();
        }

        self.info = Some(folder);
    }

    fn uncompress_loop(&mut self, exclude_after: bool) {
        let current = config().read().unwrap().current();
        let mut folder = self.info.take().expect("fileinfo");

        if folder.len(FileKind::Compressed) == 0 {
            self.gui.status("Nothing to decompress", Some(1.0));
            self.gui.scanned();
            self.info = Some(folder);
            return;
        }

        if !self.wof_preflight(&folder, FileKind::Compressed) {
            self.gui.summary(folder.summary());
            self.gui.scanned();
            self.info = Some(folder);
            return;
        }

        let _awake = SystemAwakeGuard::new();
        let worker_count = decompression_worker_count_for_path(
            &folder.path,
            current.max_threads,
            current.hdd_single_thread,
        );
        let (send_file, send_file_rx) = bounded::<CompressionJob>(worker_count);
        let (recv_result_tx, recv_result) = bounded::<(PathBuf, io::Result<bool>)>(worker_count);
        let mut tasks = Vec::with_capacity(worker_count);

        for _ in 0..worker_count {
            let compactor = BackgroundCompactor::new(
                None,
                0.0,
                send_file_rx.clone(),
                recv_result_tx.clone(),
            );
            tasks.push(BackgroundHandle::spawn(compactor));
        }

        drop(send_file_rx);
        drop(recv_result_tx);

        let start = Instant::now();
        let summary = folder.summary();
        let total_files = folder.len(FileKind::Compressed);
        let total_bytes = summary.compressed.logical_size;
        let mut done_files = 0usize;
        let mut expanded_files = 0usize;
        let mut done_bytes = 0u64;
        let mut pending: HashMap<PathBuf, FileInfo> = HashMap::with_capacity(worker_count);
        let mut failed = Vec::new();
        let mut no_more_files = false;

        let mut last_update = Instant::now();
        let mut paused = false;
        let mut stopped = false;
        let old_size = folder.physical_size;

        self.gui.compacting();
        self.gui.status("Expanding", Some(0.0));

        loop {
            while !paused && !stopped && pending.len() < worker_count && !no_more_files {
                if let Some(fi) = folder.pop(FileKind::Compressed) {
                    let path = folder.path.join(&fi.path);
                    let job = CompressionJob {
                        path: path.clone(),
                        content_len: fi.content_len,
                        modified_time: fi.modified_time,
                        estimate_valid: fi.estimate_valid,
                        estimated_ratio: fi.estimated_ratio,
                    };

                    if send_file.send(job).is_err() {
                        folder.push(FileKind::Compressed, fi);
                        stopped = true;
                        break;
                    }

                    pending.insert(path, fi);
                } else {
                    no_more_files = true;
                }
            }

            if (no_more_files || stopped) && pending.is_empty() {
                break;
            }

            loop {
                match self.msg.try_recv() {
                    Ok(GuiRequest::Pause) if !paused && !stopped => {
                        paused = true;
                        self.gui.paused();
                        self.gui.status(
                            if pending.is_empty() {
                                "Paused"
                            } else {
                                "Pausing after active files finish"
                            },
                            Some(progress(done_bytes, total_bytes)),
                        );
                    }
                    Ok(GuiRequest::Resume) if paused && !stopped => {
                        paused = false;
                        self.gui.resumed();
                        self.gui
                            .status("Expanding", Some(progress(done_bytes, total_bytes)));
                    }
                    Ok(GuiRequest::Stop) if !stopped => {
                        stopped = true;
                        self.gui.status(
                            if pending.is_empty() {
                                "Stopping"
                            } else {
                                "Stopping after active files finish"
                            },
                            Some(progress(done_bytes, total_bytes)),
                        );
                    }
                    Ok(_) => (),
                    Err(_) => break,
                }
            }

            match recv_result.recv_timeout(Duration::from_millis(25)) {
                Ok((path, result)) => {
                    let Some(mut fi) = pending.remove(&path) else {
                        continue;
                    };
                    let logical_size = fi.logical_size;
                    let display_path = fi.path.clone();
                    done_files += 1;
                    done_bytes = done_bytes.saturating_add(logical_size);

                    match result {
                        Ok(_) => {
                            expanded_files += 1;
                            fi.physical_size = path.size_on_disk().unwrap_or(fi.logical_size);
                            fi.estimated_physical_size = fi.physical_size;
                            fi.estimate_valid = false;
                            if let Ok(metadata) = std::fs::metadata(&path) {
                                use std::os::windows::fs::MetadataExt;
                                fi.content_len = metadata.len();
                                fi.logical_size = metadata.len();
                                fi.modified_time = metadata.last_write_time();
                            }
                            folder.push(FileKind::Compressible, fi);
                        }
                        Err(err) => {
                            fi.estimated_physical_size = fi.physical_size;
                            fi.estimate_valid = false;
                            self.gui.status(
                                format!("Error: {}, {}", err, display_path.display()),
                                Some(progress(done_bytes, total_bytes)),
                            );
                            failed.push(fi);
                        }
                    }

                    if last_update.elapsed() > Duration::from_millis(50) {
                        last_update = Instant::now();
                        self.gui.status(
                            if paused {
                                if pending.is_empty() {
                                    "Paused".to_string()
                                } else {
                                    "Pausing after active files finish".to_string()
                                }
                            } else if stopped {
                                if pending.is_empty() {
                                    "Stopping".to_string()
                                } else {
                                    "Stopping after active files finish".to_string()
                                }
                            } else {
                                format!("Expanding: {}", display_path.display())
                            },
                            Some(progress(done_bytes, total_bytes)),
                        );

                        if pending.is_empty() {
                            self.gui.summary(folder.summary());
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if paused
                        && pending.is_empty()
                        && last_update.elapsed() > Duration::from_millis(50)
                    {
                        last_update = Instant::now();
                        self.gui
                            .status("Paused", Some(progress(done_bytes, total_bytes)));
                        self.gui.summary(folder.summary());
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    stopped = true;
                    break;
                }
            }
        }

        drop(send_file);
        for task in tasks {
            task.wait();
        }

        for fi in failed {
            folder.push(FileKind::Compressed, fi);
        }

        let mut exclusion_added = false;
        if exclude_after && !stopped {
            match add_folder_exclusion(&folder.path) {
                Ok(added) => {
                    exclusion_added = true;
                    if added {
                        self.gui.config();
                    }
                }
                Err(err) => self.gui.error("Unable to add exclusion", err),
            }
        }

        let new_size = folder.physical_size;
        let decimal = config().read().unwrap().current().decimal;
        let wasted = new_size.saturating_sub(old_size);

        let msg = if stopped {
            format!(
                "Stopped after expanding {} of {} files in {:.2?}",
                expanded_files,
                total_files,
                start.elapsed()
            )
        } else if exclude_after && exclusion_added {
            format!(
                "Expanded {} files using {} more space and excluded this folder in {:.2?}",
                expanded_files,
                format_size(wasted, decimal),
                start.elapsed()
            )
        } else {
            format!(
                "Expanded {} files using {} more space in {:.2?}",
                expanded_files,
                format_size(wasted, decimal),
                start.elapsed()
            )
        };

        self.gui
            .status(msg, Some(progress(done_bytes, total_bytes)));
        self.gui.summary(folder.summary());
        if stopped {
            self.gui.stopped();
        } else {
            self.gui.scanned();
        }

        self.info = Some(folder);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_compressed_folder() -> FolderInfo {
        let mut folder = FolderInfo::new(PathBuf::from("C:").join("Games"));

        for (path, logical_size, physical_size) in [
            (PathBuf::from("Data").join("one.bin"), 8192, 4096),
            (PathBuf::from("Data").join("two.bin"), 16384, 8192),
            (
                PathBuf::from("Data").join("Sub").join("three.bin"),
                32768,
                12288,
            ),
        ] {
            folder.push(
                FileKind::Compressed,
                FileInfo {
                    path,
                    content_len: logical_size,
                    modified_time: 0,
                    estimate_valid: false,
                    estimated_ratio: 1.0,
                    logical_size,
                    physical_size,
                    estimated_physical_size: physical_size,
                },
            );
        }

        folder
    }

    #[test]
    fn compressed_file_view_filters_paths() {
        let folder = sample_compressed_folder();
        let items = compressed_file_items(&folder, "sub");

        assert_eq!(1, items.len());
        match &items[0] {
            CompressedViewItem::File { path, .. } => {
                assert_eq!(&PathBuf::from("Data").join("Sub").join("three.bin"), path);
            }
            _ => panic!("expected file item"),
        }
    }

    #[test]
    fn compressed_folder_view_groups_files() {
        let folder = sample_compressed_folder();
        let items = compressed_folder_items(&folder, "");

        assert_eq!(2, items.len());
        let data = items
            .iter()
            .find(|item| match item {
                CompressedViewItem::Folder { path, .. } => path == &PathBuf::from("Data"),
                _ => false,
            })
            .expect("Data folder");

        match data {
            CompressedViewItem::Folder {
                count,
                logical_size,
                physical_size,
                ..
            } => {
                assert_eq!(2, *count);
                assert_eq!(24576, *logical_size);
                assert_eq!(12288, *physical_size);
            }
            _ => panic!("expected folder item"),
        }
    }

    #[test]
    fn compressed_view_pagination_clamps_page() {
        let items = (0..3)
            .map(|index| CompressedViewItem::File {
                path: PathBuf::from(format!("{}.bin", index)),
                logical_size: 8192,
                physical_size: 4096,
            })
            .collect();

        let (page, pages, total, page_items) = paginate_compressed_items(items, usize::MAX);
        assert_eq!(0, page);
        assert_eq!(1, pages);
        assert_eq!(3, total);
        assert_eq!(3, page_items.len());
    }

    #[test]
    fn direct_storage_protection_moves_only_matching_candidates() {
        let mut folder = FolderInfo::new(PathBuf::from("D:").join("Games"));
        for path in ["Foo\\data.bin", "Bar\\data.bin"] {
            folder.push(
                FileKind::Compressible,
                FileInfo {
                    path: PathBuf::from(path),
                    content_len: 8192,
                    modified_time: 0,
                    estimate_valid: true,
                    estimated_ratio: 0.5,
                    logical_size: 8192,
                    physical_size: 8192,
                    estimated_physical_size: 4096,
                },
            );
        }

        let protected = protect_direct_storage_candidates(
            &mut folder,
            &[PathBuf::from(r"D:\Games\Foo")],
        );
        assert_eq!(1, protected);
        assert_eq!(1, folder.compressible.count);
        assert_eq!(1, folder.skipped.count);
        assert_eq!(PathBuf::from(r"Bar\data.bin"), folder.compressible.files[0].path);
    }

    #[test]
    fn decompression_auto_is_capped_but_manual_is_respected() {
        // The exact storage result is exercised in folder.rs. This helper only
        // adds the decompression-specific Auto cap.
        assert_eq!(
            1,
            decompression_worker_count_for_path(Path::new(r"Z:\unknown"), 0, true)
        );
    }
}
