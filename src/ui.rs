//! NORAD-style rendering.
//!
//! The whole screen is one double-ruled console frame: classification banner +
//! live clock in the top rule, key legend in the bottom rule. Inside sit three
//! panels — SYSTEM status, the focused line's train board, and the home-station
//! arrivals ticker. A 4 fps frame counter (`app.frame`) drives the radar sweep
//! and the APP/DLY blink so the board reads as "live" between polls.

use crate::app::{heading_arrow, route_color, status_color, App};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem},
    Frame,
};

const AMBER: Color = Color::Rgb(0xff, 0xb0, 0x00);
const DIM: Color = Color::Rgb(0x55, 0x66, 0x55);
const GRID: Color = Color::Rgb(0x0a, 0xff, 0x9a);
const PHOS: Color = Color::Rgb(0x9a, 0xff, 0xd0); // bright phosphor highlight
const RED: Color = Color::Rgb(0xff, 0x3b, 0x3b);

/// Rotating "radar dish" glyph for the header sweep.
const SWEEP: [char; 4] = ['◜', '◝', '◞', '◟'];

pub fn draw(f: &mut Frame, app: &App) {
    let blink_on = (app.frame / 2) % 2 == 0; // ~0.5s on/off at 4 fps
    let sweep = SWEEP[(app.frame as usize / 2) % SWEEP.len()];

    // --- outer console frame: banner + clock top, legend bottom ---
    let n_trains: usize = app.snap.boards.iter().map(|b| b.trains.len()).sum();
    let n_lines = app.snap.boards.len();

    let scan = if app.loading {
        Span::styled(
            if blink_on { " ACQUIRING " } else { "           " },
            Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
        )
    } else if app.snap.error.is_some() {
        Span::styled(
            if blink_on { " LINK FAULT " } else { "            " },
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            " TRACKING ",
            Style::default().fg(GRID).add_modifier(Modifier::BOLD),
        )
    };

    let banner = Line::from(vec![
        Span::styled(
            format!(" {sweep} "),
            Style::default().fg(GRID).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "CTA TRACK GRID",
            Style::default()
                .fg(PHOS)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
        ),
        Span::styled(" NORAD COMMAND ", Style::default().fg(DIM)),
        scan,
    ]);

    let clock = Line::from(vec![
        Span::styled("UNCLASS ", Style::default().fg(DIM)),
        Span::styled(
            format!(" UPD {} ", app.snap.updated),
            Style::default().fg(GRID),
        ),
    ])
    .right_aligned();

    let mut legend_spans = Vec::new();
    for (k, label) in [("q", "QUIT"), ("r", "RESCAN"), ("←/→", "LINE"), ("↑/↓", "SCROLL")] {
        legend_spans.extend(key(k, label));
    }
    let legend = Line::from(legend_spans);

    let status_line = match app.snap.error.as_deref() {
        Some(e) => Line::from(Span::styled(
            format!(" ⚠ {} ", trunc(e, 40)),
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        ))
        .right_aligned(),
        None => Line::from(Span::styled(
            format!(" {n_trains} TRAINS // {n_lines} LINES TRACKED "),
            Style::default().fg(DIM),
        ))
        .right_aligned(),
    };

    let frame = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(GRID))
        .title_top(banner)
        .title_top(clock)
        .title_bottom(legend)
        .title_bottom(status_line);

    let inner = frame.inner(f.area());
    f.render_widget(frame, f.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(24), // system board
            Constraint::Min(0),     // focused line trains
            Constraint::Length(30), // home arrivals
        ])
        .split(inner);

    system_board(f, body[0], app, blink_on);
    train_panel(f, body[1], app, blink_on);
    arrivals_panel(f, body[2], app, blink_on);
}

/// A bottom-rule key hint: reverse-video key cap + dim label.
fn key(k: &str, label: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(
            format!(" {k} "),
            Style::default()
                .fg(Color::Black)
                .bg(GRID)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {label}  "), Style::default().fg(DIM)),
    ]
}

fn panel_block(title: Line<'static>, color: Color, focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Plain
        })
        .border_style(Style::default().fg(color))
        .title_top(title)
}

fn system_board(f: &mut Frame, area: Rect, app: &App, blink_on: bool) {
    let title = Line::from(Span::styled(
        " SYSTEM ",
        Style::default().fg(GRID).add_modifier(Modifier::BOLD),
    ));

    let items: Vec<ListItem> = if app.snap.statuses.is_empty() {
        vec![ListItem::new(Span::styled(
            " no feed",
            Style::default().fg(DIM),
        ))]
    } else {
        app.snap
            .statuses
            .iter()
            .map(|s| {
                let normal = s.status.to_lowercase().contains("normal");
                let dot_style = Style::default().fg(status_color(&s.color_hex));
                let dot = Span::styled("● ", dot_style);
                let name = Span::styled(
                    format!("{:<5}", short_line(&s.route)),
                    Style::default().fg(Color::White),
                );
                // Non-normal status blinks amber like a real alert annunciator;
                // normal status takes the feed's own status color (muted green).
                let st_style = if normal {
                    Style::default().fg(status_color(&s.status_color_hex))
                } else if blink_on {
                    Style::default().fg(AMBER).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(DIM)
                };
                let st = Span::styled(format!(" {}", trunc(&s.status, 11)), st_style);
                ListItem::new(Line::from(vec![Span::raw(" "), dot, name, st]))
            })
            .collect()
    };
    f.render_widget(List::new(items).block(panel_block(title, DIM, false)), area);
}

fn train_panel(f: &mut Frame, area: Rect, app: &App, blink_on: bool) {
    let (label, key, trains) = match app.snap.boards.get(app.focused) {
        Some(b) => (b.label.clone(), b.key.clone(), b.trains.as_slice()),
        None => ("—".into(), String::new(), [].as_slice()),
    };
    let color = route_color(&key);

    let title = Line::from(vec![
        Span::styled(
            format!(" {} LINE ", label.to_uppercase()),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("[{} TRK] ←/→ ", trains.len()),
            Style::default().fg(DIM),
        ),
    ]);

    let mut rows: Vec<ListItem> = Vec::new();
    for t in trains.iter().skip(app.scroll) {
        let eta = fmt_eta(t.eta_min);
        let (flag_style, tag) = if t.delayed {
            let s = if blink_on {
                Style::default().fg(AMBER).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(DIM)
            };
            (s, "DLY")
        } else if t.approaching {
            (Style::default().fg(GRID).add_modifier(Modifier::BOLD), "APP")
        } else {
            (Style::default().fg(Color::White), "")
        };
        // Flag goes up front (after ETA) so APP/DLY can never clip off the
        // right edge on a narrow terminal. Then next stop, then terminal dest.
        let line = Line::from(vec![
            Span::styled(
                format!(" {} ", heading_arrow(t.heading)),
                Style::default().fg(color),
            ),
            Span::styled(format!("#{:<4} ", t.run), Style::default().fg(DIM)),
            Span::styled(format!("{:>4} ", eta), flag_style),
            Span::styled(format!("{:<4}", tag), flag_style),
            Span::styled(
                format!("→ {:<16}", trunc(&t.next_station, 16)),
                Style::default().fg(PHOS),
            ),
            Span::styled(
                format!(" ▸ {:<12}", trunc(&t.dest, 12)),
                Style::default().fg(DIM),
            ),
        ]);
        rows.push(ListItem::new(line));
    }
    if rows.is_empty() {
        rows.push(ListItem::new(Span::styled(
            " no trains reported",
            Style::default().fg(DIM),
        )));
    }
    f.render_widget(
        List::new(rows).block(panel_block(title, color, true)),
        area,
    );
}

fn arrivals_panel(f: &mut Frame, area: Rect, app: &App, blink_on: bool) {
    let title = Line::from(vec![
        Span::styled(" ★ ", Style::default().fg(AMBER)),
        Span::styled(
            format!("{} ", trunc(&app.home_label, 18).to_uppercase()),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let items: Vec<ListItem> = if app.snap.arrivals.is_empty() {
        vec![ListItem::new(Span::styled(
            " no arrivals",
            Style::default().fg(DIM),
        ))]
    } else {
        app.snap
            .arrivals
            .iter()
            .take(14)
            .map(|a| {
                let eta = fmt_eta(a.eta_min);
                let st = if a.delayed {
                    if blink_on {
                        Style::default().fg(AMBER).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(DIM)
                    }
                } else if a.approaching {
                    Style::default().fg(GRID).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {:>4} ", eta), st),
                    Span::styled(
                        "● ",
                        Style::default().fg(route_color(&a.route.to_lowercase())),
                    ),
                    Span::styled(trunc(&a.dest, 17), Style::default().fg(PHOS)),
                ]))
            })
            .collect()
    };
    f.render_widget(
        List::new(items).block(panel_block(title, AMBER, false)),
        area,
    );
}

fn fmt_eta(eta_min: Option<i64>) -> String {
    match eta_min {
        Some(m) if m <= 0 => "DUE".to_string(),
        Some(m) => format!("{m}m"),
        None => "--".to_string(),
    }
}

/// "Red Line" -> "RED", "Purple Line Express" -> "PEXP".
fn short_line(name: &str) -> String {
    match name.trim().to_lowercase().as_str() {
        "red line" => "RED",
        "blue line" => "BLUE",
        "brown line" => "BRN",
        "green line" => "GRN",
        "orange line" => "ORG",
        "purple line" => "PURP",
        "purple line express" => "PEXP",
        "pink line" => "PINK",
        "yellow line" => "YEL",
        other => other,
    }
    .to_string()
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
