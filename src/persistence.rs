use directories::ProjectDirs;
use hashfilter::HashFilter;
use lazy_static::lazy_static;
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::UNIX_EPOCH;

use crate::config::ConfigFile;

lazy_static! {
    static ref PATHDB: RwLock<HashFilter> = RwLock::new(HashFilter::default());
    static ref CONFIG: RwLock<ConfigFile> = RwLock::new(ConfigFile::default());
}

pub fn init() {
    if let Some(dirs) = ProjectDirs::from("", "Freaky", "Compactor") {
        // V2 keys include file identity (size + modification time), so files that
        // are replaced or updated are automatically reconsidered for compression.
        pathdb()
            .write()
            .unwrap()
            .set_backing(dirs.cache_dir().join("incompressible-v2.dat"));
        *config().write().unwrap() = ConfigFile::new(dirs.config_dir().join("config.json"));
    }
}

pub fn incompressible_key(path: &Path, metadata: &Metadata) -> (PathBuf, u64, u64, u32) {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .unwrap_or_default();

    (
        path.to_path_buf(),
        metadata.len(),
        modified.as_secs(),
        modified.subsec_nanos(),
    )
}

pub fn config() -> &'static RwLock<ConfigFile> {
    &CONFIG
}

pub fn pathdb() -> &'static RwLock<HashFilter> {
    &PATHDB
}
