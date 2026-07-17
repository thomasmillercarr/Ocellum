mod chat;
mod db;
mod leads;
mod mood;
mod prompt;
mod providers;

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

/// Layer filenames of the character asset contract (§8.2). Fixed set —
/// discovery is "which of these exist in the directory".
const LAYER_NAMES: [&str; 11] = [
    "body",
    "shadow",
    "eyes_open",
    "eyes_half",
    "eyes_closed",
    "mouth_closed",
    "mouth_open",
    "brows_neutral",
    "brows_happy",
    "brows_restless",
    "brows_flat",
];

#[derive(Serialize, Debug)]
pub struct CharacterDirData {
    pub manifest: String,
    /// layer name -> base64 PNG bytes. Validation happens in the frontend
    /// loader (registration check on IHDR before decode).
    pub layers: std::collections::HashMap<String, String>,
}

#[tauri::command]
fn read_character_dir(path: String) -> Result<CharacterDirData, String> {
    use base64::Engine;
    let dir = std::path::Path::new(&path);
    let manifest = std::fs::read_to_string(dir.join("character.json"))
        .map_err(|e| format!("cannot read character.json in {path}: {e}"))?;
    let mut layers = std::collections::HashMap::new();
    for name in LAYER_NAMES {
        let file = dir.join(format!("{name}.png"));
        if file.exists() {
            let bytes =
                std::fs::read(&file).map_err(|e| format!("cannot read {name}.png: {e}"))?;
            layers.insert(
                name.to_string(),
                base64::engine::general_purpose::STANDARD.encode(bytes),
            );
        }
    }
    Ok(CharacterDirData { manifest, layers })
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
                "chat-log" => {
                    let chat_state = app.state::<chat::ChatState>();
                    let last = chat_state.last_response.lock().unwrap().clone();
                    serde_json::json!({ "last": last })
                }
                "egress-hosts" => {
                    let dbs = app.state::<chat::DbState>();
                    let conn = dbs.0.lock().unwrap();
                    let hosts: Vec<String> = conn
                        .prepare("SELECT DISTINCT destination FROM egress_log")
                        .and_then(|mut s| {
                            s.query_map([], |r| r.get(0))
                                .map(|rows| rows.filter_map(Result::ok).collect())
                        })
                        .unwrap_or_default();
                    serde_json::json!({ "hosts": hosts })
                }
                "egress-count" => {
                    let dbs = app.state::<chat::DbState>();
                    let n = db::egress_count(&dbs.0.lock().unwrap());
                    serde_json::json!({ "count": n })
                }
                cmd if cmd.starts_with("set-setting ") => {
                    let rest = &cmd["set-setting ".len()..];
                    match rest.split_once(' ') {
                        Some((key, value)) => {
                            let dbs = app.state::<chat::DbState>();
                            let conn = dbs.0.lock().unwrap();
                            match db::setting_set(&conn, key, value) {
                                Ok(()) => serde_json::json!({ "ok": true }),
                                Err(e) => serde_json::json!({ "error": e.to_string() }),
                            }
                        }
                        None => serde_json::json!({ "error": "usage: set-setting <key> <value>" }),
                    }
                }
                cmd if cmd.starts_with("get-setting ") => {
                    let key = cmd["get-setting ".len()..].trim();
                    let dbs = app.state::<chat::DbState>();
                    let conn = dbs.0.lock().unwrap();
                    serde_json::json!({ "value": chat::setting_or_default(&conn, key) })
                }
                cmd if cmd.starts_with("capture ") => {
                    let text = cmd["capture ".len()..].replace("\\n", "\n");
                    let dbs = app.state::<chat::DbState>();
                    let conn = dbs.0.lock().unwrap();
                    match leads::insert_lead(&conn, &text, "test") {
                        Ok(id) => serde_json::json!({ "id": id }),
                        Err(e) => serde_json::json!({ "error": e.to_string() }),
                    }
                }
                cmd if cmd.starts_with("enrich ") => {
                    match cmd["enrich ".len()..].trim().parse::<i64>() {
                        Ok(id) => match leads::enrich(&app, id) {
                            Ok(notes) => serde_json::json!({ "notes": notes }),
                            Err(e) => serde_json::json!({ "error": e }),
                        },
                        Err(_) => serde_json::json!({ "error": "bad id" }),
                    }
                }
                cmd if cmd.starts_with("draft ") => {
                    match cmd["draft ".len()..].trim().parse::<i64>() {
                        Ok(id) => match leads::draft(&app, id) {
                            Ok(text) => serde_json::json!({ "text": text }),
                            Err(e) => serde_json::json!({ "error": e }),
                        },
                        Err(_) => serde_json::json!({ "error": "bad id" }),
                    }
                }
                cmd if cmd.starts_with("remind ") => {
                    let parts: Vec<&str> = cmd["remind ".len()..].split_whitespace().collect();
                    match (parts.first().and_then(|s| s.parse::<i64>().ok()),
                           parts.get(1).and_then(|s| s.parse::<i64>().ok())) {
                        (Some(id), Some(secs)) => {
                            let due = (chrono::Utc::now() + chrono::Duration::seconds(secs)).to_rfc3339();
                            let dbs = app.state::<chat::DbState>();
                            let conn = dbs.0.lock().unwrap();
                            match leads::schedule_reminder(&conn, id, &due, "gate") {
                                Ok(rid) => serde_json::json!({ "reminder_id": rid }),
                                Err(e) => serde_json::json!({ "error": e.to_string() }),
                            }
                        }
                        _ => serde_json::json!({ "error": "usage: remind <lead_id> <secs>" }),
                    }
                }
                cmd if cmd.starts_with("reminder-state ") => {
                    let id: i64 = cmd["reminder-state ".len()..].trim().parse().unwrap_or(0);
                    let dbs = app.state::<chat::DbState>();
                    let conn = dbs.0.lock().unwrap();
                    let state: Result<String, _> = conn.query_row(
                        "SELECT state FROM reminder WHERE id = ?1",
                        [id],
                        |r| r.get(0),
                    );
                    match state {
                        Ok(s) => serde_json::json!({ "state": s }),
                        Err(e) => serde_json::json!({ "error": e.to_string() }),
                    }
                }
                "lead-rows" => {
                    let dbs = app.state::<chat::DbState>();
                    let conn = dbs.0.lock().unwrap();
                    let leads: i64 = conn.query_row("SELECT COUNT(*) FROM lead", [], |r| r.get(0)).unwrap_or(0);
                    let enrichments: i64 = conn.query_row("SELECT COUNT(*) FROM enrichment", [], |r| r.get(0)).unwrap_or(0);
                    let drafts: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM interaction WHERE kind = 'draft_email'", [], |r| r.get(0)).unwrap_or(0);
                    serde_json::json!({ "leads": leads, "enrichments": enrichments, "drafts": drafts })
                }
                "mood" => {
                    let dbs = app.state::<chat::DbState>();
                    let conn = dbs.0.lock().unwrap();
                    let events: i64 = conn
                        .query_row("SELECT COUNT(*) FROM mood_event", [], |r| r.get(0))
                        .unwrap_or(0);
                    serde_json::json!({
                        "mood": mood::derive_mood(&conn).as_str(),
                        "events": events,
                    })
                }
                "surfaces" => {
                    let log = app.state::<leads::SurfaceLog>();
                    let map = log.0.lock().unwrap().clone();
                    serde_json::json!({ "surfaces": map })
                }
                cmd if cmd.starts_with("fire-surface ") => {
                    let t = cmd["fire-surface ".len()..].trim().to_string();
                    let shown = leads::surface(
                        &app,
                        &t,
                        "gate-test evidence: synthetic trigger",
                        serde_json::json!({}),
                    );
                    serde_json::json!({ "shown": shown })
                }
                cmd if cmd.starts_with("dismiss ") => {
                    let t = cmd["dismiss ".len()..].trim();
                    let dbs = app.state::<chat::DbState>();
                    let conn = dbs.0.lock().unwrap();
                    match leads::record_dismissal(&conn, t, "gate") {
                        Ok(()) => serde_json::json!({ "ok": true }),
                        Err(e) => serde_json::json!({ "error": e.to_string() }),
                    }
                }
                cmd if cmd.starts_with("delete-key ") => {
                    let provider = cmd["delete-key ".len()..].trim();
                    match chat::remove_key(provider) {
                        Ok(()) => serde_json::json!({ "ok": true }),
                        Err(e) => serde_json::json!({ "error": e }),
                    }
                }
                cmd if cmd.starts_with("set-key ") => {
                    let rest = &cmd["set-key ".len()..];
                    match rest.split_once(' ') {
                        Some((provider, key)) => match chat::store_key(provider, key) {
                            Ok(()) => serde_json::json!({ "ok": true }),
                            Err(e) => serde_json::json!({ "error": e }),
                        },
                        None => serde_json::json!({ "error": "usage: set-key <provider> <key>" }),
                    }
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
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState(Mutex::new(Shared::default())))
        .manage(chat::ChatState::default())
        .manage(leads::SurfaceLog::default())
        .invoke_handler(tauri::generate_handler![
            set_hit_regions,
            report_input,
            bubble_state,
            read_character_dir,
            chat::get_settings,
            chat::set_setting,
            chat::set_api_key,
            chat::delete_api_key,
            chat::get_budget,
            chat::get_egress_log,
            chat::chat_send,
            leads::capture_lead,
            leads::list_leads,
            leads::enrich_lead,
            leads::draft_lead_email,
            leads::remind_lead,
            leads::dismiss_surface,
            mood::get_mood
        ])
        .setup(|app| {
            // Local store: OCELLUM_DB_PATH overrides for tests.
            let db_path = match std::env::var("OCELLUM_DB_PATH") {
                Ok(p) => std::path::PathBuf::from(p),
                Err(_) => {
                    let dir = app.path().app_data_dir().expect("app data dir");
                    std::fs::create_dir_all(&dir)?;
                    dir.join("ocellum.db")
                }
            };
            let conn = db::open(&db_path)?;
            app.manage(chat::DbState(Mutex::new(conn)));

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
            let silence_on = {
                let dbs = app.state::<chat::DbState>();
                let conn = dbs.0.lock().unwrap();
                chat::setting_or_default(&conn, "hard_silence") == "1"
            };
            let silence = tauri::menu::CheckMenuItem::with_id(
                app,
                "silence",
                "Hard silence",
                true,
                silence_on,
                None::<&str>,
            )?;
            let menu = Menu::with_items(app, &[&summon, &silence, &quit])?;
            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().expect("bundled icon").clone())
                .tooltip("Ocellum")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "summon" => {
                        let _ = app.emit("toggle-bubble", ());
                    }
                    "silence" => {
                        let dbs = app.state::<chat::DbState>();
                        let conn = dbs.0.lock().unwrap();
                        let now = chat::setting_or_default(&conn, "hard_silence") == "1";
                        let _ = db::setting_set(&conn, "hard_silence", if now { "0" } else { "1" });
                        let _ = silence.set_checked(!now);
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
            leads::spawn_reminder_scanner(app.handle().clone());
            leads::spawn_clipboard_monitor(app.handle().clone());
            if std::env::var("OCELLUM_TEST").as_deref() == Ok("1") {
                spawn_test_control(app.handle().clone());
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Ocellum");
}

#[cfg(test)]
mod tests {
    #[test]
    fn read_character_dir_reads_manifest_and_existing_layers() {
        let dir = std::env::temp_dir().join(format!("ocellum-chardir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("character.json"), r#"{"name":"t"}"#).unwrap();
        std::fs::write(dir.join("body.png"), [1u8, 2, 3]).unwrap();
        std::fs::write(dir.join("eyes_open.png"), [4u8, 5]).unwrap();
        std::fs::write(dir.join("unrelated.txt"), "x").unwrap();
        let data = super::read_character_dir(dir.to_string_lossy().into()).unwrap();
        assert_eq!(data.manifest, r#"{"name":"t"}"#);
        assert_eq!(data.layers.len(), 2);
        assert!(data.layers.contains_key("body"));
        assert!(data.layers.contains_key("eyes_open"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_character_dir_without_manifest_errors_clearly() {
        let dir = std::env::temp_dir().join(format!("ocellum-nomanifest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let err = super::read_character_dir(dir.to_string_lossy().into()).unwrap_err();
        assert!(err.contains("character.json"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
