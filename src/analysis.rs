//! Station-level analysis helpers: event-driven synthetic raw traces
//! (seismogram style), Gutenberg-Richter cumulative counts and coastline
//! dots for the lat/lon mini map.

use std::sync::OnceLock;

use crate::model::Quake;
use crate::sensors::SENSORS;

/// Great-circle distance in degrees.
pub fn haversine_deg(lat1: f32, lon1: f32, lat2: f32, lon2: f32) -> f32 {
    let (la1, lo1) = (lat1.to_radians(), lon1.to_radians());
    let (la2, lo2) = (lat2.to_radians(), lon2.to_radians());
    let dla = la2 - la1;
    let dlo = lo2 - lo1;
    let a = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    (2.0 * a.sqrt().asin()).to_degrees()
}

/// Live raw-data style trace for a station: background microseism plus
/// P (6.0 km/s) and S (3.5 km/s) arrivals from every logged event, amplitude
/// scaled by magnitude and epicentral distance. Window ends at `t_end`.
pub fn station_trace(
    station_idx: usize,
    events: &[Quake],
    t_end: f64,
    secs: f64,
    n: usize,
    gain: f32,
) -> Vec<f32> {
    let st = &SENSORS[station_idx % SENSORS.len()];
    let mut out = vec![0.0f32; n];
    let mut phase = 0u64.wrapping_sub(station_idx as u64 * 7919);
    for (i, o) in out.iter_mut().enumerate() {
        let t = t_end - secs + secs * i as f64 / n as f64;
        // microseism hum + deterministic jitter
        phase = phase.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let jit = (((phase >> 40) as f64) / 8_388_608.0 - 0.5) * 0.22;
        *o = ((t * 1.9).sin() * 0.16 + (t * 0.61 + 1.7).sin() * 0.10 + jit) as f32;
    }
    for q in events {
        let ddeg = haversine_deg(st.lat, st.lon, q.lat, q.lon);
        let km = ddeg as f64 * 111.32;
        let tp = q.time_s() + km / 6.0;
        let ts = q.time_s() + km / 3.5;
        let amp = 10f32.powf((q.mag - 4.0) * 0.7) / (1.0 + ddeg * 0.05).powi(2);
        if amp < 0.02 {
            continue;
        }
        for (i, o) in out.iter_mut().enumerate() {
            let t = t_end - secs + secs * i as f64 / n as f64;
            if t > tp {
                let dt = (t - tp) as f32;
                *o += (-dt / 16.0).exp() * (dt * 2.6).sin() * amp;
            }
            if t > ts {
                let dt = (t - ts) as f32;
                *o += (-dt / 34.0).exp() * (dt * 1.4).sin() * amp * 1.8;
            }
        }
    }
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(0.6);
    for o in out.iter_mut() {
        *o = (*o / peak).clamp(-1.0, 1.0) * gain;
    }
    out
}

/// Cumulative event counts per magnitude step (Gutenberg-Richter style).
pub fn gr_cumulative(events: &[Quake]) -> [(f32, u32); 9] {
    let mut out = [(0.0f32, 0u32); 9];
    for (i, o) in out.iter_mut().enumerate() {
        let m = 4.0 + i as f32 * 0.5;
        o.0 = m;
        o.1 = events.iter().filter(|q| q.mag >= m).count() as u32;
    }
    out
}

/// Depth-zone counts: crustal / upper / lower mantle bands.
pub fn depth_zones(events: &[Quake]) -> [u32; 3] {
    let mut z = [0u32; 3];
    for q in events {
        z[if q.depth_km < 70.0 {
            0
        } else if q.depth_km < 300.0 {
            1
        } else {
            2
        }] += 1;
    }
    z
}

/// Coastline dots (mask-space coords) for the mini map, computed once.
pub fn coast_dots() -> &'static Vec<(u32, u32)> {
    static DOTS: OnceLock<Vec<(u32, u32)>> = OnceLock::new();
    DOTS.get_or_init(|| {
        use crate::assets::{land_at, MASK_H, MASK_W};
        let mut v = Vec::new();
        let step = 4;
        let mut y = 0;
        while y < MASK_H {
            let mut x = 0;
            while x < MASK_W {
                let lon = (x as f32 / MASK_W as f32) * 360.0 - 180.0;
                let lat = 90.0 - (y as f32 / MASK_H as f32) * 180.0;
                if land_at(lon, lat) {
                    let mut edge = false;
                    for (dx, dy) in [(4i32, 0), (-4, 0), (0, 4), (0, -4)] {
                        let nx = (x as i32 + dx).rem_euclid(MASK_W as i32) as f32;
                        let ny = (y as i32 + dy).clamp(0, MASK_H as i32 - 1) as f32;
                        let lo = (nx / MASK_W as f32) * 360.0 - 180.0;
                        let la = 90.0 - (ny / MASK_H as f32) * 180.0;
                        if !land_at(lo, la) {
                            edge = true;
                            break;
                        }
                    }
                    if edge {
                        v.push((x, y));
                    }
                }
                x += step;
            }
            y += step;
        }
        v
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(mag: f32, lat: f32, lon: f32, t: i64) -> Quake {
        Quake {
            id: format!("t{t}"),
            mag,
            place: "T".into(),
            time_ms: t,
            lat,
            lon,
            depth_km: 30.0,
            simulated: true,
        }
    }

    #[test]
    fn haversine_sane() {
        assert!(haversine_deg(0.0, 0.0, 0.0, 90.0) - 90.0 < 1e-3);
        assert!(haversine_deg(10.0, 10.0, 10.0, 10.0) < 1e-6);
    }

    #[test]
    fn trace_has_arrivals() {
        // event 30 deg away: P arrives after ~9.2 min
        let ev = vec![q(6.5, 0.0, 30.0, 0)];
        let tr = station_trace(0, &ev, 1500.0, 300.0, 300, 1.0);
        assert_eq!(tr.len(), 300);
        assert!(tr.iter().all(|v| v.abs() <= 1.0));
        // energy present near the end (arrivals inside the window)
        let tail: f32 = tr[200..].iter().map(|v| v.abs()).sum();
        assert!(tail > 1.0, "trace should ring after arrivals");
    }

    #[test]
    fn gr_and_zones() {
        let ev = vec![q(4.2, 0.0, 0.0, 1), q(6.4, 1.0, 1.0, 2), q(7.5, 2.0, 2.0, 3)];
        let gr = gr_cumulative(&ev);
        assert_eq!(gr[0].1, 3);
        assert_eq!(gr[4].1, 2); // >=6.0
        assert_eq!(gr[7].1, 1); // >=7.5
        let ev2 = vec![q(4.0, 0.0, 0.0, 1), q(4.0, 0.0, 0.0, 2), q(4.0, 0.0, 0.0, 3)];
        let _ = depth_zones(&ev2);
    }

    #[test]
    fn coast_dots_present() {
        let d = coast_dots();
        assert!(d.len() > 1000, "{}", d.len());
    }
}
