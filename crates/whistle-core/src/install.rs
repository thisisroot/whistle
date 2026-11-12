//! Carry out a resolved plan and keep the installed-apps record in step.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use crate::backend;
use crate::fetch;
use crate::host::{Host, Tool};
use crate::index::{App, Source, SourceKind};
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
                    let done = wine::install(recipe, ctx.paths, ctx.runner, opts.user_file)?;
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

/// When nothing can install `app` here but a Flatpak could once Flatpak is
/// installed, the commands that install Flatpak.
pub fn flatpak_setup(app: &App, host: &Host, prefs: &Prefs) -> Option<Vec<Cmd>> {
    if host.has(Tool::Flatpak) || best(&resolve(app, host, prefs)).is_some() {
        return None;
    }
    let mut with_flatpak = host.clone();
    with_flatpak.tools.insert(Tool::Flatpak);
    let cands = resolve(app, &with_flatpak, prefs);
    let chosen = best(&cands)?;
    if !matches!(&chosen.target, Target::Source(s) if s.kind == SourceKind::Flatpak) {
        return None;
    }
    backend::setup_flatpak_cmds(host).ok()
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
    fn offers_flatpak_setup_only_when_flatpak_is_the_way() {
        let index = Index::builtin();
        let spotify = index.find("spotify").unwrap();
        let fedora = |tools: &[Tool]| {
            Host::from_parts(
                Os::Linux,
                Some("ID=fedora\n"),
                "x86_64",
                tools.iter().copied().collect(),
            )
        };
        let prefs = Prefs::default();
        let setup = flatpak_setup(spotify, &fedora(&[Tool::Dnf, Tool::Sudo]), &prefs).unwrap();
        let shown: Vec<String> = setup.iter().map(Cmd::display).collect();
        assert_eq!(shown, ["sudo dnf install -y flatpak"]);
        // Already there, or not needed because a native package exists.
        assert!(flatpak_setup(spotify, &fedora(&[Tool::Dnf, Tool::Flatpak]), &prefs).is_none());
        let vlc = index.find("vlc").unwrap();
        assert!(flatpak_setup(vlc, &fedora(&[Tool::Dnf, Tool::Sudo]), &prefs).is_none());
        // No package manager to install Flatpak with.
        assert!(flatpak_setup(spotify, &fedora(&[]), &prefs).is_none());
    }

    #[test]
    fn blocked_candidates_are_refused() {
        let index = Index::builtin();
        let app = index.find("notepad").unwrap();
        let host = Host::from_parts(Os::Linux, Some("ID=arch\n"), "x86_64", Default::default());
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
