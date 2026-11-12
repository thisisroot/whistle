//! The catalog index: every app and the ways to get it. whistle-server
//! publishes it; the client only reads it.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use crate::paths::Paths;
use crate::recipe::Runtime;

pub const FORMAT: u32 = 1;

/// Where whistle-server publishes the index; `$WHISTLE_INDEX_URL` overrides it.
pub const DEFAULT_URL: &str =
    "https://github.com/thisisroot/whistle-server/releases/download/index/catalog.json";

/// Largest index `update` accepts.
const MAX_INDEX_BYTES: u64 = 64 * 1024 * 1024;

/// whistle-server signs every index it publishes with minisign. A downloaded
/// index must be signed by one of these keys; `$WHISTLE_INDEX_PUBKEY` replaces
/// them, for catalogs published by someone else.
const TRUSTED_KEYS: &[&str] = &["RWRlRc/lhixaEkgN7SMrn4oxizAm3Qhl7RD6vr/wduv6wxpTJk4ifQrW"];

/// Whether a word of `name` (split at spaces and punctuation) starts with `query`.
fn name_word_starts_with(name: &str, query: &str) -> bool {
    !query.is_empty()
        && (name.starts_with(query)
            || name
                .match_indices(|c: char| !c.is_alphanumeric())
                .any(|(i, sep)| name[i + sep.len()..].starts_with(query)))
}

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
    /// Screenshot image URLs, best first.
    #[serde(default)]
    pub screenshots: Vec<String>,
    #[serde(default)]
    pub homepage: String,
    /// SPDX expression, or `LicenseRef-proprietary`.
    #[serde(default)]
    pub license: String,
    /// How widely it's used, 0 to 1000 (a percentile of Flathub installs or
    /// Debian popcon). Search puts popular apps first.
    #[serde(default)]
    pub popularity: u16,
    #[serde(default, deserialize_with = "skip_unreadable")]
    pub sources: Vec<Source>,
    #[serde(default, deserialize_with = "skip_unreadable")]
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
    /// Generated automatically and never tried: offered, with a warning.
    Untested,
    Draft,
    Retired,
}

impl fmt::Display for RecipeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Verified => "verified",
            Self::Community => "community",
            Self::Untested => "untested",
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
    ///
    /// A downloaded index must carry a valid signature (`<url>.minisig`) and
    /// must not be older than the cached one. A local file is trusted as is:
    /// the user picked it.
    pub fn update(from: &str, paths: &Paths) -> Result<Self> {
        let downloaded = from.starts_with("https://");
        let text = if downloaded {
            // The compressed copy is a sixth of the size; older servers may not have it.
            let text =
                match crate::fetch::download_gzip_text(&format!("{from}.gz"), MAX_INDEX_BYTES)? {
                    Some(text) => text,
                    None => crate::fetch::download_text(from, MAX_INDEX_BYTES)?,
                };
            let signature = crate::fetch::download_text(&format!("{from}.minisig"), 4096)
                .context("downloading the catalog's signature")?;
            let keys = match std::env::var("WHISTLE_INDEX_PUBKEY") {
                Ok(key) => vec![key],
                Err(_) => TRUSTED_KEYS.iter().map(|k| (*k).to_owned()).collect(),
            };
            verify_signature(text.as_bytes(), &signature, &keys)
                .with_context(|| format!("checking the signature of {from}"))?;
            text
        } else if from.contains("://") {
            bail!("refusing to fetch the index over anything but HTTPS: {from}");
        } else {
            std::fs::read_to_string(from).with_context(|| format!("reading {from}"))?
        };
        let index = Self::parse(&text).with_context(|| format!("loading index from {from}"))?;
        let dest = paths.index_cache();
        if downloaded && let Ok(cached) = Self::from_file(&dest) {
            check_not_older(&index, &cached)?;
        }
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
    /// Apps matching `query`, best first: how well the name matches plus how
    /// popular the app is. A close name match leads, but popularity can
    /// overtake it, so "code" finds Visual Studio Code before a little-used
    /// app called Code, and "office" finds LibreOffice.
    pub fn search(&self, query: &str) -> Vec<&App> {
        let q = query.trim().to_lowercase();
        let mut hits: Vec<(u16, &App)> = self
            .apps
            .iter()
            .filter_map(|app| {
                let name = app.name.to_lowercase();
                let id = app.id.to_lowercase();
                // The app's own part of its id: "code" in com.visualstudio.code,
                // not the publisher's domain.
                let id_tail = id
                    .trim_end_matches(".desktop")
                    .rsplit('.')
                    .next()
                    .unwrap_or("");
                let bonus = if name == q || id == q {
                    400
                } else if name_word_starts_with(&name, &q) || id_tail.starts_with(&q) {
                    300
                } else if name.contains(&q) || id_tail.contains(&q) {
                    250
                } else if app.summary.to_lowercase().contains(&q) {
                    0
                } else {
                    return None;
                };
                Some((app.popularity.min(1000) + bonus, app))
            })
            .collect();
        // Equal scores: apps that install on Linux before Windows-only twins.
        hits.sort_by_cached_key(|(score, app)| {
            (
                std::cmp::Reverse(*score),
                !app.has_linux_source(),
                app.name.to_lowercase(),
            )
        });
        hits.into_iter().map(|(_, app)| app).collect()
    }

    /// The app `query` names: an exact id, an exact name (any case), or the
    /// only app whose name or id contains it.
    pub fn find(&self, query: &str) -> Result<&App> {
        match self.lookup(query) {
            Lookup::One(app) => Ok(app),
            Lookup::None => bail!("no app matches {query:?}; try `whistle search`"),
            Lookup::Many(apps) => bail!("{}", Lookup::describe(query, &apps)),
        }
    }

    /// What `query` refers to: an exact id; an exact name (any case), where
    /// the one app that installs on Linux wins over Windows-only twins; or
    /// the apps whose name or id contains it.
    pub fn lookup(&self, query: &str) -> Lookup<'_> {
        if let Some(app) = self.apps.iter().find(|a| a.id == query) {
            return Lookup::One(app);
        }
        let named: Vec<&App> = self
            .apps
            .iter()
            .filter(|a| a.name.eq_ignore_ascii_case(query))
            .collect();
        let linux: Vec<&App> = named
            .iter()
            .copied()
            .filter(|a| a.has_linux_source())
            .collect();
        match (named.as_slice(), linux.as_slice()) {
            ([app], _) | (_, [app]) => return Lookup::One(app),
            ([], _) => {}
            _ => {
                return Lookup::Many(
                    self.search(query)
                        .into_iter()
                        .filter(|a| named.iter().any(|n| std::ptr::eq(*n, *a)))
                        .collect(),
                );
            }
        }
        let q = query.to_lowercase();
        let partial: Vec<&App> = self
            .search(query)
            .into_iter()
            .filter(|a| a.name.to_lowercase().contains(&q) || a.id.to_lowercase().contains(&q))
            .collect();
        match partial.as_slice() {
            [app] => Lookup::One(app),
            [] => Lookup::None,
            _ => Lookup::Many(partial),
        }
    }
}

/// What a name refers to in the catalog.
#[derive(Debug)]
pub enum Lookup<'a> {
    One(&'a App),
    None,
    /// Several apps, best matches first.
    Many(Vec<&'a App>),
}

impl Lookup<'_> {
    /// "\"blink\" matches several apps: ..." with the first few listed.
    pub fn describe(query: &str, apps: &[&App]) -> String {
        let mut names: Vec<String> = apps
            .iter()
            .take(8)
            .map(|a| format!("{} ({})", a.name, a.id))
            .collect();
        if apps.len() > 8 {
            names.push(format!("and {} more", apps.len() - 8));
        }
        format!("{query:?} matches several apps: {}", names.join(", "))
    }
}

impl App {
    /// Whether some source installs it on Linux, rather than only on Windows
    /// or through a recipe.
    pub fn has_linux_source(&self) -> bool {
        self.sources.iter().any(|s| {
            !matches!(
                s.kind,
                SourceKind::Winget | SourceKind::VendorInstaller | SourceKind::Wsl
            )
        })
    }
}

/// Read a list, leaving out the entries this version can't read, such as a
/// source kind or recipe status added to the server later, instead of
/// rejecting the whole catalog.
fn skip_unreadable<'de, D, T>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let items = Vec::<serde_json::Value>::deserialize(deserializer)?;
    Ok(items
        .into_iter()
        .filter_map(|item| serde_json::from_value(item).ok())
        .collect())
}

/// Fail unless `signature` (a minisign `.minisig` file) signs `data` with one of `keys`.
pub fn verify_signature(data: &[u8], signature: &str, keys: &[String]) -> Result<()> {
    use minisign_verify::{PublicKey, Signature};
    let signature =
        Signature::decode(signature).map_err(|e| anyhow::anyhow!("unreadable signature: {e}"))?;
    for key in keys {
        let key = PublicKey::from_base64(key.trim())
            .map_err(|e| anyhow::anyhow!("invalid public key {key:?}: {e}"))?;
        if key.verify(data, &signature, false).is_ok() {
            return Ok(());
        }
    }
    bail!("the signature doesn't match Whistle's catalog key; refusing this catalog")
}

/// Refuse to go back to an older catalog: replaying an old, validly signed
/// index could bring back entries that have since been fixed.
fn check_not_older(new: &Index, cached: &Index) -> Result<()> {
    // Only real timestamps compare; the dev catalog says "dev".
    let stamped = |s: &str| s.starts_with(|c: char| c.is_ascii_digit());
    if stamped(&new.generated_at)
        && stamped(&cached.generated_at)
        && new.generated_at < cached.generated_at
    {
        bail!(
            "the downloaded catalog ({}) is older than the one you have ({}); refusing it",
            new.generated_at,
            cached.generated_at
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway key, and its signature of `SIGNED`. Not the production key.
    const TEST_KEY: &str = "RWRcxJ29e0CY/r/Y7ii14xOQjyGlzqM4R+hqj9T+9CJegW2KfBa1iiUY";
    const SIGNED: &str = r#"{"format":1,"apps":[]}"#;
    const SIGNATURE: &str = "untrusted comment: signature from minisign secret key
RURcxJ29e0CY/itLjeG9NPEDnsyz73rMXaZel2ei8bqcgjbSywkgskQjv3U3BzWlDWwb3b9JIG5TzD7sEyZz3A9LDxuZLrzHvw0=
trusted comment: whistle test
oD+we1r382ycHPx0yXJ0npPRw/bHuG4CBnct31JezJ2zoO6aa3Y5anDHa/eMRvkMLRr/6WBx8ZkJXWK69kPsBg==
";

    #[test]
    fn signatures_are_checked() {
        let test_key = [TEST_KEY.to_owned()];
        verify_signature(SIGNED.as_bytes(), SIGNATURE, &test_key).unwrap();
        // Tampered data, the wrong key, or garbage are all refused.
        assert!(verify_signature(br#"{"format":1,"apps":[ ]}"#, SIGNATURE, &test_key).is_err());
        let production: Vec<String> = TRUSTED_KEYS.iter().map(|k| (*k).to_owned()).collect();
        assert!(verify_signature(SIGNED.as_bytes(), SIGNATURE, &production).is_err());
        assert!(verify_signature(SIGNED.as_bytes(), "not a signature", &test_key).is_err());
        // Any one of several keys may match, which allows rotating keys.
        let both = [production[0].clone(), TEST_KEY.to_owned()];
        verify_signature(SIGNED.as_bytes(), SIGNATURE, &both).unwrap();
    }

    #[test]
    fn older_catalogs_are_refused() {
        let at = |t: &str| Index {
            format: FORMAT,
            generated_at: t.to_owned(),
            apps: Vec::new(),
        };
        let today = at("2026-11-12T04:00:00Z");
        assert!(check_not_older(&at("2026-11-11T04:00:00Z"), &today).is_err());
        assert!(check_not_older(&at("2026-11-12T04:00:00Z"), &today).is_ok());
        assert!(check_not_older(&at("2026-11-13T04:00:00Z"), &today).is_ok());
        // The dev catalog isn't dated, so it never blocks an update.
        assert!(check_not_older(&today, &at("dev")).is_ok());
    }

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
    fn same_names_prefer_the_linux_app() {
        let index = Index::parse(
            r#"{"format": 1, "apps": [
                {"id": "AG.Blink", "name": "Blink", "sources": [{"kind": "winget", "ref": "AG.Blink", "platforms": ["windows"]}]},
                {"id": "com.icanblink.blink", "name": "Blink", "sources": [{"kind": "flatpak", "ref": "com.icanblink.blink", "platforms": ["linux"]}]},
                {"id": "S.Blink", "name": "Blink", "sources": [{"kind": "winget", "ref": "S.Blink", "platforms": ["windows"]}]},
                {"id": "org.example.BlinkTool", "name": "Blink Tool"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(index.find("blink").unwrap().id, "com.icanblink.blink");
        assert_eq!(index.search("blink")[0].id, "com.icanblink.blink");
        // Without a Linux app among them, it's the user's choice.
        let windows_only = Index::parse(
            r#"{"format": 1, "apps": [
                {"id": "A.Cube", "name": "Cube"}, {"id": "B.Cube", "name": "Cube"}
            ]}"#,
        )
        .unwrap();
        assert!(matches!(windows_only.lookup("cube"), Lookup::Many(apps) if apps.len() == 2));
        assert!(
            matches!(index.lookup("tool"), Lookup::One(app) if app.id == "org.example.BlinkTool")
        );
        assert!(matches!(index.lookup("nothing"), Lookup::None));
    }

    #[test]
    fn search_puts_popular_name_matches_first() {
        let index = Index::parse(
            r#"{"format": 1, "apps": [
                {"id": "io.elementary.code", "name": "Code", "popularity": 300},
                {"id": "org.example.Codec", "name": "Codec Pack", "popularity": 10},
                {"id": "com.visualstudio.code", "name": "Visual Studio Code", "popularity": 990},
                {"id": "org.example.Barcode", "name": "Barcode Maker", "popularity": 999},
                {"id": "org.example.Ide", "name": "IDE", "summary": "Write code", "popularity": 1000}
            ]}"#,
        )
        .unwrap();
        let ids: Vec<&str> = index.search("code").iter().map(|a| a.id.as_str()).collect();
        // 990+300, 999+250, 1000+0, 300+400, 10+300.
        assert_eq!(
            ids,
            [
                "com.visualstudio.code",
                "org.example.Barcode",
                "org.example.Ide",
                "io.elementary.code",
                "org.example.Codec"
            ]
        );
        assert!(name_word_starts_with("visual studio code", "code"));
        assert!(name_word_starts_with("code::blocks", "blocks"));
        assert!(!name_word_starts_with("barcode maker", "code"));
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
    fn skips_entries_from_newer_servers() {
        let index = Index::parse(
            r#"{"format": 1, "apps": [
                {"id": "org.example.A", "name": "A",
                 "sources": [{"kind": "snap", "ref": "a", "platforms": ["linux"]},
                             {"kind": "apt", "ref": "a", "platforms": ["linux"]}],
                 "recipes": [{"id": "r", "runtime": "wine", "status": "experimental", "revision": 1, "spec": {}}],
                 "rating": "gold"}
            ]}"#,
        )
        .unwrap();
        let app = &index.apps[0];
        assert_eq!(app.sources.len(), 1);
        assert_eq!(app.sources[0].kind, SourceKind::Apt);
        assert!(app.recipes.is_empty());
    }

    #[test]
    fn rejects_unknown_format() {
        let err = Index::parse(r#"{"format": 99, "apps": []}"#).unwrap_err();
        assert!(err.to_string().contains("format 99"));
    }
}
