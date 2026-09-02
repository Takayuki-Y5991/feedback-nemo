use rusqlite::{params, Connection};
use serde::Serialize;
use std::{fs, sync::Mutex};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Position, Size, State,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

struct Database(Mutex<Connection>);

#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Could not locate the app data directory")]
    MissingDataDirectory,
    #[error("File operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Validation(String),
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Person {
    id: i64,
    name: String,
    is_active: bool,
    created_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Memo {
    id: i64,
    person_id: i64,
    person_name: String,
    occurred_at: String,
    content: String,
    created_at: String,
    updated_at: String,
}

fn open_database(app: &AppHandle) -> Result<Connection, AppError> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|_| AppError::MissingDataDirectory)?;
    fs::create_dir_all(&directory)?;
    let connection = Connection::open(directory.join("feedback-memo.sqlite3"))?;
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         CREATE TABLE IF NOT EXISTS people (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           name TEXT NOT NULL COLLATE NOCASE UNIQUE,
           is_active INTEGER NOT NULL DEFAULT 1,
           created_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS memos (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           person_id INTEGER NOT NULL REFERENCES people(id),
           occurred_at TEXT NOT NULL,
           content TEXT NOT NULL,
           created_at TEXT NOT NULL,
           updated_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_memos_occurred_at ON memos(occurred_at DESC);
         CREATE INDEX IF NOT EXISTS idx_memos_person_id ON memos(person_id);
         CREATE TABLE IF NOT EXISTS settings (
           key TEXT PRIMARY KEY,
           value TEXT NOT NULL
         );",
    )?;
    Ok(connection)
}

#[tauri::command]
fn get_people(database: State<'_, Database>) -> Result<Vec<Person>, AppError> {
    let connection = database.0.lock().expect("database lock poisoned");
    let mut statement = connection.prepare(
        "SELECT id, name, is_active, created_at FROM people ORDER BY is_active DESC, name ASC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(Person {
            id: row.get(0)?,
            name: row.get(1)?,
            is_active: row.get::<_, i64>(2)? == 1,
            created_at: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

#[tauri::command]
fn add_person(name: String, database: State<'_, Database>) -> Result<Person, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Validation("Enter a display name.".into()));
    }
    if name.chars().count() > 80 {
        return Err(AppError::Validation(
            "Display names must be 80 characters or fewer.".into(),
        ));
    }
    let now = chrono::Utc::now().to_rfc3339();
    let connection = database.0.lock().expect("database lock poisoned");
    connection.execute(
        "INSERT INTO people (name, is_active, created_at) VALUES (?1, 1, ?2)",
        params![name, now],
    )?;
    Ok(Person {
        id: connection.last_insert_rowid(),
        name: name.to_owned(),
        is_active: true,
        created_at: now,
    })
}

#[tauri::command]
fn create_memo(
    person_id: i64,
    occurred_at: String,
    content: String,
    database: State<'_, Database>,
) -> Result<Memo, AppError> {
    let content = content.trim();
    if content.is_empty() {
        return Err(AppError::Validation(
            "Write a short note before saving.".into(),
        ));
    }
    let now = chrono::Utc::now().to_rfc3339();
    let connection = database.0.lock().expect("database lock poisoned");
    let person_name: String = connection.query_row(
        "SELECT name FROM people WHERE id = ?1 AND is_active = 1",
        [person_id],
        |row| row.get(0),
    )?;
    connection.execute(
        "INSERT INTO memos (person_id, occurred_at, content, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![person_id, occurred_at, content, now],
    )?;
    Ok(Memo {
        id: connection.last_insert_rowid(),
        person_id,
        person_name,
        occurred_at,
        content: content.to_owned(),
        created_at: now.clone(),
        updated_at: now,
    })
}

#[tauri::command]
fn get_memos(person_id: Option<i64>, database: State<'_, Database>) -> Result<Vec<Memo>, AppError> {
    let connection = database.0.lock().expect("database lock poisoned");
    let sql =
        "SELECT m.id, m.person_id, p.name, m.occurred_at, m.content, m.created_at, m.updated_at
               FROM memos m JOIN people p ON p.id = m.person_id
               WHERE (?1 IS NULL OR m.person_id = ?1)
               ORDER BY m.occurred_at DESC, m.id DESC";
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map([person_id], |row| {
        Ok(Memo {
            id: row.get(0)?,
            person_id: row.get(1)?,
            person_name: row.get(2)?,
            occurred_at: row.get(3)?,
            content: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

#[tauri::command]
fn delete_memo(id: i64, database: State<'_, Database>) -> Result<(), AppError> {
    let connection = database.0.lock().expect("database lock poisoned");
    connection.execute("DELETE FROM memos WHERE id = ?1", [id])?;
    Ok(())
}

fn position_bottom_right(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let Some(monitor) = window.current_monitor()?.or(window.primary_monitor()?) else {
        return Ok(());
    };
    let monitor_position = monitor.position();
    let monitor_size = monitor.size();
    let scale = monitor.scale_factor();
    let window_size = window.outer_size().unwrap_or(PhysicalSize::new(400, 380));
    let margin = (16.0 * scale) as i32;
    let x = monitor_position.x + monitor_size.width as i32 - window_size.width as i32 - margin;
    let y = monitor_position.y + monitor_size.height as i32 - window_size.height as i32 - margin;
    window.set_position(Position::Physical(PhysicalPosition::new(x, y)))
}

fn restore_saved_position(app: &AppHandle, window: &tauri::WebviewWindow) -> bool {
    let Some(database) = app.try_state::<Database>() else {
        return false;
    };
    let connection = database.0.lock().expect("database lock poisoned");
    let position = connection.query_row(
        "SELECT x.value, y.value
         FROM settings x, settings y
         WHERE x.key = 'window_x' AND y.key = 'window_y'",
        [],
        |row| {
            let x: String = row.get(0)?;
            let y: String = row.get(1)?;
            Ok((x.parse::<i32>().ok(), y.parse::<i32>().ok()))
        },
    );
    match position {
        Ok((Some(x), Some(y))) => window
            .set_position(Position::Physical(PhysicalPosition::new(x, y)))
            .is_ok(),
        _ => false,
    }
}

fn save_window_position(app: &AppHandle, position: &PhysicalPosition<i32>) {
    let Some(database) = app.try_state::<Database>() else {
        return;
    };
    let connection = database.0.lock().expect("database lock poisoned");
    let _ = connection.execute(
        "INSERT INTO settings (key, value) VALUES ('window_x', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [position.x.to_string()],
    );
    let _ = connection.execute(
        "INSERT INTO settings (key, value) VALUES ('window_y', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [position.y.to_string()],
    );
}

fn show_view(app: &AppHandle, mode: &str) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    let is_capture = mode == "capture";
    window.set_always_on_top(is_capture)?;
    window.set_resizable(!is_capture)?;
    if is_capture {
        window.set_size(Size::Physical(PhysicalSize::new(400, 380)))?;
        if !restore_saved_position(app, &window) {
            position_bottom_right(&window)?;
        }
    } else {
        window.set_size(Size::Physical(PhysicalSize::new(940, 680)))?;
        window.center()?;
    }
    window.emit("view-mode", mode)?;
    window.show()?;
    window.set_focus()?;
    Ok(())
}

#[tauri::command]
fn set_view_mode(mode: String, app: AppHandle) -> Result<(), String> {
    match mode.as_str() {
        "capture" | "history" | "settings" => {
            show_view(&app, &mode).map_err(|error| error.to_string())
        }
        _ => Err("Unknown view mode.".into()),
    }
}

#[tauri::command]
fn hide_main_window(app: AppHandle) -> Result<(), String> {
    app.get_webview_window("main")
        .ok_or_else(|| "Window not found.".to_string())?
        .hide()
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn quick_capture_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyM)
}

#[cfg(not(target_os = "macos"))]
fn quick_capture_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyM)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if shortcut == &quick_capture_shortcut()
                        && event.state() == ShortcutState::Pressed
                    {
                        let _ = show_view(app, "capture");
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_people,
            add_person,
            create_memo,
            get_memos,
            delete_memo,
            set_view_mode,
            hide_main_window
        ])
        .setup(|app| {
            let database = open_database(app.handle())?;
            app.manage(Database(Mutex::new(database)));

            let capture = MenuItem::with_id(app, "capture", "New note", true, None::<&str>)?;
            let history = MenuItem::with_id(app, "history", "History", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&capture, &history, &quit])?;

            let mut tray = TrayIconBuilder::new();
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.tooltip("Feedback Memo")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "capture" => {
                        let _ = show_view(app, "capture");
                    }
                    "history" => {
                        let _ = show_view(app, "history");
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let _ = show_view(tray.app_handle(), "capture");
                    }
                })
                .build(app)?;

            app.global_shortcut().register(quick_capture_shortcut())?;
            if let Some(window) = app.get_webview_window("main") {
                if !restore_saved_position(app.handle(), &window) {
                    let _ = position_bottom_right(&window);
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::Moved(position) => {
                let is_quick_panel = window.outer_size().is_ok_and(|size| size.width <= 500);
                if is_quick_panel {
                    save_window_position(window.app_handle(), position);
                }
            }
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running Feedback Memo");
}
