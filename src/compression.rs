use std::io;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::PathBuf;

use compresstimator::Compresstimator;
use crossbeam_channel::{Receiver, Sender};
use filetime::FileTime;
use fs2::FileExt;
use winapi::um::processthreadsapi::{GetCurrentThread, SetThreadPriority};
use winapi::um::winbase::THREAD_PRIORITY_BELOW_NORMAL;
use winapi::um::winnt::{FILE_READ_DATA, FILE_SHARE_READ, FILE_WRITE_ATTRIBUTES};

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
        // Allow readers, but deny concurrent writers/deleters while a WOF
        // operation is in flight. This avoids racing game/app updates.
        .share_mode(FILE_SHARE_READ)
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
        // Compression is deliberately background-friendly. WOF/LZX can be CPU
        // intensive, so lower only these worker threads rather than the GUI or
        // analysis thread. Failure to change priority is non-fatal.
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        }

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
