use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use whistle_core::host::{Host, Tool};
use whistle_core::index::{self, App, Index, IndexOrigin};
use whistle_core::install::{self, Ctx, Options};
use whistle_core::paths::Paths;
use whistle_core::resolve::{Prefs, Via, best, resolve};
use whistle_core::runner::{DryRunner, RealRunner, Runner};
use whistle_core::state::State;
use whistle_core::upgrade;

/// One app store for Linux and Windows.
#[derive(Parser)]
#[command(name = "whistle", version)]
struct Cli {
    /// Use this catalog index file instead of the cached or built-in one.
    #[arg(long, global = true, value_name = "PATH")]
    index: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Find apps in the catalog.
    Search {
        #[arg(required = true)]
        query: Vec<String>,
    },
    /// Show an app and every way to install it here.
    Info {
        app: String,
        /// Treat draft recipes as usable.
        #[arg(long)]
        allow_draft: bool,
    },
    /// Install an app the best available way.
    Install {
        app: String,
        /// Force a method: native, pacman, apt, dnf, zypper, flatpak, appimage, winget, wine, ...
        #[arg(long, value_name = "METHOD")]
        via: Option<Via>,
        /// Show what would happen without changing anything.
        #[arg(short = 'n', long)]
        dry_run: bool,
        /// Don't ask for confirmation.
        #[arg(short, long)]
        yes: bool,
        /// Installer file, for recipes that need you to supply it.
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
        /// Allow draft recipes.
        #[arg(long)]
        allow_draft: bool,
    },
    /// Remove an app Whistle installed.
    Remove {
        app: String,
        #[arg(short = 'n', long)]
        dry_run: bool,
        #[arg(short, long)]
        yes: bool,
    },
    /// List apps Whistle installed.
    List,
    /// Upgrade apps Whistle installed (all of them, or the ones named).
    Upgrade {
        apps: Vec<String>,
        #[arg(short = 'n', long)]
        dry_run: bool,
        #[arg(short, long)]
        yes: bool,
    },
    /// Download the latest catalog.
    Update {
        /// URL or file to load the index from instead of the default.
        #[arg(long, value_name = "URL|PATH")]
        from: Option<String>,
    },
    /// Show what Whistle knows about this machine.
    Doctor,
}

fn main() -> ExitCode {
    // Rust ignores SIGPIPE, so `whistle search x | head` would panic on the
    // closed pipe. Like other command-line tools, just stop instead.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let paths = Paths::detect()?;
    let host = Host::detect();
    let load = || Index::load(cli.index.as_deref(), &paths);

    match cli.command {
        Command::Search { query } => search(&load()?.0, &host, &query.join(" ")),
        Command::Info { app, allow_draft } => {
            let (index, _) = load()?;
            info(
                index.find(&app)?,
                &host,
                &Prefs {
                    via: None,
                    allow_draft,
                },
            )
        }
        Command::Install {
            app,
            via,
            dry_run,
            yes,
            file,
            allow_draft,
        } => {
            let (index, _) = load()?;
            let app = index.find(&app)?;
            let prefs = Prefs { via, allow_draft };
            let runner = runner(dry_run);
            let ctx = Ctx {
                host: &host,
                paths: &paths,
                runner: runner.as_ref(),
            };
            install_app(&ctx, app, &prefs, file.as_deref(), yes)
        }
        Command::Remove { app, dry_run, yes } => {
            let state = State::load(&paths.state_file())?;
            let found = state.find(&app).or_else(|| {
                let (index, _) = load().ok()?;
                state.get(&index.find(&app).ok()?.id)
            });
            let Some(installed) = found else {
                bail!("{app:?} was not installed by Whistle")
            };
            let runner = runner(dry_run);
            let ctx = Ctx {
                host: &host,
                paths: &paths,
                runner: runner.as_ref(),
            };
            println!(
                "Remove {} ({})",
                installed.app_name,
                installed.how.describe()
            );
            if !dry_run && !confirm(yes)? {
                return Ok(());
            }
            install::remove(&ctx, installed)?;
            if !dry_run {
                println!("Removed {}.", installed.app_name);
            }
            Ok(())
        }
        Command::List => {
            let state = State::load(&paths.state_file())?;
            if state.installed.is_empty() {
                println!("Nothing installed with Whistle yet.");
            }
            for i in &state.installed {
                println!("{:<24} {:<40} {}", i.app_name, i.app_id, i.how.describe());
            }
            Ok(())
        }
        Command::Upgrade { apps, dry_run, yes } => {
            // Like `apt update && apt upgrade`, unless a specific index was asked for.
            if cli.index.is_none()
                && std::env::var_os("WHISTLE_INDEX").is_none()
                && let Err(e) = update(None, &paths)
            {
                eprintln!("warning: couldn't update the catalog, using the one I have: {e:#}");
            }
            let (index, _) = load()?;
            let runner = runner(dry_run);
            let ctx = Ctx {
                host: &host,
                paths: &paths,
                runner: runner.as_ref(),
            };
            upgrade_apps(&ctx, &index, &apps, yes)
        }
        Command::Update { from } => update(from, &paths),
        Command::Doctor => doctor(&host, &paths, load()),
    }
}

fn update(from: Option<String>, paths: &Paths) -> Result<()> {
    let from = from
        .or_else(|| std::env::var("WHISTLE_INDEX_URL").ok())
        .unwrap_or_else(|| index::DEFAULT_URL.to_owned());
    println!("Fetching {from}");
    let index = Index::update(&from, paths)?;
    println!("Catalog updated: {} apps.", index.apps.len());
    Ok(())
}

fn upgrade_apps(ctx: &Ctx, index: &Index, names: &[String], yes: bool) -> Result<()> {
    let state = State::load(&ctx.paths.state_file())?;
    if state.installed.is_empty() {
        println!("Nothing installed with Whistle yet.");
        return Ok(());
    }
    let mut ids = Vec::new();
    for name in names {
        let found = state.find(name).map(|i| i.app_id.clone()).or_else(|| {
            Some(index.find(name).ok()?.id.clone()).filter(|id| state.get(id).is_some())
        });
        match found {
            Some(id) => ids.push(id),
            None => bail!("{name:?} was not installed by Whistle"),
        }
    }
    let only = (!names.is_empty()).then_some(ids.as_slice());
    let plan = upgrade::plan(&state, index, ctx.host, only);

    for (app, why) in &plan.skipped {
        println!("Skip {app}: {why}");
    }
    if plan.is_empty() {
        println!("Nothing to upgrade.");
        return Ok(());
    }
    println!("Upgrade:");
    for batch in &plan.batches {
        println!("  {}: {}", batch.kind, batch.apps.join(", "));
    }
    for r in &plan.reinstalls {
        println!("  {}: {}", r.app.name, r.change);
    }
    let dry = ctx.runner.is_dry();
    if !dry && !confirm(yes)? {
        return Ok(());
    }
    let failed = upgrade::apply(ctx, &plan);
    for (what, e) in &failed {
        eprintln!("error: upgrading {what}: {e:#}");
    }
    if !failed.is_empty() {
        bail!("{} upgrade(s) failed", failed.len());
    }
    if !dry {
        println!("Done.");
    }
    Ok(())
}

fn runner(dry_run: bool) -> Box<dyn Runner> {
    if dry_run {
        Box::new(DryRunner::printing())
    } else {
        Box::new(RealRunner)
    }
}

fn search(index: &Index, host: &Host, query: &str) -> Result<()> {
    let hits = index.search(query);
    if hits.is_empty() {
        println!("No apps match {query:?}.");
    }
    let mut needs_flatpak = false;
    for app in hits {
        let prefs = Prefs::default();
        let cands = resolve(app, host, &prefs);
        let how = match best(&cands) {
            Some(c) => c.method().to_owned(),
            None if install::flatpak_setup(app, host, &prefs).is_some() => {
                needs_flatpak = true;
                "flatpak*".to_owned()
            }
            None => "unavailable".to_owned(),
        };
        println!(
            "{:<20} {:<40} {:<14} {}",
            fit(&app.name, 20),
            fit(&app.id, 40),
            format!("[{how}]"),
            app.summary
        );
    }
    if needs_flatpak {
        println!(
            "\n* Needs Flatpak, which isn't installed. `whistle install` can set it up for you."
        );
    }
    Ok(())
}

/// Shorten `text` to `width` characters, marking the cut with an ellipsis.
fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(width - 1).collect();
    short.push('…');
    short
}

fn info(app: &App, host: &Host, prefs: &Prefs) -> Result<()> {
    println!("{} ({})", app.name, app.id);
    if !app.summary.is_empty() {
        println!("{}", app.summary);
    }
    println!("\nOn {}:", host.describe());
    let cands = resolve(app, host, prefs);
    if cands.is_empty() {
        println!("  No way to install this here yet.");
    }
    let chosen = best(&cands).map(|c| c as *const _);
    for (n, c) in cands.iter().enumerate() {
        let note = match &c.blocker {
            Some(why) => format!("  ({why})"),
            None if Some(c as *const _) == chosen => "  <- will use".to_owned(),
            None => String::new(),
        };
        println!("  {}. {c}{note}", n + 1);
    }
    if chosen.is_none() && install::flatpak_setup(app, host, prefs).is_some() {
        println!(
            "\n`whistle install {}` can set up Flatpak for you first.",
            app.id
        );
    }
    Ok(())
}

fn install_app(
    ctx: &Ctx,
    app: &App,
    prefs: &Prefs,
    file: Option<&std::path::Path>,
    yes: bool,
) -> Result<()> {
    let dry = ctx.runner.is_dry();
    if !dry && let Some(existing) = State::load(&ctx.paths.state_file())?.get(&app.id) {
        bail!(
            "{} is already installed ({}); remove it first",
            app.name,
            existing.how.describe()
        );
    }
    // Flatpak may be the only way to get the app, and not installed yet.
    let setup = install::flatpak_setup(app, ctx.host, prefs);
    let mut host = ctx.host.clone();
    if setup.is_some() {
        host.tools.insert(Tool::Flatpak);
    }
    let ctx = &Ctx {
        host: &host,
        paths: ctx.paths,
        runner: ctx.runner,
    };
    let cands = resolve(app, ctx.host, prefs);
    let Some(chosen) = best(&cands) else {
        let mut msg = format!(
            "no usable way to install {} on {}",
            app.name,
            ctx.host.describe()
        );
        for c in &cands {
            msg.push_str(&format!("\n  {c}: {}", c.blocker.as_deref().unwrap_or("")));
        }
        bail!(msg);
    };
    println!("Install {} with {chosen}", app.name);
    if let Some(setup) = &setup {
        println!("Flatpak isn't installed yet. This sets it up first:");
        for cmd in setup {
            println!("  {}", cmd.display());
        }
    }
    if !dry && !confirm(yes)? {
        return Ok(());
    }
    for cmd in setup.iter().flatten() {
        ctx.runner.run(cmd)?;
    }
    let installed = install::install(ctx, app, chosen, &Options { user_file: file })?;
    if !dry {
        println!("Installed {} ({}).", app.name, installed.how.describe());
        if setup.is_some() {
            println!(
                "Flatpak is new on this system: log out and back in for its apps to show in your menu."
            );
        }
    }
    Ok(())
}

/// Ask before changing the system; `-y` skips the question.
fn confirm(yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        bail!("not a terminal; pass -y to confirm");
    }
    print!("Proceed? [Y/n] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .context("reading the answer")?;
    let ok = matches!(answer.trim().to_lowercase().as_str(), "" | "y" | "yes");
    if !ok {
        println!("Cancelled.");
    }
    Ok(ok)
}

fn doctor(host: &Host, paths: &Paths, index: Result<(Index, IndexOrigin)>) -> Result<()> {
    println!("System:     {}", host.describe());
    println!("Platforms:  {}", host.platform_tags().join(", "));
    println!("CPU:        {}", host.arch);
    if host.in_wsl {
        println!("            running inside WSL");
    }
    let (found, missing): (Vec<Tool>, Vec<Tool>) =
        Tool::ALL.into_iter().partition(|t| host.has(*t));
    let names = |tools: &[Tool]| {
        tools
            .iter()
            .map(|t| t.binary())
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!("Tools:      {}", names(&found));
    println!("Missing:    {}", names(&missing));
    match index {
        Ok((index, origin)) => {
            println!("Catalog:    {} apps from {origin}", index.apps.len());
            if !index.generated_at.is_empty() && index.generated_at != "dev" {
                println!("            generated {}", index.generated_at);
            }
            if matches!(origin, IndexOrigin::Builtin) {
                println!("            run `whistle update` to get the full catalog");
            }
        }
        Err(e) => println!("Catalog:    failed to load: {e:#}"),
    }
    println!("Data:       {}", paths.data.display());
    println!("Cache:      {}", paths.cache.display());
    println!("Menu files: {}", paths.applications.display());
    Ok(())
}
