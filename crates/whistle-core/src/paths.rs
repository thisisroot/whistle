//! Where Whistle keeps its files.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::{BaseDirs, ProjectDirs};

#[derive(Debug, Clone)]
pub struct Paths {
    /// Prefixes, AppImages and the installed-apps record.
    pub data: PathBuf,
    /// The downloaded index and installer downloads.
    pub cache: PathBuf,
    /// Where .desktop files go so launchers show up in the desktop's menu.
    pub applications: PathBuf,
}

impl Paths {
    /// The per-user default dirs (`~/.local/share/whistle`, `~/.cache/whistle`
    /// on Linux), or everything under `$WHISTLE_ROOT` when it is set.
    pub fn detect() -> Result<Self> {
        if let Some(root) = std::env::var_os("WHISTLE_ROOT") {
            return Ok(Self::under(Path::new(&root)));
        }
        let project = ProjectDirs::from("", "", "whistle").context("can't find the home directory")?;
        let base = BaseDirs::new().context("can't find the home directory")?;
        Ok(Self {
            data: project.data_dir().to_owned(),
            cache: project.cache_dir().to_owned(),
            applications: base.data_dir().join("applications"),
        })
    }

    /// All dirs inside `root`; for tests and sandboxed runs.
    pub fn under(root: &Path) -> Self {
        Self {
            data: root.join("data"),
            cache: root.join("cache"),
            applications: root.join("applications"),
        }
    }

    pub fn state_file(&self) -> PathBuf {
        self.data.join("installed.json")
    }

    pub fn prefixes(&self) -> PathBuf {
        self.data.join("prefixes")
    }

    pub fn appimages(&self) -> PathBuf {
        self.data.join("appimages")
    }

    pub fn index_cache(&self) -> PathBuf {
        self.cache.join("index.json")
    }

    pub fn downloads(&self) -> PathBuf {
        self.cache.join("downloads")
    }
}
