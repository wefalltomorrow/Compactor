use std::io;
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

    let estimate_is_current = job.estimate_valid
        && meta.len() == job.content_len
        && meta.last_write_time() == job.modified_time;

    let ret = match compression {
        Some(compression) if estimate_is_current => {
            compact::compress_file_handle(&handle, compression)
        }
        Some(compression) => {
            let est = Compresstimator::with_block_size(8192);
            match est.compresstimate(&handle, meta.len()) {
                Ok(ratio) if ratio < ratio_limit => {
                    compact::compress_file_handle(&handle, compression)
                }
                Ok(_) => Ok(false),
                Err(e) => Err(e),
            }
        }
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
