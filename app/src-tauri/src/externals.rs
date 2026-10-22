//! Programs Runqora does not install but can run for you when they are already on this PC: FileZilla Server and Mercury Mail.
//! Both are licensed for their own installers only, so Runqora never downloads them; it finds them, starts and stops them and
//! opens their own admin window. Their settings stay in their own windows.

use crate::{win, Blocker, NO_WINDOW};
use serde::Serialize;
use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};

struct External {
    id: &'static str,
    name: &'static str,
    /// where the program is normally installed
    exes: &'static [&'static str],
    /// Windows service that runs it; None = a normal program Runqora starts itself
    service: Option<&'static str>,
    /// the window for its settings
    gui: Option<&'static str>,
    /// its usual port; changed inside its own program
    port: u16,
    get_url: &'static str,
}

const EXTERNALS: &[External] = &[
    External {
        id: "filezilla",
        name: "FileZilla Server",
        exes: &[r"C:\Program Files\FileZilla Server\filezilla-server.exe", r"C:\Program Files (x86)\FileZilla Server\FileZilla Server.exe"],
        service: Some("FileZilla Server"),
        gui: Some("filezilla-server-gui.exe"),
        port: 21,
        get_url: "https://filezilla-project.org/download.php?type=server",
    },
    External {
        id: "mercury",
        name: "Mercury Mail",
        exes: &[r"C:\Mercury\mercury.exe", r"C:\xampp\MercuryMail\mercury.exe"],
        service: None,
        gui: None,
        port: 25,
        get_url: "https://www.pmail.com/downloads_s3_t.htm",
    },
];

#[derive(Serialize)]
pub struct ExternalState {
    pub id: String,
    pub name: String,
    /// missing | stopped | running | blocked
    pub status: &'static str,
    pub pid: Option<u32>,
    pub port: u16,
    pub path: String,
    pub has_gui: bool,
    pub blocker: Option<Blocker>,
}

fn find(e: &External) -> Option<PathBuf> {
    e.exes.iter().map(PathBuf::from).find(|p| p.exists())
}

fn exe_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// installed? and who listens on its port (process name, if anyone) -> status
pub fn classify(installed: bool, listener: Option<&str>, exe: &str) -> &'static str {
    match (installed, listener) {
        (false, _) => "missing",
        (true, None) => "stopped",
        (true, Some(n)) if n.eq_ignore_ascii_case(exe) => "running",
        (true, Some(_)) => "blocked",
    }
}

fn state_of(e: &External, root: &Path) -> ExternalState {
    let path = find(e);
    let exe = path.as_deref().map(exe_name).unwrap_or_default();
    let listener = win::listener_pid(e.port);
    let name = listener.and_then(win::process_name);
    let status = classify(path.is_some(), name.as_deref(), &exe);
    ExternalState {
        id: e.id.into(),
        name: e.name.into(),
        status,
        pid: listener.filter(|_| status == "running"),
        port: e.port,
        path: path.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        has_gui: e.gui.is_some_and(|g| find(e).and_then(|p| p.parent().map(|d| d.join(g))).is_some_and(|p| p.exists())),
        blocker: listener.filter(|_| status == "blocked").map(|p| crate::describe_blocker(p, root)),
    }
}

pub fn all(root: &Path) -> Vec<ExternalState> {
    EXTERNALS.iter().map(|e| state_of(e, root)).collect()
}

fn get(id: &str) -> Result<&'static External, String> {
    EXTERNALS.iter().find(|e| e.id == id).ok_or_else(|| format!("unknown program {id}"))
}

pub fn start(root: &Path, id: &str) -> Result<String, String> {
    let e = get(id)?;
    let s = state_of(e, root);
    match s.status {
        "missing" => return Err(format!("{} is not installed on this PC; use Get it to open its download page", e.name)),
        "running" => return Ok(format!("{} is already running", e.name)),
        "blocked" => return Err(format!("port {} is used by {}; {} was not started", e.port, s.blocker.map(|b| b.name).unwrap_or_default(), e.name)),
        _ => {}
    }
    match e.service {
        Some(svc) => {
            let rc = win::run_elevated("net", &format!("start \"{svc}\""))?;
            if rc != 0 && win::listener_pid(e.port).is_none() {
                return Err(format!("{} did not start (net start returned {rc})", e.name));
            }
        }
        None => {
            Command::new(&s.path).current_dir(Path::new(&s.path).parent().unwrap_or(root)).creation_flags(NO_WINDOW).spawn().map_err(|e| e.to_string())?;
        }
    }
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if state_of(e, root).status == "running" {
            return Ok(format!("{} started", e.name));
        }
    }
    Err(format!("{} did not start listening on port {}; open it to see why", e.name, e.port))
}

pub fn stop(root: &Path, id: &str) -> Result<String, String> {
    let e = get(id)?;
    let s = state_of(e, root);
    let Some(pid) = s.pid else { return Ok(format!("{} is not running", e.name)) };
    match e.service {
        Some(svc) => {
            win::run_elevated("net", &format!("stop \"{svc}\""))?;
        }
        None => win::terminate(pid).map_err(|c| format!("could not end {} (error {c})", e.name))?,
    }
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if state_of(e, root).status != "running" {
            return Ok(format!("{} stopped", e.name));
        }
    }
    Err(format!("{} is still running", e.name))
}

/// Open the program's own settings window.
pub fn open_gui(root: &Path, id: &str) -> Result<(), String> {
    let e = get(id)?;
    let s = state_of(e, root);
    let target = match e.gui {
        Some(g) => Path::new(&s.path).parent().map(|d| d.join(g)).filter(|p| p.exists()),
        None => Some(PathBuf::from(&s.path)).filter(|p| p.exists()),
    }
    .ok_or_else(|| format!("{} is not installed on this PC", e.name))?;
    Command::new(&target).current_dir(target.parent().unwrap_or(root)).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// The page where the program is downloaded from its own publisher (never downloaded by Runqora).
pub fn get_url(id: &str) -> Result<&'static str, String> {
    Ok(get(id)?.get_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_follows_what_is_installed_and_who_holds_the_port() {
        assert_eq!(classify(false, None, "x.exe"), "missing");
        assert_eq!(classify(false, Some("x.exe"), "x.exe"), "missing", "not installed here, so it is somebody else's");
        assert_eq!(classify(true, None, "x.exe"), "stopped");
        assert_eq!(classify(true, Some("X.EXE"), "x.exe"), "running");
        assert_eq!(classify(true, Some("w3wp.exe"), "x.exe"), "blocked");
    }

    #[test]
    fn both_programs_are_known_and_only_official_pages_are_offered() {
        let r = std::env::temp_dir();
        let ids: Vec<_> = all(&r).into_iter().map(|s| s.id).collect();
        assert_eq!(ids, ["filezilla", "mercury"]);
        assert!(get_url("filezilla").unwrap().starts_with("https://filezilla-project.org/"));
        assert!(get_url("mercury").unwrap().starts_with("https://www.pmail.com/"));
        assert!(get_url("nope").is_err() && start(&r, "nope").is_err());
        // not installed in the test environment's sense: starting says so instead of failing obscurely
        if all(&r).iter().any(|s| s.id == "mercury" && s.status == "missing") {
            assert!(start(&r, "mercury").unwrap_err().contains("not installed"));
        }
    }
}
