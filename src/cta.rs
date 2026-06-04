//! CTA Train Tracker + Customer Alerts API client.
//!
//! Native app => no CORS, no Worker proxy. The key lives in $CTA_KEY.
//! Train Tracker needs the key; the route-status feed is keyless.

use anyhow::Result;
use serde::Deserialize;

const TT_BASE: &str = "https://lapi.transitchicago.com/api/1.0";
const ALERTS_BASE: &str = "https://www.transitchicago.com/api/1.0";

/// CTA JSON (converted from XML) collapses single-element arrays into a bare
/// object, so every list field is really "one or many". This normalizes it.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T> OneOrMany<T> {
    fn into_vec(self) -> Vec<T> {
        match self {
            OneOrMany::One(x) => vec![x],
            OneOrMany::Many(v) => v,
        }
    }
}

fn flat<T>(o: Option<OneOrMany<T>>) -> Vec<T> {
    o.map(OneOrMany::into_vec).unwrap_or_default()
}

// ---------- Public, UI-facing types ----------

#[derive(Debug, Clone)]
pub struct Train {
    pub run: String,
    pub dest: String,
    pub next_station: String,
    pub eta_min: Option<i64>,
    pub approaching: bool,
    pub delayed: bool,
    pub heading: Option<u16>,
    // Reserved for fio 4 (ASCII track map): project lat/lon onto a station line.
    #[allow(dead_code)]
    pub lat: Option<f64>,
    #[allow(dead_code)]
    pub lon: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct RouteBoard {
    pub key: String,   // api route key: "red", "g", ...
    pub label: String, // "Red", "Green", ...
    pub trains: Vec<Train>,
}

#[derive(Debug, Clone)]
pub struct Arrival {
    #[allow(dead_code)] // home station is implied by the panel title for now
    pub station: String,
    pub route: String,
    pub dest: String,
    pub eta_min: Option<i64>,
    pub approaching: bool,
    pub delayed: bool,
}

#[derive(Debug, Clone)]
pub struct RouteStatus {
    pub route: String,
    pub status: String,
    pub color_hex: Option<String>,
    pub status_color_hex: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub updated: String,
    pub boards: Vec<RouteBoard>,
    pub arrivals: Vec<Arrival>,
    pub statuses: Vec<RouteStatus>,
    pub error: Option<String>,
}

// ---------- Raw wire types ----------

#[derive(Deserialize)]
struct PosResp {
    ctatt: PosCtatt,
}
#[derive(Deserialize)]
struct PosCtatt {
    tmst: Option<String>,
    #[serde(rename = "errCd")]
    err_cd: Option<String>,
    #[serde(rename = "errNm")]
    err_nm: Option<String>,
    route: Option<OneOrMany<RawRoute>>,
}
#[derive(Deserialize)]
struct RawRoute {
    #[serde(rename = "@name")]
    name: String,
    train: Option<OneOrMany<RawTrain>>,
}
#[derive(Deserialize)]
struct RawTrain {
    rn: Option<String>,
    #[serde(rename = "destNm")]
    dest_nm: Option<String>,
    #[serde(rename = "nextStaNm")]
    next_sta_nm: Option<String>,
    #[serde(rename = "arrT")]
    arr_t: Option<String>,
    #[serde(rename = "isApp")]
    is_app: Option<String>,
    #[serde(rename = "isDly")]
    is_dly: Option<String>,
    lat: Option<String>,
    lon: Option<String>,
    heading: Option<String>,
}

#[derive(Deserialize)]
struct ArrResp {
    ctatt: ArrCtatt,
}
#[derive(Deserialize)]
struct ArrCtatt {
    eta: Option<OneOrMany<RawEta>>,
}
#[derive(Deserialize)]
struct RawEta {
    #[serde(rename = "staNm")]
    sta_nm: Option<String>,
    rt: Option<String>,
    #[serde(rename = "destNm")]
    dest_nm: Option<String>,
    #[serde(rename = "arrT")]
    arr_t: Option<String>,
    #[serde(rename = "isApp")]
    is_app: Option<String>,
    #[serde(rename = "isDly")]
    is_dly: Option<String>,
}

#[derive(Deserialize)]
struct RoutesResp {
    #[serde(rename = "CTARoutes")]
    cta_routes: CtaRoutes,
}
#[derive(Deserialize)]
struct CtaRoutes {
    #[serde(rename = "RouteInfo")]
    route_info: Option<OneOrMany<RawRouteInfo>>,
}
#[derive(Deserialize)]
struct RawRouteInfo {
    #[serde(rename = "Route")]
    route: String,
    #[serde(rename = "RouteColorCode")]
    color: Option<String>,
    #[serde(rename = "RouteStatus")]
    status: Option<String>,
    #[serde(rename = "RouteStatusColor")]
    status_color: Option<String>,
}

// ---------- Helpers ----------

fn truthy(s: &Option<String>) -> bool {
    matches!(s.as_deref(), Some("1") | Some("true"))
}

/// CTA arrival timestamps look like "20240101 14:30:00" in local Chicago time.
/// We treat "now" as local naive time and diff in whole minutes.
fn eta_minutes(arr_t: &Option<String>) -> Option<i64> {
    let s = arr_t.as_deref()?;
    let parsed = chrono::NaiveDateTime::parse_from_str(s, "%Y%m%d %H:%M:%S")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
        .ok()?;
    let now = chrono::Local::now().naive_local();
    let mins = (parsed - now).num_seconds() as f64 / 60.0;
    Some(mins.round() as i64)
}

// ---------- Client ----------

pub struct Cta {
    http: reqwest::Client,
    key: String,
}

impl Cta {
    pub fn new(key: String) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("cta-tui/0.1 (+terminal)")
            .build()
            .expect("http client");
        Self { http, key }
    }

    /// Pull positions for the given routes, arrivals for the home station, and
    /// the keyless system-wide route status, into one Snapshot.
    pub async fn snapshot(&self, routes: &[&str], home_mapid: &str) -> Snapshot {
        let mut snap = Snapshot {
            updated: chrono::Local::now().format("%H:%M:%S").to_string(),
            ..Default::default()
        };

        match self.positions(routes).await {
            Ok(b) => snap.boards = b,
            Err(e) => snap.error = Some(format!("positions: {e}")),
        }
        match self.arrivals(home_mapid).await {
            Ok(a) => snap.arrivals = a,
            Err(e) => {
                if snap.error.is_none() {
                    snap.error = Some(format!("arrivals: {e}"));
                }
            }
        }
        if let Ok(s) = self.statuses().await {
            snap.statuses = s;
        }
        snap
    }

    async fn positions(&self, routes: &[&str]) -> Result<Vec<RouteBoard>> {
        let url = format!(
            "{TT_BASE}/ttpositions.aspx?key={}&rt={}&outputType=JSON",
            self.key,
            routes.join(",")
        );
        let resp: PosResp = self.http.get(url).send().await?.json().await?;
        if resp.ctatt.err_cd.as_deref().unwrap_or("0") != "0" {
            anyhow::bail!(resp.ctatt.err_nm.unwrap_or_else(|| "CTA error".into()));
        }
        let _ = resp.ctatt.tmst;

        let boards = flat(resp.ctatt.route)
            .into_iter()
            .map(|r| {
                let trains = flat(r.train)
                    .into_iter()
                    .map(|t| Train {
                        run: t.rn.unwrap_or_default(),
                        dest: t.dest_nm.unwrap_or_default(),
                        next_station: t.next_sta_nm.unwrap_or_default(),
                        eta_min: eta_minutes(&t.arr_t),
                        approaching: truthy(&t.is_app),
                        delayed: truthy(&t.is_dly),
                        heading: t.heading.and_then(|h| h.parse().ok()),
                        lat: t.lat.and_then(|v| v.parse().ok()),
                        lon: t.lon.and_then(|v| v.parse().ok()),
                    })
                    .collect();
                RouteBoard {
                    label: pretty_route(&r.name),
                    key: r.name,
                    trains,
                }
            })
            .collect();
        Ok(boards)
    }

    async fn arrivals(&self, mapid: &str) -> Result<Vec<Arrival>> {
        let url = format!(
            "{TT_BASE}/ttarrivals.aspx?key={}&mapid={mapid}&outputType=JSON",
            self.key
        );
        let resp: ArrResp = self.http.get(url).send().await?.json().await?;
        let arrivals = flat(resp.ctatt.eta)
            .into_iter()
            .map(|e| Arrival {
                station: e.sta_nm.unwrap_or_default(),
                route: e.rt.unwrap_or_default(),
                dest: e.dest_nm.unwrap_or_default(),
                eta_min: eta_minutes(&e.arr_t),
                approaching: truthy(&e.is_app),
                delayed: truthy(&e.is_dly),
            })
            .collect();
        Ok(arrivals)
    }

    async fn statuses(&self) -> Result<Vec<RouteStatus>> {
        let url = format!("{ALERTS_BASE}/routes.aspx?outputType=JSON");
        let resp: RoutesResp = self.http.get(url).send().await?.json().await?;
        let statuses = flat(resp.cta_routes.route_info)
            .into_iter()
            .filter(|r| is_rail_line(&r.route))
            .map(|r| RouteStatus {
                route: r.route,
                status: r.status.unwrap_or_else(|| "—".into()),
                color_hex: r.color,
                status_color_hex: r.status_color,
            })
            .collect();
        Ok(statuses)
    }
}

/// `routes.aspx` returns all ~130 CTA routes (bus + rail). The command board
/// only flies the 8 'L' lines, so keep just those.
fn is_rail_line(name: &str) -> bool {
    matches!(
        name.trim().to_lowercase().as_str(),
        "red line"
            | "blue line"
            | "brown line"
            | "green line"
            | "orange line"
            | "purple line"
            | "purple line express"
            | "pink line"
            | "yellow line"
    )
}

pub fn pretty_route(key: &str) -> String {
    match key.to_lowercase().as_str() {
        "red" => "Red",
        "blue" => "Blue",
        "brn" => "Brown",
        "g" => "Green",
        "org" => "Orange",
        "p" | "pexp" => "Purple",
        "pink" => "Pink",
        "y" => "Yellow",
        other => other,
    }
    .to_string()
}
