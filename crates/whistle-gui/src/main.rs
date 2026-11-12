//! Whistle's app store window. The Svelte frontend in `ui/` calls the
//! commands below; installing, removing and upgrading run in the background
//! and report through the `job` event.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod catalog;
#[cfg(target_os = "linux")]
mod launch;
mod runner;

use std::collections::HashSet;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use tauri::{AppHandle, State};
use whistle_core::host::Host;
use whistle_core::index::{self, Index, IndexOrigin};
use whistle_core::install::{self, Ctx, Options};
use whistle_core::paths::Paths;
use whistle_core::runner::{Cmd, Runner};
use whistle_core::state::{How, State as Installed};
use whistle_core::upgrade;

use catalog::{Card, Catalog, Details, Overview, Results};
use runner::{GuiRunner, sudo_ready};

#[derive(Default)]
struct App {
    catalog: Mutex<Option<Arc<Catalog>>>,
    /// Jobs running now, by app id ("upgrade" for upgrades).
    busy: Mutex<HashSet<String>>,
}

/// Errors reach the window as their message.
type Reply<T> = Result<T, String>;

fn reply<T>(result: Result<T>) -> Reply<T> {
    result.map_err(|e| format!("{e:#}"))
}

impl App {
    /// The catalog, loaded on first use. A missing one is downloaded, and a
    /// stale one refreshed when the network allows.
    fn catalog(&self) -> Result<Arc<Catalog>> {
        let mut slot = self.catalog.lock().unwrap();
        if let Some(catalog) = &*slot {
            return Ok(catalog.clone());
        }
        let paths = Paths::detect()?;
        let index = match Index::load(None, &paths)? {
            (_, IndexOrigin::Builtin) => Index::update(index::DEFAULT_URL, &paths)?,
            (index, IndexOrigin::File(path)) if is_stale(&path) => {
                Index::update(index::DEFAULT_URL, &paths).unwrap_or(index)
            }
            (index, _) => index,
        };
        let catalog = Arc::new(Catalog::new(index, Host::detect()));
        *slot = Some(catalog.clone());
        Ok(catalog)
    }
}

/// The catalog is rebuilt daily; check for a new one a few times a day.
fn is_stale(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_none_or(|age| age > std::time::Duration::from_secs(6 * 60 * 60))
}

#[tauri::command]
async fn overview(app: State<'_, App>) -> Reply<Overview> {
    reply(app.catalog().map(|c| c.overview()))
}

#[tauri::command]
async fn refresh(app: State<'_, App>) -> Reply<Overview> {
    reply((|| {
        let index = Index::update(index::DEFAULT_URL, &Paths::detect()?)?;
        let catalog = Arc::new(Catalog::new(index, Host::detect()));
        *app.catalog.lock().unwrap() = Some(catalog.clone());
        Ok(catalog.overview())
    })())
}

#[tauri::command]
async fn search(
    app: State<'_, App>,
    query: String,
    filter: String,
    category: String,
) -> Reply<Results> {
    reply(app.catalog().map(|c| c.search(&query, &filter, &category)))
}

#[derive(Serialize)]
struct AppPage {
    #[serde(flatten)]
    details: Details,
    /// How it was installed, when Whistle installed it.
    installed: Option<String>,
}

#[tauri::command]
async fn details(app: State<'_, App>, id: String) -> Reply<AppPage> {
    reply((|| {
        let catalog = app.catalog()?;
        let state = Installed::load(&Paths::detect()?.state_file())?;
        Ok(AppPage {
            details: catalog.details(&id)?,
            installed: state.get(&id).map(|i| how_label(&i.how, &catalog)),
        })
    })())
}

/// What the confirmation shows before anything runs.
#[derive(Serialize)]
struct Preview {
    commands: Vec<String>,
    /// Ask for the password: something runs with sudo, and sudo would ask.
    needs_password: bool,
    setup: Option<String>,
    warning: Option<String>,
}

fn needs_password(cmds: &[Cmd]) -> bool {
    cmds.iter().any(|c| c.program == "sudo") && !sudo_ready()
}

#[tauri::command]
async fn preview_install(app: State<'_, App>, id: String, method: String) -> Reply<Preview> {
    reply((|| {
        let catalog = app.catalog()?;
        let plan = catalog.plan(&id, &method)?;
        let cmds = plan.commands()?;
        Ok(Preview {
            commands: cmds.iter().map(|c| c.display()).collect(),
            needs_password: needs_password(&cmds),
            setup: plan.setup.as_ref().map(|s| s.name.to_owned()),
            warning: plan.warning(),
        })
    })())
}

/// Check the password before starting, so a typo is caught up front.
fn check_password(password: &Option<String>) -> Result<()> {
    let Some(password) = password else {
        return Ok(());
    };
    let mut child = Command::new("sudo")
        .args(["-S", "-k", "-v", "-p", ""])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("can't start sudo")?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{password}");
    }
    if !child.wait()?.success() {
        bail!("That password isn't right.");
    }
    Ok(())
}

/// Run `work` in the background as job `job`, reporting how it ends.
fn start(
    handle: AppHandle,
    app: &App,
    job: String,
    password: Option<String>,
    work: impl FnOnce(&GuiRunner) -> Result<String> + Send + 'static,
) -> Result<()> {
    if !app.busy.lock().unwrap().insert(job.clone()) {
        bail!("Already working on it.");
    }
    if let Err(e) = check_password(&password) {
        app.busy.lock().unwrap().remove(&job);
        return Err(e);
    }
    std::thread::spawn(move || {
        let runner = GuiRunner {
            app: handle.clone(),
            job: job.clone(),
            password,
        };
        match work(&runner) {
            Ok(message) => runner.emit("done", message),
            Err(e) => runner.emit("failed", format!("{e:#}")),
        }
        use tauri::Manager;
        handle.state::<App>().busy.lock().unwrap().remove(&job);
    });
    Ok(())
}

#[tauri::command]
async fn install(
    handle: AppHandle,
    app: State<'_, App>,
    id: String,
    method: String,
    password: Option<String>,
) -> Reply<()> {
    reply((|| {
        let catalog = app.catalog()?;
        catalog.plan(&id, &method)?;
        start(handle, &app, id.clone(), password, move |runner| {
            let plan = catalog.plan(&id, &method)?;
            let paths = Paths::detect()?;
            for cmd in &plan.setup_cmds {
                runner.run(cmd)?;
            }
            let ctx = Ctx {
                host: &plan.host,
                paths: &paths,
                runner,
            };
            let done = install::install(&ctx, plan.app, &plan.candidate, &Options::default())?;
            Ok(format!("Installed {}.", done.app_name))
        })
    })())
}

#[tauri::command]
async fn preview_remove(app: State<'_, App>, id: String) -> Reply<Preview> {
    reply((|| {
        let catalog = app.catalog()?;
        let state = Installed::load(&Paths::detect()?.state_file())?;
        let installed = state.get(&id).context("Whistle didn't install this app")?;
        let cmds = match &installed.how {
            How::Package { kind, reference } => {
                whistle_core::backend::remove_cmds(*kind, reference, &catalog.host)?
            }
            _ => Vec::new(),
        };
        Ok(Preview {
            commands: cmds.iter().map(|c| c.display()).collect(),
            needs_password: needs_password(&cmds),
            setup: None,
            warning: None,
        })
    })())
}

#[tauri::command]
async fn remove(
    handle: AppHandle,
    app: State<'_, App>,
    id: String,
    password: Option<String>,
) -> Reply<()> {
    reply((|| {
        let catalog = app.catalog()?;
        start(handle, &app, id.clone(), password, move |runner| {
            let paths = Paths::detect()?;
            let state = Installed::load(&paths.state_file())?;
            let installed = state
                .get(&id)
                .context("Whistle didn't install this app")?
                .clone();
            let ctx = Ctx {
                host: &catalog.host,
                paths: &paths,
                runner,
            };
            install::remove(&ctx, &installed)?;
            Ok(format!("Removed {}.", installed.app_name))
        })
    })())
}

#[tauri::command]
async fn open(app: State<'_, App>, id: String) -> Reply<()> {
    reply((|| {
        let catalog = app.catalog()?;
        let state = Installed::load(&Paths::detect()?.state_file())?;
        let installed = state.get(&id).context("Whistle didn't install this app")?;
        #[cfg(target_os = "linux")]
        return launch::open(installed, catalog.app(&id).ok());
        #[cfg(not(target_os = "linux"))]
        bail!(
            "Opening {} from Whistle isn't supported here yet",
            installed.app_name
        )
    })())
}

#[derive(Serialize)]
struct InstalledApp {
    id: String,
    name: String,
    summary: String,
    icon: String,
    how: String,
    /// "native", "flatpak", "appimage" or "windows", for the badge.
    kind: &'static str,
    /// What a known update changes ("rev 1 -> rev 2").
    update: Option<String>,
}

#[derive(Serialize)]
struct InstalledList {
    apps: Vec<InstalledApp>,
    /// Package managers whose apps an upgrade would refresh.
    managers: Vec<String>,
    preview: Preview,
}

fn how_label(how: &How, catalog: &Catalog) -> String {
    match how {
        How::Package { kind, .. } if kind.is_native() => {
            format!("{} package", catalog::distro_name(&catalog.host))
        }
        How::Package { kind, .. } if *kind == index::SourceKind::Flatpak => "Flatpak".into(),
        How::Package { kind, .. } => kind.to_string(),
        How::Appimage { .. } => "AppImage".into(),
        How::Recipe { .. } => "Windows app".into(),
    }
}

#[tauri::command]
async fn installed(app: State<'_, App>) -> Reply<InstalledList> {
    reply((|| {
        let catalog = app.catalog()?;
        let state = Installed::load(&Paths::detect()?.state_file())?;
        let plan = upgrade::plan(&state, &catalog.index, &catalog.host, None);
        let cmds: Vec<_> = plan.batches.iter().flat_map(|b| b.cmds.clone()).collect();
        let apps = state
            .installed
            .iter()
            .map(|i| {
                let app = catalog.app(&i.app_id).ok();
                let card: Option<&Card> = catalog.card(&i.app_id);
                InstalledApp {
                    id: i.app_id.clone(),
                    name: i.app_name.clone(),
                    summary: app.map(|a| a.summary.clone()).unwrap_or_default(),
                    icon: app.map(|a| a.icon_url.clone()).unwrap_or_default(),
                    how: how_label(&i.how, &catalog),
                    kind: match &i.how {
                        How::Package { kind, .. } if *kind == index::SourceKind::Flatpak => {
                            "flatpak"
                        }
                        How::Package { .. } => "native",
                        How::Appimage { .. } => "appimage",
                        How::Recipe { .. } => "windows",
                    },
                    update: plan
                        .reinstalls
                        .iter()
                        .find(|r| r.app.id == i.app_id)
                        .map(|r| r.change.clone()),
                }
                .with_card(card)
            })
            .collect();
        Ok(InstalledList {
            apps,
            managers: plan.batches.iter().map(|b| b.kind.to_string()).collect(),
            preview: Preview {
                commands: cmds.iter().map(|c| c.display()).collect(),
                needs_password: needs_password(&cmds),
                setup: None,
                warning: None,
            },
        })
    })())
}

impl InstalledApp {
    /// Prefer the catalog's name, which may be newer than the recorded one.
    fn with_card(mut self, card: Option<&Card>) -> Self {
        if let Some(card) = card {
            self.name = card.name.clone();
        }
        self
    }
}

#[tauri::command]
async fn upgrade_all(
    handle: AppHandle,
    app: State<'_, App>,
    password: Option<String>,
) -> Reply<()> {
    reply((|| {
        let catalog = app.catalog()?;
        start(handle, &app, "upgrade".into(), password, move |runner| {
            let paths = Paths::detect()?;
            let state = Installed::load(&paths.state_file())?;
            let plan = upgrade::plan(&state, &catalog.index, &catalog.host, None);
            if plan.is_empty() {
                return Ok("Everything is up to date.".into());
            }
            let ctx = Ctx {
                host: &catalog.host,
                paths: &paths,
                runner,
            };
            let failed = upgrade::apply(&ctx, &plan);
            if let Some((what, e)) = failed.first() {
                bail!("Upgrading {what} failed: {e:#}");
            }
            Ok("Everything is up to date.".into())
        })
    })())
}

fn main() {
    tauri::Builder::default()
        .manage(App::default())
        .invoke_handler(tauri::generate_handler![
            overview,
            refresh,
            search,
            details,
            preview_install,
            install,
            preview_remove,
            remove,
            open,
            installed,
            upgrade_all
        ])
        .run(tauri::generate_context!())
        .expect("error while running Whistle");
}
