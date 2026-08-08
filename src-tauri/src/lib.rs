pub mod ask;
pub mod codex;
pub mod commands;
pub mod coop;
pub mod dialog;
pub mod diagnose;
pub mod eac;
pub mod edition;
pub mod erss;
pub mod error;
pub mod formats;
pub mod game;
pub mod games;
pub mod install;
pub mod language;
pub mod live;
pub mod launch;
pub mod loader;
pub mod matchup;
pub mod mods;
pub mod net;
pub mod perf;
pub mod presence;
pub mod saves;
pub mod server;
pub mod settings;
pub mod steam;
pub mod sys;
pub mod text;
pub mod tune;
pub mod unlock;
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

/// The label of the window that sits over the game.
const OVERLAY: &str = "overlay";

/// A handle to the app, for the parts that do not have one.
///
/// The interface is served over the local HTTP server and talks to nothing but
/// that — there is no Tauri bridge in the page, because for most of the app the
/// page is running in the user's own browser. The overlay is a Tauri window all
/// the same, so when it asks to close itself the request arrives at the server,
/// and the server needs a way to reach the window. This is that way.
static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// Closes the overlay. Called from the server, on the main thread.
pub fn hide_overlay() {
    let Some(app) = APP.get() else { return };
    let handle = app.clone();
    // Window calls have to happen on the main thread on Windows; from a request
    // handler they silently do nothing otherwise.
    let _ = app.run_on_main_thread(move || {
        use tauri::Manager;
        if let Some(window) = handle.get_webview_window(OVERLAY) {
            let _ = window.hide();
        }
    });
}

/// Whether any part of the window is somewhere a person could see it.
///
/// A window dragged off the edge, or onto a monitor that has since been
/// unplugged, opens where it was and looks like it did not open at all.
fn on_a_screen(window: &tauri::WebviewWindow) -> bool {
    let Ok(position) = window.outer_position() else {
        return true;
    };
    let Ok(size) = window.outer_size() else {
        return true;
    };
    window.available_monitors().is_ok_and(|monitors| {
        monitors.iter().any(|monitor| {
            let at = monitor.position();
            let span = monitor.size();
            // A hundred pixels of it, which is enough to take hold of.
            position.x + 100 < at.x + span.width as i32
                && position.x + size.width as i32 - 100 > at.x
                && position.y + 40 < at.y + span.height as i32
                && position.y + size.height as i32 - 40 > at.y
        })
    })
}

/// Hands the overlay to the window manager to be dragged.
///
/// The page cannot move its own window — there is no Tauri bridge in it — and
/// moving it by posting a new position on every pointer event would be a round
/// trip per frame and would lag behind the cursor. This is one call: the OS
/// takes the drag from here and follows the mouse itself, which is why it feels
/// like moving any other window rather than like dragging something in a page.
pub fn drag_overlay() {
    let Some(app) = APP.get() else { return };
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        use tauri::Manager;
        if let Some(window) = handle.get_webview_window(OVERLAY) {
            let _ = window.start_dragging();
        }
    });
}

/// Puts the overlay back where it was opened.
pub fn centre_overlay() {
    let Some(app) = APP.get() else { return };
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        use tauri::Manager;
        if let Some(window) = handle.get_webview_window(OVERLAY) {
            let _ = window.center();
        }
    });
}

/// How long the window stays up after the page has been told it is going.
///
/// Long enough for a short exit, short enough that the key still feels like a
/// switch. Without the pause the window is gone in the same frame it is told,
/// and the animation is written but never seen.
const LEAVING: std::time::Duration = std::time::Duration::from_millis(170);

/// Tells the overlay page what is happening to the window around it.
///
/// A DOM event rather than one of Tauri's, because the same page is also served
/// to a browser tab where Tauri's event bus does not exist. There these never
/// arrive and the page behaves as it always did.
///
/// This is the only way the page can know: the window is hidden and shown
/// rather than built and destroyed, so nothing unmounts and nothing remounts,
/// and every opening after the first would otherwise appear with no animation
/// at all.
fn tell_overlay(window: &tauri::WebviewWindow, what: &str) {
    let _ = window.eval(format!("window.dispatchEvent(new Event('roundtable:{what}'))"));
}

/// Plays the page's exit, then takes the window away.
fn leave_overlay(window: tauri::WebviewWindow) {
    tell_overlay(&window, "leaving");
    std::thread::spawn(move || {
        std::thread::sleep(LEAVING);
        let _ = window.hide();
    });
}

/// Shows the overlay, building it the first time it is asked for.
///
/// It is a Tauri window rather than the browser the rest of the interface runs
/// in, because only a native window can be told to stay above a game. That
/// works because Roundtable has already moved the game to borderless — the
/// desktop composites a borderless window, so anything marked always-on-top
/// draws over it. In exclusive fullscreen nothing would, which is one more
/// reason the optimiser changes that first.
fn show_overlay(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;

    if let Some(window) = app.get_webview_window(OVERLAY) {
        // Back on screen if it was dragged off one, and in front whatever was
        // in front a moment ago.
        if !on_a_screen(&window) {
            let _ = window.center();
        }
        let _ = window.show();
        let _ = window.set_always_on_top(true);
        let _ = window.set_focus();
        tell_overlay(&window, "shown");
        return Ok(());
    }

    // The overlay is a page on the same local server as everything else, so it
    // shares the stylesheet and needs the session key like any other caller.
    let url = app
        .try_state::<Arc<Session>>()
        .and_then(|session| session.server.lock().as_ref().map(server::Server::url))
        .ok_or_else(|| "the server is not running yet".to_string())?;
    let target = format!("{url}#/overlay");

    let parsed = target
        .parse()
        .map_err(|_| "could not build the overlay address".to_string())?;

    // Tall and narrow, because that is the shape of the thing inside it: a
    // standing column with the answer above and the line you type in at the
    // bottom, where your hands already are. A wide panel across the middle of
    // the screen covers the fight; a column down one side covers a wall.
    tauri::WebviewWindowBuilder::new(app, OVERLAY, tauri::WebviewUrl::External(parsed))
        .title("Roundtable")
        .inner_size(400.0, 780.0)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .center()
        .resizable(false)
        // Shadows draw a rectangle around a transparent window on Windows.
        .shadow(false)
        .build()
        .map_err(|e| format!("could not open the overlay: {e}"))?;

    Ok(())
}

#[tauri::command]
fn overlay_hide(app: tauri::AppHandle) {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window(OVERLAY) {
        let _ = window.hide();
    }
}

#[tauri::command]
/// Shift F1: open it, or put it away if it is already in front of you.
///
/// Visible is not the same as in front of you. Pressing the key over a game
/// shows the window, but the game keeps keyboard focus — so the next press
/// found it visible and hid it, and from the outside the key looked like it
/// worked every other time. Hiding needs both.
fn overlay_toggle(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    match app.get_webview_window(OVERLAY) {
        Some(window)
            if window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false) =>
        {
            leave_overlay(window);
            Ok(())
        }
        _ => show_overlay(&app),
    }
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
            let _ = APP.set(app.handle().clone());

            // Bind now rather than on the click. Loopback binding is instant, so
            // by the time anyone reaches the button the page is already being
            // served and the browser opens on the first paint.
            let state = Arc::clone(&*app.state::<Arc<AppState>>());
            let session = Arc::clone(&*app.state::<Arc<Session>>());
            tauri::async_runtime::spawn(async move {
                let _ = ensure_server(&state, &session).await;
            });

            // One key opens the overlay over the game. Shift is in there
            // because a bare function key belongs to whatever is in focus, and
            // taking F1 off the whole machine to save a modifier is rude.
            {
                use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};

                let overlay = Shortcut::new(Some(Modifiers::SHIFT), Code::F1);
                // The pointer going jerky happens mid-game, and the cure is a
                // display mode rebuild. Alt-tabbing to a menu to fix a problem
                // that is *caused* by leaving a game is no cure at all.
                let bounce = Shortcut::new(Some(Modifiers::SHIFT), Code::F2);

                app.handle().plugin(
                    tauri_plugin_global_shortcut::Builder::new()
                        .with_handler(move |app, pressed, event| {
                            // Both edges arrive; acting on the release as well
                            // would toggle it straight back shut.
                            if event.state() != ShortcutState::Pressed {
                                return;
                            }
                            if *pressed == overlay {
                                let handle = app.clone();
                                let _ = app.run_on_main_thread(move || {
                                    let _ = overlay_toggle(handle);
                                });
                            } else if *pressed == bounce {
                                // Blocking and nearly two seconds long, so it
                                // must not sit on the event thread.
                                std::thread::spawn(|| match crate::perf::bounce_refresh() {
                                    Ok(what) => tracing::info!("{what}"),
                                    Err(error) => tracing::warn!(%error, "display bounce failed"),
                                });
                            }
                        })
                        .build(),
                )?;

                use tauri_plugin_global_shortcut::GlobalShortcutExt;
                for (key, what) in [(overlay, "overlay"), (bounce, "display bounce")] {
                    if let Err(error) = app.global_shortcut().register(key) {
                        // Another program may already own the combination. Worth
                        // a line in the log and no more — everything else still
                        // works without it.
                        tracing::warn!(%error, "could not register the {what} hotkey");
                    }
                }
            }

            build_tray(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_in_browser,
            quit_app,
            reopen,
            session_url,
            overlay_toggle,
            overlay_hide
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
