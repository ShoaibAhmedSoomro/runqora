//! Settings as forms, so nobody has to edit php.ini, my.ini or httpd.conf by hand.
//!
//! Every field says which file and key it maps to. The program's own file stays the source of truth: values are
//! read from it, and a change is written back line by line, leaving everything else (including the user's own
//! edits) alone. The same fields back the Settings page, `rq config` and the `runqora.ini` import/export.

use crate::{component, component_with, refresh_extra, Settings, save_settings_in, set_auto_update_in, set_autostart_in, set_editor_in, set_port_in, settings_in, start, state, stop};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

// ---- the catalogue ----

struct Def {
    module: &'static str,
    key: &'static str,
    label: &'static str,
    help: &'static str,
    /// number | size | bool | choice | text | path
    kind: &'static str,
    choices: &'static [(&'static str, &'static str)],
    min: u32,
    max: u32,
    default: &'static str,
    group: &'static str,
}

const NONE: &[(&str, &str)] = &[];
const ON_OFF: &[(&str, &str)] = &[("On", "On"), ("Off", "Off")];
const D: Def = Def { module: "", key: "", label: "", help: "", kind: "text", choices: NONE, min: 0, max: 0, default: "", group: "" };

const DEFS: &[Def] = &[
    Def { module: "general", key: "editor", label: "Editor", help: "The program that opens configuration files and logs.", kind: "text", default: "notepad.exe", group: "General", ..D },
    Def { module: "general", key: "auto_update", label: "Update automatically", help: "Checks every module and Runqora itself once a day. Patch updates install themselves while the servers are stopped; bigger upgrades and new Runqora versions always ask first.", kind: "choice", choices: &[("install", "Check everything and install patch updates"), ("notify", "Check everything and tell me"), ("off", "Do not check")], default: "install", group: "Updates", ..D },
    Def { module: "general", key: "launch_at_login", label: "Start Runqora when I sign in", help: "Opens Runqora hidden in the tray when you sign in to Windows. Servers marked Start with Runqora then start by themselves.", kind: "bool", default: "off", group: "Startup", ..D },
    Def { module: "general", key: "theme", label: "Theme", help: "Light, dark, or follow the Windows setting as it changes.", kind: "choice", choices: &[("system", "Match Windows"), ("light", "Light"), ("dark", "Dark")], default: "system", group: "Appearance", ..D },
    Def { module: "general", key: "accent_mode", label: "Accent colour", help: "Buttons, switches and highlights. Match Windows follows your personalisation colour live.", kind: "choice", choices: &[("system", "Match Windows"), ("custom", "Choose a colour")], default: "system", group: "Appearance", ..D },
    Def { module: "general", key: "accent_color", label: "Your colour", help: "Used when the accent is set to a custom colour. Text on it is kept readable automatically.", kind: "color", default: "#0067C0", group: "Appearance", ..D },
    Def { module: "apache", key: "port", label: "Port", help: "Where the web server listens. 80 is the default; use 8080 if something else has 80.", kind: "number", min: 1, max: 65535, default: "80", group: "Server", ..D },
    Def { module: "apache", key: "document_root", label: "Website folder", help: "The folder Apache serves. Leave empty to use the htdocs folder inside the install folder.", kind: "path", default: "", group: "Server", ..D },
    Def { module: "apache", key: "allow_override", label: ".htaccess files", help: "Let sites change Apache settings with .htaccess files. Most frameworks and CMSs need this.", kind: "bool", default: "on", group: "Websites", ..D },
    Def { module: "apache", key: "directory_listing", label: "Show folder contents", help: "List the files in a folder that has no index page.", kind: "bool", default: "on", group: "Websites", ..D },
    Def { module: "apache", key: "https", label: "Serve sites over HTTPS", help: "Makes a private certificate for localhost, the sites you add and the names below, and turns on Apache's TLS. Trust it once on this PC (button below) so browsers show no warning.", kind: "bool", default: "off", group: "HTTPS", ..D },
    Def { module: "apache", key: "https_port", label: "HTTPS port", help: "443 is the standard. Use 8443 if something else has 443.", kind: "number", min: 1, max: 65535, default: "443", group: "HTTPS", ..D },
    Def { module: "apache", key: "hostnames", label: "Extra names", help: "Names the certificate should also cover, separated by commas, like myapp.test, shop.local. localhost is always included, and so is every site you add.", kind: "names", default: "", group: "HTTPS", ..D },
    Def { module: "apache", key: "autostart", label: "Start with Runqora", help: "Start Apache when Runqora opens.", kind: "bool", default: "off", group: "Startup", ..D },
    Def { module: "php", key: "memory_limit", label: "Memory limit", help: "The most memory one script may use, such as 256M. -1 means no limit.", kind: "size", default: "128M", group: "Limits", ..D },
    Def { module: "php", key: "upload_max_filesize", label: "Largest upload", help: "The biggest file a visitor can upload, such as 64M.", kind: "size", default: "2M", group: "Limits", ..D },
    Def { module: "php", key: "post_max_size", label: "Largest form post", help: "Keep this at least as large as the largest upload.", kind: "size", default: "8M", group: "Limits", ..D },
    Def { module: "php", key: "max_execution_time", label: "Script time limit", help: "Seconds before a script is stopped. 0 means no limit.", kind: "number", min: 0, max: 3600, default: "30", group: "Limits", ..D },
    Def { module: "php", key: "display_errors", label: "Show errors on the page", help: "Handy while developing. Turn it off for anything public.", kind: "choice", choices: ON_OFF, default: "Off", group: "Errors", ..D },
    Def { module: "php", key: "error_reporting", label: "Which errors to report", help: "What PHP complains about.", kind: "choice", choices: &[("E_ALL", "Everything (best for development)"), ("E_ALL & ~E_DEPRECATED & ~E_STRICT", "Everything except deprecation notices"), ("E_ALL & ~E_NOTICE & ~E_DEPRECATED", "Hide notices too (older apps)")], default: "E_ALL & ~E_DEPRECATED & ~E_STRICT", group: "Errors", ..D },
    Def { module: "php", key: "date.timezone", label: "Time zone", help: "For example Europe/Paris or Asia/Dubai. Dates in PHP use this.", kind: "text", default: "UTC", group: "Locale", ..D },
    Def { module: "mariadb", key: "port", label: "Port", help: "Where the database listens. 3306 is the default.", kind: "number", min: 1, max: 65535, default: "3306", group: "Server", ..D },
    Def { module: "mariadb", key: "bind_address", label: "Who can connect", help: "Keep this PC only unless you really need other computers. The root account has no password.", kind: "choice", choices: &[("127.0.0.1", "This PC only"), ("0.0.0.0", "Other computers on the network too")], default: "127.0.0.1", group: "Server", ..D },
    Def { module: "mariadb", key: "max_connections", label: "Most connections", help: "How many clients can be connected at once.", kind: "number", min: 10, max: 10000, default: "151", group: "Performance", ..D },
    Def { module: "mariadb", key: "innodb_buffer_pool_size", label: "Memory for data", help: "Bigger is faster for large databases, such as 512M.", kind: "size", default: "128M", group: "Performance", ..D },
    Def { module: "mariadb", key: "max_allowed_packet", label: "Largest query", help: "Raise this if imports fail with a packet size error.", kind: "size", default: "16M", group: "Performance", ..D },
    Def { module: "mariadb", key: "character_set_server", label: "Default character set", help: "utf8mb4 stores every language and emoji.", kind: "choice", choices: &[("utf8mb4", "utf8mb4 (recommended)"), ("utf8mb3", "utf8mb3"), ("latin1", "latin1 (older sites)")], default: "utf8mb4", group: "Data", ..D },
    Def { module: "mariadb", key: "autostart", label: "Start with Runqora", help: "Start MariaDB when Runqora opens.", kind: "bool", default: "off", group: "Startup", ..D },
];

/// PHP extensions offered as switches, when their DLL is present.
const EXTENSIONS: &[(&str, &str)] = &[
    ("bz2", "bzip2 compression"), ("curl", "Web requests from PHP"), ("exif", "Photo metadata"), ("fileinfo", "File type detection"), ("ftp", "FTP client"), ("gd", "Image drawing and resizing"),
    ("gettext", "Translations"), ("gmp", "Very large numbers"), ("intl", "Internationalisation"), ("ldap", "Directory lookups"), ("mbstring", "Text in every language (phpMyAdmin needs it)"),
    ("mysqli", "MariaDB / MySQL access (phpMyAdmin needs it)"), ("openssl", "Encryption and secure connections"), ("pdo_mysql", "MariaDB / MySQL through PDO"), ("pdo_pgsql", "PostgreSQL through PDO"),
    ("pdo_sqlite", "SQLite through PDO"), ("pgsql", "PostgreSQL access"), ("soap", "SOAP web services"), ("sockets", "Network sockets"), ("sodium", "Modern encryption"), ("xsl", "XSLT"), ("zip", "ZIP archives"),
];

#[derive(Serialize, Clone)]
pub struct Field {
    pub module: String,
    pub key: String,
    pub label: String,
    pub help: String,
    pub kind: String,
    pub choices: Vec<(String, String)>,
    pub min: u32,
    pub max: u32,
    pub group: String,
    pub value: String,
    pub default: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Applied {
    pub changed: bool,
    pub message: String,
    /// the module whose running server must restart for this to take effect
    pub restart: Option<String>,
}

// ---- reading and writing the ini-style files ----

fn norm(k: &str) -> String {
    k.trim().to_lowercase().replace('_', "-")
}

fn section_name(line: &str) -> Option<String> {
    let t = line.trim();
    (t.starts_with('[') && t.ends_with(']')).then(|| t[1..t.len() - 1].trim().to_lowercase())
}

fn clean_value(v: &str) -> String {
    let v = v.trim();
    if let Some(rest) = v.strip_prefix('"') {
        return rest.split('"').next().unwrap_or("").to_string();
    }
    let cut = v.char_indices().find(|(i, c)| (*c == ';' || *c == '#') && v[..*i].ends_with(char::is_whitespace)).map_or(v.len(), |(i, _)| i);
    v[..cut].trim().to_string()
}

/// (commented out?, normalized key, value) of a `key = value` line. None for prose, blanks and [sections].
pub fn parse_line(line: &str) -> Option<(bool, String, String)> {
    let t = line.trim();
    let commented = t.starts_with(';') || t.starts_with('#');
    let body = if commented { t.trim_start_matches([';', '#']).trim() } else { t };
    if body.is_empty() || body.starts_with('[') {
        return None;
    }
    let (k, v) = body.split_once('=')?;
    let k = k.trim();
    (!k.is_empty() && !k.contains(char::is_whitespace)).then(|| (commented, norm(k), clean_value(v)))
}

/// Current value of `key`. `section` None searches the whole file (PHP ignores section names), Some limits it to that section.
pub fn ini_get(text: &str, section: Option<&str>, key: &str) -> Option<String> {
    let (nk, mut cur, mut found) = (norm(key), String::new(), None);
    for l in text.lines() {
        if let Some(s) = section_name(l) {
            cur = s;
        } else if let Some((false, k, v)) = parse_line(l) {
            if k == nk && section.is_none_or(|s| cur == s) {
                found = Some(v); // the last one wins, as it does for PHP and MariaDB
            }
        }
    }
    found
}

/// Set `key = value`: replace the active line, else uncomment a commented example, else add it (inside `section` if given).
pub fn ini_set(text: &str, section: Option<&str>, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let nk = norm(key);
    let new_line = format!("{key} = {value}");
    let (mut cur, mut active, mut commented) = (String::new(), None, None);
    let (mut sec_start, mut sec_end) = (None, lines.len());
    for (i, l) in lines.iter().enumerate() {
        if let Some(s) = section_name(l) {
            if section.is_some_and(|w| s != w) && sec_start.is_some() && sec_end == lines.len() {
                sec_end = i;
            }
            if section.is_some_and(|w| s == w) {
                sec_start = Some(i);
            }
            cur = s;
            continue;
        }
        let in_scope = section.is_none_or(|s| cur == s);
        if let (true, Some((c, k, _))) = (in_scope, parse_line(l)) {
            if k == nk {
                if c { commented = Some(i) } else { active = Some(i) }
            }
        }
    }
    if let Some(i) = active.or(commented) {
        lines[i] = new_line;
    } else if let (Some(w), Some(start)) = (section, sec_start) {
        let _ = w;
        let at = (start + 1..sec_end).rev().find(|i| !lines[*i].trim().is_empty()).map_or(start + 1, |i| i + 1);
        lines.insert(at, new_line);
    } else if let Some(w) = section {
        lines.push(String::new());
        lines.push(format!("[{w}]"));
        lines.push(new_line);
    } else {
        lines.push(new_line);
    }
    lines.join("\n") + "\n"
}

fn ext_line(line: &str) -> Option<(bool, String)> {
    let (c, k, v) = parse_line(line)?;
    (k == "extension").then(|| {
        let n = v.trim_matches('"').trim();
        let n = n.strip_prefix("php_").unwrap_or(n);
        (c, n.strip_suffix(".dll").unwrap_or(n).to_string())
    })
}

pub fn php_ext_active(text: &str) -> BTreeSet<String> {
    text.lines().filter_map(ext_line).filter(|(c, _)| !*c).map(|(_, n)| n).collect()
}

/// Switch an extension on or off. Turning on prefers the last commented line (the real list, not the prose example).
pub fn php_ext_set(text: &str, name: &str, on: bool) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let hits: Vec<(usize, bool)> = lines.iter().enumerate().filter_map(|(i, l)| ext_line(l).filter(|(_, n)| n == name).map(|(c, _)| (i, c))).collect();
    if on {
        if hits.iter().any(|(_, c)| !*c) {
            return text.to_string();
        }
        if let Some((i, _)) = hits.last() {
            lines[*i] = format!("extension={name}");
        } else {
            let at = lines.iter().rposition(|l| ext_line(l).is_some_and(|(c, _)| !c)).map_or(lines.len(), |i| i + 1);
            lines.insert(at, format!("extension={name}"));
        }
    } else {
        for (i, c) in &hits {
            if !*c {
                lines[*i] = format!(";{}", lines[*i].trim_start());
            }
        }
    }
    lines.join("\n") + "\n"
}

fn backup_once(p: &Path) {
    let b = p.with_extension(format!("{}.runqora-backup", p.extension().and_then(|e| e.to_str()).unwrap_or("")));
    if p.exists() && !b.exists() {
        let _ = fs::copy(p, b); // the file as it was before Runqora first touched it
    }
}

fn read_text(p: &Path) -> Result<String, String> {
    fs::read_to_string(p).map_err(|_| format!("{} does not exist yet; install the module first", p.file_name().and_then(|n| n.to_str()).unwrap_or("the file")))
}

fn edit(p: &Path, f: impl FnOnce(&str) -> String) -> Result<(), String> {
    let t = read_text(p)?;
    backup_once(p);
    fs::write(p, f(&t)).map_err(|e| e.to_string())
}

fn php_ini(root: &Path) -> PathBuf {
    root.join("php/php.ini")
}

fn my_ini(root: &Path) -> PathBuf {
    root.join("mariadb/my.ini")
}

/// Extensions we offer that this PHP build actually has.
pub fn available_extensions(root: &Path) -> Vec<(&'static str, &'static str)> {
    EXTENSIONS.iter().filter(|(n, _)| root.join(format!("php/ext/php_{n}.dll")).exists()).copied().collect()
}

// ---- values ----

fn on_off(b: bool) -> String {
    if b { "on" } else { "off" }.to_string()
}

fn read_value(root: &Path, module: &str, key: &str) -> Option<String> {
    let s = settings_in(root);
    match (module, key) {
        ("general", "editor") => Some(if s.editor.is_empty() { "notepad.exe".into() } else { s.editor }),
        ("general", "auto_update") => Some(if s.auto_update.is_empty() { "install".into() } else { s.auto_update }),
        ("general", "launch_at_login") => Some(on_off(crate::launch_at_login())),
        ("general", k @ ("theme" | "accent_mode" | "accent_color")) => s.custom.get(&format!("general.{k}")).cloned(),
        (m, "port") => component_with(m, &s).ok().map(|c| c.port.to_string()),
        (m, "autostart") => Some(on_off(s.autostart.get(m).copied().unwrap_or(false))),
        ("apache", "document_root") => Some(s.custom.get("apache.document_root").cloned().unwrap_or_default()),
        ("apache", "https") => Some(s.custom.get("apache.https").cloned().unwrap_or_else(|| "off".into())),
        ("apache", "https_port") => Some(s.custom.get("apache.https_port").cloned().unwrap_or_else(|| "443".into())),
        ("apache", "hostnames") => Some(s.custom.get("apache.hostnames").cloned().unwrap_or_default()),
        ("apache", k) => Some(s.custom.get(&format!("apache.{k}")).cloned().unwrap_or_else(|| "on".into())),
        ("php", k) if k.starts_with("ext.") => Some(on_off(php_ext_active(&read_text(&php_ini(root)).ok()?).contains(&k[4..]))),
        ("php", k) => ini_get(&read_text(&php_ini(root)).ok()?, None, k),
        ("mariadb", k) => ini_get(&read_text(&my_ini(root)).ok()?, Some("mysqld"), k),
        _ => None,
    }
}

fn installed(root: &Path, module: &str) -> bool {
    module == "general" || component(module).is_ok_and(|c| root.join(&c.binary).exists())
}

/// All fields of a module with their current values (defaults where the file does not say).
pub fn config_get(root: &Path, module: &str) -> Result<Vec<Field>, String> {
    if !["general", "apache", "php", "mariadb"].contains(&module) {
        return Err(format!("{module} has no settings"));
    }
    if !installed(root, module) {
        return Ok(vec![]);
    }
    let field = |d: &Def| Field {
        module: d.module.into(),
        key: d.key.into(),
        label: d.label.into(),
        help: d.help.into(),
        kind: d.kind.into(),
        choices: d.choices.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        min: d.min,
        max: d.max,
        group: d.group.into(),
        value: read_value(root, d.module, d.key).filter(|v| !v.is_empty() || d.kind == "path").unwrap_or_else(|| d.default.into()),
        default: d.default.into(),
    };
    let mut out: Vec<Field> = DEFS.iter().filter(|d| d.module == module).map(field).collect();
    if module == "php" {
        for (n, help) in available_extensions(root) {
            out.push(Field { module: "php".into(), key: format!("ext.{n}"), label: n.into(), help: help.into(), kind: "bool".into(), choices: vec![], min: 0, max: 0, group: "Extensions".into(), value: read_value(root, "php", &format!("ext.{n}")).unwrap_or_else(|| "off".into()), default: "off".into() });
        }
    }
    Ok(out)
}

fn def(module: &str, key: &str, root: &Path) -> Result<Def, String> {
    if module == "php" && key.starts_with("ext.") {
        return if available_extensions(root).iter().any(|(n, _)| *n == &key[4..]) {
            Ok(Def { module: "php", kind: "bool", default: "off", group: "Extensions", ..D })
        } else {
            Err(format!("this PHP has no {} extension", &key[4..]))
        };
    }
    DEFS.iter().find(|d| d.module == module && norm(d.key) == norm(key)).map(|d| Def { ..*d }).ok_or_else(|| format!("there is no setting called {module}.{key}"))
}

/// Check a typed value and bring it to its canonical spelling.
fn normalize(d: &Def, raw: &str) -> Result<String, String> {
    let v = raw.trim();
    match d.kind {
        "number" => {
            let n: u32 = v.parse().map_err(|_| format!("{} must be a whole number", d.label))?;
            if n < d.min || n > d.max {
                return Err(format!("{} must be between {} and {}", d.label, d.min, d.max));
            }
            Ok(n.to_string())
        }
        "size" => {
            let up = v.to_uppercase();
            let (digits, unit) = up.split_at(up.trim_end_matches(['K', 'M', 'G']).len());
            let ok = (digits == "-1" && d.key == "memory_limit" && unit.is_empty()) || (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) && unit.len() <= 1 && digits != "0");
            if ok { Ok(up) } else { Err(format!("{} should look like 64M, 512K or 1G", d.label)) }
        }
        "bool" => match v.to_lowercase().as_str() {
            "on" | "true" | "yes" | "1" => Ok("on".into()),
            "off" | "false" | "no" | "0" => Ok("off".into()),
            _ => Err(format!("{} must be on or off", d.label)),
        },
        "choice" => d.choices.iter().find(|(c, _)| c.eq_ignore_ascii_case(v)).map(|(c, _)| c.to_string()).ok_or_else(|| format!("{} must be one of: {}", d.label, d.choices.iter().map(|(c, _)| *c).collect::<Vec<_>>().join(", "))),
        "names" => crate::https::parse_hostnames(v).map(|n| n.join(", ")),
        "color" => {
            let h = v.trim_start_matches('#');
            let h = if h.len() == 3 { h.chars().flat_map(|c| [c, c]).collect() } else { h.to_string() };
            if h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()) { Ok(format!("#{}", h.to_uppercase())) } else { Err(format!("{} should look like #0067C0", d.label)) }
        }
        "path" => {
            if v.is_empty() {
                return Ok(String::new());
            }
            let p = Path::new(v);
            if !p.is_absolute() || v.contains(['"', '\n', '\r', '<', '>', '|', '*', '?']) {
                return Err(format!("{} must be a full folder path such as C:\\Sites", d.label));
            }
            if p.parent().is_none() {
                return Err(format!("{} cannot be the root of a drive", d.label));
            }
            Ok(v.replace('/', "\\"))
        }
        _ => {
            if v.is_empty() || v.contains(['"', '\n', '\r']) {
                return Err(format!("{} cannot be empty or contain quotes", d.label));
            }
            if d.key == "date.timezone" && !v.chars().all(|c| c.is_ascii_alphanumeric() || "/_+-".contains(c)) {
                return Err("a time zone looks like Europe/Paris".into());
            }
            Ok(v.to_string())
        }
    }
}

/// Ask PHP itself whether it knows a time zone name. None when PHP cannot answer (not runnable): then we do not block.
fn timezone_known(root: &Path, tz: &str) -> Option<bool> {
    if !tz.chars().all(|c| c.is_ascii_alphanumeric() || "/_+-".contains(c)) {
        return Some(false); // normalize already checks this; never put anything else on a command line
    }
    use crate::CommandExt as _;
    let out = std::process::Command::new(root.join("php").join(crate::exe("php"))).args(["-n", "-r", &format!("echo in_array('{tz}', timezone_identifiers_list(DateTimeZone::ALL_WITH_BC), true) ? 'yes' : 'no';")]).creation_flags(0x0800_0000).output().ok()?;
    match String::from_utf8_lossy(&out.stdout).trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

/// The module whose running server must restart for a change in `module` to apply.
fn restart_target(root: &Path, module: &str) -> Option<String> {
    let id = if module == "php" { "apache" } else { module };
    let c = component(id).ok().filter(|c| c.args.is_some() && root.join(&c.binary).exists())?;
    (state(&c, root).status == "running").then(|| id.to_string())
}

/// Change one setting. With `dry` it only checks and reports whether anything would change.
pub fn config_set(root: &Path, module: &str, key: &str, raw: &str, dry: bool) -> Result<Applied, String> {
    let d = def(module, key, root)?;
    if !installed(root, module) {
        return Err(format!("{module} is not installed yet"));
    }
    let key = if module == "php" && key.starts_with("ext.") { key.to_string() } else { d.key.to_string() };
    let v = normalize(&d, raw)?;
    if module == "php" && key == "date.timezone" && timezone_known(root, &v) == Some(false) {
        return Err(format!("PHP does not know the time zone {v}. Use a name like Europe/Paris or Asia/Dubai"));
    }
    let current = read_value(root, module, &key);
    let same = match current.as_deref() {
        Some(c) => c.eq_ignore_ascii_case(&v),
        None => v.eq_ignore_ascii_case(d.default), // a setting the file does not mention already has its default
    };
    if same {
        return Ok(Applied { changed: false, message: format!("{} is already {}", if d.label.is_empty() { &key } else { d.label }, if v.is_empty() { "at its default" } else { &v }), restart: None });
    }
    if dry {
        return Ok(Applied { changed: true, message: format!("would change {} to {}", key, if v.is_empty() { "the default" } else { &v }), restart: None });
    }
    let mut warn = String::new();
    match (module, key.as_str()) {
        ("general", "editor") => return set_editor_in(root, &v).map(|m| Applied { changed: true, message: m, restart: None }),
        ("general", "auto_update") => return set_auto_update_in(root, &v).map(|m| Applied { changed: true, message: m, restart: None }),
        ("general", "launch_at_login") => return crate::set_launch_at_login(v == "on").map(|m| Applied { changed: true, message: m, restart: None }),
        ("general", k @ ("theme" | "accent_mode" | "accent_color")) => {
            let mut s = settings_in(root);
            s.custom.insert(format!("general.{k}"), v.clone());
            save_settings_in(root, &s)?;
            return Ok(Applied { changed: true, message: format!("{} set to {}", d.label, d.choices.iter().find(|(c, _)| *c == v).map_or(v.as_str(), |(_, l)| *l)), restart: None });
        }
        (m, "port") => return set_port_in(root, m, v.parse().unwrap_or(0)).map(|m| Applied { changed: true, message: format!("{m}. Start it again to use the new port"), restart: None }),
        (m, "autostart") => return set_autostart_in(root, m, v == "on").map(|m| Applied { changed: true, message: m, restart: None }),
        ("apache", k) => {
            let mut s = settings_in(root);
            let on = |s: &Settings| s.custom.get("apache.https").is_some_and(|x| x == "on");
            let extra = |s: &Settings, now: &str| {
                let mut v = crate::https::parse_hostnames(&if k == "hostnames" { now.to_string() } else { s.custom.get("apache.hostnames").cloned().unwrap_or_default() }).unwrap_or_default();
                v.extend(s.sites.keys().map(|n| format!("{n}.localhost"))); // the certificate always covers every site
                v
            };
            match k {
                "https" if v == "on" => {
                    let port: u16 = s.custom.get("apache.https_port").and_then(|p| p.parse().ok()).unwrap_or(443);
                    check_https_port(root, &s, port)?;
                    crate::https::ensure(root, &extra(&s, &v))?; // before saving: a failed certificate must not leave HTTPS switched on
                }
                "https_port" => check_https_port(root, &s, v.parse().unwrap_or(0))?,
                "hostnames" if on(&s) => {
                    crate::https::ensure(root, &extra(&s, &v))?;
                }
                _ => {}
            }
            if k == "document_root" && !v.is_empty() {
                fs::create_dir_all(&v).map_err(|e| format!("could not create {v}: {e}"))?;
            }
            if v.is_empty() {
                s.custom.remove(&format!("apache.{k}"));
            } else {
                s.custom.insert(format!("apache.{k}"), v.clone());
            }
            save_settings_in(root, &s)?;
            refresh_extra(root)?;
        }
        ("php", k) if k.starts_with("ext.") => {
            let name = &k[4..];
            edit(&php_ini(root), |t| php_ext_set(t, name, v == "on"))?;
        }
        ("php", k) => {
            let val = if k == "date.timezone" { format!("\"{v}\"") } else { v.clone() };
            edit(&php_ini(root), |t| ini_set(t, None, k, &val))?;
        }
        ("mariadb", k) => {
            if k == "bind_address" && v == "0.0.0.0" {
                warn = " Other computers can now reach the database, and the root account has no password: only do this on a network you trust.".into();
            }
            edit(&my_ini(root), |t| ini_set(t, Some("mysqld"), k, &v))?;
        }
        _ => return Err(format!("there is no setting called {module}.{key}")),
    }
    let restart = restart_target(root, module);
    let label = if d.label.is_empty() { key.clone() } else { d.label.to_string() };
    Ok(Applied { changed: true, message: format!("{label} set to {}.{warn}{}", if v.is_empty() { "the default" } else { &v }, restart.as_ref().map_or(String::new(), |m| format!(" Restart {} to apply it.", component(m).map_or(m.clone(), |c| c.name)))), restart })
}

/// The HTTPS port must be free, and not Apache's own HTTP port or another module's.
fn check_https_port(root: &Path, s: &Settings, port: u16) -> Result<(), String> {
    if port == 0 {
        return Err("a port is a number from 1 to 65535".into());
    }
    for id in ["apache", "mariadb"] {
        if let Ok(c) = component_with(id, s) {
            if c.port == port {
                return Err(format!("{} already uses port {port}", c.name));
            }
        }
    }
    let apache_pid = component_with("apache", s).ok().and_then(|c| crate::read_pid(&c, root));
    match crate::win::listener_pid(port) {
        Some(pid) if Some(pid) != apache_pid => Err(format!("port {port} is already used by {} (PID {pid})", crate::describe_blocker(pid, root).name)),
        _ => Ok(()),
    }
}

pub fn restart_module(root: &Path, id: &str) -> Result<String, String> {
    let c = component(id)?;
    stop(&c, root)?;
    start(&c, root)
}

// ---- runqora.ini: the whole configuration in one portable file ----

pub fn export_ini(root: &Path) -> String {
    let mut out = String::from("; Runqora settings. Edit and import this file (Settings > Configuration file, or `rq config import`).\n; Keys are not case sensitive and `_` or `-` are the same. Leave a key out to leave that setting alone.\n");
    for module in ["general", "apache", "php", "mariadb"] {
        let Ok(fields) = config_get(root, module) else { continue };
        if fields.is_empty() {
            continue;
        }
        out += &format!("\n[{module}]\n");
        for f in fields.iter().filter(|f| !f.key.starts_with("ext.")) {
            out += &format!("; {}\n{} = {}\n", f.help, f.key, f.value);
        }
        if module == "php" {
            let on: Vec<&str> = fields.iter().filter(|f| f.key.starts_with("ext.") && f.value == "on").map(|f| &f.key[4..]).collect();
            out += &format!("; Extensions that are switched on; all other available ones are switched off.\n; Available: {}\nextensions = {}\n", fields.iter().filter(|f| f.key.starts_with("ext.")).map(|f| &f.key[4..]).collect::<Vec<_>>().join(", "), on.join(", "));
        }
    }
    out
}

#[derive(Serialize, Clone, Debug)]
pub struct ImportItem {
    pub module: String,
    pub key: String,
    pub value: String,
    /// ok | unchanged | error | unknown
    pub status: &'static str,
    pub message: String,
}

/// Read a runqora.ini. Without `apply` nothing is written: each line is only checked and compared with today's value.
pub fn import_ini(root: &Path, text: &str, apply: bool) -> Vec<ImportItem> {
    let mut items = Vec::new();
    let mut module = String::new();
    let mut push = |module: &str, key: &str, value: &str, r: Result<Applied, String>| {
        let (status, message) = match r {
            Ok(a) if a.changed => ("ok", a.message),
            Ok(a) => ("unchanged", a.message),
            Err(e) => ("error", e),
        };
        items.push(ImportItem { module: module.into(), key: key.into(), value: value.into(), status, message });
    };
    for line in text.lines() {
        if let Some(s) = section_name(line) {
            module = s;
            continue;
        }
        let Some((false, _, _)) = parse_line(line) else { continue };
        let (k, v) = line.split_once('=').map(|(k, v)| (k.trim().to_string(), clean_value(v))).unwrap_or_default();
        if module == "php" && norm(&k) == "extensions" {
            let want: BTreeSet<String> = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            for (n, _) in available_extensions(root) {
                let key = format!("ext.{n}");
                push("php", &key, if want.contains(n) { "on" } else { "off" }, config_set(root, "php", &key, if want.contains(n) { "on" } else { "off" }, !apply));
            }
            for n in want.iter().filter(|n| !available_extensions(root).iter().any(|(a, _)| a == &n.as_str())) {
                push("php", &format!("ext.{n}"), "on", Err(format!("this PHP has no {n} extension")));
            }
            continue;
        }
        if !["general", "apache", "php", "mariadb"].contains(&module.as_str()) {
            push(&module, &k, &v, Err(format!("[{module}] is not a section Runqora knows")));
            continue;
        }
        let r = config_set(root, &module, &k, &v, !apply);
        push(&module, &k, &v, r);
    }
    items
}

pub fn ini_path(root: &Path) -> PathBuf {
    root.join("runqora.ini")
}

pub fn export_to_file(root: &Path) -> Result<PathBuf, String> {
    let p = ini_path(root);
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    fs::write(&p, export_ini(root)).map_err(|e| e.to_string())?;
    Ok(p)
}

#[cfg(test)]
mod tests;
