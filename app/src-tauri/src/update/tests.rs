use super::*;
use std::io::Write;

fn z(path: &Path, entries: &[(&str, &str)]) -> String {
    let mut w = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (n, body) in entries {
        w.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(body.as_bytes()).unwrap();
    }
    w.finish().unwrap();
    sha256_file(path).unwrap()
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(name);
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn ok(_: &Component, _: &Path, _: &str) -> Result<Vec<String>, String> {
    Ok(vec![])
}

#[test]
fn versions_lines_and_kinds() {
    assert_eq!(parse_ver("10.11.19-MariaDB for Win64"), [10, 11, 19]);
    assert!(cmp_ver("8.3.36", "8.3.9").is_gt() && cmp_ver("10.11.19", "10.4.32").is_gt());
    assert_eq!(line_of("8.3.35"), "8.3");
    assert_eq!((kind_of("8.3.35", "8.3.36"), kind_of("8.3.35", "8.4.2"), kind_of("10.11.19", "11.4.9")), ("patch", "minor", "major"));
    assert!(php_ok(">=7.2,<8.4", "8.3.36") && !php_ok(">=7.2,<8.4", "8.4.1") && php_ok(">=7.2", "8.5.0"));
    assert!(host_allowed("https://windows.php.net/x.zip", HOSTS) && !host_allowed("http://windows.php.net/x.zip", HOSTS) && !host_allowed("https://windows.php.net.evil.com/x", HOSTS));
    assert!(open_notes("https://evil.example/").is_err());
}

const PHP_JSON: &str = r#"{"8.2":{"version":"8.2.30","ts-vs16-x64":{"zip":{"path":"php-8.2.30-Win32-vs16-x64.zip","sha256":"11"}}},
  "8.3":{"version":"8.3.36","ts-vs16-x64":{"zip":{"path":"php-8.3.36-Win32-vs16-x64.zip","sha256":"0606810FF8FC232406AAAB677B760E06AF93A63A366D33FF738D4CE90501DFA5"}},"nts-vs16-x64":{"zip":{"path":"nts.zip","sha256":"22"}}},
  "8.4":{"version":"8.4.27","ts-vs17-x64":{"zip":{"path":"php-8.4.27-Win32-vs17-x64.zip","sha256":"abcd"}}},"source":{"path":"x"}}"#;

#[test]
fn php_candidates_cover_patch_and_minor_with_warnings() {
    let c = parse_php(PHP_JSON, "8.3.35", Some("<8.4"));
    assert_eq!(c.iter().map(|x| (x.version.as_str(), x.kind)).collect::<Vec<_>>(), [("8.3.36", "patch"), ("8.4.27", "minor")]);
    assert_eq!(c[0].url, "https://windows.php.net/downloads/releases/php-8.3.36-Win32-vs16-x64.zip");
    assert_eq!(c[0].sha256, "0606810ff8fc232406aaab677b760e06af93a63a366d33ff738d4ce90501dfa5");
    assert!(c[0].warnings.is_empty());
    assert!(c[1].warnings.iter().any(|w| w.contains("new release line")) && c[1].warnings.iter().any(|w| w.contains("phpMyAdmin")));
    assert!(parse_php(PHP_JSON, "8.4.27", None).is_empty());
}

#[test]
fn mariadb_lines_and_release() {
    let majors = r#"{"major_releases":[{"release_id":"13.0","release_status":"Stable","release_support_type":"Rolling"},{"release_id":"12.3","release_status":"Stable","release_support_type":"Long Term Support"},{"release_id":"11.4","release_status":"Stable","release_support_type":"Long Term Support"},{"release_id":"11.1","release_status":"Stable","release_support_type":"Short Term Support"},{"release_id":"10.11","release_status":"Stable","release_support_type":"Long Term Support"},{"release_id":"10.6","release_status":"Stable","release_support_type":"Long Term Support"},{"release_id":"13.1","release_status":"RC","release_support_type":"Rolling"}]}"#;
    let lines = parse_mariadb_lines(majors, "10.11");
    assert_eq!(lines.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(), ["10.11", "11.1", "11.4", "12.3", "13.0"]); // 10.6 is older, 13.1 is a release candidate
    let rel = r#"{"releases":{"10.11.19":{"files":[]},"10.11.20":{"files":[{"file_name":"mariadb-10.11.20-winx64.msi","checksum":{"sha256sum":"bad"}},{"file_name":"mariadb-10.11.20-winx64.zip","checksum":{"sha256sum":"ABCDEF"}}]}}}"#;
    let c = parse_mariadb_release(rel, "10.11", true, "10.11.19").unwrap();
    assert_eq!((c.version.as_str(), c.kind, c.sha256.as_str(), c.strip_prefix.as_str()), ("10.11.20", "patch", "abcdef", "mariadb-10.11.20-winx64/"));
    assert!(c.warnings.is_empty() && c.url.ends_with("mariadb-10.11.20-winx64.zip"));
    assert!(parse_mariadb_release(rel, "10.11", true, "10.11.20").is_none());
    let major = parse_mariadb_release(&rel.replace("10.11.", "13.0."), "13.0", false, "10.11.19").unwrap();
    assert_eq!(major.kind, "major");
    assert!(major.warnings.iter().any(|w| w.contains("databases")) && major.warnings.iter().any(|w| w.contains("short-term")));
}

#[test]
fn apache_lounge_page_and_checksum() {
    let html = r#"<a href="/download/VS17/binaries/httpd-2.4.66-251206-Win64-VS17.zip">x</a> <a href="/download/VS17/binaries/httpd-2.4.66-251206-Win64-VS17.zip.asc">sig</a>"#;
    let (v, f) = parse_lounge_page(html, 17).unwrap();
    assert_eq!((v.as_str(), f.as_str()), ("2.4.66", "httpd-2.4.66-251206-Win64-VS17.zip"));
    assert!(parse_lounge_page(html, 18).is_none());
    let txt = "\u{feff}Checksums created with GPGHash\n\nSHA1-Checksum for: a.zip:\nD7B557581787727CA222C6A5FF0E9E6FA6029893\n\nSHA256-Checksum for: a.zip:\n2CD1F349B6705E43E784E876A233EEB1A859FA6B8ABC693A91F87B35A368F7F9\n";
    assert_eq!(parse_lounge_sha(txt).unwrap(), "2cd1f349b6705e43e784e876a233eeb1a859fa6b8abc693a91f87b35a368f7f9");
    let c = lounge_candidate("2.4.69", "httpd-2.4.69-261002-Win64-VS18.zip", 18, "aa", "2.4.66", Some(17));
    assert_eq!(c.kind, "toolchain");
    assert!(c.warnings[0].contains("VS18") && c.url.contains("/VS18/"));
    assert_eq!(lounge_candidate("2.4.67", "f", 17, "aa", "2.4.66", Some(17)).kind, "patch");
}

#[test]
fn phpmyadmin_candidates_check_the_php_you_have() {
    let json = r#"{"version":"5.2.3","releases":[{"version":"5.2.3","php_versions":">=7.2,<8.4"},{"version":"4.9.11","php_versions":">=5.5,<8.0"}]}"#;
    assert!(parse_pma(json, "5.2.3", Some("8.3.36")).is_empty());
    let c = parse_pma(json, "5.2.2", Some("8.5.1"));
    assert_eq!(c.len(), 1);
    assert!(c[0].1.iter().any(|w| w.contains("needs PHP >=7.2,<8.4")));
    assert_eq!(pma_php_range(json, "5.2.1").as_deref(), Some(">=7.2,<8.4"));
}

/// A mariadb-shaped install with user data, to exercise the swap rules without real binaries.
fn fixture(name: &str, version_txt: &str) -> (PathBuf, Component) {
    let root = tmp(name);
    let c = component("mariadb").unwrap();
    let live = root.join("mariadb");
    fs::create_dir_all(live.join("bin")).unwrap();
    fs::create_dir_all(live.join("data/mydb")).unwrap();
    fs::write(live.join("bin/mysqld.exe"), "").unwrap();
    fs::write(live.join("version.txt"), version_txt).unwrap();
    fs::write(live.join("my.ini"), "user-edited my.ini").unwrap();
    fs::write(live.join("data/mydb/orders.ibd"), "ORDERS").unwrap();
    (root, c)
}

fn cand(version: &str, kind: &'static str, sha: String) -> Candidate {
    Candidate { version: version.into(), kind, line: line_of(version), url: String::new(), sha256: sha, strip_prefix: format!("mariadb-{version}-winx64/"), notes_url: String::new(), warnings: vec![] }
}

fn new_zip(root: &Path, v: &str) -> (PathBuf, String) {
    let p = root.join("new.zip");
    let top = format!("mariadb-{v}-winx64");
    let sha = z(&p, &[(&format!("{top}/bin/mysqld.exe"), "NEW"), (&format!("{top}/version.txt"), "new"), (&format!("{top}/my.ini"), "default my.ini"), (&format!("{top}/data/mysql/sys.ibd"), "fresh system tables")]);
    (p, sha)
}

#[test]
fn patch_update_keeps_config_and_data_then_rolls_back() {
    let (root, c) = fixture("rt-upd-patch", "old");
    let (zip, sha) = new_zip(&root, "10.11.20");
    let msg = apply_from_zip(&c, &root, &cand("10.11.20", "patch", sha), &zip, &ok).unwrap();
    assert!(msg.contains("updated to 10.11.20"), "{msg}");
    let live = root.join("mariadb");
    assert_eq!(fs::read_to_string(live.join("version.txt")).unwrap(), "new");
    assert_eq!(fs::read_to_string(live.join("my.ini")).unwrap(), "user-edited my.ini"); // not replaced by the default
    assert_eq!(fs::read_to_string(live.join("data/mydb/orders.ibd")).unwrap(), "ORDERS"); // moved, not copied
    assert!(!latest_backup(&root, "mariadb").unwrap().1.data_copied);
    // work done on the new version must survive a roll back of a patch update
    fs::write(live.join("data/mydb/new_order.ibd"), "NEW ORDER").unwrap();
    let msg = rollback_with(&c, &root, &ok).unwrap();
    assert!(msg.contains("rolled back"), "{msg}");
    assert_eq!(fs::read_to_string(live.join("version.txt")).unwrap(), "old");
    assert_eq!(fs::read_to_string(live.join("data/mydb/new_order.ibd")).unwrap(), "NEW ORDER");
    assert!(latest_backup(&root, "mariadb").is_none()); // consumed
    assert!(rollback_with(&c, &root, &ok).unwrap_err().contains("no previous version"));
}

#[test]
fn major_upgrade_copies_the_database_and_rollback_restores_it() {
    let (root, c) = fixture("rt-upd-major", "old");
    let (zip, sha) = new_zip(&root, "11.4.9");
    apply_from_zip(&c, &root, &cand("11.4.9", "major", sha), &zip, &ok).unwrap();
    let live = root.join("mariadb");
    assert!(latest_backup(&root, "mariadb").unwrap().1.data_copied);
    assert_eq!(fs::read_to_string(live.join("data/mydb/orders.ibd")).unwrap(), "ORDERS"); // carried into the new version
    fs::write(live.join("data/mydb/after_upgrade.ibd"), "x").unwrap();
    rollback_with(&c, &root, &ok).unwrap();
    assert_eq!(fs::read_to_string(live.join("version.txt")).unwrap(), "old");
    assert!(live.join("data/mydb/orders.ibd").exists());
    assert!(!live.join("data/mydb/after_upgrade.ibd").exists()); // the old files come back exactly as they were
}

#[test]
fn a_failed_check_restores_everything() {
    let (root, c) = fixture("rt-upd-fail", "old");
    let (zip, sha) = new_zip(&root, "10.11.20");
    let bad = |_: &Component, _: &Path, _: &str| -> Result<Vec<String>, String> { Err("does not start".into()) };
    let e = apply_from_zip(&c, &root, &cand("10.11.20", "patch", sha), &zip, &bad).unwrap_err();
    assert!(e.contains("did not pass its checks") && e.contains("restored"), "{e}");
    let live = root.join("mariadb");
    assert_eq!(fs::read_to_string(live.join("version.txt")).unwrap(), "old");
    assert_eq!(fs::read_to_string(live.join("data/mydb/orders.ibd")).unwrap(), "ORDERS");
    assert_eq!(fs::read_to_string(live.join("my.ini")).unwrap(), "user-edited my.ini");
    assert!(latest_backup(&root, "mariadb").is_none());
}

#[test]
fn a_failed_major_upgrade_leaves_the_pristine_database() {
    let (root, c) = fixture("rt-upd-fail-major", "old");
    let (zip, sha) = new_zip(&root, "11.4.9");
    // the upgrade "damages" the carried-over copy before the check fails
    let bad = |_: &Component, r: &Path, _: &str| -> Result<Vec<String>, String> {
        fs::write(r.join("mariadb/data/mydb/orders.ibd"), "DAMAGED").unwrap();
        Err("does not start".into())
    };
    apply_from_zip(&c, &root, &cand("11.4.9", "major", sha), &zip, &bad).unwrap_err();
    assert_eq!(fs::read_to_string(root.join("mariadb/data/mydb/orders.ibd")).unwrap(), "ORDERS");
}

#[test]
fn bad_checksum_and_running_servers_change_nothing() {
    let (root, c) = fixture("rt-upd-guard", "old");
    let (zip, _) = new_zip(&root, "10.11.20");
    let e = apply_from_zip(&c, &root, &cand("10.11.20", "patch", "00".repeat(32)), &zip, &ok).unwrap_err();
    assert!(e.contains("does not match the publisher's checksum"));
    assert_eq!(fs::read_to_string(root.join("mariadb/version.txt")).unwrap(), "old");
    // a live pid file (this process) makes the server count as running
    fs::write(root.join("mariadb/data/mysql.pid"), std::process::id().to_string()).unwrap();
    let (zip, sha) = new_zip(&root, "10.11.20");
    let e = apply_from_zip(&c, &root, &cand("10.11.20", "patch", sha), &zip, &ok).unwrap_err();
    assert_eq!(e, "Stop MariaDB first.");
    assert_eq!(fs::read_to_string(root.join("mariadb/version.txt")).unwrap(), "old");
}

#[test]
fn only_one_rollback_target_is_kept() {
    let (root, c) = fixture("rt-upd-prune", "v1");
    for v in ["10.11.20", "10.11.21", "10.11.22"] {
        let (zip, sha) = new_zip(&root, v);
        apply_from_zip(&c, &root, &cand(v, "patch", sha), &zip, &ok).unwrap();
        std::thread::sleep(Duration::from_millis(1100)); // backups are named by the second
    }
    assert_eq!(read_backups(&root, "mariadb").len(), 1, "older backups are deleted");
}
