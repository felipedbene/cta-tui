//! CTA Track Grid — terminal edition.
//!
//! Env:
//!   CTA_KEY        Train Tracker API key (required)
//!   CTA_ROUTES     comma routes (default: red,blue,brn,g,org,p,pink,y)
//!   CTA_HOME_MAPID home station map id (default: 41070 = Kedzie/Green)
//!   CTA_HOME_NAME  label for the home panel (default: Kedzie)
//!   CTA_REFRESH    seconds between polls (default: 30)

mod app;
mod cta;
mod track;
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
}

#[tokio::main]
async fn main() -> Result<()> {
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
        let mut app = App::new(home_name);
        app.apply(snap);
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

    // --- terminal ---
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let res = run(&mut terminal, &mut rx, refresh_tx, home_name).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    res
}

async fn run<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    rx: &mut mpsc::Receiver<Msg>,
    refresh_tx: mpsc::Sender<()>,
    home_name: String,
) -> Result<()> {
    let mut app = App::new(home_name);
    let mut events = EventStream::new();
    // ~4 fps render tick so the radar sweep + APP/DLY blink stay alive between polls.
    let mut frame = tokio::time::interval(Duration::from_millis(250));
    terminal.draw(|f| ui::draw(f, &app))?;

    loop {
        tokio::select! {
            _ = frame.tick() => {
                app.tick();
            }
            Some(Msg::Snap(snap)) = rx.recv() => {
                app.apply(snap);
            }
            Some(Ok(ev)) = events.next() => {
                if let Event::Key(k) = ev {
                    if k.kind == KeyEventKind::Press {
                        match k.code {
                            KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                            KeyCode::Char('r') => { let _ = refresh_tx.try_send(()); app.loading = true; }
                            KeyCode::Right | KeyCode::Tab => app.next_route(),
                            KeyCode::Left  => app.prev_route(),
                            KeyCode::Down  => app.scroll_down(),
                            KeyCode::Up    => app.scroll_up(),
                            _ => {}
                        }
                    }
                }
            }
        }
        if app.should_quit {
            break;
        }
        terminal.draw(|f| ui::draw(f, &app))?;
    }
    Ok(())
}
