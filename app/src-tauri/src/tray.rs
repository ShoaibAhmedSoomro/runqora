//! The tray icon and its menu, in the spirit of XAMPP's control panel: every installed server is a submenu with its
//! state (a coloured dot), Start/Stop, Open, Settings, and its config and log files. The menu is rebuilt only when
//! something changes, so it never closes under the user's cursor.

use runqora as rt;
use std::time::Duration;
use tauri::{
    image::Image,
    menu::{CheckMenuItem, IconMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIcon,
    AppHandle, Emitter, Manager, Wry,
};

/// What the menu needs to know about one server (ours, or FileZilla/Mercury found on this PC).
#[derive(Clone, PartialEq, Debug)]
pub struct Row {
    pub id: String,
    pub external: bool,
    pub name: String,
    /// running | stopped | blocked
    pub status: &'static str,
    pub port: u16,
    pub has_admin: bool,
    pub has_gui: bool,
    pub files: Vec<(String, String)>,
}

fn short(name: &str) -> String {
    name.split(" (").next().unwrap_or(name).replace("HTTP Server", "").replace("Apache Tomcat", "Tomcat").trim().to_string()
}

/// Every installed server, in the order the window lists them.
pub fn rows() -> Vec<Row> {
    let root = rt::root();
    let mut v: Vec<Row> = rt::all_states()
        .into_iter()
        .filter(|s| s.server && s.status != "missing")
        .map(|s| Row {
            files: s.configs.iter().chain(&s.logs).filter(|f| f.exists).map(|f| (f.label.clone(), f.rel.clone())).collect(),
            id: s.id,
            external: false,
            name: short(&s.name),
            status: if s.status == "running" { "running" } else if s.status == "blocked" { "blocked" } else { "stopped" },
            port: s.port,
            has_admin: s.has_admin,
            has_gui: false,
        })
        .collect();
    v.extend(rt::externals::all(&root).into_iter().filter(|x| x.status != "missing").map(|x| Row {
        id: x.id,
        external: true,
        name: short(&x.name),
        status: if x.status == "running" { "running" } else if x.status == "blocked" { "blocked" } else { "stopped" },
        port: x.port,
        has_admin: false,
        has_gui: true,
        files: vec![],
    }));
    v
}

/// A 16x16 antialiased dot.
fn dot((r, g, b): (u8, u8, u8)) -> Image<'static> {
    let mut px = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16 {
        for x in 0..16 {
            let d = (((x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2)).sqrt() - 5.5).clamp(-1.0, 1.0);
            px.extend([r, g, b, ((0.5 - d * 0.5) * 255.0) as u8]);
        }
    }
    Image::new_owned(px, 16, 16)
}

fn colour(status: &str) -> (u8, u8, u8) {
    match status {
        "running" => (46, 160, 67),
        "blocked" => (210, 153, 34),
        _ => (229, 72, 77),
    }
}

type Item = Box<dyn IsMenuItem<Wry>>;

fn submenu(app: &AppHandle, r: &Row) -> tauri::Result<Submenu<Wry>> {
    let key = format!("{}:{}", if r.external { "x" } else { "m" }, r.id);
    let state = match r.status {
        "running" => format!("Running · port {}", r.port),
        "blocked" => format!("Port {} is used by another program", r.port),
        _ => "Stopped".to_string(),
    };
    let mut items: Vec<Item> = vec![Box::new(IconMenuItem::with_id(app, format!("{key}:none"), state, false, Some(dot(colour(r.status))), None::<&str>)?)];
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    let label = if r.status == "running" { "Stop" } else { "Start" };
    items.push(Box::new(MenuItem::with_id(app, format!("{key}:toggle"), label, r.status != "blocked", None::<&str>)?));
    if r.has_admin {
        items.push(Box::new(MenuItem::with_id(app, format!("{key}:admin"), "Open in browser", r.status == "running", None::<&str>)?));
    }
    if r.has_gui {
        items.push(Box::new(MenuItem::with_id(app, format!("{key}:gui"), "Open its own window", true, None::<&str>)?));
    }
    if !r.external {
        items.push(Box::new(MenuItem::with_id(app, format!("{key}:settings"), "Settings...", true, None::<&str>)?));
    }
    if !r.files.is_empty() {
        items.push(Box::new(PredefinedMenuItem::separator(app)?));
        for (label, rel) in &r.files {
            items.push(Box::new(MenuItem::with_id(app, format!("{key}:file:{rel}"), label, true, None::<&str>)?));
        }
    }
    let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(|b| b.as_ref()).collect();
    let text = if r.status == "running" { format!("{}  ·  running", r.name) } else { r.name.clone() };
    Submenu::with_items(app, text, true, &refs)
}

pub fn build(app: &AppHandle, rows: &[Row], login: bool) -> tauri::Result<Menu<Wry>> {
    let mut items: Vec<Item> = vec![Box::new(MenuItem::with_id(app, "show", "Show / Hide", true, None::<&str>)?), Box::new(PredefinedMenuItem::separator(app)?)];
    if rows.is_empty() {
        items.push(Box::new(MenuItem::with_id(app, "none", "No servers installed yet", false, None::<&str>)?));
    }
    for r in rows {
        items.push(Box::new(submenu(app, r)?));
    }
    let any = rows.iter().any(|r| !r.external);
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(MenuItem::with_id(app, "start_all", "Start all", any, None::<&str>)?));
    items.push(Box::new(MenuItem::with_id(app, "stop_all", "Stop all", any, None::<&str>)?));
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(CheckMenuItem::with_id(app, "login", "Start with Windows", true, login, None::<&str>)?));
    items.push(Box::new(MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?));
    let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(|b| b.as_ref()).collect();
    Menu::with_items(app, &refs)
}

pub fn tooltip(rows: &[Row]) -> String {
    let up: Vec<&str> = rows.iter().filter(|r| r.status == "running").map(|r| r.name.as_str()).collect();
    if up.is_empty() { "Runqora: nothing running".into() } else { format!("Runqora: {} running", up.join(", ")) }
}

pub fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn note(ok: bool, text: String) {
    rt::push_note(if ok { "success" } else { "error" }, text);
}

/// A tray click on a server's submenu. Work happens off the UI thread; results appear as notices in the window.
/// Returns false when the id is not one of ours.
pub fn on_menu(app: &AppHandle, id: &str) -> bool {
    let mut p = id.splitn(4, ':');
    let (kind, who, action, arg) = (p.next().unwrap_or(""), p.next().unwrap_or(""), p.next().unwrap_or(""), p.next().unwrap_or(""));
    if kind != "m" && kind != "x" {
        return false;
    }
    let (app, kind, who, action, arg) = (app.clone(), kind.to_string(), who.to_string(), action.to_string(), arg.to_string());
    std::thread::spawn(move || {
        let root = rt::root();
        if kind == "x" {
            match action.as_str() {
                "toggle" => {
                    let running = rt::externals::all(&root).iter().any(|x| x.id == who && x.status == "running");
                    let r = if running { rt::externals::stop(&root, &who) } else { rt::externals::start(&root, &who) };
                    note(r.is_ok(), r.unwrap_or_else(|e| format!("{who}: {e}")));
                }
                "gui" => {
                    if let Err(e) = rt::externals::open_gui(&root, &who) {
                        note(false, e);
                    }
                }
                _ => {}
            }
            return;
        }
        let Ok(c) = rt::component(&who) else { return };
        match action.as_str() {
            "toggle" => {
                let running = rt::state(&c, &root).status == "running";
                let r = if running { rt::stop(&c, &root) } else { rt::start(&c, &root) };
                note(r.is_ok(), r.unwrap_or_else(|e| format!("{}: {e}", c.name)));
            }
            "admin" => match rt::admin_url(&c) {
                Some(url) => open_url(&url),
                None => note(false, format!("{} has no page to open", c.name)),
            },
            "settings" => {
                show_window(&app);
                let _ = app.emit("navigate", serde_json::json!({ "page": "settings", "tab": who }));
            }
            "file" => {
                if let Err(e) = rt::open_file(&c, &root, &arg) {
                    note(false, e);
                }
            }
            _ => {}
        }
    });
    true
}

fn open_url(url: &str) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).creation_flags(0x0800_0000).spawn();
}

/// Rebuild the menu and tooltip whenever a server starts, stops or appears. Polls the same state the window shows.
pub fn keep_fresh(app: AppHandle, tray: TrayIcon) {
    std::thread::spawn(move || {
        let mut last: Option<(Vec<Row>, bool)> = None;
        loop {
            let now = (rows(), rt::launch_at_login());
            if last.as_ref() != Some(&now) {
                let (a, t, (r, login)) = (app.clone(), tray.clone(), now.clone());
                // menus belong to the main thread; building them from here can stall every window call
                let _ = app.run_on_main_thread(move || {
                    if let Ok(m) = build(&a, &r, login) {
                        let _ = t.set_menu(Some(m));
                        let _ = t.set_tooltip(Some(tooltip(&r)));
                    }
                });
                last = Some(now);
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_short_and_the_tooltip_lists_what_runs() {
        assert_eq!(short("Apache HTTP Server"), "Apache");
        assert_eq!(short("Apache Tomcat"), "Tomcat");
        assert_eq!(short("Mailpit (mail catcher)"), "Mailpit");
        assert_eq!(short("SFTPGo (FTP and SFTP)"), "SFTPGo");
        let row = |name: &str, status| Row { id: name.into(), external: false, name: name.into(), status, port: 1, has_admin: false, has_gui: false, files: vec![] };
        assert_eq!(tooltip(&[row("Apache", "running"), row("MariaDB", "stopped"), row("Tomcat", "running")]), "Runqora: Apache, Tomcat running");
        assert_eq!(tooltip(&[row("Apache", "stopped")]), "Runqora: nothing running");
    }

    #[test]
    fn the_status_dot_is_a_round_coloured_image() {
        let d = dot(colour("running"));
        let px = d.rgba();
        assert_eq!((d.width(), d.height(), px.len()), (16, 16, 16 * 16 * 4));
        let at = |x: usize, y: usize| &px[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4];
        assert_eq!(at(8, 8)[3], 255, "solid in the middle");
        assert_eq!(at(0, 0)[3], 0, "clear in the corner");
        assert_eq!(&at(8, 8)[..3], &[46, 160, 67]);
        assert_ne!(colour("running"), colour("stopped"));
        assert_ne!(colour("blocked"), colour("stopped"));
    }
}
