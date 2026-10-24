//! Sites: a folder served at http://name.localhost/ (and https:// when HTTPS is on), plus one-click WordPress.
//! `*.localhost` resolves to this PC in every modern browser, so no hosts-file edit (and no administrator rights) is needed.

use crate::CommandExt as _;
use crate::{component, document_root, https, random_hex, refresh_extra, save_settings_in, settings_in, state, Settings, NO_WINDOW};
use serde::Serialize;
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Serialize, Clone, Debug)]
pub struct Site {
    pub name: String,
    pub host: String,
    pub folder: String,
    pub exists: bool,
    pub url: String,
    pub https_url: Option<String>,
}

/// Names that would shadow something Runqora already serves.
const RESERVED: &[&str] = &["localhost", "phpmyadmin", "www"];

pub fn check_name(name: &str) -> Result<String, String> {
    let n = name.trim().to_lowercase();
    let ok = (1..=63).contains(&n.len()) && !n.starts_with('-') && !n.ends_with('-') && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !ok {
        return Err(format!("'{n}' is not a site name (use letters, digits and dashes, like shop or my-blog)"));
    }
    if RESERVED.contains(&n.as_str()) {
        return Err(format!("'{n}' is reserved; pick another name"));
    }
    Ok(n)
}

fn urls(root: &Path, s: &Settings, host: &str) -> (String, Option<String>) {
    let port = crate::component_with("apache", s).map(|c| c.port).unwrap_or(80);
    let http = if port == 80 { format!("http://{host}/") } else { format!("http://{host}:{port}/") };
    let tls = s.custom.get("apache.https").is_some_and(|v| v == "on") && https::ssl_dir(root).join("server.crt").exists();
    let hp = s.custom.get("apache.https_port").and_then(|p| p.parse::<u16>().ok()).unwrap_or(443);
    (http, tls.then(|| if hp == 443 { format!("https://{host}/") } else { format!("https://{host}:{hp}/") }))
}

pub fn list(root: &Path) -> Vec<Site> {
    let s = settings_in(root);
    s.sites
        .iter()
        .map(|(name, folder)| {
            let host = format!("{name}.localhost");
            let (url, https_url) = urls(root, &s, &host);
            Site { name: name.clone(), host, folder: folder.clone(), exists: Path::new(folder).is_dir(), url, https_url }
        })
        .collect()
}

/// Add a site. An empty `folder` means a new folder named after the site inside the document root.
pub fn add(root: &Path, name: &str, folder: &str) -> Result<String, String> {
    let name = check_name(name)?;
    let mut s = settings_in(root);
    if s.sites.contains_key(&name) {
        return Err(format!("there is already a site called {name}"));
    }
    let folder = folder.trim();
    let dir: PathBuf = if folder.is_empty() { document_root(root, &s).join(&name) } else { PathBuf::from(folder) };
    let text = dir.to_string_lossy().into_owned();
    if !dir.is_absolute() || text.contains(['"', '\n', '\r']) {
        return Err("the folder must be a full path such as C:\\Sites\\shop, without quotes".into());
    }
    let fresh = !dir.exists() || fs::read_dir(&dir).map_or(true, |mut d| d.next().is_none());
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {text}: {e}"))?;
    if fresh {
        let page = format!("<!doctype html><meta charset=utf-8><title>{name}</title><h1>{name}.localhost works.</h1><p>Put this site's files in <code>{text}</code>.</p>\n");
        let _ = fs::write(dir.join("index.html"), page); // a nicety: failing to write it does not matter
    }
    s.sites.insert(name.clone(), text);
    // the certificate must cover the new name BEFORE it is saved, like every other HTTPS change
    if s.custom.get("apache.https").is_some_and(|v| v == "on") {
        let mut names = https::parse_hostnames(s.custom.get("apache.hostnames").map(String::as_str).unwrap_or("")).unwrap_or_default();
        names.extend(s.sites.keys().map(|n| format!("{n}.localhost")));
        https::ensure(root, &names)?;
    }
    save_settings_in(root, &s)?;
    refresh_extra(root)?;
    let (url, _) = urls(root, &s, &format!("{name}.localhost"));
    Ok(format!("Site {name} is ready at {url} (restart Apache to apply)"))
}

/// Forget a site. Its files are never touched.
pub fn remove(root: &Path, name: &str) -> Result<String, String> {
    let name = name.trim().to_lowercase();
    let mut s = settings_in(root);
    let folder = s.sites.remove(&name).ok_or_else(|| format!("there is no site called {name}"))?;
    save_settings_in(root, &s)?;
    refresh_extra(root)?;
    Ok(format!("Site {name} removed from Runqora; its files are still in {folder} (restart Apache to apply)"))
}

// ---- WordPress ----

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() { copy_tree(&e.path(), &target)? } else { fs::copy(e.path(), target).map(|_| ())? }
    }
    Ok(())
}

/// wp-config.php made from WordPress' own sample: database details and a fresh set of eight secret keys.
pub fn wp_config(sample: &str, db: &str, port: u16, mut salt: impl FnMut() -> Result<String, String>) -> Result<String, String> {
    if !sample.contains("database_name_here") {
        return Err("this WordPress has no usable wp-config-sample.php".into());
    }
    let mut out = Vec::new();
    for l in sample.lines() {
        let l = if l.contains("put your unique phrase here") {
            let (head, _) = l.split_once("'put your unique phrase here'").unwrap_or((l, ""));
            format!("{head}'{}' );", salt()?)
        } else {
            l.replace("database_name_here", db).replace("username_here", "root").replace("'password_here'", "''").replace("'localhost'", &format!("'127.0.0.1:{port}'"))
        };
        out.push(l);
    }
    Ok(out.join("\n") + "\n")
}

/// Database names go inside backticks: only these characters are ever allowed in.
fn db_name(site: &str) -> String {
    format!("wp_{}", site.replace('-', "_"))
}

pub fn wordpress_create(root: &Path, name: &str) -> Result<String, String> {
    let name = check_name(name)?;
    let master = root.join("wordpress");
    if !master.join("wp-load.php").exists() {
        return Err("WordPress is not installed yet; install it from the WordPress card first".into());
    }
    let maria = component("mariadb")?;
    if state(&maria, root).status != "running" {
        return Err("start MariaDB first: WordPress needs its database".into());
    }
    let s = settings_in(root);
    if s.sites.contains_key(&name) {
        return Err(format!("there is already a site called {name}"));
    }
    let dir = document_root(root, &s).join(&name);
    if dir.exists() && fs::read_dir(&dir).map_or(true, |mut d| d.next().is_some()) {
        return Err(format!("{} already has files in it; choose another name", dir.display()));
    }
    let db = db_name(&name);
    let sample = fs::read_to_string(master.join("wp-config-sample.php")).map_err(|e| e.to_string())?;
    let config = wp_config(&sample, &db, maria.port, || random_hex(32))?; // before anything is created
    let client = [crate::exe("mariadb"), crate::exe("mysql")].iter().map(|f| root.join("mariadb/bin").join(f)).find(|p| p.exists()).ok_or("MariaDB's command line client is missing")?;
    let sql = |q: String| -> Result<(), String> {
        let o = Command::new(&client).args(["-h127.0.0.1", &format!("-P{}", maria.port), "-uroot", "-e", &q]).stdin(Stdio::null()).creation_flags(NO_WINDOW).output().map_err(|e| e.to_string())?;
        if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) }
    };
    sql(format!("CREATE DATABASE `{db}` CHARACTER SET utf8mb4")).map_err(|e| format!("could not create the database {db}: {e}"))?;
    let rollback = |why: String| {
        let _ = fs::remove_dir_all(&dir);
        let _ = sql(format!("DROP DATABASE `{db}`")); // we made it a moment ago, so it is empty
        why
    };
    copy_tree(&master, &dir).map_err(|e| rollback(format!("could not copy WordPress: {e}")))?;
    fs::write(dir.join("wp-config.php"), config).map_err(|e| rollback(e.to_string()))?;
    let r = add(root, &name, &dir.to_string_lossy());
    match r {
        Ok(m) => Ok(format!("WordPress site {name} created with database {db}. {m}. Finish setup in the browser (language, site title, admin account).")),
        Err(e) => Err(rollback(e)),
    }
}

// the fixtures of these tests are Windows programs and services: Linux and macOS are covered by e2e/unix.sh
#[cfg(all(test, windows))]
mod tests;
