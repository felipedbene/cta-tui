//! CTA Track Grid — terminal edition.
//!
//! Env (also read from a gitignored `.env` at the repo root, if present):
//!   CTA_KEY        Train Tracker API key (required)
//!   CTA_ROUTES     comma routes (default: red,blue,brn,g,org,p,pink,y)
//!   CTA_HOME_MAPID home station map id (default: 41070 = Kedzie/Green)
//!   CTA_HOME_NAME  label for the home panel (default: Kedzie)
//!   CTA_REFRESH    seconds between polls (default: 30)

mod ai;
mod app;
mod cta;
mod notify;
mod store;
mod track;
mod tts;
mod ui;

use anyhow::Result;
use app::App;
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use cta::{Cta, Snapshot};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::time::Duration;
use tokio::sync::mpsc;

enum Msg {
    Snap(Snapshot),
    Ai(store::AiState),
    Frames(Vec<cta::Frame>),
    ReplaySnap(cta::HistSnap),
}

/// Fetch the replay frame index (last 30 days) and post it to the UI.
fn fetch_replay_index(tx: mpsc::Sender<Msg>, http: reqwest::Client, base: String) {
    tokio::spawn(async move {
        let now = chrono::Utc::now().timestamp_millis();
        let from = now - 30 * 24 * 3600 * 1000;
        if let Ok(frames) = cta::history_index(&http, &base, from, now).await {
            let _ = tx.send(Msg::Frames(frames)).await;
        }
    });
}

/// Fetch one historical snapshot by frame id and post it to the UI.
fn fetch_replay_snapshot(tx: mpsc::Sender<Msg>, http: reqwest::Client, base: String, id: i64) {
    tokio::spawn(async move {
        if let Ok(hist) = cta::history_snapshot(&http, &base, id).await {
            let _ = tx.send(Msg::ReplaySnap(hist)).await;
        }
    });
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load a local .env (gitignored) if present, so CTA_KEY and friends can live
    // in a file instead of the shell. Real environment vars still win.
    let _ = dotenvy::dotenv();

    // Background daemon: poll the Worker's AI endpoints into the local SQLite
    // cache. No terminal, no CTA_KEY needed.  `CTA_DAEMON=1 cta-tui`
    if std::env::var("CTA_DAEMON").is_ok() {
        let home_mapid = std::env::var("CTA_HOME_MAPID").unwrap_or_else(|_| "41070".into());
        let home_name = std::env::var("CTA_HOME_NAME").unwrap_or_else(|_| "Kedzie".into());
        return daemon_loop(home_mapid, home_name).await;
    }

    let key = std::env::var("CTA_KEY").unwrap_or_default();
    if key.trim().is_empty() {
        eprintln!(
            "CTA_KEY is not set.\nGet a free key at \
             https://www.transitchicago.com/developers/traintrackerapply/\n\
             then:  CTA_KEY=xxxx cargo run"
        );
        std::process::exit(2);
    }

    let routes: Vec<String> = std::env::var("CTA_ROUTES")
        .unwrap_or_else(|_| "red,blue,brn,g,org,p,pink,y".into())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let home_mapid = std::env::var("CTA_HOME_MAPID").unwrap_or_else(|_| "41070".into());
    let home_name = std::env::var("CTA_HOME_NAME").unwrap_or_else(|_| "Kedzie".into());
    let refresh: u64 = std::env::var("CTA_REFRESH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let alert_min: i64 = std::env::var("CTA_ALERT_MIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);
    // Desktop delay notifications: on unless CTA_NOTIFY is 0/false/off.
    let notify_enabled = !matches!(
        std::env::var("CTA_NOTIFY").unwrap_or_default().to_lowercase().as_str(),
        "0" | "false" | "off"
    );
    // Orientation: auto by terminal width unless CTA_VERTICAL is explicitly set
    // (then forced vertical, or horizontal for 0/false/off). `v` cycles at runtime.
    let orient_override: Option<bool> = std::env::var("CTA_VERTICAL")
        .ok()
        .map(|v| !matches!(v.to_lowercase().as_str(), "0" | "false" | "off"));

    // Headless probe: one snapshot to stdout, no terminal. `CTA_PROBE=1 cargo run`.
    if std::env::var("CTA_PROBE").is_ok() {
        let cta = Cta::new(key);
        let refs: Vec<&str> = routes.iter().map(String::as_str).collect();
        let snap = cta.snapshot(&refs, &home_mapid).await;
        println!("updated   {}", snap.updated);
        println!("error     {:?}", snap.error);
        println!("statuses  {}", snap.statuses.len());
        for b in &snap.boards {
            println!("board {:<6} {:>3} trains", b.key, b.trains.len());
            for t in b.trains.iter().take(3) {
                println!(
                    "  #{:<4} eta={:?} app={} dly={} hd={:?} -> {}",
                    t.run, t.eta_min, t.approaching, t.delayed, t.heading, t.next_station
                );
            }
        }
        println!("arrivals @ {} = {}", home_name, snap.arrivals.len());
        for a in snap.arrivals.iter().take(5) {
            println!("  {:<4} eta={:?} -> {}", a.route, a.eta_min, a.dest);
        }
        return Ok(());
    }

    // Render-dump: draw one real frame into an off-screen buffer and print it as
    // text, so the layout can be inspected without a TTY. `CTA_RENDER=1 cargo run`.
    if std::env::var("CTA_RENDER").is_ok() {
        let cta = Cta::new(key);
        let refs: Vec<&str> = routes.iter().map(String::as_str).collect();
        let snap = cta.snapshot(&refs, &home_mapid).await;
        let mut app = App::new(home_name, alert_min, false); // no notifications in probe
        app.apply(snap);
        // Load the AI cache so the dispatch bar / intel panel render off-screen too.
        if let Ok(conn) = store::open() {
            app.set_ai(store::read_all(&conn));
        }
        if std::env::var("CTA_INTEL").is_ok() {
            app.show_ai = true;
        }
        if std::env::var("CTA_LOOP").is_ok() {
            app.show_loop = true;
        }
        // Exercise the replay backend off-screen: pull the frame index and the
        // latest snapshot from the Worker, then draw the frozen frame.
        if std::env::var("CTA_REPLAY").is_ok() {
            app.toggle_replay();
            let http = reqwest::Client::new();
            let base = ai::base();
            let now = chrono::Utc::now().timestamp_millis();
            if let Ok(frames) = cta::history_index(&http, &base, now - 6 * 3600 * 1000, now).await {
                if let Some(id) = app.set_replay_frames(frames) {
                    if let Ok(hist) = cta::history_snapshot(&http, &base, id).await {
                        app.set_replay_snap(hist);
                    }
                }
            }
        }
        // Drive search/zoom for off-screen visual checks.
        if let Ok(q) = std::env::var("CTA_SEARCH") {
            app.open_search();
            for c in q.chars() { app.search_input(c); }
        } else if let Ok(q) = std::env::var("CTA_ZOOM") {
            app.open_search();
            for c in q.chars() { app.search_input(c); }
            app.commit_search();
        }
        if std::env::var("CTA_ALERTS").is_ok() {
            app.show_alerts = true;
        }
        app.orient_override = orient_override; // honor CTA_VERTICAL in render dumps too
        if std::env::var("CTA_VERT").is_ok() {
            app.orient_override = Some(true);
        }
        let w: u16 = std::env::var("CTA_COLS").ok().and_then(|v| v.parse().ok()).unwrap_or(110);
        let h: u16 = std::env::var("CTA_ROWS").ok().and_then(|v| v.parse().ok()).unwrap_or(26);
        let backend = ratatui::backend::TestBackend::new(w, h);
        let mut term = Terminal::new(backend)?;
        term.draw(|f| ui::draw(f, &app))?;
        let buf = term.backend().buffer();
        for y in 0..h {
            let mut row = String::new();
            for x in 0..w {
                row.push_str(buf[(x, y)].symbol());
            }
            println!("{}", row.trim_end());
        }
        return Ok(());
    }

    // --- poller task ---
    let (tx, mut rx) = mpsc::channel::<Msg>(8);
    let (refresh_tx, mut refresh_rx) = mpsc::channel::<()>(1);
    {
        let tx = tx.clone();
        let cta = Cta::new(key);
        let route_refs: Vec<String> = routes.clone();
        let home_mapid = home_mapid.clone();
        tokio::spawn(async move {
            let refs: Vec<&str> = route_refs.iter().map(String::as_str).collect();
            let mut tick = tokio::time::interval(Duration::from_secs(refresh));
            loop {
                let snap = cta.snapshot(&refs, &home_mapid).await;
                if tx.send(Msg::Snap(snap)).await.is_err() {
                    break;
                }
                tokio::select! {
                    _ = tick.tick() => {}
                    _ = refresh_rx.recv() => {}
                }
            }
        });
    }

    // --- AI cache reader: every few seconds, read the local SQLite the daemon
    // keeps fresh and push the latest AI text into the UI. ---
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(3));
            loop {
                tick.tick().await;
                let ai = tokio::task::spawn_blocking(|| {
                    store::open().ok().map(|c| store::read_all(&c))
                })
                .await
                .ok()
                .flatten();
                if let Some(ai) = ai {
                    if tx.send(Msg::Ai(ai)).await.is_err() {
                        break;
                    }
                }
            }
        });
    }

    // Auto-manage the AI daemon: spawn it detached if the cache has no fresh
    // heartbeat (so the user just runs `cta-tui` and AI appears).
    ensure_daemon();

    // Restore the terminal on panic (runs even under panic=abort) so a crash
    // never leaves the user in raw mode / the alternate screen.
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        orig_hook(info);
    }));

    // --- terminal ---
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let res = run(&mut terminal, &mut rx, tx.clone(), refresh_tx, home_name, alert_min, notify_enabled, orient_override, refresh).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    res
}

async fn run<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    rx: &mut mpsc::Receiver<Msg>,
    tx: mpsc::Sender<Msg>,
    refresh_tx: mpsc::Sender<()>,
    home_name: String,
    alert_min: i64,
    notify_enabled: bool,
    orient_override: Option<bool>,
    refresh: u64,
) -> Result<()> {
    let mut app = App::new(home_name, alert_min, notify_enabled);
    app.orient_override = orient_override;
    app.refresh_secs = refresh;
    // Replay backend: a shared HTTP client + Worker base for history fetches.
    let http = reqwest::Client::new();
    let base = ai::base();
    let mut events = EventStream::new();
    // ~4 fps render tick so the radar sweep + APP/DLY blink stay alive between polls.
    let mut frame = tokio::time::interval(Duration::from_millis(250));
    terminal.draw(|f| ui::draw(f, &app))?;

    loop {
        tokio::select! {
            _ = frame.tick() => {
                app.tick();
            }
            Some(msg) = rx.recv() => {
                match msg {
                    Msg::Ai(ai) => app.set_ai(ai),
                    Msg::Frames(frames) => {
                        // Index arrived → load the latest frame's snapshot.
                        if let Some(id) = app.set_replay_frames(frames) {
                            fetch_replay_snapshot(tx.clone(), http.clone(), base.clone(), id);
                        }
                    }
                    Msg::ReplaySnap(hist) => app.set_replay_snap(hist),
                    Msg::Snap(snap) => {
                        app.apply(snap);
                        if app.take_bell() {
                            use std::io::Write;
                            let mut out = std::io::stdout();
                            let _ = out.write_all(b"\x07"); // terminal bell on a fresh approach
                            let _ = out.flush();
                        }
                        let notes = app.take_notes();
                        if !notes.is_empty() {
                            // One notification per poll; cap the body so a meltdown can't spam it.
                            let shown: Vec<&str> = notes.iter().take(4).map(String::as_str).collect();
                            let extra = notes.len().saturating_sub(shown.len());
                            let mut body = shown.join("\n");
                            if extra > 0 {
                                body.push_str(&format!("\n(+{extra} more)"));
                            }
                            notify::send("CTA Track Grid — Delays", &body);
                        }
                    }
                }
            }
            Some(Ok(ev)) = events.next() => {
                if let Event::Key(k) = ev {
                    if k.kind == KeyEventKind::Press {
                        if app.search.is_some() {
                            // Search overlay captures all keys.
                            match k.code {
                                KeyCode::Esc => app.close_search(),
                                KeyCode::Enter => app.commit_search(),
                                KeyCode::Up => app.search_move(-1),
                                KeyCode::Down => app.search_move(1),
                                KeyCode::Backspace => app.search_backspace(),
                                KeyCode::Char(c) => app.search_input(c),
                                _ => {}
                            }
                        } else {
                            match k.code {
                                KeyCode::Char('q') => app.should_quit = true,
                                KeyCode::Esc => {
                                    if app.show_loop { app.show_loop = false; }
                                    else if app.show_ai { app.show_ai = false; }
                                    else if app.show_alerts { app.show_alerts = false; }
                                    else if app.is_replaying() { app.exit_replay(); }
                                    else if app.zoom.is_some() { app.clear_zoom(); }
                                    else { app.should_quit = true; }
                                }
                                KeyCode::Char('/') => app.open_search(),
                                KeyCode::Char('a') => app.toggle_alerts(),
                                KeyCode::Char('i') => app.toggle_ai(),
                                KeyCode::Char('l') => app.toggle_loop(),
                                KeyCode::Char('s') => app.toggle_voice(),
                                KeyCode::Char('v') => app.toggle_vertical(),
                                KeyCode::Char('p') => {
                                    // Toggle historical replay; on entry, fetch the frame index.
                                    if app.toggle_replay() {
                                        fetch_replay_index(tx.clone(), http.clone(), base.clone());
                                    }
                                }
                                KeyCode::Char('r') => { let _ = refresh_tx.try_send(()); app.loading = true; }
                                KeyCode::Right | KeyCode::Tab => {
                                    if app.is_replaying() {
                                        if let Some(id) = app.replay_scrub(1) {
                                            fetch_replay_snapshot(tx.clone(), http.clone(), base.clone(), id);
                                        }
                                    } else { app.clear_zoom(); app.next_route(); }
                                }
                                KeyCode::Left => {
                                    if app.is_replaying() {
                                        if let Some(id) = app.replay_scrub(-1) {
                                            fetch_replay_snapshot(tx.clone(), http.clone(), base.clone(), id);
                                        }
                                    } else { app.clear_zoom(); app.prev_route(); }
                                }
                                KeyCode::Down  => app.select_next(),
                                KeyCode::Up    => app.select_prev(),
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        if let Some(text) = app.take_speak() {
            tts::speak(&text);
        }
        if app.should_quit {
            break;
        }
        terminal.draw(|f| ui::draw(f, &app))?;
    }
    Ok(())
}

/// Background daemon: keep the local AI cache fresh from the deployed Worker.
/// Dispatch every minute, SITREP every 5, events every 30 (the Worker is heavily
/// cached, so this is ~free). On any fetch error we keep the prior cached row.
async fn daemon_loop(home_mapid: String, home_name: String) -> Result<()> {
    let conn = store::open()?;
    let client = reqwest::Client::builder()
        .user_agent("cta-tui/0.1 (+daemon)")
        .build()?;
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    let mut n: u64 = 0;
    loop {
        tick.tick().await; // first tick fires immediately → populates on start
        if let Ok(r) = ai::fetch_dispatch(&client).await {
            let _ = store::upsert(&conn, "dispatch", r.summary.as_deref().unwrap_or(""), r.count.unwrap_or(0));
        }
        if n % 5 == 0 {
            if let Ok(r) = ai::fetch_sitrep(&client, &home_mapid, &home_name).await {
                let _ = store::upsert(&conn, "sitrep", r.summary.as_deref().unwrap_or(""), r.count.unwrap_or(0));
            }
        }
        if n % 30 == 0 {
            if let Ok(r) = ai::fetch_events(&client).await {
                let _ = store::upsert(&conn, "events", r.summary.as_deref().unwrap_or(""), r.count.unwrap_or(0));
            }
        }
        let _ = store::touch_heartbeat(&conn);
        n = n.wrapping_add(1);
    }
}

/// Spawn the AI daemon detached if the cache has no fresh heartbeat (≤90s). The
/// freshness check prevents duplicate daemons across multiple TUI instances.
fn ensure_daemon() {
    let fresh = store::open()
        .ok()
        .and_then(|c| store::heartbeat_age_secs(&c))
        .is_some_and(|age| age <= 90);
    if fresh {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = std::process::Command::new(exe);
    cmd.env("CTA_DAEMON", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        extern "C" {
            fn setsid() -> i32;
        }
        // New session so the daemon outlives the TUI and its terminal.
        unsafe {
            cmd.pre_exec(|| {
                setsid();
                Ok(())
            });
        }
    }
    let _ = cmd.spawn();
}
