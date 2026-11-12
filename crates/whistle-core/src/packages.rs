//! Search the distro's own package database, for everything the catalog
//! doesn't list: command-line tools, libraries, fonts. Read-only and offline:
//! it uses what the package manager already knows, and never needs root.

use std::process::{Command, Stdio};

use crate::host::{Host, Tool};
use crate::index::SourceKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub summary: String,
}

/// The package manager `search` uses on this host.
pub fn manager(host: &Host) -> Option<SourceKind> {
    [
        (Tool::Pacman, SourceKind::Pacman),
        (Tool::AptGet, SourceKind::Apt),
        (Tool::Dnf, SourceKind::Dnf),
        (Tool::Zypper, SourceKind::Zypper),
    ]
    .into_iter()
    .find(|(tool, _)| host.has(*tool))
    .map(|(_, kind)| kind)
}

/// Packages whose name contains `query`, best matches first: the exact name,
/// then names starting with it, then the rest.
pub fn search(kind: SourceKind, query: &str) -> Vec<Package> {
    let query = query.trim().to_lowercase();
    // Never let a query pass itself off as an option.
    if query.is_empty() || query.starts_with('-') || query.contains(char::is_whitespace) {
        return Vec::new();
    }
    let output = match kind {
        SourceKind::Apt => run("apt-cache", &["search", "--names-only", &query]),
        SourceKind::Pacman => run("pacman", &["-Ss", &query]),
        SourceKind::Dnf => run(
            "dnf",
            &[
                "-q",
                "-C",
                "repoquery",
                "--qf",
                "%{name}\t%{summary}\n",
                &format!("*{query}*"),
            ],
        ),
        SourceKind::Zypper => run(
            "zypper",
            &["--no-refresh", "-x", "search", "-t", "package", &query],
        ),
        _ => None,
    };
    let Some(output) = output else {
        return Vec::new();
    };
    let mut found = match kind {
        SourceKind::Apt => parse_apt(&output),
        SourceKind::Pacman => parse_pacman(&output),
        SourceKind::Dnf => parse_dnf(&output),
        SourceKind::Zypper => parse_zypper(&output),
        _ => Vec::new(),
    };
    found.retain(|p| p.name.to_lowercase().contains(&query));
    found.sort_by_key(|p| {
        let name = p.name.to_lowercase();
        let rank = if name == query {
            0
        } else if name.starts_with(&query) {
            1
        } else {
            2
        };
        (rank, name.len(), name)
    });
    found.dedup_by(|a, b| a.name == b.name);
    found
}

/// The package called exactly `name`, if the package manager knows it.
pub fn find(kind: SourceKind, name: &str) -> Option<Package> {
    search(kind, name).into_iter().find(|p| p.name == name)
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `name - summary` per line.
fn parse_apt(text: &str) -> Vec<Package> {
    text.lines()
        .filter_map(|line| {
            let (name, summary) = line.split_once(" - ")?;
            Some(Package {
                name: name.trim().to_owned(),
                summary: summary.trim().to_owned(),
            })
        })
        .collect()
}

/// `repo/name version [installed]`, then the summary indented on the next line.
fn parse_pacman(text: &str) -> Vec<Package> {
    let mut found = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let Some(name) = line
            .split_whitespace()
            .next()
            .and_then(|n| n.rsplit_once('/'))
            .map(|(_, n)| n.to_owned())
        else {
            continue;
        };
        let summary = match lines.peek() {
            Some(next) if next.starts_with(char::is_whitespace) => next.trim().to_owned(),
            _ => String::new(),
        };
        found.push(Package { name, summary });
    }
    found
}

/// `name<TAB>summary` per line.
fn parse_dnf(text: &str) -> Vec<Package> {
    text.lines()
        .filter_map(|line| {
            let (name, summary) = line.split_once('\t')?;
            Some(Package {
                name: name.trim().to_owned(),
                summary: summary.trim().to_owned(),
            })
        })
        .collect()
}

/// `<solvable ... name="htop" summary="..." .../>` elements.
fn parse_zypper(text: &str) -> Vec<Package> {
    let attr = |element: &str, key: &str| -> Option<String> {
        let start = element.find(&format!(" {key}=\""))? + key.len() + 3;
        let end = element[start..].find('"')? + start;
        Some(unescape_xml(&element[start..end]))
    };
    text.split("<solvable")
        .skip(1)
        .filter_map(|element| {
            let element = element.split("/>").next()?;
            Some(Package {
                name: attr(element, "name")?,
                summary: attr(element, "summary").unwrap_or_default(),
            })
        })
        .collect()
}

fn unescape_xml(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(found: &[Package]) -> Vec<&str> {
        found.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn parses_each_package_manager() {
        let apt = parse_apt(
            "htop - interactive processes viewer\nbtop - Modern and colorful command line resource monitor\n",
        );
        assert_eq!(names(&apt), ["htop", "btop"]);
        assert_eq!(apt[0].summary, "interactive processes viewer");

        let pacman = parse_pacman(
            "extra/htop 3.4.1-1 [installed]\n    Interactive process viewer\nextra/bpytop 1.0.68-5\n    Resource monitor\n",
        );
        assert_eq!(names(&pacman), ["htop", "bpytop"]);
        assert_eq!(pacman[0].summary, "Interactive process viewer");

        let dnf = parse_dnf("htop\tInteractive process viewer\n");
        assert_eq!(names(&dnf), ["htop"]);

        let zypper = parse_zypper(
            r#"<?xml version='1.0'?><stream><search-result version="0.0"><solvable-list>
<solvable status="not-installed" name="htop" summary="An interactive process viewer &amp; more" kind="package"/>
<solvable status="installed" name="htop-doc" summary="Docs" kind="package"/>
</solvable-list></search-result></stream>"#,
        );
        assert_eq!(names(&zypper), ["htop", "htop-doc"]);
        assert_eq!(zypper[0].summary, "An interactive process viewer & more");
    }

    #[test]
    fn queries_cannot_be_options() {
        assert!(search(SourceKind::Apt, "--help").is_empty());
        assert!(search(SourceKind::Apt, "a b").is_empty());
    }
}
