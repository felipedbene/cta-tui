//! fio 4 — ASCII track map geometry.
//!
//! `track.json` (baked by `scripts/build_track.mjs` from the Worker's
//! `lines.geojson` + `ctaData.js`) gives, per route, the concatenated rail
//! polyline and the set of stations. At load we order each line's stations by
//! projecting them onto the polyline; at render we project live train lat/lon
//! the same way to get a 0..1 position along the rail.
//!
//! The rail is drawn as a straight strip, so only the *along-line* position
//! matters — geographic shape is discarded. Branched lines (e.g. Green) are
//! concatenated into one polyline, so branch trains snap to the nearest trunk
//! point; good enough for a 1-D strip.

use serde::Deserialize;
use std::collections::HashMap;

const TRACK_JSON: &str = include_str!("track.json");

// Chicago is ~41.9°N; a fixed cos(lat) keeps the lon/lat planar projection
// cheap and accurate enough at city scale for nearest-point math.
const COS_LAT: f64 = 0.743;

#[derive(Deserialize)]
struct RawTrack {
    polyline: Vec<[f64; 2]>, // [lon, lat]
    stations: Vec<RawStation>,
}
#[derive(Deserialize)]
struct RawStation {
    name: String,
    lat: f64,
    lon: f64,
}

#[derive(Clone)]
pub struct TrackStation {
    pub name: String,
    pub pos01: f64,
}

/// Result of projecting a point onto the rail: fractional position plus the
/// unit direction of the rail at that point (planar x,y), so callers can tell
/// which way along the strip a train with a known compass heading is moving.
pub struct Proj {
    pub pos01: f64,
    pub seg: (f64, f64),
    pub dist2: f64, // squared planar distance to the rail (for branch assignment)
}

pub struct RouteTrack {
    pts: Vec<(f64, f64)>, // planar (x, y)
    cum: Vec<f64>,        // cumulative arc length to each vertex
    total: f64,
    pub stations: Vec<TrackStation>, // ordered by pos01
}

/// A searchable reference to one station on one line.
#[derive(Clone)]
pub struct StationRef {
    pub name: String,
    pub route: String,
    pub branch: usize, // which branch of the route
    pub index: usize,  // position in that branch's ordered station list
}

pub struct TrackMap {
    routes: HashMap<String, Vec<RouteTrack>>, // one or more branches per route
    index: Vec<StationRef>,                   // flat, for fuzzy search
}

fn planar(lon: f64, lat: f64) -> (f64, f64) {
    (lon * COS_LAT, lat)
}

impl RouteTrack {
    fn build(raw: RawTrack) -> Self {
        let pts: Vec<(f64, f64)> = raw.polyline.iter().map(|c| planar(c[0], c[1])).collect();
        let mut cum = Vec::with_capacity(pts.len());
        let mut total = 0.0;
        for (i, p) in pts.iter().enumerate() {
            if i > 0 {
                let q = pts[i - 1];
                total += ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt();
            }
            cum.push(total);
        }
        let mut rt = RouteTrack { pts, cum, total: total.max(1e-9), stations: Vec::new() };
        let mut stations: Vec<TrackStation> = raw
            .stations
            .iter()
            .filter_map(|s| rt.project(s.lat, s.lon).map(|p| TrackStation { name: s.name.clone(), pos01: p.pos01 }))
            .collect();
        stations.sort_by(|a, b| a.pos01.partial_cmp(&b.pos01).unwrap_or(std::cmp::Ordering::Equal));
        rt.stations = stations;
        rt
    }

    /// Nearest point on the rail → fractional position + local rail direction.
    pub fn project(&self, lat: f64, lon: f64) -> Option<Proj> {
        if self.pts.len() < 2 {
            return None;
        }
        let (px, py) = planar(lon, lat);
        let mut best_d2 = f64::INFINITY;
        let mut best_along = 0.0;
        let mut best_seg = (1.0, 0.0);
        for i in 1..self.pts.len() {
            let (ax, ay) = self.pts[i - 1];
            let (bx, by) = self.pts[i];
            let (dx, dy) = (bx - ax, by - ay);
            let seg2 = dx * dx + dy * dy;
            let t = if seg2 <= 1e-18 {
                0.0
            } else {
                (((px - ax) * dx + (py - ay) * dy) / seg2).clamp(0.0, 1.0)
            };
            let (cx, cy) = (ax + t * dx, ay + t * dy);
            let d2 = (px - cx).powi(2) + (py - cy).powi(2);
            if d2 < best_d2 {
                best_d2 = d2;
                let seg_len = seg2.sqrt();
                best_along = self.cum[i - 1] + t * seg_len;
                best_seg = if seg_len > 1e-12 {
                    (dx / seg_len, dy / seg_len)
                } else {
                    (1.0, 0.0)
                };
            }
        }
        Some(Proj {
            pos01: (best_along / self.total).clamp(0.0, 1.0),
            seg: best_seg,
            dist2: best_d2,
        })
    }

    /// Map a raw along-line position (0..1) to *station space* (0..1), where
    /// stations are evenly spaced. This is the strip-map warp: it spreads the
    /// geographically-dense stretches out so the diagram is legible and a train
    /// always sits proportionally between its two bracketing stations.
    pub fn pos_to_slot(&self, p: f64) -> f64 {
        let n = self.stations.len();
        if n < 2 {
            return 0.0;
        }
        if p <= self.stations[0].pos01 {
            return 0.0;
        }
        if p >= self.stations[n - 1].pos01 {
            return 1.0;
        }
        for j in 0..n - 1 {
            let (a, b) = (self.stations[j].pos01, self.stations[j + 1].pos01);
            if p >= a && p <= b {
                let frac = if b > a { (p - a) / (b - a) } else { 0.0 };
                return (j as f64 + frac) / (n - 1) as f64;
            }
        }
        1.0
    }

    /// Continuous station-index of a raw position (0..n-1), for the zoom window.
    pub fn pos_to_index(&self, p: f64) -> f64 {
        self.pos_to_slot(p) * (self.stations.len().saturating_sub(1)) as f64
    }
}

impl TrackMap {
    /// Parse the baked asset. Panics on a malformed embed — it ships with the binary.
    pub fn load() -> Self {
        let raw: HashMap<String, Vec<RawTrack>> =
            serde_json::from_str(TRACK_JSON).expect("track.json is a valid baked asset");
        let routes: HashMap<String, Vec<RouteTrack>> = raw
            .into_iter()
            .map(|(k, brs)| (k, brs.into_iter().map(RouteTrack::build).collect()))
            .collect();

        // Flat, searchable index of every station (sorted by route for stability).
        // A station name shared by branches (the trunk) is indexed once, on the
        // first branch that has it, so search shows it a single time per line.
        let mut keys: Vec<&String> = routes.keys().collect();
        keys.sort();
        let mut index = Vec::new();
        for k in keys {
            let mut seen = std::collections::HashSet::new();
            for (b, branch) in routes[k].iter().enumerate() {
                for (i, s) in branch.stations.iter().enumerate() {
                    if seen.insert(s.name.to_lowercase()) {
                        index.push(StationRef {
                            name: s.name.clone(),
                            route: k.clone(),
                            branch: b,
                            index: i,
                        });
                    }
                }
            }
        }
        TrackMap { routes, index }
    }

    /// All branches of a route (primary first).
    pub fn branches(&self, key: &str) -> &[RouteTrack] {
        self.routes.get(&key.to_lowercase()).map_or(&[], |b| b.as_slice())
    }

    pub fn station_index(&self) -> &[StationRef] {
        &self.index
    }
}
