use super::*;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(name);
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn site_names_are_checked() {
    assert_eq!(check_name("  My-Shop ").unwrap(), "my-shop");
    for bad in ["", "-x", "x-", "a.b", "a b", "über", "a/b", "localhost", "phpmyadmin", &"x".repeat(64)] {
        assert!(check_name(bad).is_err(), "{bad:?} must be refused");
    }
}

#[test]
fn a_site_becomes_a_virtual_host_and_a_certificate_name() {
    let r = tmp("rt-sites");
    fs::create_dir_all(r.join("apache/conf")).unwrap();
    let folder = r.join("shop-files");
    let msg = add(&r, "Shop", &folder.to_string_lossy()).unwrap();
    assert!(msg.contains("http://shop.localhost/"), "{msg}");
    assert!(folder.join("index.html").exists(), "a new folder gets a first page");
    let conf = fs::read_to_string(r.join("apache/conf/extra/runqora.conf")).unwrap();
    assert!(conf.contains("ServerName shop.localhost") && conf.contains(&format!("DocumentRoot \"{}\"", crate::fwd(&folder))), "{conf}");
    assert!(conf.find("ServerName localhost").unwrap() < conf.find("ServerName shop.localhost").unwrap(), "localhost stays the default host");
    assert!(https::configured_names(&r).contains(&"shop.localhost".to_string()));
    assert_eq!(list(&r)[0].folder, folder.to_string_lossy());
    assert!(add(&r, "shop", "").unwrap_err().contains("already"));
    assert!(add(&r, "x", "relative/path").is_err() && add(&r, "y", "C:\\a\"b").is_err());
    // removing forgets the site, never the files
    remove(&r, "shop").unwrap();
    assert!(folder.join("index.html").exists() && list(&r).is_empty());
    assert!(!fs::read_to_string(r.join("apache/conf/extra/runqora.conf")).unwrap().contains("shop.localhost"));
    assert!(remove(&r, "shop").is_err());
}

#[test]
fn a_site_without_a_folder_gets_one_in_the_document_root() {
    let r = tmp("rt-sites-default");
    add(&r, "blog", "").unwrap();
    assert!(r.join("htdocs/blog/index.html").exists());
}

#[test]
fn wp_config_gets_the_database_and_fresh_keys() {
    let sample = "<?php\ndefine( 'DB_NAME', 'database_name_here' );\ndefine( 'DB_USER', 'username_here' );\ndefine( 'DB_PASSWORD', 'password_here' );\ndefine( 'DB_HOST', 'localhost' );\ndefine( 'AUTH_KEY',         'put your unique phrase here' );\ndefine( 'SECURE_AUTH_KEY',  'put your unique phrase here' );\n";
    let mut n = 0;
    let c = wp_config(sample, "wp_shop", 3307, || {
        n += 1;
        Ok(format!("salt{n}"))
    })
    .unwrap();
    for want in ["'DB_NAME', 'wp_shop'", "'DB_USER', 'root'", "'DB_PASSWORD', ''", "'DB_HOST', '127.0.0.1:3307'", "'AUTH_KEY',         'salt1' );", "'SECURE_AUTH_KEY',  'salt2' );"] {
        assert!(c.contains(want), "{want} missing in\n{c}");
    }
    assert!(!c.contains("put your unique phrase") && !c.contains("_here"));
    assert!(wp_config("<?php // nothing to fill", "d", 1, || Ok("x".into())).is_err());
    assert_eq!(db_name("my-blog"), "wp_my_blog");
}

#[test]
fn wordpress_needs_its_install_and_a_running_database() {
    let r = tmp("rt-wp-refuse");
    assert!(wordpress_create(&r, "blog").unwrap_err().contains("not installed"));
    fs::create_dir_all(r.join("wordpress")).unwrap();
    fs::write(r.join("wordpress/wp-load.php"), "").unwrap();
    assert!(wordpress_create(&r, "blog").unwrap_err().contains("start MariaDB"));
    assert!(!r.join("htdocs/blog").exists(), "a refused request leaves nothing behind");
}
