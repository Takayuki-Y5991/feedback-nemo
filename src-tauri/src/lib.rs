use chrono::SecondsFormat;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::{
    fs,
    sync::{Mutex, MutexGuard},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Position, Size,
    State,
};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// Window geometry is declared once, in logical points, so that the size applied
/// at launch from `tauri.conf.json` and the size applied by `show_view` cannot
/// drift apart on a Retina display.
const PANEL_SIZE: LogicalSize<f64> = LogicalSize::new(400.0, 420.0);
const LIBRARY_SIZE: LogicalSize<f64> = LogicalSize::new(940.0, 680.0);
const SCREEN_MARGIN: f64 = 16.0;

/// The two window shapes the app switches between. Capture and Settings are both
/// compact always-on-top panels, so they share one variant; only History needs
/// the roomier shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewMode {
    Panel,
    Library,
}

impl ViewMode {
    fn from_view_name(name: &str) -> Option<Self> {
        match name {
            "capture" | "settings" => Some(Self::Panel),
            "history" => Some(Self::Library),
            _ => None,
        }
    }

    fn logical_size(self) -> LogicalSize<f64> {
        match self {
            Self::Panel => PANEL_SIZE,
            Self::Library => LIBRARY_SIZE,
        }
    }
}

struct CurrentViewMode(Mutex<ViewMode>);

impl CurrentViewMode {
    fn get(&self) -> ViewMode {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set(&self, mode: ViewMode) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = mode;
    }
}

struct Database(Mutex<Connection>);

impl Database {
    /// Returns the pooled connection, recovering from a poisoned lock instead of
    /// panicking: a note that failed to save should never take the app down.
    fn connection(&self) -> MutexGuard<'_, Connection> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Could not locate the app data directory")]
    MissingDataDirectory,
    #[error("File operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Could not build the export: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Could not copy to the clipboard: {0}")]
    Clipboard(String),
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Person {
    id: i64,
    name: String,
    is_active: bool,
    created_at: String,
}

#[derive(Debug, Serialize)]
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

/// A note as it appears inside an export document. `id` and `personId` are
/// deliberately absent: the document is written for people and downstream tools,
/// not for re-import, so internal keys would only be noise.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportedMemo {
    person: String,
    occurred_at: String,
    content: String,
    created_at: String,
    updated_at: String,
}

/// The filters the export was taken with, echoed back so a saved file explains
/// itself. `None` means "no restriction" and serializes to `null`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportFilter {
    person: Option<String>,
    from: Option<String>,
    to: Option<String>,
    /// The keyword as the user typed it (trimmed), not the escaped `LIKE`
    /// pattern: the document explains the search to a reader, it does not
    /// reproduce the SQL.
    query: Option<String>,
}

/// One page of the History list plus the size of the full result set. `total`
/// counts every row matching the filters, ignoring `limit`/`offset`, so the UI
/// can render "N notes" and a page count from a single round trip.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MemoPage {
    memos: Vec<Memo>,
    total: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportDocument {
    exported_at: String,
    filter: ExportFilter,
    count: usize,
    memos: Vec<ExportedMemo>,
}

/// Schema migrations applied in order. The index of a migration plus one is the
/// `PRAGMA user_version` it leaves behind, so never reorder or remove entries.
const MIGRATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS people (
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
    // The original build stored whatever JS `toISOString()` produced
    // ("2026-09-24T08:30:00.000Z") while the current build stores
    // "2026-09-24T08:30:00+00:00". Those two forms do not compare or sort
    // correctly against each other as plain text ('.' is 0x2E, '+' is 0x2B, and
    // 'Z' sorts after every digit), which would silently drop legacy rows from a
    // date-filtered export and already misorders the History list. SQLite's date
    // functions understand both suffixes and answer in UTC, so one UPDATE brings
    // every row into the canonical form.
    //
    // The guard is on the *result* of `strftime`, not on `occurred_at`: an
    // unparseable value makes `strftime` return NULL, and writing that back into
    // a NOT NULL column aborts the whole migration. Rows SQLite cannot parse are
    // therefore left exactly as they are.
    "UPDATE memos
     SET occurred_at = strftime('%Y-%m-%dT%H:%M:%S+00:00', occurred_at)
     WHERE strftime('%Y-%m-%dT%H:%M:%S+00:00', occurred_at) IS NOT NULL;",
];

/// Brings a connection up to the latest schema version. Databases created by the
/// pre-migration build report `user_version = 0` but already own every table, so
/// migration 1 is written with `IF NOT EXISTS` and upgrades them in place.
fn initialize_schema(connection: &Connection) -> Result<(), AppError> {
    let applied: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let applied = applied.max(0) as usize;
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(applied) {
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch(migration)?;
        transaction.pragma_update(None, "user_version", (index + 1) as i64)?;
        transaction.commit()?;
    }
    Ok(())
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
         PRAGMA journal_mode = WAL;",
    )?;
    initialize_schema(&connection)?;
    Ok(connection)
}

/// Accepts an RFC 3339 timestamp from the UI and normalizes it to UTC at second
/// precision so that the `occurred_at DESC` ordering and the range filters
/// compare like-for-like strings.
///
/// Seconds, not sub-seconds: the UI hands us `Date.prototype.toISOString()`
/// output, which always carries milliseconds, and migration 2 rewrites stored
/// timestamps through SQLite's `strftime`, which truncates them. Emitting the
/// same precision here keeps freshly written rows byte-comparable with migrated
/// ones.
fn normalize_occurred_at(occurred_at: &str) -> Result<String, AppError> {
    chrono::DateTime::parse_from_rfc3339(occurred_at.trim())
        .map(|parsed| {
            parsed
                .with_timezone(&chrono::Utc)
                .to_rfc3339_opts(SecondsFormat::Secs, false)
        })
        .map_err(|_| AppError::Validation("Choose a valid date.".into()))
}

fn is_unique_violation(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == rusqlite::ErrorCode::ConstraintViolation
                && failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    )
}

fn list_people(connection: &Connection) -> Result<Vec<Person>, AppError> {
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

fn insert_person(connection: &Connection, name: &str) -> Result<Person, AppError> {
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
    connection
        .execute(
            "INSERT INTO people (name, is_active, created_at) VALUES (?1, 1, ?2)",
            params![name, now],
        )
        .map_err(|error| {
            if is_unique_violation(&error) {
                AppError::Validation("A teammate with that name already exists.".into())
            } else {
                AppError::Database(error)
            }
        })?;
    Ok(Person {
        id: connection.last_insert_rowid(),
        name: name.to_owned(),
        is_active: true,
        created_at: now,
    })
}

fn insert_memo(
    connection: &Connection,
    person_id: i64,
    occurred_at: &str,
    content: &str,
) -> Result<Memo, AppError> {
    let content = content.trim();
    if content.is_empty() {
        return Err(AppError::Validation(
            "Write a short note before saving.".into(),
        ));
    }
    let occurred_at = normalize_occurred_at(occurred_at)?;
    let now = chrono::Utc::now().to_rfc3339();
    let person_name: String = connection
        .query_row(
            "SELECT name FROM people WHERE id = ?1 AND is_active = 1",
            [person_id],
            |row| row.get(0),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                AppError::Validation("Choose an active teammate.".into())
            }
            other => AppError::Database(other),
        })?;
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

/// The character given to SQLite's `ESCAPE` clause. Backslash is not special to
/// SQLite's `LIKE` on its own, which is exactly why it makes a usable escape:
/// nothing else in a pattern can be mistaken for it.
const LIKE_ESCAPE: char = '\\';

/// Wraps `raw` in `%` wildcards for a substring match, neutralising every
/// character that `LIKE` would otherwise read as a wildcard.
///
/// `%`, `_` and the escape character itself are each prefixed with
/// [`LIKE_ESCAPE`], so a search for `50%` looks for the literal three
/// characters rather than "anything starting with 50", and `a_b` does not match
/// `axb`. The surrounding `%` are added after escaping so they stay wildcards.
fn like_pattern(raw: &str) -> String {
    // Every character may need an escape, plus the two wrapping wildcards.
    let mut pattern = String::with_capacity(raw.len() + 2);
    pattern.push('%');
    for character in raw.chars() {
        if matches!(character, '%' | '_' | LIKE_ESCAPE) {
            pattern.push(LIKE_ESCAPE);
        }
        pattern.push(character);
    }
    pattern.push('%');
    pattern
}

/// A person filter, a half-open `[from, to)` window over `occurred_at` with both
/// bounds already normalized into the canonical storage format, and an optional
/// content keyword. Building one is the only way to reach [`select_memos`], so a
/// caller cannot accidentally compare a raw `...Z` string against canonical rows
/// or feed an unescaped keyword to `LIKE`.
#[derive(Debug, Default)]
struct MemoQuery {
    person_id: Option<i64>,
    from: Option<String>,
    to: Option<String>,
    /// The keyword as typed, trimmed; `None` when absent or blank.
    query: Option<String>,
}

impl MemoQuery {
    fn new(
        person_id: Option<i64>,
        from: Option<&str>,
        to: Option<&str>,
        query: Option<&str>,
    ) -> Result<Self, AppError> {
        Ok(Self {
            person_id,
            from: from.map(normalize_occurred_at).transpose()?,
            to: to.map(normalize_occurred_at).transpose()?,
            // A keyword of spaces is the user clearing the box, not a request
            // for notes containing a space.
            query: query
                .map(str::trim)
                .filter(|keyword| !keyword.is_empty())
                .map(str::to_owned),
        })
    }

    /// The `LIKE` pattern bound as `?4`, or `None` for "no keyword filter".
    fn content_pattern(&self) -> Option<String> {
        self.query.as_deref().map(like_pattern)
    }
}

/// The joined, filtered set every memo read shares: the page, its `COUNT(*)`,
/// and the export all run this exact predicate, so they cannot disagree about
/// which notes match.
///
/// `from` is inclusive and `to` is exclusive, so a day-boundary export can use
/// the start of the following day as its upper bound without double-counting.
///
/// The keyword match relies on SQLite's built-in `LIKE`, which folds case for
/// ASCII only. Non-ASCII text — Japanese, for instance — is therefore matched
/// case-sensitively; that is fine, because those scripts have no case, and
/// buying anything better would mean linking ICU.
macro_rules! memo_source {
    () => {
        "FROM memos m JOIN people p ON p.id = m.person_id
         WHERE (?1 IS NULL OR m.person_id = ?1)
           AND (?2 IS NULL OR m.occurred_at >= ?2)
           AND (?3 IS NULL OR m.occurred_at < ?3)
           AND (?4 IS NULL OR m.content LIKE ?4 ESCAPE '\\')"
    };
}

/// Newest first, as the History list shows it. The `m.id DESC` tiebreak is not
/// cosmetic: without it, rows sharing an `occurred_at` have no defined order
/// between them and `OFFSET` paging may show one twice and skip another.
macro_rules! select_memos_sql {
    () => {
        concat!(
            "SELECT m.id, m.person_id, p.name, m.occurred_at, m.content, m.created_at, m.updated_at ",
            memo_source!(),
            " ORDER BY m.occurred_at DESC, m.id DESC"
        )
    };
}

const SELECT_MEMOS: &str = select_memos_sql!();
const SELECT_MEMOS_PAGE: &str = concat!(select_memos_sql!(), " LIMIT ?5 OFFSET ?6");
const COUNT_MEMOS: &str = concat!("SELECT COUNT(*) ", memo_source!());

/// The widest page the UI may ask for. A page is meant to be read, and an
/// unbounded one would pull an entire history across the IPC boundary.
const MAX_PAGE_LIMIT: i64 = 200;

/// Forces `limit` into `1..=MAX_PAGE_LIMIT`. SQLite reads a negative `LIMIT` as
/// "no limit", so passing a caller's value straight through would turn a typo
/// into a full-table dump; zero would return a permanently empty list.
fn clamp_limit(limit: i64) -> i64 {
    limit.clamp(1, MAX_PAGE_LIMIT)
}

fn memo_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memo> {
    Ok(Memo {
        id: row.get(0)?,
        person_id: row.get(1)?,
        person_name: row.get(2)?,
        occurred_at: row.get(3)?,
        content: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn select_memos(connection: &Connection, query: &MemoQuery) -> Result<Vec<Memo>, AppError> {
    let mut statement = connection.prepare(SELECT_MEMOS)?;
    let rows = statement.query_map(
        params![
            query.person_id,
            query.from,
            query.to,
            query.content_pattern()
        ],
        memo_from_row,
    )?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Reads one page and the total match count in a single call, so the UI never
/// has to issue two invokes and risk them disagreeing.
fn select_memo_page(
    connection: &Connection,
    query: &MemoQuery,
    limit: i64,
    offset: i64,
) -> Result<MemoPage, AppError> {
    let limit = clamp_limit(limit);
    let offset = offset.max(0);
    let pattern = query.content_pattern();
    let total: i64 = connection.query_row(
        COUNT_MEMOS,
        params![query.person_id, query.from, query.to, pattern],
        |row| row.get(0),
    )?;
    let mut statement = connection.prepare(SELECT_MEMOS_PAGE)?;
    let rows = statement.query_map(
        params![
            query.person_id,
            query.from,
            query.to,
            pattern,
            limit,
            offset
        ],
        memo_from_row,
    )?;
    Ok(MemoPage {
        memos: rows.collect::<Result<Vec<_>, _>>()?,
        total,
    })
}

/// Looks up the display name for `filter.person`. Unlike note creation this does
/// not require the teammate to still be active: notes about someone who has left
/// remain worth exporting.
fn export_person_name(connection: &Connection, person_id: i64) -> Result<String, AppError> {
    connection
        .query_row(
            "SELECT name FROM people WHERE id = ?1",
            [person_id],
            |row| row.get(0),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                AppError::Validation("That teammate no longer exists.".into())
            }
            other => AppError::Database(other),
        })
}

/// Builds the pretty-printed export document. Takes a bare `&Connection` rather
/// than app state so it is directly testable.
fn build_export(connection: &Connection, query: &MemoQuery) -> Result<String, AppError> {
    let person = query
        .person_id
        .map(|id| export_person_name(connection, id))
        .transpose()?;
    let memos = select_memos(connection, query)?;
    let document = ExportDocument {
        exported_at: chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, false),
        filter: ExportFilter {
            person,
            from: query.from.clone(),
            to: query.to.clone(),
            query: query.query.clone(),
        },
        count: memos.len(),
        memos: memos
            .into_iter()
            .map(|memo| ExportedMemo {
                person: memo.person_name,
                occurred_at: memo.occurred_at,
                content: memo.content,
                created_at: memo.created_at,
                updated_at: memo.updated_at,
            })
            .collect(),
    };
    Ok(serde_json::to_string_pretty(&document)?)
}

fn remove_memo(connection: &Connection, id: i64) -> Result<(), AppError> {
    let affected = connection.execute("DELETE FROM memos WHERE id = ?1", [id])?;
    if affected == 0 {
        return Err(AppError::Validation("That note no longer exists.".into()));
    }
    Ok(())
}

#[tauri::command]
fn get_people(database: State<'_, Database>) -> Result<Vec<Person>, AppError> {
    list_people(&database.connection())
}

#[tauri::command]
fn add_person(name: String, database: State<'_, Database>) -> Result<Person, AppError> {
    insert_person(&database.connection(), &name)
}

#[tauri::command]
fn create_memo(
    person_id: i64,
    occurred_at: String,
    content: String,
    database: State<'_, Database>,
) -> Result<Memo, AppError> {
    insert_memo(&database.connection(), person_id, &occurred_at, &content)
}

/// Returns one page of the History list. Person, period and keyword are all
/// applied in SQL, so the UI holds only the rows it is about to draw.
#[tauri::command]
fn search_memos(
    person_id: Option<i64>,
    from: Option<String>,
    to: Option<String>,
    query: Option<String>,
    limit: i64,
    offset: i64,
    database: State<'_, Database>,
) -> Result<MemoPage, AppError> {
    let memo_query = MemoQuery::new(person_id, from.as_deref(), to.as_deref(), query.as_deref())?;
    select_memo_page(&database.connection(), &memo_query, limit, offset)
}

#[tauri::command]
fn delete_memo(id: i64, database: State<'_, Database>) -> Result<(), AppError> {
    remove_memo(&database.connection(), id)
}

#[tauri::command]
fn export_memos(
    person_id: Option<i64>,
    from: Option<String>,
    to: Option<String>,
    query: Option<String>,
    database: State<'_, Database>,
) -> Result<String, AppError> {
    let memo_query = MemoQuery::new(person_id, from.as_deref(), to.as_deref(), query.as_deref())?;
    build_export(&database.connection(), &memo_query)
}

/// Shows the macOS save panel and writes `contents` to the chosen path.
///
/// Returns `Ok(None)` when the user dismisses the panel: cancelling is a normal
/// outcome, not a failure the UI should surface as an error.
#[tauri::command]
async fn save_export(
    default_file_name: String,
    contents: String,
    app: AppHandle,
) -> Result<Option<String>, AppError> {
    // `blocking_save_file` parks this thread on an `std::sync::mpsc` channel fed
    // by the panel's completion callback. That is safe here and nowhere else:
    // Tauri runs `async` commands off the main thread, so the main thread stays
    // free to actually drive the panel.
    let chosen = app
        .dialog()
        .file()
        .set_file_name(default_file_name)
        .add_filter("JSON", &["json"])
        .blocking_save_file();
    let Some(path) = chosen else {
        return Ok(None);
    };
    let path = path
        .into_path()
        .map_err(|error| AppError::Validation(error.to_string()))?;
    fs::write(&path, contents)?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[tauri::command]
fn copy_to_clipboard(contents: String, app: AppHandle) -> Result<(), AppError> {
    app.clipboard()
        .write_text(contents)
        .map_err(|error| AppError::Clipboard(error.to_string()))
}

/// The compact panel's size in device pixels for a given display scale.
fn quick_panel_physical_size(scale: f64) -> PhysicalSize<u32> {
    PANEL_SIZE.to_physical(scale)
}

fn position_bottom_right(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let Some(monitor) = window.current_monitor()?.or(window.primary_monitor()?) else {
        return Ok(());
    };
    let monitor_position = monitor.position();
    let monitor_size = monitor.size();
    let scale = monitor.scale_factor();
    let window_size = quick_panel_physical_size(scale);
    let margin = (SCREEN_MARGIN * scale).round() as i32;
    let x = monitor_position.x + monitor_size.width as i32 - window_size.width as i32 - margin;
    let y = monitor_position.y + monitor_size.height as i32 - window_size.height as i32 - margin;
    window.set_position(Position::Physical(PhysicalPosition::new(x, y)))
}

/// True when the saved top-left corner still falls inside a connected monitor.
/// Guards against restoring a window onto a display that has been unplugged.
fn is_position_on_a_monitor(window: &tauri::WebviewWindow, x: i32, y: i32) -> bool {
    let Ok(monitors) = window.available_monitors() else {
        return false;
    };
    monitors.iter().any(|monitor| {
        let position = monitor.position();
        let size = monitor.size();
        x >= position.x
            && y >= position.y
            && x < position.x + size.width as i32
            && y < position.y + size.height as i32
    })
}

fn restore_saved_position(app: &AppHandle, window: &tauri::WebviewWindow) -> bool {
    let Some(database) = app.try_state::<Database>() else {
        return false;
    };
    let connection = database.connection();
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
        Ok((Some(x), Some(y))) => {
            if !is_position_on_a_monitor(window, x, y) {
                return false;
            }
            window
                .set_position(Position::Physical(PhysicalPosition::new(x, y)))
                .is_ok()
        }
        _ => false,
    }
}

fn save_window_position(app: &AppHandle, position: &PhysicalPosition<i32>) {
    let Some(database) = app.try_state::<Database>() else {
        return;
    };
    let connection = database.connection();
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
    let view_mode = ViewMode::from_view_name(mode).unwrap_or(ViewMode::Panel);
    // Recorded before any geometry change: centering and positioning both emit
    // `Moved`, and only the compact panel's position may be persisted.
    if let Some(current) = app.try_state::<CurrentViewMode>() {
        current.set(view_mode);
    }
    // Capture and Settings are the same physical panel wearing different
    // contents, so they also share the always-on-top, fixed-size and
    // remembered-position behaviour.
    let is_panel = view_mode == ViewMode::Panel;
    window.set_always_on_top(is_panel)?;
    window.set_resizable(!is_panel)?;
    window.set_size(Size::Logical(view_mode.logical_size()))?;
    if is_panel {
        if !restore_saved_position(app, &window) {
            position_bottom_right(&window)?;
        }
    } else {
        window.center()?;
    }
    window.emit("view-mode", mode)?;
    window.show()?;
    window.set_focus()?;
    Ok(())
}

#[tauri::command]
fn set_view_mode(mode: String, app: AppHandle) -> Result<(), String> {
    if ViewMode::from_view_name(&mode).is_none() {
        return Err("Unknown view mode.".into());
    }
    show_view(&app, &mode).map_err(|error| error.to_string())
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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
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
            search_memos,
            delete_memo,
            export_memos,
            save_export,
            copy_to_clipboard,
            set_view_mode,
            hide_main_window
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let database = open_database(app.handle())?;
            app.manage(Database(Mutex::new(database)));
            // The window created from the config already has the panel shape.
            app.manage(CurrentViewMode(Mutex::new(ViewMode::Panel)));

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

            // Another app may already own the shortcut; that is not fatal, the tray
            // icon still opens the capture window.
            if let Err(error) = app.global_shortcut().register(quick_capture_shortcut()) {
                eprintln!("Feedback Memo: could not register the quick capture shortcut: {error}");
            }
            if let Some(window) = app.get_webview_window("main") {
                if !restore_saved_position(app.handle(), &window) {
                    let _ = position_bottom_right(&window);
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::Moved(position) => {
                let app = window.app_handle();
                // Capture and Settings share one remembered position, which is
                // what we want: they are the same window at the same size, so
                // dragging either one moves "the panel".
                let is_quick_panel = app
                    .try_state::<CurrentViewMode>()
                    .is_some_and(|mode| mode.get() == ViewMode::Panel);
                if is_quick_panel {
                    save_window_position(app, position);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The schema exactly as the pre-migration build wrote it, without a
    /// `user_version` bump.
    const LEGACY_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS people (
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
         );";

    fn user_version(connection: &Connection) -> i64 {
        connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version is readable")
    }

    fn migrated_connection() -> Connection {
        let connection = Connection::open_in_memory().expect("in-memory database opens");
        initialize_schema(&connection).expect("schema initializes");
        connection
    }

    /// Every note, unfiltered and unpaged — the seam that replaced the old
    /// `list_memos` helper once the History list moved to SQL paging.
    fn all_memos(connection: &Connection) -> Vec<Memo> {
        select_memos(connection, &MemoQuery::default()).expect("memos load")
    }

    fn ids_of(memos: &[Memo]) -> Vec<i64> {
        memos.iter().map(|memo| memo.id).collect()
    }

    #[test]
    fn schema_initialization_is_idempotent() {
        let connection = migrated_connection();
        initialize_schema(&connection).expect("second run is a no-op");
        assert_eq!(user_version(&connection), MIGRATIONS.len() as i64);
    }

    #[test]
    fn a_legacy_database_upgrades_without_error() {
        let connection = Connection::open_in_memory().expect("in-memory database opens");
        connection
            .execute_batch(LEGACY_SCHEMA)
            .expect("legacy schema applies");
        assert_eq!(user_version(&connection), 0);

        initialize_schema(&connection).expect("legacy database migrates");
        assert_eq!(user_version(&connection), MIGRATIONS.len() as i64);

        insert_person(&connection, "Ada").expect("the upgraded schema still accepts people");
    }

    #[test]
    fn occurred_at_is_normalized_to_utc() {
        let normalized =
            normalize_occurred_at("2026-09-04T10:30:00+09:00").expect("a valid timestamp parses");
        assert_eq!(normalized, "2026-09-04T01:30:00+00:00");
    }

    #[test]
    fn occurred_at_validation_rejects_garbage() {
        for input in ["", "not a date", "2026-13-45", "04/09/2026"] {
            let error = normalize_occurred_at(input).expect_err("garbage is rejected");
            assert!(
                matches!(error, AppError::Validation(message) if message == "Choose a valid date.")
            );
        }
    }

    #[test]
    fn a_duplicate_name_reports_a_friendly_message() {
        let connection = migrated_connection();
        insert_person(&connection, "Grace").expect("the first insert succeeds");

        let error = insert_person(&connection, "Grace").expect_err("the second insert fails");
        assert!(
            matches!(error, AppError::Validation(message) if message == "A teammate with that name already exists.")
        );
    }

    #[test]
    fn a_memo_for_an_unknown_person_reports_a_friendly_message() {
        let connection = migrated_connection();
        let error = insert_memo(&connection, 404, "2026-09-04T00:00:00Z", "Great demo")
            .expect_err("an unknown person is rejected");
        assert!(
            matches!(error, AppError::Validation(message) if message == "Choose an active teammate.")
        );
    }

    #[test]
    fn deleting_a_missing_note_reports_a_friendly_message() {
        let connection = migrated_connection();
        let person = insert_person(&connection, "Alan").expect("the person is created");
        let memo = insert_memo(
            &connection,
            person.id,
            "2026-09-04T00:00:00Z",
            "  Nice work  ",
        )
        .expect("the memo is created");
        assert_eq!(memo.content, "Nice work");
        assert_eq!(all_memos(&connection).len(), 1);

        remove_memo(&connection, memo.id).expect("the first delete succeeds");
        let error = remove_memo(&connection, memo.id).expect_err("the second delete fails");
        assert!(
            matches!(error, AppError::Validation(message) if message == "That note no longer exists.")
        );
    }

    #[test]
    fn the_quick_panel_scales_with_the_display() {
        assert_eq!(quick_panel_physical_size(1.0), PhysicalSize::new(400, 420));
        assert_eq!(quick_panel_physical_size(2.0), PhysicalSize::new(800, 840));
    }

    #[test]
    fn view_names_map_onto_the_two_window_shapes() {
        assert_eq!(ViewMode::from_view_name("capture"), Some(ViewMode::Panel));
        assert_eq!(ViewMode::from_view_name("settings"), Some(ViewMode::Panel));
        assert_eq!(ViewMode::from_view_name("history"), Some(ViewMode::Library));
        assert_eq!(ViewMode::from_view_name("nonsense"), None);
        assert_eq!(ViewMode::Panel.logical_size(), PANEL_SIZE);
        assert_eq!(ViewMode::Library.logical_size(), LIBRARY_SIZE);
    }

    /// The launch size comes from the config and the re-show size from
    /// `PANEL_SIZE`; if they diverged the panel would look different after
    /// returning from History.
    #[test]
    fn the_configured_window_matches_the_panel_size() {
        const CONFIG: &str = include_str!("../tauri.conf.json");

        fn number_field(source: &str, key: &str) -> f64 {
            let needle = format!("\"{key}\":");
            let start = source.find(&needle).expect("field is present") + needle.len();
            source[start..]
                .trim_start()
                .split([',', '}', '\n'])
                .next()
                .expect("field has a value")
                .trim()
                .parse()
                .expect("field is a number")
        }

        assert_eq!(number_field(CONFIG, "width"), PANEL_SIZE.width);
        assert_eq!(number_field(CONFIG, "height"), PANEL_SIZE.height);
        assert!(number_field(CONFIG, "minWidth") <= PANEL_SIZE.width);
        assert!(number_field(CONFIG, "minHeight") <= PANEL_SIZE.height);
    }

    /// Writes a memo row straight through SQL, bypassing `normalize_occurred_at`,
    /// so a test can reproduce exactly what an older build left on disk.
    fn insert_raw_memo(connection: &Connection, person_id: i64, occurred_at: &str) -> i64 {
        connection
            .execute(
                "INSERT INTO memos (person_id, occurred_at, content, created_at, updated_at)
                 VALUES (?1, ?2, 'note', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
                params![person_id, occurred_at],
            )
            .expect("the raw insert succeeds");
        connection.last_insert_rowid()
    }

    fn occurred_at_of(connection: &Connection, id: i64) -> String {
        connection
            .query_row("SELECT occurred_at FROM memos WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .expect("the row is readable")
    }

    /// A pre-migration database holding every timestamp shape that has ever
    /// reached the column.
    struct LegacyRows {
        connection: Connection,
        /// `toISOString()` output: UTC with milliseconds and a `Z` suffix.
        utc_z: i64,
        /// Already in the format the current build writes.
        canonical: i64,
        /// An instant carried with a non-UTC offset. Plain text comparison
        /// reads its wall-clock digits, which belong to the *following* day.
        offset: i64,
        /// Something SQLite's date functions cannot parse at all.
        garbage: i64,
    }

    fn legacy_rows() -> LegacyRows {
        let connection = Connection::open_in_memory().expect("in-memory database opens");
        connection
            .execute_batch(LEGACY_SCHEMA)
            .expect("legacy schema applies");
        connection
            .execute(
                "INSERT INTO people (name, is_active, created_at)
                 VALUES ('Alex', 1, '2026-01-01T00:00:00+00:00')",
                [],
            )
            .expect("the person is created");
        let person_id = connection.last_insert_rowid();

        // 08:30 UTC, 09:00 UTC, and 17:00 UTC on 2026-09-24.
        let utc_z = insert_raw_memo(&connection, person_id, "2026-09-24T08:30:00.000Z");
        let canonical = insert_raw_memo(&connection, person_id, "2026-09-24T09:00:00+00:00");
        let offset = insert_raw_memo(&connection, person_id, "2026-09-25T02:00:00+09:00");
        let garbage = insert_raw_memo(&connection, person_id, "sometime last spring");
        LegacyRows {
            connection,
            utc_z,
            canonical,
            offset,
            garbage,
        }
    }

    #[test]
    fn migration_two_rewrites_legacy_timestamps_and_leaves_canonical_ones_alone() {
        let rows = legacy_rows();
        let connection = &rows.connection;

        initialize_schema(connection).expect("the legacy database migrates");

        assert_eq!(
            occurred_at_of(connection, rows.utc_z),
            "2026-09-24T08:30:00+00:00"
        );
        assert_eq!(
            occurred_at_of(connection, rows.canonical),
            "2026-09-24T09:00:00+00:00",
            "an already-canonical row is left byte-for-byte alone"
        );
        assert_eq!(
            occurred_at_of(connection, rows.offset),
            "2026-09-24T17:00:00+00:00",
            "a non-UTC offset is folded into UTC"
        );
        // Unparseable text survives untouched rather than being nulled out,
        // which would violate the column's NOT NULL constraint and abort the
        // whole migration.
        assert_eq!(
            occurred_at_of(connection, rows.garbage),
            "sometime last spring"
        );
    }

    /// Text comparison reads `2026-09-25T02:00:00+09:00` as the 25th even though
    /// it is 17:00 on the 24th, so before migration 2 it outranks every genuinely
    /// later row. Afterwards the whole column is one comparable format.
    #[test]
    fn migration_two_repairs_newest_first_ordering() {
        let rows = legacy_rows();
        let connection = &rows.connection;
        let chronological = vec![rows.offset, rows.canonical, rows.utc_z];

        let before: Vec<i64> = ids_of(&all_memos(connection))
            .into_iter()
            .filter(|id| chronological.contains(id))
            .collect();
        // The offset row does land first here, but only by accident of its
        // wall-clock digits; the assertion below is what proves the repair.
        assert_eq!(before, chronological);

        initialize_schema(connection).expect("the legacy database migrates");

        let after: Vec<i64> = ids_of(&all_memos(connection))
            .into_iter()
            .filter(|id| chronological.contains(id))
            .collect();
        assert_eq!(after, chronological, "17:00 > 09:00 > 08:30 on 2026-09-24");
    }

    /// Every legacy shape must still be found by a range filter the UI expresses
    /// in `toISOString()` form. The offset row is the one a naive text
    /// comparison silently drops: its digits say the 25th, its instant says the
    /// 24th.
    #[test]
    fn migration_two_stops_legacy_rows_falling_out_of_a_date_range() {
        let rows = legacy_rows();
        let connection = &rows.connection;
        let day = || {
            MemoQuery::new(
                None,
                Some("2026-09-24T00:00:00.000Z"),
                Some("2026-09-25T00:00:00.000Z"),
                None,
            )
            .expect("the bounds parse")
        };

        let before = select_memos(connection, &day()).expect("memos load before migrating");
        assert!(
            !before.iter().any(|memo| memo.id == rows.offset),
            "the offset row is wrongly excluded before migrating"
        );

        initialize_schema(connection).expect("the legacy database migrates");

        let after: Vec<i64> = select_memos(connection, &day())
            .expect("memos load after migrating")
            .iter()
            .map(|memo| memo.id)
            .collect();
        assert_eq!(after, vec![rows.offset, rows.canonical, rows.utc_z]);
    }

    #[test]
    fn the_range_includes_its_lower_bound_and_excludes_its_upper_bound() {
        let connection = migrated_connection();
        let person = insert_person(&connection, "Alex").expect("the person is created");
        let at_from = insert_memo(&connection, person.id, "2026-07-01T00:00:00Z", "start")
            .expect("the memo is created");
        let inside = insert_memo(&connection, person.id, "2026-08-15T12:00:00Z", "middle")
            .expect("the memo is created");
        let at_to = insert_memo(&connection, person.id, "2026-10-01T00:00:00Z", "end")
            .expect("the memo is created");

        let query = MemoQuery::new(
            None,
            Some("2026-07-01T00:00:00Z"),
            Some("2026-10-01T00:00:00Z"),
            None,
        )
        .expect("the bounds parse");
        let found = ids_of(&select_memos(&connection, &query).expect("memos load"));

        assert_eq!(found, vec![inside.id, at_from.id]);
        assert!(!found.contains(&at_to.id), "the upper bound is exclusive");
    }

    #[test]
    fn the_person_filter_and_the_range_filter_combine() {
        let connection = migrated_connection();
        let alex = insert_person(&connection, "Alex").expect("the person is created");
        let robin = insert_person(&connection, "Robin").expect("the person is created");
        let wanted = insert_memo(&connection, alex.id, "2026-08-10T09:00:00Z", "in range")
            .expect("the memo is created");
        insert_memo(&connection, alex.id, "2026-12-10T09:00:00Z", "out of range")
            .expect("the memo is created");
        insert_memo(
            &connection,
            robin.id,
            "2026-08-11T09:00:00Z",
            "wrong person",
        )
        .expect("the memo is created");

        let query = MemoQuery::new(
            Some(alex.id),
            Some("2026-08-01T00:00:00Z"),
            Some("2026-09-01T00:00:00Z"),
            None,
        )
        .expect("the bounds parse");
        let found = select_memos(&connection, &query).expect("memos load");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, wanted.id);
    }

    #[test]
    fn an_invalid_range_bound_reports_a_friendly_message() {
        let error =
            MemoQuery::new(None, Some("whenever"), None, None).expect_err("garbage is rejected");
        assert!(
            matches!(error, AppError::Validation(message) if message == "Choose a valid date.")
        );
    }

    #[test]
    fn the_export_document_has_the_documented_shape() {
        let connection = migrated_connection();
        let alex = insert_person(&connection, "Alex").expect("the person is created");
        insert_memo(
            &connection,
            alex.id,
            "2026-07-02T09:00:00Z",
            "Shipped the fix",
        )
        .expect("the memo is created");
        insert_memo(
            &connection,
            alex.id,
            "2026-08-02T09:00:00Z",
            "Ran the review",
        )
        .expect("the memo is created");

        let query = MemoQuery::new(
            Some(alex.id),
            Some("2026-07-01T00:00:00Z"),
            Some("2026-10-01T00:00:00Z"),
            None,
        )
        .expect("the bounds parse");
        let json = build_export(&connection, &query).expect("the export builds");
        let document: serde_json::Value =
            serde_json::from_str(&json).expect("the export is valid JSON");

        assert_eq!(document["filter"]["person"], "Alex");
        assert_eq!(document["filter"]["from"], "2026-07-01T00:00:00+00:00");
        assert_eq!(document["filter"]["to"], "2026-10-01T00:00:00+00:00");
        assert!(document["filter"]["query"].is_null());
        assert_eq!(document["count"], 2);
        assert!(document["exportedAt"].is_string());

        let memos = document["memos"].as_array().expect("memos is an array");
        assert_eq!(memos.len(), 2);
        // Newest first, matching the History list.
        assert_eq!(memos[0]["content"], "Ran the review");
        assert_eq!(memos[0]["person"], "Alex");
        assert_eq!(memos[0]["occurredAt"], "2026-08-02T09:00:00+00:00");
        assert!(memos[0]["createdAt"].is_string());
        assert!(memos[0]["updatedAt"].is_string());
        // Internal keys stay out of a document meant for people.
        assert!(memos[0].get("id").is_none());
        assert!(memos[0].get("personId").is_none());
    }

    #[test]
    fn an_unfiltered_export_reports_null_filters() {
        let connection = migrated_connection();
        let alex = insert_person(&connection, "Alex").expect("the person is created");
        insert_memo(
            &connection,
            alex.id,
            "2026-07-02T09:00:00Z",
            "Shipped the fix",
        )
        .expect("the memo is created");

        let json = build_export(&connection, &MemoQuery::default()).expect("the export builds");
        let document: serde_json::Value =
            serde_json::from_str(&json).expect("the export is valid JSON");

        assert!(document["filter"]["person"].is_null());
        assert!(document["filter"]["from"].is_null());
        assert!(document["filter"]["to"].is_null());
        assert!(document["filter"]["query"].is_null());
        assert_eq!(document["count"], 1);
    }

    #[test]
    fn exporting_an_unknown_person_reports_a_friendly_message() {
        let connection = migrated_connection();
        let query = MemoQuery::new(Some(404), None, None, None).expect("the bounds parse");
        let error = build_export(&connection, &query).expect_err("an unknown person is rejected");
        assert!(
            matches!(error, AppError::Validation(message) if message == "That teammate no longer exists.")
        );
    }

    fn keyword_query(keyword: &str) -> MemoQuery {
        MemoQuery::new(None, None, None, Some(keyword)).expect("a keyword needs no parsing")
    }

    fn contents_of(memos: &[Memo]) -> Vec<&str> {
        memos.iter().map(|memo| memo.content.as_str()).collect()
    }

    #[test]
    fn like_pattern_escapes_every_wildcard() {
        assert_eq!(like_pattern("review"), "%review%");
        assert_eq!(like_pattern("50%"), "%50\\%%");
        assert_eq!(like_pattern("a_b"), "%a\\_b%");
        // The escape character itself must be escaped, or a trailing backslash
        // would consume the closing wildcard and change the pattern's meaning.
        assert_eq!(like_pattern("c:\\tmp"), "%c:\\\\tmp%");
        assert_eq!(like_pattern("100%_\\"), "%100\\%\\_\\\\%");
        assert_eq!(like_pattern(""), "%%");
    }

    #[test]
    fn a_literal_percent_or_underscore_is_not_treated_as_a_wildcard() {
        let connection = migrated_connection();
        let person = insert_person(&connection, "Alex").expect("the person is created");
        for content in [
            "shipped 50% of the backlog",
            "shipped 5 of the backlog",
            "a_b naming came up",
            "axb naming came up",
            "raw c:\\tmp path",
            "raw c:tmp path",
        ] {
            insert_memo(&connection, person.id, "2026-08-01T09:00:00Z", content)
                .expect("the memo is created");
        }

        // Unescaped, "50%" would match anything starting with "50" and "a_b"
        // would match "axb"; both of those rows must stay out.
        let percent = select_memos(&connection, &keyword_query("50%")).expect("memos load");
        assert_eq!(contents_of(&percent), vec!["shipped 50% of the backlog"]);

        let underscore = select_memos(&connection, &keyword_query("a_b")).expect("memos load");
        assert_eq!(contents_of(&underscore), vec!["a_b naming came up"]);

        let backslash = select_memos(&connection, &keyword_query("c:\\")).expect("memos load");
        assert_eq!(contents_of(&backslash), vec!["raw c:\\tmp path"]);
    }

    #[test]
    fn a_keyword_matches_a_substring_case_insensitively_for_ascii() {
        let connection = migrated_connection();
        let person = insert_person(&connection, "Alex").expect("the person is created");
        insert_memo(
            &connection,
            person.id,
            "2026-08-01T09:00:00Z",
            "Ran the Design Review today",
        )
        .expect("the memo is created");
        insert_memo(
            &connection,
            person.id,
            "2026-08-02T09:00:00Z",
            "Paired on the parser",
        )
        .expect("the memo is created");

        for keyword in ["review", "REVIEW", "gn Rev"] {
            let found = select_memos(&connection, &keyword_query(keyword)).expect("memos load");
            assert_eq!(
                contents_of(&found),
                vec!["Ran the Design Review today"],
                "{keyword} should match"
            );
        }
    }

    /// Japanese has no case, so SQLite's ASCII-only `LIKE` folding costs nothing
    /// here; a substring match still has to work.
    #[test]
    fn a_non_ascii_keyword_still_matches_as_a_substring() {
        let connection = migrated_connection();
        let person = insert_person(&connection, "Alex").expect("the person is created");
        insert_memo(
            &connection,
            person.id,
            "2026-08-01T09:00:00Z",
            "設計レビューがとても良かった",
        )
        .expect("the memo is created");
        insert_memo(&connection, person.id, "2026-08-02T09:00:00Z", "朝会の共有")
            .expect("the memo is created");

        let found = select_memos(&connection, &keyword_query("レビュー")).expect("memos load");
        assert_eq!(contents_of(&found), vec!["設計レビューがとても良かった"]);
    }

    #[test]
    fn a_blank_keyword_does_not_filter() {
        for blank in ["", "   ", "\t\n"] {
            let query = MemoQuery::new(None, None, None, Some(blank)).expect("a blank is accepted");
            assert!(query.query.is_none(), "{blank:?} should clear the filter");
            assert!(query.content_pattern().is_none());
        }
        // Surrounding whitespace is noise from the search box, not part of the
        // keyword.
        let trimmed = MemoQuery::new(None, None, None, Some("  review  "))
            .expect("a padded keyword is accepted");
        assert_eq!(trimmed.query.as_deref(), Some("review"));
    }

    #[test]
    fn the_keyword_person_and_range_filters_combine() {
        let connection = migrated_connection();
        let alex = insert_person(&connection, "Alex").expect("the person is created");
        let robin = insert_person(&connection, "Robin").expect("the person is created");
        let wanted = insert_memo(
            &connection,
            alex.id,
            "2026-08-10T09:00:00Z",
            "Great design review",
        )
        .expect("the memo is created");
        insert_memo(
            &connection,
            alex.id,
            "2026-08-11T09:00:00Z",
            "Great standup notes",
        )
        .expect("the keyword misses this one");
        insert_memo(
            &connection,
            alex.id,
            "2026-12-11T09:00:00Z",
            "Great design review",
        )
        .expect("the range misses this one");
        insert_memo(
            &connection,
            robin.id,
            "2026-08-12T09:00:00Z",
            "Great design review",
        )
        .expect("the person filter misses this one");

        let query = MemoQuery::new(
            Some(alex.id),
            Some("2026-08-01T00:00:00Z"),
            Some("2026-09-01T00:00:00Z"),
            Some("design review"),
        )
        .expect("the bounds parse");
        let found = select_memos(&connection, &query).expect("memos load");

        assert_eq!(ids_of(&found), vec![wanted.id]);

        // The paged read applies exactly the same predicate.
        let page = select_memo_page(&connection, &query, 50, 0).expect("the page loads");
        assert_eq!(page.total, 1);
        assert_eq!(ids_of(&page.memos), vec![wanted.id]);
    }

    /// Seeds `count` notes for one person, every one of them at the same
    /// instant. Returns the ids in the order the History list must produce them.
    fn seed_same_instant_memos(connection: &Connection, person_id: i64, count: usize) -> Vec<i64> {
        let mut ids: Vec<i64> = (0..count)
            .map(|index| {
                insert_memo(
                    connection,
                    person_id,
                    "2026-08-10T09:00:00Z",
                    &format!("note {index}"),
                )
                .expect("the memo is created")
                .id
            })
            .collect();
        // Equal timestamps, so the `m.id DESC` tiebreak decides: newest id first.
        ids.reverse();
        ids
    }

    #[test]
    fn total_counts_every_match_while_the_page_respects_the_limit() {
        let connection = migrated_connection();
        let person = insert_person(&connection, "Alex").expect("the person is created");
        seed_same_instant_memos(&connection, person.id, 7);
        insert_memo(&connection, person.id, "2026-08-11T09:00:00Z", "unrelated")
            .expect("the memo is created");

        let page =
            select_memo_page(&connection, &keyword_query("note"), 3, 0).expect("the page loads");
        assert_eq!(page.memos.len(), 3, "the page is capped by the limit");
        assert_eq!(page.total, 7, "total ignores the limit and the offset");

        // A window past the end is empty but still reports the full count, so
        // the UI can tell "no results" from "you walked off the last page".
        let beyond =
            select_memo_page(&connection, &keyword_query("note"), 3, 90).expect("the page loads");
        assert!(beyond.memos.is_empty());
        assert_eq!(beyond.total, 7);
    }

    /// The classic `OFFSET` paging bug: rows that tie on the `ORDER BY` key have
    /// no defined order between pages, so one can be served twice while another
    /// is never served at all. The `m.id DESC` tiebreak is what prevents it.
    #[test]
    fn paging_across_duplicate_timestamps_yields_each_row_exactly_once() {
        let connection = migrated_connection();
        let person = insert_person(&connection, "Alex").expect("the person is created");
        // Nine notes sharing one instant, bracketed by rows on either side so
        // the page boundaries fall inside the tied block.
        let newer = insert_memo(&connection, person.id, "2026-08-11T09:00:00Z", "newer")
            .expect("the memo is created");
        let tied = seed_same_instant_memos(&connection, person.id, 9);
        let older = insert_memo(&connection, person.id, "2026-08-09T09:00:00Z", "older")
            .expect("the memo is created");

        let mut expected = vec![newer.id];
        expected.extend(&tied);
        expected.push(older.id);
        assert_eq!(
            ids_of(&all_memos(&connection)),
            expected,
            "the unpaged read is the reference order"
        );

        const PAGE: i64 = 2;
        let mut paged = Vec::new();
        let mut offset = 0;
        loop {
            let page = select_memo_page(&connection, &MemoQuery::default(), PAGE, offset)
                .expect("the page loads");
            assert_eq!(page.total, expected.len() as i64, "total is page-invariant");
            if page.memos.is_empty() {
                break;
            }
            paged.extend(ids_of(&page.memos));
            offset += PAGE;
        }

        assert_eq!(
            paged, expected,
            "no row is skipped or repeated across a page boundary"
        );
        let unique: std::collections::BTreeSet<i64> = paged.iter().copied().collect();
        assert_eq!(unique.len(), paged.len(), "no id appears twice");
    }

    #[test]
    fn the_page_window_is_clamped_to_a_sane_range() {
        assert_eq!(clamp_limit(0), 1, "an empty page would never load");
        assert_eq!(
            clamp_limit(-1),
            1,
            "SQLite reads a negative LIMIT as no limit at all"
        );
        assert_eq!(clamp_limit(i64::MIN), 1);
        assert_eq!(clamp_limit(1), 1);
        assert_eq!(clamp_limit(50), 50);
        assert_eq!(clamp_limit(MAX_PAGE_LIMIT), MAX_PAGE_LIMIT);
        assert_eq!(clamp_limit(10_000), MAX_PAGE_LIMIT);
        assert_eq!(clamp_limit(i64::MAX), MAX_PAGE_LIMIT);
    }

    #[test]
    fn a_hostile_limit_or_offset_cannot_dump_the_table() {
        let connection = migrated_connection();
        let person = insert_person(&connection, "Alex").expect("the person is created");
        let ids = seed_same_instant_memos(&connection, person.id, 5);

        // Without the clamp this would return all five rows.
        let negative_limit =
            select_memo_page(&connection, &MemoQuery::default(), -1, 0).expect("the page loads");
        assert_eq!(ids_of(&negative_limit.memos), vec![ids[0]]);
        assert_eq!(negative_limit.total, 5);

        let zero_limit =
            select_memo_page(&connection, &MemoQuery::default(), 0, 0).expect("the page loads");
        assert_eq!(zero_limit.memos.len(), 1);

        // A negative offset is treated as the first page rather than being
        // handed to SQLite, which would reject it.
        let negative_offset =
            select_memo_page(&connection, &MemoQuery::default(), 2, -7).expect("the page loads");
        assert_eq!(ids_of(&negative_offset.memos), vec![ids[0], ids[1]]);

        let huge_limit = select_memo_page(&connection, &MemoQuery::default(), i64::MAX, 0)
            .expect("the page loads");
        assert_eq!(ids_of(&huge_limit.memos), ids);
    }

    #[test]
    fn the_export_honours_the_keyword_and_echoes_it_back() {
        let connection = migrated_connection();
        let alex = insert_person(&connection, "Alex").expect("the person is created");
        insert_memo(
            &connection,
            alex.id,
            "2026-07-02T09:00:00Z",
            "Ran the design review",
        )
        .expect("the memo is created");
        insert_memo(
            &connection,
            alex.id,
            "2026-08-02T09:00:00Z",
            "Design review follow-up",
        )
        .expect("the memo is created");
        insert_memo(
            &connection,
            alex.id,
            "2026-08-03T09:00:00Z",
            "Standup notes",
        )
        .expect("the memo is created");

        let query = MemoQuery::new(None, None, None, Some("  design review  "))
            .expect("the keyword is accepted");
        let json = build_export(&connection, &query).expect("the export builds");
        let document: serde_json::Value =
            serde_json::from_str(&json).expect("the export is valid JSON");

        // The trimmed keyword, not the `%design review%` pattern.
        assert_eq!(document["filter"]["query"], "design review");
        assert!(document["filter"]["person"].is_null());
        assert_eq!(document["count"], 2);

        let memos = document["memos"].as_array().expect("memos is an array");
        assert_eq!(memos.len(), 2);
        assert_eq!(memos[0]["content"], "Design review follow-up");
        assert_eq!(memos[1]["content"], "Ran the design review");
    }

    /// The export runs the shared predicate without a `LIMIT`, so a keyword that
    /// matches more than one page still writes every row.
    #[test]
    fn the_export_is_not_limited_to_one_page() {
        let connection = migrated_connection();
        let person = insert_person(&connection, "Alex").expect("the person is created");
        seed_same_instant_memos(&connection, person.id, (MAX_PAGE_LIMIT + 5) as usize);

        let query = keyword_query("note");
        let page =
            select_memo_page(&connection, &query, MAX_PAGE_LIMIT, 0).expect("the page loads");
        assert_eq!(page.memos.len() as i64, MAX_PAGE_LIMIT);
        assert_eq!(page.total, MAX_PAGE_LIMIT + 5);

        let document: serde_json::Value =
            serde_json::from_str(&build_export(&connection, &query).expect("the export builds"))
                .expect("the export is valid JSON");
        assert_eq!(document["count"], MAX_PAGE_LIMIT + 5);
    }
}
