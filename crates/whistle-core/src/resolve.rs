//! Pick how to install an app on this host: the distro's package first, then
//! Flatpak, AppImage, the Windows installers, and recipes for running apps on
//! the "other" OS.

use std::fmt;
use std::str::FromStr;

use anyhow::bail;

use crate::backend;
use crate::host::{Host, Os, Tool};
use crate::index::{App, RecipeEntry, RecipeStatus, Source, SourceKind};
use crate::recipe::{Recipe, Runtime, Step};

/// What `--via` asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// Whichever of pacman/apt/dnf/zypper fits the distro.
    Native,
    Source(SourceKind),
    Recipe(Runtime),
}

impl FromStr for Via {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        if s == "native" {
            return Ok(Self::Native);
        }
        if let Some(kind) = SourceKind::ALL.into_iter().find(|k| k.as_str() == s) {
            // `wsl` means a recipe run in WSL on Windows; a Source of kind wsl is the same idea.
            if kind != SourceKind::Wsl {
                return Ok(Self::Source(kind));
            }
        }
        match s {
            "wine" => Ok(Self::Recipe(Runtime::Wine)),
            "proton" => Ok(Self::Recipe(Runtime::Proton)),
            "wsl" => Ok(Self::Recipe(Runtime::Wsl)),
            "vm" => Ok(Self::Recipe(Runtime::Vm)),
            _ => bail!("unknown method {s:?}; use native, pacman, apt, dnf, zypper, flatpak, appimage, winget, wine, proton, wsl or vm"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Prefs {
    pub via: Option<Via>,
    pub allow_draft: bool,
}

#[derive(Debug, Clone)]
pub enum Target<'a> {
    Source(&'a Source),
    Recipe { entry: &'a RecipeEntry, recipe: Option<Box<Recipe>> },
}

#[derive(Debug, Clone)]
pub struct Candidate<'a> {
    pub target: Target<'a>,
    /// Why this option can't be used right now; `None` means it can.
    pub blocker: Option<String>,
    rank: (u8, u8),
}

impl Candidate<'_> {
    pub fn available(&self) -> bool {
        self.blocker.is_none()
    }

    /// Short method name, as accepted by `--via`.
    pub fn method(&self) -> &'static str {
        match &self.target {
            Target::Source(s) => s.kind.as_str(),
            Target::Recipe { entry, .. } => entry.runtime.as_str(),
        }
    }
}

impl fmt::Display for Candidate<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.target {
            Target::Source(s) => write!(f, "{}: {}", s.kind, s.reference),
            Target::Recipe { entry, .. } => write!(
                f,
                "{}: recipe {} (rev {}, {})",
                entry.runtime, entry.id, entry.revision, entry.status
            ),
        }
    }
}

/// Every way to get `app` on `host`, best first. Options for other platforms are
/// left out; options that fit but can't run yet carry a blocker.
pub fn resolve<'a>(app: &'a App, host: &Host, prefs: &Prefs) -> Vec<Candidate<'a>> {
    let mut out: Vec<Candidate<'a>> = Vec::new();

    for source in app.sources.iter().filter(|s| host.matches_platforms(&s.platforms)) {
        let tier = match source.kind {
            k if k.is_native() => 0,
            SourceKind::Flatpak => 1,
            SourceKind::Appimage => 2,
            SourceKind::Winget => 3,
            SourceKind::VendorInstaller => 4,
            SourceKind::Wsl => 7,
            _ => unreachable!(),
        };
        let blocker = source_blocker(source, host);
        out.push(Candidate { target: Target::Source(source), blocker, rank: (tier, 0) });
    }

    for entry in app.recipes.iter().filter(|r| r.status != RecipeStatus::Retired) {
        let tier = match (entry.runtime, host.os) {
            (Runtime::Wine, Os::Linux) => 5,
            (Runtime::Proton, Os::Linux) => 6,
            (Runtime::Wsl, Os::Windows) => 7,
            (Runtime::Vm, _) => 8,
            _ => continue,
        };
        let parsed = Recipe::from_value(&entry.spec);
        let blocker = match &parsed {
            Err(e) => Some(format!("this Whistle can't use the recipe ({e})")),
            Ok(recipe) => recipe_blocker(entry, recipe, host, prefs),
        };
        out.push(Candidate {
            target: Target::Recipe { entry, recipe: parsed.ok().map(Box::new) },
            blocker,
            rank: (tier, entry.status as u8),
        });
    }

    if let Some(via) = prefs.via {
        out.retain(|c| match (&c.target, via) {
            (Target::Source(s), Via::Native) => s.kind.is_native(),
            (Target::Source(s), Via::Source(kind)) => s.kind == kind,
            (Target::Recipe { entry, .. }, Via::Recipe(runtime)) => entry.runtime == runtime,
            _ => false,
        });
    }
    // Stable: equal ranks keep index order, which is the catalog's preference.
    out.sort_by_key(|c| (c.blocker.is_some() && prefs.via.is_none(), c.rank));
    out
}

/// The first option that can be used.
pub fn best<'c, 'a>(candidates: &'c [Candidate<'a>]) -> Option<&'c Candidate<'a>> {
    candidates.iter().find(|c| c.available())
}

fn source_blocker(source: &Source, host: &Host) -> Option<String> {
    if !host.matches_arch(&source.arches) {
        return Some(format!("built for {}, this machine is {}", source.arches.join("/"), host.arch));
    }
    match source.kind {
        SourceKind::Appimage => {
            (source.url.is_empty() || source.sha256.is_empty()).then(|| "catalog entry has no pinned download".into())
        }
        SourceKind::VendorInstaller | SourceKind::Wsl => Some("not supported yet".into()),
        kind => {
            let tool = backend::tool_for(kind)?;
            if !host.has(tool) {
                return Some(format!("{} is not installed", tool.binary()));
            }
            if kind.is_native() && !host.is_root && !host.has(Tool::Sudo) {
                return Some("needs sudo, which is not installed".into());
            }
            None
        }
    }
}

fn recipe_blocker(entry: &RecipeEntry, recipe: &Recipe, host: &Host, prefs: &Prefs) -> Option<String> {
    if entry.status == RecipeStatus::Draft && !prefs.allow_draft {
        return Some("draft recipe; pass --allow-draft to try it".into());
    }
    let needs_winetricks = recipe.steps.iter().any(|s| matches!(s, Step::Winetricks(_)));
    match entry.runtime {
        Runtime::Wine if !host.has(Tool::Wine) => Some("wine is not installed".into()),
        Runtime::Wine if needs_winetricks && !host.has(Tool::Winetricks) => Some("winetricks is not installed".into()),
        Runtime::Wine => None,
        Runtime::Proton | Runtime::Wsl | Runtime::Vm => Some(format!("{} recipes are not supported yet", entry.runtime)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Index;
    use std::collections::BTreeSet;

    fn host(os_release: &str, tools: &[Tool]) -> Host {
        Host::from_parts(Os::Linux, Some(os_release), "x86_64", tools.iter().copied().collect::<BTreeSet<_>>())
    }

    fn methods(cands: &[Candidate]) -> Vec<String> {
        cands.iter().map(|c| format!("{}{}", c.method(), if c.available() { "" } else { "!" })).collect()
    }

    #[test]
    fn native_first_then_flatpak() {
        let index = Index::builtin();
        let app = index.find("firefox").unwrap();
        let ubuntu = host("ID=ubuntu\nID_LIKE=debian\n", &[Tool::AptGet, Tool::Flatpak, Tool::Sudo]);
        let cands = resolve(app, &ubuntu, &Prefs::default());
        assert_eq!(methods(&cands), ["apt", "flatpak"]);
        assert_eq!(best(&cands).unwrap().to_string(), "apt: firefox");
    }

    #[test]
    fn unavailable_options_sink_with_a_reason() {
        let index = Index::builtin();
        let app = index.find("firefox").unwrap();
        let arch = host("ID=arch\n", &[Tool::Flatpak, Tool::Sudo]);
        let cands = resolve(app, &arch, &Prefs::default());
        assert_eq!(methods(&cands), ["flatpak", "pacman!"]);
        assert_eq!(cands[1].blocker.as_deref(), Some("pacman is not installed"));
    }

    #[test]
    fn windows_apps_use_wine_recipes_on_linux() {
        let index = Index::builtin();
        let app = index.find("notepad++").unwrap();
        let fedora = host("ID=fedora\n", &[Tool::Dnf, Tool::Wine]);
        let cands = resolve(app, &fedora, &Prefs::default());
        assert_eq!(methods(&cands), ["wine"]);
        let no_wine = host("ID=fedora\n", &[Tool::Dnf]);
        assert_eq!(methods(&resolve(app, &no_wine, &Prefs::default())), ["wine!"]);
    }

    #[test]
    fn drafts_need_opt_in() {
        let index = Index::builtin();
        let office = index.find("com.microsoft.Office").unwrap();
        let h = host("ID=arch\n", &[Tool::Wine]);
        assert!(best(&resolve(office, &h, &Prefs::default())).is_none());
        let prefs = Prefs { allow_draft: true, ..Prefs::default() };
        assert_eq!(resolve(office, &h, &prefs)[0].blocker.as_deref(), Some("winetricks is not installed"));
        let h = host("ID=arch\n", &[Tool::Wine, Tool::Winetricks]);
        assert!(best(&resolve(office, &h, &prefs)).is_some());
    }

    #[test]
    fn via_forces_a_method() {
        let index = Index::builtin();
        let app = index.find("gimp").unwrap();
        let h = host("ID=ubuntu\nID_LIKE=debian\n", &[Tool::AptGet, Tool::Flatpak, Tool::Sudo]);
        let prefs = Prefs { via: Some("flatpak".parse().unwrap()), ..Prefs::default() };
        assert_eq!(methods(&resolve(app, &h, &prefs)), ["flatpak"]);
        let prefs = Prefs { via: Some(Via::Native), ..Prefs::default() };
        assert_eq!(methods(&resolve(app, &h, &prefs)), ["apt"]);
    }

    #[test]
    fn wsl_recipes_only_on_windows() {
        let index = Index::builtin();
        let app = index.find("org.gnome.Builder").unwrap();
        let linux = host("ID=arch\n", &[Tool::Pacman, Tool::Sudo]);
        assert!(!methods(&resolve(app, &linux, &Prefs::default())).contains(&"wsl!".to_owned()));
        let windows = Host::from_parts(Os::Windows, None, "x86_64", BTreeSet::new());
        assert_eq!(methods(&resolve(app, &windows, &Prefs::default())), ["wsl!"]);
    }
}
