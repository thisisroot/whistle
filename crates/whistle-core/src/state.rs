//! What Whistle has installed, so `list` and `remove` know how each app got here.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::index::SourceKind;
use crate::recipe::Runtime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Installed {
    pub app_id: String,
    pub app_name: String,
    /// Unix seconds.
    pub installed_at: u64,
    #[serde(flatten)]
    pub how: How,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "via", rename_all = "snake_case")]
pub enum How {
    Package {
        kind: SourceKind,
        #[serde(rename = "ref")]
        reference: String,
    },
    Appimage {
        path: PathBuf,
        desktop_file: PathBuf,
    },
    Recipe {
        recipe_id: String,
        revision: u32,
        runtime: Runtime,
        prefix: PathBuf,
        desktop_files: Vec<PathBuf>,
    },
}

impl How {
    pub fn describe(&self) -> String {
        match self {
            Self::Package { kind, reference } => format!("{kind}: {reference}"),
            Self::Appimage { path, .. } => format!("appimage: {}", path.display()),
            Self::Recipe {
                recipe_id,
                revision,
                runtime,
                ..
            } => format!("{runtime}: {recipe_id} (rev {revision})"),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub installed: Vec<Installed>,
}

impl State {
    /// A missing file is an empty state.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                serde_json::from_str(&text).with_context(|| format!("reading {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Write via a temp file and rename, so a crash never leaves half a file.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)? + "\n")?;
        std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
    }

    pub fn get(&self, app_id: &str) -> Option<&Installed> {
        self.installed.iter().find(|i| i.app_id == app_id)
    }

    /// By id, or by name ignoring case.
    pub fn find(&self, query: &str) -> Option<&Installed> {
        self.get(query).or_else(|| {
            self.installed
                .iter()
                .find(|i| i.app_name.eq_ignore_ascii_case(query))
        })
    }

    pub fn record(&mut self, entry: Installed) {
        self.forget(&entry.app_id.clone());
        self.installed.push(entry);
        self.installed.sort_by_key(|a| a.app_name.to_lowercase());
    }

    pub fn forget(&mut self, app_id: &str) {
        self.installed.retain(|i| i.app_id != app_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("whistle-state-{}", std::process::id()));
        let path = dir.join("installed.json");
        assert!(State::load(&path).unwrap().installed.is_empty());

        let mut state = State::default();
        state.record(Installed {
            app_id: "org.mozilla.firefox".into(),
            app_name: "Firefox".into(),
            installed_at: 1,
            how: How::Package {
                kind: SourceKind::Apt,
                reference: "firefox".into(),
            },
        });
        state.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""via": "package""#) && text.contains(r#""ref": "firefox""#));

        let mut loaded = State::load(&path).unwrap();
        assert_eq!(
            loaded.find("firefox").unwrap().how.describe(),
            "apt: firefox"
        );
        loaded.forget("org.mozilla.firefox");
        assert!(loaded.installed.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
