use std::io;
use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde_derive::{Deserialize, Serialize};

use crate::compact::Compression;

fn default_min_savings_percent() -> f32 {
    1.0
}

#[derive(Debug, Default)]
pub struct ConfigFile {
    backing: Option<PathBuf>,
    config: Config,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub decimal: bool,
    pub compression: Compression,
    #[serde(default = "default_min_savings_percent")]
    pub min_savings_percent: f32,
    pub excludes: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            decimal: false,
            compression: Compression::default(),
            min_savings_percent: default_min_savings_percent(),
            excludes: vec![
                "*:\\Windows*",
                "*:\\System Volume Information*",
                "*:\\$*",
                "*.7z",
                "*.aac",
                "*.avi",
                "*.ba",
                "*.{bik,bk2,bnk,pc_binkvid}",
                "*.br",
                "*.bz2",
                "*.cab",
                "*.dl_",
                "*.docx",
                "*.flac",
                "*.flv",
                "*.gif",
                "*.gz",
                "*.jpeg",
                "*.jpg",
                "*.log",
                "*.lz4",
                "*.lzma",
                "*.lzx",
                "*.m[24]v",
                "*.m4a",
                "*.mkv",
                "*.mp[234]",
                "*.mpeg",
                "*.mpg",
                "*.ogg",
                "*.onepkg",
                "*.png",
                "*.pptx",
                "*.rar",
                "*.upk",
                "*.vob",
                "*.vs[st]x",
                "*.wem",
                "*.webm",
                "*.wm[afv]",
                "*.xap",
                "*.xnb",
                "*.xlsx",
                "*.xz",
                "*.zst",
                "*.zstd",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        }
    }
}

impl ConfigFile {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            backing: Some(path.as_ref().to_owned()),
            config: std::fs::read(path)
                .and_then(|data| {
                    serde_json::from_slice::<Config>(&data)
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
                })
                .unwrap_or_default(),
        }
    }

    pub fn save(&self) -> io::Result<()> {
        match &self.backing {
            Some(path) => {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)?;
                }

                let data = serde_json::to_string_pretty(&self.config).expect("Serialize");
                std::fs::write(path, &data)
            }
            None => Ok(()),
        }
    }

    pub fn current(&self) -> Config {
        self.config.clone()
    }

    pub fn replace(&mut self, c: Config) {
        self.config = c;
    }
}

impl Config {
    pub fn ratio_limit(&self) -> f32 {
        1.0 - (self.min_savings_percent.clamp(0.0, 100.0) / 100.0)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.min_savings_percent.is_finite()
            || self.min_savings_percent < 0.0
            || self.min_savings_percent > 100.0
        {
            return Err("Minimum estimated savings must be between 0 and 100%.".to_string());
        }

        self.globset().map(|_| ())
    }

    pub fn globset(&self) -> Result<GlobSet, String> {
        let mut globs = GlobSetBuilder::new();

        for pattern in self.excludes.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let glob = GlobBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .map_err(|e| e.to_string())?;
            globs.add(glob);
        }

        globs.build().map_err(|e| e.to_string())
    }
}

#[test]
fn test_config() {
    let s = Config::default();

    assert!(s.globset().is_ok());
    assert_eq!(s.min_savings_percent, 1.0);
    assert!((s.ratio_limit() - 0.99).abs() < f32::EPSILON);

    let gs = s.globset().unwrap();
    assert!(gs.is_match("C:\\foo\\bar\\hmm.rar"));
    assert!(gs.is_match("C:\\Windows\\System32\\floop\\bla.txt"));
    assert!(gs.is_match("C:\\x.lz4"));
    assert!(gs.is_match("C:\\foo\\PHOTO.JPG"));
}

#[test]
fn blank_excludes_are_ignored() {
    let mut s = Config::default();
    s.excludes.push(String::new());
    s.excludes.push("   ".to_string());

    let gs = s.globset().unwrap();
    assert!(!gs.is_match("C:\\foo\\ordinary.txt"));
}

#[test]
fn invalid_threshold_is_rejected() {
    let mut s = Config::default();
    s.min_savings_percent = 101.0;
    assert!(s.validate().is_err());
}
