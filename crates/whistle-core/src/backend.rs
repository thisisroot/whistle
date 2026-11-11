//! Commands for the package managers Whistle drives. Whistle has already asked
//! the user, so the managers run non-interactively.

use anyhow::{Result, bail};

use crate::host::{Host, Tool};
use crate::index::SourceKind;
use crate::runner::Cmd;

const FLATHUB: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";

/// The tool a package source needs, if it is driven by a package manager.
pub fn tool_for(kind: SourceKind) -> Option<Tool> {
    match kind {
        SourceKind::Pacman => Some(Tool::Pacman),
        SourceKind::Apt => Some(Tool::AptGet),
        SourceKind::Dnf => Some(Tool::Dnf),
        SourceKind::Zypper => Some(Tool::Zypper),
        SourceKind::Flatpak => Some(Tool::Flatpak),
        SourceKind::Winget => Some(Tool::Winget),
        SourceKind::Appimage | SourceKind::VendorInstaller | SourceKind::Wsl => None,
    }
}

/// Prefix system package managers with sudo unless we're already root.
fn privileged(host: &Host, program: &str) -> Cmd {
    if host.is_root { Cmd::new(program) } else { Cmd::new("sudo").arg(program) }
}

pub fn install_cmds(kind: SourceKind, reference: &str, host: &Host) -> Result<Vec<Cmd>> {
    check_ref(reference)?;
    Ok(match kind {
        SourceKind::Pacman => vec![privileged(host, "pacman").args(["-S", "--needed", "--noconfirm", reference])],
        SourceKind::Apt => vec![privileged(host, "apt-get").args(["install", "-y", reference])],
        SourceKind::Dnf => vec![privileged(host, "dnf").args(["install", "-y", reference])],
        SourceKind::Zypper => vec![privileged(host, "zypper").args(["--non-interactive", "install", reference])],
        // Per-user installs: no sudo, and nothing touches the system.
        SourceKind::Flatpak => vec![
            Cmd::new("flatpak").args(["remote-add", "--user", "--if-not-exists", "flathub", FLATHUB]),
            Cmd::new("flatpak").args(["install", "--user", "-y", "--noninteractive", "flathub", reference]),
        ],
        SourceKind::Winget => vec![Cmd::new("winget").args([
            "install",
            "--id",
            reference,
            "--exact",
            "--accept-package-agreements",
            "--accept-source-agreements",
        ])],
        SourceKind::Appimage | SourceKind::VendorInstaller | SourceKind::Wsl => {
            bail!("{kind} is not installed through a package manager")
        }
    })
}

pub fn remove_cmds(kind: SourceKind, reference: &str, host: &Host) -> Result<Vec<Cmd>> {
    check_ref(reference)?;
    Ok(match kind {
        SourceKind::Pacman => vec![privileged(host, "pacman").args(["-Rs", "--noconfirm", reference])],
        SourceKind::Apt => vec![privileged(host, "apt-get").args(["remove", "-y", reference])],
        SourceKind::Dnf => vec![privileged(host, "dnf").args(["remove", "-y", reference])],
        SourceKind::Zypper => vec![privileged(host, "zypper").args(["--non-interactive", "remove", reference])],
        SourceKind::Flatpak => vec![Cmd::new("flatpak").args(["uninstall", "--user", "-y", "--noninteractive", reference])],
        SourceKind::Winget => vec![Cmd::new("winget").args(["uninstall", "--id", reference, "--exact"])],
        SourceKind::Appimage | SourceKind::VendorInstaller | SourceKind::Wsl => {
            bail!("{kind} is not removed through a package manager")
        }
    })
}

/// Package refs come from the index; make sure one can't pass itself off as an option.
fn check_ref(reference: &str) -> Result<()> {
    let ok = reference.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && reference.chars().all(|c| c.is_ascii_alphanumeric() || "._+:/@-".contains(c));
    if !ok {
        bail!("invalid package reference {reference:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::Os;
    use std::collections::BTreeSet;

    fn host(root: bool) -> Host {
        let mut h = Host::from_parts(Os::Linux, Some("ID=arch\n"), "x86_64", BTreeSet::new());
        h.is_root = root;
        h
    }

    fn shown(cmds: Vec<Cmd>) -> Vec<String> {
        cmds.iter().map(Cmd::display).collect()
    }

    #[test]
    fn native_installs_use_sudo_unless_root() {
        assert_eq!(
            shown(install_cmds(SourceKind::Apt, "firefox", &host(false)).unwrap()),
            ["sudo apt-get install -y firefox"]
        );
        assert_eq!(
            shown(install_cmds(SourceKind::Pacman, "gimp", &host(true)).unwrap()),
            ["pacman -S --needed --noconfirm gimp"]
        );
    }

    #[test]
    fn flatpak_is_per_user() {
        let cmds = shown(install_cmds(SourceKind::Flatpak, "org.gimp.GIMP", &host(false)).unwrap());
        assert_eq!(cmds[1], "flatpak install --user -y --noninteractive flathub org.gimp.GIMP");
        assert!(!cmds.iter().any(|c| c.starts_with("sudo")));
    }

    #[test]
    fn refs_cannot_be_options() {
        assert!(install_cmds(SourceKind::Apt, "--allow-unauthenticated", &host(false)).is_err());
        assert!(install_cmds(SourceKind::Apt, "a b", &host(false)).is_err());
    }
}
