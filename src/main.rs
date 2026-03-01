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
    let args: Vec<String> = std::env::args().collect();

    let db_dir = dirs();
    std::fs::create_dir_all(&db_dir).expect("failed to create data directory");
    let db_path = format!("{db_dir}/timetracker.db");

    let conn = Connection::open(&db_path).expect("failed to open database");
    create_tables(&conn);

    match args.get(1).map(|s| s.as_str()) {
        Some("report") => report(&conn),
        Some(other) => {
            eprintln!("unknown command: {other}");
            eprintln!("usage: timetracker [report]");
            std::process::exit(1);
        }
        None => snapshot(&conn),
    }
}

fn snapshot(conn: &Connection) {
    ensure_hyprland_env();

    let windows = get_windows();
    let active_address = get_active_window_address();
    let locked = is_session_locked();

    let timestamp = Local::now().to_rfc3339();
    conn.execute(
        "INSERT INTO snapshots (timestamp, locked) VALUES (?1, ?2)",
        rusqlite::params![timestamp, locked as i32],
    )
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

    let status = if locked { "locked" } else { "active" };
    println!(
        "[{timestamp}] {count} windows | {status} | focused: {focused_title}",
        count = windows.len()
    );
}

fn report(conn: &Connection) {
    let since = (Local::now() - chrono::Duration::hours(24)).to_rfc3339();

    // Total snapshots and locked/active breakdown
    let (total, active, locked): (i64, i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), SUM(CASE WHEN locked = 0 THEN 1 ELSE 0 END), SUM(CASE WHEN locked = 1 THEN 1 ELSE 0 END) FROM snapshots WHERE timestamp > ?1",
            [&since],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap_or((0, 0, 0));

    if total == 0 {
        println!("No data in the last 24 hours.");
        return;
    }

    // Time range
    let (first_ts, last_ts): (String, String) = conn
        .query_row(
            "SELECT MIN(timestamp), MAX(timestamp) FROM snapshots WHERE timestamp > ?1",
            [&since],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    println!("=== Time Report (last 24h) ===\n");
    println!(
        "  From:   {}",
        &first_ts[..19].replace('T', " ")
    );
    println!(
        "  To:     {}",
        &last_ts[..19].replace('T', " ")
    );
    println!(
        "  Active: {}",
        format_minutes(active)
    );
    println!(
        "  Locked: {}",
        format_minutes(locked)
    );
    println!(
        "  Total:  {}",
        format_minutes(total)
    );

    // Focused time by app class
    println!("\n--- Focus time by app ---\n");

    let mut stmt = conn
        .prepare(
            "SELECT w.class, COUNT(*) as mins
             FROM windows w
             JOIN snapshots s ON s.id = w.snapshot_id
             WHERE w.is_focused = 1 AND s.timestamp > ?1 AND s.locked = 0
             GROUP BY w.class
             ORDER BY mins DESC",
        )
        .unwrap();

    let app_rows: Vec<(String, i64)> = stmt
        .query_map([&since], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    let max_mins = app_rows.first().map(|(_, m)| *m).unwrap_or(1);

    for (class, mins) in &app_rows {
        let bar_len = (*mins as f64 / max_mins as f64 * 20.0) as usize;
        let bar: String = "█".repeat(bar_len);
        let name = pretty_class(class);
        println!("  {name:<20} {bar} {time}", time = format_minutes(*mins));
    }

    // Top focused window titles (what you were actually doing)
    println!("\n--- What you were doing ---\n");

    let mut stmt = conn
        .prepare(
            "SELECT w.class, w.title, COUNT(*) as mins
             FROM windows w
             JOIN snapshots s ON s.id = w.snapshot_id
             WHERE w.is_focused = 1 AND s.timestamp > ?1 AND s.locked = 0
             GROUP BY w.class, w.title
             ORDER BY mins DESC
             LIMIT 15",
        )
        .unwrap();

    let title_rows: Vec<(String, String, i64)> = stmt
        .query_map([&since], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    for (class, title, mins) in &title_rows {
        let name = pretty_class(class);
        let display_title = truncate(title, 60);
        println!("  {mins:>3}m  {name:<16} {display_title}");
    }

    // Timeline: hour-by-hour
    println!("\n--- Hourly timeline ---\n");

    let mut stmt = conn
        .prepare(
            "SELECT substr(timestamp, 12, 2) as hour,
                    SUM(CASE WHEN locked = 0 THEN 1 ELSE 0 END) as active,
                    SUM(CASE WHEN locked = 1 THEN 1 ELSE 0 END) as locked
             FROM snapshots
             WHERE timestamp > ?1
             GROUP BY hour
             ORDER BY hour",
        )
        .unwrap();

    let hour_rows: Vec<(String, i64, i64)> = stmt
        .query_map([&since], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    for (hour, active, locked) in &hour_rows {
        let active_bar: String = "█".repeat(*active as usize);
        let locked_bar: String = "░".repeat(*locked as usize);
        println!("  {hour}:00  {active_bar}{locked_bar}  {active}m active, {locked}m locked");
    }

    println!("\n  █ = active  ░ = locked  (each char = 1 minute)");
}

fn format_minutes(mins: i64) -> String {
    let h = mins / 60;
    let m = mins % 60;
    if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m")
    }
}

fn pretty_class(class: &str) -> String {
    match class {
        "com.mitchellh.ghostty" => "Ghostty".to_string(),
        "brave-browser" => "Brave".to_string(),
        "firefox" => "Firefox".to_string(),
        "google-chrome" => "Chrome".to_string(),
        "chromium" => "Chromium".to_string(),
        "code" | "Code" => "VS Code".to_string(),
        "obsidian" => "Obsidian".to_string(),
        "Slack" | "slack" => "Slack".to_string(),
        "discord" => "Discord".to_string(),
        "spotify" | "Spotify" => "Spotify".to_string(),
        "org.telegram.desktop" => "Telegram".to_string(),
        other => other.to_string(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

fn is_session_locked() -> bool {
    let output = Command::new("loginctl")
        .args(["show-session", "auto", "--property=LockedHint", "--value"])
        .output();
    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim() == "yes",
        Err(_) => false,
    }
}

fn ensure_hyprland_env() {
    if std::env::var("XDG_RUNTIME_DIR").is_err() {
        let uid = unsafe { libc::getuid() };
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", format!("/run/user/{uid}")) };
    }

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
            timestamp TEXT NOT NULL,
            locked INTEGER NOT NULL DEFAULT 0
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

    // Migrate: add locked column if missing (existing DBs from before this change)
    let has_locked: bool = conn
        .prepare("SELECT locked FROM snapshots LIMIT 0")
        .is_ok();
    if !has_locked {
        conn.execute_batch("ALTER TABLE snapshots ADD COLUMN locked INTEGER NOT NULL DEFAULT 0;")
            .expect("failed to add locked column");
    }
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
    if output.stdout.is_empty() {
        return String::new();
    }
    serde_json::from_slice::<ActiveWindow>(&output.stdout)
        .map(|w| w.address)
        .unwrap_or_default()
}
