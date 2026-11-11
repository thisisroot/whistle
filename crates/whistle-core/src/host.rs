//! What machine we're on: OS, distro, CPU and which tools are installed.

use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    Windows,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tool {
    Pacman,
    AptGet,
    Dnf,
    Zypper,
    Flatpak,
    Wine,
    Winetricks,
    Winget,
    Wsl,
    Sudo,
}

impl Tool {
    pub const ALL: [Tool; 10] = [
        Self::Pacman,
        Self::AptGet,
        Self::Dnf,
        Self::Zypper,
        Self::Flatpak,
        Self::Wine,
        Self::Winetricks,
        Self::Winget,
        Self::Wsl,
        Self::Sudo,
    ];

    pub fn binary(self) -> &'static str {
        match self {
            Self::Pacman => "pacman",
            Self::AptGet => "apt-get",
            Self::Dnf => "dnf",
            Self::Zypper => "zypper",
            Self::Flatpak => "flatpak",
            Self::Wine => "wine",
            Self::Winetricks => "winetricks",
            Self::Winget => "winget",
            Self::Wsl => "wsl",
            Self::Sudo => "sudo",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Host {
    pub os: Os,
    /// os-release `ID`, e.g. `arch`, `ubuntu`, `fedora`. Empty off Linux.
    pub distro: String,
    /// os-release `ID_LIKE`.
    pub distro_like: Vec<String>,
    pub distro_version: String,
    pub pretty_name: String,
    /// `x86_64`, `aarch64`, ...
    pub arch: String,
    pub tools: BTreeSet<Tool>,
    /// Running inside WSL (a Linux guest on Windows).
    pub in_wsl: bool,
    pub is_root: bool,
}

impl Host {
    pub fn detect() -> Self {
        let os = match std::env::consts::OS {
            "linux" => Os::Linux,
            "windows" => Os::Windows,
            _ => Os::Other,
        };
        let os_release = std::fs::read_to_string("/etc/os-release")
            .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
            .ok();
        let tools = Tool::ALL.into_iter().filter(|t| which::which(t.binary()).is_ok()).collect();
        let mut host = Self::from_parts(os, os_release.as_deref(), std::env::consts::ARCH, tools);
        if os == Os::Linux {
            host.in_wsl = std::fs::read_to_string("/proc/sys/kernel/osrelease")
                .is_ok_and(|r| r.to_lowercase().contains("microsoft"));
            host.is_root = std::fs::read_to_string("/proc/self/status").is_ok_and(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("Uid:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    == Some("0")
            });
        }
        host
    }

    /// Build a host from its raw facts; detection is the only side effect-y part.
    pub fn from_parts(os: Os, os_release: Option<&str>, arch: &str, tools: BTreeSet<Tool>) -> Self {
        let fields = os_release.map(parse_os_release).unwrap_or_default();
        let get = |k: &str| fields.get(k).cloned().unwrap_or_default();
        Self {
            os,
            distro: get("ID"),
            distro_like: get("ID_LIKE").split_whitespace().map(str::to_owned).collect(),
            distro_version: get("VERSION_ID"),
            pretty_name: get("PRETTY_NAME"),
            arch: arch.to_owned(),
            tools,
            in_wsl: false,
            is_root: false,
        }
    }

    pub fn has(&self, tool: Tool) -> bool {
        self.tools.contains(&tool)
    }

    /// Platform tags this host satisfies, in the server's vocabulary
    /// (`windows`, `linux`, `linux:arch`, `linux:debian`, ...).
    pub fn platform_tags(&self) -> Vec<String> {
        match self.os {
            Os::Windows => vec!["windows".into()],
            Os::Other => vec![],
            Os::Linux => {
                let mut tags = vec!["linux".to_owned()];
                for id in std::iter::once(&self.distro).chain(&self.distro_like) {
                    let id = if id.starts_with("opensuse") || id == "suse" { "opensuse" } else { id.as_str() };
                    let tag = format!("linux:{id}");
                    if !id.is_empty() && !tags.contains(&tag) {
                        tags.push(tag);
                    }
                }
                tags
            }
        }
    }

    pub fn matches_platforms(&self, platforms: &[String]) -> bool {
        let tags = self.platform_tags();
        platforms.iter().any(|p| tags.contains(p))
    }

    /// An empty list means any architecture.
    pub fn matches_arch(&self, arches: &[String]) -> bool {
        arches.is_empty() || arches.iter().any(|a| a == &self.arch)
    }

    pub fn describe(&self) -> String {
        match self.os {
            Os::Linux if !self.pretty_name.is_empty() => self.pretty_name.clone(),
            Os::Linux => format!("Linux ({})", self.distro),
            Os::Windows => "Windows".into(),
            Os::Other => std::env::consts::OS.into(),
        }
    }
}

/// Parse os-release(5): `KEY=value`, with optional single or double quotes.
pub fn parse_os_release(text: &str) -> HashMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| {
            let v = v.trim();
            let unquoted = v
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(v);
            (k.trim().to_owned(), unquoted.replace("\\\"", "\"").replace("\\\\", "\\"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux(os_release: &str) -> Host {
        Host::from_parts(Os::Linux, Some(os_release), "x86_64", BTreeSet::new())
    }

    #[test]
    fn ubuntu_is_also_debian() {
        let host = linux("NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\nVERSION_ID=\"26.04\"\nPRETTY_NAME=\"Ubuntu 26.04 LTS\"\n");
        assert_eq!(host.platform_tags(), ["linux", "linux:ubuntu", "linux:debian"]);
        assert_eq!(host.distro_version, "26.04");
        assert_eq!(host.describe(), "Ubuntu 26.04 LTS");
    }

    #[test]
    fn arch_derivatives_and_opensuse() {
        assert_eq!(linux("ID=manjaro\nID_LIKE=arch\n").platform_tags(), ["linux", "linux:manjaro", "linux:arch"]);
        assert_eq!(linux("ID=arch\n").platform_tags(), ["linux", "linux:arch"]);
        assert_eq!(
            linux("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n").platform_tags(),
            ["linux", "linux:opensuse"]
        );
    }

    #[test]
    fn fedora_matches_fedora_sources_only() {
        let host = linux("ID=fedora\nVERSION_ID=41\n");
        assert!(host.matches_platforms(&["linux:fedora".into()]));
        assert!(host.matches_platforms(&["linux".into()]));
        assert!(!host.matches_platforms(&["linux:arch".into(), "windows".into()]));
    }

    #[test]
    fn windows_tags() {
        let host = Host::from_parts(Os::Windows, None, "x86_64", BTreeSet::new());
        assert_eq!(host.platform_tags(), ["windows"]);
    }

    #[test]
    fn arch_filter() {
        let host = linux("ID=arch\n");
        assert!(host.matches_arch(&[]));
        assert!(host.matches_arch(&["x86_64".into()]));
        assert!(!host.matches_arch(&["aarch64".into()]));
    }
}
