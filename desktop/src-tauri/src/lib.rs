//! Moss desktop: a native window and tray icon around the bundled Moss controller.
//! Closing the window only hides it. The controller (and your game servers) keep running until you choose Quit.
use moss_desktop_core::{Handle, Paths, attach_or_start, nav, stop};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::{NewWindowResponse, WebviewWindowBuilder};
use tauri::{AppHandle, Manager, WebviewUrl, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogResult};
use tauri_plugin_opener::OpenerExt;

/// The controller this app talks to, once it is up.
struct Controller { paths: Paths, handle: Mutex<Option<Handle>> }

fn open_external(app: &AppHandle, url: &str) {
    if nav::may_open_externally(url) { let _ = app.opener().open_url(url, None::<&str>); }
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") { let _ = w.unminimize(); let _ = w.show(); let _ = w.set_focus(); }
}

fn show_error(app: &AppHandle, msg: &str) {
    if let Some(w) = app.get_webview_window("main") {
        let js = format!("window.__mossError && window.__mossError({})", serde_json::to_string(msg).unwrap_or_default());
        let _ = w.eval(&js);
    }
}

fn paths(app: &AppHandle) -> Result<Paths, String> {
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let binary = exe.parent().ok_or("no executable folder")?.join(if cfg!(windows) { "moss.exe" } else { "moss" });
    let shipped_env = app.path().resource_dir().ok().map(|d| d.join("shipped.env")).filter(|p| p.is_file());
    Ok(Paths { root, binary, shipped_env })
}

/// Quit: ask whether to stop the controller too. Game servers survive either way.
fn ask_quit(app: &AppHandle) {
    let app2 = app.clone();
    app.dialog()
        .message("Closing Moss does not stop your game servers. Do you also want to stop the Moss controller? Without it you cannot manage servers until you open Moss again.")
        .title("Quit Moss")
        .buttons(MessageDialogButtons::YesNoCancelCustom("Stop controller and quit".into(), "Quit, leave it running".into(), "Cancel".into()))
        .show_with_result(move |r| {
            let stop_it = match r {
                MessageDialogResult::Custom(s) if s == "Stop controller and quit" => true,
                MessageDialogResult::Custom(s) if s == "Quit, leave it running" => false,
                MessageDialogResult::Yes => true,
                MessageDialogResult::No => false,
                _ => return,
            };
            if stop_it {
                if let Some(c) = app2.try_state::<Controller>() {
                    if let Some(h) = c.handle.lock().ok().and_then(|mut g| g.take()) {
                        let mut h = h;
                        stop(&c.paths, &mut h, Duration::from_secs(20));
                    }
                }
            }
            app2.exit(0);
        });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let paths = paths(&handle)?;
            // The window opens at once on a local "starting" page and is pointed at the panel when the controller answers.
            let nav_handle = handle.clone();
            let win = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Moss").inner_size(1280.0, 820.0).min_inner_size(420.0, 600.0)
                .on_navigation(move |url| {
                    let u = url.as_str();
                    let local = url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost") || url.scheme() == "about";
                    let port = nav_handle.try_state::<Controller>().and_then(|c| c.handle.lock().ok().and_then(|g| g.as_ref().map(|h| h.controller.port)));
                    if local || port.is_some_and(|p| nav::is_panel_url(u, p)) { return true; }
                    open_external(&nav_handle, u); // everything else goes to the system browser
                    false
                })
                .on_new_window({ let h = handle.clone(); move |url, _| { open_external(&h, url.as_str()); NewWindowResponse::Deny } })
                .build()?;
            let w2 = win.clone();
            win.on_window_event(move |e| {
                if let WindowEvent::CloseRequested { api, .. } = e { api.prevent_close(); let _ = w2.hide(); }
            });

            let open = MenuItem::with_id(app, "open", "Open Moss", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let mut tray = TrayIconBuilder::new().tooltip("Moss").menu(&Menu::with_items(app, &[&open, &quit])?)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() { "open" => show_main(app), "quit" => ask_quit(app), _ => {} })
                .on_tray_icon_event(|t, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e { show_main(t.app_handle()); }
                });
            if let Some(i) = app.default_window_icon() { tray = tray.icon(i.clone()); }
            tray.build(app)?;

            app.manage(Controller { paths, handle: Mutex::new(None) });
            std::thread::spawn(move || {
                let c = handle.state::<Controller>();
                match attach_or_start(&c.paths) {
                    Ok(h) => {
                        let url = h.url();
                        if let Ok(mut g) = c.handle.lock() { *g = Some(h); }
                        if let (Some(w), Ok(u)) = (handle.get_webview_window("main"), url.parse()) { let _ = w.navigate(u); }
                    }
                    Err(e) => show_error(&handle, &e),
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Moss");
}
