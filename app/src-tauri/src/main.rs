#![cfg_attr(not(test), windows_subsystem = "windows")]

//! The window. All logic lives in the library (shared with the `runtime` command line); this file
//! only exposes it to the UI and owns the tray.

mod tray;

use runqora as rt;
use serde::Serialize;
use std::{fs, os::windows::process::CommandExt, process::Command};
use tauri::Manager;

const NO_WINDOW: u32 = 0x0800_0000;

/// Run blocking core work off the UI thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn modules_status() -> Vec<rt::State> {
    rt::all_states()
}

#[tauri::command]
async fn module_start(id: String) -> Result<String, String> {
    blocking(move || rt::start(&rt::component(&id)?, &rt::root())).await
}

#[tauri::command]
async fn module_stop(id: String) -> Result<String, String> {
    blocking(move || rt::stop(&rt::component(&id)?, &rt::root())).await
}

#[tauri::command]
async fn component_install(id: String) -> Result<String, String> {
    blocking(move || {
        let c = rt::component(&id)?;
        rt::install(&c, &rt::root())?;
        Ok(format!("{} {} installed. Restart Apache to apply changes.", c.name, c.version))
    })
    .await
}

/// Open a module's page, starting Apache and MariaDB first when it needs them.
#[tauri::command]
async fn module_launch(id: String) -> Result<String, String> {
    blocking(move || {
        rt::launch(&id, &|url: &str| Command::new("cmd").args(["/C", "start", "", url]).creation_flags(NO_WINDOW).spawn().map(|_| ()).map_err(|e| e.to_string()))
    })
    .await
}

/// Show a module's folder in Explorer. Only the module's name comes from the UI.
#[tauri::command]
fn open_module_folder(id: String) -> Result<(), String> {
    let dir = rt::module_folder(&id)?;
    Command::new("explorer.exe").arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn module_admin(id: String) -> Result<(), String> {
    // the url is built in the core from the manifest and ports, never from UI input
    let url = rt::admin_url(&rt::component(&id)?).ok_or("this module has no admin page")?;
    Command::new("cmd").args(["/C", "start", "", &url]).creation_flags(NO_WINDOW).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn module_service(id: String, install: bool) -> Result<String, String> {
    blocking(move || {
        let c = rt::component(&id)?;
        if install { rt::service_install(&c, &rt::root()) } else { rt::service_remove(&c, &rt::root()) }
    })
    .await
}

#[tauri::command]
fn module_open_file(id: String, rel: String) -> Result<(), String> {
    rt::open_file(&rt::component(&id)?, &rt::root(), &rel)
}

// ---- configuration forms, fixes ----

#[tauri::command]
fn config_get(module: String) -> Result<Vec<rt::config::Field>, String> {
    rt::config::config_get(&rt::root(), &module)
}

#[tauri::command]
async fn config_set(module: String, key: String, value: String) -> Result<rt::config::Applied, String> {
    blocking(move || rt::config::config_set(&rt::root(), &module, &key, &value, false)).await
}

#[tauri::command]
async fn module_restart(id: String) -> Result<String, String> {
    blocking(move || rt::config::restart_module(&rt::root(), &id)).await
}

/// Writes runqora.ini into the install folder and returns its path.
#[tauri::command]
fn config_export() -> Result<String, String> {
    rt::config::export_to_file(&rt::root()).map(|p| p.to_string_lossy().into_owned())
}

/// `text` is the content of a runqora.ini; without `apply` it is only checked.
#[tauri::command]
fn config_import(text: String, apply: bool) -> Vec<rt::config::ImportItem> {
    rt::config::import_ini(&rt::root(), &text, apply)
}

/// The runqora.ini that sits in the install folder, if there is one.
#[tauri::command]
fn config_ini_text() -> Result<String, String> {
    fs::read_to_string(rt::config::ini_path(&rt::root())).map_err(|_| "there is no runqora.ini in the install folder yet; use Export first".to_string())
}

#[tauri::command]
async fn setup_check() -> Vec<rt::fixes::Problem> {
    blocking(|| Ok(rt::fixes::diagnose(&rt::root()))).await.unwrap_or_default()
}

#[tauri::command]
async fn setup_fix(fix: String, module: String) -> Result<String, String> {
    blocking(move || rt::fixes::apply_fix(&rt::root(), &fix, &module)).await
}

#[tauri::command]
async fn setup_fix_all() -> Vec<Result<String, String>> {
    blocking(|| Ok(rt::fixes::fix_all(&rt::root()))).await.unwrap_or_default()
}

#[tauri::command]
async fn port_remedy(pid: u32) -> rt::fixes::Remedy {
    blocking(move || Ok(rt::fixes::remedy(pid, &rt::root()))).await.unwrap_or_else(|e| rt::fixes::Remedy { pid, name: String::new(), kind: "none", services: vec![], detail: e })
}

#[tauri::command]
async fn port_end(pid: u32) -> Result<String, String> {
    blocking(move || rt::fixes::end_process(&rt::root(), pid)).await
}

#[tauri::command]
async fn port_stop_service(pid: u32, name: String) -> Result<String, String> {
    blocking(move || rt::fixes::stop_service(&rt::root(), pid, &name)).await
}

#[tauri::command]
fn port_suggest(id: String) -> Option<u16> {
    rt::fixes::suggest_port(&rt::root(), &id)
}

/// Open one of the user's own sites in the default browser.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !rt::is_local_url(&url, &rt::root()) {
        return Err("Runqora only opens addresses on this PC".into());
    }
    Command::new("cmd").args(["/C", "start", "", &url]).creation_flags(NO_WINDOW).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn https_status() -> rt::https::HttpsStatus {
    blocking(|| Ok(rt::https::status(&rt::root()))).await.unwrap_or_default()
}

/// Adds the Runqora certificate authority to THIS user's trusted roots; Windows shows its own confirmation.
#[tauri::command]
async fn https_trust() -> Result<String, String> {
    blocking(|| rt::https::trust(&rt::root())).await
}

#[tauri::command]
async fn https_untrust() -> Result<String, String> {
    blocking(|| rt::https::untrust(&rt::root())).await
}

#[tauri::command]
async fn https_regenerate() -> Result<rt::https::HttpsStatus, String> {
    blocking(|| {
        let root = rt::root();
        let s = rt::https::regenerate(&root, &rt::https::configured_names(&root))?;
        rt::refresh_extra(&root)?;
        Ok(s)
    })
    .await
}

// ---- sites, WordPress, programs found on this PC ----

#[tauri::command]
fn sites_list() -> Vec<rt::sites::Site> {
    rt::sites::list(&rt::root())
}

#[tauri::command]
async fn site_add(name: String, folder: String) -> Result<String, String> {
    blocking(move || rt::sites::add(&rt::root(), &name, &folder)).await
}

#[tauri::command]
async fn site_remove(name: String) -> Result<String, String> {
    blocking(move || rt::sites::remove(&rt::root(), &name)).await
}

/// Show a site's folder in Explorer. Only a site name comes from the UI; the folder is looked up in the settings.
#[tauri::command]
fn site_open_folder(name: String) -> Result<(), String> {
    let site = rt::sites::list(&rt::root()).into_iter().find(|s| s.name == name).ok_or("no such site")?;
    Command::new("explorer.exe").arg(site.folder).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn wordpress_create(name: String) -> Result<String, String> {
    blocking(move || rt::sites::wordpress_create(&rt::root(), &name)).await
}

#[tauri::command]
async fn externals_status() -> Vec<rt::externals::ExternalState> {
    blocking(|| Ok(rt::externals::all(&rt::root()))).await.unwrap_or_default()
}

#[tauri::command]
async fn external_start(id: String) -> Result<String, String> {
    blocking(move || rt::externals::start(&rt::root(), &id)).await
}

#[tauri::command]
async fn external_stop(id: String) -> Result<String, String> {
    blocking(move || rt::externals::stop(&rt::root(), &id)).await
}

#[tauri::command]
fn external_open(id: String) -> Result<(), String> {
    rt::externals::open_gui(&rt::root(), &id)
}

/// Open the publisher's own download page; the address comes from a fixed list, never from the UI.
#[tauri::command]
fn external_get(id: String) -> Result<(), String> {
    let url = rt::externals::get_url(&id)?;
    Command::new("cmd").args(["/C", "start", "", url]).creation_flags(NO_WINDOW).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// The developer's own pages. The UI sends only a key; the address comes from this list.
const DEVELOPER: &[(&str, &str)] = &[
    ("github", "https://github.com/ShoaibAhmedSoomro"),
    ("linkedin", "https://www.linkedin.com/in/shoaibaofficial/"),
    ("instagram", "https://www.instagram.com/shoaib_ahmedsoomro"),
    ("facebook", "https://www.facebook.com/shoaibahmedsoomroofficial/"),
];

#[tauri::command]
fn developer_links() -> Vec<(&'static str, &'static str)> {
    DEVELOPER.to_vec()
}

#[tauri::command]
fn open_developer(id: String) -> Result<(), String> {
    let url = DEVELOPER.iter().find(|(k, _)| *k == id).map(|(_, u)| *u).ok_or("unknown link")?;
    Command::new("cmd").args(["/C", "start", "", url]).creation_flags(NO_WINDOW).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// What every running download or update is doing right now (stage and bytes), for the progress bars.
#[tauri::command]
fn progress() -> std::collections::BTreeMap<String, rt::Progress> {
    rt::progress_all()
}

/// The page has loaded: show the window, unless Runqora was started hidden (at sign-in, in the tray).
#[tauri::command]
fn ui_ready(app: tauri::AppHandle) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static FIRST: AtomicBool = AtomicBool::new(true);
    // only the first load decides; later reloads must not pop a window the user closed to the tray
    if FIRST.swap(false, Ordering::Relaxed) && !std::env::args().any(|a| a == "--hidden") {
        tray::show_window(&app);
    }
}

/// Is there a newer Runqora release on GitHub?
#[tauri::command]
async fn app_update_check() -> Result<Option<rt::appupdate::AppUpdate>, String> {
    blocking(rt::appupdate::check).await
}

/// Download the newest installer, verify it, start it and close this window so it can replace the app.
#[tauri::command]
async fn app_update_install(app: tauri::AppHandle) -> Result<String, String> {
    let msg = blocking(rt::appupdate::install).await?;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        app.exit(0); // servers keep running: they are independent processes
    });
    Ok(msg)
}

/// Set once at startup: did the window get the Mica material? Only then may the UI turn translucent.
static FX: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[tauri::command]
fn fx_enabled() -> bool {
    FX.load(std::sync::atomic::Ordering::Relaxed)
}

#[tauri::command]
fn modules_all(start: bool) {
    rt::all_servers(start);
}

#[tauri::command]
async fn ports_scan() -> Vec<rt::PortRow> {
    blocking(|| Ok(rt::ports_scan())).await.unwrap_or_default()
}

#[tauri::command]
fn take_notes() -> Vec<rt::Note> {
    rt::take_notes()
}

#[tauri::command]
fn settings_get() -> rt::Settings {
    rt::settings()
}

#[tauri::command]
fn settings_set_editor(editor: String) -> Result<String, String> {
    rt::set_editor_in(&rt::root(), &editor)
}

#[tauri::command]
async fn settings_set_port(id: String, port: u32) -> Result<String, String> {
    blocking(move || rt::set_port_in(&rt::root(), &id, port)).await
}

#[tauri::command]
fn settings_set_autostart(id: String, on: bool) -> Result<String, String> {
    rt::set_autostart_in(&rt::root(), &id, on)
}

#[tauri::command]
async fn updates_check() -> Vec<rt::update::UpdateInfo> {
    blocking(|| Ok(rt::update::check_updates())).await.unwrap_or_default()
}

#[tauri::command]
async fn update_apply(id: String, version: String) -> Result<String, String> {
    blocking(move || rt::update::apply_update(&id, &version)).await
}

#[tauri::command]
async fn update_rollback(id: String) -> Result<String, String> {
    blocking(move || rt::update::rollback(&id)).await
}

#[tauri::command]
fn open_notes(url: String) -> Result<(), String> {
    rt::update::open_notes(&url)
}

#[tauri::command]
fn settings_set_auto_update(mode: String) -> Result<String, String> {
    rt::set_auto_update_in(&rt::root(), &mode)
}

/// Theme, accent choice, and what Windows' own accent is right now. The window polls this, so a change made in
/// Windows Settings (or with `rq config set general theme dark`) shows up without a restart.
#[tauri::command]
fn appearance() -> rt::Appearance {
    rt::appearance_in(&rt::root())
}

/// Give the window frame (title bar, Mica) the same theme as the page: dark, light, or None to follow Windows.
#[tauri::command]
fn apply_theme(app: tauri::AppHandle, theme: String) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_theme(match theme.as_str() {
            "light" => Some(tauri::Theme::Light),
            "dark" => Some(tauri::Theme::Dark),
            _ => None,
        });
    }
}

#[derive(Serialize)]
struct AppInfo {
    version: &'static str,
    root: String,
    cli: String,
}

#[tauri::command]
fn app_info() -> AppInfo {
    let cli = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("rq.exe"))).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    AppInfo { version: env!("CARGO_PKG_VERSION"), root: rt::root().to_string_lossy().into_owned(), cli }
}

#[tauri::command]
fn open_root() -> Result<(), String> {
    let r = rt::root();
    fs::create_dir_all(&r).map_err(|e| e.to_string())?;
    Command::new("explorer.exe").arg(r).spawn().map(|_| ()).map_err(|e| e.to_string())
}

fn toggle_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

/// One Runqora per install folder, so a second launch cannot add a second tray icon.
fn instance_name(what: &str) -> Vec<u16> {
    let key: String = rt::root().to_string_lossy().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    format!("Local\\Runqora_{what}_{key}\0").encode_utf16().collect()
}

/// True when another copy already runs here; it is then told to show its window (it may only be in the tray).
fn already_running() -> bool {
    use windows_sys::Win32::{
        Foundation::{GetLastError, ERROR_ALREADY_EXISTS},
        System::Threading::{CreateMutexW, OpenEventW, SetEvent},
    };
    // SAFETY: names are NUL-terminated; the mutex handle is kept for the life of the process on purpose
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, instance_name("lock").as_ptr());
        if h.is_null() || GetLastError() != ERROR_ALREADY_EXISTS {
            return false;
        }
        let ev = OpenEventW(0x0002, 0, instance_name("show").as_ptr()); // EVENT_MODIFY_STATE
        if !ev.is_null() {
            SetEvent(ev);
        }
        true
    }
}

/// The running copy waits for the "show" signal from a second launch and opens its window.
fn listen_for_second_launch(app: tauri::AppHandle) {
    use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    // SAFETY: the event handle lives as long as the process; the thread only waits on it
    let ev = unsafe { CreateEventW(std::ptr::null(), 0, 0, instance_name("show").as_ptr()) } as usize;
    if ev == 0 {
        return;
    }
    std::thread::spawn(move || loop {
        // SAFETY: ev is the valid event handle created above
        if unsafe { WaitForSingleObject(ev as _, u32::MAX) } == 0 {
            // window calls only take effect on the main thread
            let a = app.clone();
            let _ = app.run_on_main_thread(move || tray::show_window(&a));
        }
    });
}

fn main() {
    use tauri::{
        tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
        WindowEvent,
    };
    if already_running() {
        return; // the copy that is running (maybe only in the tray) was brought to the front
    }
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            modules_status, module_launch, open_module_folder, module_start, module_stop, component_install, module_admin, module_service, module_open_file, modules_all, ports_scan, take_notes,
            settings_get, settings_set_editor, settings_set_port, settings_set_autostart, settings_set_auto_update, app_info, open_root,
            updates_check, update_apply, update_rollback, open_notes, fx_enabled,
            ui_ready, progress, app_update_check, app_update_install, developer_links, open_developer, sites_list, site_add, site_remove, site_open_folder, wordpress_create, externals_status, external_start, external_stop, external_open, external_get, appearance, apply_theme, open_url, https_status, https_trust, https_untrust, https_regenerate, config_get, config_set, module_restart, config_export, config_import, config_ini_text, setup_check, setup_fix, setup_fix_all, port_remedy, port_end, port_stop_service, port_suggest
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let menu = tray::build(&handle, &tray::rows(), rt::launch_at_login())?;
            let tray_icon = TrayIconBuilder::new()
                .icon(app.default_window_icon().expect("window icon is set by tauri.conf.json").clone())
                .tooltip(tray::tooltip(&tray::rows()))
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "show" => toggle_window(app),
                    "root_folder" => {
                        let _ = std::process::Command::new("explorer.exe").arg(rt::root()).spawn();
                    }
                    "start_all" => rt::all_servers(true),
                    "stop_all" => rt::all_servers(false),
                    "login" => {
                        std::thread::spawn(|| match rt::set_launch_at_login(!rt::launch_at_login()) {
                            Ok(m) => rt::push_note("success", m),
                            Err(e) => rt::push_note("error", format!("Start with Windows: {e}")),
                        });
                    }
                    "quit" => app.exit(0), // servers keep running: they are independent processes
                    other => {
                        tray::on_menu(app, other);
                    }
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        toggle_window(tray.app_handle());
                    }
                })
                .build(app)?;
            tray::keep_fresh(handle.clone(), tray_icon);
            listen_for_second_launch(handle);
            if let Some(w) = app.get_webview_window("main") {
                // open at 85% of this screen (never smaller than the minimum), not a fixed 1040x700
                if let Ok(Some(m)) = w.primary_monitor() {
                    let (sz, sf) = (m.size(), m.scale_factor());
                    let (lw, lh) = (f64::from(sz.width) / sf * 0.85, f64::from(sz.height) / sf * 0.85);
                    let _ = w.set_size(tauri::LogicalSize::new(lw.max(560.0), lh.max(480.0)));
                    let _ = w.center();
                }
                // Mica needs Windows 11; where it is refused (Windows 10) the UI keeps its solid surfaces
                use tauri::window::{Effect, EffectsBuilder};
                let ok = w.set_effects(EffectsBuilder::new().effect(Effect::Mica).build()).is_ok();
                FX.store(ok, std::sync::atomic::Ordering::Relaxed);
                // started at sign-in (or with --hidden): live in the tray and stay out of the way
                // the window starts hidden (visible:false in the config); the page asks to be shown once it has loaded
                // (ui_ready), unless this start is --hidden, which stays in the tray until someone opens it
            }
            rt::run_autostart();
            rt::update::start_scheduler();
            Ok(())
        })
        // closing the window hides it to the tray; Quit in the tray menu exits
        .on_window_event(|w, e| {
            if let WindowEvent::CloseRequested { api, .. } = e {
                api.prevent_close();
                let _ = w.hide();
                static TOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                if !TOLD.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    rt::push_note("info", "Runqora is still running in the tray, so your servers stay manageable. Right-click the tray icon for them, or choose Quit to exit.".into());
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to start Runqora");
}
