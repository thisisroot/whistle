//! Bring the apps Whistle installed up to date, each the way it was installed.
//!
//! Packages are upgraded by their package manager, in one batch per manager.
//! A Wine app is upgraded when the catalog has a newer revision of its recipe:
//! the recipe runs again on the same prefix, so the app keeps its settings.
//! An AppImage is replaced when the catalog pins a different file.

use std::collections::BTreeMap;

use anyhow::Result;

use crate::backend;
use crate::fetch;
use crate::host::Host;
use crate::index::{App, Index, SourceKind};
use crate::install::{self, Ctx, Options};
use crate::resolve::{Candidate, Prefs, Target, Via, resolve};
use crate::runner::Cmd;
use crate::state::{How, Installed, State};

#[derive(Default)]
pub struct Plan<'a> {
    /// Package-manager upgrades, one batch per manager.
    pub batches: Vec<Batch>,
    /// Apps to install again from a newer recipe revision or AppImage.
    pub reinstalls: Vec<Reinstall<'a>>,
    /// Apps that are left alone, and why.
    pub skipped: Vec<(String, String)>,
}

pub struct Batch {
    pub kind: SourceKind,
    pub apps: Vec<String>,
    pub cmds: Vec<Cmd>,
}

pub struct Reinstall<'a> {
    pub app: &'a App,
    pub candidate: Candidate<'a>,
    /// What changes, for the user: "rev 1 -> rev 2".
    pub change: String,
}

impl Plan<'_> {
    pub fn is_empty(&self) -> bool {
        self.batches.is_empty() && self.reinstalls.is_empty()
    }
}

/// Work out what to upgrade. `only` limits it to the apps with these ids.
pub fn plan<'a>(state: &State, index: &'a Index, host: &Host, only: Option<&[String]>) -> Plan<'a> {
    let mut plan = Plan::default();
    let mut packages: BTreeMap<&'static str, (SourceKind, Vec<String>, Vec<String>)> =
        BTreeMap::new();
    let wanted = |i: &&Installed| only.is_none_or(|ids| ids.contains(&i.app_id));

    for installed in state.installed.iter().filter(wanted) {
        match &installed.how {
            How::Package { kind, reference } => {
                let entry = packages
                    .entry(kind.as_str())
                    .or_insert_with(|| (*kind, Vec::new(), Vec::new()));
                entry.1.push(reference.clone());
                entry.2.push(installed.app_name.clone());
            }
            How::Recipe {
                recipe_id,
                revision,
                runtime,
                ..
            } => {
                let Some(app) = index.apps.iter().find(|a| a.id == installed.app_id) else {
                    plan.skipped.push((
                        installed.app_name.clone(),
                        "no longer in the catalog".into(),
                    ));
                    continue;
                };
                let Some(entry) = app.recipes.iter().find(|r| &r.id == recipe_id) else {
                    plan.skipped
                        .push((installed.app_name.clone(), "its recipe was retired".into()));
                    continue;
                };
                if entry.revision <= *revision {
                    continue;
                }
                // The user already chose this recipe, so a draft revision is fine.
                let prefs = Prefs {
                    via: Some(Via::Recipe(*runtime)),
                    allow_draft: true,
                };
                let candidate = resolve(app, host, &prefs).into_iter().find(
                    |c| matches!(&c.target, Target::Recipe { entry: e, .. } if e.id == *recipe_id),
                );
                match candidate {
                    Some(c) if c.available() => plan.reinstalls.push(Reinstall {
                        app,
                        candidate: c,
                        change: format!("recipe rev {revision} -> rev {}", entry.revision),
                    }),
                    Some(c) => plan.skipped.push((
                        installed.app_name.clone(),
                        format!(
                            "rev {} can't be installed here: {}",
                            entry.revision,
                            c.blocker.unwrap_or_default()
                        ),
                    )),
                    None => plan.skipped.push((
                        installed.app_name.clone(),
                        format!("rev {} can't be installed here", entry.revision),
                    )),
                }
            }
            How::Appimage { path, .. } => {
                let source = index
                    .apps
                    .iter()
                    .find(|a| a.id == installed.app_id)
                    .and_then(|app| {
                        let s = app
                            .sources
                            .iter()
                            .find(|s| s.kind == SourceKind::Appimage)?;
                        Some((app, s))
                    });
                let Some((app, source)) = source else {
                    plan.skipped.push((
                        installed.app_name.clone(),
                        "no AppImage in the catalog".into(),
                    ));
                    continue;
                };
                // A changed or unreadable file is replaced with the pinned one.
                let current = fetch::sha256_file(path).unwrap_or_default();
                if current.eq_ignore_ascii_case(&source.sha256) {
                    continue;
                }
                let prefs = Prefs {
                    via: Some(Via::Source(SourceKind::Appimage)),
                    allow_draft: false,
                };
                if let Some(c) = resolve(app, host, &prefs)
                    .into_iter()
                    .find(|c| c.available())
                {
                    let version = if source.version.is_empty() {
                        "a new version".to_owned()
                    } else {
                        source.version.clone()
                    };
                    plan.reinstalls.push(Reinstall {
                        app,
                        candidate: c,
                        change: format!("AppImage -> {version}"),
                    });
                }
            }
        }
    }

    for (kind, refs, apps) in packages.into_values() {
        match backend::upgrade_cmds(kind, &refs, host) {
            Ok(cmds) => plan.batches.push(Batch { kind, apps, cmds }),
            Err(e) => {
                for app in apps {
                    plan.skipped.push((app, format!("{e:#}")));
                }
            }
        }
    }
    plan
}

/// Run a plan. Keeps going after a failure, and returns the failures.
pub fn apply(ctx: &Ctx, plan: &Plan) -> Vec<(String, anyhow::Error)> {
    let mut failed = Vec::new();
    for batch in &plan.batches {
        for cmd in &batch.cmds {
            if let Err(e) = ctx.runner.run(cmd) {
                failed.push((batch.apps.join(", "), e));
                break;
            }
        }
    }
    for r in &plan.reinstalls {
        let result: Result<_> = install::install(ctx, r.app, &r.candidate, &Options::default());
        if let Err(e) = result {
            failed.push((r.app.name.clone(), e));
        }
    }
    failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{Os, Tool};
    use crate::paths::Paths;
    use crate::runner::DryRunner;

    fn package(app: &str, kind: SourceKind, reference: &str) -> Installed {
        Installed {
            app_id: format!("org.example.{app}"),
            app_name: app.into(),
            installed_at: 0,
            how: How::Package {
                kind,
                reference: reference.into(),
            },
        }
    }

    fn host(os_release: &str, tools: &[Tool]) -> Host {
        Host::from_parts(
            Os::Linux,
            Some(os_release),
            "x86_64",
            tools.iter().copied().collect(),
        )
    }

    #[test]
    fn batches_packages_per_manager() {
        let state = State {
            installed: vec![
                package("Gimp", SourceKind::Apt, "gimp"),
                package("Vlc", SourceKind::Apt, "vlc"),
                package("Spotify", SourceKind::Flatpak, "com.spotify.Client"),
            ],
        };
        let index = Index::builtin();
        let host = host("ID=ubuntu\n", &[Tool::AptGet, Tool::Flatpak, Tool::Sudo]);
        let plan = plan(&state, &index, &host, None);
        let cmds: Vec<String> = plan
            .batches
            .iter()
            .flat_map(|b| b.cmds.iter().map(Cmd::display))
            .collect();
        assert_eq!(
            cmds,
            [
                "sudo apt-get update",
                "sudo apt-get install --only-upgrade -y gimp vlc",
                "flatpak update --user -y --noninteractive com.spotify.Client",
            ]
        );
        assert!(plan.skipped.is_empty() && plan.reinstalls.is_empty());

        let only = ["org.example.Vlc".to_owned()];
        let plan = super::plan(&state, &index, &host, Some(&only));
        assert_eq!(plan.batches.len(), 1);
        assert_eq!(plan.batches[0].apps, ["Vlc"]);
    }

    #[test]
    fn arch_packages_are_skipped_with_advice() {
        let state = State {
            installed: vec![package("Gimp", SourceKind::Pacman, "gimp")],
        };
        let index = Index::builtin();
        let plan = plan(&state, &index, &host("ID=arch\n", &[Tool::Pacman]), None);
        assert!(plan.batches.is_empty());
        assert!(plan.skipped[0].1.contains("pacman -Syu"));
    }

    fn wine_app(revision: u32) -> (State, Index) {
        let mut index = Index::builtin();
        let app = index
            .apps
            .iter_mut()
            .find(|a| a.id == "org.notepad-plus-plus.NotepadPlusPlus")
            .unwrap();
        app.recipes[0].revision = revision;
        let installed = Installed {
            app_id: app.id.clone(),
            app_name: app.name.clone(),
            installed_at: 0,
            how: How::Recipe {
                recipe_id: app.recipes[0].id.clone(),
                revision: 1,
                runtime: app.recipes[0].runtime,
                prefix: "/w/prefixes/npp".into(),
                desktop_files: Vec::new(),
            },
        };
        (
            State {
                installed: vec![installed],
            },
            index,
        )
    }

    #[test]
    fn newer_recipe_revisions_reinstall() {
        let host = host("ID=arch\n", &[Tool::Wine]);
        let (state, index) = wine_app(1);
        assert!(plan(&state, &index, &host, None).is_empty());

        let (state, index) = wine_app(2);
        let plan = plan(&state, &index, &host, None);
        assert_eq!(plan.reinstalls.len(), 1);
        assert_eq!(plan.reinstalls[0].change, "recipe rev 1 -> rev 2");

        let runner = DryRunner::default();
        let paths = Paths::under(std::path::Path::new("/w"));
        let ctx = Ctx {
            host: &host,
            paths: &paths,
            runner: &runner,
        };
        assert!(apply(&ctx, &plan).is_empty());
        assert!(runner.commands().iter().any(|c| c.contains("wineboot")));
    }

    #[test]
    fn newer_recipe_needing_missing_wine_is_skipped() {
        let (state, index) = wine_app(2);
        let plan = plan(&state, &index, &host("ID=arch\n", &[]), None);
        assert!(plan.reinstalls.is_empty());
        assert!(plan.skipped[0].1.contains("wine is not installed"));
    }
}
