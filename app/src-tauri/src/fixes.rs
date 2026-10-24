//! Fixing what is wrong without editing files: free a port, pick another one, and repair a setup that drifted
//! (a moved install folder, a duplicated PHP extension, a config pointing at the wrong port).
//!
//! Nothing here trusts the window: a process is only ended if it is currently listening on a port, is not part of
//! Windows, and is not Runqora itself, and the answer to "may I?" is recomputed on every call.

use crate::CommandExt as _;
use crate::config::{ini_get, ini_set, parse_line};
use crate::{component, component_with, describe_blocker, fwd, manifest, read_pid, refresh_extra, replace_pma_port, rewrite_conf, settings_in, state, win, Component};
use serde::Serialize;
use std::{collections::BTreeSet, fs, path::Path, process::Command, thread::sleep, time::Duration};

// ---- ports ----

fn pid_alive(pid: u32) -> bool {
    win::pid_alive(pid)
}

#[derive(Serialize, Clone, Debug)]
pub struct Remedy {
    pub pid: u32,
    pub name: String,
    /// end | service | none
    pub kind: &'static str,
    /// Windows services that hold the port (stop one instead of killing the host process)
    pub services: Vec<String>,
    /// what we can offer, or why we cannot
    pub detail: String,
}

const PROTECTED: &[&str] = &["init", "systemd", "launchd", "kernel_task", "system", "registry", "smss.exe", "csrss.exe", "wininit.exe", "services.exe", "lsass.exe", "winlogon.exe", "dwm.exe"];

/// Service names hosted by a process (`tasklist /svc`), no admin rights needed.
#[cfg(unix)]
pub fn services_in_process(_pid: u32) -> Vec<String> {
    Vec::new() // no Windows services here
}

#[cfg(windows)]
pub fn services_in_process(pid: u32) -> Vec<String> {
    let out = Command::new("tasklist").args(["/svc", "/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]).creation_flags(0x0800_0000).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    out.lines().next().and_then(|l| l.split("\",\"").nth(2)).map(|s| s.trim_matches('"').split(',').map(str::trim).filter(|s| !s.is_empty() && *s != "N/A").map(String::from).collect()).unwrap_or_default()
}

pub fn remedy(pid: u32, root: &Path) -> Remedy {
    let b = describe_blocker(pid, root);
    let lname = b.name.to_lowercase();
    let mk = |kind, services: Vec<String>, detail: String| Remedy { pid, name: b.name.clone(), kind, services, detail };
    if pid == std::process::id() {
        return mk("none", vec![], "That is Runqora itself.".into());
    }
    if b.ours {
        return mk("end", vec![], format!("{} is a leftover from this install. Ending it frees the port.", b.name));
    }
    let hosted: Vec<String> = if pid == 4 { vec!["W3SVC".into()] } else { services_in_process(pid) };
    let hosted: Vec<String> = hosted.into_iter().filter(|s| win::service_exists(s)).collect();
    if pid == 4 || lname == "svchost.exe" || !hosted.is_empty() {
        return if hosted.is_empty() {
            mk("none", vec![], "Windows itself is using this port. Choose another port for Runqora instead.".into())
        } else {
            let list = hosted.join(", ");
            mk("service", hosted, format!("The port is held by the Windows service {list}. Stopping the service frees it without ending a system process."))
        };
    }
    if PROTECTED.contains(&lname.as_str()) || pid <= 4 {
        return mk("none", vec![], "Windows needs this program. Choose another port for Runqora instead.".into());
    }
    mk("end", vec![], format!("{} is using the port. Ending it frees the port, and anything unsaved in it is lost.", b.name))
}

fn listening(pid: u32) -> bool {
    win::listeners().iter().any(|(_, p)| *p == pid)
}

fn gone(pid: u32) -> bool {
    (0..30).any(|_| {
        sleep(Duration::from_millis(100));
        !pid_alive(pid)
    })
}

/// End the program holding a port. Asks Windows for administrator approval only if a normal attempt is refused.
pub fn end_process(root: &Path, pid: u32) -> Result<String, String> {
    let r = remedy(pid, root);
    if r.kind != "end" {
        return Err(r.detail);
    }
    if !listening(pid) {
        return Err(format!("{} is no longer holding a port", r.name));
    }
    match win::terminate(pid) {
        Ok(()) => {}
        Err(5) => {
            let rc = win::run_elevated("taskkill", &format!("/PID {pid} /F"))?;
            if rc != 0 && pid_alive(pid) {
                return Err(format!("could not end {} (taskkill returned {rc})", r.name));
            }
        }
        Err(e) => return Err(format!("could not end {} (error {e}; it may belong to another user)", r.name)),
    }
    if gone(pid) { Ok(format!("{} was ended and its port is free", r.name)) } else { Err(format!("{} is still running", r.name)) }
}

/// Stop a Windows service that holds a port. The service must be one `remedy` named for this very process.
pub fn stop_service(root: &Path, pid: u32, name: &str) -> Result<String, String> {
    let r = remedy(pid, root);
    if r.kind != "service" || !r.services.iter().any(|s| s == name) {
        return Err("that service is not holding the port".into());
    }
    win::run_elevated("net", &format!("stop {name}"))?;
    let freed = (0..50).any(|_| {
        sleep(Duration::from_millis(100));
        !listening(pid)
    });
    if freed { Ok(format!("The {name} service was stopped and its port is free")) } else { Err(format!("{name} did not stop, or another service still holds the port")) }
}

/// A free port to use instead: well-known alternatives first, then the next numbers up.
pub fn suggest_port(root: &Path, id: &str) -> Option<u16> {
    let s = settings_in(root);
    let c = component_with(id, &s).ok().filter(|c| c.args.is_some())?;
    let taken: BTreeSet<u16> = win::listeners().into_iter().map(|(p, _)| p).chain(manifest().components.iter().filter(|o| o.args.is_some() && o.id != id).filter_map(|o| component_with(&o.id, &s).ok()).map(|o| o.port)).collect();
    let favourites: &[u16] = match id {
        "apache" => &[8080, 8000, 8888, 8081, 8088, 9000],
        "mariadb" => &[3307, 3308, 3316, 3406, 13306],
        _ => &[],
    };
    favourites.iter().copied().chain(c.port.saturating_add(1)..=c.port.saturating_add(200)).find(|p| !taken.contains(p) && *p != c.port)
}

// ---- check setup ----

#[derive(Serialize, Clone, Debug)]
pub struct Problem {
    pub id: String,
    pub module: String,
    /// error | warning
    pub severity: &'static str,
    pub title: String,
    pub detail: String,
    /// the fix to ask for, when there is an automatic one
    pub fix: Option<&'static str>,
    /// set for a blocked port, so the window can offer End / Stop service / Use another port
    pub port: Option<u16>,
    pub pid: Option<u32>,
}

fn same_path(a: &str, b: &str) -> bool {
    let n = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_lowercase();
    n(a) == n(b)
}

fn server_root(conf: &str) -> Option<String> {
    conf.lines().find_map(|l| {
        let l = l.trim();
        let v = l.strip_prefix("Define SRVROOT").or_else(|| l.strip_prefix("ServerRoot"))?;
        let v = v.trim().trim_matches('"');
        (!v.contains("${")).then(|| v.to_string())
    })
}

fn listen_port(conf: &str) -> Option<u16> {
    conf.lines().find_map(|l| l.trim().strip_prefix("Listen ")?.trim().rsplit(':').next()?.parse().ok())
}

fn p(id: &str, module: &str, severity: &'static str, title: impl Into<String>, detail: impl Into<String>, fix: Option<&'static str>) -> Problem {
    Problem { id: id.into(), module: module.into(), severity, title: title.into(), detail: detail.into(), fix, port: None, pid: None }
}

/// Find everything Runqora can see is wrong. Cheap and read-only.
pub fn diagnose(root: &Path) -> Vec<Problem> {
    let s = settings_in(root);
    let mut out = vec![];
    let has = |id: &str| component(id).is_ok_and(|c| root.join(&c.binary).exists());
    let port_of = |id: &str| component_with(id, &s).map(|c| c.port).unwrap_or(0);

    if has("apache") {
        let conf = fs::read_to_string(root.join("apache/conf/httpd.conf")).unwrap_or_default();
        let dir = fwd(&root.join("apache"));
        if conf.is_empty() {
            out.push(p("apache.conf", "apache", "error", "Apache has no configuration file", "conf/httpd.conf is missing. Reinstall Apache.", None));
        } else {
            if server_root(&conf).is_some_and(|r| !same_path(&r, &dir)) {
                out.push(p("apache.root", "apache", "error", "Apache points at the wrong folder", format!("Its configuration says {} but it lives in {dir}. This happens when the install folder is moved.", server_root(&conf).unwrap_or_default()), Some("apache.paths")));
            }
            if listen_port(&conf).is_some_and(|l| l != port_of("apache")) {
                out.push(p("apache.port", "apache", "error", "Apache listens on a different port than Settings says", format!("Settings: {}. httpd.conf: {}.", port_of("apache"), listen_port(&conf).unwrap_or(0)), Some("apache.paths")));
            }
            let inc = format!("Include {}", crate::EXTRA_CONF);
            if conf.matches(&inc).count() > 1 || conf.contains("runtime-easy.conf") {
                out.push(p("apache.dupinclude", "apache", "error", "Apache loads Runqora's settings twice", "httpd.conf includes the generated file more than once (or by its old name), so Apache refuses to start.", Some("apache.paths")));
            }
            if !conf.contains(&format!("Include {}", crate::EXTRA_CONF)) || !root.join("apache").join(crate::EXTRA_CONF).exists() {
                out.push(p("apache.include", "apache", "error", "Apache is missing Runqora's generated settings", "PHP, phpMyAdmin and the website folder are wired in through one generated file, and it is not connected.", Some("apache.paths")));
            }
        }
        let docroot = crate::document_root(root, &s);
        if !docroot.is_dir() {
            out.push(p("apache.docroot", "apache", "error", "The website folder is missing", format!("Apache serves {} but that folder does not exist, so Apache will not start. It may have been moved or deleted.", docroot.display()), Some("apache.paths")));
        }
        if !root.join("apache/logs").is_dir() {
            out.push(p("apache.logs", "apache", "error", "Apache has no logs folder", "It will not start without one.", Some("apache.paths")));
        }
    }
    if has("mariadb") {
        let ini = fs::read_to_string(root.join("mariadb/my.ini")).unwrap_or_default();
        let dir = fwd(&root.join("mariadb"));
        let mut wrong = vec![];
        for (k, want) in [("basedir", dir.clone()), ("datadir", format!("{dir}/data"))] {
            if ini_get(&ini, Some("mysqld"), k).is_some_and(|v| !same_path(&v, &want)) {
                wrong.push(k);
            }
        }
        if !wrong.is_empty() {
            out.push(p("mariadb.root", "mariadb", "error", "MariaDB points at the wrong folder", format!("{} in my.ini do not match {dir}. This happens when the install folder is moved.", wrong.join(" and ")), Some("mariadb.paths")));
        }
        if ini_get(&ini, Some("mysqld"), "port").is_some_and(|v| v.parse::<u16>().ok() != Some(port_of("mariadb"))) {
            out.push(p("mariadb.port", "mariadb", "error", "MariaDB listens on a different port than Settings says", format!("Settings: {}. my.ini: {}.", port_of("mariadb"), ini_get(&ini, Some("mysqld"), "port").unwrap_or_default()), Some("mariadb.paths")));
        }
        if !root.join("mariadb/data/mysql").is_dir() {
            out.push(p("mariadb.data", "mariadb", "error", "The database folder is empty", "MariaDB has no system tables to start from. Reinstall MariaDB (your databases are not touched if the data folder exists).", None));
        }
    }
    if has("php") {
        let ini = fs::read_to_string(root.join("php/php.ini")).unwrap_or_default();
        if ini.is_empty() {
            out.push(p("php.ini", "php", "error", "PHP has no php.ini", "Without it PHP runs with no extensions, so phpMyAdmin cannot work.", Some("php.ini")));
        } else {
            let want = format!("{}/ext", fwd(&root.join("php")));
            if ini_get(&ini, None, "extension_dir").is_none_or(|v| !same_path(&v, &want)) {
                out.push(p("php.extdir", "php", "error", "PHP cannot find its extensions", format!("extension_dir should be {want}. This happens when the install folder is moved."), Some("php.paths")));
            }
            let dups = duplicate_extensions(&ini);
            if !dups.is_empty() {
                out.push(p("php.dups", "php", "warning", "PHP loads some extensions twice", format!("{} appear more than once in php.ini, which prints a warning on every start.", dups.join(", ")), Some("php.paths")));
            }
        }
    }
    if has("phpmyadmin") && has("mariadb") {
        let cfg = fs::read_to_string(root.join("phpmyadmin/config.inc.php")).unwrap_or_default();
        let want = port_of("mariadb");
        if cfg.lines().find(|l| l.contains("['port']")).is_some_and(|l| !l.contains(&format!("'{want}'"))) {
            out.push(p("pma.port", "phpmyadmin", "error", "phpMyAdmin looks for the database on the wrong port", format!("MariaDB uses {want}."), Some("pma.port")));
        }
    }
    for c in manifest().components.iter().filter(|c| c.args.is_some()).filter_map(|c| component_with(&c.id, &s).ok()).filter(|c| root.join(&c.binary).exists()) {
        let st = state(&c, root);
        if st.status == "stopped" && read_pid(&c, root).is_some() {
            out.push(p(&format!("stale.{}", c.id), &c.id, "warning", format!("{} left a stale process file", c.name), "A crash left a pid file behind. It is harmless but can confuse other tools.", Some("stale")));
        }
        let blocked = st.status == "blocked";
        if let Some(b) = st.blocker.filter(|_| blocked) {
            let mut q = p(&format!("blocked.{}", c.id), &c.id, "error", format!("Port {} is used by {}", c.port, b.name), format!("{} cannot start until the port is free.", c.name), None);
            q.port = Some(c.port);
            q.pid = Some(b.pid);
            out.push(q);
        }
    }
    out
}

/// Names of extensions that are switched on by more than one active line.
pub fn duplicate_extensions(ini: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut dups = BTreeSet::new();
    for l in ini.lines() {
        if let Some((false, k, v)) = parse_line(l) {
            if k == "extension" {
                let n = v.trim_matches('"').trim().trim_start_matches("php_").trim_end_matches(".dll").to_string();
                if !seen.insert(n.clone()) {
                    dups.insert(n);
                }
            }
        }
    }
    dups.into_iter().collect()
}

/// Comment out every active `extension=` line after the first for the same extension.
pub fn dedupe_extensions(ini: &str) -> String {
    let mut seen = BTreeSet::new();
    ini.lines()
        .map(|l| match parse_line(l) {
            Some((false, k, v)) if k == "extension" && !seen.insert(v.trim_matches('"').trim().trim_start_matches("php_").trim_end_matches(".dll").to_string()) => format!(";{l} ; duplicate"),
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// Apply one fix by its id (from `Problem.fix`). Returns what was done.
pub fn apply_fix(root: &Path, fix: &str, module: &str) -> Result<String, String> {
    let s = settings_in(root);
    let c: Component = component_with(module, &s)?;
    let write = |rel: &str, f: &dyn Fn(&str) -> String| -> Result<(), String> {
        let path = root.join(rel);
        let t = fs::read_to_string(&path).map_err(|e| format!("{rel}: {e}"))?;
        let b = path.with_extension(format!("{}.runqora-backup", path.extension().and_then(|e| e.to_str()).unwrap_or("")));
        if !b.exists() {
            let _ = fs::copy(&path, b);
        }
        fs::write(&path, f(&t)).map_err(|e| e.to_string())
    };
    match fix {
        "apache.paths" => {
            fs::create_dir_all(root.join("apache/logs")).map_err(|e| e.to_string())?;
            write("apache/conf/httpd.conf", &|t| rewrite_conf(t, &root.join("apache"), c.port))?;
            refresh_extra(root)?;
            Ok("Apache's folders, port and generated settings are connected again".into())
        }
        "mariadb.paths" => {
            let dir = fwd(&root.join("mariadb"));
            write("mariadb/my.ini", &|t| {
                let t = ini_set(t, Some("mysqld"), "basedir", &format!("\"{dir}\""));
                let t = ini_set(&t, Some("mysqld"), "datadir", &format!("\"{dir}/data\""));
                let t = ini_set(&t, Some("mysqld"), "pid-file", &format!("\"{dir}/data/mysql.pid\""));
                crate::replace_port_lines(&t, c.port)
            })?;
            Ok("MariaDB's folders and port in my.ini match this install again".into())
        }
        "php.paths" => {
            write("php/php.ini", &|t| ini_set(&dedupe_extensions(t), None, "extension_dir", &format!("\"{}/ext\"", fwd(&root.join("php")))))?;
            Ok("PHP's extension folder is right and each extension loads once".into())
        }
        "php.ini" => {
            let dev = fs::read_to_string(root.join("php/php.ini-development")).map_err(|_| "php.ini-development is missing; reinstall PHP".to_string())?;
            fs::write(root.join("php/php.ini"), crate::configure_php_ini(&dev, &root.join("php"))).map_err(|e| e.to_string())?;
            Ok("PHP has a php.ini again, with the usual extensions on".into())
        }
        "pma.port" => {
            let port = component_with("mariadb", &s)?.port;
            write("phpmyadmin/config.inc.php", &|t| replace_pma_port(t, port))?;
            Ok(format!("phpMyAdmin now looks for the database on port {port}"))
        }
        "stale" => {
            let _ = fs::remove_file(root.join(&c.pidfile));
            Ok(format!("Removed the stale process file of {}", c.name))
        }
        other => Err(format!("I do not know how to fix {other}")),
    }
}

/// Apply every automatic fix that diagnose offers, once each. Returns one line per fix.
pub fn fix_all(root: &Path) -> Vec<Result<String, String>> {
    let mut done = BTreeSet::new();
    diagnose(root).into_iter().filter_map(|p| p.fix.map(|f| (f, p.module))).filter(|k| done.insert(k.clone())).map(|(f, m)| apply_fix(root, f, &m)).collect()
}

// the fixtures of these tests are Windows programs and services: Linux and macOS are covered by e2e/unix.sh
#[cfg(all(test, windows))]
mod tests;
