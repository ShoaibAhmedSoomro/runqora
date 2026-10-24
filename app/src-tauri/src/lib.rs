use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{self, Read},
    sync::Mutex,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread::sleep,
    time::Duration,
};

#[cfg(windows)]
pub use std::os::windows::process::CommandExt;

const NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const MANIFEST: &str = include_str!("../manifest.json");
#[cfg(unix)]
const MANIFEST: &str = include_str!("../manifest.unix.json");

/// The Windows-only `creation_flags(NO_WINDOW)` calls stay as they are; here they put the child in its own process group
/// (so a stop can end the whole tree, and Ctrl-C in a terminal does not reach the servers).
#[cfg(unix)]
pub trait CommandExt {
    fn creation_flags(&mut self, flags: u32) -> &mut Self;
}
#[cfg(unix)]
impl CommandExt for Command {
    fn creation_flags(&mut self, _flags: u32) -> &mut Self {
        std::os::unix::process::CommandExt::process_group(self, 0)
    }
}

/// Open a web address in the default browser. Callers pass addresses they built themselves, never text from the UI.
pub fn open_url(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    let mut c = {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]).creation_flags(NO_WINDOW);
        c
    };
    #[cfg(not(windows))]
    let mut c = {
        let mut c = Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" });
        c.arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        c
    };
    c.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Show a folder in the file manager.
pub fn open_folder(dir: impl AsRef<Path>) -> Result<(), String> {
    let mut c = Command::new(if cfg!(windows) { "explorer.exe" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" });
    c.arg(dir.as_ref());
    #[cfg(not(windows))]
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    c.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// `name` with the platform's program suffix.
pub fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}
const EXTRA_CONF: &str = "conf/extra/runqora.conf";

#[derive(Deserialize, Clone)]
pub struct Component {
    pub id: String,
    pub name: String,
    pub version: String,
    pub url: String,
    /// at least one of the three hashes must be given; the publisher's own hash of the archive
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub sha512: String,
    /// only for publishers that offer nothing stronger (WordPress)
    #[serde(default)]
    pub sha1: String,
    /// folder inside the archive to drop; "*" = the archive's single top folder, whatever it is called
    pub strip_prefix: String,
    pub extract_to: String,
    /// run this instead of `binary` (Tomcat runs java.exe from the JRE)
    #[serde(default)]
    pub exe: String,
    /// environment for the server; {root} and {port} are replaced
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// the process has no pid file of its own: Runqora writes the one it started
    #[serde(default)]
    pub write_pid: bool,
    /// stop by sending `shutdown_text` to this local TCP port (Tomcat's own way)
    #[serde(default)]
    pub shutdown_port: u16,
    #[serde(default)]
    pub shutdown_text: String,
    /// components that must be installed first (Tomcat needs the JRE)
    #[serde(default)]
    pub needs: Vec<String>,
    /// the download is one file (Composer, Adminer) saved as `binary`, not an archive
    #[serde(default)]
    pub single_file: bool,
    /// per-platform downloads ("linux-x86_64", "linux-aarch64", "macos-aarch64"): the entry for this machine replaces the
    /// fields it names; a component that lists variants but none for this machine is not offered here
    #[serde(default)]
    pub variants: BTreeMap<String, Variant>,
    /// also the "is installed" marker
    pub binary: String,
    /// None = library (no process): php, phpmyadmin. {root} is replaced.
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub pidfile: String,
    /// named event that asks the process to exit; {pid} is replaced
    #[serde(default)]
    pub shutdown_event: String,
    #[serde(default)]
    pub port: u16,
    /// {port} and {apache_port} are replaced
    pub admin_url: Option<String>,
    /// files the Config / Logs menus may open, relative to the install dir
    #[serde(default)]
    pub configs: Vec<String>,
    #[serde(default)]
    pub logs: Vec<String>,
    pub service: Option<ServiceSpec>,}

/// Windows service wiring. Arguments get {root} and {service} replaced.
#[derive(Deserialize, Clone)]
pub struct ServiceSpec {
    pub name: String,
    pub install: Vec<String>,
    pub remove: Vec<String>,}

#[derive(Deserialize)]
pub struct Manifest {
    pub components: Vec<Component>,}

#[derive(Serialize)]
pub struct Blocker {
    pub pid: u32,
    pub name: String,
    pub path: String,
    /// a leftover of this install (e.g. an Apache whose pid file was lost), not a stranger
    pub ours: bool,}

#[derive(Serialize)]
pub struct FileRef {
    pub label: String,
    pub rel: String,
    pub exists: bool,}

#[derive(Serialize)]
pub struct State {
    pub id: String,
    pub name: String,
    pub version: String,
    /// missing | installed (library) | stopped | running | blocked
    pub status: &'static str,
    pub pid: Option<u32>,
    pub port: u16,
    pub server: bool,
    pub has_admin: bool,
    pub can_service: bool,
    pub service_installed: bool,
    pub autostart: bool,
    pub blocker: Option<Blocker>,
    pub configs: Vec<FileRef>,
    pub logs: Vec<FileRef>,}

/// What a platform-specific download changes about a component.
#[derive(Deserialize, Clone, Default)]
pub struct Variant {
    pub url: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub sha512: String,
    #[serde(default)]
    pub sha1: String,
    pub strip_prefix: Option<String>,
    pub version: Option<String>,
}

/// "linux-x86_64", "linux-aarch64", "macos-aarch64", "windows-x86_64"
pub fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// The built-in manifest narrowed to this machine.
pub fn manifest_for(text: &str, platform: &str) -> Manifest {
    let mut m: Manifest = serde_json::from_str(text).expect("the built-in manifest must parse");
    m.components.retain(|c| c.variants.is_empty() || c.variants.contains_key(platform));
    for c in &mut m.components {
        if let Some(v) = c.variants.get(platform).cloned() {
            (c.url, c.sha256, c.sha512, c.sha1) = (v.url, v.sha256, v.sha512, v.sha1);
            c.strip_prefix = v.strip_prefix.unwrap_or_else(|| c.strip_prefix.clone());
            c.version = v.version.unwrap_or_else(|| c.version.clone());
        }
    }
    m
}

pub fn manifest() -> Manifest {
    manifest_for(MANIFEST, &platform())
}

pub fn component(id: &str) -> Result<Component, String> {
    component_with(id, &settings())
}

/// Manifest entry with the user's port applied. Precedence: environment (tests, scripts) > settings > manifest.
pub fn component_with(id: &str, s: &Settings) -> Result<Component, String> {
    let mut c = manifest().components.into_iter().find(|c| c.id == id).ok_or_else(|| format!("unknown component {id}"))?;
    if let Some(p) = s.ports.get(id) {
        c.port = *p;
    }
    if let Some(p) = std::env::var(format!("RUNQORA_PORT_{}", id.to_uppercase())).ok().and_then(|v| v.parse().ok()) {
        c.port = p;
    }
    Ok(c)
}

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// The file that starts Runqora at sign-in: a freedesktop autostart entry on Linux, a LaunchAgent on macOS.
#[cfg(unix)]
fn autostart_file() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(if cfg!(target_os = "macos") { home.join("Library/LaunchAgents/app.runqora.plist") } else { std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("autostart/runqora.desktop") })
}

/// Is Runqora set to open (hidden, in the tray) when this user signs in?
#[cfg(windows)]
pub fn launch_at_login() -> bool {
    win::reg_get_string(RUN_KEY, "Runqora").is_some_and(|v| v.to_lowercase().contains("runqora"))
}
#[cfg(unix)]
pub fn launch_at_login() -> bool {
    autostart_file().is_some_and(|f| f.exists())
}

/// The text of the sign-in entry for the program at `exe`.
#[cfg(unix)]
pub fn autostart_entry(exe: &Path) -> String {
    if cfg!(target_os = "macos") {
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict><key>Label</key><string>app.runqora</string><key>ProgramArguments</key><array><string>{}</string><string>--hidden</string></array><key>RunAtLoad</key><true/></dict></plist>\n", exe.display())
    } else {
        format!("[Desktop Entry]\nType=Application\nName=Runqora\nExec=\"{}\" --hidden\nX-GNOME-Autostart-enabled=true\n", exe.display())
    }
}

/// Turn "start when I sign in" on or off. Only this user's Run key is touched, and only when the user flips the setting.
pub fn set_launch_at_login(on: bool) -> Result<String, String> {
    #[cfg(windows)]
    {
        if !on {
            win::reg_delete(RUN_KEY, "Runqora");
            return Ok("Runqora will not open when you sign in".into());
        }
        let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("runqora.exe"))).filter(|p| p.exists()).ok_or("the Runqora program was not found next to this one; change this setting from the window")?;
        win::reg_set_string(RUN_KEY, "Runqora", &format!("\"{}\" --hidden", exe.display())).map_err(|e| format!("Windows refused (error {e})"))?;
        Ok("Runqora will open in the tray when you sign in".into())
    }
    #[cfg(unix)]
    {
        let f = autostart_file().ok_or("this account has no home folder")?;
        if !on {
            let _ = fs::remove_file(&f);
            return Ok("Runqora will not open when you sign in".into());
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        fs::create_dir_all(f.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
        fs::write(&f, autostart_entry(&exe)).map_err(|e| e.to_string())?;
        Ok("Runqora will open in the tray when you sign in".into())
    }
}

pub fn root() -> PathBuf {
    // not under OneDrive: synced folders break pid files and locked binaries
    if let Some(d) = std::env::var_os("RUNQORA_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(unix)]
    return PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into())).join("Runqora");
    #[cfg(windows)]
    let (new, old) = (PathBuf::from(r"C:\Runqora"), PathBuf::from(r"C:\runtime-easy"));
    #[cfg(windows)]
    // an install made before the rename keeps working in place
    if !new.exists() && (old.join("apache").exists() || old.join("settings.json").exists()) { old } else { new }
}

/// Installs made under the old name include `runtime-easy.conf`; point them at the renamed file so Apache still starts.
pub fn migrate_legacy_conf(root: &Path) {
    let (conf, old, new) = (root.join("apache/conf/httpd.conf"), root.join("apache").join("conf/extra/runtime-easy.conf"), root.join("apache").join(EXTRA_CONF));
    if let Ok(t) = fs::read_to_string(&conf) {
        if t.contains("runtime-easy.conf") {
            // swap the name, then keep a single include (the new one may already be there)
            let include = format!("Include {EXTRA_CONF}");
            let mut seen = false;
            let fixed: Vec<String> = t.replace("conf/extra/runtime-easy.conf", EXTRA_CONF).lines().filter(|l| l.trim() != include || !std::mem::replace(&mut seen, true)).map(str::to_string).collect();
            let _ = fs::write(&conf, fixed.join("\n") + "\n");
        }
    }
    if old.exists() {
        let _ = fs::remove_file(&old);
    }
    let _ = new; // regenerated by refresh_extra
}

/// Forward slashes: what Apache, PHP and MariaDB configs want, and valid on Windows.
pub fn fwd(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

// ---- manifest validation (trust boundary: manifest and zip are untrusted input) ----

pub fn validate(c: &Component) -> Result<(), String> {
    let hex = |h: &str, n: usize| h.len() == n && h.chars().all(|ch| ch.is_ascii_hexdigit());
    let hashes = [(&c.sha256, 64, "sha256"), (&c.sha512, 128, "sha512"), (&c.sha1, 40, "sha1")];
    if hashes.iter().all(|(h, ..)| h.is_empty()) || hashes.iter().any(|(h, n, _)| !h.is_empty() && !hex(h, *n)) {
        return Err(format!("{}: a sha256 (64), sha512 (128) or sha1 (40) hex hash is required", c.id));
    }
    let rel = |p: &str| !p.is_empty() && !Path::new(p).is_absolute() && !p.split(['/', '\\']).any(|s| s == "..");
    if !rel(&c.extract_to) || !rel(&c.binary) || (c.args.is_some() && !rel(&c.pidfile)) || (!c.exe.is_empty() && !rel(&c.exe)) {
        return Err(format!("{}: paths must be relative with no ..", c.id));
    }
    if c.args.is_some() && (c.port == 0 || (c.shutdown_event.is_empty() && !c.write_pid)) {
        return Err(format!("{}: a server needs a port and a shutdown_event (or write_pid)", c.id));
    }
    if !c.configs.iter().chain(&c.logs).all(|p| rel(p)) {
        return Err(format!("{}: config and log paths must be relative with no ..", c.id));
    }
    if let Some(s) = &c.service {
        if c.args.is_none() || s.name.is_empty() || !s.name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
            return Err(format!("{}: service needs a server and a plain name", c.id));
        }
    }
    Ok(())
}

// ---- config generation (all idempotent) ----

/// Point a stock Apache conf at our install dir and port, and include our generated extras.
pub fn rewrite_conf(conf: &str, apache_dir: &Path, port: u16) -> String {
    let srv = fwd(apache_dir);
    let include = format!("Include {EXTRA_CONF}");
    // Some builds hard-code the stock root (ServerRoot "C:/Apache24-64") instead of Define SRVROOT
    let old_root = conf.lines().find_map(|l| l.strip_prefix("ServerRoot \"")?.strip_suffix('"')).filter(|r| !r.contains("${"));
    let mut lines: Vec<String> = conf
        .lines()
        .map(|l| {
            let l = &old_root.map_or(l.to_string(), |o| l.replace(o, &srv));
            if l.starts_with("Define SRVROOT") {
                format!("Define SRVROOT \"{srv}\"")
            } else if l.starts_with("Listen ") {
                format!("Listen {port}")
            } else if l.starts_with("ServerName ") {
                format!("ServerName localhost:{port}")
            } else {
                l.to_string()
            }
        })
        .collect();
    // exactly one include of ours: drop the one from before the rename, and any repeat
    lines.retain(|l| l.trim() != "Include conf/extra/runtime-easy.conf");
    let mut seen = false;
    lines.retain(|l| l.trim() != include || !std::mem::replace(&mut seen, true));
    if !seen {
        lines.push(include);
    }
    if !lines.iter().any(|l| l.starts_with("ServerName ")) {
        lines.push(format!("ServerName localhost:{port}")); // silences the startup warning
    }
    lines.join("\n") + "\n"
}

/// What Apache gets from the other components. Regenerated after every install, so
/// install order does not matter. Apache must be restarted to pick it up.
pub fn extra_conf(root: &Path) -> String {
    extra_conf_with(root, &settings_in(root))
}

/// The folder Apache serves: the user's choice in Settings, else `<install>/htdocs`.
pub fn document_root(root: &Path, s: &Settings) -> PathBuf {
    s.custom.get("apache.document_root").filter(|p| !p.is_empty()).map(PathBuf::from).unwrap_or_else(|| root.join("htdocs"))
}

/// Libraries in the PHP folder that its extensions need. OpenSSL's are left out on purpose: Apache brings its own.
pub fn php_support_dlls(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(root.join("php"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| {
            let l = n.to_lowercase();
            l.ends_with(".dll") && (l.starts_with("icu") || ["libssh2.dll", "nghttp2.dll", "nghttp3.dll", "libsodium.dll", "libpq.dll"].contains(&l.as_str()))
        })
        .collect();
    v.sort();
    v
}

pub fn extra_conf_with(root: &Path, settings: &Settings) -> String {
    let (r, s) = (fwd(root), settings);
    let doc = fwd(&document_root(root, s));
    let off = |k: &str| s.custom.get(k).is_some_and(|v| v == "off");
    let https = s.custom.get("apache.https").is_some_and(|v| v == "on").then(|| s.custom.get("apache.https_port").and_then(|p| p.parse::<u16>().ok()).unwrap_or(443));
    let (overrides, indexes) = (if off("apache.allow_override") { "None" } else { "All" }, if off("apache.directory_listing") { "" } else { "Indexes " });
    let mut s = String::from("# generated by Runqora from Settings; edits here are overwritten\n");
    s += &format!("DocumentRoot \"{doc}\"\n<Directory \"{doc}\">\n    Options {indexes}FollowSymLinks\n    AllowOverride {overrides}\n    Require all granted\n</Directory>\n");
    let sites: Vec<(String, String)> = settings.sites.iter().map(|(n, f)| (format!("{n}.localhost"), fwd(Path::new(f)))).collect();
    for (_, folder) in &sites {
        s += &format!("<Directory \"{folder}\">\n    Options {indexes}FollowSymLinks\n    AllowOverride {overrides}\n    Require all granted\n</Directory>\n");
    }
    if !sites.is_empty() {
        // name-based hosts: once one exists the first becomes the default for every other name, so localhost gets its own
        let port = component_with("apache", &settings).map(|c| c.port).unwrap_or(80);
        for (host, folder) in std::iter::once(&("localhost".to_string(), doc.clone())).chain(&sites) {
            s += &format!("<VirtualHost *:{port}>\n    ServerName {host}\n    DocumentRoot \"{folder}\"\n</VirtualHost>\n");
        }
    }
    if let Some(port) = https {
        s += &https::apache_block(root, port, &doc, &sites);
    }
    if root.join("php/php8apache2_4.dll").exists() {
        // PHP's own libraries (ICU for intl, libssh2 and nghttp2 for curl, sodium...) sit next to php.exe, which Apache does
        // not search; without these lines those extensions fail to load inside Apache although `php -m` shows them
        for dll in php_support_dlls(root) {
            s += &format!("LoadFile \"{r}/php/{dll}\"\n");
        }
        s += &format!("LoadModule php_module \"{r}/php/php8apache2_4.dll\"\nPHPIniDir \"{r}/php\"\nAddHandler application/x-httpd-php .php\nDirectoryIndex index.php index.html\n");
    }
    if root.join("phpmyadmin/index.php").exists() {
        // local only: the database root account has no password by default
        s += &format!("Alias /phpmyadmin \"{r}/phpmyadmin\"\n<Directory \"{r}/phpmyadmin\">\n    Require local\n</Directory>\n");
    }
    if root.join("adminer/adminer.php").exists() {
        s += &format!("Alias /adminer \"{r}/adminer\"\n<Directory \"{r}/adminer\">\n    Require local\n</Directory>\n");
    }
    s
}

pub fn refresh_extra(root: &Path) -> Result<(), String> {
    let apache = root.join("apache");
    if !apache.join("conf").is_dir() {
        return Ok(()); // Apache not installed yet; its install calls this again
    }
    let htdocs = document_root(root, &settings_in(root));
    if !htdocs.exists() {
        fs::create_dir_all(&htdocs).map_err(|e| e.to_string())?;
        fs::write(htdocs.join("index.html"), "<!doctype html><title>Runqora</title><h1>It works.</h1><p>Put your site in this folder.</p>\n").map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(apache.join("conf/extra")).map_err(|e| e.to_string())?;
    fs::write(apache.join(EXTRA_CONF), extra_conf(root)).map_err(|e| e.to_string())
}

/// Enable what phpMyAdmin and typical apps need. Only touches commented-out lines.
pub fn configure_php_ini(ini: &str, php_dir: &Path) -> String {
    const EXT: &[&str] = &["bz2", "curl", "exif", "fileinfo", "gd", "gettext", "intl", "mbstring", "mysqli", "openssl", "pdo_mysql", "pdo_sqlite", "sodium", "zip"];
    let dir = fwd(php_dir);
    // (name, commented-out?) of an `extension=name` line, tolerating a trailing "; comment"
    fn ext(l: &str) -> Option<(&str, bool)> {
        let commented = l.starts_with(';');
        let name = l.trim_start_matches(';').trim().strip_prefix("extension=")?.split([' ', '\t', ';']).next()?;
        EXT.contains(&name).then_some((name, commented))
    }
    let lines: Vec<&str> = ini.lines().collect();
    // the template mentions some extensions in prose before the real list: enable each name once, at its last mention
    let mut last: HashMap<&str, usize> = HashMap::new();
    for (i, l) in lines.iter().enumerate() {
        if let Some((n, _)) = ext(l) {
            last.insert(n, i);
        }
    }
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| match ext(l) {
            Some((n, true)) if last[n] == i => format!("extension={n}"),
            _ if l.starts_with(';') && l.trim_start_matches(';').trim() == "extension_dir = \"ext\"" => format!("extension_dir = \"{dir}/ext\""),
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

pub fn random_hex(bytes: usize) -> Result<String, String> {
    #[allow(unused_mut)]
    let mut buf = vec![0u8; bytes];
    #[cfg(unix)]
    {
        fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).map_err(|e| format!("system RNG failed: {e}"))?;
        Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
    }
    #[cfg(windows)]
    random_hex_windows(buf)
}

#[cfg(windows)]
fn random_hex_windows(mut buf: Vec<u8>) -> Result<String, String> {
    use windows_sys::Win32::Security::Cryptography::BCryptGenRandom;
    // SAFETY: buf is valid for buf.len() bytes; null algorithm + flag 2 = system RNG
    let status = unsafe { BCryptGenRandom(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32, 2) };
    if status != 0 {
        return Err(format!("system RNG failed: {status:#x}"));
    }
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn post_install(c: &Component, root: &Path) -> Result<(), String> {
    let dir = root.join(&c.extract_to);
    let write_new = |p: PathBuf, text: String| -> Result<(), String> {
        if p.exists() { Ok(()) } else { fs::write(p, text).map_err(|e| e.to_string()) } // never clobber user edits
    };
    match c.id.as_str() {
        "apache" => {
            fs::create_dir_all(dir.join("logs")).map_err(|e| e.to_string())?; // httpd will not start without it
            let conf = dir.join("conf/httpd.conf");
            let text = fs::read_to_string(&conf).map_err(|e| format!("httpd.conf missing after unpack: {e}"))?;
            fs::write(&conf, rewrite_conf(&text, &dir, c.port)).map_err(|e| e.to_string())?;
        }
        "php" => {
            let dev = fs::read_to_string(dir.join("php.ini-development")).map_err(|e| e.to_string())?;
            write_new(dir.join("php.ini"), configure_php_ini(&dev, &dir))?;
        }
        "phpmyadmin" => {
            let tmp = root.join("tmp/phpmyadmin");
            fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
            let cfg = format!(
                "<?php\n$cfg['blowfish_secret'] = '{}';\n$i = 1;\n$cfg['Servers'][$i]['auth_type'] = 'cookie';\n$cfg['Servers'][$i]['host'] = '127.0.0.1';\n$cfg['Servers'][$i]['port'] = '{}';\n$cfg['Servers'][$i]['AllowNoPassword'] = true;\n$cfg['TempDir'] = '{}';\n",
                random_hex(16)?,
                component("mariadb").map(|m| m.port).unwrap_or(3306),
                fwd(&tmp)
            );
            write_new(dir.join("config.inc.php"), cfg)?;
        }
        "mariadb" => {
            let r = fwd(&dir);
            let ini = format!(
                "[client]\nport={p}\n\n[mysqld]\nport={p}\nbind-address=127.0.0.1\nbasedir=\"{r}\"\ndatadir=\"{r}/data\"\npid-file=\"{r}/data/mysql.pid\"\nlog-error=\"{r}/data/mysql_error.log\"\ncharacter-set-server=utf8mb4\n",
                p = c.port
            );
            write_new(dir.join("my.ini"), ini)?;
            if !dir.join("data/mysql").is_dir() {
                // the zip ships no initialised data directory
                let out = Command::new(dir.join("bin").join(exe("mariadb-install-db")))
                    .arg(format!("--datadir={r}/data"))
                    .stdin(Stdio::null())
                    .creation_flags(NO_WINDOW)
                    .output()
                    .map_err(|e| e.to_string())?;
                if !out.status.success() {
                    return Err(format!("mariadb-install-db failed: {}", String::from_utf8_lossy(&out.stderr)));
                }
            }
        }
        "composer" => {
            // `composer` on the command line: runs the phar with Runqora's PHP
            #[cfg(windows)]
            {
                let bat = format!("@echo off\r\n\"{}\\php\\php.exe\" \"%~dp0composer.phar\" %*\r\n", root.display());
                fs::write(dir.join("composer.bat"), bat).map_err(|e| e.to_string())?;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let sh = format!("#!/bin/sh\nexec \"{}/php/php\" \"$(dirname \"$0\")/composer.phar\" \"$@\"\n", fwd(root));
                let f = dir.join("composer");
                fs::write(&f, sh).map_err(|e| e.to_string())?;
                fs::set_permissions(&f, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
            }
        }
        "tomcat" => {
            let xml = dir.join("conf/server.xml");
            let text = fs::read_to_string(&xml).map_err(|e| format!("server.xml missing after unpack: {e}"))?;
            fs::write(&xml, tomcat_server_xml(&text, c.port)).map_err(|e| e.to_string())?;
        }
        _ => {}
    }
    if matches!(c.id.as_str(), "php" | "mailpit") {
        wire_mail(root)?;
    }
    refresh_extra(root)
}

/// Point Tomcat's web connector at our port and keep it on this PC only. Idempotent.
pub fn tomcat_server_xml(xml: &str, port: u16) -> String {
    let mut done = false;
    xml.lines()
        .map(|l| {
            let at = l.find("<Connector port=\"").filter(|_| !done && l.contains("HTTP/1.1"));
            let Some(at) = at else { return l.to_string() };
            done = true; // the first HTTP/1.1 connector is the plain one; the TLS and AJP examples are comments further down
            let start = at + "<Connector port=\"".len();
            let end = start + l[start..].find('"').unwrap_or(0);
            let rest = &l[end + 1..];
            let rest = if rest.trim_start().starts_with("address=") { rest.to_string() } else { format!(" address=\"127.0.0.1\"{rest}") };
            format!("{}{port}\"{rest}", &l[..start])
        })
        .collect::<Vec<_>>()
        .join("
")
        + "
"
}

/// Rewrite the SMTP lines of a php.ini. Idempotent; commented-out lines are left alone.
pub fn set_mail_lines(ini: &str, port: u16) -> String {
    ini.lines()
        .map(|l| match l.split('=').next().map(str::trim) {
            Some("SMTP") if l.contains('=') => "SMTP = localhost".to_string(),
            Some("smtp_port") if l.contains('=') => format!("smtp_port = {port}"),
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("
")
        + "
"
}

/// PHP's mail() delivers to Mailpit (SMTP on 1025) once both are installed, whichever came first.
pub fn wire_mail(root: &Path) -> Result<(), String> {
    let ini = root.join("php/php.ini");
    if !root.join("mailpit").join(exe("mailpit")).exists() {
        return Ok(());
    }
    match fs::read_to_string(&ini) {
        Ok(t) => fs::write(&ini, set_mail_lines(&t, 1025)).map_err(|e| e.to_string()),
        Err(_) => Ok(()), // PHP not installed yet: its install calls this again
    }
}

// ---- install ----

pub fn unpack(c: &Component, zip_path: &Path, root: &Path) -> Result<(), String> {
    unpack_component(c, zip_path, &root.join(&c.extract_to), &c.strip_prefix)
}

/// Put a download into `dest`: an archive is extracted, a single file is copied in under the name of `binary`.
pub fn unpack_component(c: &Component, file: &Path, dest: &Path, strip_prefix: &str) -> Result<(), String> {
    if !c.single_file {
        return unpack_to_with(file, dest, strip_prefix, &|done, total| progress_set(&c.id, "Unpacking", done, total));
    }
    let name = Path::new(&c.binary).file_name().ok_or("bad binary path")?;
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    fs::copy(file, dest.join(name)).map(|_| ()).map_err(|e| e.to_string())
}

/// Runqora's own record of which version it put in a component folder. Programs that do not report a version
/// reliably (and every component after an update) are read from here.
pub const MARKER: &str = ".runqora-version";

pub fn write_marker(dir: &Path, version: &str) {
    let _ = fs::write(dir.join(MARKER), version);
}

pub fn read_marker(dir: &Path) -> Option<String> {
    fs::read_to_string(dir.join(MARKER)).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// "sha256:<hex>", "sha512:<hex>", "sha1:<hex>" or a bare SHA-256 hex.
pub fn check_hash(spec: &str, file: &Path) -> Result<(), String> {
    let (algo, want) = spec.split_once(':').unwrap_or(("sha256", spec));
    let got = match algo {
        "sha256" => sha256_file(file).map_err(|e| e.to_string())?,
        "sha512" => sha512_file(file).map_err(|e| e.to_string())?,
        "sha1" => sha1_file(file)?,
        other => return Err(format!("unknown checksum type {other}")),
    };
    if got.eq_ignore_ascii_case(want) { Ok(()) } else { Err(format!("the download does not match the publisher's checksum (expected {want}, got {got}); nothing was changed")) }
}

// ---- progress: what a download or update is doing right now, for the window to show ----

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Progress {
    /// "Downloading", "Unpacking", ...
    pub stage: String,
    /// bytes or files done, and the total; total 0 = unknown (the bar shows activity instead of a percentage)
    pub done: u64,
    pub total: u64,
}

static PROGRESS: Mutex<BTreeMap<String, Progress>> = Mutex::new(BTreeMap::new());

pub fn progress_set(id: &str, stage: &str, done: u64, total: u64) {
    PROGRESS.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), Progress { stage: stage.into(), done, total });
}

pub fn progress_all() -> BTreeMap<String, Progress> {
    PROGRESS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Marks `id` as busy for as long as it lives; the entry disappears when the work ends, however it ends.
pub struct Busy(pub String);

impl Busy {
    pub fn new(id: &str, stage: &str) -> Busy {
        progress_set(id, stage, 0, 0);
        Busy(id.to_string())
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        PROGRESS.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

/// Download `url` into `to`, reporting bytes as they arrive. The file appears under its real name only when complete.
pub fn download_to(id: &str, url: &str, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to.parent().ok_or("bad path")?).map_err(|e| e.to_string())?;
    let part = to.with_extension("part");
    let resp = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).user_agent("runqora/0.1").build().get(url).call().map_err(|e| format!("{url}: {e}"))?;
    let total = resp.header("Content-Length").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let mut r = resp.into_reader();
    progress_set(id, "Downloading", 0, total);
    // whatever goes wrong, a half-written file must not stay behind
    let copied = (|| -> Result<u64, String> {
        let mut f = fs::File::create(&part).map_err(|e| e.to_string())?;
        let (mut buf, mut done, mut last) = (vec![0u8; 64 * 1024], 0u64, std::time::Instant::now());
        loop {
            let n = r.read(&mut buf).map_err(|e| format!("the download was interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            io::Write::write_all(&mut f, &buf[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            if last.elapsed() >= Duration::from_millis(100) {
                progress_set(id, "Downloading", done, total); // about ten updates a second is plenty
                last = std::time::Instant::now();
            }
        }
        Ok(done)
    })();
    let done = match copied {
        Ok(d) if total == 0 || d == total => d,
        Ok(d) => {
            let _ = fs::remove_file(&part);
            return Err(format!("the download stopped early ({d} of {total} bytes)"));
        }
        Err(e) => {
            let _ = fs::remove_file(&part);
            return Err(e);
        }
    };
    progress_set(id, "Downloading", done, total.max(done));
    fs::rename(&part, to).map_err(|e| e.to_string())
}

/// Extract `zip_path` into `dest`, dropping the archive's top folder `strip_prefix`. Refuses unsafe paths.
pub fn unpack_to(zip_path: &Path, dest: &Path, strip_prefix: &str) -> Result<(), String> {
    unpack_to_with(zip_path, dest, strip_prefix, &|_, _| {})
}

/// Like `unpack_to`, calling `on(files_done, files_total)` as it goes.
pub fn unpack_to_with(zip_path: &Path, dest: &Path, strip_prefix: &str, on: &dyn Fn(u64, u64)) -> Result<(), String> {
    let mut magic = [0u8; 6];
    let _ = fs::File::open(zip_path).and_then(|mut f| f.read(&mut magic));
    if magic[..2] == [0x1f, 0x8b] {
        return unpack_tar(zip_path, dest, strip_prefix, on, false);
    }
    if magic == [0xfd, b'7', b'z', b'X', b'Z', 0] {
        return unpack_tar(zip_path, dest, strip_prefix, on, true);
    }
    let mut z = zip::ZipArchive::new(fs::File::open(zip_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let auto;
    let strip_prefix = if strip_prefix == "*" {
        // the one folder everything sits in, if there is exactly one; otherwise nothing to strip
        let tops: std::collections::BTreeSet<String> = z.file_names().map(|n| n.split('/').next().unwrap_or("").to_string()).collect();
        let nested = z.file_names().all(|n| n.contains('/'));
        auto = if tops.len() == 1 && nested { tops.into_iter().next().unwrap_or_default() } else { String::new() };
        auto.as_str()
    } else {
        strip_prefix
    };
    let count = z.len() as u64;
    let mut last = std::time::Instant::now();
    for i in 0..z.len() {
        if last.elapsed() >= Duration::from_millis(100) {
            on(i as u64, count);
            last = std::time::Instant::now();
        }
        let mut f = z.by_index(i).map_err(|e| e.to_string())?;
        // enclosed_name is None for absolute paths and ..: refuse the whole archive (zip-slip)
        let name = f.enclosed_name().ok_or_else(|| format!("unsafe path in archive: {}", f.name()))?;
        let Ok(rel) = name.strip_prefix(strip_prefix.trim_end_matches('/')) else { continue };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(rel);
        if f.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        } else {
            fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
            io::copy(&mut f, &mut fs::File::create(&out).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            if let Some(mode) = f.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o777));
            }
        }
    }
    Ok(())
}

/// A .tar.gz or .tar.xz (what Linux and macOS programs come in), with the same rules as the zip path: one top folder can
/// be dropped, nothing may land outside `dest`, and program files keep their run permission.
fn unpack_tar(path: &Path, dest: &Path, strip_prefix: &str, on: &dyn Fn(u64, u64), xz: bool) -> Result<(), String> {
    let es = |e: io::Error| e.to_string();
    let plain = path.with_extension("tar.part");
    if xz {
        // the xz decoder works stream to stream; unpack it to a plain tar next to the download first
        let mut r = io::BufReader::new(fs::File::open(path).map_err(es)?);
        let mut w = io::BufWriter::new(fs::File::create(&plain).map_err(es)?);
        lzma_rs::xz_decompress(&mut r, &mut w).map_err(|e| format!("could not read the xz archive: {e}"))?;
        io::Write::flush(&mut w).map_err(es)?;
    }
    let src = if xz { plain.as_path() } else { path };
    let open = || -> Result<tar::Archive<Box<dyn Read>>, String> {
        let f = fs::File::open(src).map_err(es)?;
        let r: Box<dyn Read> = if xz { Box::new(f) } else { Box::new(flate2::read::GzDecoder::new(f)) };
        let mut a = tar::Archive::new(r);
        a.set_preserve_permissions(true);
        Ok(a)
    };
    // `./x` and `x` are the same; absolute paths and `..` are refused
    let clean = |p: &Path| -> Result<PathBuf, String> {
        let mut out = PathBuf::new();
        for c in p.components() {
            match c {
                std::path::Component::Normal(n) => out.push(n),
                std::path::Component::CurDir => {}
                _ => return Err(format!("unsafe path in archive: {}", p.display())),
            }
        }
        Ok(out)
    };
    let result = (|| {
        let mut names: Vec<PathBuf> = vec![];
        for e in open()?.entries().map_err(es)? {
            names.push(clean(&e.map_err(es)?.path().map_err(es)?)?);
        }
        let strip = if strip_prefix == "*" {
            let tops: std::collections::BTreeSet<_> = names.iter().filter_map(|n| n.components().next()).collect();
            let nested = names.iter().filter(|n| !n.as_os_str().is_empty()).all(|n| n.components().count() > 1 || tops.len() == 1);
            if tops.len() == 1 && nested { tops.into_iter().next().map(|c| PathBuf::from(c.as_os_str())).unwrap_or_default() } else { PathBuf::new() }
        } else {
            PathBuf::from(strip_prefix.trim_end_matches('/'))
        };
        let count = names.len() as u64;
        let mut last = std::time::Instant::now();
        for (i, e) in open()?.entries().map_err(es)?.enumerate() {
            if last.elapsed() >= Duration::from_millis(100) {
                on(i as u64, count);
                last = std::time::Instant::now();
            }
            let mut e = e.map_err(es)?;
            let Ok(rel) = names[i].strip_prefix(&strip) else { continue };
            if rel.as_os_str().is_empty() {
                continue;
            }
            let out = dest.join(rel);
            match e.header().entry_type() {
                tar::EntryType::Directory => fs::create_dir_all(&out).map_err(es)?,
                tar::EntryType::Regular | tar::EntryType::Continuous | tar::EntryType::Symlink => {
                    fs::create_dir_all(out.parent().unwrap_or(dest)).map_err(es)?;
                    let _ = fs::remove_file(&out);
                    e.unpack(&out).map_err(es)?;
                }
                _ => {} // hard links, devices: nothing Runqora's programs ship
            }
        }
        Ok(())
    })();
    let _ = fs::remove_file(&plain);
    result
}

pub fn sha256_file(p: &Path) -> io::Result<String> {
    let mut h = Sha256::new();
    let mut f = fs::File::open(p)?;
    let mut buf = [0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

pub fn sha512_file(p: &Path) -> io::Result<String> {
    let mut h = sha2::Sha512::new();
    io::copy(&mut fs::File::open(p)?, &mut h)?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// SHA-1: the only publisher that needs it (WordPress) offers nothing stronger.
pub fn sha1_file(p: &Path) -> Result<String, String> {
    use sha1::Digest as _;
    let mut h = sha1::Sha1::new();
    io::copy(&mut fs::File::open(p).map_err(|e| e.to_string())?, &mut h).map_err(|e| e.to_string())?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The download must match the publisher's hash (the strongest one the manifest has).
pub fn verify_hash(c: &Component, zip_path: &Path) -> Result<(), String> {
    let (want, got) = if !c.sha256.is_empty() {
        (&c.sha256, sha256_file(zip_path).map_err(|e| e.to_string())?)
    } else if !c.sha512.is_empty() {
        (&c.sha512, sha512_file(zip_path).map_err(|e| e.to_string())?)
    } else {
        (&c.sha1, sha1_file(zip_path)?)
    };
    if got.eq_ignore_ascii_case(want) { Ok(()) } else { Err(format!("hash mismatch for {}: expected {want}, got {got}", c.id)) }
}

pub fn install_from_zip(c: &Component, zip_path: &Path, root: &Path) -> Result<(), String> {
    validate(c)?;
    if root.join(&c.binary).exists() {
        // overlaying a new unpack on a live install would clobber data (MariaDB system tables)
        return Err(format!("{} is already installed", c.name));
    }
    progress_set(&c.id, "Checking the download", 0, 0);
    verify_hash(c, zip_path)?;
    progress_set(&c.id, "Unpacking", 0, 0);
    unpack(c, zip_path, root)?;
    progress_set(&c.id, "Setting up", 0, 0);
    write_marker(&root.join(&c.extract_to), &c.version);
    if !root.join(&c.binary).exists() {
        // the publisher changed the archive layout: do not leave a half install that says "missing" forever
        let _ = fs::remove_dir_all(root.join(&c.extract_to));
        return Err(format!("{} is not where expected in the download ({}); nothing was installed", c.binary, c.name));
    }
    post_install(c, root)
}

pub fn install(c: &Component, root: &Path) -> Result<(), String> {
    validate(c)?;
    let _busy = Busy::new(&c.id, "Starting");
    for n in &c.needs {
        let need = component(n)?;
        if !root.join(&need.binary).exists() {
            install(&need, root)?; // Tomcat brings the Java it runs on
        }
    }
    // Publishers move old files (PHP puts a superseded release under /archives/) and re-issue builds, so a pinned link can
    // stop working. Try the pinned link, then the same file's archive address (same checksum), then whatever the publisher
    // offers today for the same release line. The first that downloads and verifies wins.
    let mut failures: Vec<String> = vec![];
    let mut sources = vec![c.clone()];
    if let Some(url) = archive_url(&c.url) {
        sources.push(Component { url, ..c.clone() });
    }
    let mut live_tried = false;
    loop {
        for src in std::mem::take(&mut sources) {
            match install_one(&src, root) {
                Ok(()) => return Ok(()),
                Err(e) if e.contains("is already installed") => return Err(e),
                Err(e) => failures.push(e),
            }
        }
        if live_tried {
            break;
        }
        live_tried = true;
        match update::installable_now(c) {
            Ok(live) => sources.push(live),
            Err(e) => failures.push(format!("looking for a current version: {e}")),
        }
        if sources.is_empty() {
            break;
        }
    }
    Err(format!("{} could not be downloaded: {}", c.name, failures.join("; ")))
}

/// Download one source and install it. Everything downloaded is removed afterwards.
fn install_one(c: &Component, root: &Path) -> Result<(), String> {
    let tmp = root.join("tmp");
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let part = tmp.join(format!("{}.zip.part", c.id));
    let zip = tmp.join(format!("{}.zip", c.id));
    let result = download_to(&c.id, &c.url, &zip).and_then(|_| install_from_zip(c, &zip, root));
    let _ = fs::remove_file(&part);
    let _ = fs::remove_file(&zip); // bad or used, never reuse a download
    result
}

/// PHP for Windows moves a release to /archives/ once a newer one is out; the file and its checksum stay the same.
pub fn archive_url(url: &str) -> Option<String> {
    const BASES: [&str; 2] = ["https://windows.php.net/downloads/releases/", "https://downloads.php.net/~windows/releases/"];
    let base = BASES.iter().find(|b| url.starts_with(**b))?;
    let file = &url[base.len()..];
    (!file.contains('/')).then(|| format!("{base}archives/{file}"))
}

// ---- Win32 ----

pub mod config;
pub mod fixes;
pub mod appupdate;
pub mod externals;
pub mod https;
pub mod sites;
pub mod update;

#[cfg(unix)]
#[path = "unixos.rs"]
pub mod win;

#[cfg(windows)]
pub mod win {
    use std::{mem, ptr};
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER};
    use windows_sys::Win32::Networking::WinSock::AF_INET;
    use windows_sys::Win32::System::Services::{CloseServiceHandle, OpenSCManagerW, OpenServiceW, SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    /// PID listening on this IPv4 TCP port, if any.
    /// ponytail: IPv4 only; a pure-IPv6 listener is not seen (add AF_INET6 table if it matters)
    pub fn listener_pid(port: u16) -> Option<u32> {
        listeners().into_iter().find(|(p, _)| *p == port).map(|(_, pid)| pid)
    }

    /// Every IPv4 TCP listener as (port, pid).
    pub fn listeners() -> Vec<(u16, u32)> {
        let mut size = 0u32;
        // SAFETY: first call only asks for the required size
        unsafe { GetExtendedTcpTable(ptr::null_mut(), &mut size, 0, AF_INET as u32, TCP_TABLE_OWNER_PID_LISTENER, 0) };
        let mut buf = vec![0u32; size as usize / 4 + 64]; // u32 buffer: the rows need 4-byte alignment; slack for table growth
        let mut size = (buf.len() * 4) as u32;
        // SAFETY: buf is valid for `size` bytes
        let rc = unsafe { GetExtendedTcpTable(buf.as_mut_ptr().cast(), &mut size, 0, AF_INET as u32, TCP_TABLE_OWNER_PID_LISTENER, 0) };
        if rc != 0 {
            return Vec::new();
        }
        let n = buf[0] as usize;
        // SAFETY: the API filled `n` rows right after the u32 count, inside buf
        let rows = unsafe { std::slice::from_raw_parts(buf.as_ptr().add(1).cast::<MIB_TCPROW_OWNER_PID>(), n) };
        rows.iter().map(|r| (u16::from_be(r.dwLocalPort as u16), r.dwOwningPid)).collect()
    }

    /// A text value under HKEY_CURRENT_USER.
    pub fn reg_get_string(subkey: &str, name: &str) -> Option<String> {
        use windows_sys::Win32::System::Registry::HKEY_CURRENT_USER;
        reg_get_text(HKEY_CURRENT_USER, subkey, name)
    }

    fn reg_get_text(hive: windows_sys::Win32::System::Registry::HKEY, subkey: &str, name: &str) -> Option<String> {
        use windows_sys::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ};
        let (k, n) = (wide(subkey), wide(name));
        let mut buf = vec![0u16; 2048];
        let mut size = (buf.len() * 2) as u32;
        // SAFETY: buf is valid for `size` bytes; the key and value names are NUL-terminated
        let rc = unsafe { RegGetValueW(hive, k.as_ptr(), n.as_ptr(), RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ, ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) };
        (rc == 0).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
    }

    /// The command line a Windows service runs (its ImagePath). Anyone may read it.
    pub fn service_image_path(name: &str) -> Option<String> {
        use windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        reg_get_text(HKEY_LOCAL_MACHINE, &format!(r"SYSTEM\CurrentControlSet\Services\{name}"), "ImagePath")
    }

    pub fn reg_set_string(subkey: &str, name: &str, value: &str) -> Result<(), u32> {
        use windows_sys::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
        let (k, n, v) = (wide(subkey), wide(name), wide(value));
        // SAFETY: v is a NUL-terminated UTF-16 string and the byte count includes the terminator
        let rc = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr(), REG_SZ, v.as_ptr().cast(), (v.len() * 2) as u32) };
        if rc == 0 { Ok(()) } else { Err(rc) }
    }

    pub fn reg_delete(subkey: &str, name: &str) {
        use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, HKEY_CURRENT_USER};
        let (k, n) = (wide(subkey), wide(name));
        // SAFETY: both names are NUL-terminated; a missing value is simply an error code we ignore
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr()) };
    }

    /// A DWORD value under HKEY_CURRENT_USER.
    pub fn reg_dword(subkey: &str, name: &str) -> Option<u32> {
        use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
        let (k, n) = (wide(subkey), wide(name));
        let (mut v, mut size) = (0u32, 4u32);
        // SAFETY: v and size outlive the call; the key and value names are NUL-terminated
        let rc = unsafe { RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr(), RRF_RT_REG_DWORD, ptr::null_mut(), (&mut v as *mut u32).cast(), &mut size) };
        (rc == 0).then_some(v)
    }

    /// End a process. Err carries the Windows error code (5 = access denied).
    pub fn terminate(pid: u32) -> Result<(), u32> {
        use windows_sys::Win32::System::Threading::TerminateProcess;
        // SAFETY: the handle is closed on every path
        unsafe {
            let h = OpenProcess(0x0001, 0, pid); // PROCESS_TERMINATE
            if h.is_null() {
                return Err(GetLastError());
            }
            let ok = TerminateProcess(h, 1);
            let e = GetLastError();
            CloseHandle(h);
            if ok != 0 { Ok(()) } else { Err(e) }
        }
    }

    /// Mark our own console/pipe handles non-inheritable so a server we start does not keep them open:
    /// otherwise `rq start | anything` (or a CI step) never ends, because the server holds the pipe.
    pub fn keep_std_handles_private() {
        use windows_sys::Win32::Foundation::{SetHandleInformation, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
        for id in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: only flips the inherit flag of handles the process already owns; invalid ones are skipped
            unsafe {
                let h = GetStdHandle(id);
                if !h.is_null() && h != INVALID_HANDLE_VALUE {
                    SetHandleInformation(h, 1, 0); // HANDLE_FLAG_INHERIT off
                }
            }
        }
    }

    /// Executable name of any process, no special rights needed (toolhelp snapshot).
    pub fn process_name(pid: u32) -> Option<String> {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
        // SAFETY: zeroed PROCESSENTRY32W with dwSize set is the documented way to start; the snapshot handle is closed
        unsafe {
            let h = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if h == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut e: PROCESSENTRY32W = mem::zeroed();
            e.dwSize = mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(h, &mut e);
            let mut found = None;
            while more != 0 {
                if e.th32ProcessID == pid {
                    let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                    found = Some(String::from_utf16_lossy(&e.szExeFile[..n]));
                    break;
                }
                more = Process32NextW(h, &mut e);
            }
            CloseHandle(h);
            found
        }
    }

    pub fn pid_alive(pid: u32) -> bool {
        // SAFETY: handle is closed on every path
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                // a service runs as SYSTEM: access denied (5) means it exists, invalid parameter (87) means it does not
                return GetLastError() == 5;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
            ok != 0 && code == 259 // STILL_ACTIVE
        }
    }

    /// Full image path; None for protected processes we may not open.
    pub fn image_path(pid: u32) -> Option<String> {
        // SAFETY: buf outlives the call; len is its capacity in u16s
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
            CloseHandle(h);
            (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
        }
    }

    pub fn service_exists(name: &str) -> bool {
        let n = wide(name);
        // SAFETY: both handles are closed
        unsafe {
            let scm = OpenSCManagerW(ptr::null(), ptr::null(), SC_MANAGER_CONNECT);
            if scm.is_null() {
                return false;
            }
            let svc = OpenServiceW(scm, n.as_ptr(), SERVICE_QUERY_STATUS);
            let found = !svc.is_null();
            if found {
                CloseServiceHandle(svc);
            }
            CloseServiceHandle(scm);
            found
        }
    }

    /// Run `file params` with a UAC prompt, wait up to 60 s, return its exit code.
    pub fn run_elevated(file: &str, params: &str) -> Result<u32, String> {
        let (verb, f, p) = (wide("runas"), wide(file), wide(params));
        // SAFETY: zeroed SHELLEXECUTEINFOW is a valid "all defaults" value; strings outlive the call
        unsafe {
            let mut info: SHELLEXECUTEINFOW = mem::zeroed();
            info.cbSize = mem::size_of::<SHELLEXECUTEINFOW>() as u32;
            info.fMask = 0x40; // SEE_MASK_NOCLOSEPROCESS
            info.lpVerb = verb.as_ptr();
            info.lpFile = f.as_ptr();
            info.lpParameters = p.as_ptr();
            info.nShow = 0; // SW_HIDE
            if ShellExecuteExW(&mut info) == 0 {
                return Err(match GetLastError() {
                    1223 => "administrator permission was declined".to_string(),
                    e => format!("could not start {file} as administrator (error {e})"),
                });
            }
            WaitForSingleObject(info.hProcess, 60_000);
            let mut code = 0u32;
            GetExitCodeProcess(info.hProcess, &mut code);
            CloseHandle(info.hProcess);
            Ok(code)
        }
    }
}

// ---- process control ----

/// Is the Windows service of `c` registered AND does it run a program inside THIS install folder? Service names are
/// machine-wide, so a second Runqora folder (or a test copy) must not start, stop or remove the first one's service.
pub fn service_is_ours(c: &Component, root: &Path) -> bool {
    let Some(sv) = &c.service else { return false };
    if !win::service_exists(&sv.name) {
        return false;
    }
    match win::service_image_path(&sv.name) {
        Some(image) => path_inside(&image, root),
        None => true, // cannot tell: behave as before
    }
}

/// Does a service command line such as `"C:\Runqora\apache\bin\httpd.exe" -k runservice` point into `root`?
pub fn path_inside(command: &str, root: &Path) -> bool {
    let norm = |t: &str| t.replace('/', "\\").replace('"', "").to_lowercase();
    let mut r = norm(&root.to_string_lossy());
    if !r.ends_with('\\') {
        r.push('\\');
    }
    norm(command).contains(&r)
}

#[cfg(test)] // only the smoke tests ask "is it free"; the app reads the listener table directly
pub fn port_in_use(port: u16) -> bool {
    win::listener_pid(port).is_some()
}

pub fn read_pid(c: &Component, root: &Path) -> Option<u32> {
    fs::read_to_string(root.join(&c.pidfile)).ok()?.trim().parse().ok()
}

pub fn is_ours(path: &str, root: &Path) -> bool {
    !path.is_empty() && path.replace('\\', "/").to_lowercase().starts_with(&fwd(root).to_lowercase())
}

pub fn describe_blocker(pid: u32, root: &Path) -> Blocker {
    let path = win::image_path(pid).unwrap_or_default();
    // the path needs rights we often lack (services run as SYSTEM); the process snapshot still gives the name
    let name = path
        .rsplit(['\\', '/'])
        .next()
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .or_else(|| win::process_name(pid))
        .unwrap_or_else(|| format!("a protected process (PID {pid})"));
    Blocker { pid, name, ours: is_ours(&path, root), path }
}

pub fn file_refs(root: &Path, rels: &[String]) -> Vec<FileRef> {
    rels.iter()
        .map(|r| FileRef { label: r.rsplit('/').next().unwrap_or(r).to_string(), rel: r.clone(), exists: root.join(r).exists() })
        .collect()
}

pub fn state(c: &Component, root: &Path) -> State {
    let server = c.args.is_some();
    // a pid file we wrote ourselves may outlive the process and the number be reused: it must still be that program
    let want = Path::new(if c.exe.is_empty() { &c.binary } else { &c.exe }).file_name().map(|n| n.to_string_lossy().to_lowercase());
    let same = |p: u32| !c.write_pid || win::process_name(p).map(|n| n.to_lowercase()) == want;
    let pid = if server { read_pid(c, root).filter(|p| win::pid_alive(*p) && same(*p)) } else { None };
    let listener = if server && pid.is_none() { win::listener_pid(c.port) } else { None };
    let status = if !root.join(&c.binary).exists() {
        "missing"
    } else if !server {
        "installed"
    } else if pid.is_some() {
        "running"
    } else if listener.is_some() {
        "blocked"
    } else {
        "stopped"
    };
    State {
        id: c.id.clone(),
        name: c.name.clone(),
        version: c.version.clone(),
        status,
        pid,
        port: c.port,
        server,
        has_admin: c.admin_url.is_some(),
        can_service: c.service.is_some() && status != "missing",
        service_installed: service_is_ours(c, root),
        autostart: false, // all_states fills it from settings

        blocker: listener.map(|p| describe_blocker(p, root)),
        configs: file_refs(root, &c.configs),
        logs: file_refs(root, &c.logs),
    }
}

pub fn blocker_message(c: &Component, b: &Blocker) -> String {
    let who = if b.path.is_empty() { b.name.clone() } else { format!("{} ({})", b.name, b.path) };
    if b.ours {
        format!("port {} is held by a leftover {} from this install (PID {}); end that process, then start again", c.port, b.name, b.pid)
    } else {
        let hint = if b.pid == 4 { ", which means Windows itself, often IIS or another web service" } else { "" };
        format!("port {} is used by {} (PID {}{hint}); {} was not started. Close that program or change the port", c.port, who, b.pid, c.name)
    }
}

pub fn service_params(spec: &ServiceSpec, args: &[String], root: &Path) -> String {
    let r = fwd(root);
    args.iter()
        .map(|a| a.replace("{root}", &r).replace("{service}", &spec.name))
        .map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn start(c: &Component, root: &Path) -> Result<String, String> {
    let Some(args) = &c.args else { return Err(format!("{} has nothing to start", c.name)) };
    let s = state(c, root);
    match s.status {
        "missing" => return Err(format!("{} is not installed", c.name)),
        "running" => return Ok(format!("{} is already running (PID {})", c.name, s.pid.unwrap_or(0))),
        "blocked" => return Err(s.blocker.map(|b| blocker_message(c, &b)).unwrap_or_else(|| format!("port {} is in use", c.port))),
        _ => {}
    }
    let _ = fs::remove_file(root.join(&c.pidfile)); // stale pid from a crash
    if c.id == "apache" {
        migrate_legacy_conf(root);
    }
    let mut note = String::new();
    match c.service.as_ref().filter(|sv| s.service_installed && !sv.name.is_empty()) {
        Some(sv) => {
            let rc = win::run_elevated("net", &format!("start {}", sv.name))?;
            if rc != 0 {
                note = format!(" (net start returned {rc})");
            }
        }
        None => {
            let (r, port) = (fwd(root), c.port.to_string());
            let fill = |a: &str| a.replace("{root}", &r).replace("{port}", &port);
            for n in &c.needs {
                if component(n).is_ok_and(|d| !root.join(&d.binary).exists()) {
                    return Err(format!("{} needs {n}; install it first", c.name));
                }
            }
            win::keep_std_handles_private();
            let exe = if c.exe.is_empty() { &c.binary } else { &c.exe };
            let child = Command::new(root.join(exe))
                .args(args.iter().map(|a| fill(a)))
                .envs(c.env.iter().map(|(k, v)| (k, fill(v))))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null()) // do not inherit our pipes: the server outlives us
                .creation_flags(NO_WINDOW)
                .spawn()
                .map_err(|e| e.to_string())?;
            #[allow(unused_mut)]
            let mut child = child;
            if c.write_pid {
                let p = root.join(&c.pidfile);
                let _ = fs::create_dir_all(p.parent().unwrap_or(root));
                fs::write(p, child.id().to_string()).map_err(|e| e.to_string())?;
            }
            // a server that ends while Runqora runs must not linger as a zombie that still looks alive
            #[cfg(unix)]
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
    for _ in 0..100 {
        sleep(Duration::from_millis(100));
        if state(c, root).status == "running" {
            // the pid file appears before the socket opens: only report success once connections are accepted
            let listening = (0..100).any(|_| {
                let up = win::listener_pid(c.port).is_some();
                if !up {
                    sleep(Duration::from_millis(100));
                }
                up
            });
            return if listening { Ok(format!("{} started on port {}", c.name, c.port)) } else { Err(format!("{} is running but not accepting connections on port {} yet; check the logs under {}", c.name, c.port, c.extract_to)) };
        }
    }
    Err(format!("{} did not come up{note}; check the logs under {}", c.name, c.extract_to))
}

#[cfg(unix)]
pub fn signal_shutdown(_event: &str) -> bool {
    false // named events are a Windows thing; modules with no other graceful channel get SIGTERM
}

#[cfg(windows)]
pub fn signal_shutdown(event: &str) -> bool {
    use windows_sys::Win32::System::Threading::{OpenEventW, SetEvent};
    let name: Vec<u16> = format!("{event}\0").encode_utf16().collect();
    // SAFETY: name is NUL-terminated; the handle is used once and leaked on purpose (one call per stop)
    unsafe {
        let h = OpenEventW(0x0002, 0, name.as_ptr()); // EVENT_MODIFY_STATE
        !h.is_null() && SetEvent(h) != 0
    }
}

pub fn stop(c: &Component, root: &Path) -> Result<String, String> {
    let Some(pid) = state(c, root).pid else {
        return Ok(format!("{} is not running", c.name));
    };
    match c.service.as_ref().filter(|_| service_is_ours(c, root)) {
        // a service runs as SYSTEM: ask the service manager, not the process
        Some(sv) => {
            win::run_elevated("net", &format!("stop {}", sv.name))?;
        }
        None if c.shutdown_port != 0 => {
            // Tomcat's own stop: its shutdown port listens for a word
            use std::io::Write;
            if let Ok(mut s) = std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], c.shutdown_port)), Duration::from_secs(2)) {
                let _ = s.write_all(c.shutdown_text.as_bytes());
            }
        }
        None if c.shutdown_event.is_empty() => {
            // no graceful channel (Mailpit, SFTPGo): they keep their data on disk, ending the process is how they are stopped
            let _ = win::terminate(pid);
        }
        None => {
            signal_shutdown(&c.shutdown_event.replace("{pid}", &pid.to_string()));
        }
    }
    for _ in 0..150 {
        sleep(Duration::from_millis(100));
        if !win::pid_alive(pid) {
            return Ok(format!("{} stopped", c.name));
        }
    }
    // graceful signal ignored for 15 s: force the tree
    #[cfg(windows)]
    let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).creation_flags(NO_WINDOW).output();
    #[cfg(unix)]
    win::kill_tree(pid);
    Ok(format!("{} did not stop gracefully; terminated PID {}", c.name, pid))
}

pub fn service_install(c: &Component, root: &Path) -> Result<String, String> {
    let sv = c.service.as_ref().ok_or_else(|| format!("{} cannot run as a service", c.name))?;
    if !root.join(&c.binary).exists() {
        return Err(format!("{} is not installed", c.name));
    }
    if win::service_exists(&sv.name) {
        return if service_is_ours(c, root) {
            Ok(format!("{} is already installed as service {}", c.name, sv.name))
        } else {
            Err(format!("another Runqora install already owns the Windows service {}; remove it there first", sv.name))
        };
    }
    let exe = root.join(&c.binary);
    match win::run_elevated(&exe.to_string_lossy(), &service_params(sv, &sv.install, root))? {
        0 => Ok(format!("{} installed as Windows service {}", c.name, sv.name)),
        rc => Err(format!("service install for {} failed (exit code {rc})", c.name)),
    }
}

pub fn service_remove(c: &Component, root: &Path) -> Result<String, String> {
    let sv = c.service.as_ref().ok_or_else(|| format!("{} cannot run as a service", c.name))?;
    if !service_is_ours(c, root) {
        return Ok(format!("service {} is not installed for this folder", sv.name));
    }
    if state(c, root).status == "running" {
        return Err(format!("stop {} first, then remove the service", c.name));
    }
    let exe = root.join(&c.binary);
    let rc = win::run_elevated(&exe.to_string_lossy(), &service_params(sv, &sv.remove, root))?;
    for _ in 0..50 {
        if !win::service_exists(&sv.name) {
            return Ok(format!("Windows service {} removed", sv.name));
        }
        sleep(Duration::from_millis(100));
    }
    // exit code 0 is not proof: a delete can stay pending while the service is still running
    Err(format!("service {} is still registered after removal (exit code {rc}); stop it, then try again", sv.name))
}

/// Open a manifest-listed file in the editor. The path never comes from the UI, only an index into the manifest list.
pub fn open_file(c: &Component, root: &Path, rel: &str) -> Result<(), String> {
    if !c.configs.iter().chain(&c.logs).any(|r| r == rel) {
        return Err("that file is not on the module's list".into());
    }
    let path = root.join(rel);
    if !path.exists() {
        return Err(format!("{rel} does not exist yet"));
    }
    let editor = std::env::var("RUNQORA_EDITOR").ok().or_else(|| Some(settings().editor).filter(|e| !e.is_empty())).unwrap_or_else(|| default_editor().into());
    Command::new(editor).arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

fn default_editor() -> &'static str {
    if cfg!(windows) { "notepad.exe" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" }
}

pub fn admin_url(c: &Component) -> Option<String> {
    let apache_port = component("apache").map(|a| a.port).unwrap_or(80);
    let url = c.admin_url.as_ref()?.replace("{apache_port}", &apache_port.to_string()).replace("{port}", &c.port.to_string());
    // drop the :80 so the address bar stays clean
    Some(url.replace("localhost:80/", "localhost/"))
}

/// What has to be running before this module's page can be opened, in start order. Optional ones are skipped when
/// they are not installed (Adminer works without MariaDB).
pub fn needs_running(id: &str) -> &'static [&'static str] {
    match id {
        "phpmyadmin" | "mariadb" => &["mariadb", "apache"], // its page is phpMyAdmin, served by Apache
        "adminer" => &["mariadb", "apache"],
        "apache" => &["apache"],
        _ => &[],
    }
}

/// Open a module's page, starting what it needs first: Open on phpMyAdmin starts MariaDB and Apache if they are stopped.
/// Reports progress under `id` so its card says what is happening ("Starting Apache...").
pub fn launch(id: &str, open: &dyn Fn(&str) -> Result<(), String>) -> Result<String, String> {
    let root = root();
    let c = component(id)?;
    let url = admin_url(&c).ok_or_else(|| format!("{} has no page to open", c.name))?;
    if !root.join(&c.binary).exists() {
        return Err(format!("{} is not installed", c.name));
    }
    if matches!(id, "mariadb") && !root.join("phpmyadmin/index.php").exists() {
        return Err("MariaDB is managed in the browser with phpMyAdmin: install phpMyAdmin first".into());
    }
    let _busy = Busy::new(id, "Starting");
    let mut started = vec![];
    let mut wanted: Vec<&str> = needs_running(id).to_vec();
    if c.args.is_some() && !wanted.contains(&id) {
        wanted.push(id);
    }
    for dep in wanted {
        let d = component(dep)?;
        if !root.join(&d.binary).exists() {
            if dep == "apache" {
                return Err(format!("{} is shown through Apache, which is not installed", c.name));
            }
            continue; // an optional helper that is not there
        }
        if state(&d, &root).status != "running" {
            progress_set(id, &format!("Starting {}", d.name), 0, 0);
            start(&d, &root)?;
            started.push(d.name);
        }
    }
    progress_set(id, "Opening the page", 0, 0);
    open(&url)?;
    Ok(if started.is_empty() { format!("Opened {}", c.name) } else { format!("Started {} and opened {}", started.join(" and "), c.name) })
}

/// The folder a module lives in (it must exist).
pub fn module_folder(id: &str) -> Result<PathBuf, String> {
    let c = component(id)?;
    let dir = root().join(&c.extract_to);
    if dir.is_dir() { Ok(dir) } else { Err(format!("{} is not installed yet", c.name)) }
}

// ---- settings (settings.json in the install folder) ----

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Settings {
    /// program that opens config and log files; empty = Notepad
    #[serde(default)]
    pub editor: String,
    /// only ports that differ from the manifest default
    #[serde(default)]
    pub ports: BTreeMap<String, u16>,
    /// start this server when Runqora opens
    #[serde(default)]
    pub autostart: BTreeMap<String, bool>,
    /// "off" | "notify" | "install"; empty means install (patch updates only)
    #[serde(default)]
    pub auto_update: String,
    /// unix seconds of the last automatic check
    #[serde(default)]
    pub last_check: u64,
    /// the update summary the user was last told about, so we do not repeat it
    #[serde(default)]
    pub last_notified: String,
    /// settings that are not read back from a program's own config file (the Apache extras are generated from these)
    #[serde(default)]
    pub custom: BTreeMap<String, String>,
    /// your sites: name -> folder; each one is served as http://name.localhost/
    #[serde(default)]
    pub sites: BTreeMap<String, String>,
}

fn settings_file(root: &Path) -> PathBuf {
    root.join("settings.json")
}

pub fn settings_in(root: &Path) -> Settings {
    fs::read_to_string(settings_file(root)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn settings() -> Settings {
    settings_in(&root())
}

fn save_settings_in(root: &Path, s: &Settings) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let p = settings_file(root);
    if fs::read_to_string(&p).is_ok_and(|old| serde_json::from_str::<Settings>(&old).is_err()) {
        let _ = fs::copy(&p, p.with_extension("json.bad")); // never silently destroy a file the user may have hand-edited
    }
    let tmp = p.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(s).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

pub fn set_editor_in(root: &Path, editor: &str) -> Result<String, String> {
    let e = editor.trim();
    if e.is_empty() || e.contains(['"', '\n', '\r']) {
        return Err("enter the name or path of a program, without quotes".into());
    }
    let mut s = settings_in(root);
    s.editor = if e.eq_ignore_ascii_case("notepad.exe") { String::new() } else { e.to_string() };
    save_settings_in(root, &s)?;
    Ok(format!("Files will open in {e}"))
}

pub fn set_auto_update_in(root: &Path, mode: &str) -> Result<String, String> {
    let text = match mode {
        "off" => "Runqora will not look for updates by itself",
        "notify" => "Runqora will tell you when updates are available",
        "install" => "Runqora will install patch updates when the servers are stopped",
        _ => return Err("choose off, notify or install".into()),
    };
    let mut s = settings_in(root);
    s.auto_update = mode.to_string();
    save_settings_in(root, &s)?;
    Ok(text.to_string())
}

pub fn set_autostart_in(root: &Path, id: &str, on: bool) -> Result<String, String> {
    let c = component(id)?;
    if c.args.is_none() {
        return Err(format!("{} is not a server", c.name));
    }
    let mut s = settings_in(root);
    s.autostart.insert(id.to_string(), on);
    save_settings_in(root, &s)?;
    Ok(format!("{} {} when Runqora opens", c.name, if on { "will start" } else { "will not start" }))
}

/// Rewrite `port=` lines of a my.ini. Idempotent.
pub fn replace_port_lines(ini: &str, port: u16) -> String {
    ini.lines().map(|l| if l.trim_start().starts_with("port=") { format!("port={port}") } else { l.to_string() }).collect::<Vec<_>>().join("\n") + "\n"
}

/// Rewrite the phpMyAdmin server port line. Idempotent.
pub fn replace_pma_port(php: &str, port: u16) -> String {
    php.lines().map(|l| if l.contains("['port']") { format!("$cfg['Servers'][$i]['port'] = '{port}';") } else { l.to_string() }).collect::<Vec<_>>().join("\n") + "\n"
}

fn apply_port_files(c: &Component, root: &Path) -> Result<(), String> {
    let edit = |rel: &str, f: &dyn Fn(&str) -> String| -> Result<(), String> {
        let p = root.join(rel);
        match fs::read_to_string(&p) {
            Ok(t) => fs::write(&p, f(&t)).map_err(|e| e.to_string()),
            Err(_) => Ok(()), // not installed yet: the install writes the right port itself
        }
    };
    match c.id.as_str() {
        "apache" => edit("apache/conf/httpd.conf", &|t| rewrite_conf(t, &root.join("apache"), c.port)),
        "tomcat" => edit("tomcat/conf/server.xml", &|t| tomcat_server_xml(t, c.port)),
        "mariadb" => {
            edit("mariadb/my.ini", &|t| replace_port_lines(t, c.port))?;
            edit("phpmyadmin/config.inc.php", &|t| replace_pma_port(t, c.port))
        }
        _ => Ok(()),
    }
}

pub fn set_port_in(root: &Path, id: &str, port: u32) -> Result<String, String> {
    let s = settings_in(root);
    let c = component_with(id, &s)?;
    if c.args.is_none() {
        return Err(format!("{} has no port", c.name));
    }
    let port = u16::try_from(port).ok().filter(|p| *p != 0).ok_or("a port is a number from 1 to 65535")?;
    if port == c.port {
        return Ok(format!("{} already uses port {port}", c.name));
    }
    if root.join(&c.binary).exists() && state(&c, root).status == "running" {
        return Err(format!("stop {} first, then change its port", c.name));
    }
    for o in manifest().components.iter().filter(|o| o.args.is_some() && o.id != id) {
        let other = component_with(&o.id, &s)?;
        if other.port == port {
            return Err(format!("{} already uses port {port}", other.name));
        }
    }
    if let Some(pid) = win::listener_pid(port) {
        return Err(format!("port {port} is already used by {} (PID {pid})", describe_blocker(pid, root).name));
    }
    let default = manifest().components.iter().find(|m| m.id == id).map(|m| m.port).unwrap_or(0);
    let mut s = s;
    if port == default { s.ports.remove(id) } else { s.ports.insert(id.to_string(), port) };
    save_settings_in(root, &s)?;
    apply_port_files(&component_with(id, &s)?, root)?;
    Ok(format!("{} will use port {port}", c.name))
}

// ---- notes: messages from background work (tray, autostart) for the window to show ----

#[derive(Serialize, Clone)]
pub struct Note {
    pub level: &'static str,
    pub text: String,
}

static NOTES: Mutex<Vec<Note>> = Mutex::new(Vec::new());

pub fn push_note(level: &'static str, text: String) {
    NOTES.lock().unwrap_or_else(|e| e.into_inner()).push(Note { level, text });
}

pub fn take_notes() -> Vec<Note> {
    std::mem::take(&mut *NOTES.lock().unwrap_or_else(|e| e.into_inner()))
}

// ---- whole-stack helpers shared by the window, the tray and the command line ----

pub fn servers() -> Vec<Component> {
    let s = settings();
    manifest().components.iter().filter(|c| c.args.is_some()).filter_map(|c| component_with(&c.id, &s).ok()).collect()
}

/// Servers in the order they should start: the database before the web server that talks to it. Stopping goes the other way.
pub fn servers_in_start_order() -> Vec<Component> {
    let mut v = servers();
    v.sort_by_key(|c| c.id != "mariadb"); // stable: everything else keeps the manifest order
    v
}

/// Windows stores colours as 0xAABBGGRR; "#RRGGBB" is what the UI wants.
pub fn abgr_to_hex(v: u32) -> String {
    format!("#{:02X}{:02X}{:02X}", v & 0xFF, (v >> 8) & 0xFF, (v >> 16) & 0xFF)
}

/// The accent colour from the system's personalisation settings, if it can be read (only Windows has one to read).
#[cfg(unix)]
pub fn system_accent() -> Option<String> {
    None
}

#[cfg(windows)]
pub fn system_accent() -> Option<String> {
    win::reg_dword(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent", "AccentColorMenu")
        .or_else(|| win::reg_dword(r"Software\Microsoft\Windows\DWM", "AccentColor"))
        .map(abgr_to_hex)
        // ColorizationColor is the older ARGB spelling of the same setting
        .or_else(|| win::reg_dword(r"Software\Microsoft\Windows\DWM", "ColorizationColor").map(|v| format!("#{:06X}", v & 0xFF_FFFF)))
}

#[derive(Serialize)]
pub struct Appearance {
    /// system | light | dark
    pub theme: String,
    /// system | custom
    pub accent_mode: String,
    pub accent_color: String,
    pub system_accent: Option<String>,
}

pub fn appearance_in(root: &Path) -> Appearance {
    let s = settings_in(root);
    let get = |k: &str, d: &str| s.custom.get(&format!("general.{k}")).cloned().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string());
    Appearance { theme: get("theme", "system"), accent_mode: get("accent_mode", "system"), accent_color: get("accent_color", "#0067C0"), system_accent: system_accent() }
}

/// Is this a page on THIS PC (localhost, 127.0.0.1, *.localhost) or on a name the user put in Settings? The window may only
/// open those: it is the user's own sites, never an address a web page could talk it into.
pub fn is_local_url(url: &str, root: &Path) -> bool {
    let Some(rest) = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")) else { return false };
    let host_port = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host_port.contains('@') {
        return false; // user@host tricks
    }
    let host = if let Some(v6) = host_port.strip_prefix('[') { v6.split(']').next().unwrap_or("") } else { host_port.split(':').next().unwrap_or("") }.to_lowercase();
    host == "localhost" || host == "127.0.0.1" || host == "::1" || host.ends_with(".localhost") || https::configured_names(root).iter().any(|n| *n == host || n.strip_prefix("*.").is_some_and(|d| host.ends_with(&format!(".{d}"))))
}

pub fn all_states() -> Vec<State> {
    let (r, s) = (root(), settings());
    manifest()
        .components
        .iter()
        .filter_map(|c| component_with(&c.id, &s).ok())
        .map(|c| State { autostart: s.autostart.get(&c.id).copied().unwrap_or(false), ..state(&c, &r) })
        .collect()
}

fn report(prefix: &str, name: &str, r: Result<String, String>) {
    match r {
        Ok(m) => push_note("success", format!("{prefix}{m}")),
        Err(e) => push_note("error", format!("{prefix}{name}: {e}")),
    }
}

/// Start or stop every server in the background; results arrive as notes. Stop runs in reverse order.
pub fn all_servers(start_them: bool) {
    std::thread::spawn(move || {
        let r = root();
        // only what is installed: "Start all" must not complain about modules the user never added
        let mut cs: Vec<Component> = servers_in_start_order().into_iter().filter(|c| r.join(&c.binary).exists()).collect();
        if !start_them {
            cs.reverse(); // web server before its database
        }
        for c in cs {
            report("", &c.name, if start_them { start(&c, &r) } else { stop(&c, &r) });
        }
    });
}

/// On app launch: start the servers the user marked.
pub fn run_autostart() {
    std::thread::spawn(|| {
        let (r, s) = (root(), settings());
        for c in servers_in_start_order().into_iter().filter(|c| s.autostart.get(&c.id).copied().unwrap_or(false)) {
            report("Autostart: ", &c.name, start(&c, &r));
        }
    });
}

#[derive(Serialize)]
pub struct PortRow {
    pub port: u16,
    pub pid: u32,
    pub name: String,
    pub path: String,
    pub ours: bool,
    /// which of our servers is configured for this port
    pub module: Option<String>,
}

/// Every IPv4 TCP listener with its program, like `netstat -ano` but with names.
pub fn ports_scan() -> Vec<PortRow> {
    let (r, mods) = (root(), servers());
    let mut seen: HashMap<u32, Blocker> = HashMap::new();
    let mut ls = win::listeners();
    ls.sort_unstable();
    ls.dedup();
    ls.into_iter()
        .map(|(port, pid)| {
            let b = seen.entry(pid).or_insert_with(|| describe_blocker(pid, &r));
            PortRow { port, pid, name: b.name.clone(), path: b.path.clone(), ours: b.ours, module: mods.iter().find(|c| c.port == port).map(|c| c.name.clone()) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn comp(id: &str) -> Component {
        manifest().components.into_iter().find(|c| c.id == id).unwrap()
    }

    fn make_zip(path: &Path, entries: &[(&str, &str)]) {
        let mut w = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (n, body) in entries {
            w.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn new_modules_are_described_and_keep_their_rules() {
        let (tomcat, jre) = (comp("tomcat"), comp("jre"));
        assert_eq!(tomcat.needs, ["jre"]);
        assert!(jre.args.is_none() && tomcat.write_pid && tomcat.shutdown_port == 8005);
        let ports: Vec<u16> = manifest().components.iter().filter(|c| c.args.is_some()).map(|c| c.port).collect();
        let mut uniq = ports.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(ports.len(), uniq.len(), "no two servers share a default port: {ports:?}");
        let mut no_hash = comp("mailpit");
        no_hash.sha256.clear();
        assert!(validate(&no_hash).is_err(), "a download without a hash is refused");
        no_hash.sha1 = "13eb".into();
        assert!(validate(&no_hash).is_err(), "a hash of the wrong length is refused");
        let mut no_stop = comp("mailpit");
        no_stop.write_pid = false;
        assert!(validate(&no_stop).is_err(), "a server needs some way to be found again and stopped");
    }

    #[test]
    fn tomcat_connector_follows_the_port_and_stays_local() {
        let xml = "<Server port=\"8005\" shutdown=\"SHUTDOWN\">
    <Connector port=\"8080\" protocol=\"HTTP/1.1\"
               connectionTimeout=\"20000\"
               redirectPort=\"8443\" />
    <!--
    <Connector port=\"8443\" protocol=\"org.apache.coyote.http11.Http11NioProtocol\"
    -->
";
        let once = tomcat_server_xml(xml, 9090);
        assert!(once.contains("<Connector port=\"9090\" address=\"127.0.0.1\" protocol=\"HTTP/1.1\""), "{once}");
        assert!(once.contains("<Server port=\"8005\"") && once.contains("<Connector port=\"8443\""), "only the plain connector changes");
        assert_eq!(once, tomcat_server_xml(&once, 9090), "idempotent");
        assert!(tomcat_server_xml(&once, 8081).contains("port=\"8081\" address=\"127.0.0.1\" protocol"));
    }

    #[test]
    fn php_mail_goes_to_mailpit() {
        let ini = "[mail function]
; For Win32 only.
SMTP = localhost
smtp_port = 25
;sendmail_from = me@example.com
";
        let once = set_mail_lines(ini, 1025);
        assert!(once.contains("SMTP = localhost
smtp_port = 1025
;sendmail_from"));
        assert_eq!(once, set_mail_lines(&once, 1025));
        let d = std::env::temp_dir().join("rt-mail");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("php")).unwrap();
        fs::write(d.join("php/php.ini"), ini).unwrap();
        wire_mail(&d).unwrap();
        assert!(fs::read_to_string(d.join("php/php.ini")).unwrap().contains("smtp_port = 25"), "no Mailpit, nothing changes");
        fs::create_dir_all(d.join("mailpit")).unwrap();
        fs::write(d.join("mailpit/mailpit.exe"), "").unwrap();
        wire_mail(&d).unwrap();
        assert!(fs::read_to_string(d.join("php/php.ini")).unwrap().contains("smtp_port = 1025"));
    }

    #[test]
    fn archives_with_one_top_folder_are_flattened_and_checked_by_any_hash() {
        let d = std::env::temp_dir().join("rt-star");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let z = d.join("a.zip");
        make_zip(&z, &[("jdk-21.0.99+1-jre/bin/java.exe", "j"), ("jdk-21.0.99+1-jre/release", "r")]);
        let mut c = comp("jre");
        c.sha256.clear();
        c.sha512 = sha512_file(&z).unwrap();
        install_from_zip(&c, &z, &d).unwrap();
        assert!(d.join("jre/bin/java.exe").exists(), "the unknown top folder is dropped");
        // a zip that already has files at the top is left as it is
        let flat = d.join("flat");
        make_zip(&z, &[("mailpit.exe", "m"), ("README.md", "r")]);
        unpack_to(&z, &flat, "*").unwrap();
        assert!(flat.join("mailpit.exe").exists());
        // sha1 (WordPress) goes through certutil
        make_zip(&z, &[("wordpress/wp-load.php", "w")]);
        let mut wp = comp("wordpress");
        wp.sha1 = sha1_file(&z).unwrap();
        let root = d.join("wproot");
        install_from_zip(&wp, &z, &root).unwrap();
        assert!(root.join("wordpress/wp-load.php").exists());
        let mut bad = comp("wordpress");
        bad.sha1 = "0".repeat(40);
        assert!(verify_hash(&bad, &z).unwrap_err().contains("hash mismatch"));
        // layout changed upstream: refused, nothing half-installed
        make_zip(&z, &[("other/readme.txt", "x")]);
        let mut m = comp("mailpit");
        m.sha256 = sha256_file(&z).unwrap();
        let err = install_from_zip(&m, &z, &d.join("r2")).unwrap_err();
        assert!(err.contains("not where expected") && !d.join("r2/mailpit").exists(), "{err}");
    }

    #[test]
    fn a_server_we_started_gets_its_environment_a_pid_file_and_is_stopped_by_pid() {
        let r = std::env::temp_dir().join("rt-spawn");
        let _ = fs::remove_dir_all(&r);
        fs::create_dir_all(&r).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let ps = Path::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        let mut c = comp("mailpit");
        c.port = port;
        c.binary = ps.to_string_lossy().into_owned();
        c.args = Some(vec!["-NoProfile".into(), "-Command".into(), "$l=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,[int]$env:RQ_PORT); $l.Start(); Start-Sleep 60".into()]);
        c.env = BTreeMap::from([("RQ_PORT".to_string(), "{port}".to_string())]);
        c.pidfile = "x/test.pid".into();
        let msg = start(&c, &r).unwrap();
        assert!(msg.contains("started"), "{msg}");
        let s = state(&c, &r);
        assert_eq!(s.status, "running");
        assert_eq!(fs::read_to_string(r.join("x/test.pid")).unwrap().trim(), s.pid.unwrap().to_string());
        assert!(stop(&c, &r).unwrap().contains("stopped"));
        assert_eq!(state(&c, &r).status, "stopped");
        // a pid file that now names some other program is not mistaken for ours
        fs::write(r.join("x/test.pid"), std::process::id().to_string()).unwrap();
        assert_eq!(state(&c, &r).pid, None);
    }

    #[test]
    fn tomcat_is_stopped_by_the_word_on_its_shutdown_port() {
        let r = std::env::temp_dir().join("rt-tcpstop");
        let _ = fs::remove_dir_all(&r);
        fs::create_dir_all(&r).unwrap();
        let free = || std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let (port, stop_port) = (free(), free());
        let ps = Path::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        // listens on both ports; exits when it reads SHUTDOWN on the second
        let script = format!("$l=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,{port}); $l.Start(); $s=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,{stop_port}); $s.Start(); $c=$s.AcceptTcpClient(); $t=(New-Object IO.StreamReader($c.GetStream())).ReadToEnd(); if($t -eq 'SHUTDOWN'){{exit 0}}; Start-Sleep 60");
        let mut c = comp("tomcat");
        (c.port, c.shutdown_port, c.needs) = (port, stop_port, vec![]);
        c.binary = ps.to_string_lossy().into_owned();
        c.exe.clear();
        c.args = Some(vec!["-NoProfile".into(), "-Command".into(), script]);
        c.pidfile = "t.pid".into();
        start(&c, &r).unwrap();
        let t = std::time::Instant::now();
        assert!(stop(&c, &r).unwrap().contains("stopped"), "the graceful word was understood");
        assert!(t.elapsed() < Duration::from_secs(10), "no forced kill needed");
    }

    #[test]
    fn a_moved_php_release_is_found_in_the_archives() {
        assert_eq!(archive_url("https://downloads.php.net/~windows/releases/php-8.3.35-Win32-vs16-x64.zip").as_deref(), Some("https://downloads.php.net/~windows/releases/archives/php-8.3.35-Win32-vs16-x64.zip"));
        assert_eq!(archive_url("https://windows.php.net/downloads/releases/php-8.3.35-Win32-vs16-x64.zip").as_deref(), Some("https://windows.php.net/downloads/releases/archives/php-8.3.35-Win32-vs16-x64.zip"));
        assert_eq!(archive_url("https://windows.php.net/downloads/releases/archives/php-8.3.35-Win32-vs16-x64.zip"), None, "already there");
        assert_eq!(archive_url("https://archive.mariadb.org/mariadb-10.11.19/winx64-packages/x.zip"), None, "other publishers keep their old files");
        let php = comp("php");
        assert!(php.url.starts_with("https://windows.php.net/downloads/releases/php-"), "the manifest points at the current release, not a staging path: {}", php.url);
    }

    /// The exact link that used to 404 (a release that has since moved to /archives/) still installs: real download of PHP.
    #[test]
    #[ignore]
    fn a_dead_pinned_php_link_still_installs() {
        let root = std::env::temp_dir().join("rt-dead-link");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut c = comp("php");
        c.version = "8.3.35".into();
        c.url = "https://downloads.php.net/~windows/releases/php-8.3.35-Win32-vs16-x64.zip".into();
        c.sha256 = "b454bed828a74536125d8a96119eae8ffbb186b305759dd6cf8d9bb4b55968b1".into();
        assert!(ureq::get(&c.url).call().is_err(), "this address is expected to be dead; if it works again the test is moot");
        install(&c, &root).unwrap();
        assert!(root.join("php/php.exe").exists() && root.join("php/php.ini").exists());
        assert_eq!(read_marker(&root.join("php")).as_deref(), Some("8.3.35"));
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn registry_text_values_round_trip_in_a_throwaway_key() {
        // never the real Run key: a scratch key under HKCU that is emptied again
        let key = r"Software\RunqoraUnitTest";
        assert_eq!(win::reg_get_string(key, "v"), None);
        win::reg_set_string(key, "v", r#""C:\Program Files\Runqora\runqora.exe" --hidden"#).unwrap();
        assert_eq!(win::reg_get_string(key, "v").as_deref(), Some(r#""C:\Program Files\Runqora\runqora.exe" --hidden"#));
        win::reg_delete(key, "v");
        assert_eq!(win::reg_get_string(key, "v"), None);
        // the sign-in setting is read from the real Run key but is off unless it names Runqora
        let _ = launch_at_login();
    }

    #[cfg(windows)]
    #[test]
    fn a_service_belongs_to_the_install_folder_it_runs_from() {
        let mine = Path::new(r"C:\Runqora");
        assert!(path_inside(r#""C:\Runqora\apache\bin\httpd.exe" -k runservice"#, mine));
        assert!(path_inside(r#""c:/runqora/mariadb/bin/mysqld.exe" --defaults-file="c:/runqora/mariadb/my.ini""#, mine), "slashes and case do not matter");
        assert!(!path_inside(r#""C:\Runqora\apache\bin\httpd.exe" -k runservice"#, Path::new(r"C:\Users\x\AppData\Local\Temp\rt-smoke")), "another install's service is not ours");
        assert!(!path_inside(r#""C:\Runqora2\apache\bin\httpd.exe""#, mine), "a folder that merely starts with the same letters is not inside it");
        // a service that is not registered is never ours; EventLog is registered but is not Runqora's
        let apache = comp("apache");
        assert!(!service_is_ours(&Component { service: Some(ServiceSpec { name: "RunqoraNoSuchService".into(), install: vec![], remove: vec![] }), ..apache.clone() }, mine));
        assert!(!service_is_ours(&Component { service: Some(ServiceSpec { name: "EventLog".into(), install: vec![], remove: vec![] }), ..apache }, mine));
    }

    #[test]
    fn opening_a_page_starts_what_it_needs_and_says_why_it_cannot() {
        assert_eq!(needs_running("phpmyadmin"), ["mariadb", "apache"], "the database first, then the web server");
        assert_eq!(needs_running("apache"), ["apache"]);
        assert!(needs_running("mailpit").is_empty(), "a server that needs nothing else is started by its own Start");
        // pages exist for every server with one, and for phpMyAdmin and Adminer
        for id in ["apache", "mariadb", "phpmyadmin", "adminer", "tomcat", "mailpit", "sftpgo"] {
            assert!(admin_url(&component(id).unwrap()).is_some(), "{id} has a page");
        }
        // nothing is installed in the default folder of this unit test run unless the machine has Runqora: only the messages are checked
        let nowhere = std::env::temp_dir().join("rt-launch-none");
        let _ = fs::remove_dir_all(&nowhere);
        std::env::set_var("RUNQORA_DIR", &nowhere);
        let e = launch("phpmyadmin", &|_| Ok(())).unwrap_err();
        let f = module_folder("apache").unwrap_err();
        std::env::remove_var("RUNQORA_DIR");
        assert!(e.contains("not installed"), "{e}");
        assert!(f.contains("not installed yet"), "{f}");
        assert!(launch("nope", &|_| Ok(())).is_err() && module_folder("nope").is_err());
    }

    #[test]
    fn manifest_is_valid() {
        let m = manifest();
        let ids: Vec<_> = m.components.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["apache", "php", "mariadb", "phpmyadmin", "jre", "tomcat", "mailpit", "sftpgo", "wordpress", "node", "composer", "adminer"]);
        for c in &m.components {
            validate(c).unwrap();
        }
        let mut bad = comp("apache");
        bad.extract_to = "../x".into();
        assert!(validate(&bad).is_err());
        bad = comp("apache");
        bad.sha256 = "zz".into();
        assert!(validate(&bad).is_err());
        bad = comp("mariadb");
        bad.shutdown_event.clear();
        assert!(validate(&bad).is_err());
    }

    #[test]
    fn an_install_from_before_the_rename_keeps_starting() {
        let d = std::env::temp_dir().join("rt-legacy");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("apache/conf/extra")).unwrap();
        // the old include, and the new one a later rewrite had already added next to it
        fs::write(d.join("apache/conf/httpd.conf"), "Listen 80\nInclude conf/extra/runtime-easy.conf\nInclude conf/extra/runqora.conf\n").unwrap();
        fs::write(d.join("apache/conf/extra/runtime-easy.conf"), "old").unwrap();
        migrate_legacy_conf(&d);
        let conf = fs::read_to_string(d.join("apache/conf/httpd.conf")).unwrap();
        assert_eq!(conf.matches(&format!("Include {EXTRA_CONF}")).count(), 1, "exactly one include: {conf}");
        assert!(!conf.contains("runtime-easy"));
        // a rewrite (port change, repair) of a file that still has both also leaves exactly one
        let both = rewrite_conf("Listen 80\nInclude conf/extra/runtime-easy.conf\nInclude conf/extra/runqora.conf\nInclude conf/extra/runqora.conf\n", Path::new("C:/x"), 80);
        assert_eq!(both.matches("Include conf/extra/").count(), 1);
        assert!(!d.join("apache/conf/extra/runtime-easy.conf").exists());
        migrate_legacy_conf(&d); // idempotent
        assert_eq!(conf, fs::read_to_string(d.join("apache/conf/httpd.conf")).unwrap());
    }

    #[test]
    fn rewrite_is_idempotent() {
        let src = "Define SRVROOT \"c:/Apache24\"\nListen 80\nServerName www.example.com:80\nother\n";
        let once = rewrite_conf(src, Path::new(r"C:\rt\apache"), 8081);
        assert!(once.contains("Define SRVROOT \"C:/rt/apache\"") && once.contains("Listen 8081"));
        assert_eq!(once, rewrite_conf(&once, Path::new(r"C:\rt\apache"), 8081));
        assert_eq!(once.matches("runqora.conf").count(), 1);
        // hard-coded stock root (Lounge VS17 style): every occurrence is moved, not just ServerRoot
        let hard = "ServerRoot \"C:/Apache24-64\"\nDocumentRoot \"C:/Apache24-64/htdocs\"\nListen 80\n";
        let moved = rewrite_conf(hard, Path::new(r"C:\rt\apache"), 8081);
        assert!(moved.contains("ServerRoot \"C:/rt/apache\"") && moved.contains("DocumentRoot \"C:/rt/apache/htdocs\"") && !moved.contains("Apache24-64"));
        assert_eq!(moved, rewrite_conf(&moved, Path::new(r"C:\rt\apache"), 8081));
    }

    #[test]
    fn php_ini_enables_only_listed_extensions_idempotently() {
        // shaped like php.ini-development: a prose example of mysqli first, the real list later, a trailing comment on exif
        let dev = "; For example:\n;\n;extension=mysqli\n; extension_dir = \"ext\"\n;extension=ffi\n;extension=mbstring\n;extension=exif      ; Must be after mbstring\n;extension=mysqli\nextension=openssl\n;zend_extension=opcache\n";
        let once = configure_php_ini(dev, Path::new(r"C:\rt\php"));
        let on = |name: &str| once.lines().filter(|l| *l == format!("extension={name}")).count();
        assert_eq!((on("mysqli"), on("mbstring"), on("exif"), on("openssl")), (1, 1, 1, 1), "{once}"); // no duplicates, exif found despite its comment
        assert!(once.contains(";extension=ffi") && once.contains(";zend_extension=opcache") && once.contains("extension_dir = \"C:/rt/php/ext\""));
        assert!(once.lines().any(|l| l == ";extension=mysqli"), "the prose example stays a comment");
        assert_eq!(once, configure_php_ini(&once, Path::new(r"C:\rt\php")));
    }

    #[test]
    fn extra_conf_follows_what_is_installed() {
        let d = std::env::temp_dir().join("rt-extra");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("php")).unwrap();
        assert!(!extra_conf(&d).contains("LoadModule"));
        fs::write(d.join("php/php8apache2_4.dll"), "").unwrap();
        fs::write(d.join("php/icuuc74.dll"), "").unwrap();
        fs::write(d.join("php/libssl-3-x64.dll"), "").unwrap(); // OpenSSL's belong to Apache: never LoadFile'd
        fs::create_dir_all(d.join("phpmyadmin")).unwrap();
        fs::write(d.join("phpmyadmin/index.php"), "").unwrap();
        let e = extra_conf(&d);
        assert!(e.contains("LoadModule php_module") && e.contains("Alias /phpmyadmin") && e.contains("Require local"));
        assert!(e.contains("LoadFile \"") && e.contains("php/icuuc74.dll\"") && !e.contains("libssl"), "{e}");
        assert!(e.find("LoadFile").unwrap() < e.find("LoadModule").unwrap(), "libraries load before the module");
    }

    #[test]
    fn install_rejects_bad_hash_zip_slip_and_reinstall() {
        let d = std::env::temp_dir().join("rt-unit").join("root");
        let _ = fs::remove_dir_all(d.parent().unwrap());
        fs::create_dir_all(&d).unwrap();
        let z = d.join("a.zip");
        make_zip(&z, &[("Apache24/conf/httpd.conf", "Listen 80\n"), ("Apache24/bin/httpd.exe", "x")]);
        let mut c = comp("apache");
        assert!(install_from_zip(&c, &z, &d).unwrap_err().contains("hash mismatch"));
        c.sha256 = sha256_file(&z).unwrap();
        install_from_zip(&c, &z, &d).unwrap();
        assert!(fs::read_to_string(d.join("apache/conf/httpd.conf")).unwrap().contains(&format!("Listen {}", c.port)));
        assert!(install_from_zip(&c, &z, &d).unwrap_err().contains("already installed"));
        make_zip(&z, &[("../evil.txt", "x")]);
        c.sha256 = sha256_file(&z).unwrap();
        c.binary = "apache/never.exe".into();
        assert!(install_from_zip(&c, &z, &d).unwrap_err().contains("unsafe path"));
        assert!(!d.parent().unwrap().join("evil.txt").exists());
    }

    #[test]
    fn listener_pid_finds_a_real_listener_and_names_it() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert_eq!(win::listener_pid(port), Some(std::process::id()));
        let b = describe_blocker(std::process::id(), Path::new(r"C:\somewhere-else"));
        assert!(b.name.starts_with("runqora") && !b.ours && !b.path.is_empty());
        drop(l);
        assert_eq!(win::listener_pid(port), None);
    }

    #[test]
    fn blocked_start_names_the_blocker_and_ours_is_told_apart() {
        let d = std::env::temp_dir().join("rt-blocked");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("apache/bin")).unwrap();
        fs::write(d.join("apache/bin/httpd.exe"), "").unwrap();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut c = comp("apache");
        c.port = l.local_addr().unwrap().port();
        let err = start(&c, &d).unwrap_err();
        assert!(err.contains(&format!("port {} is used by runqora", c.port)) && err.contains(&std::process::id().to_string()), "{err}");
        assert!(is_ours(r"C:\RT\apache\bin\httpd.exe", Path::new(r"c:\rt")) && !is_ours("", Path::new(r"c:\rt")) && !is_ours(r"C:\other\httpd.exe", Path::new(r"c:\rt")));
    }

    #[cfg(windows)]
    #[test]
    fn service_lookup_and_params() {
        assert!(win::service_exists("EventLog") && !win::service_exists("RunqoraNoSuchService"));
        let sv = comp("mariadb").service.unwrap();
        assert_eq!(service_params(&sv, &sv.install, Path::new(r"C:\rt")), "--install RunqoraMariaDB --defaults-file=C:/rt/mariadb/my.ini");
        // a path with a space is quoted so the elevated process sees one argument
        assert!(service_params(&sv, &sv.install, Path::new(r"C:\My Files\rt")).contains("\"--defaults-file=C:/My Files/rt/mariadb/my.ini\""));
    }

    fn tmp_root(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn settings_round_trip_and_a_corrupt_file_is_kept() {
        let d = tmp_root("rt-settings");
        assert_eq!(settings_in(&d), Settings::default());
        set_editor_in(&d, "code.exe").unwrap();
        set_autostart_in(&d, "apache", true).unwrap();
        let s = settings_in(&d);
        assert_eq!((s.editor.as_str(), s.autostart.get("apache")), ("code.exe", Some(&true)));
        set_editor_in(&d, "Notepad.exe").unwrap(); // the default is stored as "unset"
        assert_eq!(settings_in(&d).editor, "");
        assert!(set_editor_in(&d, "").is_err() && set_editor_in(&d, "a\"b").is_err());
        assert!(set_autostart_in(&d, "php", true).is_err()); // a library has nothing to start
        // a hand-edited file that no longer parses is backed up before we overwrite it
        fs::write(d.join("settings.json"), "{ not json").unwrap();
        set_editor_in(&d, "code.exe").unwrap();
        assert_eq!(fs::read_to_string(d.join("settings.json.bad")).unwrap(), "{ not json");
    }

    #[test]
    fn port_changes_are_validated_and_rewrite_the_configs() {
        let d = tmp_root("rt-ports");
        let m = |id: &str| manifest().components.into_iter().find(|c| c.id == id).unwrap();
        let (apache_default, maria_default) = (m("apache").port, m("mariadb").port);
        assert!(set_port_in(&d, "php", 8080).unwrap_err().contains("no port"));
        assert!(set_port_in(&d, "apache", 0).unwrap_err().contains("1 to 65535"));
        assert!(set_port_in(&d, "apache", 70000).unwrap_err().contains("1 to 65535"));
        assert!(set_port_in(&d, "apache", maria_default as u32).unwrap_err().contains("MariaDB already uses"));
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let busy = l.local_addr().unwrap().port() as u32;
        assert!(set_port_in(&d, "apache", busy).unwrap_err().contains("already used by"));
        drop(l);
        // not installed yet: the setting is saved, files are written later by the install
        let free = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port() as u32
        };
        set_port_in(&d, "apache", free).unwrap();
        assert_eq!(settings_in(&d).ports.get("apache"), Some(&(free as u16)));
        assert_eq!(component_with("apache", &settings_in(&d)).unwrap().port, free as u16);
        // installed: the config follows; back to the default clears the override
        fs::create_dir_all(d.join("apache/conf")).unwrap();
        fs::write(d.join("apache/conf/httpd.conf"), "Listen 80\nServerName localhost:80\n").unwrap();
        // (only when the default port is free on this machine: a web server already on port 80 rightly blocks the switch)
        if win::listener_pid(apache_default).is_none() {
            set_port_in(&d, "apache", apache_default as u32).unwrap();
            assert!(settings_in(&d).ports.is_empty());
            assert!(fs::read_to_string(d.join("apache/conf/httpd.conf")).unwrap().contains(&format!("Listen {apache_default}")));
        } else {
            assert!(set_port_in(&d, "apache", apache_default as u32).unwrap_err().contains("already used by"));
        }
        // pure text helpers are idempotent
        let ini = "[client]\nport=3306\n[mysqld]\nport=3306\nbind-address=127.0.0.1\n";
        let once = replace_port_lines(ini, 3399);
        assert_eq!(once.matches("port=3399").count(), 2);
        assert_eq!(once, replace_port_lines(&once, 3399));
        let pma = "$cfg['Servers'][$i]['host'] = '127.0.0.1';\n$cfg['Servers'][$i]['port'] = '3306';\n";
        assert!(replace_pma_port(pma, 3399).contains("['port'] = '3399';") && replace_pma_port(pma, 3399).contains("['host']"));
    }

    #[test]
    fn running_server_refuses_a_port_change() {
        let d = tmp_root("rt-ports-running");
        let c = component("apache").unwrap();
        fs::create_dir_all(d.join("apache/bin")).unwrap();
        fs::create_dir_all(d.join("apache/logs")).unwrap();
        fs::write(d.join(&c.binary), "").unwrap();
        fs::write(d.join(&c.pidfile), std::process::id().to_string()).unwrap(); // this very process counts as the live server
        let err = set_port_in(&d, "apache", 18999).unwrap_err();
        assert!(err.contains("stop Apache HTTP Server first"), "{err}");
    }

    #[test]
    fn ports_scan_lists_our_listener_with_a_name() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let rows = ports_scan();
        let mine = rows.iter().find(|r| r.port == port).expect("own listener is listed");
        assert_eq!(mine.pid, std::process::id());
        assert!(mine.name.starts_with("runqora"));
        assert!(rows.iter().all(|r| !r.name.is_empty()));
        assert!(win::process_name(std::process::id()).is_some_and(|n| n.starts_with("runqora")));
    }

    #[test]
    fn only_local_addresses_may_be_opened() {
        let r = std::env::temp_dir().join("rt-openurl");
        let _ = fs::remove_dir_all(&r);
        fs::create_dir_all(&r).unwrap();
        for ok in ["http://localhost/", "https://localhost:8443/a?b=1", "http://127.0.0.1:18080/phpmyadmin/", "http://[::1]:8080/", "http://shop.localhost/", "http://LOCALHOST"] {
            assert!(is_local_url(ok, &r), "{ok}");
        }
        for bad in ["https://example.com/", "http://localhost.evil.com/", "http://evil.com/localhost", "http://localhost@evil.com/", "file:///C:/Windows/win.ini", "javascript:alert(1)", "ftp://localhost/", "http://127.0.0.1.evil.com/", ""] {
            assert!(!is_local_url(bad, &r), "{bad}");
        }
        let mut s = settings_in(&r);
        s.custom.insert("apache.hostnames".into(), "myapp.test, *.dev.test".into());
        save_settings_in(&r, &s).unwrap();
        assert!(is_local_url("https://myapp.test/", &r) && is_local_url("http://a.dev.test:8080/", &r) && !is_local_url("http://dev.test.evil.com/", &r));
    }

    #[test]
    fn windows_colours_become_css_hex() {
        assert_eq!(abgr_to_hex(0xFFD47800), "#0078D4"); // the classic Windows blue
        assert_eq!(abgr_to_hex(0xFF0000FF), "#FF0000");
        assert_eq!(abgr_to_hex(0x00000000), "#000000");
        let a = system_accent();
        assert!(a.as_deref().is_none_or(|h| h.len() == 7 && h.starts_with('#') && h[1..].chars().all(|c| c.is_ascii_hexdigit())), "{a:?}");
    }

    #[test]
    fn the_database_starts_before_the_web_server() {
        let ids: Vec<String> = servers_in_start_order().into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["mariadb", "apache", "tomcat", "mailpit", "sftpgo"], "the database first, the rest as listed");
        let stop: Vec<&String> = ids.iter().rev().collect();
        assert_eq!(stop, ["sftpgo", "mailpit", "tomcat", "apache", "mariadb"], "stop order is the reverse");
    }

    #[test]
    fn notes_are_delivered_once() {
        take_notes();
        push_note("success", "a".into());
        push_note("error", "b".into());
        let n = take_notes();
        assert_eq!(n.iter().map(|x| (x.level, x.text.as_str())).collect::<Vec<_>>(), [("success", "a"), ("error", "b")]);
        assert!(take_notes().is_empty());
    }

    #[test]
    fn open_file_only_opens_listed_files() {
        let d = std::env::temp_dir().join("rt-open");
        let c = comp("apache");
        assert!(open_file(&c, &d, "../../Windows/win.ini").unwrap_err().contains("not on the module's list"));
        assert!(open_file(&c, &d, "apache/conf/httpd.conf").unwrap_err().contains("does not exist"));
        let mut bad = comp("php");
        bad.configs = vec!["../x.ini".into()];
        assert!(validate(&bad).is_err());
    }

    /// Manifest pins vs the real downloads. Set RT_DL_DIR to a folder holding
    /// apache.zip php.zip mariadb.zip phpmyadmin.zip.
    #[test]
    #[ignore]
    fn manifest_hashes_match_downloads() {
        let dir = PathBuf::from(std::env::var("RT_DL_DIR").expect("set RT_DL_DIR"));
        for c in manifest().components {
            let f = dir.join(format!("{}.zip", c.id));
            if f.exists() {
                verify_hash(&c, &f).unwrap(); // only what was downloaded: the newer modules are checked when someone installs them
            }
        }
    }

    /// Real Windows service round trip for Apache on a spare port. Needs a person at the
    /// keyboard: four UAC prompts (install, net start, net stop, remove).
    /// RT_DL_DIR must hold apache.zip. Root is C:\rt-svc-test so SYSTEM can read it.
    #[test]
    #[ignore]
    fn smoke_apache_service() {
        let dl = PathBuf::from(std::env::var("RT_DL_DIR").expect("set RT_DL_DIR"));
        let d = PathBuf::from(r"C:\rt-svc-test");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let mut c = component("apache").unwrap();
        c.port = 18081; // spare port, set on the value (not the environment: tests share one process)
        assert!(!win::service_exists(&c.service.as_ref().unwrap().name), "service already exists; remove it first");
        install_from_zip(&c, &dl.join("apache.zip"), &d).unwrap();
        let steps = (|| -> Result<(), String> {
            println!("{}", service_install(&c, &d)?);
            assert!(state(&c, &d).service_installed);
            println!("{}", start(&c, &d)?);
            let code = ureq::get("http://127.0.0.1:18081/").call().map(|r| r.status());
            println!("GET -> {code:?}");
            println!("{}", stop(&c, &d)?);
            assert_eq!(code.ok(), Some(200));
            Ok(())
        })();
        let removed = service_remove(&c, &d); // always try to leave the machine clean
        println!("{removed:?}");
        steps.unwrap();
        removed.unwrap();
        assert!(!win::service_exists(&c.service.as_ref().unwrap().name));
        assert!(!port_in_use(18081));
        let _ = fs::remove_dir_all(&d);
    }

    /// Whole stack: install all four, start Apache + MariaDB on spare ports, PHP runs,
    /// PHP reaches MariaDB, phpMyAdmin serves, then stop and the ports are free.
    #[test]
    #[ignore]
    fn smoke_full_stack() {
        let dl = PathBuf::from(std::env::var("RT_DL_DIR").expect("set RT_DL_DIR"));
        let d = std::env::temp_dir().join("rt-smoke");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let comp = |id: &str| {
            let mut c = component(id).unwrap();
            match id {
                "apache" => c.port = 18080,
                "mariadb" => c.port = 13306,
                _ => {}
            }
            c
        };
        let (apache, mariadb) = (comp("apache"), comp("mariadb"));
        for id in ["apache", "php", "mariadb", "phpmyadmin"] {
            let c = comp(id);
            install_from_zip(&c, &dl.join(format!("{id}.zip")), &d).unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        fs::write(d.join("htdocs/t.php"), "<?php $m=@new mysqli('127.0.0.1','root','',null,13306); echo 'php-'.PHP_MAJOR_VERSION.(extension_loaded('mbstring')?'-mb':'').($m->connect_errno?'-nodb':'-db');").unwrap();
        start(&mariadb, &d).unwrap();
        start(&apache, &d).unwrap();
        let get = |p: &str| ureq::get(&format!("http://127.0.0.1:18080{p}")).call().map(|r| (r.status(), r.into_string().unwrap_or_default()));
        let (home, php, pma) = (get("/"), get("/t.php"), get("/phpmyadmin/"));
        stop(&apache, &d).unwrap(); // always stop, even when a request failed
        stop(&mariadb, &d).unwrap();
        assert!(home.unwrap().1.contains("It works"));
        assert_eq!(php.unwrap().1, "php-8-mb-db");
        let (code, body) = pma.unwrap();
        assert!(code == 200 && body.contains("phpMyAdmin"));
        assert!(!port_in_use(18080) && !port_in_use(13306));
    }
}

#[cfg(test)]
mod platform_tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rq-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_tar_gz_unpacks_with_its_top_folder_dropped_and_programs_stay_runnable() {
        let d = temp("targz");
        let tgz = d.join("a.tar.gz");
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(fs::File::create(&tgz).unwrap(), flate2::Compression::fast()));
        for (name, body, mode) in [("pkg-1.0/bin/prog", "#!/bin/sh\n", 0o755), ("pkg-1.0/README", "hi", 0o644)] {
            let mut h = tar::Header::new_gnu();
            h.set_size(body.len() as u64);
            h.set_mode(mode);
            h.set_cksum();
            b.append_data(&mut h, name, body.as_bytes()).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap();
        unpack_to(&tgz, &d.join("out"), "*").unwrap();
        assert_eq!(fs::read_to_string(d.join("out/README")).unwrap(), "hi");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(d.join("out/bin/prog")).unwrap().permissions().mode() & 0o111, 0o111, "the run permission survives");
        }
        assert!(unpack_to(&tgz, &d.join("out2"), "pkg-1.0/").is_ok() && d.join("out2/bin/prog").exists(), "a named prefix works too");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_variant_replaces_the_download_and_a_missing_one_hides_the_component() {
        let json = r#"{"schema":1,"components":[
            {"id":"a","name":"A","version":"1","url":"u0","sha256":"h0","strip_prefix":"","extract_to":"a","binary":"a/x","variants":{"linux-x86_64":{"url":"u1","sha256":"h1","strip_prefix":"top/"}}},
            {"id":"b","name":"B","version":"1","url":"u","sha256":"h","strip_prefix":"","extract_to":"b","binary":"b/x"}]}"#;
        let linux = manifest_for(json, "linux-x86_64");
        let a = linux.components.iter().find(|c| c.id == "a").unwrap();
        assert_eq!((a.url.as_str(), a.sha256.as_str(), a.strip_prefix.as_str()), ("u1", "h1", "top/"));
        let mac = manifest_for(json, "macos-aarch64");
        assert_eq!(mac.components.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["b"], "no macOS build of A: it is not offered");
    }
}
