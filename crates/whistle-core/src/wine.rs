//! Runs a `runtime: wine` recipe: one Wine prefix per recipe, typed steps,
//! then menu entries for each launcher.

use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::fetch;
use crate::paths::Paths;
use crate::recipe::{Installer, Launcher, Recipe, RegValue, Runtime, Step, WinArch, reg_root};
use crate::runner::{Cmd, Runner};

const DEFAULT_INSTALLER_TIMEOUT: u64 = 1800;

/// Start Menu shortcuts that aren't the app itself.
const NOT_LAUNCHERS: &[&str] = &[
    "uninstall",
    "remove",
    "readme",
    "read me",
    "website",
    "web site",
    "homepage",
    "help",
    "manual",
    "documentation",
    "license",
    "changelog",
    "release notes",
];

#[derive(Debug)]
pub struct WineInstall {
    pub prefix: PathBuf,
    pub desktop_files: Vec<PathBuf>,
    /// The programs that got menu entries, and the environment they run with.
    pub launchers: Vec<Launcher>,
    pub launcher_env: Vec<(String, String)>,
}

/// Programs every prefix runs that aren't the app.
const WINE_PROGRAMS: &[&str] = &[
    "conhost.exe",
    "explorer.exe",
    "mscorsvw.exe",
    "plugplay.exe",
    "rpcss.exe",
    "rundll32.exe",
    "services.exe",
    "start.exe",
    "svchost.exe",
    "tabtip.exe",
    "wineboot.exe",
    "winedevice.exe",
    "winemenubuilder.exe",
];

pub enum Health {
    /// The app was still running when checked.
    Running,
    /// The app had exited by then.
    Exited,
    /// The check couldn't run, and why.
    Skipped(String),
}

pub fn prefix_for(paths: &Paths, recipe_id: &str) -> PathBuf {
    paths.prefixes().join(recipe_id)
}

pub fn install(
    recipe: &Recipe,
    paths: &Paths,
    runner: &dyn Runner,
    user_file: Option<&Path>,
) -> Result<WineInstall> {
    if recipe.runtime != Runtime::Wine {
        bail!(
            "{} is a {} recipe, not a Wine one",
            recipe.id,
            recipe.runtime
        );
    }
    recipe.check()?;
    let media = acquire(recipe, paths, runner, user_file)?;
    let prefix = prefix_for(paths, &recipe.id);

    let mut env = vec![
        ("WINEPREFIX".to_owned(), prefix.display().to_string()),
        ("WINEDEBUG".to_owned(), "-all".to_owned()),
    ];
    if let Some(arch) = recipe.requires.arch {
        let arch = match arch {
            WinArch::Win32 => "win32",
            WinArch::Win64 => "win64",
        };
        env.push(("WINEARCH".to_owned(), arch.to_owned()));
    }
    // Wine's menu builder writes menu entries, icons and file associations
    // straight into the user's home. During the install it writes into this
    // staging folder instead, where Whistle picks what it needs.
    let staging = paths.cache.join("menus").join(&recipe.id);
    env.push((
        "XDG_DATA_HOME".to_owned(),
        staging.join("data").display().to_string(),
    ));
    env.push((
        "XDG_CONFIG_HOME".to_owned(),
        staging.join("config").display().to_string(),
    ));
    // Variables from `env` steps also apply to the launchers, which never run
    // the menu builder.
    let mut launcher_env = vec![(
        "WINEDLLOVERRIDES".to_owned(),
        "winemenubuilder.exe=d".to_owned(),
    )];

    if !runner.is_dry() {
        reset_staging(&staging)?;
    }
    create_dir(runner, &prefix)?;
    runner.run(&Cmd::new("wineboot").arg("--init").envs(&env))?;

    for (n, step) in recipe.steps.iter().enumerate() {
        match step {
            Step::Winetricks(verbs) => {
                runner.run(&Cmd::new("winetricks").arg("-q").args(verbs).envs(&env))?;
            }
            Step::DllOverride(dlls) => {
                let mut reg = String::from("[HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides]\r\n");
                for (dll, mode) in dlls {
                    writeln!(reg, "\"{}\"=\"{}\"\r", reg_escape(dll), reg_escape(mode))?;
                }
                apply_reg(runner, &prefix, n, &reg, &env)?;
            }
            Step::Registry(step) => {
                let mut reg = format!("[{}]\r\n", reg_root(&step.key)?);
                for (name, value) in &step.value {
                    match value {
                        RegValue::Dword(d) => {
                            writeln!(reg, "\"{}\"=dword:{d:08x}\r", reg_escape(name))?
                        }
                        RegValue::String(s) => {
                            writeln!(reg, "\"{}\"=\"{}\"\r", reg_escape(name), reg_escape(s))?
                        }
                    }
                }
                apply_reg(runner, &prefix, n, &reg, &env)?;
            }
            Step::Copy(copy) => {
                let from = media_path(&media, &copy.from)?;
                let to = windows_to_unix(&prefix, &copy.to)?;
                if runner.is_dry() {
                    runner.note(&format!("copy {} to {}", from.display(), to.display()));
                } else {
                    copy_recursive(&from, &to)?;
                }
            }
            Step::Env(vars) => {
                for (k, v) in vars {
                    env.push((k.clone(), v.clone()));
                    launcher_env.retain(|(name, _)| name != k);
                    launcher_env.push((k.clone(), v.clone()));
                }
            }
            Step::RunInstaller(run) => {
                let timeout = run.timeout_s.unwrap_or(DEFAULT_INSTALLER_TIMEOUT);
                let is_msi = media
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("msi"));
                let program = if is_msi {
                    Cmd::new("wine").args(["msiexec", "/i"])
                } else {
                    Cmd::new("wine")
                };
                let mut cmd = program
                    .arg(media.display().to_string())
                    .args(&run.args)
                    .envs(&env)
                    .timeout(Duration::from_secs(timeout));
                if let Some(dir) = media.parent() {
                    cmd = cmd.cwd(dir);
                }
                runner.run(&cmd)?;
            }
        }
    }

    // Let the installer's background processes finish before touching the prefix.
    if let Some(server) = wineserver() {
        let _ = runner.run(
            &Cmd::new(server.display().to_string())
                .arg("-w")
                .envs(&env)
                .timeout(Duration::from_secs(120)),
        );
    }
    // Done last, so the recipe's steps and installer could still replace any DLL.
    match wine_dir() {
        Some(dir) if runner.is_dry() => runner.note(&format!(
            "link Wine's own DLLs from {} into the prefix",
            dir.display()
        )),
        Some(dir) => {
            // Nothing new to share when upgrading a prefix that's already linked.
            let saved = link_builtin_dlls(&prefix, &dir)?;
            if saved > 0 {
                runner.note(&format!(
                    "shared {} MB of Wine DLLs instead of copying them",
                    saved / (1024 * 1024)
                ));
            }
        }
        None => runner.note("Wine's install directory not found; keeping the prefix's DLL copies"),
    }

    let launchers = if !recipe.launchers.is_empty() {
        recipe.launchers.clone()
    } else if runner.is_dry() {
        runner.note("add the installer's Start Menu shortcuts to the menu");
        Vec::new()
    } else {
        let found = discover_launchers(&staging, &prefix)?;
        if found.is_empty() {
            runner.note("the installer created no Start Menu shortcuts, so there is no menu entry");
        }
        found
    };
    if !runner.is_dry() {
        let _ = std::fs::remove_dir_all(&staging);
    }

    let mut desktop_files = Vec::new();
    for launcher in &launchers {
        let path = paths
            .applications
            .join(desktop_file_name(&recipe.id, &launcher.name));
        write_file(
            runner,
            &path,
            &desktop_entry(&recipe.id, launcher, &prefix, &launcher_env),
        )?;
        desktop_files.push(path);
    }
    if recipe.health_check.is_some() {
        runner.note("skip the health check (not implemented yet)");
    }
    Ok(WineInstall {
        prefix,
        desktop_files,
        launchers,
        launcher_env,
    })
}

/// Start `launcher` and check the app is still running after `wait`, then
/// close it again. Wine's `start` exits right away even when the app runs, so
/// this looks at the prefix's processes rather than an exit code.
pub fn health_check(done: &WineInstall, launcher: &Launcher, wait: Duration) -> Result<Health> {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Ok(Health::Skipped("no display to open a window on".into()));
    }
    let mut child = std::process::Command::new("wine")
        .arg(&launcher.exe)
        .args(&launcher.args)
        .env("WINEPREFIX", &done.prefix)
        .envs(done.launcher_env.iter().map(|(k, v)| (k, v)))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("starting wine")?;
    std::thread::sleep(wait);
    let running = !app_processes(&done.prefix).is_empty();
    // Close the app and everything else in the prefix.
    if let Some(server) = wineserver() {
        let _ = std::process::Command::new(&server)
            .arg("-k")
            .env("WINEPREFIX", &done.prefix)
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
    Ok(if running {
        Health::Running
    } else {
        Health::Exited
    })
}

/// The Windows programs running in `prefix`, apart from Wine's own.
fn app_processes(prefix: &Path) -> Vec<String> {
    let wanted = format!("WINEPREFIX={}", prefix.display());
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for proc in procs.flatten() {
        let dir = proc.path();
        let Ok(environ) = std::fs::read(dir.join("environ")) else {
            continue;
        };
        if !environ
            .split(|b| *b == 0)
            .any(|var| var == wanted.as_bytes())
        {
            continue;
        }
        let Ok(cmdline) = std::fs::read(dir.join("cmdline")) else {
            continue;
        };
        let argv0 = String::from_utf8_lossy(cmdline.split(|b| *b == 0).next().unwrap_or_default())
            .into_owned();
        let exe = argv0
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or_default()
            .to_lowercase();
        if exe.ends_with(".exe") && !WINE_PROGRAMS.contains(&exe.as_str()) {
            found.push(argv0);
        }
    }
    found
}

fn reset_staging(staging: &Path) -> Result<()> {
    match std::fs::remove_dir_all(staging) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("clearing {}", staging.display())),
    }
    std::fs::create_dir_all(staging.join("config"))?;
    // Wine maps Documents, Pictures, ... to the user's own folders, which it
    // finds through this file.
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")));
    if let Some(dirs) = config.map(|c| c.join("user-dirs.dirs"))
        && dirs.is_file()
    {
        std::fs::copy(&dirs, staging.join("config/user-dirs.dirs"))?;
    }
    Ok(())
}

/// The menu entries Wine's menu builder made for the installer's Start Menu
/// shortcuts, as launchers that run those shortcuts. Each launcher's icon is
/// copied into the prefix, so it goes away with it.
fn discover_launchers(staging: &Path, prefix: &Path) -> Result<Vec<Launcher>> {
    let programs = staging.join("data/applications/wine/Programs");
    let mut entries = Vec::new();
    collect_desktop_files(&programs, &mut entries)?;
    entries.sort();

    let start_menus: Vec<PathBuf> = {
        let mut dirs =
            vec![prefix.join("drive_c/ProgramData/Microsoft/Windows/Start Menu/Programs")];
        if let Ok(users) = std::fs::read_dir(prefix.join("drive_c/users")) {
            for user in users.flatten() {
                dirs.push(
                    user.path()
                        .join("AppData/Roaming/Microsoft/Windows/Start Menu/Programs"),
                );
            }
        }
        dirs
    };
    let mut launchers = Vec::new();
    for entry in entries {
        let Ok(relative) = entry.strip_prefix(&programs) else {
            continue;
        };
        let fields = desktop_fields(&std::fs::read_to_string(&entry).unwrap_or_default());
        let name = fields
            .get("Name")
            .cloned()
            .unwrap_or_else(|| relative.with_extension("").display().to_string());
        let lower = name.to_lowercase();
        if NOT_LAUNCHERS.iter().any(|w| lower.contains(w)) {
            continue;
        }
        // The shortcut the entry was made from, as a Windows path.
        let lnk = relative.with_extension("lnk");
        let Some(found) = start_menus
            .iter()
            .map(|d| d.join(&lnk))
            .find(|p| p.is_file())
        else {
            continue;
        };
        let Ok(in_drive) = found.strip_prefix(prefix.join("drive_c")) else {
            continue;
        };
        let exe = format!("C:\\{}", in_drive.display().to_string().replace('/', "\\"));
        let icon = fields
            .get("Icon")
            .and_then(|name| copy_icon(staging, prefix, name));
        launchers.push(Launcher {
            name,
            exe,
            args: Vec::new(),
            icon,
            mime: Vec::new(),
            categories: Vec::new(),
        });
    }
    Ok(launchers)
}

fn collect_desktop_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_desktop_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "desktop") {
            out.push(path);
        }
    }
    Ok(())
}

/// The `[Desktop Entry]` group's keys (untranslated values only).
fn desktop_fields(text: &str) -> std::collections::HashMap<String, String> {
    let mut fields = std::collections::HashMap::new();
    let mut in_entry = false;
    for line in text.lines() {
        if line.starts_with('[') {
            in_entry = line.trim() == "[Desktop Entry]";
        } else if in_entry && let Some((key, value)) = line.split_once('=') {
            fields.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    fields
}

/// Copy the largest PNG of a menu-builder icon into the prefix, and return its path.
fn copy_icon(staging: &Path, prefix: &Path, name: &str) -> Option<String> {
    if name.contains(['/', '\\']) || name.starts_with('.') {
        return None;
    }
    let sizes = std::fs::read_dir(staging.join("data/icons/hicolor")).ok()?;
    let (_, source) = sizes
        .flatten()
        .filter_map(|dir| {
            let size: u32 = dir.file_name().to_str()?.split('x').next()?.parse().ok()?;
            let png = dir.path().join("apps").join(format!("{name}.png"));
            png.is_file().then_some((size, png))
        })
        .max_by_key(|(size, _)| *size)?;
    let dest = prefix.join(".whistle-icons").join(format!("{name}.png"));
    std::fs::create_dir_all(dest.parent()?).ok()?;
    std::fs::copy(&source, &dest).ok()?;
    Some(dest.display().to_string())
}

/// Stop the prefix's wineserver and delete the prefix and its menu entries.
pub fn uninstall(
    paths: &Paths,
    prefix: &Path,
    desktop_files: &[PathBuf],
    runner: &dyn Runner,
) -> Result<()> {
    // Only ever delete inside our own prefixes dir, whatever the state file says.
    let escapes = prefix
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir));
    if escapes || !prefix.starts_with(paths.prefixes()) || prefix == paths.prefixes() {
        bail!(
            "refusing to delete {}: not a Whistle prefix",
            prefix.display()
        );
    }
    let env = [("WINEPREFIX".to_owned(), prefix.display().to_string())];
    if let Some(server) = wineserver() {
        let _ = runner.run(&Cmd::new(server.display().to_string()).arg("-k").envs(&env));
        let _ = runner.run(
            &Cmd::new(server.display().to_string())
                .arg("-w")
                .envs(&env)
                .timeout(Duration::from_secs(30)),
        );
    }
    for file in desktop_files {
        remove_path(runner, file)?;
    }
    remove_path(runner, prefix)
}

/// Where Wine keeps its own Windows DLLs (`x86_64-windows`, `i386-windows`) and `wineserver`.
pub fn wine_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(root) = which::which("wine")
        .ok()
        .and_then(|w| w.canonicalize().ok())
        .and_then(|w| Some(w.parent()?.parent()?.to_owned()))
    {
        for rel in ["lib/wine", "lib64/wine", "lib/x86_64-linux-gnu/wine"] {
            candidates.push(root.join(rel));
        }
    }
    candidates.extend(
        [
            "/usr/lib/x86_64-linux-gnu/wine",
            "/usr/lib/wine",
            "/usr/lib64/wine",
        ]
        .map(PathBuf::from),
    );
    candidates
        .into_iter()
        .find(|d| d.join("x86_64-windows").is_dir())
}

/// `wineserver` is often not on PATH (Debian and Ubuntu keep it in the Wine dir).
pub fn wineserver() -> Option<PathBuf> {
    which::which("wineserver").ok().or_else(|| {
        wine_dir()
            .map(|d| d.join("wineserver"))
            .filter(|p| p.is_file())
    })
}

/// Replace the prefix's copies of Wine's own DLLs with symlinks to them.
///
/// Wine copies about 1.1 GB of DLLs into every prefix. Linking them back is
/// safe only because the targets are read-only to us: a Windows program that
/// tries to overwrite one gets "access denied" instead of changing Wine's file
/// (and so every other prefix). Files the recipe changed no longer match and
/// stay as they are. Returns the bytes saved.
pub fn link_builtin_dlls(prefix: &Path, wine_dir: &Path) -> Result<u64> {
    let mut saved = 0;
    for (sub, arch) in [("system32", "x86_64-windows"), ("syswow64", "i386-windows")] {
        let base = prefix.join("drive_c/windows").join(sub);
        for dir in [base.clone(), base.join("drivers")] {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let target = wine_dir.join(arch).join(entry.file_name());
                let len = entry.metadata()?.len();
                let matches =
                    std::fs::metadata(&target).is_ok_and(|m| m.is_file() && m.len() == len);
                if !matches || is_writable(&target) || !same_content(&entry.path(), &target)? {
                    continue;
                }
                let path = entry.path();
                let tmp = path.with_extension("whistle-link");
                let _ = std::fs::remove_file(&tmp);
                symlink(&target, &tmp)?;
                std::fs::rename(&tmp, &path)
                    .with_context(|| format!("replacing {}", path.display()))?;
                saved += len;
            }
        }
    }
    Ok(saved)
}

fn is_writable(path: &Path) -> bool {
    std::fs::OpenOptions::new().write(true).open(path).is_ok()
}

fn same_content(a: &Path, b: &Path) -> Result<bool> {
    let (mut fa, mut fb) = (File::open(a)?, File::open(b)?);
    let (mut ba, mut bb) = (vec![0; 64 * 1024], vec![0; 64 * 1024]);
    loop {
        let n = fa.read(&mut ba)?;
        if n == 0 {
            return Ok(fb.read(&mut bb[..1])? == 0);
        }
        fb.read_exact(&mut bb[..n])
            .map_err(|_| anyhow::anyhow!("short read"))?;
        if ba[..n] != bb[..n] {
            return Ok(false);
        }
    }
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "symlinks need a Unix host",
    ))
}

/// Get the installer onto disk and prove it is the expected file.
fn acquire(
    recipe: &Recipe,
    paths: &Paths,
    runner: &dyn Runner,
    user_file: Option<&Path>,
) -> Result<PathBuf> {
    match &recipe.installer {
        Installer::Download {
            url,
            sha256,
            file_name,
        } => {
            let name = fetch::file_name_for(url, file_name.as_deref())?;
            let dir = paths.downloads().join(&recipe.id);
            if runner.is_dry() {
                runner.note(&format!("download {url} and check its SHA-256 is {sha256}"));
                return Ok(dir.join(name));
            }
            fetch::download_verified(url, sha256, &dir, &name)
        }
        Installer::UserSupplied {
            prompt,
            sha256_any,
            file_name_hint,
        } => {
            let Some(file) = user_file else {
                let hint = file_name_hint
                    .as_deref()
                    .map(|h| format!(" (usually {h})"))
                    .unwrap_or_default();
                bail!("{prompt}{hint}, and pass it with --file <path>");
            };
            fetch::verify(file, sha256_any)?;
            file.canonicalize()
                .with_context(|| format!("resolving {}", file.display()))
        }
        Installer::Package { .. } => bail!("package installers only apply to WSL recipes"),
    }
}

fn apply_reg(
    runner: &dyn Runner,
    prefix: &Path,
    step: usize,
    body: &str,
    env: &[(String, String)],
) -> Result<()> {
    let file = prefix.join("whistle").join(format!("step-{step}.reg"));
    write_file(runner, &file, &format!("REGEDIT4\r\n\r\n{body}"))?;
    runner.run(
        &Cmd::new("wine")
            .args(["regedit", "/S"])
            .arg(file.display().to_string())
            .envs(env),
    )
}

fn reg_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// `C:\Program Files\App` -> `<prefix>/drive_c/Program Files/App`.
pub fn windows_to_unix(prefix: &Path, path: &str) -> Result<PathBuf> {
    crate::recipe::check_windows_path(path)?;
    let (drive, rest) = path.split_at(3);
    if !drive.eq_ignore_ascii_case("c:\\") {
        bail!("{path:?}: only the C: drive is supported");
    }
    let mut out = prefix.join("drive_c");
    for part in rest.split('\\').filter(|p| !p.is_empty() && *p != ".") {
        out.push(part);
    }
    Ok(out)
}

/// A path inside the installer media: a folder, or the installer file itself.
fn media_path(media: &Path, relative: &str) -> Result<PathBuf> {
    let relative = relative.replace('\\', "/");
    if media.is_dir() {
        return Ok(media.join(relative));
    }
    if media
        .file_name()
        .is_some_and(|n| n.to_string_lossy() == relative)
    {
        return Ok(media.to_owned());
    }
    bail!(
        "copy: {relative:?} is not part of the installer media {}",
        media.display()
    )
}

fn copy_recursive(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::copy(from, to)
            .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
    }
    Ok(())
}

pub fn desktop_file_name(recipe_id: &str, launcher: &str) -> String {
    let slug: String = launcher
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    format!("whistle-{recipe_id}-{}.desktop", slug.trim_matches('-'))
}

/// A freedesktop menu entry that starts the launcher in its prefix.
pub fn desktop_entry(
    recipe_id: &str,
    launcher: &Launcher,
    prefix: &Path,
    env: &[(String, String)],
) -> String {
    let mut argv = vec!["env".to_owned(), format!("WINEPREFIX={}", prefix.display())];
    argv.extend(env.iter().map(|(k, v)| format!("{k}={v}")));
    argv.push("wine".to_owned());
    argv.push(launcher.exe.clone());
    argv.extend(launcher.args.iter().cloned());
    let mut exec = argv
        .iter()
        .map(|a| desktop_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
    if !launcher.mime.is_empty() {
        exec.push_str(" %f");
    }
    let one_line = |s: &str| s.replace(['\n', '\r'], " ");

    let mut out = String::from("[Desktop Entry]\nType=Application\n");
    let _ = writeln!(out, "Name={}", one_line(&launcher.name));
    let _ = writeln!(out, "Comment=Runs with Wine, installed by Whistle");
    // The Exec value is a string first (backslashes escaped), then a command line.
    let _ = writeln!(out, "Exec={}", exec.replace('\\', "\\\\"));
    let _ = writeln!(
        out,
        "Icon={}",
        launcher
            .icon
            .as_deref()
            .map(one_line)
            .unwrap_or_else(|| "wine".into())
    );
    if !launcher.categories.is_empty() {
        let _ = writeln!(out, "Categories={};", launcher.categories.join(";"));
    }
    if !launcher.mime.is_empty() {
        let _ = writeln!(out, "MimeType={};", launcher.mime.join(";"));
    }
    let _ = writeln!(out, "X-Whistle-Recipe={recipe_id}");
    out
}

/// Quote one argument per the Desktop Entry spec's Exec rules.
fn desktop_quote(arg: &str) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(',
        ')', '`',
    ];
    let arg = arg.replace('%', "%%");
    if arg.is_empty() || arg.contains(RESERVED) {
        let mut quoted = String::from("\"");
        for c in arg.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                quoted.push('\\');
            }
            quoted.push(c);
        }
        quoted.push('"');
        quoted
    } else {
        arg
    }
}

fn create_dir(runner: &dyn Runner, dir: &Path) -> Result<()> {
    if runner.is_dry() {
        runner.note(&format!("create {}", dir.display()));
        return Ok(());
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))
}

pub(crate) fn write_file(runner: &dyn Runner, path: &Path, contents: &str) -> Result<()> {
    if runner.is_dry() {
        runner.note(&format!("write {}", path.display()));
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}

pub(crate) fn remove_path(runner: &dyn Runner, path: &Path) -> Result<()> {
    if runner.is_dry() {
        runner.note(&format!("delete {}", path.display()));
        return Ok(());
    }
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match result {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("deleting {}", path.display()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Index;
    use crate::runner::DryRunner;

    fn recipe(app: &str) -> Recipe {
        let index = Index::builtin();
        Recipe::from_value(&index.find(app).unwrap().recipes[0].spec).unwrap()
    }

    #[test]
    fn exec_line_follows_desktop_spec_escaping() {
        let launcher = Launcher {
            name: "Notepad++".into(),
            exe: r"C:\Program Files\Notepad++\notepad++.exe".into(),
            args: vec!["50%".into()],
            icon: None,
            mime: vec!["text/plain".into()],
            categories: vec!["Development".into()],
        };
        let entry = desktop_entry("org.x.Y", &launcher, Path::new("/p"), &[]);
        assert!(entry.contains(r#"Exec=env WINEPREFIX=/p wine "C:\\\\Program Files\\\\Notepad++\\\\notepad++.exe" 50%% %f"#), "{entry}");
        assert!(entry.contains("MimeType=text/plain;\n"));
        assert!(entry.contains("Categories=Development;\n"));
    }

    #[test]
    fn windows_paths_map_into_the_prefix() {
        let p = windows_to_unix(Path::new("/pfx"), r"C:\Program Files\App\a.exe").unwrap();
        assert_eq!(p, Path::new("/pfx/drive_c/Program Files/App/a.exe"));
        assert!(windows_to_unix(Path::new("/pfx"), r"D:\a").is_err());
        assert!(windows_to_unix(Path::new("/pfx"), r"C:\..\..\etc").is_err());
    }

    #[test]
    fn dry_run_plans_every_step() {
        let paths = Paths::under(Path::new("/w"));
        let runner = DryRunner::default();
        let result = install(&recipe("notepad"), &paths, &runner, None).unwrap();
        assert_eq!(
            result.prefix,
            Path::new("/w/data/prefixes/org.notepad-plus-plus.NotepadPlusPlus")
        );
        let cmds = runner.commands();
        assert!(
            cmds[0].starts_with("WINEPREFIX=/w/data/prefixes/")
                && cmds[0].ends_with("wineboot --init")
        );
        assert!(
            cmds[1].ends_with("npp.8.9.9.Installer.x64.exe /S"),
            "{cmds:?}"
        );
        assert_eq!(
            result.desktop_files,
            [Path::new(
                "/w/applications/whistle-org.notepad-plus-plus.NotepadPlusPlus-notepad.desktop"
            )]
        );
    }

    #[test]
    fn registry_and_winetricks_steps() {
        let dir = std::env::temp_dir().join(format!("whistle-wine-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let setup = dir.join("setup.exe");
        std::fs::write(&setup, b"abc").unwrap();
        let mut office = recipe("com.microsoft.Office");
        let Installer::UserSupplied { sha256_any, .. } = &mut office.installer else {
            panic!()
        };
        *sha256_any =
            vec!["ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into()];

        let runner = DryRunner::default();
        let paths = Paths::under(&dir);
        assert!(
            install(&office, &paths, &runner, None)
                .unwrap_err()
                .to_string()
                .contains("--file")
        );
        install(&office, &paths, &runner, Some(&setup)).unwrap();
        let cmds = runner.commands();
        assert!(
            cmds.iter()
                .any(|c| c.ends_with("winetricks -q corefonts msxml6 riched20"))
        );
        assert_eq!(
            cmds.iter()
                .filter(|c| c.contains("wine regedit /S"))
                .count(),
            2
        );
        assert!(cmds.iter().any(|c| c.ends_with("setup.exe /quiet")));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn user_supplied_file_must_match_hash() {
        let dir = std::env::temp_dir().join(format!("whistle-wine-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let setup = dir.join("setup.exe");
        std::fs::write(&setup, b"not office").unwrap();
        let err = install(
            &recipe("com.microsoft.Office"),
            &Paths::under(&dir),
            &DryRunner::default(),
            Some(&setup),
        );
        assert!(err.unwrap_err().to_string().contains("doesn't match"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn links_only_identical_read_only_dlls() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("whistle-link-{}", std::process::id()));
        let wine = root.join("wine/x86_64-windows");
        let sys32 = root.join("pfx/drive_c/windows/system32");
        std::fs::create_dir_all(&wine).unwrap();
        std::fs::create_dir_all(&sys32).unwrap();
        for (name, wine_body, prefix_body, read_only) in [
            ("same.dll", "MZ same", "MZ same", true),
            ("changed.dll", "MZ wine", "MZ native", true),
            ("writable.dll", "MZ same", "MZ same", false),
        ] {
            std::fs::write(wine.join(name), wine_body).unwrap();
            if read_only {
                std::fs::set_permissions(wine.join(name), std::fs::Permissions::from_mode(0o444))
                    .unwrap();
            }
            std::fs::write(sys32.join(name), prefix_body).unwrap();
        }
        if is_writable(&wine.join("same.dll")) {
            eprintln!("skipping: running as root");
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        let saved = link_builtin_dlls(&root.join("pfx"), &root.join("wine")).unwrap();
        assert_eq!(saved, 7);
        assert_eq!(
            std::fs::read_link(sys32.join("same.dll")).unwrap(),
            wine.join("same.dll")
        );
        assert!(
            std::fs::read_link(sys32.join("changed.dll")).is_err(),
            "a DLL the recipe replaced stays"
        );
        assert!(
            std::fs::read_link(sys32.join("writable.dll")).is_err(),
            "never link to a file we could write through"
        );
        std::fs::set_permissions(
            wine.join("same.dll"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        std::fs::set_permissions(
            wine.join("changed.dll"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn uninstall_stays_inside_prefixes() {
        let paths = Paths::under(Path::new("/w"));
        let runner = DryRunner::default();
        assert!(uninstall(&paths, Path::new("/home/me"), &[], &runner).is_err());
        assert!(uninstall(&paths, &paths.prefixes(), &[], &runner).is_err());
        assert!(uninstall(&paths, &paths.prefixes().join(".."), &[], &runner).is_err());
        uninstall(&paths, &prefix_for(&paths, "org.x.Y"), &[], &runner).unwrap();
    }
}
