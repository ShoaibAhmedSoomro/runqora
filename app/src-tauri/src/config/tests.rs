use super::*;
use crate::{settings_in, EXTRA_CONF};

const PHP_INI: &str = "[PHP]\n; Default Value: 128M\nmemory_limit = 128M\n;upload_max_filesize = 2M\npost_max_size = 8M ; trailing note\n\n; For example:\n;\n;extension=mysqli\n\n[Date]\n;date.timezone =\n\n[Dynamic]\n;extension=bz2\nextension=openssl\n;extension=exif      ; Must be after mbstring\n;extension=mysqli\nextension=php_curl.dll\n";

const MY_INI: &str = "[client]\nport=3306\n\n[mysqld]\nport=3306\nbind-address=127.0.0.1\nmax_connections = 100\n\n[mariadb]\nmax_connections = 5\n";

fn root(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(name);
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("php/ext")).unwrap();
    fs::create_dir_all(d.join("apache/conf")).unwrap();
    fs::create_dir_all(d.join("apache/bin")).unwrap();
    fs::create_dir_all(d.join("mariadb/bin")).unwrap();
    fs::write(d.join("php/php.exe"), "").unwrap();
    fs::write(d.join("apache/bin/httpd.exe"), "").unwrap();
    fs::write(d.join("mariadb/bin/mysqld.exe"), "").unwrap();
    for e in ["mysqli", "mbstring", "curl", "openssl", "exif", "bz2"] {
        fs::write(d.join(format!("php/ext/php_{e}.dll")), "").unwrap();
    }
    fs::write(d.join("php/php.ini"), PHP_INI).unwrap();
    fs::write(d.join("mariadb/my.ini"), MY_INI).unwrap();
    d
}

#[test]
fn reading_ini_values_ignores_comments_and_respects_sections() {
    assert_eq!(ini_get(PHP_INI, None, "memory_limit").as_deref(), Some("128M"));
    assert_eq!(ini_get(PHP_INI, None, "post_max_size").as_deref(), Some("8M")); // trailing comment dropped
    assert_eq!(ini_get(PHP_INI, None, "upload_max_filesize"), None); // only commented out
    assert_eq!(ini_get(MY_INI, Some("mysqld"), "max_connections").as_deref(), Some("100")); // not the [mariadb] one
    assert_eq!(ini_get(MY_INI, Some("mysqld"), "bind_address").as_deref(), Some("127.0.0.1")); // - and _ are the same
    assert_eq!(ini_get(MY_INI, Some("client"), "max_connections"), None);
}

#[test]
fn writing_ini_values_replaces_uncomments_or_adds() {
    let a = ini_set(PHP_INI, None, "memory_limit", "512M");
    assert_eq!(ini_get(&a, None, "memory_limit").as_deref(), Some("512M"));
    assert_eq!(a.matches("memory_limit").count(), 1, "replaced in place, not duplicated");
    let b = ini_set(PHP_INI, None, "upload_max_filesize", "64M");
    assert!(b.contains("\nupload_max_filesize = 64M\n") && !b.contains(";upload_max_filesize"), "the commented example is replaced in place");
    let c = ini_set(PHP_INI, None, "max_execution_time", "120");
    assert!(c.ends_with("max_execution_time = 120\n"));
    // sections: inside the right one, after its last key; a missing section is created
    let m = ini_set(MY_INI, Some("mysqld"), "max_connections", "300");
    assert_eq!(ini_get(&m, Some("mysqld"), "max_connections").as_deref(), Some("300"));
    assert_eq!(ini_get(&m, Some("mariadb"), "max_connections").as_deref(), Some("5"), "the other section is untouched");
    let n = ini_set(MY_INI, Some("mysqld"), "innodb_buffer_pool_size", "512M");
    let (mysqld, mariadb) = (n.find("[mysqld]").unwrap(), n.find("[mariadb]").unwrap());
    assert!(n.find("innodb_buffer_pool_size").is_some_and(|i| i > mysqld && i < mariadb));
    assert!(ini_set("[client]\nport=1\n", Some("mysqld"), "port", "2").ends_with("\n[mysqld]\nport = 2\n"));
    // idempotent, and CRLF input is accepted
    assert_eq!(a, ini_set(&a, None, "memory_limit", "512M"));
    assert_eq!(ini_get(&ini_set("[PHP]\r\nmemory_limit = 1M\r\n", None, "memory_limit", "2M"), None, "memory_limit").as_deref(), Some("2M"));
}

#[test]
fn extensions_switch_on_and_off_without_duplicates() {
    assert_eq!(php_ext_active(PHP_INI), ["curl", "openssl"].into_iter().map(String::from).collect()); // php_curl.dll counts as curl
    let on = php_ext_set(PHP_INI, "mysqli", true);
    assert_eq!(on.lines().filter(|l| *l == "extension=mysqli").count(), 1);
    assert!(on.lines().any(|l| l == ";extension=mysqli"), "the prose example stays a comment");
    assert_eq!(on, php_ext_set(&on, "mysqli", true));
    let exif = php_ext_set(PHP_INI, "exif", true);
    assert!(exif.lines().any(|l| l == "extension=exif"), "found despite its trailing comment");
    let off = php_ext_set(&on, "openssl", false);
    assert!(!php_ext_active(&off).contains("openssl") && off.contains(";extension=openssl"));
    assert!(php_ext_set(PHP_INI, "mbstring", true).lines().any(|l| l == "extension=mbstring"), "an extension the file never mentions is added");
    let curl_off = php_ext_set(PHP_INI, "curl", false);
    assert!(!php_ext_active(&curl_off).contains("curl"));
}

#[test]
fn values_are_checked_and_spelled_the_same_way() {
    let d = |m, k| def(m, k, Path::new("nowhere")).unwrap();
    assert_eq!(normalize(&d("php", "memory_limit"), " 256m ").unwrap(), "256M");
    assert_eq!(normalize(&d("php", "memory_limit"), "-1").unwrap(), "-1");
    assert!(normalize(&d("php", "post_max_size"), "-1").is_err() && normalize(&d("php", "post_max_size"), "lots").is_err() && normalize(&d("php", "post_max_size"), "0").is_err());
    assert_eq!(normalize(&d("php", "max_execution_time"), "0").unwrap(), "0");
    assert!(normalize(&d("php", "max_execution_time"), "99999").unwrap_err().contains("between 0 and 3600"));
    assert_eq!(normalize(&d("apache", "allow_override"), "YES").unwrap(), "on");
    assert_eq!(normalize(&d("php", "display_errors"), "on").unwrap(), "On");
    assert!(normalize(&d("php", "display_errors"), "maybe").unwrap_err().contains("On, Off"));
    assert!(normalize(&d("php", "date.timezone"), "Europe/Paris").is_ok() && normalize(&d("php", "date.timezone"), "Paris\"; evil").is_err());
    assert_eq!(normalize(&d("apache", "document_root"), "C:/Sites/shop").unwrap(), "C:\\Sites\\shop");
    assert!(normalize(&d("apache", "document_root"), "sites").is_err() && normalize(&d("apache", "document_root"), "C:\\").is_err() && normalize(&d("apache", "document_root"), "C:\\a\"b").is_err());
    assert_eq!(normalize(&d("apache", "document_root"), "").unwrap(), "");
    assert!(def("php", "nonsense", Path::new("nowhere")).is_err());
}

#[test]
fn settings_write_back_to_the_real_files_and_keep_a_backup() {
    let r = root("rt-config-set");
    let a = config_set(&r, "php", "memory_limit", "512m", false).unwrap();
    assert!(a.changed && a.message.contains("512M") && a.restart.is_none());
    let ini = fs::read_to_string(r.join("php/php.ini")).unwrap();
    assert_eq!(ini_get(&ini, None, "memory_limit").as_deref(), Some("512M"));
    assert_eq!(fs::read_to_string(r.join("php/php.ini.runqora-backup")).unwrap(), PHP_INI, "the original is kept once");
    config_set(&r, "php", "post_max_size", "64M", false).unwrap();
    assert_eq!(fs::read_to_string(r.join("php/php.ini.runqora-backup")).unwrap(), PHP_INI, "and not overwritten by later edits");
    assert!(!config_set(&r, "php", "memory_limit", "512M", false).unwrap().changed, "same value: nothing written");
    config_set(&r, "php", "date.timezone", "Asia/Dubai", false).unwrap();
    assert!(fs::read_to_string(r.join("php/php.ini")).unwrap().contains("date.timezone = \"Asia/Dubai\""));
    config_set(&r, "php", "ext.mysqli", "on", false).unwrap();
    config_set(&r, "php", "ext.openssl", "off", false).unwrap();
    let active = php_ext_active(&fs::read_to_string(r.join("php/php.ini")).unwrap());
    assert!(active.contains("mysqli") && !active.contains("openssl"));
    assert!(config_set(&r, "php", "ext.imap", "on", false).unwrap_err().contains("no imap extension"));
    config_set(&r, "mariadb", "max_connections", "400", false).unwrap();
    assert_eq!(ini_get(&fs::read_to_string(r.join("mariadb/my.ini")).unwrap(), Some("mysqld"), "max_connections").as_deref(), Some("400"));
    let w = config_set(&r, "mariadb", "bind_address", "0.0.0.0", false).unwrap();
    assert!(w.message.contains("no password"), "exposing the database comes with a warning: {}", w.message);
    // invalid values change nothing
    assert!(config_set(&r, "mariadb", "max_connections", "5", false).is_err());
    assert_eq!(ini_get(&fs::read_to_string(r.join("mariadb/my.ini")).unwrap(), Some("mysqld"), "max_connections").as_deref(), Some("400"));
}

#[test]
fn apache_settings_regenerate_the_extra_config() {
    let r = root("rt-config-apache");
    let site = r.join("sites with space").to_string_lossy().into_owned();
    config_set(&r, "apache", "document_root", &site, false).unwrap();
    assert!(Path::new(&site).is_dir(), "the folder is created");
    config_set(&r, "apache", "allow_override", "off", false).unwrap();
    config_set(&r, "apache", "directory_listing", "off", false).unwrap();
    let conf = fs::read_to_string(r.join("apache").join(EXTRA_CONF)).unwrap();
    assert!(conf.contains("sites with space\"") && conf.contains("AllowOverride None") && conf.contains("Options FollowSymLinks") && !conf.contains("Indexes"), "{conf}");
    config_set(&r, "apache", "directory_listing", "on", false).unwrap();
    config_set(&r, "apache", "document_root", "", false).unwrap(); // empty goes back to the default
    let conf = fs::read_to_string(r.join("apache").join(EXTRA_CONF)).unwrap();
    assert!(conf.contains("Options Indexes FollowSymLinks") && conf.contains("/htdocs\"") && !conf.contains("sites with space"));
    assert!(settings_in(&r).custom.get("apache.document_root").is_none());
}

#[test]
fn ini_export_then_import_changes_nothing_and_a_bad_line_does_not_stop_the_rest() {
    let r = root("rt-config-ini");
    config_set(&r, "php", "memory_limit", "384M", false).unwrap();
    let text = export_ini(&r);
    assert!(text.contains("[php]") && text.contains("memory_limit = 384M") && text.contains("extensions = "), "{text}");
    let back = import_ini(&r, &text, false);
    assert!(!back.is_empty() && back.iter().all(|i| i.status == "unchanged"), "exporting then importing is a no-op: {back:#?}");
    // a hand-written file: two good lines, one bad value, one unknown key, one unknown section
    let mine = "[php]\nmemory_limit = 1G\nmax_execution_time = banana\nnot_a_setting = 1\nextensions = mbstring, imap\n[mariadb]\nmax_connections = 250\n[nginx]\nport = 1\n";
    let dry = import_ini(&r, mine, false);
    let st = |k: &str| dry.iter().find(|i| i.key == k).map(|i| i.status);
    assert_eq!((st("memory_limit"), st("max_execution_time"), st("not_a_setting"), st("ext.imap"), st("max_connections"), st("port")), (Some("ok"), Some("error"), Some("error"), Some("error"), Some("ok"), Some("error")));
    assert_eq!(ini_get(&fs::read_to_string(r.join("php/php.ini")).unwrap(), None, "memory_limit").as_deref(), Some("384M"), "a preview writes nothing");
    let done = import_ini(&r, mine, true);
    assert_eq!(done.iter().filter(|i| i.status == "ok").count(), dry.iter().filter(|i| i.status == "ok").count());
    assert_eq!(ini_get(&fs::read_to_string(r.join("php/php.ini")).unwrap(), None, "memory_limit").as_deref(), Some("1G"));
    assert_eq!(ini_get(&fs::read_to_string(r.join("mariadb/my.ini")).unwrap(), Some("mysqld"), "max_connections").as_deref(), Some("250"));
    let active = php_ext_active(&fs::read_to_string(r.join("php/php.ini")).unwrap());
    assert_eq!(active.iter().cloned().collect::<Vec<_>>(), ["mbstring"], "the list is exact: the others were switched off");
    assert_eq!(export_to_file(&r).unwrap(), r.join("runqora.ini"));
}

#[test]
fn modules_that_are_not_installed_offer_no_fields() {
    let r = std::env::temp_dir().join("rt-config-empty");
    let _ = fs::remove_dir_all(&r);
    fs::create_dir_all(&r).unwrap();
    assert!(config_get(&r, "php").unwrap().is_empty() && !config_get(&r, "general").unwrap().is_empty());
    assert!(config_get(&r, "nginx").is_err());
    assert!(config_set(&r, "php", "memory_limit", "1G", false).unwrap_err().contains("not installed"));
}

#[test]
fn theme_and_accent_are_settings_with_checked_values() {
    let r = root("rt-config-appearance");
    let a = crate::appearance_in(&r);
    assert_eq!((a.theme.as_str(), a.accent_mode.as_str(), a.accent_color.as_str()), ("system", "system", "#0067C0"), "defaults");
    assert!(config_set(&r, "general", "theme", "dark", false).unwrap().changed);
    assert!(config_set(&r, "general", "theme", "Purple", false).unwrap_err().contains("one of"));
    config_set(&r, "general", "accent_mode", "custom", false).unwrap();
    config_set(&r, "general", "accent_color", "#abc", false).unwrap(); // short form is expanded and upper-cased
    config_set(&r, "general", "accent_color", "e81123", false).unwrap(); // the leading # is optional
    for bad in ["red", "#12", "#gggggg", "#1234567", ""] {
        assert!(config_set(&r, "general", "accent_color", bad, false).is_err(), "{bad:?} must be refused");
    }
    let a = crate::appearance_in(&r);
    assert_eq!((a.theme.as_str(), a.accent_mode.as_str(), a.accent_color.as_str()), ("dark", "custom", "#E81123"));
    // they travel with the rest of the settings
    let ini = export_ini(&r);
    assert!(ini.contains("theme = dark") && ini.contains("accent_color = #E81123"), "{ini}");
    assert!(import_ini(&r, &ini, false).iter().all(|i| i.status == "unchanged"));
    // and reset to the default
    assert!(!config_set(&r, "general", "theme", "system", true).unwrap().message.is_empty());
}
