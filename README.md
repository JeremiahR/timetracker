# timetracker

A simple Rust CLI that snapshots your desktop activity every minute via cron. Tracks all open windows, which one is focused (including browser tab titles), and whether your session is locked.

Built for Hyprland on Wayland. Uses SQLite for storage.

## What it captures

- All open windows: class, title, workspace, PID
- The currently focused window
- Session lock state (active vs locked/away)
- Browser tab titles (e.g. "GitHub - Brave")

## Setup

```
cargo build --release
```

Add to crontab (`crontab -e`):

```
* * * * * /path/to/timetracker >> ~/logs/timetracker.log 2>&1
```

The binary auto-detects the Hyprland instance signature at runtime, so it survives reboots without any environment variables in the cron entry.

## Usage

**Snapshot** (what cron runs):
```
timetracker
```

**Daily report**:
```
timetracker report
```

Example output:

```
=== Time Report (last 24h) ===

  From:   2026-03-02 09:01:22
  To:     2026-03-02 17:45:01
  Active: 6h 32m
  Locked: 2h 11m
  Total:  8h 43m

--- Focus time by app ---

  Ghostty              ████████████████████ 3h 15m
  Brave                ██████████ 1h 40m
  VS Code              ██████ 1h 02m
  Slack                ███ 35m

--- What you were doing ---

   98m  Ghostty          Claude Code
   62m  Brave            GitHub - Pull Requests
   45m  VS Code          src/main.rs
   35m  Slack             #engineering

--- Hourly timeline ---

  09:00  ██████████████████████████████░░░░░░░░░░  30m active, 10m locked
  10:00  ████████████████████████████████████████░  39m active, 1m locked
  ...

  █ = active  ░ = locked  (each char = 1 minute)
```

## Data

SQLite database at `~/.local/share/timetracker/timetracker.db`.

Query it directly:

```sql
-- focused window for each snapshot
SELECT s.timestamp, w.class, w.title
FROM snapshots s
JOIN windows w ON w.snapshot_id = s.id
WHERE w.is_focused = 1
ORDER BY s.id DESC
LIMIT 20;
```
