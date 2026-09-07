from pathlib import Path


def replace(path, old, new):
    p = Path(path)
    text = p.read_text(encoding="utf-8")
    if old not in text:
        raise SystemExit(f"Expected text not found in {path}: {old[:120]!r}")
    p.write_text(text.replace(old, new, 1), encoding="utf-8")


replace(
    "src/folder.rs",
    "const AUTO_MAX_THREADS: usize = 6;\nconst MAX_CONFIGURED_THREADS: usize = 16;",
    "const AUTO_MAX_ANALYSIS_THREADS: usize = 8;\nconst AUTO_MAX_COMPRESSION_THREADS: usize = 16;\nconst MAX_CONFIGURED_THREADS: usize = 16;",
)
replace(
    "src/folder.rs",
    "pub struct FileInfo {\n    pub path: PathBuf,\n    pub logical_size: u64,\n    pub physical_size: u64,\n    pub estimated_physical_size: u64,\n}",
    "pub struct FileInfo {\n    pub path: PathBuf,\n    pub content_len: u64,\n    pub modified_time: u64,\n    pub estimate_valid: bool,\n    pub estimated_ratio: f32,\n    pub logical_size: u64,\n    pub physical_size: u64,\n    pub estimated_physical_size: u64,\n}",
)
replace(
    "src/folder.rs",
    "fn configured_thread_count(cpus: usize, max_threads: usize) -> usize {\n    if max_threads == 0 {\n        cpus.max(1).min(AUTO_MAX_THREADS)\n    } else {\n        max_threads.clamp(1, MAX_CONFIGURED_THREADS)\n    }\n}\n\nfn worker_count_for_storage(\n    cpus: usize,\n    incurs_seek_penalty: Option<bool>,\n    max_threads: usize,\n    hdd_single_thread: bool,\n) -> usize {\n    let configured = configured_thread_count(cpus, max_threads);",
    "fn configured_thread_count(cpus: usize, max_threads: usize, auto_max_threads: usize) -> usize {\n    if max_threads == 0 {\n        cpus.max(1).min(auto_max_threads)\n    } else {\n        max_threads.clamp(1, MAX_CONFIGURED_THREADS)\n    }\n}\n\nfn worker_count_for_storage(\n    cpus: usize,\n    incurs_seek_penalty: Option<bool>,\n    max_threads: usize,\n    hdd_single_thread: bool,\n    auto_max_threads: usize,\n) -> usize {\n    let configured = configured_thread_count(cpus, max_threads, auto_max_threads);",
)
replace(
    "src/folder.rs",
    "pub fn worker_count_for_path(path: &Path, max_threads: usize, hdd_single_thread: bool) -> usize {\n    let cpus = thread::available_parallelism()\n        .map(|count| count.get())\n        .unwrap_or(1);\n    worker_count_for_storage(\n        cpus,\n        volume_incurs_seek_penalty(path),\n        max_threads,\n        hdd_single_thread,\n    )\n}",
    "pub fn compression_worker_count_for_path(\n    path: &Path,\n    max_threads: usize,\n    hdd_single_thread: bool,\n) -> usize {\n    let cpus = thread::available_parallelism()\n        .map(|count| count.get())\n        .unwrap_or(1);\n    worker_count_for_storage(\n        cpus,\n        volume_incurs_seek_penalty(path),\n        max_threads,\n        hdd_single_thread,\n        AUTO_MAX_COMPRESSION_THREADS,\n    )\n}",
)
replace(
    "src/folder.rs",
    "        let analysis_workers = worker_count_for_storage(\n            cpus,\n            seek_penalty,\n            max_threads,\n            hdd_single_thread,\n        );",
    "        let analysis_workers = worker_count_for_storage(\n            cpus,\n            seek_penalty,\n            max_threads,\n            hdd_single_thread,\n            AUTO_MAX_ANALYSIS_THREADS,\n        );",
)
replace(
    "src/folder.rs",
    "            let fi = FileInfo {\n                path: shortname,\n                logical_size,\n                physical_size: physical,\n                estimated_physical_size: physical,\n            };",
    "            let fi = FileInfo {\n                path: shortname,\n                content_len: metadata.len(),\n                modified_time: metadata.last_write_time(),\n                estimate_valid: false,\n                estimated_ratio: 1.0,\n                logical_size,\n                physical_size: physical,\n                estimated_physical_size: physical,\n            };",
)
replace(
    "src/folder.rs",
    "        Ok(ratio) if ratio < ratio_limit => {\n            fi.estimated_physical_size =\n                estimate_physical_size(fi.logical_size, ratio).min(fi.physical_size);\n            ds.push(FileKind::Compressible, fi);",
    "        Ok(ratio) if ratio < ratio_limit => {\n            fi.estimated_physical_size =\n                estimate_physical_size(fi.logical_size, ratio).min(fi.physical_size);\n            fi.estimate_valid = true;\n            fi.estimated_ratio = ratio;\n            ds.push(FileKind::Compressible, fi);",
)
replace(
    "src/folder.rs",
    "fn analysis_workers_are_storage_aware() {\n    assert_eq!(1, worker_count_for_storage(16, Some(true), 0, true));\n    assert_eq!(1, worker_count_for_storage(16, None, 0, true));\n    assert_eq!(1, worker_count_for_storage(1, Some(false), 0, true));\n    assert_eq!(6, worker_count_for_storage(16, Some(false), 0, true));\n    assert_eq!(4, worker_count_for_storage(16, Some(false), 4, true));\n    assert_eq!(6, worker_count_for_storage(16, Some(true), 0, false));\n    assert_eq!(16, worker_count_for_storage(4, Some(false), 16, true));\n}",
    "fn analysis_workers_are_storage_aware() {\n    assert_eq!(\n        1,\n        worker_count_for_storage(16, Some(true), 0, true, AUTO_MAX_ANALYSIS_THREADS)\n    );\n    assert_eq!(\n        1,\n        worker_count_for_storage(16, None, 0, true, AUTO_MAX_ANALYSIS_THREADS)\n    );\n    assert_eq!(\n        1,\n        worker_count_for_storage(1, Some(false), 0, true, AUTO_MAX_ANALYSIS_THREADS)\n    );\n    assert_eq!(\n        8,\n        worker_count_for_storage(16, Some(false), 0, true, AUTO_MAX_ANALYSIS_THREADS)\n    );\n    assert_eq!(\n        4,\n        worker_count_for_storage(16, Some(false), 4, true, AUTO_MAX_ANALYSIS_THREADS)\n    );\n    assert_eq!(\n        8,\n        worker_count_for_storage(16, Some(true), 0, false, AUTO_MAX_ANALYSIS_THREADS)\n    );\n    assert_eq!(\n        16,\n        worker_count_for_storage(32, Some(false), 0, true, AUTO_MAX_COMPRESSION_THREADS)\n    );\n    assert_eq!(\n        16,\n        worker_count_for_storage(4, Some(false), 16, true, AUTO_MAX_ANALYSIS_THREADS)\n    );\n}",
)

Path("src/compression.rs").write_text(r'''use std::io;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::PathBuf;

use compresstimator::Compresstimator;
use crossbeam_channel::{Receiver, Sender};
use filetime::FileTime;
use fs2::FileExt;
use winapi::um::winnt::{FILE_READ_DATA, FILE_WRITE_ATTRIBUTES};

use crate::background::Background;
use crate::background::ControlToken;
use crate::compact::{self, Compression};

#[derive(Debug, Clone)]
pub struct CompressionJob {
    pub path: PathBuf,
    pub content_len: u64,
    pub modified_time: u64,
    pub estimate_valid: bool,
    pub estimated_ratio: f32,
}

#[derive(Debug)]
pub struct BackgroundCompactor {
    compression: Option<Compression>,
    ratio_limit: f32,
    files_in: Receiver<CompressionJob>,
    files_out: Sender<(PathBuf, io::Result<bool>)>,
}

impl BackgroundCompactor {
    pub fn new(
        compression: Option<Compression>,
        ratio_limit: f32,
        files_in: Receiver<CompressionJob>,
        files_out: Sender<(PathBuf, io::Result<bool>)>,
    ) -> Self {
        Self {
            compression,
            ratio_limit,
            files_in,
            files_out,
        }
    }
}

fn reuse_estimate(job: &CompressionJob, content_len: u64, modified_time: u64) -> Option<f32> {
    if job.estimate_valid
        && job.content_len == content_len
        && job.modified_time == modified_time
        && job.estimated_ratio.is_finite()
    {
        Some(job.estimated_ratio)
    } else {
        None
    }
}

fn handle_file(
    job: &CompressionJob,
    compression: Option<Compression>,
    ratio_limit: f32,
) -> io::Result<bool> {
    let meta = std::fs::metadata(&job.path)?;
    let handle = std::fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES | FILE_READ_DATA)
        .open(&job.path)?;

    handle.try_lock_exclusive()?;

    let ret = match compression {
        Some(compression) => match reuse_estimate(job, meta.len(), meta.last_write_time()) {
            Some(ratio) if ratio < ratio_limit => {
                compact::compress_file_handle(&handle, compression)
            }
            Some(_) => Ok(false),
            None => {
                let est = Compresstimator::with_block_size(8192);
                match est.compresstimate(&handle, meta.len()) {
                    Ok(ratio) if ratio < ratio_limit => {
                        compact::compress_file_handle(&handle, compression)
                    }
                    Ok(_) => Ok(false),
                    Err(e) => Err(e),
                }
            }
        },
        None => compact::uncompress_file_handle(&handle).map(|_| true),
    };

    let _ = filetime::set_file_handle_times(
        &handle,
        Some(FileTime::from_last_access_time(&meta)),
        Some(FileTime::from_last_modification_time(&meta)),
    );

    handle.unlock()?;

    ret
}

impl Background for BackgroundCompactor {
    type Output = ();
    type Status = ();

    fn run(self, control: &ControlToken<Self::Status>) -> Self::Output {
        for job in &self.files_in {
            if control.is_cancelled_with_pause() {
                break;
            }

            let path = job.path.clone();
            let ret = handle_file(&job, self.compression, self.ratio_limit);
            if self.files_out.send((path, ret)).is_err() {
                break;
            }
        }
    }
}

#[test]
fn estimate_reuse_requires_matching_fingerprint() {
    let job = CompressionJob {
        path: PathBuf::from("test.bin"),
        content_len: 8192,
        modified_time: 1234,
        estimate_valid: true,
        estimated_ratio: 0.5,
    };

    assert_eq!(Some(0.5), reuse_estimate(&job, 8192, 1234));
    assert_eq!(None, reuse_estimate(&job, 4096, 1234));
    assert_eq!(None, reuse_estimate(&job, 8192, 5678));
}

#[test]
fn invalid_estimate_is_not_reused() {
    let job = CompressionJob {
        path: PathBuf::from("test.bin"),
        content_len: 8192,
        modified_time: 1234,
        estimate_valid: false,
        estimated_ratio: 0.5,
    };

    assert_eq!(None, reuse_estimate(&job, 8192, 1234));
}
''', encoding="utf-8")

replace(
    "src/backend.rs",
    "use crate::compression::BackgroundCompactor;\nuse crate::folder::{worker_count_for_path, FileInfo, FileKind, FolderInfo, FolderScan};",
    "use crate::compression::{BackgroundCompactor, CompressionJob};\nuse crate::folder::{\n    compression_worker_count_for_path, FileInfo, FileKind, FolderInfo, FolderScan,\n};",
)
replace(
    "src/backend.rs",
    "        let worker_count =\n            worker_count_for_path(&folder.path, current.max_threads, current.hdd_single_thread);\n\n        let (send_file, send_file_rx) = bounded::<(PathBuf, u64)>(worker_count);",
    "        let worker_count = compression_worker_count_for_path(\n            &folder.path,\n            current.max_threads,\n            current.hdd_single_thread,\n        );\n\n        let (send_file, send_file_rx) = bounded::<CompressionJob>(worker_count);",
)
replace(
    "src/backend.rs",
    "                    let logical_size = fi.logical_size;\n\n                    if send_file.send((path.clone(), logical_size)).is_err() {",
    "                    let logical_size = fi.logical_size;\n                    let job = CompressionJob {\n                        path: path.clone(),\n                        content_len: fi.content_len,\n                        modified_time: fi.modified_time,\n                        estimate_valid: fi.estimate_valid,\n                        estimated_ratio: fi.estimated_ratio,\n                    };\n\n                    if send_file.send(job).is_err() {",
)
replace(
    "src/backend.rs",
    "                        Ok(true) => {\n                            fi.physical_size = path.size_on_disk().unwrap_or(fi.physical_size);\n                            fi.estimated_physical_size = fi.physical_size;",
    "                        Ok(true) => {\n                            fi.physical_size = path.size_on_disk().unwrap_or(fi.physical_size);\n                            fi.estimated_physical_size = fi.physical_size;\n                            fi.estimate_valid = false;\n                            if let Ok(metadata) = std::fs::metadata(&path) {\n                                use std::os::windows::fs::MetadataExt;\n                                fi.content_len = metadata.len();\n                                fi.modified_time = metadata.last_write_time();\n                            }",
)
replace(
    "src/backend.rs",
    "                        Ok(false) => {\n                            fi.estimated_physical_size = fi.physical_size;",
    "                        Ok(false) => {\n                            fi.estimated_physical_size = fi.physical_size;\n                            fi.estimate_valid = false;",
)
replace(
    "src/backend.rs",
    "        let (send_file, send_file_rx) = bounded::<(PathBuf, u64)>(1);",
    "        let (send_file, send_file_rx) = bounded::<CompressionJob>(1);",
)
replace(
    "src/backend.rs",
    "                send_file\n                    .send((folder.path.join(&fi.path), logical_size))\n                    .expect(\"send_file\");",
    "                send_file\n                    .send(CompressionJob {\n                        path: folder.path.join(&fi.path),\n                        content_len: fi.content_len,\n                        modified_time: fi.modified_time,\n                        estimate_valid: fi.estimate_valid,\n                        estimated_ratio: fi.estimated_ratio,\n                    })\n                    .expect(\"send_file\");",
)
replace(
    "src/backend.rs",
    "                FileInfo {\n                    path,\n                    logical_size,\n                    physical_size,\n                    estimated_physical_size: physical_size,\n                },",
    "                FileInfo {\n                    path,\n                    content_len: logical_size,\n                    modified_time: 0,\n                    estimate_valid: false,\n                    estimated_ratio: 1.0,\n                    logical_size,\n                    physical_size,\n                    estimated_physical_size: physical_size,\n                },",
)

replace(
    "src/ui/index.html",
    '<input id="Max_Threads" name="Max_Threads" type="number" min="0" max="16" step="1" value="0" style="width: 70px"> <small>0 = Auto</small>',
    '<input id="Max_Threads" name="Max_Threads" type="number" min="0" max="16" step="1" value="0" style="width: 70px"> <small title="Auto uses up to 8 analysis workers and up to 16 compression workers on SSDs; HDDs remain single-threaded by default.">0 = Auto (hardware-aware)</small>',
)
replace(
    "README.md",
    "`Auto` uses up to six worker threads on storage that Windows reports as having no seek penalty. HDDs use one thread by default, and unknown storage is handled conservatively with one thread. A manual limit from 1 to 16 can be set in Settings.",
    "`Auto` is storage- and workload-aware. On SSDs it uses up to eight workers for analysis and up to 16 logical-CPU workers for compression; HDDs use one worker by default to avoid seek thrashing, and unknown storage is handled conservatively with one worker. A manual limit from 1 to 16 can be set in Settings. Compression reuses a valid analysis estimate when the file has not changed, avoiding duplicate sampling before WOF compression.",
)
replace(
    "CHANGELOG.md",
    "### Added\n\n- In-app viewer for WOF-compressed files and containing folders, with path filtering and pagination for large scans.\n\n## [0.11.1]",
    "### Added\n\n- In-app viewer for WOF-compressed files and containing folders, with path filtering and pagination for large scans.\n\n### Changed\n\n- Auto threading now uses up to eight analysis workers and up to 16 compression workers on SSDs while retaining the single-thread HDD safeguard.\n- Compression reuses the analysis compressibility estimate when file size and modification time are unchanged, avoiding duplicate sampling; changed files are re-estimated before compression.\n\n### Maintenance\n\n- Windows CI validates the embedded JavaScript syntax before building.\n\n## [0.11.1]",
)
