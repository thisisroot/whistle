//! The catalog as the window sees it: for every app, the ways it can be
//! installed on this computer, worked out once when the catalog loads.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Serialize;
use whistle_core::backend;
use whistle_core::host::Host;
use whistle_core::index::{App, Index, RecipeStatus, SourceKind};
use whistle_core::install::{self, Setup};
use whistle_core::recipe::Runtime;
use whistle_core::resolve::{Candidate, Prefs, Target, best, resolve};
use whistle_core::runner::Cmd;

/// Apps shown on Discover when this computer can install them.
const POPULAR: &[&str] = &[
    "org.mozilla.firefox",
    "org.gimp.GIMP",
    "org.kde.krita",
    "org.blender.Blender",
    "org.inkscape.Inkscape",
    "org.videolan.VLC",
    "com.obsproject.Studio",
    "org.libreoffice.LibreOffice",
    "org.audacityteam.Audacity",
    "md.obsidian.Obsidian",
    "com.spotify.Client",
    "com.valvesoftware.Steam",
    "com.visualstudio.code",
    "org.telegram.desktop",
    "com.discordapp.Discord",
    "org.kde.kdenlive",
];

/// Main freedesktop categories, and what to call them.
const CATEGORIES: &[(&str, &str)] = &[
    ("Utility", "Utilities"),
    ("Network", "Internet"),
    ("Game", "Games"),
    ("AudioVideo", "Audio & Video"),
    ("Development", "Development"),
    ("Office", "Office"),
    ("Graphics", "Graphics"),
    ("System", "System"),
    ("Education", "Education"),
    ("Science", "Science"),
];

/// Most results one search returns.
const MAX_RESULTS: usize = 300;

/// One way to install an app on this computer.
#[derive(Debug, Clone, Serialize)]
pub struct Way {
    /// What `--via` takes: "apt", "flatpak", "wine", ...
    pub method: String,
    /// "native", "flatpak", "appimage" or "windows".
    pub kind: &'static str,
    /// "Ubuntu package", "Flatpak", "Windows · untested", ...
    pub label: String,
    /// The recipe's status, for Windows apps.
    pub status: Option<String>,
    /// Usable now; otherwise Whistle sets up Flatpak or Wine first.
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Card {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub icon: String,
    /// Best first. Never empty: apps nothing can install here have no card.
    pub ways: Vec<Way>,
}

impl Card {
    fn has(&self, kind: &str) -> bool {
        self.ways.iter().any(|w| w.kind == kind)
    }
}

pub struct Catalog {
    pub index: Index,
    pub host: Host,
    /// Lined up with `index.apps`.
    cards: Vec<Option<Card>>,
    by_id: HashMap<String, usize>,
}

#[derive(Serialize)]
pub struct HostView {
    pub name: String,
    pub arch: String,
    pub tools: Vec<&'static str>,
    pub in_wsl: bool,
}

#[derive(Serialize)]
pub struct Category {
    pub id: &'static str,
    pub label: &'static str,
    pub count: usize,
}

#[derive(Serialize)]
pub struct Featured {
    pub card: Card,
    pub screenshot: String,
}

#[derive(Serialize)]
pub struct Overview {
    pub host: HostView,
    pub generated_at: String,
    pub total: usize,
    pub native: usize,
    pub flatpak: usize,
    pub windows: usize,
    pub featured: Option<Featured>,
    pub popular: Vec<Card>,
    pub windows_verified: Vec<Card>,
    pub categories: Vec<Category>,
}

#[derive(Serialize)]
pub struct Results {
    pub total: usize,
    pub apps: Vec<Card>,
}

/// An install option on the app page.
#[derive(Serialize)]
pub struct Choice {
    #[serde(flatten)]
    pub way: Way,
    pub sub: String,
    /// What Whistle will run, for package managers.
    pub command: Option<String>,
    pub note: String,
    /// Worth a warning: an untested recipe.
    pub warn: bool,
    /// What Whistle sets up first, when the option isn't ready.
    pub needs: Option<String>,
}

#[derive(Serialize)]
pub struct Details {
    pub card: Card,
    pub homepage: String,
    pub license: String,
    pub categories: Vec<String>,
    pub screenshots: Vec<String>,
    pub choices: Vec<Choice>,
}

/// What installing with one method will do.
pub struct Plan<'a> {
    pub app: &'a App,
    pub host: Host,
    pub setup: Option<Setup>,
    pub setup_cmds: Vec<Cmd>,
    pub candidate: Candidate<'a>,
}

impl Plan<'_> {
    /// Every command it runs that's known up front (recipes run more).
    pub fn commands(&self) -> Result<Vec<Cmd>> {
        let mut cmds = self.setup_cmds.clone();
        if let Target::Source(source) = &self.candidate.target
            && source.kind != SourceKind::Appimage
        {
            cmds.extend(backend::install_cmds(
                source.kind,
                &source.reference,
                &self.host,
            )?);
        }
        Ok(cmds)
    }

    pub fn warning(&self) -> Option<String> {
        match &self.candidate.target {
            Target::Recipe { entry, .. } if entry.status == RecipeStatus::Untested => Some(
                "This recipe was generated from winget and nobody has tried it yet. \
                 Whistle starts the app once after installing to check that it runs."
                    .into(),
            ),
            _ => None,
        }
    }
}

impl Catalog {
    pub fn new(index: Index, host: Host) -> Self {
        let prefs = Prefs::default();
        let cards = index
            .apps
            .iter()
            .map(|app| {
                let ways: Vec<Way> = resolve(app, &host, &prefs)
                    .iter()
                    .map(|c| way(c, &host))
                    .collect();
                (!ways.is_empty()).then(|| Card {
                    id: app.id.clone(),
                    name: app.name.clone(),
                    summary: app.summary.clone(),
                    icon: app.icon_url.clone(),
                    ways,
                })
            })
            .collect();
        let by_id = index
            .apps
            .iter()
            .enumerate()
            .map(|(n, app)| (app.id.clone(), n))
            .collect();
        Self {
            index,
            host,
            cards,
            by_id,
        }
    }

    pub fn app(&self, id: &str) -> Result<&App> {
        let n = self
            .by_id
            .get(id)
            .with_context(|| format!("no app {id:?} in the catalog"))?;
        Ok(&self.index.apps[*n])
    }

    pub fn card(&self, id: &str) -> Option<&Card> {
        self.cards[*self.by_id.get(id)?].as_ref()
    }

    fn available(&self) -> impl Iterator<Item = (&App, &Card)> {
        self.index
            .apps
            .iter()
            .zip(&self.cards)
            .filter_map(|(app, card)| Some((app, card.as_ref()?)))
    }

    pub fn overview(&self) -> Overview {
        let cards: Vec<&Card> = self.available().map(|(_, c)| c).collect();
        let popular: Vec<Card> = POPULAR
            .iter()
            .filter_map(|id| self.card(id).cloned())
            .collect();
        // A different featured app each day, from the popular ones with a screenshot.
        let with_shots: Vec<(&Card, &str)> = popular
            .iter()
            .filter_map(|c| Some((c, self.app(&c.id).ok()?.screenshots.first()?.as_str())))
            .collect();
        let day = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() / 86_400) as usize;
        let featured = (!with_shots.is_empty()).then(|| {
            let (card, shot) = with_shots[day % with_shots.len()];
            Featured {
                card: card.clone(),
                screenshot: shot.to_owned(),
            }
        });
        let windows_verified = self
            .available()
            .filter(|(app, card)| {
                card.has("windows")
                    && !app.has_linux_source()
                    && app.recipes.iter().any(|r| {
                        matches!(r.status, RecipeStatus::Verified | RecipeStatus::Community)
                    })
            })
            .map(|(_, c)| c.clone())
            .collect();
        let categories = CATEGORIES
            .iter()
            .map(|(id, label)| Category {
                id,
                label,
                count: self
                    .available()
                    .filter(|(app, _)| app.categories.iter().any(|c| c == id))
                    .count(),
            })
            .collect();
        Overview {
            host: HostView {
                name: self.host.describe(),
                arch: self.host.arch.clone(),
                tools: self.host.tools.iter().map(|t| t.binary()).collect(),
                in_wsl: self.host.in_wsl,
            },
            generated_at: self.index.generated_at.clone(),
            total: cards.len(),
            native: cards.iter().filter(|c| c.has("native")).count(),
            flatpak: cards.iter().filter(|c| c.has("flatpak")).count(),
            windows: cards.iter().filter(|c| c.has("windows")).count(),
            featured,
            popular,
            windows_verified,
            categories,
        }
    }

    /// Apps matching `query` (all of them when it's empty) that this computer
    /// can install, narrowed to one way of installing and one category.
    pub fn search(&self, query: &str, filter: &str, category: &str) -> Results {
        let query = query.trim();
        let apps: Vec<&App> = if query.is_empty() {
            let mut all: Vec<&App> = self.index.apps.iter().collect();
            all.sort_by_cached_key(|a| (!a.has_linux_source(), a.name.to_lowercase()));
            all
        } else {
            self.index.search(query)
        };
        let matching: Vec<&Card> = apps
            .into_iter()
            .filter(|app| category.is_empty() || app.categories.iter().any(|c| c == category))
            .filter_map(|app| self.card(&app.id))
            .filter(|card| filter == "all" || card.has(filter))
            .collect();
        Results {
            total: matching.len(),
            apps: matching.into_iter().take(MAX_RESULTS).cloned().collect(),
        }
    }

    pub fn details(&self, id: &str) -> Result<Details> {
        let app = self.app(id)?;
        let card = self
            .card(id)
            .cloned()
            .with_context(|| format!("{} can't be installed on this computer", app.name))?;
        let distro = distro_name(&self.host);
        let choices = resolve(app, &self.host, &Prefs::default())
            .iter()
            .map(|c| {
                let way = way(c, &self.host);
                let needs = (!way.ready)
                    .then(|| {
                        let prefs = Prefs {
                            via: way.method.parse().ok(),
                            ..Prefs::default()
                        };
                        install::runtime_setup(app, &self.host, &prefs).map(|s| s.name.to_owned())
                    })
                    .flatten();
                let (sub, note, command) = match &c.target {
                    Target::Source(s) => {
                        let command = backend::install_cmds(s.kind, &s.reference, &self.host)
                            .ok()
                            .map(|cmds| {
                                cmds.iter().map(Cmd::display).collect::<Vec<_>>().join(" && ")
                            });
                        let (sub, note) = match s.kind {
                            SourceKind::Flatpak => (
                                "Flathub · sandboxed".to_owned(),
                                "The same version on every distro, in a sandbox.".to_owned(),
                            ),
                            SourceKind::Appimage => (
                                "AppImage · one file".to_owned(),
                                "A pinned download, checked against its checksum.".to_owned(),
                            ),
                            kind => (
                                format!("{kind} · updates with your system"),
                                format!(
                                    "Installed and updated by {distro}, like the rest of your system."
                                ),
                            ),
                        };
                        (sub, note, command)
                    }
                    Target::Recipe { entry, .. } => {
                        let note = match entry.status {
                            RecipeStatus::Verified => {
                                "A maintainer installed and launched it with Wine."
                            }
                            RecipeStatus::Community => "Reviewed, and reported working.",
                            RecipeStatus::Untested => {
                                "Generated from winget and not tried by anyone yet. Whistle \
                                 starts it once after installing to check that it runs."
                            }
                            RecipeStatus::Draft | RecipeStatus::Retired => {
                                "Not reviewed yet."
                            }
                        };
                        (
                            format!("Wine · {} recipe", entry.status),
                            note.to_owned(),
                            None,
                        )
                    }
                };
                Choice {
                    warn: way.status.as_deref() == Some("untested"),
                    way,
                    sub,
                    command,
                    note,
                    needs,
                }
            })
            .collect();
        Ok(Details {
            card,
            homepage: app.homepage.clone(),
            license: app.license.clone(),
            categories: app.categories.clone(),
            screenshots: app.screenshots.clone(),
            choices,
        })
    }

    /// What installing `id` with `method` will do, including setting up
    /// Flatpak or Wine when it needs them.
    pub fn plan(&self, id: &str, method: &str) -> Result<Plan<'_>> {
        let app = self.app(id)?;
        let prefs = Prefs {
            via: Some(method.parse()?),
            ..Prefs::default()
        };
        let setup = install::runtime_setup(app, &self.host, &prefs);
        let setup_cmds = match &setup {
            Some(setup) => install::setup_cmds(setup, &self.host)
                .with_context(|| format!("{} needs {}, but", app.name, setup.name))?,
            None => Vec::new(),
        };
        let mut host = self.host.clone();
        if let Some(setup) = &setup {
            host.tools.extend(setup.tools.iter().map(|(tool, _)| *tool));
        }
        let cands = resolve(app, &host, &prefs);
        let candidate = match best(&cands) {
            Some(c) => c.clone(),
            None => anyhow::bail!(
                "{} can't be installed this way: {}",
                app.name,
                cands
                    .first()
                    .and_then(|c| c.blocker.clone())
                    .unwrap_or_else(|| "no such option here".into())
            ),
        };
        Ok(Plan {
            app,
            host,
            setup,
            setup_cmds,
            candidate,
        })
    }
}

/// "Ubuntu", "Fedora", "Arch": the first word of the distro's name.
pub fn distro_name(host: &Host) -> String {
    host.describe()
        .split_whitespace()
        .next()
        .unwrap_or("Linux")
        .to_owned()
}

fn way(c: &Candidate, host: &Host) -> Way {
    let (kind, label, status) = match &c.target {
        Target::Source(s) => match s.kind {
            SourceKind::Flatpak => ("flatpak", "Flatpak".to_owned(), None),
            SourceKind::Appimage => ("appimage", "AppImage".to_owned(), None),
            SourceKind::Winget => ("native", "winget".to_owned(), None),
            _ => ("native", format!("{} package", distro_name(host)), None),
        },
        Target::Recipe { entry, .. } => {
            let kind = match entry.runtime {
                Runtime::Wine | Runtime::Proton => "windows",
                Runtime::Wsl | Runtime::Vm => "native",
            };
            (
                kind,
                format!("Windows · {}", entry.status),
                Some(entry.status.to_string()),
            )
        }
    };
    Way {
        method: c.method().to_owned(),
        kind,
        label,
        status,
        ready: c.available(),
    }
}
