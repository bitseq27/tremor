//! Network domain: isolated tokio worker thread that lives on the EMSC-CSEM
//! realtime socket (`wss://cobra.emsc-csem.org/seismologist` - the exact
//! source the public emsc-csem.org event pages are driven by), cleans payload
//! variations, filters sub-threshold noise and ships normalised events across
//! a thread-safe channel.
//!
//! Behaviour:
//! * on connect the worker back-fills the last 48 h by walking the paged
//!   `get.geojson` list (page 1 ~= last 8 h, ~200 events each) until the
//!   payload reaches past the 48 h cut-off;
//! * afterwards the socket pushes `INSERT` / `UPDATE` messages within seconds
//!   of a new solution, and page 1 is re-requested every 60 s to reconcile
//!   revisions; a 30 s `get.timeUTC` keeps the socket alive;
//! * if the EMSC socket is unreachable the worker degrades to the USGS
//!   `all_hour.geojson` HTTP feed (previous primary source) and keeps trying
//!   EMSC with capped exponential backoff;
//! * guardrail 3: every error path here is contained in the worker; the render
//!   thread only ever sees `NetEvent`s and therefore keeps drawing. After 45 s
//!   without any successful live fetch a labelled deterministic SIM feed keeps
//!   the globe alive.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;

use crate::model::Quake;
use crate::ws::WsClient;
use crate::MAG_FILTER;

pub const FEED_URL: &str =
    "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/all_hour.geojson";
pub const POLL_EVERY: Duration = Duration::from_secs(60);
/// after this long without a single successful live fetch the worker starts
/// synthesising a demo feed so the globe is never empty (clearly labelled SIM)
pub const SIM_FALLBACK_AFTER: Duration = Duration::from_secs(45);

/// EMSC-CSEM realtime socket (same endpoint as www.emsc-csem.org pages).
pub const EMSC_WS_HOST: &str = "cobra.emsc-csem.org";
pub const EMSC_WS_PATH: &str = "/seismologist";
pub const EMSC_WS_ORIGIN: &str = "https://www.emsc-csem.org";
/// how many paged list responses to walk while back-filling the 48 h window
pub const EMSC_MAX_BACKFILL_PAGES: u32 = 14;
const KEEPALIVE_EVERY: Duration = Duration::from_secs(30);
const RECV_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Live,
    Sim,
    Off,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NetEvent {
    Batch(Vec<Quake>),
    Link(LinkState),
    /// human-readable name of the source currently feeding events
    Source(&'static str),
}

pub const SOURCE_EMSC: &str = "EMSC CSEM RT";
pub const SOURCE_USGS: &str = "USGS ALL_HOUR";

// ---- USGS GeoJSON schema (tolerant: every field optional) ------------------

#[derive(Debug, Deserialize)]
struct Feed {
    #[serde(default)]
    features: Vec<Feature>,
}

#[derive(Debug, Deserialize)]
struct Feature {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    properties: Option<Props>,
    #[serde(default)]
    geometry: Option<Geom>,
}

#[derive(Debug, Default, Deserialize)]
struct Props {
    #[serde(default)]
    mag: Option<f64>,
    #[serde(default)]
    place: Option<String>,
    #[serde(default)]
    time: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct Geom {
    #[serde(default)]
    coordinates: Vec<f64>,
}

/// Parse + normalise + filter one USGS feed payload. Pure, hence unit-testable.
pub fn parse_feed(body: &str) -> Vec<Quake> {
    let feed: Feed = match serde_json::from_str(body) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for (i, f) in feed.features.iter().enumerate() {
        let props = match f.properties.as_ref() {
            Some(p) => p,
            None => continue,
        };
        let mag = match props.mag {
            Some(m) if m.is_finite() => m as f32,
            _ => continue,
        };
        // network-level noise filter (guardrail 1)
        if mag < MAG_FILTER {
            continue;
        }
        let geo = match f.geometry.as_ref() {
            Some(g) if g.coordinates.len() >= 3 => g,
            Some(g) if g.coordinates.len() == 2 => g,
            _ => continue,
        };
        let lon = geo.coordinates.first().copied().unwrap_or(0.0) as f32;
        let lat = geo.coordinates.get(1).copied().unwrap_or(0.0) as f32;
        let depth = geo.coordinates.get(2).copied().unwrap_or(0.0) as f32;
        if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90.0 || lon.abs() > 180.0 {
            continue;
        }
        out.push(Quake {
            id: f
                .id
                .clone()
                .unwrap_or_else(|| format!("anon-{i}-{}", props.time.unwrap_or(0))),
            mag,
            place: props
                .place
                .clone()
                .unwrap_or_else(|| "UNKNOWN REGION".into())
                .to_uppercase(),
            time_ms: props.time.unwrap_or_else(crate::timeutil::now_millis),
            lat,
            lon,
            depth_km: depth.max(0.0),
            simulated: false,
        });
    }
    out
}

// ---- EMSC-CSEM schema ------------------------------------------------------

/// Normalise one EMSC event object (list entry or INSERT/UPDATE push).
pub fn parse_emsc_event(e: &Value) -> Option<Quake> {
    let mag = e.get("mag").and_then(Value::as_f64)?;
    if !mag.is_finite() || (mag as f32) < MAG_FILTER {
        return None;
    }
    let lat = e.get("lat").and_then(Value::as_f64)? as f32;
    let lon = e.get("lon").and_then(Value::as_f64)? as f32;
    if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90.0 || lon.abs() > 180.0 {
        return None;
    }
    let time_ms = e.get("timest").and_then(Value::as_i64)?;
    let evid = e
        .get("evid")
        .and_then(Value::as_i64)
        .map(|v| v.to_string())
        .or_else(|| e.get("fid").and_then(Value::as_str).map(str::to_string))?;
    let depth = e
        .get("depth")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .max(0.0) as f32;
    let place = e
        .get("reg")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN REGION")
        .to_uppercase();
    Some(Quake {
        id: format!("emsc-{evid}"),
        mag: mag as f32,
        place,
        time_ms,
        lat,
        lon,
        depth_km: depth,
        simulated: false,
    })
}

/// Parse a `full_json` paged-list payload: (events, page number, oldest ts).
pub fn parse_emsc_page(v: &Value) -> (Vec<Quake>, Option<u32>, Option<i64>) {
    let data = match v.get("data") {
        Some(d) => d,
        None => return (Vec::new(), None, None),
    };
    let page = data
        .get("tbiconf")
        .and_then(|t| t.get("view"))
        .and_then(Value::as_u64)
        .map(|p| p as u32);
    let feats = data.get("features").and_then(Value::as_array);
    let mut out = Vec::new();
    let mut oldest: Option<i64> = None;
    if let Some(feats) = feats {
        for f in feats {
            if let Some(q) = parse_emsc_event(f) {
                oldest = Some(match oldest {
                    Some(o) => o.min(q.time_ms),
                    None => q.time_ms,
                });
                out.push(q);
            }
        }
    }
    (out, page, oldest)
}

// ---- offline simulation feed ----------------------------------------------

const ZONES: &[(&str, f32, f32)] = &[
    ("OFF E HONSHU, JAPAN", 38.3, 142.4),
    ("VALPARAISO, CHILE", -33.4, -71.6),
    ("SUMATRA, INDONESIA", -2.5, 101.9),
    ("ALEUTIAN ISLANDS, ALASKA", 51.9, -176.2),
    ("GUERRERO, MEXICO", 16.8, -99.9),
    ("TONGA TRENCH", -20.6, -173.9),
    ("REYKJANES, ICELAND", 64.0, -22.4),
    ("EAST ANATOLIA, TURKEY", 38.9, 43.4),
    ("LEYTE, PHILIPPINES", 11.9, 125.0),
    ("KERMADAC ISLANDS", -30.0, -177.8),
    ("KAMCHATKA PENINSULA", 52.6, 160.0),
    ("VANUATU REGION", -16.4, 167.9),
    ("MID-ATLANTIC RIDGE", 0.7, -29.9),
    ("HINDU KUSH, AFGHANISTAN", 36.5, 70.7),
    ("NEPAL HIMALAYA", 28.5, 84.7),
    ("PUERTO RICO TRENCH", 19.2, -66.9),
    ("SUNDA STRAIT, INDONESIA", -6.5, 105.5),
    ("SOUTH OF FIJI", -23.9, 179.9),
    ("CRETE, GREECE", 35.2, 25.7),
    ("NORTH ISLAND, NEW ZEALAND", -41.9, 174.4),
];

/// Deterministic tiny LCG so sim runs are reproducible per seed.
#[derive(Debug, Clone)]
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_f32(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
    }
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + self.next_f32() * (b - a)
    }
}

/// Gutenberg-Richter-ish magnitude draw: many small, few large.
pub fn sim_magnitude(r: &mut Rng) -> f32 {
    let u = 1.0 - r.next_f32().clamp(0.0, 0.999);
    (4.0 - u.ln() * 0.85).clamp(MAG_FILTER, 7.8)
}

pub fn sim_event(r: &mut Rng, seq: u32) -> Quake {
    let (name, lat, lon) = ZONES[(r.next_f32() * ZONES.len() as f32) as usize % ZONES.len()];
    let depth = if r.next_f32() < 0.7 {
        r.range(8.0, 70.0)
    } else {
        r.range(70.0, 620.0)
    };
    Quake {
        id: format!("sim-{seq:06}"),
        mag: (sim_magnitude(r) * 10.0).round() / 10.0,
        place: format!("SIM {name}"),
        time_ms: crate::timeutil::now_millis(),
        lat: (lat + r.range(-2.2, 2.2)).clamp(-89.5, 89.5),
        lon: ((lon + r.range(-2.2, 2.2) + 540.0) % 360.0) - 180.0,
        depth_km: depth,
        simulated: true,
    }
}

// ---- worker loop ------------------------------------------------------------

/// Spawns the network domain on its own OS thread + tokio runtime.
/// Returns the join handle; events flow through `tx`.
pub fn spawn_feed(tx: std::sync::mpsc::SyncSender<NetEvent>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("tremor-net".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(feed_loop(tx));
        })
        .expect("spawn net thread")
}

// ---- live maintenance ------------------------------------------------------

/// `get.geojson` request with the dispatcher-mandated key order.
pub fn page_req(page: u32) -> String {
    format!("{{\"label\":\"get.geojson\",\"data\":{{\"page\":{page}}},\"resp\":\"full_json\"}}")
}

pub const MSG_COMPRESS: &str = r#"{"label":"compress","data":{"data":false}}"#;
pub const MSG_ORIPATH: &str = r#"{"label":"oripathv2","data":{"oripath":"/Earthquake_data/index.php","referrer":"https://www.emsc-csem.org/"}}"#;
pub const MSG_KEEPALIVE: &str = r#"{"label":"get.timeUTC","data":{}}"#;

/// Dedup + ship one normalised batch; returns how many crossed the channel.
fn ship(
    sent_ids: &mut HashSet<String>,
    tx: &std::sync::mpsc::SyncSender<NetEvent>,
    mut batch: Vec<Quake>,
) -> usize {
    batch.retain(|q| sent_ids.insert(q.id.clone()));
    let n = batch.len();
    if n > 0 {
        let _ = tx.send(NetEvent::Batch(batch));
    }
    n
}

#[allow(clippy::too_many_arguments)]
fn go_live(
    tx: &std::sync::mpsc::SyncSender<NetEvent>,
    ever_live: &mut bool,
    sim_active: &mut bool,
    last_link: &mut LinkState,
    last_source: &mut &'static str,
    src: &'static str,
) {
    *ever_live = true;
    *sim_active = false;
    if *last_source != src {
        *last_source = src;
        let _ = tx.send(NetEvent::Source(src));
    }
    if *last_link != LinkState::Live {
        *last_link = LinkState::Live;
        let _ = tx.send(NetEvent::Link(LinkState::Live));
    }
}

fn go_off(
    tx: &std::sync::mpsc::SyncSender<NetEvent>,
    sim_active: bool,
    last_link: &mut LinkState,
) {
    if !sim_active && *last_link != LinkState::Off {
        *last_link = LinkState::Off;
        let _ = tx.send(NetEvent::Link(LinkState::Off));
    }
}

/// USGS HTTP feed while the EMSC socket is unavailable.
async fn usgs_fallback(
    client: &reqwest::Client,
    sent_ids: &mut HashSet<String>,
    tx: &std::sync::mpsc::SyncSender<NetEvent>,
    ever_live: &mut bool,
    sim_active: &mut bool,
    last_link: &mut LinkState,
    last_source: &mut &'static str,
) {
    let ok = match client.get(FEED_URL).send().await {
        Ok(resp) if resp.status().is_success() => match resp.text().await {
            Ok(body) => {
                ship(sent_ids, tx, parse_feed(&body));
                true
            }
            Err(_) => false,
        },
        _ => false,
    };
    if ok {
        go_live(tx, ever_live, sim_active, last_link, last_source, SOURCE_USGS);
    } else {
        go_off(tx, *sim_active, last_link);
    }
}

/// Handshake + 48 h back-fill over the paged list; returns the live socket.
async fn emsc_connect(
    sent_ids: &mut HashSet<String>,
    tx: &std::sync::mpsc::SyncSender<NetEvent>,
) -> Result<WsClient, ()> {
    let mut ws =
        WsClient::connect(EMSC_WS_HOST, EMSC_WS_PATH, EMSC_WS_ORIGIN).await.map_err(|_| ())?;
    ws.send_str(MSG_COMPRESS).await.map_err(|_| ())?;
    ws.send_str(MSG_ORIPATH).await.map_err(|_| ())?;

    let cutoff =
        crate::timeutil::now_millis() - (crate::model::EVENT_WINDOW_H * 3_600_000.0) as i64;
    for page in 1..=EMSC_MAX_BACKFILL_PAGES {
        ws.send_str(&page_req(page)).await.map_err(|_| ())?;
        let (batch, oldest) = loop {
            let v = tokio::time::timeout(RECV_TIMEOUT, ws.recv_json())
                .await
                .map_err(|_| ())?
                .map_err(|_| ())?
                .ok_or(())?;
            match v.get("label").and_then(Value::as_str) {
                Some("full_json") => {
                    let (batch, _page, oldest) = parse_emsc_page(&v);
                    break (batch, oldest);
                }
                Some("INSERT") | Some("UPDATE") => {
                    if let Some(e) = v.pointer("/data/data") {
                        if let Some(q) = parse_emsc_event(e) {
                            ship(sent_ids, tx, vec![q]);
                        }
                    }
                }
                Some(_) | None => {}
            }
        };
        let reached = oldest.map_or(true, |o| o < cutoff);
        ship(sent_ids, tx, batch);
        if reached {
            break;
        }
    }
    Ok(ws)
}

/// One pushed/refreshed message from the EMSC socket.
#[allow(clippy::too_many_arguments)]
fn handle_socket_msg(
    v: &Value,
    sent_ids: &mut HashSet<String>,
    tx: &std::sync::mpsc::SyncSender<NetEvent>,
    ever_live: &mut bool,
    sim_active: &mut bool,
    last_link: &mut LinkState,
    last_source: &mut &'static str,
) {
    match v.get("label").and_then(Value::as_str) {
        Some("full_json") => {
            let (batch, _page, _oldest) = parse_emsc_page(v);
            ship(sent_ids, tx, batch);
            go_live(tx, ever_live, sim_active, last_link, last_source, SOURCE_EMSC);
        }
        Some("INSERT") | Some("UPDATE") => {
            if let Some(e) = v.pointer("/data/data") {
                if let Some(q) = parse_emsc_event(e) {
                    ship(sent_ids, tx, vec![q]);
                    go_live(tx, ever_live, sim_active, last_link, last_source, SOURCE_EMSC);
                }
            }
        }
        Some("DELETE") => {} // events age out of the 48 h window anyway
        Some(_) | None => {}
    }
}

/// The whole live pipeline as one sequential loop: drain the EMSC socket in
/// short bounded bursts (push latency <= ~2 s), then service the sim /
/// work / keepalive deadlines. Sequential awaits keep the socket borrow
/// singular - no select! branch may hold it concurrently.
async fn feed_loop(tx: std::sync::mpsc::SyncSender<NetEvent>) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent(concat!("tremor/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("reqwest client");

    let force_sim = std::env::var("TREMOR_SIM").is_ok();
    let mut sim_active = force_sim;
    let mut ever_live = false;
    let mut ws: Option<WsClient> = None;
    let mut sent_ids: HashSet<String> = HashSet::new();
    let mut rng = Rng(0x5E15_0B0C);
    let mut sim_seq = 0u32;
    let mut backoff = Duration::from_secs(2);
    let mut reconnect_at = Instant::now();
    let started = Instant::now();
    let mut last_link = LinkState::Off;
    let mut last_source = "";
    let mut next_work = Instant::now();
    let mut next_keep = Instant::now() + KEEPALIVE_EVERY;
    let mut next_sim = Instant::now() + Duration::from_secs(9);
    let drain_gap = Duration::from_secs(2);

    if force_sim {
        last_link = LinkState::Sim;
        let _ = tx.send(NetEvent::Link(LinkState::Sim));
    } else {
        // first work tick right away: connect + back-fill on startup
        next_work = Instant::now();
    }

    loop {
        // ---- 1. bounded socket drain ------------------------------------
        if let Some(sock) = ws.as_mut() {
            let mut dead = false;
            loop {
                match tokio::time::timeout(drain_gap, sock.recv_json()).await {
                    Ok(Ok(Some(v))) => handle_socket_msg(
                        &v,
                        &mut sent_ids,
                        &tx,
                        &mut ever_live,
                        &mut sim_active,
                        &mut last_link,
                        &mut last_source,
                    ),
                    // close frame or io error: socket is gone
                    Ok(Ok(None)) | Ok(Err(_)) => {
                        dead = true;
                        break;
                    }
                    // quiet window: stop draining, service the clocks
                    Err(_) => break,
                }
            }
            if dead {
                ws = None;
                reconnect_at = Instant::now() + backoff;
                backoff = (backoff * 2).min(Duration::from_secs(30));
                next_work = Instant::now(); // try USGS / reconnect soon
                go_off(&tx, sim_active, &mut last_link);
            }
        } else if !force_sim {
            // no socket: idle until the next deadline instead of spinning
            let now = Instant::now();
            let mut next = next_work.min(next_sim);
            if sim_active {
                next = next.min(next_sim);
            }
            let wait = next.saturating_duration_since(now).min(Duration::from_millis(400));
            tokio::time::sleep(wait).await;
        } else {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        let now = Instant::now();
        // ---- 2. offline sim feed ----------------------------------------
        if sim_active && now >= next_sim {
            next_sim = now + Duration::from_secs(9);
            let n = 1 + (rng.next_f32() < 0.35) as usize;
            let mut batch = Vec::with_capacity(n);
            for _ in 0..n {
                sim_seq += 1;
                batch.push(sim_event(&mut rng, sim_seq));
            }
            let _ = tx.send(NetEvent::Batch(batch));
        }
        // ---- 3. work tick: page refresh / reconnect / USGS fallback -----
        if !force_sim && now >= next_work {
            next_work = now + POLL_EVERY;
            if ws.is_some() {
                if let Some(sock) = ws.as_mut() {
                    if sock.send_str(&page_req(1)).await.is_err() {
                        ws = None;
                        reconnect_at = now + backoff;
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                        go_off(&tx, sim_active, &mut last_link);
                    }
                }
            } else if now >= reconnect_at {
                match emsc_connect(&mut sent_ids, &tx).await {
                    Ok(sock) => {
                        ws = Some(sock);
                        backoff = Duration::from_secs(2);
                        next_keep = Instant::now() + KEEPALIVE_EVERY;
                        go_live(
                            &tx,
                            &mut ever_live,
                            &mut sim_active,
                            &mut last_link,
                            &mut last_source,
                            SOURCE_EMSC,
                        );
                    }
                    Err(_) => {
                        reconnect_at = now + backoff;
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                        usgs_fallback(
                            &client,
                            &mut sent_ids,
                            &tx,
                            &mut ever_live,
                            &mut sim_active,
                            &mut last_link,
                            &mut last_source,
                        )
                        .await;
                    }
                }
            } else {
                usgs_fallback(
                    &client,
                    &mut sent_ids,
                    &tx,
                    &mut ever_live,
                    &mut sim_active,
                    &mut last_link,
                    &mut last_source,
                )
                .await;
            }
        }
        // ---- 4. socket keepalive ----------------------------------------
        if ws.is_some() && now >= next_keep {
            next_keep = now + KEEPALIVE_EVERY;
            if let Some(sock) = ws.as_mut() {
                if sock.send_str(MSG_KEEPALIVE).await.is_err() {
                    ws = None;
                    reconnect_at = now + backoff;
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                    go_off(&tx, sim_active, &mut last_link);
                }
            }
        }
        // ---- 5. offline guardrail ---------------------------------------
        if !ever_live && started.elapsed() > SIM_FALLBACK_AFTER && !sim_active {
            sim_active = true;
            next_sim = Instant::now();
            if last_link != LinkState::Sim {
                last_link = LinkState::Sim;
                let _ = tx.send(NetEvent::Link(LinkState::Sim));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "type": "FeatureCollection",
      "features": [
        {"type":"Feature","id":"us7000a","properties":{"mag":6.2,"place":"77 km SE of Kushiro, Japan","time":1760000000000},
         "geometry":{"type":"Point","coordinates":[144.9,42.1,35.0]}},
        {"type":"Feature","id":"us7000b","properties":{"mag":2.1,"place":"tiny","time":1760000001000},
         "geometry":{"type":"Point","coordinates":[10.0,10.0,5.0]}},
        {"type":"Feature","id":"us7000c","properties":{"mag":null,"place":null,"time":null},
         "geometry":{"type":"Point","coordinates":[10.0,10.0,5.0]}},
        {"type":"Feature","id":"us7000d","properties":{"mag":4.0,"time":1760000002000},
         "geometry":{"type":"Point","coordinates":[-70.5,-33.1]}}
      ]
    }"#;

    #[test]
    fn parses_and_filters() {
        let q = parse_feed(SAMPLE);
        assert_eq!(q.len(), 2, "mag 2.1 and null-mag must be dropped");
        assert_eq!(q[0].id, "us7000a");
        assert!((q[0].mag - 6.2).abs() < 1e-4);
        assert!((q[0].depth_km - 35.0).abs() < 1e-4);
        assert_eq!(q[0].place, "77 KM SE OF KUSHIRO, JAPAN");
        assert_eq!(q[1].id, "us7000d");
        assert!((q[1].depth_km - 0.0).abs() < 1e-6, "2-coord geometry ok");
    }

    #[test]
    fn garbage_payload_is_not_fatal() {
        assert!(parse_feed("not json at all").is_empty());
        assert!(parse_feed("{\"features\": null}").is_empty());
    }

    const EMSC_PAGE: &str = r#"{"label":"full_json","data":{"tbiconf":{"view":1,"maxview":125},
      "features":[
        {"evid":2075780,"fid":"IPDMOCCUC","time":"2026-10-16T16:43:00.900000","timest":1792168980900,
         "lat":37.307,"lon":37.089,"depth":4.1,"reg":"CENTRAL TURKEY","am":"M","magtype":"ML","mag":4.2,
         "pubname":"KOERI","published":true,"preferred":true},
        {"evid":2075781,"fid":"X","timest":1792168981900,"lat":10.0,"lon":10.0,"depth":10.0,
         "reg":"TEST REGION","mag":1.6},
        {"evid":2075782,"fid":"Y","timest":1792168982900,"lat":999.0,"lon":0.0,"depth":1.0,
         "reg":"BAD LAT","mag":5.0}
      ]}}"#;

    #[test]
    fn emsc_page_parses_and_filters() {
        let v: Value = serde_json::from_str(EMSC_PAGE).unwrap();
        let (q, page, oldest) = parse_emsc_page(&v);
        assert_eq!(page, Some(1));
        assert_eq!(q.len(), 1, "sub-4.0 and bad-lat entries dropped");
        assert_eq!(q[0].id, "emsc-2075780");
        assert!((q[0].mag - 4.2).abs() < 1e-4);
        assert_eq!(q[0].place, "CENTRAL TURKEY");
        assert!((q[0].depth_km - 4.1).abs() < 1e-4);
        assert_eq!(oldest, Some(1792168980900));
    }

    #[test]
    fn emsc_insert_push_parses() {
        let v: Value = serde_json::from_str(
            r#"{"label":"INSERT","data":{"at":50,"data":{"evid":99,"timest":1792168999000,
               "lat":-33.4,"lon":-71.6,"depth":40.0,"reg":"VALPARAISO, CHILE","mag":5.5}}}"#,
        )
        .unwrap();
        let e = v.pointer("/data/data").unwrap();
        let q = parse_emsc_event(e).unwrap();
        assert_eq!(q.id, "emsc-99");
        assert!((q.mag - 5.5).abs() < 1e-4);
        assert_eq!(q.place, "VALPARAISO, CHILE");
    }

    #[test]
    fn sim_events_are_valid() {
        let mut r = Rng(1234);
        for i in 0..200 {
            let q = sim_event(&mut r, i);
            assert!(q.mag >= MAG_FILTER && q.mag <= 7.8);
            assert!(q.lat.abs() <= 90.0 && q.lon.abs() <= 180.0);
            assert!(q.place.starts_with("SIM "));
        }
    }
}
