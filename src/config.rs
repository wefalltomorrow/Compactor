use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde_derive::{Deserialize, Serialize};

use crate::compact::Compression;

fn default_min_savings_percent() -> f32 {
    1.0
}

fn default_max_threads() -> usize {
    0
}

fn default_hdd_single_thread() -> bool {
    true
}

fn default_compression_priority() -> CompressionPriority {
    CompressionPriority::BelowNormal
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompressionPriority {
    Lowest,
    BelowNormal,
    Normal,
    AboveNormal,
    Highest,
}

impl Default for CompressionPriority {
    fn default() -> Self {
        Self::BelowNormal
    }
}

impl fmt::Display for CompressionPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Lowest => "Lowest",
            Self::BelowNormal => "BelowNormal",
            Self::Normal => "Normal",
            Self::AboveNormal => "AboveNormal",
            Self::Highest => "Highest",
        })
    }
}

impl FromStr for CompressionPriority {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "lowest" => Ok(Self::Lowest),
            "belownormal" | "below normal" | "below-normal" => Ok(Self::BelowNormal),
            "normal" => Ok(Self::Normal),
            "abovenormal" | "above normal" | "above-normal" => Ok(Self::AboveNormal),
            "highest" => Ok(Self::Highest),
            _ => Err(()),
        }
    }
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
    #[serde(default = "default_max_threads")]
    pub max_threads: usize,
    #[serde(default = "default_hdd_single_thread")]
    pub hdd_single_thread: bool,
    #[serde(default = "default_compression_priority")]
    pub compression_priority: CompressionPriority,
    pub excludes: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            decimal: false,
            compression: Compression::Lzx,
            min_savings_percent: default_min_savings_percent(),
            max_threads: default_max_threads(),
            hdd_single_thread: default_hdd_single_thread(),
            compression_priority: default_compression_priority(),
            excludes: vec![
                "*:\\Windows*",
                "*:\\System Volume Information*",
                "*:\\$*",
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

        if self.max_threads > 16 {
            return Err("Maximum threads must be between 0 (Auto) and 16.".to_string());
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
    assert_eq!(s.compression, Compression::Lzx);
    assert_eq!(s.min_savings_percent, 1.0);
    assert_eq!(s.max_threads, 0);
    assert!(s.hdd_single_thread);
    assert_eq!(s.compression_priority, CompressionPriority::BelowNormal);
    assert!((s.ratio_limit() - 0.99).abs() < f32::EPSILON);

    let gs = s.globset().unwrap();
    assert!(gs.is_match("C:\\Windows\\System32\\floop\\bla.txt"));
    assert!(gs.is_match("C:\\System Volume Information\\tracking.log"));
    assert!(gs.is_match("C:\\$Recycle.Bin\\example.bin"));

    // File extensions are not excluded by default. The sampled estimator and
    // configured savings threshold decide whether compression is worthwhile.
    assert!(!gs.is_match("C:\\foo\\archive.rar"));
    assert!(!gs.is_match("C:\\foo\\data.lz4"));
    assert!(!gs.is_match("C:\\foo\\PHOTO.JPG"));
}

#[test]
fn compression_priority_parses_ui_values() {
    assert_eq!(
        CompressionPriority::BelowNormal,
        "BelowNormal".parse::<CompressionPriority>().unwrap()
    );
    assert_eq!(
        CompressionPriority::AboveNormal,
        "Above Normal".parse::<CompressionPriority>().unwrap()
    );
    assert!("Realtime".parse::<CompressionPriority>().is_err());
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

#[test]
fn invalid_thread_limit_is_rejected() {
    let mut s = Config::default();
    s.max_threads = 17;
    assert!(s.validate().is_err());
}
