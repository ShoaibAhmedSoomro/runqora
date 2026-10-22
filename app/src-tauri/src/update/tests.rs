use super::*;
use crate::sha256_file;
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

// ---- the newer modules: parsers against samples shaped like the real publishers' answers ----

const ADOPTIUM: &str = r#"[{"release_name":"jdk-21.0.13+8","binary":{"package":{"link":"https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.13%2B8/OpenJDK21U-jre_x64_windows_hotspot_21.0.13_8.zip","checksum":"AABB"}}}]"#;

#[test]
fn java_updates_and_the_next_long_term_release() {
    let c = parse_adoptium(ADOPTIUM, "21.0.12.1").unwrap();
    assert_eq!((c.version.as_str(), c.kind, c.sha256.as_str(), c.strip_prefix.as_str()), ("21.0.13", "patch", "aabb", "*"));
    assert!(c.url.starts_with("https://github.com/adoptium/") && host_allowed(&c.url, HOSTS) && c.warnings.is_empty());
    assert!(parse_adoptium(ADOPTIUM, "21.0.13").is_none() && parse_adoptium("[]", "21").is_none() && parse_adoptium("nope", "21").is_none());
    let major = parse_adoptium(&ADOPTIUM.replace("jdk-21.0.13+8", "jdk-25+36"), "21.0.12.1").unwrap();
    assert_eq!((major.version.as_str(), major.kind), ("25", "major"));
    assert!(major.warnings[0].contains("new release line"));
    assert_eq!(parse_adoptium_lts(r#"{"available_lts_releases":[8,11,17,21,25],"available_releases":[8,26]}"#), Some(25));
}

#[test]
fn tomcat_versions_come_from_the_listing_and_the_published_hash() {
    let html = r#"<a href="v10.1.7/">v10.1.7/</a><a href="v10.1.61/">v10.1.61/</a><a href="v10.1.0-M1/">v10.1.0-M1/</a><a href="v10.1.9/">v10.1.9/</a><a href="../">../</a>"#;
    let v = parse_tomcat_versions(html);
    assert_eq!(v, ["10.1.7", "10.1.9", "10.1.61"], "numeric order, milestones left out");
    let sha = "a".repeat(128);
    let c = tomcat_candidate("10.1.62", &format!("{sha} *apache-tomcat-10.1.62-windows-x64.zip"), "10.1.61").unwrap();
    assert_eq!((c.kind, c.strip_prefix.as_str()), ("patch", "apache-tomcat-10.1.62/"));
    assert_eq!(c.sha256, format!("sha512:{sha}"));
    assert_eq!(c.url, "https://archive.apache.org/dist/tomcat/tomcat-10/v10.1.62/bin/apache-tomcat-10.1.62-windows-x64.zip");
    let next = tomcat_candidate("11.0.9", &sha, "10.1.61").unwrap();
    assert_eq!(next.kind, "major");
    assert!(next.warnings.iter().any(|w| w.contains("Jakarta")));
    assert!(tomcat_candidate("10.1.62", "no hash here", "10.1.61").is_none());
}

fn gh(tag: &str, asset: &str) -> String {
    format!(r#"{{"tag_name":"{tag}","html_url":"https://github.com/o/r/releases/tag/{tag}","assets":[{{"name":"{asset}","browser_download_url":"https://github.com/o/r/releases/download/{tag}/{asset}","digest":"sha256:ABC123"}},{{"name":"other.zip","browser_download_url":"x","digest":"sha256:00"}}]}}"#)
}

#[test]
fn github_releases_give_the_asset_and_its_digest() {
    let name = |v: &str| format!("sftpgo_v{v}_windows_portable.zip");
    let c = parse_github_release(&gh("v2.7.7", "sftpgo_v2.7.7_windows_portable.zip"), "2.7.6", "SFTPGo", &name, "*").unwrap();
    assert_eq!((c.version.as_str(), c.kind, c.sha256.as_str(), c.strip_prefix.as_str()), ("2.7.7", "patch", "abc123", "*"));
    assert!(c.url.ends_with("sftpgo_v2.7.7_windows_portable.zip") && c.notes_url.ends_with("/tag/v2.7.7"));
    assert_eq!(parse_github_release(&gh("v2.8.0", "sftpgo_v2.8.0_windows_portable.zip"), "2.7.6", "SFTPGo", &name, "*").unwrap().kind, "minor");
    assert!(parse_github_release(&gh("v2.7.6", "sftpgo_v2.7.6_windows_portable.zip"), "2.7.6", "SFTPGo", &name, "*").is_none(), "same version");
    assert!(parse_github_release(&gh("v2.7.7", "something-else.zip"), "2.7.6", "SFTPGo", &name, "*").is_none(), "the expected file is not attached");
}

#[test]
fn wordpress_offers_the_same_line_and_the_newest_release() {
    let json = r#"{"offers":[{"response":"upgrade","version":"7.1.3"},{"response":"autoupdate","version":"7.1.3"},{"response":"autoupdate","version":"7.0.7"},{"response":"autoupdate","version":"6.9.10"}]}"#;
    assert_eq!(parse_wordpress(json, "7.0.5"), ["7.0.7", "7.1.3"], "a newer fix of the same line, and the newest release");
    assert_eq!(parse_wordpress(json, "7.1.3"), Vec::<String>::new());
    assert_eq!(parse_wordpress(json, "6.9.1"), ["6.9.10", "7.1.3"]);
    let sha = "1".repeat(40);
    let c = wp_candidate("7.1.3", &format!("{sha}\n"), "7.0.5").unwrap();
    assert_eq!((c.kind, c.sha256.as_str(), c.strip_prefix.as_str()), ("minor", format!("sha1:{sha}").as_str(), "wordpress/"));
    assert!(c.warnings.iter().any(|w| w.contains("Sites you already made are not touched")));
    assert!(wp_candidate("7.1.3", "<html>", "7.0.5").is_none());
}

const NODE_INDEX: &str = r#"[{"version":"v26.12.1","lts":false,"files":["win-x64-zip"]},{"version":"v24.23.0","lts":"Krypton","files":["win-x64-zip"]},{"version":"v24.22.1","lts":"Krypton","files":["win-x64-zip"]},{"version":"v22.30.0","lts":"Jod","files":["win-x64-zip"]},{"version":"v22.31.0","lts":"Jod","files":["linux-x64"]}]"#;

#[test]
fn node_offers_its_own_major_and_the_newest_long_term_one() {
    assert_eq!(parse_node(NODE_INDEX, "24.22.1"), ["24.23.0"], "a non-LTS 26 is never offered");
    assert_eq!(parse_node(NODE_INDEX, "22.20.0"), ["22.30.0", "24.23.0"], "builds without a Windows zip are skipped");
    let sums = "ffff0000ffff0000ffff0000ffff0000ffff0000ffff0000ffff0000ffff0000  node-v24.23.0-win-x64.zip\n1111111111111111111111111111111111111111111111111111111111111111  node-v24.23.0-win-x86.zip";
    let c = node_candidate("24.23.0", sums, "24.22.1").unwrap();
    assert_eq!((c.kind, c.strip_prefix.as_str()), ("patch", "*"), "minors inside a Node major are plain updates");
    assert!(c.sha256.starts_with("ffff") && c.url == "https://nodejs.org/dist/v24.23.0/node-v24.23.0-win-x64.zip");
    assert_eq!(node_candidate("26.0.0", &sums.replace("24.23.0", "26.0.0"), "24.22.1").unwrap().kind, "major");
    assert!(node_candidate("24.99.0", sums, "24.22.1").is_none(), "no hash for that file");
}

#[test]
fn composer_and_the_single_file_download() {
    let json = r#"{"stable":[{"path":"/download/2.10.3/composer.phar","version":"2.10.3"}],"preview":[{"version":"2.11.0-RC1"}]}"#;
    assert_eq!(parse_composer(json, "2.10.1").as_deref(), Some("2.10.3"));
    assert_eq!(parse_composer(json, "2.10.3"), None);
    let c = composer_candidate("2.10.3", &format!("{}  composer.phar", "ab".repeat(32)), "2.10.1").unwrap();
    assert_eq!((c.kind, c.strip_prefix.as_str()), ("patch", ""));
    assert!(host_allowed(&c.url, HOSTS) && c.url.ends_with("/2.10.3/composer.phar"));
}

#[test]
fn every_download_spelling_is_checked_and_a_marker_names_the_version() {
    let d = tmp("rt-newmods");
    let f = d.join("x.bin");
    fs::write(&f, "hello").unwrap();
    let sha256 = sha256_file(&f).unwrap();
    check_hash(&sha256, &f).unwrap();
    check_hash(&format!("sha256:{}", sha256.to_uppercase()), &f).unwrap();
    check_hash(&format!("sha512:{}", crate::sha512_file(&f).unwrap()), &f).unwrap();
    check_hash(&format!("sha1:{}", crate::sha1_file(&f).unwrap()), &f).unwrap();
    assert!(check_hash(&format!("sha512:{}", "0".repeat(128)), &f).unwrap_err().contains("does not match"));
    assert!(check_hash("md5:00", &f).is_err());
    write_marker(&d, "2.10.3");
    assert_eq!(read_marker(&d).as_deref(), Some("2.10.3"));
}

#[test]
fn a_single_file_module_installs_updates_and_keeps_its_wrapper() {
    let root = tmp("rt-single");
    let dl = root.join("tmp");
    fs::create_dir_all(&dl).unwrap();
    // install 1.0.0 (a bare file, as Composer is)
    let v1 = dl.join("v1");
    fs::write(&v1, "phar-1").unwrap();
    let mut c = component("composer").unwrap();
    c.sha256 = sha256_file(&v1).unwrap();
    c.version = "1.0.0".into();
    crate::install_from_zip(&c, &v1, &root).unwrap();
    assert_eq!(fs::read_to_string(root.join("composer/composer.phar")).unwrap(), "phar-1");
    assert!(root.join("composer/composer.bat").exists(), "the command-line wrapper is made on install");
    assert_eq!(detect_version(&c, &root).as_deref(), Some("1.0.0"));
    // update to 1.0.1: file replaced, wrapper kept, marker and backup updated
    let v2 = dl.join("v2");
    fs::write(&v2, "phar-2").unwrap();
    let cand = Candidate { version: "1.0.1".into(), kind: "patch", line: "1.0".into(), url: String::new(), sha256: sha256_file(&v2).unwrap(), strip_prefix: String::new(), notes_url: String::new(), warnings: vec![] };
    apply_from_zip(&c, &root, &cand, &v2, &verify_install).unwrap();
    assert_eq!(fs::read_to_string(root.join("composer/composer.phar")).unwrap(), "phar-2");
    assert!(root.join("composer/composer.bat").exists());
    assert_eq!(detect_version(&c, &root).as_deref(), Some("1.0.1"));
    assert_eq!(latest_backup(&root, "composer").unwrap().1.version, "1.0.0");
    // a bad hash changes nothing
    let bad = Candidate { sha256: "00".repeat(32), version: "1.0.2".into(), ..cand.clone() };
    assert!(apply_from_zip(&c, &root, &bad, &v2, &verify_install).unwrap_err().contains("does not match"));
    assert_eq!(fs::read_to_string(root.join("composer/composer.phar")).unwrap(), "phar-2");
    // roll back
    rollback_with(&c, &root, &verify_install).unwrap();
    assert_eq!(fs::read_to_string(root.join("composer/composer.phar")).unwrap(), "phar-1");
    assert_eq!(detect_version(&c, &root).as_deref(), Some("1.0.0"));
}

#[test]
fn the_data_of_mail_and_ftp_servers_survives_an_update() {
    let root = tmp("rt-mailupd");
    let live = root.join("mailpit");
    fs::create_dir_all(&live).unwrap();
    fs::write(live.join("mailpit.exe"), "old").unwrap();
    fs::write(live.join("mailpit.db"), "my 1000 emails").unwrap();
    let zp = root.join("new.zip");
    let sha = z(&zp, &[("mailpit.exe", "new")]);
    let mut c = component("mailpit").unwrap();
    c.version = "1.0.0".into();
    let cand = Candidate { version: "1.0.1".into(), kind: "patch", line: "1.0".into(), url: String::new(), sha256: sha, strip_prefix: String::new(), notes_url: String::new(), warnings: vec![] };
    apply_from_zip(&c, &root, &cand, &zp, &ok).unwrap();
    assert_eq!(fs::read_to_string(live.join("mailpit.exe")).unwrap(), "new");
    assert_eq!(fs::read_to_string(live.join("mailpit.db")).unwrap(), "my 1000 emails");
    assert_eq!(read_marker(&live).as_deref(), Some("1.0.1"));
    let tomcat = keep_rules("tomcat", "patch");
    assert!(tomcat.iter().any(|(r, _)| *r == "webapps") && tomcat.iter().any(|(r, _)| *r == "conf"), "the user's web applications and settings are carried over");
    assert!(keep_rules("sftpgo", "patch").iter().any(|(r, _)| *r == "sftpgo.db"));
}

/// The real publishers, read live (needs the internet): every parser still understands what they send today.
#[test]
#[ignore]
fn live_publishers_still_parse() {
    for (id, pretend) in [("jre", "21.0.1"), ("tomcat", "10.1.1"), ("mailpit", "1.0.0"), ("sftpgo", "2.0.0"), ("wordpress", "6.0.0"), ("node", "20.0.0"), ("composer", "2.0.0"), ("adminer", "4.0.0")] {
        let c = probe(id, pretend).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(!c.is_empty(), "{id}: nothing offered for {pretend}");
        for x in &c {
            assert!(host_allowed(&x.url, HOSTS), "{id}: {}", x.url);
            let hex = x.sha256.split(':').next_back().unwrap();
            assert!([40, 64, 128].contains(&hex.len()), "{id}: bad hash {}", x.sha256);
        }
    }
}
