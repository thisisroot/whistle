//! Check a recipe's `requires` against this machine before installing:
//! the Wine version, RAM and free disk space.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Result, bail};

use crate::recipe::Requires;

/// What stands in the way: hard problems stop the install, warnings don't.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Findings {
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
}

/// Check `requires`, with the app's files going under `data`.
pub fn check(requires: &Requires, data: &Path) -> Findings {
    check_with(
        requires,
        wine_version().as_deref(),
        total_ram_mb(),
        free_disk_mb(data),
    )
}

fn check_with(
    requires: &Requires,
    wine: Option<&str>,
    ram_mb: Option<u64>,
    free_mb: Option<u64>,
) -> Findings {
    let mut found = Findings::default();
    if let (Some(constraint), Some(version)) = (&requires.wine, wine) {
        match satisfies(version, constraint) {
            Ok(true) => {}
            Ok(false) => found.problems.push(format!(
                "needs Wine {}, but this machine has Wine {version}",
                constraint.trim()
            )),
            Err(e) => found
                .warnings
                .push(format!("couldn't check the Wine version: {e:#}")),
        }
    }
    if let (Some(needed), Some(have)) = (requires.ram_mb, ram_mb)
        && have < needed
    {
        found.warnings.push(format!(
            "recommends {needed} MB of RAM; this machine has {have} MB, so it may be slow"
        ));
    }
    if let (Some(needed), Some(free)) = (requires.disk_mb, free_mb)
        && free < needed
    {
        found.problems.push(format!(
            "needs {needed} MB of free disk space, but only {free} MB is free"
        ));
    }
    found
}

/// Whether `version` meets every comma-separated condition in `constraint`,
/// such as ">=8.0" or ">=9.0, <11".
pub fn satisfies(version: &str, constraint: &str) -> Result<bool> {
    let have = parse_version(version)?;
    for condition in constraint.split(',') {
        let condition = condition.trim();
        let (op, wanted) = [">=", "<=", "==", ">", "<"]
            .iter()
            .find_map(|op| condition.strip_prefix(op).map(|rest| (*op, rest)))
            .unwrap_or(("==", condition));
        let wanted = parse_version(wanted)?;
        let ordering = compare(&have, &wanted);
        let ok = match op {
            ">=" => ordering.is_ge(),
            "<=" => ordering.is_le(),
            ">" => ordering.is_gt(),
            "<" => ordering.is_lt(),
            _ => ordering.is_eq(),
        };
        if !ok {
            return Ok(false);
        }
    }
    Ok(true)
}

fn parse_version(text: &str) -> Result<Vec<u64>> {
    let text = text.trim();
    let numbers: Option<Vec<u64>> = text
        .split('.')
        .map(|part| {
            // "0-rc2" counts as 0.
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
        .collect();
    match numbers {
        Some(numbers) if !numbers.is_empty() => Ok(numbers),
        _ => bail!("{text:?} is not a version number"),
    }
}

/// Compare as if the shorter version had trailing zeros: 10.0 == 10.
fn compare(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    let len = a.len().max(b.len());
    let at = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    (0..len)
        .map(|i| at(a, i).cmp(&at(b, i)))
        .find(|o| o.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

/// The installed Wine's version, from `wine --version` ("wine-10.0 (Ubuntu ...)").
pub fn wine_version() -> Option<String> {
    let output = Command::new("wine")
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    parse_wine_version(&String::from_utf8_lossy(&output.stdout))
}

fn parse_wine_version(text: &str) -> Option<String> {
    let version = text.trim().strip_prefix("wine-")?;
    let version: String = version
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    (!version.is_empty()).then_some(version)
}

fn total_ram_mb() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// Free space for an unprivileged user on the filesystem holding `path` (or
/// its nearest existing parent, as the data folder may not exist yet).
#[cfg(unix)]
fn free_disk_mb(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let existing = path.ancestors().find(|p| p.exists())?;
    let c_path = std::ffi::CString::new(existing.as_os_str().as_bytes()).ok()?;
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: c_path is a valid C string and stats a valid buffer for statvfs.
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stats) } != 0 {
        return None;
    }
    // u64 on 64-bit systems, smaller on some 32-bit ones.
    #[allow(clippy::unnecessary_cast)]
    let free = stats.f_bavail as u64 * stats.f_frsize as u64;
    Some(free / (1024 * 1024))
}

#[cfg(not(unix))]
fn free_disk_mb(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_constraints() {
        assert!(satisfies("10.0", ">=8.0").unwrap());
        assert!(satisfies("10", ">=10.0").unwrap());
        assert!(!satisfies("7.22", ">=8.0").unwrap());
        assert!(satisfies("9.5", ">=9.0, <11").unwrap());
        assert!(!satisfies("11.1", ">=9.0, <11").unwrap());
        assert!(satisfies("9.0", "9").unwrap());
        assert!(satisfies("x", ">=1").is_err());
    }

    #[test]
    fn reads_wine_versions() {
        assert_eq!(
            parse_wine_version("wine-10.0 (Ubuntu 10.0~repack-12ubuntu1)\n").as_deref(),
            Some("10.0")
        );
        assert_eq!(
            parse_wine_version("wine-9.22 (Staging)").as_deref(),
            Some("9.22")
        );
        assert_eq!(parse_wine_version("garbage"), None);
    }

    #[test]
    fn problems_stop_and_warnings_dont() {
        let requires = Requires {
            wine: Some(">=9.0".into()),
            ram_mb: Some(4096),
            disk_mb: Some(2000),
            ..Default::default()
        };
        let fine = check_with(&requires, Some("10.0"), Some(16000), Some(50000));
        assert_eq!(fine, Findings::default());

        let found = check_with(&requires, Some("8.0"), Some(2048), Some(500));
        assert_eq!(found.problems.len(), 2);
        assert!(found.problems[0].contains("needs Wine >=9.0, but this machine has Wine 8.0"));
        assert!(found.problems[1].contains("only 500 MB is free"));
        assert_eq!(found.warnings.len(), 1);

        // Unknown facts don't block anything.
        assert_eq!(check_with(&requires, None, None, None), Findings::default());
    }
}
