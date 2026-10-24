//! HTTPS for local sites: a private certificate authority, a certificate for localhost and friends, and what Apache needs.
//!
//! Browsers only accept a certificate whose issuer they trust. So Runqora makes ONE local authority ("Runqora Local CA")
//! and signs the server certificate with it; trusting that single authority on this PC (a button, with Windows' own
//! confirmation) makes every site below it green. Nothing here touches Windows' trust store unless asked.
//! Uses the openssl.exe that ships with Apache, so there is nothing extra to download.

use crate::CommandExt as _;
use crate::fwd;
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const CA_NAME: &str = "Runqora Local CA";
const NO_WINDOW: u32 = 0x0800_0000;

/// The extra names saved in Settings, and one name.localhost for every site.
pub fn configured_names(root: &Path) -> Vec<String> {
    let s = crate::settings_in(root);
    let mut v = parse_hostnames(s.custom.get("apache.hostnames").map(String::as_str).unwrap_or("")).unwrap_or_default();
    v.extend(s.sites.keys().map(|n| format!("{n}.localhost")));
    v
}

pub fn ssl_dir(root: &Path) -> PathBuf {
    root.join("ssl")
}

fn openssl_exe(root: &Path) -> Result<PathBuf, String> {
    let p = root.join("apache/bin").join(crate::exe("openssl"));
    if cfg!(unix) && !p.exists() {
        return Ok("openssl".into()); // every Linux and macOS ships one
    }
    p.exists().then_some(p).ok_or_else(|| "Apache's openssl.exe is missing; install Apache first".to_string())
}

fn openssl(root: &Path, args: &[&str]) -> Result<String, String> {
    let dir = ssl_dir(root);
    let o = Command::new(openssl_exe(root)?)
        .args(args)
        .current_dir(&dir)
        .env("OPENSSL_CONF", dir.join("openssl.cnf"))
        .stdin(Stdio::null())
        .creation_flags(NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())?;
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    if o.status.success() { Ok(text) } else { Err(format!("openssl {} failed: {}", args.first().unwrap_or(&""), text.trim().lines().last().unwrap_or(""))) }
}

// ---- names ----

fn is_ip(n: &str) -> bool {
    n.parse::<std::net::IpAddr>().is_ok()
}

/// Check and tidy a list of extra host names ("myapp.test, shop.local"). Wildcards only as a leading "*.".
pub fn parse_hostnames(raw: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = vec![];
    for n in raw.split([',', ' ', ';', '\n']).map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()) {
        let body = n.strip_prefix("*.").unwrap_or(&n);
        // every dot-separated label: 1 to 63 letters, digits or dashes, not starting or ending with a dash
        let labels_ok = body.split('.').all(|l| (1..=63).contains(&l.len()) && !l.starts_with('-') && !l.ends_with('-') && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        let ok = n.len() <= 253 && (is_ip(&n) || (labels_ok && (!n.contains('*') || n.starts_with("*."))));
        if !ok {
            return Err(format!("'{n}' is not a host name (use letters, digits, dots and dashes, like myapp.test or *.myapp.test)"));
        }
        if !out.contains(&n) {
            out.push(n);
        }
    }
    Ok(out)
}

/// Everything the certificate must be valid for.
pub fn all_names(extra: &[String]) -> Vec<String> {
    // no "*.localhost": OpenSSL (and so most clients) refuse a wildcard with fewer than two labels under it, so each site
    // name such as shop.localhost is listed exactly
    let mut v: Vec<String> = ["localhost", "127.0.0.1", "::1"].iter().map(|s| s.to_string()).collect();
    for n in extra {
        if !v.contains(n) {
            v.push(n.clone());
        }
    }
    v
}

pub fn san_line(names: &[String]) -> String {
    format!("subjectAltName = {}", names.iter().map(|n| if is_ip(n) { format!("IP:{n}") } else { format!("DNS:{n}") }).collect::<Vec<_>>().join(", "))
}

// ---- certificates ----

#[derive(Serialize, Clone, Debug, Default)]
pub struct HttpsStatus {
    pub ca_exists: bool,
    pub cert_exists: bool,
    pub names: Vec<String>,
    /// notAfter of the server certificate
    pub expires: Option<String>,
    pub trusted: bool,
    pub ca_path: String,
    pub folder: String,
}

fn field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| l.trim().strip_prefix(key).map(|v| v.trim().to_string()))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn fingerprint(root: &Path, file: &str) -> Option<String> {
    openssl(root, &["x509", "-in", file, "-noout", "-fingerprint", "-sha1"]).ok().and_then(|t| field(&t, "sha1 Fingerprint=").or_else(|| field(&t, "SHA1 Fingerprint=")))
}

/// Make the authority and server certificate if they are missing, or the names changed, or it expires within 30 days.
pub fn ensure(root: &Path, extra: &[String]) -> Result<HttpsStatus, String> {
    let dir = ssl_dir(root);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join("openssl.cnf"), "[req]\ndistinguished_name = dn\nprompt = no\n[dn]\nCN = Runqora\n").map_err(|e| e.to_string())?;
    if !dir.join("ca.crt").exists() || !dir.join("ca.key").exists() {
        openssl(root, &["ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", "ca.key"])?;
        openssl(root, &["req", "-new", "-x509", "-key", "ca.key", "-sha256", "-days", "3650", "-subj", &format!("/CN={CA_NAME}/O=Runqora"), "-config", "openssl.cnf", "-addext", "basicConstraints=critical,CA:TRUE,pathlen:0", "-addext", "keyUsage=critical,keyCertSign,cRLSign", "-out", "ca.crt"])?;
        let _ = fs::remove_file(dir.join("server.crt")); // a new authority invalidates whatever it signed
    }
    let names = all_names(extra);
    let wanted = names.join("\n");
    let stale = fs::read_to_string(dir.join("names.txt")).map_or(true, |t| t.trim() != wanted) || !dir.join("server.crt").exists() || !dir.join("server.key").exists() || openssl(root, &["x509", "-in", "server.crt", "-noout", "-checkend", "2592000"]).is_err();
    if stale {
        openssl(root, &["ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", "server.key"])?;
        openssl(root, &["req", "-new", "-key", "server.key", "-subj", "/CN=localhost", "-config", "openssl.cnf", "-out", "server.csr"])?;
        fs::write(dir.join("server.ext"), format!("basicConstraints = CA:FALSE\nkeyUsage = critical, digitalSignature\nextendedKeyUsage = serverAuth\n{}\n", san_line(&names))).map_err(|e| e.to_string())?;
        openssl(root, &["x509", "-req", "-in", "server.csr", "-CA", "ca.crt", "-CAkey", "ca.key", "-CAcreateserial", "-out", "server.crt", "-days", "825", "-sha256", "-extfile", "server.ext"])?;
        openssl(root, &["verify", "-CAfile", "ca.crt", "server.crt"])?; // never leave a certificate Apache would serve but nobody could verify
        fs::write(dir.join("names.txt"), &wanted).map_err(|e| e.to_string())?;
    }
    Ok(status(root))
}

#[cfg(unix)]
fn run_ok(c: &mut Command) -> bool {
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Linux browsers read the user's NSS database; macOS uses the login keychain.
#[cfg(target_os = "linux")]
fn nssdb() -> String {
    format!("sql:{}/.pki/nssdb", std::env::var("HOME").unwrap_or_default())
}

#[cfg(unix)]
pub fn is_trusted(root: &Path) -> bool {
    let ca = ssl_dir(root).join("ca.crt");
    #[cfg(target_os = "macos")]
    return ca.exists() && run_ok(Command::new("security").args(["verify-cert", "-c"]).arg(&ca));
    #[cfg(not(target_os = "macos"))]
    return ca.exists() && run_ok(Command::new("certutil").args(["-d", &nssdb(), "-L", "-n", CA_NAME]));
}

#[cfg(windows)]
pub fn is_trusted(root: &Path) -> bool {
    let Some(fp) = fingerprint(root, "ca.crt") else { return false };
    let want = fp.replace(':', "").to_lowercase();
    Command::new("certutil").args(["-user", "-store", "Root", CA_NAME]).stdin(Stdio::null()).creation_flags(NO_WINDOW).output().ok().is_some_and(|o| {
        let t = String::from_utf8_lossy(&o.stdout).to_lowercase().replace(' ', "");
        o.status.success() && t.contains(&want)
    })
}

pub fn status(root: &Path) -> HttpsStatus {
    let dir = ssl_dir(root);
    let names = fs::read_to_string(dir.join("names.txt")).map(|t| t.lines().map(str::to_string).collect()).unwrap_or_default();
    let cert = dir.join("server.crt").exists();
    HttpsStatus {
        ca_exists: dir.join("ca.crt").exists(),
        cert_exists: cert,
        names,
        expires: cert.then(|| openssl(root, &["x509", "-in", "server.crt", "-noout", "-enddate"]).ok().and_then(|t| field(&t, "notAfter="))).flatten(),
        trusted: dir.join("ca.crt").exists() && is_trusted(root),
        ca_path: dir.join("ca.crt").to_string_lossy().into_owned(),
        folder: dir.to_string_lossy().into_owned(),
    }
}

/// Add the authority to THIS user's trusted roots. Windows asks for confirmation itself; Runqora only ever calls this
/// from a button the user pressed.
pub fn trust(root: &Path) -> Result<String, String> {
    let ca = ssl_dir(root).join("ca.crt");
    if !ca.exists() {
        return Err("there is no certificate yet; turn HTTPS on first".into());
    }
    #[cfg(target_os = "macos")]
    {
        let keychain = format!("{}/Library/Keychains/login.keychain-db", std::env::var("HOME").unwrap_or_default());
        // macOS asks for the user's password itself
        return if run_ok(Command::new("security").args(["add-trusted-cert", "-r", "trustRoot", "-k", &keychain]).arg(&ca)) && is_trusted(root) {
            Ok("This Mac now trusts your Runqora certificate. Restart your browser if it still warns.".into())
        } else {
            Err("macOS did not add the certificate (the confirmation was declined or blocked)".into())
        };
    }
    #[cfg(target_os = "linux")]
    {
        let db = nssdb();
        let _ = fs::create_dir_all(format!("{}/.pki/nssdb", std::env::var("HOME").unwrap_or_default()));
        let _ = run_ok(Command::new("certutil").args(["-d", &db, "-N", "--empty-password"])); // creates the database when there is none
        return if run_ok(Command::new("certutil").args(["-d", &db, "-A", "-t", "C,,", "-n", CA_NAME, "-i"]).arg(&ca)) && is_trusted(root) {
            Ok("Chrome, Chromium and Edge now trust your Runqora certificate. Firefox has its own list: import the file in its Settings, or set security.enterprise_roots.enabled. Restart the browser.".into())
        } else {
            Err(format!("Linux has no single place for this. Install the `certutil` tool (package libnss3-tools or nss-tools) and press the button again, or trust {} yourself: sudo cp it to /usr/local/share/ca-certificates/runqora.crt && sudo update-ca-certificates", ca.display()))
        };
    }
    #[cfg(windows)]
    windows_trust(root, &ca)
}

#[cfg(windows)]
fn windows_trust(root: &Path, ca: &Path) -> Result<String, String> {
    let o = Command::new("certutil").args(["-user", "-addstore", "Root"]).arg(ca).stdin(Stdio::null()).creation_flags(NO_WINDOW).output().map_err(|e| e.to_string())?;
    if o.status.success() && is_trusted(root) {
        Ok("This PC now trusts your Runqora certificate. Restart your browser if it still warns.".into())
    } else {
        Err("Windows did not add the certificate (the confirmation was declined or blocked)".into())
    }
}

pub fn untrust(root: &Path) -> Result<String, String> {
    if !is_trusted(root) {
        return Ok("The certificate is not trusted on this PC.".into());
    }
    #[cfg(unix)]
    {
        let ca = ssl_dir(root).join("ca.crt");
        #[cfg(target_os = "macos")]
        let removed = run_ok(Command::new("security").args(["remove-trusted-cert"]).arg(&ca));
        #[cfg(target_os = "linux")]
        let removed = {
            let _ = &ca;
            run_ok(Command::new("certutil").args(["-d", &nssdb(), "-D", "-n", CA_NAME]))
        };
        return if removed && !is_trusted(root) { Ok("This computer no longer trusts your Runqora certificate.".into()) } else { Err("the certificate could not be removed".into()) };
    }
    #[cfg(windows)]
    windows_untrust(root)
}

#[cfg(windows)]
fn windows_untrust(root: &Path) -> Result<String, String> {
    let fp = fingerprint(root, "ca.crt").unwrap_or_default().replace(':', "");
    let o = Command::new("certutil").args(["-user", "-delstore", "Root", &fp]).stdin(Stdio::null()).creation_flags(NO_WINDOW).output().map_err(|e| e.to_string())?;
    if o.status.success() && !is_trusted(root) { Ok("This PC no longer trusts your Runqora certificate.".into()) } else { Err("Windows did not remove the certificate".into()) }
}

/// Forget the authority and certificate; the next time HTTPS is used they are made again (and must be trusted again).
pub fn regenerate(root: &Path, extra: &[String]) -> Result<HttpsStatus, String> {
    let dir = ssl_dir(root);
    for f in ["ca.crt", "ca.key", "ca.srl", "server.crt", "server.key", "server.csr", "names.txt"] {
        let _ = fs::remove_file(dir.join(f));
    }
    ensure(root, extra)
}

/// The Apache lines for HTTPS. `doc` is the folder to serve. Empty unless the files exist.
pub fn apache_block(root: &Path, port: u16, doc: &str, sites: &[(String, String)]) -> String {
    let (r, d) = (fwd(root), fwd(&ssl_dir(root)));
    if !ssl_dir(root).join("server.crt").exists() || !ssl_dir(root).join("server.key").exists() {
        return String::new();
    }
    let mut s = format!("<IfModule !ssl_module>\n    LoadModule ssl_module modules/mod_ssl.so\n</IfModule>\n<IfModule !socache_shmcb_module>\n    LoadModule socache_shmcb_module modules/mod_socache_shmcb.so\n</IfModule>\nListen {port}\nSSLSessionCache \"shmcb:{r}/apache/logs/ssl_scache(512000)\"\n");
    // localhost first: it is the default for any name that matches no site
    for (host, folder) in std::iter::once(&("localhost".to_string(), doc.to_string())).chain(sites) {
        s += &format!("<VirtualHost *:{port}>\n    ServerName {host}\n    DocumentRoot \"{folder}\"\n    SSLEngine on\n    SSLCertificateFile \"{d}/server.crt\"\n    SSLCertificateKeyFile \"{d}/server.key\"\n</VirtualHost>\n");
    }
    s
}

#[cfg(test)]
mod tests;
