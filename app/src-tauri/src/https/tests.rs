use super::*;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(name);
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn host_names_are_checked_and_tidied() {
    assert_eq!(parse_hostnames("MyApp.test, shop.local;  *.dev.test\nmyapp.test").unwrap(), ["myapp.test", "shop.local", "*.dev.test"]);
    assert_eq!(parse_hostnames("  ").unwrap(), Vec::<String>::new());
    assert_eq!(parse_hostnames("192.168.1.20").unwrap(), ["192.168.1.20"]);
    for bad in ["bad name!", "-start.test", "end-.test", "a..b", "x*.test", "a.*", "*", "*.", "über.test", "a/b", "<script>"] {
        assert!(parse_hostnames(bad).is_err(), "{bad:?} must be refused");
    }
}

#[test]
fn the_certificate_always_covers_the_local_names_and_ips() {
    let n = all_names(&["myapp.test".into(), "localhost".into(), "10.0.0.5".into()]);
    assert_eq!(n, ["localhost", "127.0.0.1", "::1", "myapp.test", "10.0.0.5"], "no duplicates, extras last");
    assert_eq!(san_line(&n), "subjectAltName = DNS:localhost, IP:127.0.0.1, IP:::1, DNS:myapp.test, IP:10.0.0.5");
}

#[test]
fn apache_gets_tls_only_when_the_files_exist() {
    let r = tmp("rt-https-conf");
    assert_eq!(apache_block(&r, 8443, "C:/site", &[]), "", "no certificate, no TLS lines (Apache would refuse to start)");
    fs::create_dir_all(r.join("ssl")).unwrap();
    fs::write(r.join("ssl/server.crt"), "").unwrap();
    fs::write(r.join("ssl/server.key"), "").unwrap();
    let c = apache_block(&r, 8443, "C:/site", &[("shop.localhost".into(), "D:/shop".into())]);
    assert!(c.contains("ServerName shop.localhost") && c.contains("DocumentRoot \"D:/shop\"") && c.find("ServerName localhost").unwrap() < c.find("ServerName shop").unwrap());
    for want in ["Listen 8443", "<VirtualHost *:8443>", "SSLEngine on", "DocumentRoot \"C:/site\"", "ssl/server.crt\"", "ssl/server.key\"", "<IfModule !ssl_module>", "mod_ssl.so"] {
        assert!(c.contains(want), "missing {want} in\n{c}");
    }
}

#[test]
fn https_settings_reach_the_generated_apache_config() {
    use crate::config::config_set;
    let r = tmp("rt-https-settings");
    for d in ["apache/conf", "apache/bin"] {
        fs::create_dir_all(r.join(d)).unwrap();
    }
    fs::write(r.join("apache/bin/httpd.exe"), "").unwrap();
    // a port that is another module's is refused, and names are checked before anything is stored
    assert!(config_set(&r, "apache", "https_port", "3306", false).unwrap_err().contains("already uses"));
    assert!(config_set(&r, "apache", "hostnames", "bad name!", false).is_err());
    config_set(&r, "apache", "hostnames", "MyApp.test shop.local", false).unwrap();
    assert_eq!(crate::settings_in(&r).custom.get("apache.hostnames").map(String::as_str), Some("myapp.test, shop.local"));
    // turning HTTPS on without Apache's openssl.exe fails cleanly and stays off
    let e = config_set(&r, "apache", "https", "on", false).unwrap_err();
    assert!(e.contains("openssl.exe is missing"), "{e}");
    assert_ne!(crate::settings_in(&r).custom.get("apache.https").map(String::as_str), Some("on"));
}

/// Real certificates with Apache's own openssl.exe. Needs an install: RT_OPENSSL_ROOT or %TEMP%\rt-smoke.
#[test]
#[ignore]
fn real_certificates_are_made_signed_and_verified() {
    let src = std::env::var("RT_OPENSSL_ROOT").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join("rt-smoke"));
    let r = tmp("rt-https-real");
    fs::create_dir_all(r.join("apache")).unwrap();
    for sub in ["bin", "modules"] {
        let from = src.join("apache").join(sub);
        if from.is_dir() && sub == "bin" {
            fs::create_dir_all(r.join("apache/bin")).unwrap();
            for f in ["openssl.exe", "libcrypto-3-x64.dll", "libssl-3-x64.dll"] {
                if from.join(f).exists() {
                    fs::copy(from.join(f), r.join("apache/bin").join(f)).unwrap();
                }
            }
        }
    }
    let s = ensure(&r, &["myapp.test".into()]).unwrap();
    assert!(s.ca_exists && s.cert_exists, "{s:?}");
    assert!(s.names.contains(&"localhost".to_string()) && s.names.contains(&"myapp.test".to_string()) && !s.names.iter().any(|n| n.starts_with("*.localhost")));
    let ca_text = openssl(&r, &["x509", "-in", "ca.crt", "-noout", "-subject", "-ext", "basicConstraints"]).unwrap();
    assert!(ca_text.contains("Runqora Local CA") && ca_text.contains("CA:TRUE"), "{ca_text}");
    let text = openssl(&r, &["x509", "-in", "server.crt", "-noout", "-text"]).unwrap();
    for want in ["DNS:localhost", "DNS:myapp.test", "IP Address:127.0.0.1", "TLS Web Server Authentication", "CA:FALSE"] {
        assert!(text.contains(want), "{want} missing in\n{text}");
    }
    assert!(openssl(&r, &["verify", "-CAfile", "ca.crt", "server.crt"]).unwrap().contains("OK"));
    // a second call with the same names changes nothing; new names make a new certificate under the SAME authority
    let before = fs::read(ssl_dir(&r).join("server.crt")).unwrap();
    ensure(&r, &["myapp.test".into()]).unwrap();
    assert_eq!(before, fs::read(ssl_dir(&r).join("server.crt")).unwrap(), "unchanged names, unchanged certificate");
    let ca_before = fs::read(ssl_dir(&r).join("ca.crt")).unwrap();
    ensure(&r, &["myapp.test".into(), "other.test".into()]).unwrap();
    assert_ne!(before, fs::read(ssl_dir(&r).join("server.crt")).unwrap());
    assert_eq!(ca_before, fs::read(ssl_dir(&r).join("ca.crt")).unwrap(), "trusting the authority once keeps working");
    assert!(openssl(&r, &["verify", "-CAfile", "ca.crt", "server.crt"]).unwrap().contains("OK"));
}
