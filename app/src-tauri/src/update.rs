//! Updates. Detect what is installed from the programs themselves, ask each project's own publisher what is
//! newer, install a chosen version with a backup, verify it, and roll back on request or on failure.
//!
//! Safety rules: only https downloads from known hosts, every archive checked against the publisher's SHA-256,
//! never touch a running server, the user's configuration and data are carried over, the previous version is kept.

use crate::{check_hash, component, fwd, manifest, push_note, read_marker, root, save_settings_in, settings, settings_in, start, state, stop, unpack_component, write_marker, Component};
use serde::Serialize;
use std::{
    fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const NO_WINDOW: u32 = 0x0800_0000;
pub const HOSTS: &[&str] = &[
    "windows.php.net", "downloads.php.net", "www.apachelounge.com", "downloads.mariadb.org", "archive.mariadb.org", "files.phpmyadmin.net", "www.phpmyadmin.net",
    "api.adoptium.net", "github.com", "api.github.com", "archive.apache.org", "api.wordpress.org", "wordpress.org", "downloads.wordpress.org", "nodejs.org", "getcomposer.org",
];
const NOTES_HOSTS: &[&str] = &[
    "www.php.net", "www.apachelounge.com", "mariadb.com", "www.phpmyadmin.net", "httpd.apache.org", "github.com", "tomcat.apache.org", "adoptium.net", "wordpress.org", "nodejs.org",
];

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
    let dir = root.join(&c.extract_to);
    // the newer modules: what Runqora itself installed is the most reliable record; the program is the fallback
    if matches!(c.id.as_str(), "jre" | "tomcat" | "mailpit" | "sftpgo" | "wordpress" | "node" | "composer" | "adminer") {
        return read_marker(&dir).or_else(|| match c.id.as_str() {
            "jre" => after(&run_capture(&bin, &["-version"])?, "version \""),
            "tomcat" => after(&fs::read_to_string(dir.join("RELEASE-NOTES")).ok()?, "Apache Tomcat Version "),
            "wordpress" => after(fs::read_to_string(dir.join("wp-includes/version.php")).ok()?.split_once("$wp_version = '")?.1, ""),
            "node" => after(&run_capture(&bin, &["-v"])?, "v"),
            _ => None,
        });
    }
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

// ---- parsers for the newer modules ----

fn plain(version: &str, current: &str, name: &str, url: String, hash: String, strip: &str, notes: String, extra: Vec<String>) -> Candidate {
    let kind = kind_of(current, version);
    let mut warnings = extra;
    if kind != "patch" {
        warnings.insert(0, format!("{name} {} is a new release line: behaviour and settings can change. Test your sites and projects before relying on it.", line_of(version)));
    }
    Candidate { version: version.into(), kind, line: line_of(version), url, sha256: hash, strip_prefix: strip.into(), notes_url: notes, warnings }
}

fn hex_token(text: &str, len: usize) -> Option<String> {
    text.split(|c: char| !c.is_ascii_hexdigit()).find(|t| t.len() == len).map(str::to_lowercase)
}

/// api.adoptium.net/v3/assets/latest/<major>/hotspot -> the Windows JRE zip, if newer than `current`
pub fn parse_adoptium(json: &str, current: &str) -> Option<Candidate> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let rel = v.as_array()?.first()?;
    let version = rel["release_name"].as_str()?.strip_prefix("jdk-")?.split('+').next()?.to_string();
    if cmp_ver(&version, current).is_le() {
        return None;
    }
    let pkg = &rel["binary"]["package"];
    Some(plain(&version, current, "Java", pkg["link"].as_str()?.into(), pkg["checksum"].as_str()?.to_lowercase(), "*", "https://adoptium.net/temurin/release-notes/".into(), vec![]))
}

/// api.adoptium.net/v3/info/available_releases -> the newest long-term release
pub fn parse_adoptium_lts(json: &str) -> Option<u32> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v["available_lts_releases"].as_array()?.iter().filter_map(|x| x.as_u64()).max().map(|m| m as u32)
}

/// Directory listing of archive.apache.org/dist/tomcat/tomcat-N/ -> release versions, oldest first (no milestones)
pub fn parse_tomcat_versions(html: &str) -> Vec<String> {
    let mut v: Vec<String> = html.split("href=\"v").skip(1).filter_map(|r| r.split('/').next()).filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.')).map(str::to_string).collect();
    v.sort_by(|a, b| cmp_ver(a, b));
    v.dedup();
    v
}

pub fn tomcat_candidate(version: &str, sha512_text: &str, current: &str) -> Option<Candidate> {
    let sha = hex_token(sha512_text, 128)?;
    let major = *parse_ver(version).first()?;
    let extra = if kind_of(current, version) == "major" { vec!["Tomcat moves to a newer Jakarta EE here: web applications built for the old one may need changes.".to_string()] } else { vec![] };
    Some(plain(
        version,
        current,
        "Tomcat",
        format!("https://archive.apache.org/dist/tomcat/tomcat-{major}/v{version}/bin/apache-tomcat-{version}-windows-x64.zip"),
        format!("sha512:{sha}"),
        &format!("apache-tomcat-{version}/"),
        "https://tomcat.apache.org/whichversion.html".into(),
        extra,
    ))
}

/// A GitHub "latest release" JSON -> the asset `asset(version)`, with the digest GitHub shows for it
pub fn parse_github_release(json: &str, current: &str, name: &str, asset: &dyn Fn(&str) -> String, strip: &str) -> Option<Candidate> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let version = v["tag_name"].as_str()?.trim_start_matches('v').to_string();
    if cmp_ver(&version, current).is_le() {
        return None;
    }
    let want = asset(&version);
    let a = v["assets"].as_array()?.iter().find(|a| a["name"] == want.as_str())?;
    let digest = a["digest"].as_str()?.strip_prefix("sha256:")?.to_lowercase();
    Some(plain(&version, current, name, a["browser_download_url"].as_str()?.into(), digest, strip, v["html_url"].as_str()?.into(), vec![]))
}

/// api.wordpress.org/core/version-check/1.7/ -> newer versions of the same line, and the newest release
pub fn parse_wordpress(json: &str, current: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return vec![] };
    let mut out: Vec<String> = v["offers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| o["response"] == "upgrade" || line_of(o["version"].as_str().unwrap_or("")) == line_of(current))
        .filter_map(|o| o["version"].as_str().map(str::to_string))
        .filter(|ver| cmp_ver(ver, current).is_gt())
        .collect();
    out.sort_by(|a, b| cmp_ver(a, b));
    out.dedup();
    out
}

pub fn wp_candidate(version: &str, sha1_text: &str, current: &str) -> Option<Candidate> {
    let sha = hex_token(sha1_text, 40)?;
    let note = "This updates the copy Runqora uses for new sites. Sites you already made are not touched: update each one from its own Dashboard > Updates, after a backup.".to_string();
    Some(plain(version, current, "WordPress", format!("https://downloads.wordpress.org/release/wordpress-{version}.zip"), format!("sha1:{sha}"), "wordpress/", "https://wordpress.org/news/category/releases/".into(), vec![note]))
}

/// nodejs.org/dist/index.json -> newest release of the installed major, and the newest LTS if it is a higher major
pub fn parse_node(index: &str, current: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(index) else { return vec![] };
    let cur_major = parse_ver(current).first().copied().unwrap_or(0);
    let all: Vec<(String, bool)> = v
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["files"].as_array().is_some_and(|f| f.iter().any(|x| x == "win-x64-zip")))
        .filter_map(|e| Some((e["version"].as_str()?.trim_start_matches('v').to_string(), e["lts"].is_string())))
        .collect();
    let mut out = vec![];
    if let Some((ver, _)) = all.iter().filter(|(ver, _)| parse_ver(ver).first() == Some(&cur_major) && cmp_ver(ver, current).is_gt()).max_by(|a, b| cmp_ver(&a.0, &b.0)) {
        out.push(ver.clone());
    }
    if let Some((ver, _)) = all.iter().filter(|(ver, lts)| *lts && parse_ver(ver).first().is_some_and(|m| *m > cur_major)).max_by(|a, b| cmp_ver(&a.0, &b.0)) {
        out.push(ver.clone());
    }
    out
}

pub fn node_candidate(version: &str, shasums: &str, current: &str) -> Option<Candidate> {
    let file = format!("node-v{version}-win-x64.zip");
    let sha = shasums.lines().find(|l| l.trim_end().ends_with(file.as_str())).and_then(|l| hex_token(l, 64))?;
    let mut c = plain(version, current, "Node.js", format!("https://nodejs.org/dist/v{version}/{file}"), sha, "*", format!("https://nodejs.org/en/blog/release/v{version}"), vec![]);
    if parse_ver(version).first() == parse_ver(current).first() {
        c.kind = "patch"; // inside one Node major, minors only add things
        c.warnings.clear();
    }
    Some(c)
}

/// getcomposer.org/versions -> the newest stable version, if newer
pub fn parse_composer(json: &str, current: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let ver = v["stable"].as_array()?.iter().filter_map(|e| e["version"].as_str()).max_by(|a, b| cmp_ver(a, b))?.to_string();
    cmp_ver(&ver, current).is_gt().then_some(ver)
}

pub fn composer_candidate(version: &str, sha_text: &str, current: &str) -> Option<Candidate> {
    let sha = hex_token(sha_text, 64)?;
    let mut c = plain(version, current, "Composer", format!("https://getcomposer.org/download/{version}/composer.phar"), sha, "", format!("https://github.com/composer/composer/releases/tag/{version}"), vec![]);
    if parse_ver(version).first() == parse_ver(current).first() {
        c.kind = "patch";
        c.warnings.clear();
    }
    Some(c)
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
        "tomcat" if running("tomcat") => Some("Stop Tomcat first.".into()),
        "jre" if running("tomcat") => Some("Stop Tomcat first: it is running on this Java.".into()),
        "mailpit" if running("mailpit") => Some("Stop Mailpit first.".into()),
        "sftpgo" if running("sftpgo") => Some("Stop SFTPGo first.".into()),
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
        "jre" => {
            let major = parse_ver(current).first().copied().unwrap_or(21);
            let q = |m: u32| format!("https://api.adoptium.net/v3/assets/latest/{m}/hotspot?architecture=x64&image_type=jre&os=windows");
            let mut out: Vec<Candidate> = parse_adoptium(&http_get(&q(major))?, current).into_iter().collect();
            if let Some(lts) = parse_adoptium_lts(&http_get("https://api.adoptium.net/v3/info/available_releases")?).filter(|l| *l > major) {
                out.extend(parse_adoptium(&http_get(&q(lts))?, current).map(|mut c| {
                    c.warnings.push("Programs written for an older Java usually run on a newer one, but test your Tomcat applications.".into());
                    c
                }));
            }
            Ok(out)
        }
        "tomcat" => {
            let major = parse_ver(current).first().copied().unwrap_or(10);
            let mut out = vec![];
            for m in [major, major + 1] {
                let page = match http_get(&format!("https://archive.apache.org/dist/tomcat/tomcat-{m}/")) {
                    Ok(p) => p,
                    Err(_) if m > major => continue, // the next major simply may not exist
                    Err(e) => return Err(e),
                };
                let Some(v) = parse_tomcat_versions(&page).into_iter().filter(|v| cmp_ver(v, current).is_gt()).last() else { continue };
                let sha = http_get(&format!("https://archive.apache.org/dist/tomcat/tomcat-{m}/v{v}/bin/apache-tomcat-{v}-windows-x64.zip.sha512"))?;
                out.extend(tomcat_candidate(&v, &sha, current));
            }
            Ok(out)
        }
        "mailpit" => Ok(parse_github_release(&http_get("https://api.github.com/repos/axllent/mailpit/releases/latest")?, current, "Mailpit", &|_| "mailpit-windows-amd64.zip".into(), "").into_iter().collect()),
        "sftpgo" => Ok(parse_github_release(&http_get("https://api.github.com/repos/drakkan/sftpgo/releases/latest")?, current, "SFTPGo", &|v| format!("sftpgo_v{v}_windows_portable.zip"), "*").into_iter().collect()),
        "adminer" => Ok(parse_github_release(&http_get("https://api.github.com/repos/vrana/adminer/releases/latest")?, current, "Adminer", &|v| format!("adminer-{v}.php"), "").into_iter().collect()),
        "wordpress" => {
            let mut out = vec![];
            for v in parse_wordpress(&http_get(&format!("https://api.wordpress.org/core/version-check/1.7/?version={current}"))?, current) {
                let sha = http_get(&format!("https://wordpress.org/wordpress-{v}.zip.sha1"))?;
                out.extend(wp_candidate(&v, &sha, current));
            }
            Ok(out)
        }
        "node" => {
            let mut out = vec![];
            for v in parse_node(&http_get("https://nodejs.org/dist/index.json")?, current) {
                let sums = http_get(&format!("https://nodejs.org/dist/v{v}/SHASUMS256.txt"))?;
                out.extend(node_candidate(&v, &sums, current));
            }
            Ok(out)
        }
        "composer" => match parse_composer(&http_get("https://getcomposer.org/versions")?, current) {
            Some(v) => Ok(composer_candidate(&v, &http_get(&format!("https://getcomposer.org/download/{v}/composer.phar.sha256sum"))?, current).into_iter().collect()),
            None => Ok(vec![]),
        },
        _ => Ok(vec![]),
    }
}

/// The component as the publisher offers it today: the newest version of the line the manifest pins (else the newest
/// there is), with its own download address and checksum. Used when the pinned link no longer works.
pub fn installable_now(c: &Component) -> Result<Component, String> {
    let all = candidates_for(c, &root(), &c.version, None)?;
    let line = line_of(&c.version);
    let best = all.iter().filter(|x| x.line == line).max_by(|a, b| cmp_ver(&a.version, &b.version)).or_else(|| all.iter().max_by(|a, b| cmp_ver(&a.version, &b.version))).ok_or("the publisher offers nothing newer")?;
    Ok(with_candidate(c, best))
}

/// `c` pointed at a candidate: its version, address, archive layout and checksum.
pub fn with_candidate(c: &Component, cand: &Candidate) -> Component {
    let (algo, hex) = cand.sha256.split_once(':').unwrap_or(("sha256", cand.sha256.as_str()));
    let (mut sha256, mut sha512, mut sha1) = (String::new(), String::new(), String::new());
    match algo {
        "sha512" => sha512 = hex.to_string(),
        "sha1" => sha1 = hex.to_string(),
        _ => sha256 = hex.to_string(),
    }
    Component { version: cand.version.clone(), url: cand.url.clone(), strip_prefix: cand.strip_prefix.clone(), sha256, sha512, sha1, ..c.clone() }
}

/// What a probe against the real publishers returns for an installed-or-not module, pretending `current` is installed.
pub fn probe(id: &str, current: &str) -> Result<Vec<Candidate>, String> {
    let c = component(id)?;
    let pma_json = http_get("https://www.phpmyadmin.net/home_page/version.json").ok();
    candidates_for(&c, &root(), current, pma_json.as_deref())
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
        "tomcat" => vec![("conf", Keep::Copy), ("webapps", Keep::Copy), ("logs", Keep::Move)],
        "mailpit" => vec![("mailpit.db", Keep::Move)],
        // the database, its settings and the host keys SFTPGo made for itself
        "sftpgo" => vec![("sftpgo.db", Keep::Move), ("sftpgo.json", Keep::Copy), ("id_rsa", Keep::Copy), ("id_rsa.pub", Keep::Copy), ("id_ecdsa", Keep::Copy), ("id_ecdsa.pub", Keep::Copy), ("id_ed25519", Keep::Copy), ("id_ed25519.pub", Keep::Copy)],
        "composer" => vec![("composer.bat", Keep::Copy)],
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
    if !root.join(&c.binary).exists() {
        return Err(format!("{} is missing from the new version", c.binary));
    }
    let got = detect_version(c, root).ok_or("the new files do not report a version")?;
    if got != want {
        return Err(format!("expected {want} but found {got}"));
    }
    let mut warnings = vec![];
    match c.id.as_str() {
        "jre" if !run_capture(&root.join(&c.binary), &["-version"]).unwrap_or_default().to_lowercase().contains("version") => return Err("the new Java does not start".into()),
        "node" if !run_capture(&root.join(&c.binary), &["-v"]).unwrap_or_default().trim_start().starts_with('v') => return Err("the new Node.js does not start".into()),
        "mailpit" if run_capture(&root.join(&c.binary), &["version"]).unwrap_or_default().trim().is_empty() => return Err("the new Mailpit does not start".into()),
        "tomcat" if !root.join("tomcat/bin/bootstrap.jar").exists() => return Err("Tomcat's bootstrap.jar is missing".into()),
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
    check_hash(&cand.sha256, zip)?;
    let live = root.join(&c.extract_to);
    let old = detect_version(c, root).unwrap_or_else(|| c.version.clone());
    let stage = root.join("tmp").join(format!("stage-{}", c.id));
    remove_any(&stage);
    unpack_component(c, zip, &stage, &cand.strip_prefix)?;
    write_marker(&stage, &cand.version);
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
