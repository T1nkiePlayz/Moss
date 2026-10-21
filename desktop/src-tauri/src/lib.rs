//! Moss desktop: a native window and tray icon around the bundled Moss controller.
//! Closing the window only hides it. The controller (and your game servers) keep running until you choose Quit.
use moss_desktop_core::{Handle, Paths, attach_or_start_with, log_tail, nav, stop};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::{NewWindowResponse, WebviewWindowBuilder};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager, Url, WebviewUrl, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogResult};
use tauri_plugin_opener::OpenerExt;

const LABEL_STOP: &str = "Stop controller and quit";
const LABEL_LEAVE: &str = "Quit, leave it running";
const LABEL_CANCEL: &str = "Cancel";

/// What the window should show; replayed after every page load so a window that appears late still gets it.
enum Status { Starting, Ready(String), Failed(String, String) }

/// The controller this app talks to, once it is up.
struct Controller {
    paths: Paths,
    handle: Mutex<Option<Handle>>,
    status: Mutex<Status>,
    /// A start is in progress: ignore further Retry clicks. Quit waits for it to clear before stopping the controller.
    busy: AtomicBool,
    quitting: AtomicBool,
    splash: OnceLock<Url>,
}

fn is_local(url: &Url) -> bool {
    url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost") || url.scheme() == "about"
}

fn open_external(app: &AppHandle, url: &str) {
    if nav::may_open_externally(url) { let _ = app.opener().open_url(url, None::<&str>); }
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") { let _ = w.unminimize(); let _ = w.show(); let _ = w.set_focus(); }
}

fn show_error(app: &AppHandle, msg: &str, detail: &str) {
    if let Some(w) = app.get_webview_window("main") {
        let q = |t: &str| serde_json::to_string(t).unwrap_or_default();
        let _ = w.eval(format!("window.__mossError && window.__mossError({}, {})", q(msg), q(detail)));
    }
}

/// Error text for the splash: the message, plus the log path and the last lines of the log.
fn failure(c: &Controller, e: &str) -> Status {
    let detail = format!("Log file: {}\n\n{}", c.paths.log_file().display(), log_tail(&c.paths, 10));
    Status::Failed(e.to_string(), detail)
}

fn navigate_to(app: &AppHandle, url: &str) {
    if let (Some(w), Ok(u)) = (app.get_webview_window("main"), url.parse()) { let _ = w.navigate(u); }
}

/// Start (or restart) the controller in the background. Safe to call from any thread; overlapping calls are ignored.
/// The thread is the only owner of a child it spawns until the child answers: a Quit meanwhile sets `quitting`, which
/// makes the start kill and reap its own child, and then waits for `busy` to clear.
fn start(app: &AppHandle) {
    let c = app.state::<Controller>();
    if c.quitting.load(Ordering::SeqCst) || c.busy.swap(true, Ordering::SeqCst) { return; }
    if let Ok(mut s) = c.status.lock() { *s = Status::Starting; }
    if let Some(w) = app.get_webview_window("main") { let _ = w.eval("window.__mossStarting && window.__mossStarting()"); }
    let app = app.clone();
    std::thread::spawn(move || {
        let c = app.state::<Controller>();
        let result = attach_or_start_with(&c.paths, &|_| {}, &|| c.quitting.load(Ordering::SeqCst));
        // Status first, then the page: a page load in between replays the same outcome.
        match result {
            Ok(h) => {
                let url = h.url();
                if let Ok(mut g) = c.handle.lock() { *g = Some(h); }
                if let Ok(mut s) = c.status.lock() { *s = Status::Ready(url.clone()); }
                c.busy.store(false, Ordering::SeqCst);
                if c.quitting.load(Ordering::SeqCst) { return; } // Quit takes it from `handle`
                navigate_to(&app, &url);
                watch(&app);
            }
            Err(e) => {
                let st = failure(&c, &e);
                let shown = if let Status::Failed(m, d) = &st { Some((m.clone(), d.clone())) } else { None };
                if let Ok(mut s) = c.status.lock() { *s = st; }
                c.busy.store(false, Ordering::SeqCst);
                if let Some((m, d)) = shown { show_error(&app, &m, &d); }
            }
        }
    });
}

/// Crash watcher: every ~3 s, check that the controller process still exists. Only a dead process counts: a slow or
/// busy one is left alone, so a second controller is never started beside a live one. When it has died, go back to the
/// splash page in its error state, where Retry is offered.
fn watch(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let c = app.state::<Controller>();
        loop {
            std::thread::sleep(Duration::from_secs(3));
            if c.quitting.load(Ordering::SeqCst) { return; }
            let Ok(mut g) = c.handle.lock() else { return };
            let Some(h) = g.as_mut() else { return };
            if h.is_alive() { continue; } // a non-blocking check (reaps our own child): never holds the lock for long
            g.take();
            drop(g);
            moss_desktop_core::state::clear(&c.paths.root);
            if let Ok(mut s) = c.status.lock() { *s = failure(&c, "The Moss controller stopped unexpectedly."); }
            if let (Some(w), Some(u)) = (app.get_webview_window("main"), c.splash.get()) { let _ = w.navigate(u.clone()); }
            return;
        }
    });
}

/// Retry from the splash error state.
#[tauri::command]
fn retry(app: AppHandle) { start(&app); }

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
        .buttons(MessageDialogButtons::YesNoCancelCustom(LABEL_STOP.into(), LABEL_LEAVE.into(), LABEL_CANCEL.into()))
        .show_with_result(move |r| {
            let stop_it = match r {
                MessageDialogResult::Custom(s) if s == LABEL_STOP => true,
                MessageDialogResult::Custom(s) if s == LABEL_LEAVE => false,
                MessageDialogResult::Yes => true,
                MessageDialogResult::No => false,
                _ => return,
            };
            if let Some(c) = app2.try_state::<Controller>() {
                // Let a start in flight finish either way. When stopping, it sees `quitting` and kills its own
                // half-started child at once. When leaving the controller running, it must get as far as writing the
                // controller record, or the next launch could not attach to the controller left behind; its own
                // timeouts (three tries of 30 s) bound the wait.
                if stop_it { c.quitting.store(true, Ordering::SeqCst); }
                if let Some(w) = app2.get_webview_window("main") { let _ = w.hide(); }
                let end = std::time::Instant::now() + Duration::from_secs(if stop_it { 10 } else { 95 });
                while c.busy.load(Ordering::SeqCst) && std::time::Instant::now() < end { std::thread::sleep(Duration::from_millis(50)); }
                // A start that finished (or attached to a running controller) left it in `handle`.
                if stop_it && let Some(mut h) = c.handle.lock().ok().and_then(|mut g| g.take()) {
                    let _ = stop(&c.paths, &mut h, Duration::from_secs(20));
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
        .invoke_handler(tauri::generate_handler![retry])
        .setup(|app| {
            let handle = app.handle().clone();
            let paths = paths(&handle)?;
            app.manage(Controller {
                paths, handle: Mutex::new(None), status: Mutex::new(Status::Starting),
                busy: AtomicBool::new(false), quitting: AtomicBool::new(false), splash: OnceLock::new(),
            });
            // Boot the controller first so it overlaps with webview start-up; the page load replays the outcome.
            start(&handle);
            // The window opens at once on a local "starting" page and is pointed at the panel when the controller answers.
            let nav_handle = handle.clone();
            let win = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Moss").inner_size(1280.0, 820.0).min_inner_size(420.0, 600.0)
                .on_navigation(move |url| {
                    let u = url.as_str();
                    let local = is_local(url);
                    let port = nav_handle.try_state::<Controller>().and_then(|c| c.handle.lock().ok().and_then(|g| g.as_ref().map(|h| h.controller.port)));
                    if local || port.is_some_and(|p| nav::is_panel_url(u, p)) { return true; }
                    open_external(&nav_handle, u); // everything else goes to the system browser
                    false
                })
                .on_page_load(|w, p| {
                    if !matches!(p.event(), PageLoadEvent::Finished) || p.url().scheme() == "about" || !is_local(p.url()) { return; }
                    let app = w.app_handle();
                    let Some(c) = app.try_state::<Controller>() else { return };
                    let _ = c.splash.set(p.url().clone());
                    match &*c.status.lock().unwrap_or_else(|e| e.into_inner()) {
                        Status::Ready(url) => navigate_to(app, url),
                        Status::Failed(m, d) => show_error(app, m, d),
                        Status::Starting => {}
                    }
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
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Moss");
}
