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
    pub dir: Option<String>, // trDr: trip direction ("1"/"5"), splits the two rails
    // fio 4 (ASCII track map): projected onto a station line at render time.
    pub lat: Option<f64>,
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
    pub run: String, // train run number, for de-duping approach alerts
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

#[derive(Debug, Clone)]
pub struct Alert {
    pub headline: String,
    pub short: String,
    pub impact: String,
    pub major: bool,
    pub routes: Vec<String>, // impacted rail route keys (lowercased: g, brn, ...)
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub updated: String,
    pub boards: Vec<RouteBoard>,
    pub arrivals: Vec<Arrival>,
    pub statuses: Vec<RouteStatus>,
    pub alerts: Vec<Alert>,
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
    #[serde(rename = "trDr")]
    tr_dr: Option<String>,
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
    rn: Option<String>,
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

#[derive(Deserialize)]
struct AlertsResp {
    #[serde(rename = "CTAAlerts")]
    cta_alerts: CtaAlerts,
}
#[derive(Deserialize)]
struct CtaAlerts {
    #[serde(rename = "Alert")]
    alert: Option<OneOrMany<RawAlert>>,
}
#[derive(Deserialize)]
struct RawAlert {
    #[serde(rename = "Headline")]
    headline: Option<String>,
    #[serde(rename = "ShortDescription")]
    short_description: Option<String>,
    #[serde(rename = "Impact")]
    impact: Option<String>,
    #[serde(rename = "MajorAlert")]
    major_alert: Option<String>,
    #[serde(rename = "ImpactedService")]
    impacted_service: Option<RawImpacted>,
}
#[derive(Deserialize)]
struct RawImpacted {
    #[serde(rename = "Service")]
    service: Option<OneOrMany<RawService>>,
}
#[derive(Deserialize)]
struct RawService {
    #[serde(rename = "ServiceType")]
    service_type: Option<String>,
    #[serde(rename = "ServiceId")]
    service_id: Option<String>,
}

// ---------- Metra / South Shore wire types (Worker JSON) ----------

#[derive(Deserialize)]
struct MetraResp {
    #[serde(default)]
    trains: Vec<MetraTrain>,
}
#[derive(Deserialize)]
struct MetraTrain {
    label: Option<String>, // train number, used as the run id
    route: Option<String>, // Metra route id (BNSF, UP-N, ...)
    lat: Option<f64>,
    lon: Option<f64>,
    heading: Option<f64>,
}

#[derive(Deserialize)]
struct SsResp {
    #[serde(default)]
    trains: Vec<SsTrain>,
}
#[derive(Deserialize)]
struct SsTrain {
    label: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
    heading: Option<f64>,
}

#[derive(Deserialize)]
struct MetraAlertsResp {
    #[serde(default)]
    alerts: Vec<MetraAlert>,
}
#[derive(Deserialize)]
struct MetraAlert {
    header: Option<String>,
    description: Option<String>,
    effect: Option<String>, // GTFS-rt effect enum name (e.g. SIGNIFICANT_DELAYS)
    #[serde(default)]
    routes: Vec<String>,
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
    base: String, // Worker base URL for Metra/South Shore JSON (see ai::base())
    metra: bool,  // include Metra + South Shore boards (CTA_METRA opt-out)
}

impl Cta {
    pub fn new(key: String, base: String, metra: bool) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("cta-tui/0.1 (+terminal)")
            .build()
            .expect("http client");
        Self { http, key, base, metra }
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
        if let Ok(a) = self.alerts(routes).await {
            snap.alerts = a;
        }

        // Metra regional rail + NICTD South Shore, via the Worker's decoded JSON
        // (the Metra token stays server-side). All non-fatal: a Worker hiccup
        // must not blank the CTA board, so each failure just omits its lines.
        if self.metra {
            let metra_alerts = self.metra_alerts().await.unwrap_or_default();
            if let Ok(boards) = self.metra_positions().await {
                for b in &boards {
                    snap.statuses.push(metra_status(&b.key, &metra_alerts));
                }
                snap.boards.extend(boards);
            }
            if let Ok(b) = self.southshore_positions().await {
                snap.statuses.push(regional_status("SS", "ss", None));
                snap.boards.push(b);
            }
            snap.alerts.extend(metra_alerts);
        }
        snap
    }

    /// Metra vehicle positions (Worker JSON) → one board per Metra line, grouped
    /// by `route` id. The feed carries no ETA/next-stop, so those stay empty; the
    /// board shows the run/label, heading arrow, and strip-map position.
    async fn metra_positions(&self) -> Result<Vec<RouteBoard>> {
        let url = format!("{}/api/metra/positions", self.base);
        let resp: MetraResp = self.http.get(url).send().await?.json().await?;
        Ok(metra_boards(resp))
    }

    /// South Shore vehicle positions (Worker JSON). The realtime feed is
    /// route-less, so every train lands in a single "ss" board.
    async fn southshore_positions(&self) -> Result<RouteBoard> {
        let url = format!("{}/api/southshore/positions", self.base);
        let resp: SsResp = self.http.get(url).send().await?.json().await?;
        Ok(ss_board(resp))
    }

    /// Active Metra service alerts (Worker JSON, decoded from GTFS-realtime).
    /// Tagged with the lowercased route ids they impact so `App::alerts_for`
    /// matches the Metra board keys.
    async fn metra_alerts(&self) -> Result<Vec<Alert>> {
        let url = format!("{}/api/metra/alerts", self.base);
        let resp: MetraAlertsResp = self.http.get(url).send().await?.json().await?;
        Ok(resp
            .alerts
            .into_iter()
            .filter(|a| a.header.as_deref().is_some_and(|h| !h.is_empty()))
            .map(|a| Alert {
                routes: a.routes.iter().map(|r| r.to_lowercase()).collect(),
                headline: a.header.unwrap_or_default(),
                short: a.description.unwrap_or_default(),
                impact: a.effect.map(pretty_effect).unwrap_or_default(),
                major: false,
            })
            .collect())
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
        Ok(boards_from_ctatt(resp.ctatt))
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
                run: e.rn.unwrap_or_default(),
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

    /// Active Customer Alerts for the given routes (keyless). Each alert is
    /// tagged with the rail lines it impacts (ServiceType "R"), so the UI can
    /// filter to the focused line. We keep the plain ShortDescription and skip
    /// the CDATA/HTML FullDescription.
    async fn alerts(&self, routes: &[&str]) -> Result<Vec<Alert>> {
        let url = format!(
            "{ALERTS_BASE}/alerts.aspx?activeonly=true&routeid={}&outputType=JSON",
            routes.join(",")
        );
        let resp: AlertsResp = self.http.get(url).send().await?.json().await?;
        let alerts = flat(resp.cta_alerts.alert)
            .into_iter()
            .map(|a| {
                let routes = flat(a.impacted_service.and_then(|s| s.service))
                    .into_iter()
                    .filter(|s| s.service_type.as_deref() == Some("R"))
                    .filter_map(|s| s.service_id.map(|id| id.to_lowercase()))
                    .collect();
                Alert {
                    headline: a.headline.unwrap_or_default(),
                    short: a.short_description.unwrap_or_default(),
                    impact: a.impact.unwrap_or_default(),
                    major: truthy(&a.major_alert),
                    routes,
                }
            })
            .collect();
        Ok(alerts)
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
        // Metra regional rail + NICTD South Shore.
        "bnsf" => "BNSF",
        "hc" => "Heritage",
        "md-n" => "MD-N",
        "md-w" => "MD-W",
        "me" => "Metra Electric",
        "ncs" => "NCS",
        "ri" => "Rock Island",
        "sws" => "SW Service",
        "up-n" => "UP-N",
        "up-nw" => "UP-NW",
        "up-w" => "UP-W",
        "ss" => "South Shore",
        other => other,
    }
    .to_string()
}

/// Brand hex for a Metra/South Shore line (from the GTFS route colors baked into
/// metra.geojson / southshore.geojson). Used to color the synthesized SYSTEM rows.
pub fn regional_hex(key: &str) -> Option<&'static str> {
    Some(match key.to_lowercase().as_str() {
        "bnsf" => "#29C233",
        "hc" => "#550E0C",
        "md-n" => "#CC5500",
        "md-w" => "#F1AD0E",
        "me" => "#EB5C00",
        "ncs" => "#9785BC",
        "ri" => "#E02400",
        "sws" => "#0042A8",
        "up-n" => "#008000",
        "up-nw" => "#FFE600",
        "up-w" => "#FE8D81",
        "ss" => "#F6931C",
        _ => return None,
    })
}

/// Group a Metra positions response into one board per route id (shared by the
/// live fetch and historical replay decode).
fn metra_boards(resp: MetraResp) -> Vec<RouteBoard> {
    let mut by: std::collections::BTreeMap<String, Vec<Train>> = Default::default();
    for t in resp.trains {
        let Some(route) = t.route.filter(|r| !r.is_empty()) else { continue };
        by.entry(route.to_lowercase()).or_default().push(regional_train(t.label, t.heading, t.lat, t.lon));
    }
    by.into_iter()
        .map(|(key, trains)| RouteBoard { label: pretty_route(&key), key, trains })
        .collect()
}

/// The single route-less South Shore board (shared by live fetch + replay decode).
fn ss_board(resp: SsResp) -> RouteBoard {
    let trains = resp
        .trains
        .into_iter()
        .map(|t| regional_train(t.label, t.heading, t.lat, t.lon))
        .collect();
    RouteBoard { key: "ss".into(), label: pretty_route("ss"), trains }
}

/// Build one regional (Metra/SS) train. These feeds carry no ETA, next-stop, or
/// rail-direction, so those default to empty; lat/lon/heading drive the strip map.
fn regional_train(label: Option<String>, heading: Option<f64>, lat: Option<f64>, lon: Option<f64>) -> Train {
    Train {
        run: label.unwrap_or_default(),
        dest: String::new(),
        next_station: String::new(),
        eta_min: None,
        approaching: false,
        delayed: false,
        dir: None,
        heading: heading.map(|h| h.round().rem_euclid(360.0) as u16),
        lat,
        lon,
    }
}

/// Synthesize a SYSTEM-panel status row for a regional line. `display` is the
/// short label (e.g. "UP-N"); `key` keys the brand color; `effect`, when present,
/// flags the line as alerted so the panel shows it amber instead of nominal.
fn regional_status(display: &str, key: &str, effect: Option<&str>) -> RouteStatus {
    let status = match effect {
        Some(e) if !e.is_empty() => e.to_string(),
        _ => "Normal Service".to_string(), // "normal" → nominal styling in the SYSTEM panel
    };
    let status_color_hex = Some(if effect.is_some() { "#f9a825" } else { "#00b300" }.to_string());
    RouteStatus {
        route: display.to_string(),
        status,
        color_hex: regional_hex(key).map(String::from),
        status_color_hex,
    }
}

/// Status row for a Metra line, flagged by its most relevant active alert (if any).
fn metra_status(key: &str, alerts: &[Alert]) -> RouteStatus {
    let k = key.to_lowercase();
    let effect = alerts
        .iter()
        .find(|a| a.routes.contains(&k))
        .map(|a| if a.impact.is_empty() { "Alert" } else { a.impact.as_str() });
    regional_status(&key.to_uppercase(), &k, effect)
}

/// GTFS-realtime effect enum name → human label ("SIGNIFICANT_DELAYS" → "Significant Delays").
fn pretty_effect(e: String) -> String {
    e.split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------- History / replay (Worker-backed) ----------

/// Map a decoded ttpositions `ctatt` into route boards (shared by the live fetch
/// and historical replay snapshots).
fn boards_from_ctatt(ctatt: PosCtatt) -> Vec<RouteBoard> {
    flat(ctatt.route)
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
                    dir: t.tr_dr,
                    heading: t.heading.and_then(|h| h.parse().ok()),
                    lat: t.lat.and_then(|v| v.parse().ok()),
                    lon: t.lon.and_then(|v| v.parse().ok()),
                })
                .collect();
            RouteBoard { label: pretty_route(&r.name), key: r.name, trains }
        })
        .collect()
}

/// One row of the replay timeline.
#[derive(Clone, Deserialize)]
pub struct Frame {
    pub id: i64,
    pub observed_at: i64, // epoch ms
    #[serde(default)]
    pub train_count: i64,
}

#[derive(Deserialize)]
struct IndexResp {
    frames: Vec<Frame>,
}

#[derive(Deserialize)]
struct HistSnapResp {
    payload: PosResp,
    // Captured into the same snapshot row by the Worker cron; null on pre-0007
    // history rows (and absent from old deployments), so both are optional.
    #[serde(default)]
    metra: Option<MetraResp>,
    #[serde(default)]
    southshore: Option<SsResp>,
}

/// A historical snapshot decoded into renderable boards.
pub struct HistSnap {
    pub boards: Vec<RouteBoard>,
}

/// Fetch the replay frame index over [from_ms, to_ms] from the Worker.
pub async fn history_index(http: &reqwest::Client, base: &str, from_ms: i64, to_ms: i64) -> Result<Vec<Frame>> {
    let url = format!("{base}/api/history/index?from={from_ms}&to={to_ms}");
    let r: IndexResp = http.get(url).send().await?.json().await?;
    Ok(r.frames)
}

/// Fetch one historical snapshot and decode it into boards. When `metra` is set,
/// the Metra + South Shore payloads captured in the same row are folded in too,
/// so replay shows the same line set as the live view (otherwise they desync).
pub async fn history_snapshot(http: &reqwest::Client, base: &str, id: i64, metra: bool) -> Result<HistSnap> {
    let url = format!("{base}/api/history/snapshot?id={id}");
    let r: HistSnapResp = http.get(url).send().await?.json().await?;
    let mut boards = boards_from_ctatt(r.payload.ctatt);
    if metra {
        if let Some(m) = r.metra {
            boards.extend(metra_boards(m));
        }
        if let Some(s) = r.southshore {
            boards.push(ss_board(s));
        }
    }
    Ok(HistSnap { boards })
}
