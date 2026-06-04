//! UI-side application state.

use crate::cta::Snapshot;
use crate::track::TrackMap;
use ratatui::style::Color;

pub struct App {
    pub snap: Snapshot,
    pub focused: usize,  // index into snap.boards
    pub selected: usize, // index into the focused board's trains
    pub loading: bool,
    pub should_quit: bool,
    pub home_label: String,
    pub frame: u64, // animation tick, drives the radar sweep + blink
    pub track: TrackMap,
}

impl App {
    pub fn new(home_label: String) -> Self {
        Self {
            snap: Snapshot::default(),
            focused: 0,
            selected: 0,
            loading: true,
            should_quit: false,
            home_label,
            frame: 0,
            track: TrackMap::load(),
        }
    }

    pub fn tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// Number of trains on the focused line.
    pub fn focused_len(&self) -> usize {
        self.snap.boards.get(self.focused).map_or(0, |b| b.trains.len())
    }

    pub fn apply(&mut self, snap: Snapshot) {
        self.loading = false;
        if self.focused >= snap.boards.len() {
            self.focused = 0;
        }
        self.snap = snap;
        let len = self.focused_len();
        if self.selected >= len {
            self.selected = len.saturating_sub(1);
        }
    }

    pub fn next_route(&mut self) {
        if !self.snap.boards.is_empty() {
            self.focused = (self.focused + 1) % self.snap.boards.len();
            self.selected = 0;
        }
    }

    pub fn prev_route(&mut self) {
        if !self.snap.boards.is_empty() {
            self.focused = (self.focused + self.snap.boards.len() - 1) % self.snap.boards.len();
            self.selected = 0;
        }
    }

    /// Move the train cursor; `select_next` saturates at the last train.
    pub fn select_next(&mut self) {
        let len = self.focused_len();
        if len > 0 {
            self.selected = (self.selected + 1).min(len - 1);
        }
    }
    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// The currently selected train's run number, if any.
    pub fn selected_run(&self) -> Option<&str> {
        self.snap
            .boards
            .get(self.focused)
            .and_then(|b| b.trains.get(self.selected))
            .map(|t| t.run.as_str())
    }
}

/// Brand color per CTA line key.
pub fn route_color(key: &str) -> Color {
    match key.to_lowercase().as_str() {
        "red" => Color::Rgb(0xc6, 0x0c, 0x30),
        "blue" => Color::Rgb(0x00, 0xa1, 0xde),
        "brn" => Color::Rgb(0x62, 0x36, 0x1b),
        "g" => Color::Rgb(0x00, 0x9b, 0x3a),
        "org" => Color::Rgb(0xf9, 0x46, 0x1c),
        "p" | "pexp" => Color::Rgb(0x52, 0x2c, 0xa8),
        "pink" => Color::Rgb(0xe2, 0x7e, 0xa6),
        "y" => Color::Rgb(0xf9, 0xe3, 0x00),
        _ => Color::Gray,
    }
}

/// Map a CTA status color name (it returns hex) to a terminal color.
pub fn status_color(hex: &Option<String>) -> Color {
    match hex.as_deref() {
        Some(h) => parse_hex(h).unwrap_or(Color::Gray),
        None => Color::Gray,
    }
}

fn parse_hex(h: &str) -> Option<Color> {
    let h = h.trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

/// 8-way arrow from a compass heading in degrees.
pub fn heading_arrow(deg: Option<u16>) -> char {
    let Some(d) = deg else { return '•' };
    let arrows = ['↑', '↗', '→', '↘', '↓', '↙', '←', '↖'];
    let idx = (((d as f32 + 22.5) / 45.0) as usize) % 8;
    arrows[idx]
}
