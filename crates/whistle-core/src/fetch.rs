//! Downloads that are only kept when their SHA-256 matches the pinned hash.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Fail unless `path` hashes to one of `allowed`.
pub fn verify(path: &Path, allowed: &[String]) -> Result<String> {
    let actual = sha256_file(path)?;
    if !allowed.iter().any(|h| h.eq_ignore_ascii_case(&actual)) {
        bail!(
            "{} has SHA-256 {actual}, which doesn't match the catalog ({}); refusing to use it",
            path.display(),
            allowed.join(", ")
        );
    }
    Ok(actual)
}

/// The file name to save a download under: `file_name`, or the URL's last segment.
pub fn file_name_for(url: &str, file_name: Option<&str>) -> Result<String> {
    let name = file_name
        .map(str::to_owned)
        .or_else(|| url.split(['?', '#']).next()?.rsplit('/').next().map(str::to_owned))
        .unwrap_or_default();
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        bail!("can't pick a file name for {url}");
    }
    Ok(name)
}

/// Download `url` into `dir/name` and check it against `sha256`. A cached copy
/// with the right hash is reused; a wrong hash deletes the file.
pub fn download_verified(url: &str, sha256: &str, dir: &Path, name: &str) -> Result<PathBuf> {
    if !url.starts_with("https://") {
        bail!("refusing to download over plain HTTP: {url}");
    }
    let dest = dir.join(name);
    let allowed = [sha256.to_owned()];
    if dest.exists() && verify(&dest, &allowed).is_ok() {
        return Ok(dest);
    }
    std::fs::create_dir_all(dir)?;
    let partial = dir.join(format!("{name}.part"));
    eprintln!("downloading {url}");
    let response = ureq::get(url).call().with_context(|| format!("downloading {url}"))?;
    let mut reader = response.into_body().into_reader();
    let mut out = File::create(&partial)?;
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
    }
    out.sync_all()?;
    drop(out);
    if let Err(e) = verify(&partial, &allowed) {
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }
    std::fs::rename(&partial, &dest)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_accepts_matching_and_rejects_others() {
        let dir = std::env::temp_dir().join(format!("whistle-fetch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, b"abc").unwrap();
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_owned();
        assert_eq!(verify(&file, std::slice::from_ref(&abc)).unwrap(), abc);
        let err = verify(&file, &["0".repeat(64)]).unwrap_err();
        assert!(err.to_string().contains("doesn't match"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_names() {
        assert_eq!(file_name_for("https://x.org/a/b.exe?x=1", None).unwrap(), "b.exe");
        assert_eq!(file_name_for("https://x.org/a/", Some("c.exe")).unwrap(), "c.exe");
        assert!(file_name_for("https://x.org/a/", None).is_err());
        assert!(file_name_for("https://x.org/a", Some("../x")).is_err());
    }
}
