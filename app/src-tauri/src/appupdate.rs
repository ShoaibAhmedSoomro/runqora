//! Runqora's own updates. The latest GitHub release of this project is the source of truth: publish a release with a
//! newer version and `Runqora-Setup-x64.exe`, and every running copy offers it on its Updates page.
//!
//! The installer is downloaded only from this project's release URL, checked against the SHA-256 GitHub shows for the
//! file, and started the normal way (its own window, per-user, no administrator rights). Nothing runs silently.

use crate::update::{cmp_ver, download, http_get};
use crate::{check_hash, root};
use serde::Serialize;
use std::process::Command;

const RELEASE_API: &str = "https://api.github.com/repos/ShoaibAhmedSoomro/runqora/releases/latest";
const DOWNLOAD_PREFIX: &str = "https://github.com/ShoaibAhmedSoomro/runqora/releases/download/";
const ASSET: &str = "Runqora-Setup-x64.exe";

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct AppUpdate {
    pub current: String,
    pub latest: String,
    /// the release page on GitHub
    pub notes_url: String,
    pub published: String,
    /// the release notes (Markdown), shortened
    pub body: String,
    #[serde(skip)]
    pub url: String,
    #[serde(skip)]
    pub sha256: String,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// A GitHub "latest release" JSON -> an update, if it is newer than `current` and carries the installer with a digest.
pub fn parse_release(json: &str, current: &str) -> Option<AppUpdate> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let latest = v["tag_name"].as_str()?.trim_start_matches('v').to_string();
    if cmp_ver(&latest, current).is_le() {
        return None;
    }
    let a = v["assets"].as_array()?.iter().find(|a| a["name"] == ASSET)?;
    let url = a["browser_download_url"].as_str()?.to_string();
    if !url.starts_with(DOWNLOAD_PREFIX) || !url.ends_with(ASSET) {
        return None; // only this project's own release files are ever offered
    }
    let sha256 = a["digest"].as_str()?.strip_prefix("sha256:")?.to_lowercase();
    let body: String = v["body"].as_str().unwrap_or("").chars().take(1500).collect();
    Some(AppUpdate { current: current.into(), latest, notes_url: v["html_url"].as_str()?.into(), published: v["published_at"].as_str().unwrap_or("").into(), body, url, sha256 })
}

pub fn check() -> Result<Option<AppUpdate>, String> {
    Ok(parse_release(&http_get(RELEASE_API)?, current_version()))
}

/// Download the newest installer, verify it and start it. The caller then closes the app so the installer can replace it.
pub fn install() -> Result<String, String> {
    let _busy = crate::Busy::new("app", "Looking up the release");
    let u = check()?.ok_or("Runqora is already the newest version")?; // look it up again: the UI only asked for "the update"
    let file = root().join("tmp").join(ASSET);
    download("app", &u.url, &file)?;
    crate::progress_set("app", "Checking the download", 0, 0);
    if let Err(e) = check_hash(&u.sha256, &file) {
        let _ = std::fs::remove_file(&file);
        return Err(e);
    }
    crate::progress_set("app", "Opening the installer", 0, 0);
    // /UPDATE: the installer replaces the files in place (no uninstall first, no questions) and keeps everything else
    Command::new(&file).arg("/UPDATE").spawn().map_err(|e| format!("could not start the installer: {e}"))?;
    Ok(format!("The Runqora {} installer is open. Runqora closes now; finish the installer and it opens again.", u.latest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, url: &str, digest: &str) -> String {
        format!(r#"{{"tag_name":"{tag}","html_url":"https://github.com/ShoaibAhmedSoomro/runqora/releases/tag/{tag}","published_at":"2026-11-01T10:00:00Z","body":"Fixes and a new module.","assets":[{{"name":"SHA256.txt","browser_download_url":"x","digest":"sha256:00"}},{{"name":"Runqora-Setup-x64.exe","browser_download_url":"{url}","digest":"{digest}"}}]}}"#)
    }
    const OK_URL: &str = "https://github.com/ShoaibAhmedSoomro/runqora/releases/download/v0.2.0/Runqora-Setup-x64.exe";

    #[test]
    fn a_newer_release_is_offered_with_its_digest() {
        let u = parse_release(&release("v0.2.0", OK_URL, "sha256:ABCD"), "0.1.1").unwrap();
        assert_eq!((u.latest.as_str(), u.current.as_str(), u.sha256.as_str(), u.url.as_str()), ("0.2.0", "0.1.1", "abcd", OK_URL));
        assert!(u.notes_url.ends_with("/tag/v0.2.0") && u.body.contains("new module"));
    }

    #[test]
    fn same_older_or_unsafe_releases_are_not_offered() {
        assert!(parse_release(&release("v0.2.0", OK_URL, "sha256:ab"), "0.2.0").is_none(), "same version");
        assert!(parse_release(&release("v0.2.0", OK_URL, "sha256:ab"), "0.10.0").is_none(), "0.10 is newer than 0.2");
        assert!(parse_release(&release("v0.2.0", "https://evil.example/Runqora-Setup-x64.exe", "sha256:ab"), "0.1.0").is_none(), "another host");
        assert!(parse_release(&release("v0.2.0", "https://github.com/someone-else/runqora/releases/download/v0.2.0/Runqora-Setup-x64.exe", "sha256:ab"), "0.1.0").is_none(), "another repository");
        assert!(parse_release(&release("v0.2.0", OK_URL, "md5:ab"), "0.1.0").is_none(), "no SHA-256 to check");
        assert!(parse_release("not json", "0.1.0").is_none() && parse_release(r#"{"tag_name":"v9.0.0","assets":[]}"#, "0.1.0").is_none(), "no installer attached");
    }

    /// The real project release, read from GitHub (needs the internet).
    #[test]
    #[ignore]
    fn the_published_release_parses() {
        let u = parse_release(&http_get(RELEASE_API).unwrap(), "0.0.1").expect("the latest release carries the installer");
        assert!(u.url.starts_with(DOWNLOAD_PREFIX) && u.sha256.len() == 64, "{u:?}");
    }
}
