//! Carry out a resolved plan and keep the installed-apps record in step.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use crate::backend;
use crate::fetch;
use crate::host::{Host, Tool};
use crate::index::{App, RecipeStatus, Source, SourceKind};
use crate::paths::Paths;
use crate::recipe::Runtime;
use crate::resolve::{Candidate, Prefs, Target, best, resolve};
use crate::runner::{Cmd, Runner};
use crate::state::{How, Installed, State};
use crate::wine;

pub struct Ctx<'a> {
    pub host: &'a Host,
    pub paths: &'a Paths,
    pub runner: &'a dyn Runner,
}

#[derive(Default)]
pub struct Options<'a> {
    /// The installer file for recipes whose installer the user must supply.
    pub user_file: Option<&'a Path>,
}

pub fn install(ctx: &Ctx, app: &App, candidate: &Candidate, opts: &Options) -> Result<Installed> {
    if let Some(why) = &candidate.blocker {
        bail!("can't install {} with {candidate}: {why}", app.name);
    }
    let how = match &candidate.target {
        Target::Source(source) if source.kind == SourceKind::Appimage => {
            install_appimage(ctx, app, source)?
        }
        Target::Source(source) => {
            for cmd in backend::install_cmds(source.kind, &source.reference, ctx.host)? {
                ctx.runner.run(&cmd)?;
            }
            How::Package {
                kind: source.kind,
                reference: source.reference.clone(),
            }
        }
        Target::Recipe { entry, recipe } => {
            let recipe = recipe.as_ref().context("the recipe could not be parsed")?;
            match entry.runtime {
                Runtime::Wine => {
                    let found = crate::requirements::check(&recipe.requires, &ctx.paths.data);
                    for warning in &found.warnings {
                        ctx.runner
                            .note(&format!("Warning: {} {warning}.", app.name));
                    }
                    if !found.problems.is_empty() {
                        bail!("{} {}", app.name, found.problems.join("; it also "));
                    }
                    let done = wine::install(recipe, ctx.paths, ctx.runner, opts.user_file)?;
                    // Untested recipes are always checked; others when they ask for it.
                    if entry.status == RecipeStatus::Untested || recipe.health_check.is_some() {
                        check_health(ctx, app, recipe, &done);
                    }
                    How::Recipe {
                        recipe_id: entry.id.clone(),
                        revision: entry.revision,
                        runtime: entry.runtime,
                        prefix: done.prefix,
                        desktop_files: done.desktop_files,
                    }
                }
                other => bail!("{other} recipes are not supported yet"),
            }
        }
    };
    let installed = Installed {
        app_id: app.id.clone(),
        app_name: app.name.clone(),
        installed_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        how,
    };
    if !ctx.runner.is_dry() {
        let path = ctx.paths.state_file();
        let mut state = State::load(&path)?;
        state.record(installed.clone());
        state.save(&path)?;
    }
    Ok(installed)
}

/// Start the app once and tell the user whether it kept running.
fn check_health(ctx: &Ctx, app: &App, recipe: &crate::recipe::Recipe, done: &wine::WineInstall) {
    if ctx.runner.is_dry() {
        ctx.runner.note("start the app once to check it runs");
        return;
    }
    let wanted = recipe.health_check.as_ref();
    let launcher = wanted
        .and_then(|h| done.launchers.iter().find(|l| l.name == h.launcher))
        .or_else(|| done.launchers.first());
    let Some(launcher) = launcher else {
        ctx.runner
            .note("Health check skipped: the app has no menu entry to start.");
        return;
    };
    let wait = Duration::from_secs(wanted.and_then(|h| h.timeout_s).unwrap_or(15));
    ctx.runner.note(&format!(
        "Checking that {} starts (it opens for {} seconds)...",
        launcher.name,
        wait.as_secs()
    ));
    match wine::health_check(done, launcher, wait) {
        Ok(wine::Health::Running) => ctx.runner.note(&format!("{} started fine.", launcher.name)),
        Ok(wine::Health::Exited) => ctx.runner.note(&format!(
            "Warning: {} closed right after starting, so it may not work under Wine. \
             `whistle remove {}` removes it again.",
            launcher.name, app.id
        )),
        Ok(wine::Health::Skipped(why)) => ctx.runner.note(&format!("Health check skipped: {why}.")),
        Err(e) => ctx
            .runner
            .note(&format!("Health check failed to run: {e:#}")),
    }
}

/// For a name that isn't in the catalog: an app whose only source is the
/// package of that name in this distro's own package manager.
pub fn distro_package(name: &str, host: &Host) -> Option<App> {
    let kind = [
        (Tool::Pacman, SourceKind::Pacman),
        (Tool::AptGet, SourceKind::Apt),
        (Tool::Dnf, SourceKind::Dnf),
        (Tool::Zypper, SourceKind::Zypper),
    ]
    .into_iter()
    .find(|(tool, _)| host.has(*tool))?
    .1;
    backend::install_cmds(kind, name, host).ok()?;
    Some(App {
        id: name.to_owned(),
        name: name.to_owned(),
        summary: String::new(),
        categories: Vec::new(),
        icon_url: String::new(),
        screenshots: Vec::new(),
        homepage: String::new(),
        license: String::new(),
        sources: vec![Source {
            kind,
            reference: name.to_owned(),
            platforms: vec!["linux".to_owned()],
            arches: Vec::new(),
            version: String::new(),
            url: String::new(),
            sha256: String::new(),
        }],
        recipes: Vec::new(),
    })
}

/// A runtime Whistle can install with the distro's package manager when an
/// app needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    /// "Flatpak" or "Wine", for messages.
    pub name: &'static str,
    /// The tools it adds, and the distro package that provides each.
    pub tools: Vec<(Tool, &'static str)>,
}

impl Setup {
    pub fn packages(&self) -> Vec<&'static str> {
        self.tools.iter().map(|(_, package)| *package).collect()
    }
}

/// When nothing can install `app` here yet, but installing Flatpak or Wine
/// would: what to install. Flatpak first, as the resolver prefers it.
pub fn runtime_setup(app: &App, host: &Host, prefs: &Prefs) -> Option<Setup> {
    crate::packages::manager(host)?;
    if best(&resolve(app, host, prefs)).is_some() {
        return None;
    }
    let options = [
        ("Flatpak", vec![(Tool::Flatpak, "flatpak")]),
        ("Wine", vec![(Tool::Wine, "wine")]),
        (
            "Wine",
            vec![(Tool::Wine, "wine"), (Tool::Winetricks, "winetricks")],
        ),
    ];
    for (name, tools) in options {
        let missing: Vec<(Tool, &'static str)> =
            tools.into_iter().filter(|(t, _)| !host.has(*t)).collect();
        if missing.is_empty() {
            continue;
        }
        let mut with = host.clone();
        with.tools.extend(missing.iter().map(|(t, _)| *t));
        if best(&resolve(app, &with, prefs)).is_some() {
            return Some(Setup {
                name,
                tools: missing,
            });
        }
    }
    None
}

/// The commands that install `setup`, once its packages are known to be in
/// the distro's enabled repositories.
pub fn setup_cmds(setup: &Setup, host: &Host) -> Result<Vec<Cmd>> {
    if let Some(kind) = crate::packages::manager(host) {
        for package in setup.packages() {
            if crate::packages::find(kind, package).is_none() {
                bail!(
                    "{package} isn't in your distro's enabled repositories{}",
                    if kind == SourceKind::Apt {
                        " (on Debian it's in the contrib section)"
                    } else {
                        ""
                    }
                );
            }
        }
    }
    backend::setup_cmds(host, &setup.packages())
}

pub fn remove(ctx: &Ctx, installed: &Installed) -> Result<()> {
    match &installed.how {
        How::Package { kind, reference } => {
            for cmd in backend::remove_cmds(*kind, reference, ctx.host)? {
                ctx.runner.run(&cmd)?;
            }
        }
        How::Appimage { path, desktop_file } => {
            if !path.starts_with(ctx.paths.appimages()) {
                bail!(
                    "refusing to delete {}: not a Whistle AppImage",
                    path.display()
                );
            }
            wine::remove_path(ctx.runner, desktop_file)?;
            wine::remove_path(ctx.runner, path)?;
        }
        How::Recipe {
            runtime: Runtime::Wine,
            prefix,
            desktop_files,
            ..
        } => {
            wine::uninstall(ctx.paths, prefix, desktop_files, ctx.runner)?;
        }
        How::Recipe { runtime, .. } => bail!("{runtime} recipes are not supported yet"),
    }
    if !ctx.runner.is_dry() {
        let path = ctx.paths.state_file();
        let mut state = State::load(&path)?;
        state.forget(&installed.app_id);
        state.save(&path)?;
    }
    Ok(())
}

fn install_appimage(ctx: &Ctx, app: &App, source: &Source) -> Result<How> {
    let dir = ctx.paths.appimages();
    let name = format!("{}.AppImage", app.id);
    let path = dir.join(&name);
    if ctx.runner.is_dry() {
        ctx.runner.note(&format!(
            "download {} and check its SHA-256 is {}",
            source.url, source.sha256
        ));
    } else {
        fetch::download_verified(&source.url, &source.sha256, &dir, &name)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    let desktop_file = ctx
        .paths
        .applications
        .join(format!("whistle-{}.desktop", app.id));
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment={}\nExec=\"{}\" %U\nIcon=application-x-executable\nX-Whistle-App={}\n",
        app.name.replace('\n', " "),
        app.summary.replace('\n', " "),
        path.display()
            .to_string()
            .replace('\\', "\\\\\\\\")
            .replace('"', "\\\\\"")
            .replace('%', "%%"),
        app.id,
    );
    wine::write_file(ctx.runner, &desktop_file, &entry)?;
    Ok(How::Appimage { path, desktop_file })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{Os, Tool};
    use crate::index::Index;
    use crate::runner::DryRunner;

    #[test]
    fn dry_install_runs_the_package_manager_and_records_nothing() {
        let index = Index::builtin();
        let app = index.find("vlc").unwrap();
        let host = Host::from_parts(
            Os::Linux,
            Some("ID=fedora\n"),
            "x86_64",
            [Tool::Dnf, Tool::Sudo].into(),
        );
        let root = std::env::temp_dir().join(format!("whistle-install-{}", std::process::id()));
        let paths = Paths::under(&root);
        let runner = DryRunner::default();
        let ctx = Ctx {
            host: &host,
            paths: &paths,
            runner: &runner,
        };

        let cands = resolve(app, &host, &Prefs::default());
        let installed = install(&ctx, app, best(&cands).unwrap(), &Options::default()).unwrap();
        assert_eq!(runner.commands(), ["sudo dnf install -y vlc"]);
        assert_eq!(installed.how.describe(), "dnf: vlc");
        assert!(!paths.state_file().exists());

        remove(&ctx, &installed).unwrap();
        assert_eq!(runner.commands()[1], "sudo dnf remove -y vlc");
    }

    #[test]
    fn offers_the_runtime_an_app_needs() {
        let index = Index::builtin();
        let fedora = |tools: &[Tool]| {
            Host::from_parts(
                Os::Linux,
                Some("ID=fedora\n"),
                "x86_64",
                tools.iter().copied().collect(),
            )
        };
        let prefs = Prefs::default();
        let spotify = index.find("spotify").unwrap();
        let setup = runtime_setup(spotify, &fedora(&[Tool::Dnf, Tool::Sudo]), &prefs).unwrap();
        assert_eq!((setup.name, setup.packages()), ("Flatpak", vec!["flatpak"]));
        // Already there, or not needed because a native package exists.
        assert!(runtime_setup(spotify, &fedora(&[Tool::Dnf, Tool::Flatpak]), &prefs).is_none());
        let vlc = index.find("vlc").unwrap();
        assert!(runtime_setup(vlc, &fedora(&[Tool::Dnf, Tool::Sudo]), &prefs).is_none());
        // No package manager to install anything with.
        assert!(runtime_setup(spotify, &fedora(&[]), &prefs).is_none());

        // A Windows app needs Wine; winetricks only if a step uses it.
        let notepad = index.find("notepad").unwrap();
        let setup = runtime_setup(notepad, &fedora(&[Tool::Dnf, Tool::Sudo]), &prefs).unwrap();
        assert_eq!((setup.name, setup.packages()), ("Wine", vec!["wine"]));
        let shown: Vec<String> =
            backend::setup_cmds(&fedora(&[Tool::Dnf, Tool::Sudo]), &setup.packages())
                .unwrap()
                .iter()
                .map(Cmd::display)
                .collect();
        assert_eq!(shown, ["sudo dnf install -y wine"]);
    }

    #[test]
    fn names_outside_the_catalog_use_the_distro_package() {
        let host = Host::from_parts(
            Os::Linux,
            Some("ID=debian\n"),
            "x86_64",
            [Tool::AptGet, Tool::Sudo].into(),
        );
        let app = distro_package("htop", &host).unwrap();
        let cands = resolve(&app, &host, &Prefs::default());
        assert_eq!(best(&cands).unwrap().to_string(), "apt: htop");
        assert!(distro_package("--force", &host).is_none());
        let bare = Host::from_parts(Os::Linux, Some("ID=debian\n"), "x86_64", [].into());
        assert!(distro_package("htop", &bare).is_none());
    }

    #[test]
    fn blocked_candidates_are_refused() {
        let index = Index::builtin();
        let app = index.find("notepad").unwrap();
        let host = Host::from_parts(
            Os::Linux,
            Some("ID=arch\n"),
            "x86_64",
            [Tool::Pacman].into_iter().collect(),
        );
        let paths = Paths::under(Path::new("/w"));
        let runner = DryRunner::default();
        let ctx = Ctx {
            host: &host,
            paths: &paths,
            runner: &runner,
        };
        let cands = resolve(app, &host, &Prefs::default());
        let err = install(&ctx, app, &cands[0], &Options::default()).unwrap_err();
        assert!(err.to_string().contains("wine is not installed"));
    }
}
