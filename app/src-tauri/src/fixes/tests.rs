use super::*;
use std::{io::{BufRead, BufReader}, process::Stdio};

fn root(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(name);
    let _ = fs::remove_dir_all(&d);
    for dir in ["apache/conf", "apache/bin", "apache/logs", "mariadb/bin", "mariadb/data/mysql", "php/ext", "phpmyadmin"] {
        fs::create_dir_all(d.join(dir)).unwrap();
    }
    for f in ["apache/bin/httpd.exe", "mariadb/bin/mysqld.exe", "php/php.exe", "phpmyadmin/index.php"] {
        fs::write(d.join(f), "").unwrap();
    }
    d
}

/// A throw-away program that listens on a port, so the kill path can be exercised for real.
fn listener() -> (std::process::Child, u16) {
    let mut child = Command::new("powershell")
        .args(["-NoProfile", "-Command", "$l=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0); $l.Start(); [Console]::Out.WriteLine($l.LocalEndpoint.Port); [Console]::Out.Flush(); Start-Sleep 120"])
        .stdout(Stdio::piped())
        .stdin(Stdio::null())
        .creation_flags(0x0800_0000)
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    (child, line.trim().parse().expect("the listener prints its port"))
}

#[test]
fn a_program_holding_a_port_can_be_ended_but_only_while_it_holds_it() {
    let r = root("rt-fix-end");
    let (mut child, port) = listener();
    let pid = child.id();
    assert_eq!(win::listener_pid(port), Some(pid));
    let rem = remedy(pid, &r);
    assert_eq!((rem.kind, rem.pid), ("end", pid), "{rem:?}");
    let msg = end_process(&r, pid).unwrap();
    assert!(msg.contains("port is free"), "{msg}");
    assert_eq!(win::listener_pid(port), None);
    let _ = child.wait();
    // the same call a second time is refused: it is not listening any more (and pids get reused)
    assert!(end_process(&r, pid).is_err());
}

#[test]
fn a_program_that_holds_no_port_is_never_ended() {
    let r = root("rt-fix-noport");
    let mut idle = Command::new("cmd").args(["/C", "ping -n 30 127.0.0.1 >nul"]).stdout(Stdio::null()).creation_flags(0x0800_0000).spawn().unwrap();
    let e = end_process(&r, idle.id()).unwrap_err();
    assert!(e.contains("no longer holding a port"), "{e}");
    assert!(pid_alive(idle.id()));
    let _ = idle.kill();
}

#[test]
fn windows_itself_and_this_app_are_off_limits() {
    let r = root("rt-fix-protected");
    for pid in [4, std::process::id()] {
        let rem = remedy(pid, &r);
        assert_ne!(rem.kind, "end", "{rem:?}");
        assert!(end_process(&r, pid).is_err());
    }
    assert!(stop_service(&r, 4, "Spooler").is_err(), "a service that is not named for that process is refused");
}

#[test]
fn a_free_port_is_suggested_instead() {
    let r = root("rt-fix-suggest");
    let p = suggest_port(&r, "apache").unwrap();
    assert!(p != 80 && win::listener_pid(p).is_none());
    let q = suggest_port(&r, "mariadb").unwrap();
    assert!(q != 3306 && q != p && win::listener_pid(q).is_none());
    assert!(suggest_port(&r, "php").is_none(), "libraries have no port");
    // a favourite that is taken is skipped
    let l = std::net::TcpListener::bind("127.0.0.1:8080");
    let again = suggest_port(&r, "apache").unwrap();
    assert!(again != 8080 || l.is_err());
}

#[test]
fn a_moved_install_is_found_and_repaired() {
    let r = root("rt-fix-moved");
    // as it would look after copying an install from C:\Old\Runqora
    fs::write(r.join("apache/conf/httpd.conf"), "ServerRoot \"C:/Old/Runqora/apache\"\nListen 8123\nServerName localhost:8123\nInclude conf/extra/runqora.conf\nInclude conf/extra/runqora.conf\n").unwrap();
    fs::write(r.join("mariadb/my.ini"), "[mysqld]\nport=3306\nbasedir=\"C:/Old/Runqora/mariadb\"\ndatadir=\"C:/Old/Runqora/mariadb/data\"\n").unwrap();
    fs::write(r.join("php/php.ini"), "extension_dir = \"C:/Old/Runqora/php/ext\"\nextension=mysqli\nextension=php_mysqli.dll\nextension=curl\n").unwrap();
    fs::write(r.join("phpmyadmin/config.inc.php"), "$cfg['Servers'][$i]['port'] = '3999';\n").unwrap();
    let ids: Vec<String> = diagnose(&r).into_iter().map(|p| p.id).collect();
    for want in ["apache.root", "apache.port", "apache.dupinclude", "mariadb.root", "php.extdir", "php.dups", "pma.port"] {
        assert!(ids.iter().any(|i| i == want), "missing {want} in {ids:?}");
    }
    let results = fix_all(&r);
    assert!(results.iter().all(|x| x.is_ok()), "{results:?}");
    let left: Vec<String> = diagnose(&r).into_iter().filter(|p| p.fix.is_some()).map(|p| p.id).collect();
    assert!(left.is_empty(), "everything with a fix is fixed: {left:?}");
    // what the fixes wrote
    let conf = fs::read_to_string(r.join("apache/conf/httpd.conf")).unwrap();
    assert!(conf.contains(&fwd(&r.join("apache"))) && conf.matches("Include conf/extra/runqora.conf").count() == 1 && r.join("apache/conf/extra/runqora.conf").exists());
    assert_eq!(ini_get(&fs::read_to_string(r.join("php/php.ini")).unwrap(), None, "extension_dir").unwrap(), format!("{}/ext", fwd(&r.join("php"))));
    assert!(duplicate_extensions(&fs::read_to_string(r.join("php/php.ini")).unwrap()).is_empty());
    assert_eq!(ini_get(&fs::read_to_string(r.join("mariadb/my.ini")).unwrap(), Some("mysqld"), "datadir").unwrap(), format!("{}/data", fwd(&r.join("mariadb"))));
    assert!(fs::read_to_string(r.join("phpmyadmin/config.inc.php")).unwrap().contains("'3306'"));
    assert!(r.join("php/php.ini.runqora-backup").exists(), "the file as the user had it is kept");
    assert!(fix_all(&r).is_empty(), "a clean setup has nothing to fix");
}

#[test]
fn stale_process_files_and_a_missing_ini_are_cleaned_up() {
    let r = root("rt-fix-stale");
    fs::write(r.join("apache/logs/httpd.pid"), "999999").unwrap(); // no such process
    fs::write(r.join("php/php.ini-development"), ";extension=mysqli\n; extension_dir = \"ext\"\n").unwrap();
    let ids: Vec<String> = diagnose(&r).into_iter().map(|p| p.id).collect();
    assert!(ids.iter().any(|i| i == "stale.apache") && ids.iter().any(|i| i == "php.ini"), "{ids:?}");
    apply_fix(&r, "stale", "apache").unwrap();
    assert!(!r.join("apache/logs/httpd.pid").exists());
    apply_fix(&r, "php.ini", "php").unwrap();
    let ini = fs::read_to_string(r.join("php/php.ini")).unwrap();
    assert!(ini.contains("extension=mysqli") && ini.contains("php/ext"), "{ini}");
    assert!(apply_fix(&r, "make-coffee", "php").is_err());
}

#[test]
fn a_blocked_port_is_reported_with_who_holds_it() {
    let r = root("rt-fix-blocked");
    let (mut child, port) = listener();
    // point the stack at the port the stranger holds
    let mut s = settings_in(&r);
    s.ports.insert("apache".into(), port);
    crate::save_settings_in(&r, &s).unwrap();
    let blocked: Vec<Problem> = diagnose(&r).into_iter().filter(|p| p.id == "blocked.apache").collect();
    assert_eq!(blocked.len(), 1);
    assert_eq!((blocked[0].port, blocked[0].pid), (Some(port), Some(child.id())));
    assert!(blocked[0].fix.is_none(), "ending someone else's program is the user's decision, never automatic");
    let _ = child.kill();
}

#[test]
fn a_deleted_website_folder_is_found_and_recreated() {
    let r = root("rt-fix-docroot");
    fs::write(r.join("apache/conf/httpd.conf"), "Listen 80\n").unwrap();
    let site = r.join("gone").join("site");
    let mut s = settings_in(&r);
    s.custom.insert("apache.document_root".into(), site.to_string_lossy().into_owned());
    crate::save_settings_in(&r, &s).unwrap();
    let ids: Vec<String> = diagnose(&r).into_iter().map(|p| p.id).collect();
    assert!(ids.iter().any(|i| i == "apache.docroot"), "{ids:?}");
    apply_fix(&r, "apache.paths", "apache").unwrap();
    assert!(site.join("index.html").exists(), "the folder is back with a placeholder page");
    assert!(!diagnose(&r).iter().any(|p| p.id == "apache.docroot"));
}
