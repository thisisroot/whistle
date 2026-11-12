//! Opening installed apps, through their menu entries where they have one.

use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use gio::prelude::*;
use gio::{AppLaunchContext, DesktopAppInfo};
use whistle_core::index::{App, SourceKind};
use whistle_core::state::{How, Installed};

pub fn open(installed: &Installed, app: Option<&App>) -> Result<()> {
    let entry = match &installed.how {
        How::Package {
            kind: SourceKind::Flatpak,
            reference,
        } => return detach(Command::new("flatpak").args(["run", reference])),
        How::Package { reference, .. } => package_entry(&installed.app_id, reference, app)
            .with_context(|| format!("can't find {}'s menu entry", installed.app_name))?,
        How::Appimage { desktop_file, .. } => DesktopAppInfo::from_filename(desktop_file)
            .with_context(|| format!("{} is missing", desktop_file.display()))?,
        How::Recipe { desktop_files, .. } => desktop_files
            .iter()
            .find_map(DesktopAppInfo::from_filename)
            .with_context(|| format!("{} has no menu entry left", installed.app_name))?,
    };
    entry
        .launch(&[], None::<&AppLaunchContext>)
        .with_context(|| format!("couldn't open {}", installed.app_name))
}

/// A distro package's menu entry: usually named after the catalog id, else
/// after the package, else found by the app's name.
fn package_entry(id: &str, reference: &str, app: Option<&App>) -> Option<DesktopAppInfo> {
    let by_id = if id.ends_with(".desktop") {
        id.to_owned()
    } else {
        format!("{id}.desktop")
    };
    DesktopAppInfo::new(&by_id)
        .or_else(|| DesktopAppInfo::new(&format!("{reference}.desktop")))
        .or_else(|| {
            let name = app?.name.to_lowercase();
            gio::AppInfo::all()
                .into_iter()
                .filter(|info| info.should_show())
                .find(|info| info.name().to_lowercase() == name)?
                .id()
                .and_then(|id| DesktopAppInfo::new(&id))
        })
}

fn detach(command: &mut Command) -> Result<()> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("couldn't start it")?;
    // Reap it when it exits, so it doesn't linger as a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}
