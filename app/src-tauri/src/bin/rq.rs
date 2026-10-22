//! Command line for Runqora: `rq status|start|stop|ports`. Same core as the window.

use runqora as rt;
use std::process::ExitCode;

const HELP: &str = "Runqora command line

Usage:
  rq status               show every module
  rq start [module...]    start servers (all of them when none are named)
  rq stop  [module...]    stop servers (all of them, database last)
  rq install <module>     download and install a module (tomcat also brings the Java it needs)
  rq sites                list your sites (each is served at http://name.localhost/)
  rq sites add <name> [folder]   make a site; the folder is created if it does not exist
  rq sites remove <name>         forget a site (its files are never deleted)
  rq wordpress <name>     make a WordPress site with its own database (WordPress and MariaDB must be installed, MariaDB running)
  rq external [status|start|stop|open|get] [filezilla|mercury]
                               run FileZilla Server / Mercury Mail when they are installed on this PC
  rq ports                list listening TCP ports with their programs
  rq updates              look for newer versions of every installed module
  rq update <module> [version] [--yes]
                               install the newest patch update, or the version you name.
                               Minor and major upgrades show their warnings and need --yes.
  rq rollback <module>    go back to the version kept by the last update

  rq config get <module>            show a module's settings (general, apache, php, mariadb)
  rq config set <module> <key> <value>
                                    change one setting, written into the right file for you
  rq config reset <module> <key>    put one setting back to its default
  rq config export [file]           write all settings to a runqora.ini (default: in the install folder)
  rq config import <file> [--apply] check a runqora.ini and show what would change; --apply does it
  rq https [status|trust|untrust|regenerate]
                                    local HTTPS certificate: show it, trust it on this PC (Windows asks you), or make a new one
  rq doctor [--fix]                 look for broken paths, ports and configs; --fix repairs them
  rq free <port> [--yes]            end the program holding a port (asks first unless --yes)

Servers: apache, mariadb, tomcat, mailpit, sftpgo. php, phpmyadmin, jre and wordpress are libraries (status only).
Exit code: 0 ok, 1 an action failed, 2 bad usage.";

fn main() -> ExitCode {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        None | Some("status") => status(),
        Some("start") => run(true, &a[1..]),
        Some("stop") => run(false, &a[1..]),
        Some("install") => match a.get(1) {
            Some(id) => report(rt::component(id).and_then(|c| rt::install(&c, &rt::root()).map(|_| format!("{} {} installed", c.name, c.version)))),
            None => usage("install needs a module name"),
        },
        Some("sites") => sites(&a[1..]),
        Some("wordpress") => match a.get(1) {
            Some(name) => report(rt::sites::wordpress_create(&rt::root(), name)),
            None => usage("wordpress needs a site name"),
        },
        Some("external") => external(&a[1..]),
        Some("ports") => ports(),
        Some("updates") => updates(),
        Some("config") => config(&a[1..]),
        Some("https") => https(a.get(1).map(String::as_str).unwrap_or("status")),
        Some("doctor") => doctor(a.iter().any(|x| x == "--fix")),
        Some("free") => free(&a[1..]),
        Some("update") => update(&a[1..]),
        Some("rollback") => match a.get(1) {
            Some(id) => report(rt::update::rollback(id)),
            None => usage("rollback needs a module name"),
        },
        Some("help" | "-h" | "--help") => {
            println!("{HELP}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("unknown command '{other}'\n\n{HELP}");
            ExitCode::from(2)
        }
    }
}

fn status() -> ExitCode {
    println!("{:<11} {:<14} {:<8} PORT", "MODULE", "STATE", "PID");
    for s in rt::all_states() {
        let state = match s.status {
            "missing" => "not installed",
            "blocked" => "port in use",
            other => other,
        };
        let (pid, port) = (s.pid.map_or("-".into(), |p| p.to_string()), if s.server { s.port.to_string() } else { "-".into() });
        println!("{:<11} {:<14} {:<8} {}", s.id, state, pid, port);
        if let Some(b) = s.blocker {
            println!("            {} (PID {}) holds the port", b.name, b.pid);
        }
    }
    ExitCode::SUCCESS
}

fn run(start: bool, ids: &[String]) -> ExitCode {
    let all = rt::servers();
    let mut picked = Vec::new();
    for id in ids {
        match all.iter().find(|c| &c.id == id) {
            Some(c) => picked.push(c.clone()),
            None => {
                eprintln!("'{id}' is not a server (try: {})", all.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().join(", "));
                return ExitCode::from(2);
            }
        }
    }
    if ids.is_empty() {
        picked = all.into_iter().filter(|c| rt::root().join(&c.binary).exists()).collect(); // only what is installed
    }
    picked.sort_by_key(|c| c.id != "mariadb"); // database first
    if !start {
        picked.reverse(); // web server before its database
    }
    let root = rt::root();
    let mut failed = false;
    for c in picked {
        match if start { rt::start(&c, &root) } else { rt::stop(&c, &root) } {
            Ok(m) => println!("{m}"),
            Err(e) => {
                eprintln!("{}: {e}", c.name);
                failed = true;
            }
        }
    }
    if failed { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

fn ports() -> ExitCode {
    println!("{:<6} {:<7} {:<26} NOTE", "PORT", "PID", "PROGRAM");
    for r in rt::ports_scan() {
        let note = match (&r.module, r.ours) {
            (Some(m), true) => format!("{m} (this install)"),
            (Some(m), false) => format!("expected by {m}"),
            (None, true) => "this install".into(),
            (None, false) => String::new(),
        };
        println!("{:<6} {:<7} {:<26} {}", r.port, r.pid, r.name, note);
    }
    ExitCode::SUCCESS
}

fn usage(msg: &str) -> ExitCode {
    eprintln!("{msg}

{HELP}");
    ExitCode::from(2)
}

fn report(r: Result<String, String>) -> ExitCode {
    match r {
        Ok(m) => {
            println!("{m}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn updates() -> ExitCode {
    println!("Checking the publishers...");
    let infos = rt::update::check_updates();
    for i in &infos {
        println!("
{} (installed {})", i.name, i.current);
        if let Some(e) = &i.error {
            println!("  could not check: {e}");
        }
        if i.candidates.is_empty() && i.error.is_none() {
            println!("  up to date");
        }
        for c in &i.candidates {
            println!("  {:<10} {}  [{}]", c.version, c.kind, c.notes_url);
            for w in &c.warnings {
                println!("             ! {w}");
            }
        }
        if let Some(b) = &i.blocked {
            println!("  cannot update now: {b}");
        }
        if let Some(b) = &i.backup {
            println!("  roll back available: {}{}", b.version, if b.data_copied { " (with its databases)" } else { "" });
        }
    }
    ExitCode::SUCCESS
}

fn update(args: &[String]) -> ExitCode {
    let yes = args.iter().any(|a| a == "--yes");
    let pos: Vec<&String> = args.iter().filter(|a| *a != "--yes").collect();
    let Some(id) = pos.first() else { return usage("update needs a module name") };
    let infos = rt::update::check_updates();
    let Some(info) = infos.iter().find(|i| &i.id == *id) else {
        eprintln!("'{id}' is not installed or has no updates (modules: apache, php, mariadb, phpmyadmin)");
        return ExitCode::from(2);
    };
    let pick = match pos.get(1) {
        Some(v) => info.candidates.iter().find(|c| &c.version == *v),
        None => info.candidates.iter().filter(|c| c.kind == "patch").max_by(|a, b| rt::update::cmp_ver(&a.version, &b.version)),
    };
    let Some(c) = pick else {
        eprintln!("{}", if pos.len() > 1 { "that version is not on offer (run 'rq updates')" } else { "no patch update available (run 'rq updates' for newer release lines)" });
        return ExitCode::from(if pos.len() > 1 { 2 } else { 0 });
    };
    if c.kind != "patch" && !yes {
        eprintln!("{} {} is a {} upgrade:", info.name, c.version, c.kind);
        for w in &c.warnings {
            eprintln!("  ! {w}");
        }
        eprintln!("Run again with --yes to install it. You can roll back afterwards.");
        return ExitCode::from(2);
    }
    report(rt::update::apply_update(&info.id, &c.version))
}

fn config(args: &[String]) -> ExitCode {
    let root = rt::root();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["get", module] => match rt::config::config_get(&root, module) {
            Ok(f) if f.is_empty() => {
                println!("{module} is not installed");
                ExitCode::SUCCESS
            }
            Ok(fields) => {
                let mut group = String::new();
                for f in fields {
                    if f.group != group {
                        group = f.group.clone();
                        println!("
{group}");
                    }
                    println!("  {:<26} {}", f.key, if f.value.is_empty() { "(default)" } else { &f.value });
                }
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        },
        ["set", module, key, value] => match rt::config::config_set(&root, module, key, value, false) {
            Ok(a) => {
                println!("{}", a.message);
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        },
        ["reset", module, key] => {
            // back to the default (this is also the way to empty a setting: PowerShell drops an empty argument)
            let Ok(fields) = rt::config::config_get(&root, module) else { return fail(format!("{module} has no settings")) };
            match fields.iter().find(|f| f.key.eq_ignore_ascii_case(key)) {
                Some(f) => match rt::config::config_set(&root, module, &f.key, &f.default, false) {
                    Ok(a) => {
                        println!("{}", a.message);
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(e),
                },
                None => fail(format!("there is no setting called {module}.{key}")),
            }
        }
        ["export"] | ["export", _] => {
            let text = rt::config::export_ini(&root);
            match args.get(1) {
                Some(file) => match std::fs::write(file, text) {
                    Ok(()) => {
                        println!("Wrote {file}");
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(e.to_string()),
                },
                None => report(rt::config::export_to_file(&root).map(|p| format!("Wrote {}", p.display()))),
            }
        }
        ["import", file, ..] => {
            let apply = args.iter().any(|a| a == "--apply");
            let Ok(text) = std::fs::read_to_string(file) else { return fail(format!("cannot read {file}")) };
            let items = rt::config::import_ini(&root, &text, apply);
            let mut bad = false;
            for i in &items {
                let mark = match i.status {
                    "ok" => if apply { "changed " } else { "will change" },
                    "unchanged" => "same",
                    _ => {
                        bad = true;
                        "PROBLEM"
                    }
                };
                if i.status != "unchanged" {
                    println!("  {mark:<12} {}.{} = {}   {}", i.module, i.key, i.value, if i.status == "error" || i.status == "unknown" { &i.message } else { "" });
                }
            }
            println!("{} lines checked{}", items.len(), if apply { "; applied" } else { "; nothing written (add --apply)" });
            if bad { ExitCode::from(1) } else { ExitCode::SUCCESS }
        }
        _ => usage("config: get <module> | set <module> <key> <value> | reset <module> <key> | export [file] | import <file> [--apply]"),
    }
}

fn fail(msg: String) -> ExitCode {
    eprintln!("{msg}");
    ExitCode::from(1)
}

fn doctor(fix: bool) -> ExitCode {
    let root = rt::root();
    let problems = rt::fixes::diagnose(&root);
    if problems.is_empty() {
        println!("Everything looks right.");
        return ExitCode::SUCCESS;
    }
    for p in &problems {
        println!("{} {}
    {}{}", if p.severity == "error" { "[error]  " } else { "[warning]" }, p.title, p.detail, if p.fix.is_some() { "
    can be fixed automatically" } else if p.port.is_some() { "
    free the port with: rq free <port>, or choose another in Settings" } else { "" });
    }
    if !fix {
        println!("
Run 'rq doctor --fix' to repair what can be repaired.");
        return ExitCode::from(1);
    }
    let mut failed = false;
    for r in rt::fixes::fix_all(&root) {
        match r {
            Ok(m) => println!("fixed: {m}"),
            Err(e) => {
                eprintln!("could not fix: {e}");
                failed = true;
            }
        }
    }
    if failed { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

fn free(args: &[String]) -> ExitCode {
    let Some(port) = args.first().and_then(|p| p.parse::<u16>().ok()) else { return usage("free needs a port number") };
    let root = rt::root();
    let Some(pid) = rt::win::listener_pid(port) else {
        println!("Nothing is listening on port {port}.");
        return ExitCode::SUCCESS;
    };
    let r = rt::fixes::remedy(pid, &root);
    println!("Port {port} is held by {} (PID {pid}). {}", r.name, r.detail);
    match r.kind {
        "end" if args.iter().any(|a| a == "--yes") => report(rt::fixes::end_process(&root, pid)),
        "end" => {
            eprintln!("Run again with --yes to end it.");
            ExitCode::from(2)
        }
        "service" if args.iter().any(|a| a == "--yes") => report(rt::fixes::stop_service(&root, pid, &r.services[0])),
        "service" => {
            eprintln!("Run again with --yes to stop {}.", r.services.join(", "));
            ExitCode::from(2)
        }
        _ => ExitCode::from(1),
    }
}

fn https(what: &str) -> ExitCode {
    let root = rt::root();
    match what {
        "status" => {
            let s = rt::https::status(&root);
            if !s.cert_exists {
                println!("No certificate yet. Turn it on with: rq config set apache https on");
                return ExitCode::SUCCESS;
            }
            println!("Covers:  {}", s.names.join(", "));
            println!("Expires: {}", s.expires.unwrap_or_default());
            println!("Trusted on this PC: {}", if s.trusted { "yes" } else { "no (browsers will warn until you run: rq https trust)" });
            println!("Authority file: {}", s.ca_path);
            ExitCode::SUCCESS
        }
        "trust" => report(rt::https::trust(&root)),
        "untrust" => report(rt::https::untrust(&root)),
        "regenerate" => report(rt::https::regenerate(&root, &rt::https::configured_names(&root)).and_then(|_| rt::refresh_extra(&root)).map(|_| "Made a new certificate. Trust it again with: rq https trust".to_string())),
        _ => usage("https: status | trust | untrust | regenerate"),
    }
}

fn sites(args: &[String]) -> ExitCode {
    let root = rt::root();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["list"] => {
            let v = rt::sites::list(&root);
            if v.is_empty() {
                println!("No sites yet. Make one with: rq sites add <name> [folder]");
            }
            for s in v {
                println!("{:<14} {}{}  ->  {}", s.name, s.url, s.https_url.map(|u| format!("  {u}")).unwrap_or_default(), s.folder);
            }
            ExitCode::SUCCESS
        }
        ["add", name] => report(rt::sites::add(&root, name, "")),
        ["add", name, folder] => report(rt::sites::add(&root, name, folder)),
        ["remove", name] => report(rt::sites::remove(&root, name)),
        _ => usage("sites: list | add <name> [folder] | remove <name>"),
    }
}

fn external(args: &[String]) -> ExitCode {
    let root = rt::root();
    let (verb, id) = (args.first().map(String::as_str).unwrap_or("status"), args.get(1));
    if verb == "status" {
        for s in rt::externals::all(&root) {
            println!("{:<10} {:<14} port {:<4} {}", s.id, if s.status == "missing" { "not installed" } else { s.status }, s.port, s.path);
        }
        return ExitCode::SUCCESS;
    }
    let Some(id) = id else { return usage("external: status | start|stop|open|get <filezilla|mercury>") };
    match verb {
        "start" => report(rt::externals::start(&root, id)),
        "stop" => report(rt::externals::stop(&root, id)),
        "open" => report(rt::externals::open_gui(&root, id).map(|_| "Opened".to_string())),
        "get" => report(rt::externals::get_url(id).map(|u| format!("Download it from its publisher: {u}"))),
        _ => usage("external: status | start|stop|open|get <filezilla|mercury>"),
    }
}
