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
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

const AMBER: Color = Color::Rgb(0xff, 0xb0, 0x00);
const DIM: Color = Color::Rgb(0x55, 0x66, 0x55);
const GRID: Color = Color::Rgb(0x0a, 0xff, 0x9a);
const PHOS: Color = Color::Rgb(0x9a, 0xff, 0xd0); // bright phosphor highlight
const RED: Color = Color::Rgb(0xff, 0x3b, 0x3b);

/// Rotating "radar dish" glyph for the header sweep.
const SWEEP: [char; 4] = ['◜', '◝', '◞', '◟'];

/// Recognizable system anchors — major downtown/transfer stations — labeled on
/// the full rail so the line is navigable between its termini. Curated (not
/// transfer-count derived) because CTA reuses station names across physically
/// separate stops (e.g. three different "Western"s).
const LANDMARKS: &[&str] = &[
    "clark/lake", // the big Loop transfer hub (one Loop anchor is enough)
    "jackson",    // Red/Blue subway transfer
    "roosevelt",  // south downtown gateway (Red/Orange/Green)
    "fullerton",
    "belmont",
    "howard",
];

fn is_landmark(name: &str) -> bool {
    let n = name.trim().to_lowercase();
    LANDMARKS.contains(&n.as_str())
}

/// Scale an RGB color's brightness (used to dim the rail below its ticks).
fn scale(c: Color, f: f64) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f64 * f) as u8,
            (g as f64 * f) as u8,
            (b as f64 * f) as u8,
        ),
        other => other,
    }
}

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
    for (k, label) in [("/", "FIND"), ("q", "QUIT"), ("r", "RESCAN"), ("←/→", "LINE"), ("↑/↓", "TRAIN")] {
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

    if app.search.is_some() {
        search_overlay(f, inner, app, blink_on);
    }
}

/// Centered fuzzy-finder popup over the body.
fn search_overlay(f: &mut Frame, body: Rect, app: &App, blink_on: bool) {
    let Some(s) = &app.search else { return };
    // Clamp strictly to the body — never larger — so Clear can't index past the
    // buffer on a tiny/zero-size terminal.
    let w = 54.min(body.width.saturating_sub(2));
    let h = 16.min(body.height.saturating_sub(2));
    if w < 16 || h < 5 {
        return;
    }
    let x = body.x + (body.width.saturating_sub(w)) / 2;
    let y = body.y + (body.height.saturating_sub(h)) / 2;
    let area = Rect { x, y, width: w, height: h };
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(AMBER))
        .title_top(Span::styled(
            " FIND STATION ",
            Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            format!(" {} hits  ↑/↓ ⏎ go  esc ", s.matches.len()),
            Style::default().fg(DIM),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let cursor = if blink_on { '▌' } else { ' ' };
    let prompt = Line::from(vec![
        Span::styled(" › ", Style::default().fg(AMBER).add_modifier(Modifier::BOLD)),
        Span::styled(s.query.clone(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled(cursor.to_string(), Style::default().fg(AMBER)),
    ]);

    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)])
        .split(inner);
    f.render_widget(Paragraph::new(prompt), split[0]);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(inner.width as usize),
            Style::default().fg(DIM),
        ))),
        split[1],
    );

    let rows = split[2].height as usize;
    let scroll = if s.cursor >= rows { s.cursor - rows + 1 } else { 0 };
    let idx = app.track.station_index();
    let items: Vec<ListItem> = if s.matches.is_empty() {
        vec![ListItem::new(Span::styled(" no match", Style::default().fg(DIM)))]
    } else {
        s.matches
            .iter()
            .enumerate()
            .skip(scroll)
            .take(rows)
            .map(|(i, &m)| {
                let st = &idx[m];
                let sel = i == s.cursor;
                let mark = if sel { "▌" } else { " " };
                let line = Line::from(vec![
                    Span::styled(
                        format!("{mark}● "),
                        Style::default().fg(route_color(&st.route)),
                    ),
                    Span::styled(
                        trunc(&st.name, (inner.width as usize).saturating_sub(12)),
                        Style::default().fg(if sel { Color::White } else { PHOS }),
                    ),
                    Span::styled(
                        format!("  {}", st.route.to_uppercase()),
                        Style::default().fg(DIM),
                    ),
                ]);
                if sel {
                    ListItem::new(line).style(Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED))
                } else {
                    ListItem::new(line)
                }
            })
            .collect()
    };
    f.render_widget(List::new(items), split[2]);
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
    // The displayed line is the zoom target if zoomed, else the focused board.
    let key = app.view_route().unwrap_or_default();
    let board = app.snap.boards.iter().find(|b| b.key == key);
    let trains: &[crate::cta::Train] = board.map(|b| b.trains.as_slice()).unwrap_or(&[]);
    let label = board
        .map(|b| b.label.clone())
        .unwrap_or_else(|| crate::cta::pretty_route(&key));
    let color = route_color(&key);
    let zoom_center = match &app.zoom {
        Some(z) if z.route == key => Some(z.index),
        _ => None,
    };

    let mut title = vec![
        Span::styled(
            format!(" {} LINE ", label.to_uppercase()),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("[{} TRK] ←/→ ", trains.len()), Style::default().fg(DIM)),
    ];
    if let Some(c) = zoom_center {
        // Zoomed: name the station we're centered on.
        if let Some(st) = app.track.route(&key).and_then(|rt| rt.stations.get(c)) {
            title.push(Span::styled(
                format!("⊙ {} ", st.name.to_uppercase()),
                Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
            ));
        }
    } else if let Some(run) = app.selected_run() {
        title.push(Span::styled(
            format!("▌SEL #{run} "),
            Style::default().fg(PHOS).add_modifier(Modifier::BOLD),
        ));
    }

    let block = panel_block(Line::from(title), color, true);
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Track map strip on top when there's vertical room, train list below.
    let map_h: u16 = if inner.height >= 9 { 5 } else { 0 };
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(map_h), Constraint::Min(0)])
        .split(inner);

    if map_h > 0 {
        match app.track.route(&key) {
            Some(rt) if zoom_center.is_some() => {
                draw_track_zoom(f, split[0], app, rt, &key, color, zoom_center.unwrap(), blink_on)
            }
            Some(rt) => draw_track_full(f, split[0], app, rt, &key, color, blink_on),
            None => f.render_widget(
                Paragraph::new(Span::styled(" no map data", Style::default().fg(DIM))),
                split[0],
            ),
        }
    }
    draw_train_list(f, split[1], trains, app.selected, color, blink_on);
}

/// fio 4 — the ASCII track map: a straight rail with station ticks, the home
/// station starred, and live trains projected onto it (inbound above the rail,
/// outbound below). Conveys at a glance where every train on the line is.
fn draw_track_full(f: &mut Frame, area: Rect, app: &App, rt: &crate::track::RouteTrack, key: &str, color: Color, blink_on: bool) {
    let w = area.width as usize;
    if w < 8 {
        return;
    }
    let n = rt.stations.len();
    let last = n.saturating_sub(1).max(1);
    let col = |slot: f64| ((slot * (w.saturating_sub(1)) as f64).round() as usize).min(w - 1);
    // Stations are evenly spaced (strip-map style); trains warp through the same
    // station space so they land proportionally between their neighbors.
    let xof_station = |i: usize| col(i as f64 / last as f64);
    let home = app.home_label.to_lowercase();

    // Rail: heavy line in a dimmed brand color, station ticks, ◆ termini, ★
    // home. Priority keeps the star/terminus from being clobbered when stations
    // crowd the same column.
    let mut rail = RowBuf::new(w, '━', Style::default().fg(scale(color, 0.45)));
    let mut home_col: Option<usize> = None;
    for (i, s) in rt.stations.iter().enumerate() {
        let x = xof_station(i);
        let name = s.name.to_lowercase();
        if !home.is_empty() && (name == home || name.contains(&home)) {
            rail.put_prio(x, '★', Style::default().fg(AMBER).add_modifier(Modifier::BOLD), 3);
            home_col = Some(x);
        } else if i == 0 || i == n - 1 {
            rail.put_prio(x, '◆', Style::default().fg(color).add_modifier(Modifier::BOLD), 2);
        } else if is_landmark(&s.name) {
            rail.put_prio(x, '◈', Style::default().fg(color).add_modifier(Modifier::BOLD), 2);
        } else {
            rail.put_prio(x, '┿', Style::default().fg(color), 1);
        }
    }

    // Trains projected by lat/lon. Direction along the strip comes from the
    // compass heading dotted with the local rail tangent — rightward trains ride
    // the upper rail (▸/▶), leftward the lower (◂/◀); filled = approaching.
    let mut up = RowBuf::new(w, ' ', Style::default());
    let mut dn = RowBuf::new(w, ' ', Style::default());
    for (i, t) in trains_of_key(app, key).iter().enumerate() {
        let (Some(lat), Some(lon)) = (t.lat, t.lon) else { continue };
        let Some(pj) = rt.project(lat, lon) else { continue };
        let x = col(rt.pos_to_slot(pj.pos01));
        let sel = i == app.selected;
        let (style, prio) = if sel {
            // Selected train always wins and is always drawn (ignores blink).
            (
                Style::default()
                    .fg(Color::Black)
                    .bg(PHOS)
                    .add_modifier(Modifier::BOLD),
                4,
            )
        } else if t.delayed {
            if !blink_on {
                continue; // blink off → leave the cell empty this frame
            }
            (Style::default().fg(AMBER).add_modifier(Modifier::BOLD), 3)
        } else if t.approaching {
            (Style::default().fg(PHOS).add_modifier(Modifier::BOLD), 2)
        } else {
            (Style::default().fg(color).add_modifier(Modifier::BOLD), 1)
        };
        let forward = match t.heading {
            Some(h) => {
                let r = (h as f64).to_radians();
                // heading: 0°=N, 90°=E; planar x=east, y=north.
                r.sin() * pj.seg.0 + r.cos() * pj.seg.1 >= 0.0
            }
            None => t.dir.as_deref() != Some("5"),
        };
        let glyph = match (forward, t.approaching) {
            (true, true) => '▶',
            (true, false) => '▸',
            (false, true) => '◀',
            (false, false) => '◂',
        };
        let row = if forward { &mut up } else { &mut dn };
        row.put_prio(x, glyph, style, prio);
    }

    // Two staggered label rows. Termini are pinned to the ends; home and
    // landmark stations are packed in by priority, skipping any that would
    // collide with an already-placed label.
    let mut packer = LabelPacker::new(w);
    if let Some(first) = rt.stations.first() {
        packer.pin_left(&trunc(&first.name, w / 3), Style::default().fg(DIM));
    }
    if let Some(last) = rt.stations.last() {
        packer.pin_right(&trunc(&last.name, w / 3), Style::default().fg(DIM));
    }
    if let Some(hx) = home_col {
        packer.place(
            hx,
            &trunc(&app.home_label.to_uppercase(), 14),
            Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
        );
    }
    for (i, s) in rt.stations.iter().enumerate() {
        let name = s.name.to_lowercase();
        let is_home = !home.is_empty() && (name == home || name.contains(&home));
        if i == 0 || i == n - 1 || is_home || !is_landmark(&s.name) {
            continue;
        }
        packer.place(xof_station(i), &trunc(&s.name, 13), Style::default().fg(Color::White));
    }
    let (lab_a, lab_b) = packer.finish();

    let rows = vec![
        up.into_line(),
        rail.into_line(),
        dn.into_line(),
        lab_a.into_line(),
        lab_b.into_line(),
    ];
    f.render_widget(Paragraph::new(rows), area);
}

/// Zoomed track view: a window of ~9 stations centered on `center`, every one
/// labeled (staggered across two rows), the target starred, trains in the
/// window placed, and «N»/«N» counts for trains beyond either edge.
fn draw_track_zoom(f: &mut Frame, area: Rect, app: &App, rt: &crate::track::RouteTrack, key: &str, color: Color, center: usize, blink_on: bool) {
    let w = area.width as usize;
    if w < 8 {
        return;
    }
    let n = rt.stations.len();
    const R: usize = 4; // window radius → up to 9 stations
    let start = center.saturating_sub(R);
    let end = (center + R).min(n.saturating_sub(1));
    let span = (end - start).max(1);
    // Column of a continuous station-space index within the window.
    let col = |idx: f64| {
        (((idx - start as f64) / span as f64) * (w.saturating_sub(1)) as f64)
            .round()
            .clamp(0.0, (w - 1) as f64) as usize
    };

    // Rail + ticks/star for the window stations.
    let mut rail = RowBuf::new(w, '━', Style::default().fg(scale(color, 0.45)));
    for i in start..=end {
        let x = col(i as f64);
        if i == center {
            rail.put_prio(x, '★', Style::default().fg(AMBER).add_modifier(Modifier::BOLD), 3);
        } else if i == 0 || i == n - 1 {
            rail.put_prio(x, '◆', Style::default().fg(color).add_modifier(Modifier::BOLD), 2);
        } else {
            rail.put_prio(x, '┿', Style::default().fg(color), 1);
        }
    }

    // Trains: place those whose station-space index falls within the window;
    // count the rest as off-window overflow at each edge.
    let mut up = RowBuf::new(w, ' ', Style::default());
    let mut dn = RowBuf::new(w, ' ', Style::default());
    let (mut left_off, mut right_off) = (0u32, 0u32);
    for t in trains_of_key(app, key) {
        let (Some(lat), Some(lon)) = (t.lat, t.lon) else { continue };
        let Some(pj) = rt.project(lat, lon) else { continue };
        let idx = rt.pos_to_index(pj.pos01);
        if idx < start as f64 - 0.5 {
            left_off += 1;
            continue;
        }
        if idx > end as f64 + 0.5 {
            right_off += 1;
            continue;
        }
        let (style, prio) = if t.delayed {
            if !blink_on {
                continue;
            }
            (Style::default().fg(AMBER).add_modifier(Modifier::BOLD), 3)
        } else if t.approaching {
            (Style::default().fg(PHOS).add_modifier(Modifier::BOLD), 2)
        } else {
            (Style::default().fg(color).add_modifier(Modifier::BOLD), 1)
        };
        let forward = match t.heading {
            Some(h) => {
                let r = (h as f64).to_radians();
                r.sin() * pj.seg.0 + r.cos() * pj.seg.1 >= 0.0
            }
            None => t.dir.as_deref() != Some("5"),
        };
        let glyph = match (forward, t.approaching) {
            (true, true) => '▶',
            (true, false) => '▸',
            (false, true) => '◀',
            (false, false) => '◂',
        };
        let row = if forward { &mut up } else { &mut dn };
        row.put_prio(col(idx), glyph, style, prio);
    }
    if left_off > 0 {
        up.put(0, '«', Style::default().fg(DIM));
        dn.put(0, '«', Style::default().fg(DIM));
    }
    if right_off > 0 {
        up.put(w - 1, '»', Style::default().fg(DIM));
        dn.put(w - 1, '»', Style::default().fg(DIM));
    }

    // Two staggered label rows so every window station fits without overlap.
    let mut lab_a = RowBuf::new(w, ' ', Style::default().fg(DIM));
    let mut lab_b = RowBuf::new(w, ' ', Style::default().fg(DIM));
    for i in start..=end {
        let x = col(i as f64);
        let is_center = i == center;
        let label = trunc(&rt.stations[i].name, (w / span).saturating_sub(1).max(4));
        let style = if is_center {
            Style::default().fg(AMBER).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let lab = if (i - start) % 2 == 0 { &mut lab_a } else { &mut lab_b };
        let lx = x.saturating_sub(label.chars().count() / 2).min(w.saturating_sub(label.chars().count()));
        lab.write_str(lx, &label, style);
    }

    let rows = vec![
        up.into_line(),
        rail.into_line(),
        dn.into_line(),
        lab_a.into_line(),
        lab_b.into_line(),
    ];
    f.render_widget(Paragraph::new(rows), area);
}

fn draw_train_list(f: &mut Frame, area: Rect, trains: &[crate::cta::Train], selected: usize, color: Color, blink_on: bool) {
    // Auto-scroll so the selected train stays visible in the window.
    let visible = area.height as usize;
    let scroll = if visible > 0 && selected >= visible {
        selected - visible + 1
    } else {
        0
    };

    let mut rows: Vec<ListItem> = Vec::new();
    for (i, t) in trains.iter().enumerate().skip(scroll) {
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
        let sel = i == selected;
        let cursor = if sel { "▌" } else { " " };
        // Flag goes up front (after ETA) so APP/DLY can never clip off the
        // right edge on a narrow terminal. Then next stop, then terminal dest.
        let line = Line::from(vec![
            Span::styled(
                format!("{cursor}{} ", heading_arrow(t.heading)),
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
        // Selected row gets a phosphor underline so it reads against the colors.
        let item = if sel {
            ListItem::new(line).style(Style::default().add_modifier(Modifier::UNDERLINED | Modifier::BOLD))
        } else {
            ListItem::new(line)
        };
        rows.push(item);
    }
    if rows.is_empty() {
        rows.push(ListItem::new(Span::styled(
            " no trains reported",
            Style::default().fg(DIM),
        )));
    }
    f.render_widget(List::new(rows), area);
}

/// Trains on the board with the given route key (empty if not tracked).
fn trains_of_key<'a>(app: &'a App, key: &str) -> &'a [crate::cta::Train] {
    app.snap
        .boards
        .iter()
        .find(|b| b.key == key)
        .map(|b| b.trains.as_slice())
        .unwrap_or(&[])
}

fn arrivals_panel(f: &mut Frame, area: Rect, app: &App, blink_on: bool) {
    // fio 3 — flash the panel when a train is freshly within the alert window.
    let flashing = app.flash > 0;
    let border = if flashing && blink_on {
        Color::White
    } else {
        AMBER
    };
    let mut title = vec![
        Span::styled(" ★ ", Style::default().fg(AMBER)),
        Span::styled(
            format!("{} ", trunc(&app.home_label, 18).to_uppercase()),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
    ];
    if flashing {
        title.push(Span::styled(
            if blink_on { "◀ APPROACH " } else { "           " },
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ));
    }
    let title = Line::from(title);

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
        List::new(items).block(panel_block(title, border, false)),
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

/// A fixed-width row of styled cells. Markers are placed by column, then
/// adjacent same-style cells are coalesced into spans for rendering.
struct RowBuf {
    ch: Vec<char>,
    st: Vec<Style>,
    prio: Vec<u8>,
}

impl RowBuf {
    fn new(w: usize, fill: char, style: Style) -> Self {
        RowBuf {
            ch: vec![fill; w],
            st: vec![style; w],
            prio: vec![0; w],
        }
    }

    fn put(&mut self, x: usize, ch: char, style: Style) {
        if x < self.ch.len() {
            self.ch[x] = ch;
            self.st[x] = style;
        }
    }

    /// Place a cell only if it outranks what's already there (keeps APP/DLY
    /// trains visible when several project onto the same column).
    fn put_prio(&mut self, x: usize, ch: char, style: Style, prio: u8) {
        if x < self.ch.len() && prio >= self.prio[x] {
            self.ch[x] = ch;
            self.st[x] = style;
            self.prio[x] = prio;
        }
    }

    fn write_str(&mut self, x0: usize, s: &str, style: Style) {
        for (i, c) in s.chars().enumerate() {
            self.put(x0 + i, c, style);
        }
    }

    fn into_line(self) -> Line<'static> {
        let mut spans = Vec::new();
        let mut i = 0;
        let len = self.ch.len();
        while i < len {
            let style = self.st[i];
            let mut buf = String::new();
            while i < len && self.st[i] == style {
                buf.push(self.ch[i]);
                i += 1;
            }
            spans.push(Span::styled(buf, style));
        }
        Line::from(spans)
    }
}

/// Packs station labels into two staggered rows, skipping any that would
/// collide (1-column gap) with one already placed. Termini are pinned to the
/// ends; everything else is centered under its column, trying row A then B.
struct LabelPacker {
    w: usize,
    rows: [RowBuf; 2],
    occ: [Vec<(usize, usize)>; 2],
}

impl LabelPacker {
    fn new(w: usize) -> Self {
        LabelPacker {
            w,
            rows: [
                RowBuf::new(w, ' ', Style::default()),
                RowBuf::new(w, ' ', Style::default()),
            ],
            occ: [Vec::new(), Vec::new()],
        }
    }

    fn fits(&self, row: usize, start: usize, end: usize) -> bool {
        self.occ[row]
            .iter()
            .all(|&(s, e)| end + 1 <= s || start >= e + 1)
    }

    fn write(&mut self, row: usize, start: usize, text: &str, style: Style) {
        self.rows[row].write_str(start, text, style);
        self.occ[row].push((start, start + text.chars().count()));
    }

    fn pin_left(&mut self, text: &str, style: Style) {
        self.write(0, 0, text, style);
    }

    fn pin_right(&mut self, text: &str, style: Style) {
        let start = self.w.saturating_sub(text.chars().count());
        self.write(0, start, text, style);
    }

    /// Place a label centered on `center`, in the first row where it fits.
    fn place(&mut self, center: usize, text: &str, style: Style) {
        let len = text.chars().count();
        if len == 0 || len > self.w {
            return;
        }
        let start = center.saturating_sub(len / 2).min(self.w - len);
        let end = start + len;
        for row in 0..2 {
            if self.fits(row, start, end) {
                self.write(row, start, text, style);
                return;
            }
        }
    }

    fn finish(self) -> (RowBuf, RowBuf) {
        let [a, b] = self.rows;
        (a, b)
    }
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
