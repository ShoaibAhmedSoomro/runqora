// Design preview only: when the page is opened in a plain browser (no Tauri), fake a backend
// so every state can be looked at. Ignored inside the real app.
if (!window.__TAURI__) {
  const file = (rel, exists = true) => ({ label: rel.split("/").pop(), rel, exists });
  const base = (id, name, version, extra) => ({ id, name, version, status: "stopped", pid: null, port: 0, server: false, has_admin: false, autostart: false, can_service: false, service_installed: false, blocker: null, configs: [], logs: [], ...extra });
  const scenario = location.hash.includes("long") ? "mixed" : location.hash.slice(1) || "mixed";
  const long = location.hash.includes("long");
  const LONG_NAME = "Apache HTTP Server with an unreasonably long product name that keeps going and going (build 2.4.66-vs17-custom)";
  const mods = [
    base("apache", "Apache HTTP Server", "2.4.66", { status: "running", pid: 4242, port: 80, server: true, has_admin: true, can_service: true, configs: [file("apache/conf/httpd.conf"), file("apache/conf/extra/runqora.conf")], logs: [file("apache/logs/error_log"), file("apache/logs/access_log", false)] }),
    base("php", "PHP (thread safe)", "8.3.35", { status: "installed", configs: [file("php/php.ini")] }),
    base("mariadb", "MariaDB", "10.11.19", { status: scenario === "mixed" ? "blocked" : "stopped", port: 3306, server: true, has_admin: true, can_service: true, service_installed: true, blocker: scenario === "mixed" ? { pid: 1234, name: "mysqld.exe", path: "C:\\Program Files\\MySQL\\bin\\mysqld.exe", ours: false } : null, configs: [file("mariadb/my.ini")], logs: [file("mariadb/data/mysql_error.log")] }),
    base("phpmyadmin", "phpMyAdmin", "5.2.3", { status: "missing" }),
    base("jre", "Java runtime (Temurin 21)", "21.0.12.1", { status: "missing" }),
    base("tomcat", "Apache Tomcat", "10.1.61", { status: "missing", port: 8080, server: true, has_admin: true, configs: [file("tomcat/conf/server.xml")] }),
    base("mailpit", "Mailpit (mail catcher)", "1.31.6", { status: "stopped", port: 8025, server: true, has_admin: true }),
    base("sftpgo", "SFTPGo (FTP and SFTP)", "2.7.6", { status: "missing", port: 8082, server: true, has_admin: true }),
    base("wordpress", "WordPress", "7.1.3", { status: "installed" }),
    base("node", "Node.js (LTS)", "24.22.1", { status: "missing" }),
    base("composer", "Composer", "2.10.3", { status: "missing" }),
    base("adminer", "Adminer", "6.1.2", { status: "missing" }),
  ];
  let sites = location.hash.includes("nosites") ? [] : [{ name: "shop", host: "shop.localhost", folder: "C:\\Runqora\\htdocs\\shop", exists: true, url: "http://shop.localhost/", https_url: httpsOnAtLoad() ? "https://shop.localhost/" : null }, { name: "blog", host: "blog.localhost", folder: "D:\\Projects\\a-client-with-a-very-long-folder-name\\wordpress-site-root", exists: false, url: "http://blog.localhost/", https_url: null }];
  const externals = [{ id: "filezilla", name: "FileZilla Server", status: "stopped", pid: null, port: 21, path: "C:\\Program Files\\FileZilla Server\\filezilla-server.exe", has_gui: true, blocker: null }, { id: "mercury", name: "Mercury Mail", status: "missing", pid: null, port: 25, path: "", has_gui: false, blocker: null }];
  function httpsOnAtLoad() { return location.hash.includes("https"); }
  if (location.hash.includes("demo")) {
    // marketing screenshots: a healthy stack with a few extras installed
    const set = (id, o) => Object.assign(mods.find((m) => m.id === id), o);
    set("apache", { status: "running", pid: 4120, port: 80 });
    set("mariadb", { status: "running", pid: 5532, blocker: null, port: 3306 });
    set("phpmyadmin", { status: "installed" });
    set("jre", { status: "installed" });
    set("tomcat", { status: "stopped" });
    set("mailpit", { status: "running", pid: 6644 });
    set("sftpgo", { status: "stopped" });
    set("wordpress", { status: "installed" });
    set("php", { status: "installed" });
  }
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  let autoMode = "install";
  let httpsOn = location.hash.includes("https"), httpsTrusted = false;
  // ---- configuration forms (mirrors the core's catalogue closely enough to design against) ----
  const F = (module, key, label, help, kind, value, def, group, extra = {}) => ({ module, key, label, help, kind, choices: [], min: 0, max: 0, group, value, default: def, ...extra });
  const onOff = [["On", "On"], ["Off", "Off"]];
  const cfg = {
    general: [F("general", "theme", "Theme", "Light, dark, or follow the Windows setting as it changes.", "choice", "system", "system", "Appearance", { choices: [["system", "Match Windows"], ["light", "Light"], ["dark", "Dark"]] }), F("general", "accent_mode", "Accent colour", "Buttons, switches and highlights. Match Windows follows your personalisation colour live.", "choice", "system", "system", "Appearance", { choices: [["system", "Match Windows"], ["custom", "Choose a colour"]] }), F("general", "accent_color", "Your colour", "Used when the accent is set to a custom colour. Text on it is kept readable automatically.", "color", "#0067C0", "#0067C0", "Appearance"), F("general", "launch_at_login", "Start Runqora when I sign in", "Opens Runqora hidden in the tray when you sign in to Windows. Servers marked Start with Runqora then start by themselves.", "bool", "off", "off", "Startup"), F("general", "editor", "Editor", "The program that opens configuration files and logs.", "text", "notepad.exe", "notepad.exe", "General"), F("general", "auto_update", "Update automatically", "Checks every module and Runqora itself once a day. Patch updates install themselves while the servers are stopped; bigger upgrades and new Runqora versions always ask first.", "choice", "install", "install", "Updates", { choices: [["install", "Check everything and install patch updates"], ["notify", "Check everything and tell me"], ["off", "Do not check"]] })],
    apache: [F("apache", "port", "Port", "Where the web server listens. 80 is the default; use 8080 if something else has 80.", "number", "80", "80", "Server", { min: 1, max: 65535 }), F("apache", "document_root", "Website folder", "The folder Apache serves. Leave empty to use the htdocs folder inside the install folder.", "path", "", "", "Server"), F("apache", "allow_override", ".htaccess files", "Let sites change Apache settings with .htaccess files. Most frameworks and CMSs need this.", "bool", "on", "on", "Websites"), F("apache", "directory_listing", "Show folder contents", "List the files in a folder that has no index page.", "bool", "on", "on", "Websites"), F("apache", "autostart", "Start with Runqora", "Start Apache when Runqora opens.", "bool", "off", "off", "Startup")],
    php: [F("php", "memory_limit", "Memory limit", "The most memory one script may use, such as 256M. -1 means no limit.", "size", "512M", "128M", "Limits"), F("php", "upload_max_filesize", "Largest upload", "The biggest file a visitor can upload, such as 64M.", "size", "2M", "2M", "Limits"), F("php", "post_max_size", "Largest form post", "Keep this at least as large as the largest upload.", "size", "8M", "8M", "Limits"), F("php", "max_execution_time", "Script time limit", "Seconds before a script is stopped. 0 means no limit.", "number", "30", "30", "Limits", { min: 0, max: 3600 }), F("php", "display_errors", "Show errors on the page", "Handy while developing. Turn it off for anything public.", "choice", "On", "Off", "Errors", { choices: onOff }), F("php", "date.timezone", "Time zone", "For example Europe/Paris or Asia/Dubai. Dates in PHP use this.", "text", "Asia/Dubai", "UTC", "Locale"),
      ...[["curl", "Web requests from PHP", "on"], ["gd", "Image drawing and resizing", "off"], ["intl", "Internationalisation", "off"], ["mbstring", "Text in every language (phpMyAdmin needs it)", "on"], ["mysqli", "MariaDB / MySQL access (phpMyAdmin needs it)", "on"], ["openssl", "Encryption and secure connections", "on"], ["zip", "ZIP archives", "off"]].map(([n, h, v]) => F("php", `ext.${n}`, n, h, "bool", v, "off", "Extensions"))],
    mariadb: [F("mariadb", "port", "Port", "Where the database listens. 3306 is the default.", "number", "3306", "3306", "Server", { min: 1, max: 65535 }), F("mariadb", "bind_address", "Who can connect", "Keep this PC only unless you really need other computers. The root account has no password.", "choice", "127.0.0.1", "127.0.0.1", "Server", { choices: [["127.0.0.1", "This PC only"], ["0.0.0.0", "Other computers on the network too"]] }), F("mariadb", "max_connections", "Most connections", "How many clients can be connected at once.", "number", "151", "151", "Performance", { min: 10, max: 10000 }), F("mariadb", "innodb_buffer_pool_size", "Memory for data", "Bigger is faster for large databases, such as 512M.", "size", "128M", "128M", "Performance"), F("mariadb", "max_allowed_packet", "Largest query", "Raise this if imports fail with a packet size error.", "size", "16M", "16M", "Performance"), F("mariadb", "character_set_server", "Default character set", "utf8mb4 stores every language and emoji.", "choice", "utf8mb4", "utf8mb4", "Data", { choices: [["utf8mb4", "utf8mb4 (recommended)"], ["utf8mb3", "utf8mb3"], ["latin1", "latin1 (older sites)"]] }), F("mariadb", "autostart", "Start with Runqora", "Start MariaDB when Runqora opens.", "bool", "off", "off", "Startup")],
  };
  let problems = location.hash.includes("clean") ? [] : [
    { id: "php.dups", module: "php", severity: "warning", title: "PHP loads some extensions twice", detail: "mysqli appear more than once in php.ini, which prints a warning on every start.", fix: "php.paths", port: null, pid: null },
    { id: "pma.port", module: "phpmyadmin", severity: "error", title: "phpMyAdmin looks for the database on the wrong port", detail: "MariaDB uses 3306.", fix: "pma.port", port: null, pid: null },
    { id: "blocked.mariadb", module: "mariadb", severity: "error", title: "Port 3306 is used by mysqld.exe", detail: "MariaDB cannot start until the port is free.", fix: null, port: 3306, pid: 1234 },
  ];

  const cand = (version, kind, warnings = []) => ({ version, kind, line: version.split(".").slice(0, 2).join("."), url: "", sha256: "", strip_prefix: "", notes_url: `https://www.php.net/ChangeLog-8.php#${version}`, warnings });
  const updates = [
    { id: "wordpress", name: "WordPress", current: "7.1.3", error: null, blocked: null, backup: null, candidates: [{ version: "7.1.4", kind: "patch", line: "7.1", url: "", sha256: "", strip_prefix: "", notes_url: "https://wordpress.org/news/category/releases/", warnings: ["This updates the copy Runqora uses for new sites. Sites you already made are not touched: update each one from its own Dashboard > Updates, after a backup."] }] },
    { id: "tomcat", name: "Apache Tomcat", current: "10.1.61", error: null, blocked: "Stop Tomcat first.", backup: null, candidates: [{ version: "10.1.62", kind: "patch", line: "10.1", url: "", sha256: "", strip_prefix: "", notes_url: "https://tomcat.apache.org/whichversion.html", warnings: [] }, { version: "11.0.9", kind: "major", line: "11.0", url: "", sha256: "", strip_prefix: "", notes_url: "https://tomcat.apache.org/whichversion.html", warnings: ["Tomcat 11.0 is a new release line: behaviour and settings can change. Test your sites and projects before relying on it.", "Tomcat moves to a newer Jakarta EE here: web applications built for the old one may need changes."] }] },
    { id: "apache", name: "Apache HTTP Server", current: "2.4.66", error: null, blocked: "Stop Apache first.", backup: null, candidates: [cand("2.4.69", "toolchain", ["This build uses a different Visual Studio toolchain (VS18, yours is VS17). The PHP module may not load into it: check your sites after updating."])] },
    { id: "php", name: "PHP (thread safe)", current: "8.3.35", error: null, blocked: null, backup: { version: "8.3.34", made: 1, data_copied: false }, candidates: [cand("8.3.36", "patch"), cand("8.4.27", "minor", ["PHP 8.4 is a new release line: defaults, extensions and deprecations change. Test your sites before relying on it.", "The installed phpMyAdmin supports PHP >=7.2,<8.4; it may stop working with PHP 8.4."])] },
    { id: "mariadb", name: "MariaDB", current: "10.11.19", error: null, blocked: null, backup: null, candidates: [cand("11.4.13", "major", ["This upgrades the database files in place. A full copy of your databases is kept for roll back, but anything you save after the update is lost if you roll back. Export what matters first."]), cand("13.0.2", "major", ["This upgrades the database files in place. A full copy of your databases is kept for roll back, but anything you save after the update is lost if you roll back. Export what matters first.", "MariaDB 13.0 is a short-term release with a shorter support period than a long-term one."])] },
    { id: "phpmyadmin", name: "phpMyAdmin", current: "5.2.3", error: location.hash.includes("offline") ? "https://www.phpmyadmin.net/home_page/version.json: connection failed" : null, blocked: null, backup: null, candidates: [] },
  ];
  if (long) { mods[0].name = LONG_NAME; if (mods[2].blocker) mods[2].blocker.path = "C:\Program Files\Some Vendor With A Long Name\Database Server Enterprise Edition 2026\bin\mysqld-with-a-long-file-name.exe"; mods[0].configs.push(file("apache/conf/extra/a-very-long-configuration-file-name-that-keeps-going.conf")); }
  if (location.hash.includes("demo")) {
    problems = [];
    updates.forEach((u) => { u.candidates = []; u.blocked = null; u.backup = null; });
    sites = ["shop", "blog", "portfolio"].map((n) => ({ name: n, host: `${n}.localhost`, folder: `C:\\Sites\\${n}`, exists: true, url: `http://${n}.localhost/`, https_url: `https://${n}.localhost/` }));
  }
  // progress: a download then the install stages, spread over a few seconds, for the installing/updating states
  const running = {};
  const STAGES = [[0.55, "Downloading"], [0.7, "Checking the download"], [0.85, "Unpacking"], [0.95, "Installing the new version"], [1, "Checking the new version"]];
  async function withProgress(key, ms) {
    running[key] = [Date.now(), ms];
    await wait(ms);
    delete running[key];
  }
  window.mockInvoke = async (cmd, a = {}) => {
    const m = mods.find((x) => x.id === a.id);
    if (cmd === "progress") {
      const out = {};
      for (const [k, [t0, ms]] of Object.entries(running)) {
        const f = Math.min(1, (Date.now() - t0) / ms);
        const [, stage] = STAGES.find(([to]) => f <= to) || STAGES.at(-1);
        out[k] = stage === "Downloading" ? { stage, done: Math.round((f / 0.55) * 36446352), total: 36446352 } : stage === "Unpacking" ? { stage, done: Math.round(((f - 0.7) / 0.15) * 1900), total: 1900 } : { stage, done: 0, total: 0 };
      }
      return out;
    }
    if (["component_install", "update_apply", "app_update_install"].includes(cmd)) await withProgress(a.id || "app", 4500);
    await wait(cmd === "modules_status" || cmd === "progress" ? 0 : 900);
    switch (cmd) {
      case "modules_status": return JSON.parse(JSON.stringify(mods));
      case "module_start": m.status = "running"; m.pid = 5000; return `${m.name} started on port ${m.port}`;
      case "module_stop": m.status = "stopped"; m.pid = null; return `${m.name} stopped`;
      case "component_install": m.status = m.server ? "stopped" : "installed"; return `${m.name} ${m.version} installed. Restart Apache to apply changes.`;
      case "module_service": m.service_installed = a.install; return a.install ? `${m.name} installed as a Windows service` : "Windows service removed";
      case "module_open_file": case "module_admin": case "open_root": case "modules_all": return null;

      case "https_status": return { ca_exists: httpsOn, cert_exists: httpsOn, names: ["localhost", "127.0.0.1", "::1"], expires: "2029-01-17T10:00:00Z", trusted: httpsTrusted, ca_path: "C:\\Runqora\\ssl\\ca.crt", folder: "C:\\Runqora\\ssl" };
      case "https_trust": httpsTrusted = true; return "This PC now trusts your Runqora certificate. Restart your browser if it still warns.";
      case "https_untrust": httpsTrusted = false; return "This PC no longer trusts your Runqora certificate.";
      case "https_regenerate": httpsTrusted = false; return {};
      case "open_url": case "open_developer": case "ui_ready": return null;
      case "app_update_check": return location.hash.includes("uptodate") || location.hash.includes("demo") ? null : { current: "0.1.0", latest: "0.2.0", notes_url: "https://github.com/ShoaibAhmedSoomro/runqora/releases/tag/v0.2.0", published: "2026-11-01T10:00:00Z", body: "Update checks for every module.\nNew: Node.js, Composer and Adminer." };
      case "app_update_install": return "The Runqora 0.2.0 installer is open. Runqora closes now; finish the installer and it opens again.";
      case "sites_list": return JSON.parse(JSON.stringify(sites));
      case "site_add": { if (!/^[a-z0-9-]{1,63}$/i.test(a.name.trim())) throw `'${a.name}' is not a site name (use letters, digits and dashes, like shop or my-blog)`; const n = a.name.trim().toLowerCase(); if (sites.some((s) => s.name === n)) throw `there is already a site called ${n}`; sites.push({ name: n, host: `${n}.localhost`, folder: a.folder || `C:\\Runqora\\htdocs\\${n}`, exists: true, url: `http://${n}.localhost/`, https_url: null }); return `Site ${n} is ready at http://${n}.localhost/ (restart Apache to apply)`; }
      case "site_remove": sites = sites.filter((s) => s.name !== a.name); return `Site ${a.name} removed from Runqora; its files are still there (restart Apache to apply)`;
      case "site_open_folder": return null;
      case "wordpress_create": { const n = a.name.trim().toLowerCase(); sites.push({ name: n, host: `${n}.localhost`, folder: `C:\\Runqora\\htdocs\\${n}`, exists: true, url: `http://${n}.localhost/`, https_url: null }); return `WordPress site ${n} created with database wp_${n}. Finish setup in the browser.`; }
      case "externals_status": return JSON.parse(JSON.stringify(externals));
      case "external_start": { const x = externals.find((e) => e.id === a.id); x.status = "running"; x.pid = 777; return `${x.name} started`; }
      case "external_stop": { const x = externals.find((e) => e.id === a.id); x.status = "stopped"; x.pid = null; return `${x.name} stopped`; }
      case "external_open": case "external_get": return null;
      case "appearance": { const g = Object.fromEntries(cfg.general.map((f) => [f.key, f.value])); return { theme: g.theme, accent_mode: g.accent_mode, accent_color: g.accent_color, system_accent: (location.hash.match(/accent=(#?[0-9a-fA-F]{6})/) || [])[1] ? "#" + location.hash.match(/accent=#?([0-9a-fA-F]{6})/)[1] : "#C239B3" }; }
      case "apply_theme": return null;
      case "config_get": return JSON.parse(JSON.stringify(cfg[a.module] || []));
      case "config_set": {
        const f = (cfg[a.module] || []).find((x) => x.key === a.key);
        if (!f) throw `there is no setting called ${a.module}.${a.key}`;
        if (f.kind === "number" && !(Number(a.value) >= f.min && Number(a.value) <= f.max)) throw `${f.label} must be between ${f.min} and ${f.max}`;
        if (f.kind === "size" && !/^-?\d+[KMGkmg]?$/.test(a.value)) throw `${f.label} should look like 64M, 512K or 1G`;
        if (f.key === "port" && mods.find((x) => x.id === a.module).status === "running") throw `stop ${mods.find((x) => x.id === a.module).name} first, then change its port`;
        f.value = f.kind === "size" ? a.value.toUpperCase() : a.value;
        if (f.key === "port") { const m = mods.find((x) => x.id === a.module); m.port = Number(a.value); if (m.status === "blocked") { m.status = "stopped"; m.blocker = null; } }
        if (f.key === "autostart") mods.find((x) => x.id === a.module).autostart = a.value === "on";
        const run = (a.module === "apache" || a.module === "php") && mods[0].status === "running" && f.key !== "port" && f.key !== "autostart";
        return { changed: true, message: `${f.label || f.key} set to ${f.value}.${run ? " Restart apache to apply it." : ""}`, restart: run ? "apache" : null };
      }
      case "module_restart": return `${mods.find((x) => x.id === a.id).name} restarted`;
      case "config_export": return "C:\\Runqora\\runqora.ini";
      case "config_ini_text": return "[php]\nmemory_limit = 1G\nmax_execution_time = banana\n[mariadb]\nmax_connections = 300\n[apache]\nallow_override = off\n[php]\nnot_a_setting = 1\n";
      case "config_import": {
        const out = []; let sec = "";
        for (const line of a.text.split("\n")) {
          const h = line.match(/^\[(.+)\]$/); if (h) { sec = h[1]; continue; }
          const m = line.match(/^(\w[\w.]*)\s*=\s*(.*)$/); if (!m) continue;
          const f = (cfg[sec] || []).find((x) => x.key === m[1]);
          if (!f) { out.push({ module: sec, key: m[1], value: m[2], status: "error", message: `there is no setting called ${sec}.${m[1]}` }); continue; }
          if (f.kind === "number" && isNaN(Number(m[2]))) { out.push({ module: sec, key: m[1], value: m[2], status: "error", message: `${f.label} must be a whole number` }); continue; }
          out.push({ module: sec, key: m[1], value: m[2], status: f.value === m[2] ? "unchanged" : "ok", message: "" });
          if (a.apply && f.value !== m[2]) f.value = m[2];
        }
        return out;
      }
      case "setup_check": return JSON.parse(JSON.stringify(problems));
      case "setup_fix": problems = problems.filter((p) => p.fix !== a.fix); return { php: "PHP's extension folder is right and each extension loads once", pma: "phpMyAdmin now looks for the database on port 3306" }[a.fix.split(".")[0]] || "Fixed";
      case "setup_fix_all": { const n = problems.filter((p) => p.fix).length; problems = problems.filter((p) => !p.fix); return Array.from({ length: n }, (_, i) => ({ Ok: `Fixed problem ${i + 1}` })); }
      case "port_remedy": return a.pid === 4 ? { pid: 4, name: "System", kind: "none", services: [], detail: "Windows itself is using this port. Choose another port for Runqora instead." } : a.pid === 1700 ? { pid: 1700, name: "svchost.exe", kind: "service", services: ["W3SVC"], detail: "The port is held by the Windows service W3SVC. Stopping the service frees it without ending a system process." } : { pid: a.pid, name: "mysqld.exe", kind: "end", services: [], detail: "mysqld.exe is using the port. Ending it frees the port, and anything unsaved in it is lost." };
      case "port_end": { const m = mods.find((x) => x.status === "blocked"); if (m) { m.status = "stopped"; m.blocker = null; } problems = problems.filter((p) => !p.port); return "mysqld.exe was ended and its port is free"; }
      case "port_stop_service": return `The ${a.name} service was stopped and its port is free`;
      case "port_suggest": return a.id === "apache" ? 8080 : 3307;
      case "app_info": return { version: "0.1.0", root: "C:\\runqora", cli: "C:\\Program Files\\Runqora\\rq.exe" };
      case "ports_scan": return [
        { port: 80, pid: 4242, name: "httpd.exe", path: "C:\\runqora\\apache\\bin\\httpd.exe", ours: true, module: "Apache HTTP Server" },
        { port: 135, pid: 1700, name: "svchost.exe", path: "", ours: false, module: null },
        { port: 445, pid: 4, name: "System", path: "", ours: false, module: null },
        { port: 3306, pid: 1234, name: "mysqld.exe", path: "C:\\Program Files\\MySQL\\bin\\mysqld.exe", ours: false, module: "MariaDB" },
      ];
      case "take_notes": return [];
      case "settings_get": return { editor: "", ports: {}, autostart: {}, auto_update: autoMode };
      case "settings_set_auto_update": autoMode = a.mode; return { off: "Runqora will not look for updates by itself", notify: "Runqora will tell you when updates are available", install: "Runqora will install patch updates when the servers are stopped" }[a.mode];
      case "fx_enabled": return location.hash.includes("fx");
      case "open_notes": return null;
      case "updates_check": return JSON.parse(JSON.stringify(updates));
      case "update_apply": { const u = updates.find((x) => x.id === a.id); const c = u.candidates.find((x) => x.version === a.version); await wait(1800); if (u.id === "mariadb" && c.kind !== "patch") { u.backup = { version: u.current, made: 1, data_copied: true }; } else { u.backup = { version: u.current, made: 1, data_copied: false }; } const old = u.current; u.current = c.version; u.candidates = u.candidates.filter((x) => x !== c && x.version > c.version); return `${u.name} updated to ${c.version} (was ${old}). The previous version is kept so you can roll back.`; }
      case "update_rollback": { const u = updates.find((x) => x.id === a.id); const v = u.backup.version; u.current = v; u.backup = null; return `${u.name} rolled back to ${v}.`; }
      case "settings_set_editor": return `Files will open in ${a.editor}`;
      case "settings_set_port": { const t = mods.find((x) => x.id === a.id); if (t.status === "running") throw `stop ${t.name} first, then change its port`; t.port = a.port; return `${t.name} will use port ${a.port}`; }
      case "settings_set_autostart": { const t = mods.find((x) => x.id === a.id); t.autostart = a.on; return `${t.name} ${a.on ? "will start" : "will not start"} when Runqora opens`; }
      default: throw `mock: unknown command ${cmd}`;
    }
  };
}
