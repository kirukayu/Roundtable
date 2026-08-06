pub mod codex;
pub mod commands;
pub mod coop;
pub mod dialog;
pub mod diagnose;
pub mod eac;
pub mod edition;
pub mod error;
pub mod formats;
pub mod game;
pub mod games;
pub mod install;
pub mod launch;
pub mod loader;
pub mod matchup;
pub mod mods;
pub mod net;
pub mod presence;
pub mod saves;
pub mod server;
pub mod settings;
pub mod steam;
pub mod sys;
pub mod wiki;

use std::sync::Arc;

use commands::AppState;
use parking_lot::Mutex;

/// Holds the running server so the launch screen can report its address and a
/// second click does not start a second one.
#[derive(Default)]
pub struct Session {
    server: Mutex<Option<server::Server>>,
    /// Serialises starting the server. The listener comes up during setup, but a
    /// click could arrive first; without this both paths would race to bind.
    starting: tokio::sync::Mutex<()>,
}

/// Brings the local server up if it is not already listening, and returns its
/// address. Safe to call from anywhere, any number of times.
async fn ensure_server(app: &Arc<AppState>, session: &Arc<Session>) -> Result<String, String> {
    if let Some(url) = session.server.lock().as_ref().map(server::Server::url) {
        return Ok(url);
    }

    let _turn = session.starting.lock().await;

    // Somebody may have finished while this task waited for its turn.
    if let Some(url) = session.server.lock().as_ref().map(server::Server::url) {
        return Ok(url);
    }

    let started = server::start(Arc::clone(app))
        .await
        .map_err(|e| e.to_string())?;
    let url = started.url();

    // Leave the address where it can be recovered. If the browser fails to open,
    // or the tab is closed and the window is gone, this is how the session is
    // found again. It sits beside the settings file, in the user's own profile,
    // and is removed when the app exits.
    let _ = std::fs::write(app.app_data.join("session.url"), &url);

    *session.server.lock() = Some(started);
    Ok(url)
}

/// The address of the running server, if there is one. The launch screen shows
/// it so the click is an invitation rather than a gamble.
#[tauri::command]
fn session_url(session: tauri::State<'_, Arc<Session>>) -> Option<String> {
    session.server.lock().as_ref().map(server::Server::url)
}

/// Starts the local server if it is not already up, opens the browser at it, and
/// minimises the launch window. The window itself keeps running: the browser
/// cannot open a folder picker, so the desktop side stays available to do it.
#[tauri::command]
async fn open_in_browser(
    app: tauri::State<'_, Arc<AppState>>,
    session: tauri::State<'_, Arc<Session>>,
    window: tauri::Window,
) -> Result<String, String> {
    let url = ensure_server(&app, &session).await?;

    open_url(&url)?;

    // Minimise rather than hide. A hidden window with only a tray icon reads as
    // the app having vanished, because Windows files new tray icons away under
    // the overflow chevron. Minimised keeps a taskbar button, so it is always
    // one click away.
    //
    // Window operations have to run on the main thread on Windows; calling
    // `minimize` straight from this task silently does nothing.
    let handle = window.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(1400)).await;
        let inner = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            let _ = inner.minimize();
        });
    });

    Ok(url)
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// Re-opens the browser tab, for the tray menu.
#[tauri::command]
fn reopen(session: tauri::State<'_, Arc<Session>>) -> Result<String, String> {
    let url = session
        .server
        .lock()
        .as_ref()
        .map(server::Server::url)
        .ok_or_else(|| "the server is not running yet".to_string())?;
    open_url(&url)?;
    Ok(url)
}

/// Hands a URL to the default browser.
///
/// `explorer.exe <url>` looks like it should work and mostly does, but when the
/// argument is not a filesystem path Explorer can fall back to opening the
/// user's Documents folder instead. `ShellExecuteW` with the `open` verb is the
/// documented way and always reaches the registered browser.
fn open_url(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let wide = |text: &str| -> Vec<u16> {
            std::ffi::OsStr::new(text)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        };

        let verb = wide("open");
        let target = wide(url);

        // Anything above 32 is success; below that the value is an error code.
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL as i32,
            )
        };

        if (result as isize) <= 32 {
            return Err(format!(
                "could not open the browser (code {})",
                result as isize
            ));
        }
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err("unsupported platform".into())
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            use tauri::Manager;

            let app_data = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            std::fs::create_dir_all(&app_data).ok();

            app.manage(Arc::new(AppState::new(app_data)));
            app.manage(Arc::new(Session::default()));

            // Bind now rather than on the click. Loopback binding is instant, so
            // by the time anyone reaches the button the page is already being
            // served and the browser opens on the first paint.
            let state = Arc::clone(&*app.state::<Arc<AppState>>());
            let session = Arc::clone(&*app.state::<Arc<Session>>());
            tauri::async_runtime::spawn(async move {
                let _ = ensure_server(&state, &session).await;
            });

            build_tray(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_in_browser,
            quit_app,
            reopen,
            session_url
        ])
        .build(tauri::generate_context!())
        .expect("failed to start Roundtable")
        .run(|handle, event| {
            // The address file describes a listener that no longer exists once
            // the process is gone; leaving it behind would send the next run to
            // a dead port.
            if matches!(event, tauri::RunEvent::Exit) {
                use tauri::Manager;
                if let Some(state) = handle.try_state::<Arc<AppState>>() {
                    let _ = std::fs::remove_file(state.app_data.join("session.url"));
                }
            }
        });
}

/// A tray icon so the app can be reached once the window is hidden.
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;
    use tauri::Manager;

    let open = MenuItem::with_id(app, "open", "Open in browser", true, None::<&str>)?;
    let window = MenuItem::with_id(app, "window", "Show window", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &window, &quit])?;

    TrayIconBuilder::with_id("roundtable")
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            tauri::Error::AssetNotFound("window icon".into())
        })?)
        .tooltip("Roundtable")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                if let Some(session) = app.try_state::<Arc<Session>>() {
                    if let Some(url) = session.server.lock().as_ref().map(server::Server::url) {
                        let _ = open_url(&url);
                    }
                }
            }
            "window" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}
