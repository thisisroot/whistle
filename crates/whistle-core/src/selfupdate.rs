//! Replace the running `whistle` with the latest release, the same way
//! install.sh installs it: the release's tarball for this CPU, checked against
//! the release's SHA256SUMS.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::fetch;

const REPO: &str = "thisisroot/whistle";

/// The tag of the latest release, such as "v0.2.0".
pub fn latest_tag() -> Result<String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let text = fetch::download_text(&url, 1024 * 1024)?;
    let release: serde_json::Value =
        serde_json::from_str(&text).context("reading the latest release")?;
    release["tag_name"]
        .as_str()
        .map(str::to_owned)
        .context("the latest release has no tag")
}

/// Whether `tag` is a newer release than this binary.
pub fn is_newer(tag: &str) -> bool {
    let current = env!("CARGO_PKG_VERSION");
    let tag = tag.trim_start_matches('v');
    crate::requirements::satisfies(tag, &format!(">{current}")).unwrap_or(false)
}

/// Download release `tag` and put it in place of `exe`.
pub fn install(tag: &str, exe: &Path) -> Result<()> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => bail!("there is no release build for {other}"),
    };
    let dir = exe.parent().context("finding the folder whistle is in")?;
    let probe = dir.join(".whistle-write-test");
    if std::fs::write(&probe, b"").is_err() {
        bail!(
            "can't write to {}; if your package manager installed Whistle, update it there",
            dir.display()
        );
    }
    let _ = std::fs::remove_file(&probe);

    let base = format!("https://github.com/{REPO}/releases/download/{tag}");
    let asset = format!("whistle-{arch}-linux.tar.gz");
    let sums = fetch::download_text(&format!("{base}/SHA256SUMS"), 64 * 1024)?;
    let sha256 = sums
        .lines()
        .find_map(|line| {
            let (hash, name) = line.split_once(char::is_whitespace)?;
            (name.trim().trim_start_matches('*') == asset).then(|| hash.to_owned())
        })
        .with_context(|| format!("{asset} is not listed in the release's SHA256SUMS"))?;

    let work = tempdir()?;
    let result = (|| {
        let tarball = fetch::download_verified(&format!("{base}/{asset}"), &sha256, &work, &asset)?;
        let status = Command::new("tar")
            .arg("-xzf")
            .arg(&tarball)
            .arg("-C")
            .arg(&work)
            .status()
            .context("running tar")?;
        if !status.success() {
            bail!("unpacking {asset} failed");
        }
        let new = work.join("whistle/whistle");
        if !new.is_file() {
            bail!("{asset} doesn't contain whistle/whistle");
        }
        // Copy next to the old binary, then rename over it: never half a file,
        // and the running process keeps its old copy until it exits.
        let staged = dir.join(".whistle.new");
        std::fs::copy(&new, &staged)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&staged, exe).with_context(|| format!("replacing {}", exe.display()))
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn tempdir() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("whistle-update-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_newer_tags_count() {
        assert!(is_newer("v999.0.0"));
        assert!(!is_newer(&format!("v{}", env!("CARGO_PKG_VERSION"))));
        assert!(!is_newer("v0.0.1"));
        assert!(!is_newer("nightly"));
    }
}
