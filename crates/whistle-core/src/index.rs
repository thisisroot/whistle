//! The catalog index: every app and the ways to get it. whistle-server
//! publishes it; the client only reads it.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;
use crate::recipe::Runtime;

pub const FORMAT: u32 = 1;

/// Where whistle-server publishes the index; `$WHISTLE_INDEX_URL` overrides it.
pub const DEFAULT_URL: &str =
    "https://github.com/thisisroot/whistle-server/releases/download/index/index.json";

/// Largest index `update` accepts.
const MAX_INDEX_BYTES: u64 = 64 * 1024 * 1024;

/// Small hand-written catalog used until a real index has been fetched.
const DEV_INDEX: &str = include_str!("../../../catalog/dev-index.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    pub format: u32,
    #[serde(default)]
    pub generated_at: String,
    pub apps: Vec<App>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct App {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub icon_url: String,
    #[serde(default)]
    pub homepage: String,
    /// SPDX expression, or `LicenseRef-proprietary`.
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub recipes: Vec<RecipeEntry>,
}

/// One way to get an app natively. Mirrors `catalog.Source` on the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub kind: SourceKind,
    #[serde(rename = "ref")]
    pub reference: String,
    /// Platform tags such as `linux`, `linux:arch` or `windows`.
    pub platforms: Vec<String>,
    /// Empty means any architecture.
    #[serde(default)]
    pub arches: Vec<String>,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Pacman,
    Apt,
    Dnf,
    Zypper,
    Flatpak,
    Appimage,
    Winget,
    VendorInstaller,
    Wsl,
}

impl SourceKind {
    pub const ALL: [SourceKind; 9] = [
        Self::Pacman,
        Self::Apt,
        Self::Dnf,
        Self::Zypper,
        Self::Flatpak,
        Self::Appimage,
        Self::Winget,
        Self::VendorInstaller,
        Self::Wsl,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pacman => "pacman",
            Self::Apt => "apt",
            Self::Dnf => "dnf",
            Self::Zypper => "zypper",
            Self::Flatpak => "flatpak",
            Self::Appimage => "appimage",
            Self::Winget => "winget",
            Self::VendorInstaller => "vendor_installer",
            Self::Wsl => "wsl",
        }
    }

    /// The distro's own package manager.
    pub fn is_native(self) -> bool {
        matches!(self, Self::Pacman | Self::Apt | Self::Dnf | Self::Zypper)
    }
}

impl fmt::Display for SourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The current revision of a recipe for this app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeEntry {
    pub id: String,
    pub runtime: Runtime,
    pub status: RecipeStatus,
    pub revision: u32,
    /// The recipe itself. Kept as raw JSON so an index carrying a recipe this
    /// client can't parse still loads; the resolver reports it instead.
    pub spec: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipeStatus {
    Verified,
    Community,
    Draft,
    Retired,
}

impl fmt::Display for RecipeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Verified => "verified",
            Self::Community => "community",
            Self::Draft => "draft",
            Self::Retired => "retired",
        })
    }
}

/// Where an index was loaded from.
#[derive(Debug, Clone)]
pub enum IndexOrigin {
    File(PathBuf),
    Builtin,
}

impl fmt::Display for IndexOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(p) => write!(f, "{}", p.display()),
            Self::Builtin => f.write_str("built-in dev catalog"),
        }
    }
}

impl Index {
    /// Load from `explicit`, then `$WHISTLE_INDEX`, then the cached download,
    /// and fall back to the built-in dev catalog.
    pub fn load(explicit: Option<&Path>, paths: &Paths) -> Result<(Self, IndexOrigin)> {
        let chosen = explicit
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os("WHISTLE_INDEX").map(PathBuf::from));
        if let Some(path) = chosen {
            return Ok((Self::from_file(&path)?, IndexOrigin::File(path)));
        }
        let cached = paths.index_cache();
        if cached.exists() {
            return Ok((Self::from_file(&cached)?, IndexOrigin::File(cached)));
        }
        Ok((Self::builtin(), IndexOrigin::Builtin))
    }

    /// Fetch an index from a URL or local path, check it parses, and make it
    /// the cached index. Returns the new index.
    pub fn update(from: &str, paths: &Paths) -> Result<Self> {
        let text = if from.starts_with("https://") {
            crate::fetch::download_text(from, MAX_INDEX_BYTES)?
        } else if from.contains("://") {
            bail!("refusing to fetch the index over anything but HTTPS: {from}");
        } else {
            std::fs::read_to_string(from).with_context(|| format!("reading {from}"))?
        };
        let index = Self::parse(&text).with_context(|| format!("loading index from {from}"))?;
        let dest = paths.index_cache();
        std::fs::create_dir_all(&paths.cache)?;
        let tmp = dest.with_extension("json.tmp");
        std::fs::write(&tmp, &text)?;
        std::fs::rename(&tmp, &dest).with_context(|| format!("writing {}", dest.display()))?;
        Ok(index)
    }

    pub fn builtin() -> Self {
        Self::parse(DEV_INDEX).expect("the built-in dev index is valid")
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("loading index {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let index: Self = serde_json::from_str(text)?;
        if index.format != FORMAT {
            bail!(
                "index format {} is not supported (expected {FORMAT}); update Whistle",
                index.format
            );
        }
        Ok(index)
    }

    /// Apps whose id, name or summary contains `query`, best matches first.
    pub fn search(&self, query: &str) -> Vec<&App> {
        let q = query.to_lowercase();
        let mut hits: Vec<(u8, &App)> = self
            .apps
            .iter()
            .filter_map(|app| {
                let name = app.name.to_lowercase();
                let rank = if name == q || app.id.to_lowercase() == q {
                    0
                } else if name.starts_with(&q) {
                    1
                } else if name.contains(&q) || app.id.to_lowercase().contains(&q) {
                    2
                } else if app.summary.to_lowercase().contains(&q) {
                    3
                } else {
                    return None;
                };
                Some((rank, app))
            })
            .collect();
        hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
        hits.into_iter().map(|(_, app)| app).collect()
    }

    /// The app `query` names: an exact id, an exact name (any case), or the
    /// only app whose name or id contains it.
    pub fn find(&self, query: &str) -> Result<&App> {
        if let Some(app) = self.apps.iter().find(|a| a.id == query) {
            return Ok(app);
        }
        if let Some(app) = self
            .apps
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(query))
        {
            return Ok(app);
        }
        let q = query.to_lowercase();
        let partial: Vec<&App> = self
            .apps
            .iter()
            .filter(|a| a.name.to_lowercase().contains(&q) || a.id.to_lowercase().contains(&q))
            .collect();
        match partial.as_slice() {
            [app] => Ok(app),
            [] => bail!("no app matches {query:?}; try `whistle search`"),
            many => {
                let names: Vec<String> = many
                    .iter()
                    .map(|a| format!("{} ({})", a.name, a.id))
                    .collect();
                bail!("{query:?} matches several apps: {}", names.join(", "))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_index_loads() {
        let index = Index::builtin();
        assert!(index.apps.len() >= 5);
    }

    #[test]
    fn find_by_id_name_and_fragment() {
        let index = Index::builtin();
        assert_eq!(index.find("org.mozilla.firefox").unwrap().name, "Firefox");
        assert_eq!(index.find("firefox").unwrap().id, "org.mozilla.firefox");
        assert_eq!(
            index.find("notepad").unwrap().id,
            "org.notepad-plus-plus.NotepadPlusPlus"
        );
        assert!(index.find("no-such-app").is_err());
    }

    #[test]
    fn search_ranks_name_prefix_first() {
        let index = Index::builtin();
        let hits = index.search("fire");
        assert_eq!(hits[0].id, "org.mozilla.firefox");
        assert!(
            index
                .search("office")
                .iter()
                .any(|a| a.id == "com.microsoft.Office")
        );
    }

    #[test]
    fn update_from_file_replaces_the_cache() {
        let root = std::env::temp_dir().join(format!("whistle-index-{}", std::process::id()));
        let paths = Paths::under(&root);
        std::fs::create_dir_all(&root).unwrap();
        let src = root.join("new.json");
        std::fs::write(
            &src,
            r#"{"format": 1, "apps": [{"id": "org.example.A", "name": "A", "license": "MIT"}]}"#,
        )
        .unwrap();

        let index = Index::update(src.to_str().unwrap(), &paths).unwrap();
        assert_eq!(index.apps[0].license, "MIT");
        let (loaded, origin) = Index::load(None, &paths).unwrap();
        assert_eq!(loaded.apps.len(), 1);
        assert!(matches!(origin, IndexOrigin::File(p) if p == paths.index_cache()));

        std::fs::write(&src, "not json").unwrap();
        assert!(Index::update(src.to_str().unwrap(), &paths).is_err());
        assert_eq!(
            Index::load(None, &paths).unwrap().0.apps.len(),
            1,
            "a bad index must not replace a good one"
        );
        assert!(Index::update("http://example.org/index.json", &paths).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unknown_format() {
        let err = Index::parse(r#"{"format": 99, "apps": []}"#).unwrap_err();
        assert!(err.to_string().contains("format 99"));
    }
}
