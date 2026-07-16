use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};

pub const TEST_CONTROL_PORT: u16 = 47613;

/// Logical (CSS-pixel), window-relative rect reported by the frontend.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Default)]
struct Shared {
    rects: Vec<Rect>,
    /// Last click-through state actually applied to the window.
    ignoring: Option<bool>,
    bubble_open: bool,
    last_input: String,
    /// Foreground window to restore when the bubble closes.
    prev_foreground: isize,
    own_hwnd: isize,
}

struct AppState(Mutex<Shared>);

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn GetForegroundWindow() -> isize;
    fn SetForegroundWindow(hwnd: isize) -> i32;
}

#[cfg(not(windows))]
unsafe fn GetForegroundWindow() -> isize {
    0
}
#[cfg(not(windows))]
unsafe fn SetForegroundWindow(_hwnd: isize) -> i32 {
    0
}

#[tauri::command]
fn set_hit_regions(state: tauri::State<AppState>, rects: Vec<Rect>) {
    state.0.lock().unwrap().rects = rects;
}

#[tauri::command]
fn report_input(state: tauri::State<AppState>, value: String) {
    state.0.lock().unwrap().last_input = value;
}

#[tauri::command]
fn bubble_state(app: AppHandle, state: tauri::State<AppState>, open: bool) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let prev = {
        let mut s = state.0.lock().unwrap();
        s.bubble_open = open;
        if open {
            s.ignoring = Some(false);
        }
        s.prev_foreground
    };
    if open {
        let _ = window.set_ignore_cursor_events(false);
        let _ = window.set_focus();
    } else if prev != 0 {
        unsafe {
            SetForegroundWindow(prev);
        }
    }
}

/// Convert the stored logical window-relative rects to physical screen rects.
fn physical_rects(window: &WebviewWindow, rects: &[Rect]) -> Vec<Rect> {
    let (Ok(pos), Ok(scale)) = (window.outer_position(), window.scale_factor()) else {
        return Vec::new();
    };
    rects
        .iter()
        .map(|r| Rect {
            x: pos.x as f64 + r.x * scale,
            y: pos.y as f64 + r.y * scale,
            w: r.w * scale,
            h: r.h * scale,
        })
        .collect()
}

/// Poll the global cursor and toggle click-through: interactive over a hit
/// region, click-through (WS_EX_TRANSPARENT) everywhere else. Tauri has no
/// per-region hit-testing, so this poll IS the hit-region mechanism.
fn spawn_cursor_poll(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(16));
        let Some(window) = app.get_webview_window("main") else {
            continue;
        };
        let Ok(cursor) = app.cursor_position() else {
            continue;
        };
        let state = app.state::<AppState>();
        let (rects, own_hwnd) = {
            let s = state.0.lock().unwrap();
            (s.rects.clone(), s.own_hwnd)
        };
        let inside = physical_rects(&window, &rects).iter().any(|r| {
            cursor.x >= r.x && cursor.x <= r.x + r.w && cursor.y >= r.y && cursor.y <= r.y + r.h
        });
        let want_ignore = !inside;
        let mut s = state.0.lock().unwrap();
        if s.ignoring != Some(want_ignore)
            && window.set_ignore_cursor_events(want_ignore).is_ok()
        {
            s.ignoring = Some(want_ignore);
        }
        // While we are click-through, whatever is foreground is where focus
        // should return when the bubble closes.
        if want_ignore {
            let fg = unsafe { GetForegroundWindow() };
            if fg != 0 && fg != own_hwnd {
                s.prev_foreground = fg;
            }
        }
    });
}

/// Localhost-only control channel for the automated gate tests.
/// Active only when OCELLUM_TEST=1. Line in, one JSON line out.
fn spawn_test_control(app: AppHandle) {
    std::thread::spawn(move || {
        let listener = TcpListener::bind(("127.0.0.1", TEST_CONTROL_PORT))
            .expect("test control channel bind failed");
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let state = app.state::<AppState>();
            let reply = match line.trim() {
                "hwnd" => {
                    serde_json::json!({ "hwnd": state.0.lock().unwrap().own_hwnd })
                }
                "ignore-state" => {
                    serde_json::json!({ "ignoring": state.0.lock().unwrap().ignoring })
                }
                "bubble-open" => {
                    serde_json::json!({ "open": state.0.lock().unwrap().bubble_open })
                }
                "input-value" => {
                    serde_json::json!({ "value": state.0.lock().unwrap().last_input })
                }
                "rects" => {
                    let rects = state.0.lock().unwrap().rects.clone();
                    match app.get_webview_window("main") {
                        Some(w) => serde_json::json!({ "rects": physical_rects(&w, &rects) }),
                        None => serde_json::json!({ "rects": [] }),
                    }
                }
                "open-bubble" => {
                    let _ = app.emit("open-bubble", ());
                    serde_json::json!({ "ok": true })
                }
                "close-bubble" => {
                    let _ = app.emit("close-bubble", ());
                    serde_json::json!({ "ok": true })
                }
                other => serde_json::json!({ "error": format!("unknown command: {other}") }),
            };
            let _ = writeln!(stream, "{reply}");
        }
    });
}

fn position_bottom_right(window: &WebviewWindow) {
    let Ok(Some(monitor)) = window.primary_monitor() else {
        return;
    };
    let Ok(wsize) = window.outer_size() else {
        return;
    };
    let mpos = monitor.position();
    let msize = monitor.size();
    // ponytail: 56px bottom margin clears a default-height taskbar; use
    // monitor work_area if taskbar overlap is ever reported.
    let x = mpos.x + msize.width as i32 - wsize.width as i32 - 8;
    let y = mpos.y + msize.height as i32 - wsize.height as i32 - 56;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState(Mutex::new(Shared::default())))
        .invoke_handler(tauri::generate_handler![
            set_hit_regions,
            report_input,
            bubble_state
        ])
        .setup(|app| {
            let window = app.get_webview_window("main").expect("main window");

            #[cfg(windows)]
            {
                let hwnd = window.hwnd()?;
                app.state::<AppState>().0.lock().unwrap().own_hwnd = hwnd.0 as isize;
            }

            position_bottom_right(&window);
            let _ = window.set_ignore_cursor_events(true);

            // Reposition after DPI/resolution changes (also covers wake with
            // a changed display layout).
            let win_for_event = window.clone();
            window.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::ScaleFactorChanged { .. }) {
                    position_bottom_right(&win_for_event);
                }
            });

            let quit = MenuItem::with_id(app, "quit", "Quit Ocellum", true, None::<&str>)?;
            let summon = MenuItem::with_id(app, "summon", "Summon", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&summon, &quit])?;
            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().expect("bundled icon").clone())
                .tooltip("Ocellum")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "summon" => {
                        let _ = app.emit("toggle-bubble", ());
                    }
                    _ => {}
                })
                .build(app)?;

            use tauri_plugin_global_shortcut::ShortcutState;
            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_shortcuts(["ctrl+shift+o"])?
                    .with_handler(|app, _shortcut, event| {
                        if event.state() == ShortcutState::Pressed {
                            let _ = app.emit("toggle-bubble", ());
                        }
                    })
                    .build(),
            )?;

            spawn_cursor_poll(app.handle().clone());
            if std::env::var("OCELLUM_TEST").as_deref() == Ok("1") {
                spawn_test_control(app.handle().clone());
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Ocellum");
}
