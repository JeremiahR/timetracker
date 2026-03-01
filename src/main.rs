use chrono::Local;
use rusqlite::Connection;
use serde::Deserialize;
use std::process::Command;

#[derive(Deserialize)]
struct HyprWindow {
    address: String,
    class: String,
    title: String,
    workspace: Workspace,
    pid: i64,
}

#[derive(Deserialize)]
struct Workspace {
    id: i64,
}

#[derive(Deserialize)]
struct ActiveWindow {
    address: String,
}

fn main() {
    ensure_hyprland_env();

    let db_dir = dirs();
    std::fs::create_dir_all(&db_dir).expect("failed to create data directory");
    let db_path = format!("{db_dir}/timetracker.db");

    let conn = Connection::open(&db_path).expect("failed to open database");
    create_tables(&conn);

    let windows = get_windows();
    let active_address = get_active_window_address();

    let timestamp = Local::now().to_rfc3339();
    conn.execute("INSERT INTO snapshots (timestamp) VALUES (?1)", [&timestamp])
        .expect("failed to insert snapshot");
    let snapshot_id = conn.last_insert_rowid();

    let mut focused_title = String::from("(none)");
    for w in &windows {
        let is_focused = if w.address == active_address { 1 } else { 0 };
        if is_focused == 1 {
            focused_title = w.title.clone();
        }
        conn.execute(
            "INSERT INTO windows (snapshot_id, class, title, workspace, pid, is_focused) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![snapshot_id, w.class, w.title, w.workspace.id, w.pid, is_focused],
        )
        .expect("failed to insert window");
    }

    println!("[{timestamp}] {count} windows | focused: {focused_title}", count = windows.len());
}

fn ensure_hyprland_env() {
    // Set XDG_RUNTIME_DIR if missing (cron doesn't have it)
    if std::env::var("XDG_RUNTIME_DIR").is_err() {
        let uid = unsafe { libc::getuid() };
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", format!("/run/user/{uid}")) };
    }

    // Auto-detect HYPRLAND_INSTANCE_SIGNATURE from the runtime dir
    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_err() {
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap();
        let hypr_dir = format!("{runtime_dir}/hypr");
        let sig = std::fs::read_dir(&hypr_dir)
            .expect("failed to read hypr runtime dir — is Hyprland running?")
            .filter_map(|e| e.ok())
            .find(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.file_name() != "..")
            .expect("no Hyprland instance found in runtime dir")
            .file_name();
        unsafe { std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", &sig) };
    }
}

fn dirs() -> String {
    std::env::var("XDG_DATA_HOME")
        .unwrap_or_else(|_| format!("{}/.local/share", std::env::var("HOME").unwrap()))
        + "/timetracker"
}

fn create_tables(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS windows (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            snapshot_id INTEGER NOT NULL REFERENCES snapshots(id),
            class TEXT NOT NULL,
            title TEXT NOT NULL,
            workspace INTEGER NOT NULL,
            pid INTEGER NOT NULL,
            is_focused INTEGER NOT NULL DEFAULT 0
        );",
    )
    .expect("failed to create tables");
}

fn get_windows() -> Vec<HyprWindow> {
    let output = Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .expect("failed to run hyprctl clients");
    serde_json::from_slice(&output.stdout).expect("failed to parse hyprctl clients output")
}

fn get_active_window_address() -> String {
    let output = Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()
        .expect("failed to run hyprctl activewindow");
    // activewindow returns empty/error if no window focused
    if output.stdout.is_empty() {
        return String::new();
    }
    serde_json::from_slice::<ActiveWindow>(&output.stdout)
        .map(|w| w.address)
        .unwrap_or_default()
}
