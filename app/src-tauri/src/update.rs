//! Updates. Detect what is installed from the programs themselves, ask each project's own publisher what is
//! newer, install a chosen version with a backup, verify it, and roll back on request or on failure.
//!
//! Safety rules: only https downloads from known hosts, every archive checked against the publisher's SHA-256,
//! never touch a running server, the user's configuration and data are carried over, the previous version is kept.

use crate::{component, fwd, manifest, push_note, root, save_settings_in, settings, settings_in, sha256_file, start, state, stop, unpack_to, Component};
use serde::Serialize;
use std::{
    fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const NO_WINDOW: u32 = 0x0800_0000;
const HOSTS: &[&str] = &[
    "windows.php.net", "downloads.php.net", "www.apachelounge.com", "downloads.mariadb.org", "archive.mariadb.org", "files.phpmyadmin.net", "www.phpmyadmin.net",
];
const NOTES_HOSTS: &[&str] = &["www.php.net", "www.apachelounge.com", "mariadb.com", "www.phpmyadmin.net", "httpd.apache.org"];

// ---- types ----

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Candidate {
    pub version: String,
    /// patch | minor | major | toolchain
    pub kind: &'static str,
    pub line: String,
    pub url: String,
    pub sha256: String,
    pub strip_prefix: String,
    pub notes_url: String,
    pub warnings: Vec<String>,
}

#[derive(Serialize, Clone)]
pub struct BackupInfo {
    pub version: String,
    pub made: u64,
    pub data_copied: bool,
}

#[derive(Serialize)]
pub struct UpdateInfo {
    pub id: String,
    pub name: String,
    pub current: String,
    pub candidates: Vec<Candidate>,
    pub error: Option<String>,
    /// why it cannot be updated right now (a running server)
    pub blocked: Option<String>,
    pub backup: Option<BackupInfo>,
}

// ---- versions ----

pub fn parse_ver(s: &str) -> Vec<u32> {
    s.trim().split(|c: char| !(c.is_ascii_digit() || c == '.')).next().unwrap_or("").split('.').filter_map(|p| p.parse().ok()).collect()
}

pub fn cmp_ver(a: &str, b: &str) -> std::cmp::Ordering {
    parse_ver(a).cmp(&parse_ver(b))
}

/// "8.3.35" -> "8.3". The release line an update normally stays inside.
pub fn line_of(v: &str) -> String {
    parse_ver(v).iter().take(2).map(u32::to_string).collect::<Vec<_>>().join(".")
}

pub fn kind_of(current: &str, cand: &str) -> &'static str {
    let (c, n) = (parse_ver(current), parse_ver(cand));
    if line_of(current) == line_of(cand) {
        "patch"
    } else if c.first() == n.first() {
        "minor"
    } else {
        "major"
    }
}

/// `>=7.2,<8.4`-style constraint against a PHP version (compares major.minor).
pub fn php_ok(constraint: &str, php: &str) -> bool {
    let p = parse_ver(&line_of(php));
    constraint.split(',').all(|part| {
        let part = part.trim();
        let (op, v) = if let Some(v) = part.strip_prefix(">=") { (">=", v) } else if let Some(v) = part.strip_prefix("<=") { ("<=", v) } else if let Some(v) = part.strip_prefix('>') { (">", v) } else if let Some(v) = part.strip_prefix('<') { ("<", v) } else { ("=", part.trim_start_matches('=')) };
        let o = p.cmp(&parse_ver(v));
        match op {
            ">=" => o.is_ge(),
            "<=" => o.is_le(),
            ">" => o.is_gt(),
            "<" => o.is_lt(),
            _ => o.is_eq(),
        }
    })
}

fn run_capture(exe: &Path, args: &[&str]) -> Option<String> {
    let o = Command::new(exe).args(args).stdin(Stdio::null()).creation_flags(NO_WINDOW).output().ok()?;
    Some(format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

/// The installed version, read from the program itself (not from our records).
pub fn detect_version(c: &Component, root: &Path) -> Option<String> {
    let bin = root.join(&c.binary);
    if !bin.exists() {
        return None;
    }
    let after = |text: &str, marker: &str| text.split_once(marker).map(|(_, r)| parse_ver(r)).filter(|v| !v.is_empty()).map(|v| v.iter().map(u32::to_string).collect::<Vec<_>>().join("."));
    match c.id.as_str() {
        "apache" => after(&run_capture(&bin, &["-v"])?, "Apache/"),
        "php" => after(&run_capture(&bin, &["-v"])?, "PHP "),
        "mariadb" => after(&run_capture(&bin, &["--version"])?, "Ver "),
        "phpmyadmin" => {
            let t = fs::read_to_string(root.join(&c.extract_to).join("libraries/classes/Version.php")).ok()?;
            after(t.split_once("VERSION = '")?.1, "")
        }
        _ => None,
    }
}

/// Visual Studio toolchain of the installed Apache Lounge build (17, 18...).
pub fn apache_toolchain(c: &Component, root: &Path) -> Option<u32> {
    let t = run_capture(&root.join(&c.binary), &["-V"])?; // capital V: the build line; -v does not print it
    t.split("Apache Lounge VS").nth(1)?.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()
}

// ---- network (small, allow-listed) ----

pub fn host_allowed(url: &str, hosts: &[&str]) -> bool {
    url.strip_prefix("https://").and_then(|r| r.split(['/', '?', '#']).next()).is_some_and(|h| hosts.contains(&h))
}

pub fn http_get(url: &str) -> Result<String, String> {
    if !host_allowed(url, HOSTS) {
        return Err(format!("refusing to contact {url}"));
    }
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(25))
        .user_agent("runqora/0.1")
        .build()
        .get(url)
        .call()
        .map_err(|e| format!("{url}: {e}"))?
        .into_string()
        .map_err(|e| e.to_string())
}

fn first_hex64(text: &str) -> Option<String> {
    text.split(|c: char| !c.is_ascii_hexdigit()).find(|t| t.len() == 64).map(str::to_lowercase)
}

// ---- parsers: one per publisher, pure so they can be tested against saved samples ----

/// windows.php.net/downloads/releases/releases.json
pub fn parse_php(json: &str, current: &str, pma_php: Option<&str>) -> Vec<Candidate> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return vec![] };
    let mut out = Vec::new();
    for (line, rel) in v.as_object().into_iter().flatten() {
        let Some(version) = rel.get("version").and_then(|x| x.as_str()) else { continue };
        if !line.chars().all(|c| c.is_ascii_digit() || c == '.') || cmp_ver(version, current).is_le() {
            continue;
        }
        let Some(zip) = rel.as_object().into_iter().flatten().filter(|(k, _)| k.starts_with("ts-") && k.ends_with("-x64")).find_map(|(_, b)| b.get("zip")) else { continue };
        let (Some(path), Some(sha)) = (zip.get("path").and_then(|x| x.as_str()), zip.get("sha256").and_then(|x| x.as_str())) else { continue };
        let kind = kind_of(current, version);
        let mut warnings = vec![];
        if kind != "patch" {
            warnings.push(format!("PHP {line} is a new release line: defaults, extensions and deprecations change. Test your sites before relying on it."));
        }
        if let Some(c) = pma_php.filter(|c| !php_ok(c, version)) {
            warnings.push(format!("The installed phpMyAdmin supports PHP {c}; it may stop working with PHP {line}."));
        }
        out.push(Candidate { version: version.into(), kind, line: line.clone(), url: format!("https://windows.php.net/downloads/releases/{path}"), sha256: sha.to_lowercase(), strip_prefix: String::new(), notes_url: format!("https://www.php.net/ChangeLog-{}.php#{version}", parse_ver(version)[0]), warnings });
    }
    out.sort_by(|a, b| cmp_ver(&a.version, &b.version));
    out
}

/// downloads.mariadb.org/rest-api/mariadb/ -> stable release lines at or after `current_line`
pub fn parse_mariadb_lines(json: &str, current_line: &str) -> Vec<(String, bool)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return vec![] };
    let mut out: Vec<(String, bool)> = v["major_releases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["release_status"] == "Stable")
        .filter_map(|r| Some((r["release_id"].as_str()?.to_string(), r["release_support_type"].as_str().is_some_and(|t| t.contains("Long")))))
        .filter(|(l, _)| cmp_ver(l, current_line).is_ge())
        .collect();
    out.sort_by(|a, b| cmp_ver(&a.0, &b.0));
    out
}

/// downloads.mariadb.org/rest-api/mariadb/<line>/ -> newest Windows zip of that line
pub fn parse_mariadb_release(json: &str, line: &str, lts: bool, current: &str) -> Option<Candidate> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let (version, rel) = v["releases"].as_object()?.iter().max_by(|a, b| cmp_ver(a.0, b.0))?;
    if cmp_ver(version, current).is_le() {
        return None;
    }
    let file = rel["files"].as_array()?.iter().find(|f| f["file_name"].as_str().is_some_and(|n| n.ends_with("winx64.zip")))?;
    let kind = kind_of(current, version);
    let mut warnings = vec![];
    if kind != "patch" {
        warnings.push("This upgrades the database files in place. A full copy of your databases is kept for roll back, but anything you save after the update is lost if you roll back. Export what matters first.".to_string());
        if !lts {
            warnings.push(format!("MariaDB {line} is a short-term release with a shorter support period than a long-term one."));
        }
    }
    Some(Candidate {
        version: version.clone(),
        kind,
        line: line.into(),
        url: format!("https://archive.mariadb.org/mariadb-{version}/winx64-packages/mariadb-{version}-winx64.zip"),
        sha256: file["checksum"]["sha256sum"].as_str()?.to_lowercase(),
        strip_prefix: format!("mariadb-{version}-winx64/"),
        notes_url: format!("https://mariadb.com/kb/en/mariadb-{}-release-notes/", version.replace('.', "-")),
        warnings,
    })
}

/// Apache Lounge download page -> newest Win64 zip (version, file name)
pub fn parse_lounge_page(html: &str, vs: u32) -> Option<(String, String)> {
    let needle = format!("-Win64-VS{vs}.zip");
    let end = html.find(&needle)?;
    let start = html[..end].rfind("httpd-")?;
    let file = format!("{}{}", &html[start..end], needle);
    let version = file.strip_prefix("httpd-")?.split('-').next()?.to_string();
    (!parse_ver(&version).is_empty()).then_some((version, file))
}

pub fn parse_lounge_sha(txt: &str) -> Option<String> {
    first_hex64(txt.split_once("SHA256")?.1)
}

pub fn lounge_candidate(version: &str, file: &str, vs: u32, sha: &str, current: &str, installed_vs: Option<u32>) -> Candidate {
    let toolchain = installed_vs.is_some_and(|i| i != vs);
    let mut warnings = vec![];
    if toolchain {
        warnings.push(format!("This build uses a different Visual Studio toolchain (VS{vs}, yours is VS{}). The PHP module may not load into it: check your sites after updating.", installed_vs.unwrap_or(0)));
    }
    Candidate { version: version.into(), kind: if toolchain { "toolchain" } else { kind_of(current, version) }, line: line_of(version), url: format!("https://www.apachelounge.com/download/VS{vs}/binaries/{file}"), sha256: sha.into(), strip_prefix: "Apache24/".into(), notes_url: "https://www.apachelounge.com/Changelog-2.4.html".into(), warnings }
}

/// phpmyadmin.net/home_page/version.json -> releases newer than `current`
pub fn parse_pma(json: &str, current: &str, php: Option<&str>) -> Vec<(String, Vec<String>)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return vec![] };
    v["releases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| Some((r["version"].as_str()?.to_string(), r["php_versions"].as_str().unwrap_or("").to_string())))
        .filter(|(ver, _)| cmp_ver(ver, current).is_gt())
        .map(|(ver, need)| {
            let mut w = vec![];
            if kind_of(current, &ver) != "patch" {
                w.push(format!("phpMyAdmin {} is a new release line; its interface and settings can differ.", line_of(&ver)));
            }
            if let Some(p) = php.filter(|p| !need.is_empty() && !php_ok(&need, p)) {
                w.push(format!("phpMyAdmin {ver} needs PHP {need}; you have PHP {p}."));
            }
            (ver, w)
        })
        .collect()
}

/// The PHP range the *installed* phpMyAdmin supports, from the same JSON.
pub fn pma_php_range(json: &str, installed: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v["releases"].as_array()?.iter().find(|r| line_of(r["version"].as_str().unwrap_or("")) == line_of(installed)).and_then(|r| r["php_versions"].as_str()).map(str::to_string)
}

// ---- checking ----

fn installed(c: &Component, root: &Path) -> bool {
    root.join(&c.binary).exists()
}

pub fn blocked_reason(c: &Component, root: &Path) -> Option<String> {
    let running = |id: &str| component(id).ok().is_some_and(|x| x.args.is_some() && root.join(&x.binary).exists() && state(&x, root).status == "running");
    match c.id.as_str() {
        "apache" if running("apache") => Some("Stop Apache first.".into()),
        "php" if running("apache") => Some("Stop Apache first: it has PHP loaded.".into()),
        "mariadb" if running("mariadb") => Some("Stop MariaDB first.".into()),
        _ => None,
    }
}

fn candidates_for(c: &Component, root: &Path, current: &str, pma_json: Option<&str>) -> Result<Vec<Candidate>, String> {
    match c.id.as_str() {
        "php" => {
            let pma_range = pma_json.and_then(|j| component("phpmyadmin").ok().and_then(|p| detect_version(&p, root)).and_then(|v| pma_php_range(j, &v)));
            Ok(parse_php(&http_get("https://windows.php.net/downloads/releases/releases.json")?, current, pma_range.as_deref()))
        }
        "mariadb" => {
            let lines = parse_mariadb_lines(&http_get("https://downloads.mariadb.org/rest-api/mariadb/")?, &line_of(current));
            let mut out = vec![];
            for (line, lts) in lines {
                let json = http_get(&format!("https://downloads.mariadb.org/rest-api/mariadb/{line}/"))?;
                out.extend(parse_mariadb_release(&json, &line, lts, current));
            }
            Ok(out)
        }
        "apache" => {
            let mine = apache_toolchain(c, root);
            let mut out = vec![];
            for vs in [17u32, 18] {
                let page = http_get(&format!("https://www.apachelounge.com/download/VS{vs}/"))?;
                let Some((version, file)) = parse_lounge_page(&page, vs).filter(|(v, _)| cmp_ver(v, current).is_gt()) else { continue };
                let sha = parse_lounge_sha(&http_get(&format!("https://www.apachelounge.com/download/VS{vs}/binaries/{file}.txt"))?).ok_or("no checksum published for the Apache download")?;
                out.push(lounge_candidate(&version, &file, vs, &sha, current, mine));
            }
            out.sort_by(|a, b| cmp_ver(&a.version, &b.version));
            Ok(out)
        }
        "phpmyadmin" => {
            let php = component("php").ok().and_then(|p| detect_version(&p, root));
            let mut out = vec![];
            for (v, warnings) in pma_json.map(|j| parse_pma(j, current, php.as_deref())).unwrap_or_default() {
                let sha = first_hex64(&http_get(&format!("https://files.phpmyadmin.net/phpMyAdmin/{v}/phpMyAdmin-{v}-all-languages.zip.sha256"))?).ok_or("no checksum published for phpMyAdmin")?;
                out.push(Candidate { kind: kind_of(current, &v), line: line_of(&v), url: format!("https://files.phpmyadmin.net/phpMyAdmin/{v}/phpMyAdmin-{v}-all-languages.zip"), sha256: sha, strip_prefix: format!("phpMyAdmin-{v}-all-languages/"), notes_url: "https://www.phpmyadmin.net/news/".into(), warnings, version: v });
            }
            Ok(out)
        }
        _ => Ok(vec![]),
    }
}

/// Check every installed component. Failures are reported per component, never as a whole.
pub fn check_updates() -> Vec<UpdateInfo> {
    let root = root();
    let pma_json = http_get("https://www.phpmyadmin.net/home_page/version.json").ok();
    manifest()
        .components
        .iter()
        .filter_map(|m| component(&m.id).ok())
        .filter(|c| installed(c, &root))
        .map(|c| {
            let current = detect_version(&c, &root).unwrap_or_else(|| c.version.clone());
            let (candidates, error) = match candidates_for(&c, &root, &current, pma_json.as_deref()) {
                Ok(v) => (v, None),
                Err(e) => (vec![], Some(e)),
            };
            UpdateInfo { id: c.id.clone(), name: c.name.clone(), current, candidates, error, blocked: blocked_reason(&c, &root), backup: latest_backup(&root, &c.id).map(|(_, m)| m) }
        })
        .collect()
}

// ---- swapping versions ----

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Keep {
    /// carry a copy into the new tree
    Copy,
    /// move it (fast: for the database files)
    Move,
}

/// What belongs to the user inside a component folder and must survive a version change.
pub fn keep_rules(id: &str, kind: &str) -> Vec<(&'static str, Keep)> {
    match id {
        "apache" => vec![("conf", Keep::Copy), ("logs", Keep::Move)],
        "php" => vec![("php.ini", Keep::Copy)],
        "mariadb" => vec![("my.ini", Keep::Copy), ("data", if kind == "patch" { Keep::Move } else { Keep::Copy })],
        "phpmyadmin" => vec![("config.inc.php", Keep::Copy)],
        _ => vec![],
    }
}

fn copy_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        fs::create_dir_all(dst)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_all(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(src, dst).map(|_| ())
    }
}

fn remove_any(p: &Path) {
    if p.is_dir() {
        let _ = fs::remove_dir_all(p);
    } else {
        let _ = fs::remove_file(p);
    }
}

/// Replace `live` by `replacement`: the old tree is renamed to `aside`, the user's items are carried over.
/// On any failure the old tree is put back and `live` is exactly as it was.
pub fn swap_in(live: &Path, replacement: &Path, aside: &Path, keep: &[(&str, Keep)]) -> Result<(), String> {
    fs::create_dir_all(aside.parent().ok_or("bad backup path")?).map_err(|e| e.to_string())?;
    fs::rename(live, aside).map_err(|e| format!("could not move {} aside ({e}). Close anything using its files and try again", live.display()))?;
    if let Err(e) = fs::rename(replacement, live) {
        let _ = fs::rename(aside, live);
        return Err(format!("could not put the new version in place ({e})"));
    }
    let mut moved: Vec<(PathBuf, PathBuf)> = vec![];
    for (rel, mode) in keep {
        let (src, dst) = (aside.join(rel), live.join(rel));
        if !src.exists() {
            continue;
        }
        remove_any(&dst); // the fresh tree's default must not win over the user's file
        if let Some(p) = dst.parent() {
            let _ = fs::create_dir_all(p);
        }
        let r = match mode {
            Keep::Copy => copy_all(&src, &dst),
            Keep::Move => fs::rename(&src, &dst).inspect(|_| moved.push((src.clone(), dst.clone()))),
        };
        if let Err(e) = r {
            for (s, d) in moved.iter().rev() {
                let _ = fs::rename(d, s);
            }
            remove_any(live);
            let _ = fs::rename(aside, live);
            return Err(format!("could not carry {rel} over ({e}); nothing was changed"));
        }
    }
    Ok(())
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn backups_dir(root: &Path, id: &str) -> PathBuf {
    root.join("backups").join(id)
}

#[derive(Serialize, serde::Deserialize)]
struct Meta {
    version: String,
    made: u64,
    data_copied: bool,
    /// "backup" can be rolled back to; "undone" is what a roll back replaced
    role: String,
}

fn read_backups(root: &Path, id: &str) -> Vec<(PathBuf, Meta)> {
    let mut v: Vec<_> = fs::read_dir(backups_dir(root, id))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| Some((e.path(), serde_json::from_str::<Meta>(&fs::read_to_string(e.path().join("meta.json")).ok()?).ok()?)))
        .collect();
    v.sort_by_key(|(_, m)| m.made);
    v
}

pub fn latest_backup(root: &Path, id: &str) -> Option<(PathBuf, BackupInfo)> {
    read_backups(root, id).into_iter().rev().find(|(_, m)| m.role == "backup").map(|(p, m)| (p, BackupInfo { version: m.version, made: m.made, data_copied: m.data_copied }))
}

fn write_meta(dir: &Path, m: &Meta) -> Result<(), String> {
    fs::write(dir.join("meta.json"), serde_json::to_string_pretty(m).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Keep the newest rollback target and the newest undone tree; delete the rest.
fn prune(root: &Path, id: &str) {
    let all = read_backups(root, id);
    for role in ["backup", "undone"] {
        let mine: Vec<_> = all.iter().filter(|(_, m)| m.role == role).collect();
        for (p, _) in mine.iter().take(mine.len().saturating_sub(1)) {
            let _ = fs::remove_dir_all(p);
        }
    }
}

/// Checks that run after a swap. Returns warnings; an Err means "undo it".
pub type Verify<'a> = &'a dyn Fn(&Component, &Path, &str) -> Result<Vec<String>, String>;

pub fn verify_install(c: &Component, root: &Path, want: &str) -> Result<Vec<String>, String> {
    let got = detect_version(c, root).ok_or("the new files do not report a version")?;
    if got != want {
        return Err(format!("expected {want} but found {got}"));
    }
    let mut warnings = vec![];
    match c.id.as_str() {
        "apache" => {
            let out = run_capture(&root.join(&c.binary), &["-d", &fwd(&root.join(&c.extract_to)), "-t"]).unwrap_or_default();
            if !out.contains("Syntax OK") {
                return Err(format!("Apache rejects the configuration: {}", out.trim().lines().last().unwrap_or("")));
            }
        }
        "php" => {
            let out = run_capture(&root.join(&c.binary), &["-v"]).unwrap_or_default();
            if out.contains("Warning") || out.contains("Unable to load") {
                return Err(format!("PHP reports a problem on startup: {}", out.lines().next().unwrap_or("")));
            }
        }
        "mariadb" => {
            // start it once with the user's own settings, upgrade the system tables, stop it again
            start(c, root).map_err(|e| format!("MariaDB does not start: {e}"))?;
            let port = c.port.to_string();
            let up = run_capture(&root.join(c.extract_to.as_str()).join("bin/mariadb-upgrade.exe"), &["--user=root", "--host=127.0.0.1", &format!("--port={port}")]);
            if !up.as_deref().is_some_and(|o| !o.to_lowercase().contains("error")) {
                warnings.push(format!("mariadb-upgrade reported a problem: {}", up.unwrap_or_default().trim().lines().last().unwrap_or("it did not run")));
            }
            stop(c, root).map_err(|e| e.to_string())?;
        }
        _ => {}
    }
    Ok(warnings)
}

fn finish_msg(c: &Component, old: &str, new: &str, warnings: &[String]) -> String {
    let mut m = format!("{} updated to {new} (was {old}). The previous version is kept so you can roll back.", c.name);
    for w in warnings {
        m += &format!(" {w}");
    }
    m
}

/// Install `cand` from an already downloaded archive. `verify` decides whether it works; if not, everything is undone.
pub fn apply_from_zip(c: &Component, root: &Path, cand: &Candidate, zip: &Path, verify: Verify) -> Result<String, String> {
    if let Some(b) = blocked_reason(c, root) {
        return Err(b);
    }
    let got = sha256_file(zip).map_err(|e| e.to_string())?;
    if !got.eq_ignore_ascii_case(&cand.sha256) {
        return Err(format!("the download does not match the publisher's checksum (expected {}, got {got}); nothing was changed", cand.sha256));
    }
    let live = root.join(&c.extract_to);
    let old = detect_version(c, root).unwrap_or_else(|| c.version.clone());
    let stage = root.join("tmp").join(format!("stage-{}", c.id));
    remove_any(&stage);
    unpack_to(zip, &stage, &cand.strip_prefix)?;
    let made = now();
    let dir = backups_dir(root, &c.id).join(made.to_string());
    let keep = keep_rules(&c.id, cand.kind);
    let data_copied = keep.iter().any(|(r, m)| *r == "data" && *m == Keep::Copy);
    swap_in(&live, &stage, &dir.join("tree"), &keep).inspect_err(|_| remove_any(&stage))?;
    match verify(c, root, &cand.version) {
        Ok(warnings) => {
            write_meta(&dir, &Meta { version: old.clone(), made, data_copied, role: "backup".into() })?;
            prune(root, &c.id);
            Ok(finish_msg(c, &old, &cand.version, &warnings))
        }
        Err(e) => {
            // put the old version back; the failed tree is discarded
            let failed = dir.join("failed");
            // a copied database still sits untouched in the old tree: do not overwrite it with whatever the failed version did to the copy
            let undo_keep: Vec<_> = keep.iter().filter(|(r, _)| !(*r == "data" && data_copied)).cloned().collect();
            let undo = swap_in(&live, &dir.join("tree"), &failed, &undo_keep);
            let _ = fs::remove_dir_all(&dir);
            match undo {
                Ok(()) => Err(format!("{} {} did not pass its checks ({e}). {old} was restored.", c.name, cand.version)),
                Err(u) => Err(format!("{} {} did not pass its checks ({e}) and restoring {old} failed ({u}). Your previous files are in {}", c.name, cand.version, dir.display())),
            }
        }
    }
}

pub fn download(url: &str, to: &Path) -> Result<(), String> {
    if !host_allowed(url, HOSTS) {
        return Err(format!("refusing to download from {url}"));
    }
    fs::create_dir_all(to.parent().ok_or("bad path")?).map_err(|e| e.to_string())?;
    let part = to.with_extension("part");
    let resp = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).user_agent("runqora/0.1").build().get(url).call().map_err(|e| format!("{url}: {e}"))?;
    std::io::copy(&mut resp.into_reader(), &mut fs::File::create(&part).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(&part, to).map_err(|e| e.to_string())
}

/// Look the version up afresh (the UI only names it), download, verify, install.
pub fn apply_update(id: &str, version: &str) -> Result<String, String> {
    let root = root();
    let c = component(id)?;
    if !installed(&c, &root) {
        return Err(format!("{} is not installed", c.name));
    }
    if let Some(b) = blocked_reason(&c, &root) {
        return Err(b);
    }
    let pma_json = http_get("https://www.phpmyadmin.net/home_page/version.json").ok();
    let current = detect_version(&c, &root).unwrap_or_else(|| c.version.clone());
    let cand = candidates_for(&c, &root, &current, pma_json.as_deref())?.into_iter().find(|x| x.version == version).ok_or("that version is no longer offered; check for updates again")?;
    let zip = root.join("tmp").join(format!("{id}-{version}.zip"));
    download(&cand.url, &zip)?;
    let r = apply_from_zip(&c, &root, &cand, &zip, &verify_install);
    let _ = fs::remove_file(&zip);
    r
}

/// Go back to the version kept by the last update.
pub fn rollback_with(c: &Component, root: &Path, verify: Verify) -> Result<String, String> {
    if let Some(b) = blocked_reason(c, root) {
        return Err(b);
    }
    let (dir, meta) = read_backups(root, &c.id).into_iter().rev().find(|(_, m)| m.role == "backup").ok_or("there is no previous version to go back to")?;
    let live = root.join(&c.extract_to);
    let current = detect_version(c, root).unwrap_or_else(|| c.version.clone());
    // a copied database comes back as it was; otherwise the current data moves into the restored version
    let keep: Vec<(&str, Keep)> = keep_rules(&c.id, "patch").into_iter().filter(|(r, _)| !(*r == "data" && meta.data_copied)).collect();
    let made = now();
    let undone = backups_dir(root, &c.id).join(format!("{made}-undone"));
    swap_in(&live, &dir.join("tree"), &undone.join("tree"), &keep)?;
    match verify(c, root, &meta.version) {
        Ok(w) => {
            write_meta(&undone, &Meta { version: current.clone(), made, data_copied: false, role: "undone".into() })?;
            let _ = fs::remove_dir_all(&dir);
            prune(root, &c.id);
            let note = if meta.data_copied { " Databases are back to how they were before that update." } else { "" };
            Ok(format!("{} rolled back to {} (was {current}).{note}{}", c.name, meta.version, w.iter().map(|x| format!(" {x}")).collect::<String>()))
        }
        Err(e) => {
            let back = swap_in(&live, &undone.join("tree"), &dir.join("tree"), &keep);
            Err(format!("the previous version did not pass its checks ({e}); {}", if back.is_ok() { "the current version was kept" } else { "restoring also failed: look in the backups folder" }))
        }
    }
}

pub fn rollback(id: &str) -> Result<String, String> {
    rollback_with(&component(id)?, &root(), &verify_install)
}

// ---- automatic updates ----

pub fn auto_mode() -> &'static str {
    match settings().auto_update.as_str() {
        "off" => "off",
        "install" => "install",
        _ => "notify",
    }
}

fn summary(infos: &[UpdateInfo]) -> String {
    infos.iter().filter(|i| !i.candidates.is_empty()).map(|i| format!("{} {}", i.name, i.candidates.iter().map(|c| c.version.as_str()).collect::<Vec<_>>().join("/"))).collect::<Vec<_>>().join(", ")
}

/// One round of the scheduler: check at most once a day, tell the user, and in "install" mode apply patch updates
/// of servers that are stopped. Minor and major upgrades are never automatic.
pub fn auto_cycle() {
    let root = root();
    let mode = auto_mode();
    let s = settings_in(&root);
    if mode == "off" || now().saturating_sub(s.last_check) < 24 * 3600 {
        return;
    }
    let infos = check_updates();
    let mut s = settings_in(&root);
    s.last_check = now();
    let text = summary(&infos);
    if !text.is_empty() && text != s.last_notified {
        push_note("info", format!("Updates available: {text}. Open Updates to review them."));
        s.last_notified = text;
    }
    let _ = save_settings_in(&root, &s);
    for e in infos.iter().filter_map(|i| i.error.as_ref()).take(1) {
        push_note("error", format!("Could not check for updates: {e}"));
    }
    if mode != "install" {
        return;
    }
    for i in &infos {
        let Some(best) = i.candidates.iter().filter(|c| c.kind == "patch").max_by(|a, b| cmp_ver(&a.version, &b.version)) else { continue };
        if let Some(b) = &i.blocked {
            push_note("info", format!("{} {} is waiting: {b}", i.name, best.version));
            continue;
        }
        match apply_update(&i.id, &best.version) {
            Ok(m) => push_note("success", m),
            Err(e) => push_note("error", format!("{}: {e}", i.name)),
        }
    }
}

/// Background thread for the window: first look shortly after launch, then every 30 minutes (the daily limit is in auto_cycle).
pub fn start_scheduler() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(20));
        loop {
            auto_cycle();
            std::thread::sleep(Duration::from_secs(30 * 60));
        }
    });
}

pub fn open_notes(url: &str) -> Result<(), String> {
    if !host_allowed(url, NOTES_HOSTS) {
        return Err("that address is not a release-notes page this app opens".into());
    }
    Command::new("cmd").args(["/C", "start", "", url]).creation_flags(NO_WINDOW).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
