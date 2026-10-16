//! System processing domain: consumes filtered network events, converts raw
//! geographic metrics (lat/lon/depth) into 3D cartesian vectors, owns the
//! structural memory (unique-id collection, ring buffer, wave lifecycle).

use std::collections::{HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::MAG_FILTER;

/// One seismic event, normalised from the USGS GeoJSON payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quake {
    pub id: String,
    pub mag: f32,
    pub place: String,
    /// origin time, unix millis
    pub time_ms: i64,
    pub lat: f32,
    pub lon: f32,
    pub depth_km: f32,
    /// true when produced by the offline simulation fallback
    pub simulated: bool,
}

impl Quake {
    pub fn time_s(&self) -> f64 {
        self.time_ms as f64 / 1000.0
    }
    /// Unit-sphere cartesian position of the epicentre.
    pub fn unit_pos(&self) -> [f32; 3] {
        geo_to_cartesian(self.lat, self.lon, 1.0)
    }
}

/// Geographic degrees -> right-handed cartesian on a sphere of radius `r`.
/// lon = 0 / lat = 0 maps to +Z (camera-facing), lat = 90 to +Y.
pub fn geo_to_cartesian(lat_deg: f32, lon_deg: f32, r: f32) -> [f32; 3] {
    let lat = lat_deg.to_radians();
    let lon = lon_deg.to_radians();
    let cl = lat.cos();
    [r * cl * lon.sin(), r * lat.sin(), r * cl * lon.cos()]
}

/// Inverse of [`geo_to_cartesian`], used by the shader-facing uv math and tests.
pub fn cartesian_to_geo(p: [f32; 3]) -> (f32, f32) {
    let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-9);
    let lat = (p[1] / len).asin().to_degrees();
    let lon = p[0].atan2(p[2]).to_degrees();
    (lat, lon)
}

/// Expanding wave-front attached to a fresh event. The GPU evaluates the
/// decay equation; the CPU only tracks the lifecycle for cleanup.
#[derive(Debug, Clone)]
pub struct Wave {
    pub id: String,
    pub center: [f32; 3],
    pub start: f64,
    pub mag: f32,
}

/// seconds a wave stays alive: bigger quakes ring longer
pub fn wave_ttl(mag: f32) -> f64 {
    14.0 + (mag - MAG_FILTER).clamp(0.0, 4.0) as f64 * 6.0
}

/// The globe and the store show a rolling 48 h window of seismicity (the span
/// of the EMSC-CSEM realtime list the feed walks on connect).
pub const EVENT_WINDOW_H: f64 = 48.0;

/// events arriving older than this (back-fill pages) do not spawn an
/// expanding wave front - only genuinely fresh solutions ring the globe.
pub const WAVE_FRESH_S: f64 = 120.0;

#[derive(Debug, Default)]
pub struct Stats {
    pub count_1h: usize,
    pub count_24h: usize,
    pub max_mag: f32,
    pub avg_depth_km: f32,
    /// counts per magnitude bin: [4,5) [5,6) [6,7) [7,8) [8+)
    pub mag_bins: [u32; 5],
}

/// Structural memory of the processing domain.
#[derive(Debug, Default)]
pub struct EventStore {
    events: VecDeque<Quake>,
    seen: HashSet<String>,
    waves: Vec<Wave>,
    pub ingested: u64,
    pub dropped_dupes: u64,
}

impl EventStore {
    pub fn new() -> Self {
        Self {
            events: VecDeque::new(),
            seen: HashSet::new(),
            waves: Vec::new(),
            ingested: 0,
            dropped_dupes: 0,
        }
    }

    /// Guardrail 2: an event is queued for the GPU at most once, keyed by id.
    /// The deque stays sorted newest-origin-first (feeds deliver revisions
    /// and late solutions out of order), so `latest()` is the front element.
    /// Returns true when the event was new (and, if fresh, a wave spawned).
    pub fn ingest(&mut self, q: Quake, now: f64) -> bool {
        if q.mag < MAG_FILTER {
            return false; // guardrail 1, second line of defence
        }
        if !self.seen.insert(q.id.clone()) {
            self.dropped_dupes += 1;
            return false;
        }
        self.ingested += 1;
        if now - q.time_s() < WAVE_FRESH_S {
            self.waves.push(Wave {
                id: q.id.clone(),
                center: q.unit_pos(),
                start: now,
                mag: q.mag,
            });
        }
        let pos = self
            .events
            .iter()
            .position(|e| e.time_ms < q.time_ms)
            .unwrap_or(self.events.len());
        self.events.insert(pos, q);
        if self.events.len() > 512 {
            self.events.pop_back();
        }
        true
    }

    /// Rolling 48 h memory: drop events older than the display window.
    /// The deque is time-sorted, so the oldest sit at the back.
    pub fn prune_events(&mut self, now: f64) {
        let cutoff_ms = now as i64 * 1000 - (EVENT_WINDOW_H * 3_600_000.0) as i64;
        while self.events.back().map_or(false, |q| q.time_ms < cutoff_ms) {
            self.events.pop_back();
        }
    }

    /// Automatic visual fading / memory cleanup: dissipated waves are removed.
    pub fn prune_waves(&mut self, now: f64) {
        self.waves.retain(|w| now - w.start < wave_ttl(w.mag));
    }

    pub fn events(&self) -> &VecDeque<Quake> {
        &self.events
    }
    pub fn waves(&self) -> &[Wave] {
        &self.waves
    }
    pub fn latest(&self) -> Option<&Quake> {
        self.events.front()
    }
    pub fn get(&self, i: usize) -> Option<&Quake> {
        self.events.get(i)
    }

    pub fn stats(&self, now: f64) -> Stats {
        let mut s = Stats::default();
        let now_ms = (now * 1000.0) as i64;
        let mut depth_sum = 0.0f32;
        for q in &self.events {
            let age_h = (now_ms - q.time_ms) as f64 / 3_600_000.0;
            if age_h < 0.0 {
                continue; // event lies in the future of the reference time
            }
            if age_h <= 1.0 {
                s.count_1h += 1;
            }
            if age_h <= 24.0 {
                s.count_24h += 1;
            }
            s.max_mag = s.max_mag.max(q.mag);
            depth_sum += q.depth_km;
            s.mag_bins[mag_bin(q.mag)] += 1;
        }
        if !self.events.is_empty() {
            s.avg_depth_km = depth_sum / self.events.len() as f32;
        }
        s
    }
}

pub fn mag_bin(m: f32) -> usize {
    match m {
        m if m < 5.0 => 0,
        m if m < 6.0 => 1,
        m if m < 7.0 => 2,
        m if m < 8.0 => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(id: &str, mag: f32) -> Quake {
        Quake {
            id: id.into(),
            mag,
            place: format!("TEST {id}"),
            time_ms: 0,
            lat: 0.0,
            lon: 0.0,
            depth_km: 10.0,
            simulated: false,
        }
    }

    #[test]
    fn geo_mapping_axes() {
        let p = geo_to_cartesian(0.0, 0.0, 1.0);
        assert!((p[2] - 1.0).abs() < 1e-6 && p[0].abs() < 1e-6 && p[1].abs() < 1e-6);
        let p = geo_to_cartesian(90.0, 0.0, 1.0);
        assert!((p[1] - 1.0).abs() < 1e-6);
        let p = geo_to_cartesian(0.0, 90.0, 1.0);
        assert!((p[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn geo_roundtrip() {
        for (lat, lon) in [(35.2, 139.7), (-33.4, -71.6), (64.9, -17.4), (0.0, 180.0)] {
            let p = geo_to_cartesian(lat, lon, 1.0);
            let (la, lo) = cartesian_to_geo(p);
            assert!((la - lat).abs() < 1e-3, "{la} vs {lat}");
            let dlon = ((lo - lon + 540.0) % 360.0) - 180.0;
            assert!(dlon.abs() < 1e-3, "{lo} vs {lon}");
        }
    }

    #[test]
    fn filter_and_dedup() {
        let mut st = EventStore::new();
        assert!(!st.ingest(q("a", 3.9), 0.0), "sub-4.0 must be dropped");
        assert!(st.ingest(q("b", 4.5), 0.0));
        assert!(!st.ingest(q("b", 4.5), 1.0), "duplicate id must be dropped");
        assert_eq!(st.events().len(), 1);
        assert_eq!(st.dropped_dupes, 1);
        assert_eq!(st.waves().len(), 1);
    }

    #[test]
    fn store_stays_time_sorted_and_latest_is_front() {
        let mut st = EventStore::new();
        let mut q1 = q("late", 4.5);
        q1.time_ms = 1000;
        let mut q2 = q("newer", 5.0);
        q2.time_ms = 3000;
        let mut q3 = q("oldest", 4.2);
        q3.time_ms = 500;
        assert!(st.ingest(q1, 4.0));
        assert!(st.ingest(q2, 4.0));
        assert!(st.ingest(q3, 4.0), "late solutions must be accepted");
        let t: Vec<i64> = st.events().iter().map(|e| e.time_ms).collect();
        assert_eq!(t, vec![3000, 1000, 500]);
        assert_eq!(st.latest().unwrap().id, "newer");
    }

    #[test]
    fn prune_keeps_48h_window() {
        let mut st = EventStore::new();
        let mut old = q("old", 4.5);
        old.time_ms = 0;
        let mut fresh = q("fresh", 4.5);
        fresh.time_ms = 47 * 3_600_000;
        st.ingest(old, 48.5 * 3600.0);
        st.ingest(fresh, 48.5 * 3600.0);
        st.prune_events(48.5 * 3600.0);
        assert_eq!(st.events().len(), 1);
        assert_eq!(st.events()[0].id, "fresh");
    }

    #[test]
    fn backfilled_events_do_not_ring() {
        let mut st = EventStore::new();
        let mut old = q("backfill", 6.0);
        old.time_ms = 10 * 3_600_000; // 10 h old page entry
        assert!(st.ingest(old, 10.0 * 3600.0 + 600.0));
        assert!(st.waves().is_empty(), "no wave fronts for back-fill");
    }

    #[test]
    fn wave_cleanup() {
        let mut st = EventStore::new();
        st.ingest(q("a", 4.0), 0.0); // ttl 14s
        st.ingest(q("b", 7.5), 0.0); // ttl 35s
        st.prune_waves(15.0);
        assert_eq!(st.waves().len(), 1);
        assert_eq!(st.waves()[0].id, "b");
        st.prune_waves(40.0);
        assert!(st.waves().is_empty());
    }

    #[test]
    fn stats_bins() {
        let mut st = EventStore::new();
        for (i, m) in [4.2, 4.8, 5.5, 6.5, 7.2, 8.4].iter().enumerate() {
            st.ingest(q(&format!("e{i}"), *m), 0.0);
        }
        let s = st.stats(0.5);
        assert_eq!(s.count_1h, 6);
        assert_eq!(s.mag_bins, [2, 1, 1, 1, 1]);
        assert!((s.max_mag - 8.4).abs() < 1e-4);
    }
}
