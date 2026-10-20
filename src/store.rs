//! Where the diagnostic log and the backups go: `%LOCALAPPDATA%\EpsonReset`,
//! or the folder of the program when that variable is missing.
#![forbid(unsafe_code)]

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

pub struct Store {
    dir: PathBuf,
    log: PathBuf,
}

impl Store {
    /// `stamp` names this run's files, for example "20261020-143005".
    #[must_use]
    pub fn new(stamp: &str) -> Self {
        let dir = std::env::var_os("LOCALAPPDATA")
            .map(|d| PathBuf::from(d).join("EpsonReset"))
            .or_else(|| {
                std::env::current_exe()
                    .ok()?
                    .parent()
                    .map(Path::to_path_buf)
            })
            .unwrap_or_else(|| PathBuf::from("."));
        let log = dir.join(format!("log-{stamp}.txt"));
        Self { dir, log }
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn append_log(&self, text: &str) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()
    }

    /// Writes and flushes the backup to disk before a reset can be offered.
    pub fn write_backup(&self, stamp: &str, text: &str) -> std::io::Result<PathBuf> {
        fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(format!("backup-{stamp}.txt"));
        let mut f = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        Ok(path)
    }
}
