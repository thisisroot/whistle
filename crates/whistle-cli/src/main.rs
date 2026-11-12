use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

mod complete;
mod image;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::engine::ArgValueCompleter;
use serde_json::json;
use whistle_core::host::{Host, Tool};
use whistle_core::index::{self, App, Index, IndexOrigin, Lookup, RecipeStatus};
use whistle_core::install::{self, Ctx, Options};
use whistle_core::packages;
use whistle_core::paths::Paths;
use whistle_core::resolve::{Prefs, Target, Via, best, resolve};
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
    /// Print JSON instead of text (search, info, list, categories).
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Find apps in the catalog, and in your distro's packages.
    Search {
        #[arg(required_unless_present = "category")]
        query: Vec<String>,
        #[command(flatten)]
        filter: Filter,
    },
    /// List the app categories and how many apps each has.
    Categories,
    /// Show an app and every way to install it here.
    Info {
        #[arg(add = ArgValueCompleter::new(complete::catalog_apps))]
        app: String,
        /// Treat draft recipes as usable.
        #[arg(long)]
        allow_draft: bool,
    },
    /// Install an app the best available way.
    Install {
        #[arg(add = ArgValueCompleter::new(complete::catalog_apps))]
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
        #[arg(add = ArgValueCompleter::new(complete::installed_apps))]
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
        #[arg(add = ArgValueCompleter::new(complete::installed_apps))]
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
    /// Update Whistle itself to the latest release.
    SelfUpdate {
        #[arg(short, long)]
        yes: bool,
    },
}

/// Which search results to show.
#[derive(clap::Args, Default)]
struct Filter {
    /// Only apps your distro's package manager installs.
    #[arg(long, group = "method")]
    native: bool,
    /// Only Flatpaks.
    #[arg(long, group = "method")]
    flatpak: bool,
    /// Only Windows apps (run with Wine).
    #[arg(long, group = "method")]
    windows: bool,
    /// Only apps in this category, such as Game or Graphics (see `whistle categories`).
    #[arg(long, value_name = "NAME")]
    category: Option<String>,
}

/// The freedesktop main categories `categories` lists.
const MAIN_CATEGORIES: [&str; 10] = [
    "AudioVideo",
    "Development",
    "Education",
    "Game",
    "Graphics",
    "Network",
    "Office",
    "Science",
    "System",
    "Utility",
];

fn main() -> ExitCode {
    // Rust ignores SIGPIPE, so `whistle search x | head` would panic on the
    // closed pipe. Like other command-line tools, just stop instead.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    // Answers the shell's tab-completion requests (COMPLETE=bash whistle ...).
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();
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
        Command::Search { query, filter } => {
            search(&load()?.0, &host, &query.join(" "), &filter, cli.json)
        }
        Command::Categories => categories(&load()?.0, cli.json),
        Command::Info { app, allow_draft } => {
            let (index, _) = load()?;
            let prefs = Prefs {
                via: None,
                allow_draft,
            };
            match choose(index.lookup(&app), &app, false)? {
                Some(found) => info(found, &host, &prefs, cli.json),
                None => {
                    // Not an app in the catalog, but maybe one of the distro's packages.
                    let package =
                        packages::manager(&host).and_then(|kind| packages::find(kind, &app));
                    let (Some(package), Some(mut fallback)) =
                        (package, install::distro_package(&app, &host))
                    else {
                        bail!("no app or package matches {app:?}; try `whistle search`");
                    };
                    fallback.summary = package.summary;
                    info(&fallback, &host, &prefs, cli.json)
                }
            }
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
            // Anything the catalog doesn't know may still be a distro package.
            let fallback;
            let app = match choose(index.lookup(&app), &app, yes)? {
                Some(found) => found,
                None => {
                    fallback = install::distro_package(&app, &host)
                        .with_context(|| format!("no app matches {app:?}; try `whistle search`"))?;
                    println!(
                        "{app:?} isn't in the Whistle catalog; trying your distro's package of that name."
                    );
                    &fallback
                }
            };
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
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&state.installed)?);
                return Ok(());
            }
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
        Command::SelfUpdate { yes } => self_update(yes),
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

/// How many of the distro's packages `search` lists.
const MAX_PACKAGES: usize = 20;

fn search(index: &Index, host: &Host, query: &str, filter: &Filter, json: bool) -> Result<()> {
    let prefs = Prefs::default();
    let category = filter.category.as_deref();
    let mut rows = Vec::new();
    for app in index.search(query) {
        if let Some(category) = category
            && !app
                .categories
                .iter()
                .any(|c| c.eq_ignore_ascii_case(category))
        {
            continue;
        }
        let cands = resolve(app, host, &prefs);
        let chosen = best(&cands);
        let setup = chosen
            .is_none()
            .then(|| install::runtime_setup(app, host, &prefs))
            .flatten();
        let method = chosen.map(|c| c.method().to_owned());
        let keep = match (filter.native, filter.flatpak, filter.windows) {
            (true, ..) => {
                chosen.is_some_and(|c| matches!(&c.target, Target::Source(s) if s.kind.is_native()))
            }
            (_, true, _) => app
                .sources
                .iter()
                .any(|s| s.kind == index::SourceKind::Flatpak),
            (.., true) => !app.recipes.is_empty() && !app.has_linux_source(),
            _ => true,
        };
        if keep {
            rows.push((app, method, setup));
        }
    }
    // The distro's own packages, when looking for something by name.
    let packages = if query.is_empty() || category.is_some() || filter.flatpak || filter.windows {
        None
    } else {
        search_packages(index, host, query)
    };

    if json {
        let apps: Vec<serde_json::Value> = rows
            .iter()
            .map(|(app, method, setup)| {
                json!({
                    "id": app.id,
                    "name": app.name,
                    "summary": app.summary,
                    "categories": app.categories,
                    "icon_url": app.icon_url,
                    "method": method,
                    "needs_setup": setup.as_ref().map(|s| s.name),
                })
            })
            .collect();
        let (manager, found) = packages.unwrap_or((index::SourceKind::Apt, Vec::new()));
        let packages: Vec<serde_json::Value> = found
            .iter()
            .map(|p| json!({"name": p.name, "summary": p.summary, "manager": manager.as_str()}))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"apps": apps, "packages": packages}))?
        );
        return Ok(());
    }

    if rows.is_empty() && packages.as_ref().is_none_or(|(_, found)| found.is_empty()) {
        println!("Nothing matches.");
    }
    let mut needs = std::collections::BTreeSet::new();
    for (app, method, setup) in &rows {
        let how = match (method, setup) {
            (Some(method), _) => method.clone(),
            (None, Some(setup)) => {
                needs.insert(setup.name);
                format!("{}*", setup.name.to_lowercase())
            }
            (None, None) => "unavailable".to_owned(),
        };
        println!(
            "{:<20} {:<40} {:<14} {}",
            fit(&app.name, 20),
            fit(&app.id, 40),
            format!("[{how}]"),
            app.summary
        );
    }
    if !needs.is_empty() {
        let names: Vec<&str> = needs.into_iter().collect();
        println!(
            "\n* Needs {}, which isn't installed yet. `whistle install` can set it up for you.",
            names.join(" or ")
        );
    }
    if let Some((kind, found)) = packages
        && !found.is_empty()
    {
        println!("\nMore packages from {kind}:");
        for package in found.iter().take(MAX_PACKAGES) {
            println!("{:<30} {}", fit(&package.name, 30), package.summary);
        }
        if found.len() > MAX_PACKAGES {
            println!("... and {} more", found.len() - MAX_PACKAGES);
        }
    }
    Ok(())
}

fn categories(index: &Index, json: bool) -> Result<()> {
    let counts: Vec<(&str, usize)> = MAIN_CATEGORIES
        .iter()
        .map(|c| {
            (
                *c,
                index
                    .apps
                    .iter()
                    .filter(|a| a.categories.iter().any(|x| x == c))
                    .count(),
            )
        })
        .collect();
    if json {
        let map: serde_json::Map<String, serde_json::Value> = counts
            .iter()
            .map(|(c, n)| ((*c).to_owned(), json!(n)))
            .collect();
        println!("{}", serde_json::to_string_pretty(&map)?);
        return Ok(());
    }
    for (category, count) in counts {
        println!("{category:<14} {count:>6} apps");
    }
    println!("\n`whistle search --category <name>` lists them.");
    Ok(())
}

/// The distro's packages matching `query` that the catalog doesn't already
/// list as an app's package.
fn search_packages(
    index: &Index,
    host: &Host,
    query: &str,
) -> Option<(index::SourceKind, Vec<packages::Package>)> {
    let kind = packages::manager(host)?;
    let known: std::collections::HashSet<&str> = index
        .apps
        .iter()
        .flat_map(|a| &a.sources)
        .filter(|s| s.kind == kind)
        .map(|s| s.reference.as_str())
        .collect();
    let mut found = packages::search(kind, query);
    found.retain(|p| !known.contains(p.name.as_str()));
    Some((kind, found))
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

fn info(app: &App, host: &Host, prefs: &Prefs, json: bool) -> Result<()> {
    if json {
        let cands = resolve(app, host, prefs);
        let chosen = best(&cands).map(|c| c as *const _);
        let options: Vec<serde_json::Value> = cands
            .iter()
            .map(|c| {
                json!({
                    "method": c.method(),
                    "description": c.to_string(),
                    "blocker": c.blocker,
                    "chosen": Some(c as *const _) == chosen,
                })
            })
            .collect();
        let setup = chosen
            .is_none()
            .then(|| install::runtime_setup(app, host, prefs))
            .flatten();
        let out = json!({
            "id": app.id,
            "name": app.name,
            "summary": app.summary,
            "categories": app.categories,
            "homepage": app.homepage,
            "license": app.license,
            "icon_url": app.icon_url,
            "screenshots": app.screenshots,
            "host": host.describe(),
            "options": options,
            "needs_setup": setup.map(|s| s.name),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    if !app.icon_url.is_empty() {
        image::show_icon(&app.icon_url);
    }
    println!("{} ({})", app.name, app.id);
    if !app.summary.is_empty() {
        println!("{}", app.summary);
    }
    let details = [
        ("Homepage", app.homepage.clone()),
        ("License", app.license.clone()),
        ("Categories", app.categories.join(", ")),
    ];
    let mut blank_line = true;
    for (label, value) in details {
        if !value.is_empty() {
            if blank_line {
                println!();
                blank_line = false;
            }
            println!("{:<12} {value}", format!("{label}:"));
        }
    }
    if let Some(first) = app.screenshots.first() {
        if blank_line {
            println!();
        }
        println!("{:<12} {first}", "Screenshots:");
        for more in &app.screenshots[1..] {
            println!("{:<12} {more}", "");
        }
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
    if chosen.is_none()
        && let Some(setup) = install::runtime_setup(app, host, prefs)
    {
        println!(
            "\n`whistle install {}` can set up {} for you first.",
            app.id, setup.name
        );
    }
    Ok(())
}

fn self_update(yes: bool) -> Result<()> {
    let tag = whistle_core::selfupdate::latest_tag()?;
    if !whistle_core::selfupdate::is_newer(&tag) {
        println!(
            "Whistle {} is the latest version.",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    let exe = std::env::current_exe()
        .context("finding the whistle binary")?
        .canonicalize()?;
    println!(
        "Update Whistle {} to {tag} ({})",
        env!("CARGO_PKG_VERSION"),
        exe.display()
    );
    if !confirm(yes)? {
        return Ok(());
    }
    whistle_core::selfupdate::install(&tag, &exe)?;
    println!("Updated to {tag}.");
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
    // Flatpak or Wine may be the only way to get the app, and not installed yet.
    let setup = install::runtime_setup(app, ctx.host, prefs);
    let setup_cmds = match &setup {
        Some(setup) => install::setup_cmds(setup, ctx.host)
            .with_context(|| format!("{} needs {}, but", app.name, setup.name))?,
        None => Vec::new(),
    };
    let mut host = ctx.host.clone();
    if let Some(setup) = &setup {
        host.tools.extend(setup.tools.iter().map(|(tool, _)| *tool));
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
    if let Target::Recipe { entry, .. } = &chosen.target
        && entry.status == RecipeStatus::Untested
    {
        println!(
            "Note: this recipe was generated automatically and nobody has tried it yet. \
             The app may not work under Wine; Whistle checks that it starts."
        );
    }
    if let Some(setup) = &setup {
        println!("{} isn't installed yet. This sets it up first:", setup.name);
        for cmd in &setup_cmds {
            println!("  {}", cmd.display());
        }
    }
    if !dry && !confirm(yes)? {
        return Ok(());
    }
    for cmd in &setup_cmds {
        ctx.runner.run(cmd)?;
    }
    let installed = install::install(ctx, app, chosen, &Options { user_file: file })?;
    if !dry {
        println!("Installed {} ({}).", app.name, installed.how.describe());
        if setup.as_ref().is_some_and(|s| s.name == "Flatpak") {
            println!(
                "Flatpak is new on this system: log out and back in for its apps to show in your menu."
            );
        }
    }
    Ok(())
}

/// How many apps `choose` offers.
const MAX_CHOICES: usize = 15;

/// The app a lookup settled on. When several match, ask which one, unless
/// there's no one to ask (`-y`, or not a terminal).
fn choose<'a>(lookup: Lookup<'a>, query: &str, yes: bool) -> Result<Option<&'a App>> {
    let apps = match lookup {
        Lookup::One(app) => return Ok(Some(app)),
        Lookup::None => return Ok(None),
        Lookup::Many(apps) => apps,
    };
    if yes || !std::io::stdin().is_terminal() {
        bail!("{}; name one by its id", Lookup::describe(query, &apps));
    }
    println!("Several apps match {query:?}:");
    for (n, app) in apps.iter().take(MAX_CHOICES).enumerate() {
        println!(
            "  {:>2}. {:<24} {:<40} {}",
            n + 1,
            fit(&app.name, 24),
            fit(&app.id, 40),
            fit(&app.summary, 50)
        );
    }
    if apps.len() > MAX_CHOICES {
        println!(
            "  ... and {} more; `whistle search {query}` lists them all",
            apps.len() - MAX_CHOICES
        );
    }
    print!(
        "Which one? [1-{}, Enter to cancel] ",
        apps.len().min(MAX_CHOICES)
    );
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    let answer = answer.trim();
    if answer.is_empty() {
        bail!("cancelled");
    }
    match answer.parse::<usize>() {
        Ok(n) if (1..=apps.len().min(MAX_CHOICES)).contains(&n) => Ok(Some(apps[n - 1])),
        _ => bail!("{answer:?} isn't one of the numbers"),
    }
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
