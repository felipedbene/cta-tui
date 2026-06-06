//! UI-side application state.

use crate::cta::{Frame, HistSnap, Snapshot};
use crate::store::AiState;
use crate::track::TrackMap;
use ratatui::style::Color;
use std::collections::{HashMap, HashSet, VecDeque};

/// Resolution of the per-route phosphor persistence buffer — fixed bins across
/// the rail (in `pos_to_slot` space) so it's independent of the render width.
pub const PHOS_BINS: usize = 128;

/// Active station fuzzy-search overlay.
pub struct Search {
    pub query: String,
    pub matches: Vec<usize>, // indices into TrackMap::station_index()
    pub cursor: usize,
}

/// Active historical replay: the fetched frame index + scrubber position.
pub struct Replay {
    pub frames: Vec<Frame>, // ascending by observed_at
    pub idx: usize,         // scrubber position into frames
}

/// A station the map is zoomed in on.
pub struct Zoom {
    pub route: String,
    pub branch: usize,
    pub index: usize, // station index within that branch
}

pub struct App {
    pub snap: Snapshot,
    pub focused: usize,  // index into snap.boards
    pub selected: usize, // index into the focused board's trains
    pub loading: bool,
    pub should_quit: bool,
    pub home_label: String,
    pub frame: u64, // animation tick, drives the radar sweep + blink
    pub track: TrackMap,
    pub search: Option<Search>,
    pub zoom: Option<Zoom>,
    pub show_alerts: bool,
    pub orient_override: Option<bool>, // None = auto by width; Some = forced (v / CTA_VERTICAL)
    pub metra: bool,                   // include Metra/SS (mirrors the live fetch in replay decode)
    // Stage 7b — historical replay (Worker-backed). `live_snap` parks the live
    // feed while a frozen frame is on screen in `snap`.
    pub replay: Option<Replay>,
    live_snap: Option<Snapshot>,
    // AI layer (polled by the daemon into local SQLite, read here).
    pub ai: AiState,
    pub show_ai: bool,   // intel panel (SITREP + event advisory) overlay
    pub show_loop: bool, // LOOP convergence overlay
    pub voice: bool,               // speak AI text via the OS TTS command
    last_spoken: String,           // last dispatch line spoken (dedupe)
    pending_speak: Option<String>, // utterance queued for main → tts::speak
    // Telemetry: poll-countdown clock.
    pub refresh_secs: u64,         // CTA_REFRESH; set by main after construction
    polled_at_frame: u64,          // app.frame at the last apply() (for the countdown)
    pub route_hist: HashMap<String, VecDeque<u16>>, // per-route train counts (throughput sparklines)
    // Radar persistence: per-route rail intensity (PHOS_BINS slots), decayed each
    // tick and re-stamped at live train positions → fading comet-tails.
    phosphor: HashMap<String, Vec<f32>>,
    // fio 3 — home-station approach notifier.
    pub alert_min: i64,           // threshold in minutes (0 disables)
    alerted: HashSet<String>,     // runs we've already alerted at the home station
    started: bool,                // suppress an alert storm on the first poll
    pub flash: u8,                // frames remaining to flash the arrivals panel
    pending_bell: bool,           // a new approach to ring once, consumed by main
    // fio 5 — desktop notification on delay.
    notify_enabled: bool,
    delayed_seen: HashSet<String>, // runs currently flagged delayed (already notified)
    started_delay: bool,           // suppress a notification storm on the first poll
    pending_notes: Vec<String>,    // newly-delayed lines, drained by main
}

impl App {
    pub fn new(home_label: String, alert_min: i64, notify_enabled: bool) -> Self {
        Self {
            snap: Snapshot::default(),
            focused: 0,
            selected: 0,
            loading: true,
            should_quit: false,
            home_label,
            frame: 0,
            track: TrackMap::load(),
            search: None,
            zoom: None,
            show_alerts: false,
            orient_override: None,
            metra: true,
            replay: None,
            live_snap: None,
            ai: AiState::default(),
            show_ai: false,
            show_loop: false,
            voice: false,
            last_spoken: String::new(),
            pending_speak: None,
            refresh_secs: 30,
            polled_at_frame: 0,
            route_hist: HashMap::new(),
            phosphor: HashMap::new(),
            alert_min,
            alerted: HashSet::new(),
            started: false,
            flash: 0,
            pending_bell: false,
            notify_enabled,
            delayed_seen: HashSet::new(),
            started_delay: false,
            pending_notes: Vec::new(),
        }
    }

    pub fn tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.flash = self.flash.saturating_sub(1);
        self.update_phosphor();
    }

    /// Radar persistence. Decay every rail's per-column intensity, then re-stamp
    /// the current train positions back to full. A train holds its own cell hot
    /// while it sits there; when it moves (next poll) the vacated cells fade,
    /// leaving a comet-tail down the rail.
    fn update_phosphor(&mut self) {
        const DECAY: f32 = 0.82;
        for b in &self.snap.boards {
            let buf = self
                .phosphor
                .entry(b.key.clone())
                .or_insert_with(|| vec![0.0; PHOS_BINS]);
            for v in buf.iter_mut() {
                *v *= DECAY;
            }
            // Stamp on the primary branch — the rail the horizontal map draws.
            let Some(rt) = self.track.branches(&b.key).first() else {
                continue;
            };
            for t in &b.trains {
                let (Some(lat), Some(lon)) = (t.lat, t.lon) else {
                    continue;
                };
                let Some(pj) = rt.project(lat, lon) else { continue };
                let slot = rt.pos_to_slot(pj.pos01).clamp(0.0, 1.0);
                let bin = ((slot * (PHOS_BINS - 1) as f64).round() as usize).min(PHOS_BINS - 1);
                buf[bin] = 1.0;
            }
        }
    }

    /// Per-route phosphor residual (PHOS_BINS slots in `pos_to_slot` space).
    pub fn phosphor_for(&self, key: &str) -> Option<&[f32]> {
        self.phosphor.get(key).map(|v| v.as_slice())
    }

    /// Consume a queued bell (rung once when a train newly comes within range).
    pub fn take_bell(&mut self) -> bool {
        std::mem::take(&mut self.pending_bell)
    }

    /// Drain newly-delayed descriptions for the main loop to push as notifications.
    pub fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_notes)
    }

    /// After a new snapshot, queue a desktop notification for any tracked train
    /// that has newly gone delayed. Dedupes by run; one delay episode per run.
    fn check_delays(&mut self) {
        if !self.notify_enabled {
            return;
        }
        let mut current = HashSet::new();
        let mut fresh = Vec::new();
        for b in &self.snap.boards {
            for t in &b.trains {
                if t.delayed && !t.run.is_empty() {
                    current.insert(t.run.clone());
                    if self.started_delay && !self.delayed_seen.contains(&t.run) {
                        fresh.push(format!("{} #{} → {}", b.label, t.run, t.dest));
                    }
                }
            }
        }
        if !fresh.is_empty() {
            self.pending_notes = fresh;
        }
        self.delayed_seen = current;
        self.started_delay = true;
    }

    /// After a new snapshot, fire the approach alert when a home-station train
    /// newly enters the threshold. Dedupes by run so each train rings once.
    fn check_approach(&mut self) {
        if self.alert_min <= 0 {
            return;
        }
        let near: HashSet<String> = self
            .snap
            .arrivals
            .iter()
            .filter(|a| a.eta_min.map_or(false, |m| m <= self.alert_min))
            .map(|a| a.run.clone())
            .filter(|r| !r.is_empty())
            .collect();
        // A train that wasn't near last poll but is now → alert (but not on the
        // very first poll, which would ring for everything already in range).
        let fresh = self.started && near.difference(&self.alerted).next().is_some();
        if fresh {
            self.pending_bell = true;
            self.flash = 8; // ~2s at 4 fps
        }
        self.alerted = near;
        self.started = true;
    }

    /// Number of trains on the focused line.
    pub fn focused_len(&self) -> usize {
        self.snap.boards.get(self.focused).map_or(0, |b| b.trains.len())
    }

    pub fn apply(&mut self, snap: Snapshot) {
        self.loading = false;
        self.polled_at_frame = self.frame; // stamp for the poll countdown
        // Per-route history for the throughput sparklines (cap ~60 polls).
        for b in &snap.boards {
            let buf = self.route_hist.entry(b.key.clone()).or_default();
            buf.push_back(b.trains.len() as u16);
            while buf.len() > 60 {
                buf.pop_front();
            }
        }
        // While replaying, hold live data aside and keep the frozen frame on screen.
        if self.replay.is_some() {
            self.live_snap = Some(snap);
            return;
        }
        if self.focused >= snap.boards.len() {
            self.focused = 0;
        }
        self.snap = snap;
        let len = self.focused_len();
        if self.selected >= len {
            self.selected = len.saturating_sub(1);
        }
        self.check_approach();
        self.check_delays();
    }

    /// Seconds until the next poll (telemetry countdown; frame ticks at 4 fps).
    pub fn poll_left(&self) -> u64 {
        self.refresh_secs
            .saturating_sub(self.frame.saturating_sub(self.polled_at_frame) / 4)
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

    // --- station search + zoom ---

    pub fn open_search(&mut self) {
        self.search = Some(Search { query: String::new(), matches: Vec::new(), cursor: 0 });
        self.recompute_matches();
    }

    pub fn close_search(&mut self) {
        self.search = None;
    }

    pub fn search_input(&mut self, c: char) {
        if let Some(s) = &mut self.search {
            s.query.push(c);
        }
        self.recompute_matches();
    }

    pub fn search_backspace(&mut self) {
        if let Some(s) = &mut self.search {
            s.query.pop();
        }
        self.recompute_matches();
    }

    pub fn search_move(&mut self, delta: i32) {
        if let Some(s) = &mut self.search {
            if s.matches.is_empty() {
                return;
            }
            let last = s.matches.len() - 1;
            s.cursor = (s.cursor as i32 + delta).clamp(0, last as i32) as usize;
        }
    }

    /// Rank the whole station index against the current query.
    fn recompute_matches(&mut self) {
        let Some(s) = &mut self.search else { return };
        let q = s.query.trim();
        let mut scored: Vec<(i32, usize)> = self
            .track
            .station_index()
            .iter()
            .enumerate()
            .filter_map(|(i, st)| fuzzy_score(&st.name, q).map(|sc| (sc, i)))
            .collect();
        // Best score first; ties broken by station name for stability.
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0).then_with(|| {
                self.track.station_index()[a.1]
                    .name
                    .cmp(&self.track.station_index()[b.1].name)
            })
        });
        s.matches = scored.into_iter().map(|(_, i)| i).collect();
        s.cursor = 0;
    }

    /// Jump to the highlighted result: focus its line and zoom on the station.
    pub fn commit_search(&mut self) {
        let Some(s) = &self.search else { return };
        let Some(&idx) = s.matches.get(s.cursor) else {
            self.close_search();
            return;
        };
        let st = self.track.station_index()[idx].clone();
        if let Some(pos) = self.snap.boards.iter().position(|b| b.key == st.route) {
            self.focused = pos;
            self.selected = 0;
        }
        self.zoom = Some(Zoom { route: st.route, branch: st.branch, index: st.index });
        self.close_search();
    }

    pub fn clear_zoom(&mut self) {
        self.zoom = None;
    }

    pub fn toggle_alerts(&mut self) {
        self.show_alerts = !self.show_alerts;
    }

    /// Cycle the orientation override: auto → force-vertical → force-horizontal → auto.
    pub fn toggle_vertical(&mut self) {
        self.orient_override = match self.orient_override {
            None => Some(true),
            Some(true) => Some(false),
            Some(false) => None,
        };
    }

    /// Latest AI text from the local cache (written by the daemon). Queues the
    /// dispatch line to be spoken when it changes and voice is on.
    pub fn set_ai(&mut self, ai: AiState) {
        self.ai = ai;
        if self.voice {
            let d = self.ai.dispatch.summary.trim();
            if !d.is_empty() && d != self.last_spoken {
                self.last_spoken = d.to_string();
                self.pending_speak = Some(d.to_string());
            }
        }
    }

    pub fn toggle_ai(&mut self) {
        self.show_ai = !self.show_ai;
        if self.show_ai {
            self.speak_intel(); // read the SITREP + advisory aloud (voice only)
        }
    }

    pub fn toggle_loop(&mut self) {
        self.show_loop = !self.show_loop;
    }

    // --- Stage 7b: historical replay ---

    /// Toggle replay. Returns true when we just entered replay (the caller then
    /// fetches the frame index from the Worker).
    pub fn toggle_replay(&mut self) -> bool {
        if self.replay.is_some() {
            self.exit_replay();
            false
        } else {
            self.enter_replay();
            true
        }
    }

    fn enter_replay(&mut self) {
        if self.replay.is_none() {
            self.live_snap = Some(std::mem::take(&mut self.snap));
            self.replay = Some(Replay { frames: Vec::new(), idx: 0 });
            self.loading = true;
        }
    }

    pub fn exit_replay(&mut self) {
        if let Some(live) = self.live_snap.take() {
            self.snap = live;
        }
        self.replay = None;
        self.loading = false;
        if self.focused >= self.snap.boards.len() {
            self.focused = 0;
        }
        let len = self.focused_len();
        if self.selected >= len {
            self.selected = len.saturating_sub(1);
        }
    }

    pub fn is_replaying(&self) -> bool {
        self.replay.is_some()
    }

    /// Install the fetched frame index; returns the latest frame id to load.
    pub fn set_replay_frames(&mut self, frames: Vec<Frame>) -> Option<i64> {
        let r = self.replay.as_mut()?;
        if frames.is_empty() {
            return None;
        }
        r.frames = frames;
        r.idx = r.frames.len() - 1;
        r.frames.get(r.idx).map(|f| f.id)
    }

    /// Move the scrubber; returns the frame id to load when the position changes.
    pub fn replay_scrub(&mut self, delta: i32) -> Option<i64> {
        let r = self.replay.as_mut()?;
        if r.frames.is_empty() {
            return None;
        }
        let max = r.frames.len() as i32 - 1;
        let ni = (r.idx as i32 + delta).clamp(0, max) as usize;
        if ni == r.idx {
            return None;
        }
        r.idx = ni;
        r.frames.get(ni).map(|f| f.id)
    }

    /// Install a fetched historical snapshot for display.
    pub fn set_replay_snap(&mut self, hist: HistSnap) {
        self.snap = Snapshot { boards: hist.boards, ..Snapshot::default() };
        self.loading = false;
        if self.focused >= self.snap.boards.len() {
            self.focused = 0;
        }
        let len = self.focused_len();
        if self.selected >= len {
            self.selected = len.saturating_sub(1);
        }
    }

    /// Scrubber state for the replay UI: (position, total, local HH:MM:SS, train_count).
    pub fn replay_status(&self) -> Option<(usize, usize, String, i64)> {
        let r = self.replay.as_ref()?;
        let cur = r.frames.get(r.idx);
        let ts = cur.map(|f| fmt_hms(f.observed_at)).unwrap_or_else(|| "--:--:--".into());
        let count = cur.map_or(0, |f| f.train_count);
        Some((r.idx, r.frames.len(), ts, count))
    }

    pub fn toggle_voice(&mut self) {
        self.voice = !self.voice;
        if self.voice {
            let d = self.ai.dispatch.summary.trim();
            if !d.is_empty() {
                self.last_spoken = d.to_string();
                self.pending_speak = Some(d.to_string()); // read current line on enable
            }
        }
    }

    /// Queue the SITREP + event advisory to be spoken (no-op unless voice is on).
    fn speak_intel(&mut self) {
        if !self.voice {
            return;
        }
        let mut parts = Vec::new();
        let s = self.ai.sitrep.summary.trim();
        if !s.is_empty() {
            parts.push(format!("Sitrep. {s}"));
        }
        let e = self.ai.events.summary.trim();
        if !e.is_empty() {
            parts.push(format!("Event advisory. {e}"));
        }
        if !parts.is_empty() {
            self.pending_speak = Some(parts.join(" "));
        }
    }

    /// Drain a queued utterance (consumed by main → tts::speak).
    pub fn take_speak(&mut self) -> Option<String> {
        self.pending_speak.take()
    }

    /// Active alerts impacting the given route key.
    pub fn alerts_for(&self, key: &str) -> Vec<&crate::cta::Alert> {
        let key = key.to_lowercase();
        self.snap
            .alerts
            .iter()
            .filter(|a| a.routes.iter().any(|r| *r == key))
            .collect()
    }

    /// Route key the center panel should display: the zoom target if zoomed,
    /// otherwise the focused board's line.
    pub fn view_route(&self) -> Option<String> {
        if let Some(z) = &self.zoom {
            return Some(z.route.clone());
        }
        self.snap.boards.get(self.focused).map(|b| b.key.clone())
    }
}

/// Format an epoch-ms instant as local HH:MM:SS for the replay scrubber.
fn fmt_hms(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(ms).single() {
        Some(dt) => dt.format("%H:%M:%S").to_string(),
        None => "--:--:--".to_string(),
    }
}

/// Case-insensitive subsequence fuzzy match. Returns `None` if `needle` isn't a
/// subsequence of `haystack`, else a score where higher is better — rewarding
/// word-boundary hits and contiguous runs, penalizing gaps and a late start.
pub fn fuzzy_score(haystack: &str, needle: &str) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = haystack.to_lowercase().chars().collect();
    let mut hi = 0usize;
    let mut score = 0i32;
    let mut prev: Option<usize> = None;
    for nc in needle.to_lowercase().chars() {
        if nc == ' ' {
            continue;
        }
        let mut j = hi;
        let found = loop {
            if j >= hay.len() {
                return None;
            }
            if hay[j] == nc {
                break j;
            }
            j += 1;
        };
        let boundary = found == 0 || !hay[found - 1].is_alphanumeric();
        if boundary {
            score += 15;
        }
        match prev {
            Some(p) if found == p + 1 => score += 10, // contiguous run
            Some(p) => score -= ((found - p - 1).min(10)) as i32,
            None => score -= found as i32, // earlier first hit is better
        }
        score += 1;
        prev = Some(found);
        hi = found + 1;
    }
    Some(score)
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
        // Metra regional rail + NICTD South Shore (GTFS brand colors).
        "bnsf" => Color::Rgb(0x29, 0xc2, 0x33),
        "hc" => Color::Rgb(0x55, 0x0e, 0x0c),
        "md-n" => Color::Rgb(0xcc, 0x55, 0x00),
        "md-w" => Color::Rgb(0xf1, 0xad, 0x0e),
        "me" => Color::Rgb(0xeb, 0x5c, 0x00),
        "ncs" => Color::Rgb(0x97, 0x85, 0xbc),
        "ri" => Color::Rgb(0xe0, 0x24, 0x00),
        "sws" => Color::Rgb(0x00, 0x42, 0xa8),
        "up-n" => Color::Rgb(0x00, 0x80, 0x00),
        "up-nw" => Color::Rgb(0xff, 0xe6, 0x00),
        "up-w" => Color::Rgb(0xfe, 0x8d, 0x81),
        "ss" => Color::Rgb(0xf6, 0x93, 0x1c),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cta::Arrival;

    fn arr(run: &str, eta: i64) -> Arrival {
        Arrival {
            station: "Home".into(),
            run: run.into(),
            route: "G".into(),
            dest: "Loop".into(),
            eta_min: Some(eta),
            approaching: false,
            delayed: false,
        }
    }

    fn snap_with(arrivals: Vec<Arrival>) -> Snapshot {
        Snapshot { arrivals, ..Default::default() }
    }

    #[test]
    fn approach_alert_lifecycle() {
        let mut app = App::new("Home".into(), 6, false);

        // First poll already has a near train: seed silently, no bell storm.
        app.apply(snap_with(vec![arr("100", 4)]));
        assert!(!app.take_bell());
        assert_eq!(app.flash, 0);

        // Train far away: still no bell.
        app.apply(snap_with(vec![arr("200", 9)]));
        assert!(!app.take_bell());

        // It crosses into the window → ring once and flash.
        app.apply(snap_with(vec![arr("200", 5)]));
        assert!(app.take_bell());
        assert!(app.flash > 0);

        // Still near next poll → no repeat bell.
        app.apply(snap_with(vec![arr("200", 3)]));
        assert!(!app.take_bell());

        // A different train enters → ring again.
        app.apply(snap_with(vec![arr("200", 2), arr("300", 5)]));
        assert!(app.take_bell());
    }

    #[test]
    fn alert_disabled_when_zero() {
        let mut app = App::new("Home".into(), 0, false);
        app.apply(snap_with(vec![arr("100", 9)]));
        app.apply(snap_with(vec![arr("100", 1)]));
        assert!(!app.take_bell());
    }

    fn board_with(trains: Vec<crate::cta::Train>) -> Snapshot {
        Snapshot {
            boards: vec![crate::cta::RouteBoard {
                key: "g".into(),
                label: "Green".into(),
                trains,
            }],
            ..Default::default()
        }
    }

    fn train(run: &str, delayed: bool) -> crate::cta::Train {
        crate::cta::Train {
            run: run.into(),
            dest: "Loop".into(),
            next_station: "X".into(),
            eta_min: Some(5),
            approaching: false,
            delayed,
            dir: None,
            heading: None,
            lat: None,
            lon: None,
        }
    }

    #[test]
    fn delay_notifies_once_per_episode() {
        let mut app = App::new("Home".into(), 0, true);

        // First poll, train already delayed → seed silently.
        app.apply(board_with(vec![train("100", true)]));
        assert!(app.take_notes().is_empty());

        // Still delayed → no repeat.
        app.apply(board_with(vec![train("100", true)]));
        assert!(app.take_notes().is_empty());

        // A different train newly delayed → notify.
        app.apply(board_with(vec![train("100", true), train("200", true)]));
        let notes = app.take_notes();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("#200"));

        // 100 recovers then re-delays → notifies again (new episode).
        app.apply(board_with(vec![train("100", false), train("200", true)]));
        assert!(app.take_notes().is_empty());
        app.apply(board_with(vec![train("100", true), train("200", true)]));
        let notes = app.take_notes();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("#100"));
    }
}
