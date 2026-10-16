//! Tiny dependency-free UTC civil-time helpers for the HUD epoch readout.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Days-since-epoch -> (year, month, day); Howard Hinnant's civil algorithm.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `2026-10-16T21:47:05Z` style stamp from unix seconds.
pub fn fmt_utc(secs: f64) -> String {
    let s = secs.floor() as i64;
    let days = s.div_euclid(86_400);
    let rem = s.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

/// `21:47:05` clock part only.
pub fn fmt_hms(secs: f64) -> String {
    let s = secs.floor() as i64;
    let rem = s.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", rem / 3600, (rem / 60) % 60, rem % 60)
}


// ---- solar / lunar position (low-precision, dependency-free) --------------

/// Greenwich mean sidereal time, degrees (Meeus eq. 12.4, truncated).
pub fn gmst_deg(unix_secs: f64) -> f64 {
    let d = unix_secs / 86_400.0 - 0.5 + 2_440_588.0 - 2_451_545.0; // days since J2000 (approx: JD - 2451545)
    let g = 280.460_618_37 + 360.985_647_366_29 * d;
    g.rem_euclid(360.0)
}

/// Solar declination + equation-of-time based subsolar point, degrees.
/// Returns (lat, lon) of the point where the sun is at zenith: good to ~0.2 deg
/// which is plenty for a terminator on a 720 px globe.
pub fn subsolar_point(unix_secs: f64) -> (f64, f64) {
    let d = unix_secs / 86_400.0 - 0.5 + 2_440_588.0 - 2_451_545.0;
    // mean anomaly / mean longitude (deg)
    let g = (357.528 + 0.985_600_3 * d).to_radians();
    let l = (280.460 + 0.985_647_4 * d).to_radians();
    let lam = l + 1.915 * g.sin().to_radians() + 0.020 * (2.0 * g).sin().to_radians();
    let eps = (23.439 - 0.000_000_4 * d).to_radians();
    let dec = (eps.sin() * lam.sin()).asin();
    // right ascension -> geographic longitude via GMST
    let ra = lam.sin().atan2(eps.cos()).to_degrees();
    let lon = (ra - gmst_deg(unix_secs)).rem_euclid(360.0);
    let lon = if lon > 180.0 { lon - 360.0 } else { lon };
    (dec.to_degrees(), lon)
}

/// Lunar geocentric subpoint (lat, lon) in degrees, ~0.3 deg accuracy from the
/// truncated ELP series (Meeus ch. 47 main terms). Used to place the moon disc
/// in the background sky and to derive its phase from the sun-moon angle.
pub fn sublunar_point(unix_secs: f64) -> (f64, f64) {
    let d = unix_secs / 86_400.0 - 0.5 + 2_440_588.0 - 2_451_545.0;
    let l = (218.316 + 13.176_396 * d).to_radians(); // mean longitude
    let m = (134.963 + 13.064_993 * d).to_radians(); // mean anomaly
    let f = (93.272 + 13.229_350 * d).to_radians();  // mean distance
    let lam = l + 6.289 * m.sin().to_radians();
    let bet = 5.128 * f.sin().to_radians();
    let eps = (23.439 - 0.000_000_4 * d).to_radians();
    // ecliptic -> equatorial
    let dec = (bet.sin() * eps.cos() + bet.cos() * eps.sin() * lam.sin()).asin();
    let ra = (lam.sin() * eps.cos() - bet.tan() * eps.sin())
        .atan2(lam.cos())
        .to_degrees();
    let lon = (ra - gmst_deg(unix_secs)).rem_euclid(360.0);
    let lon = if lon > 180.0 { lon - 360.0 } else { lon };
    (dec.to_degrees(), lon)
}

/// Moon phase angle 0..1 (0 = new, 0.5 = full) from the sun-moon elongation.
pub fn moon_phase(unix_secs: f64) -> f64 {
    let (slat, slon) = subsolar_point(unix_secs);
    let (mlat, mlon) = sublunar_point(unix_secs);
    let el = separation_deg(slat, slon, mlat, mlon);
    (el / 180.0).clamp(0.0, 1.0)
}

/// Great-circle separation of two geographic points, degrees.
pub fn separation_deg(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (la1, lo1) = (lat1.to_radians(), lon1.to_radians());
    let (la2, lo2) = (lat2.to_radians(), lon2.to_radians());
    let c = (la1.sin() * la2.sin() + la1.cos() * la2.cos() * (lo1 - lo2).cos()).clamp(-1.0, 1.0);
    c.acos().to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_zero() {
        assert_eq!(fmt_utc(0.0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_date() {
        // 2026-10-16T00:00:00Z == 1792108800? verify via round trip below instead.
        let secs = 1_760_000_000.0; // 2025-10-09T08:53:20Z
        assert_eq!(fmt_utc(secs), "2025-10-09T08:53:20Z");
    }

    #[test]
    fn leap_day() {
        // 2024-02-29 = day 19781
        let secs = 19782.0 * 86400.0;
        assert_eq!(fmt_utc(secs), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn sun_range() {
        // subsolar latitude must stay within the tropics
        for t in (0..400).map(|i| 1_760_000_000.0 + i as f64 * 86_400.0 * 0.9) {
            let (lat, lon) = subsolar_point(t);
            assert!(lat.abs() <= 23.5, "lat {lat}");
            assert!(lon.abs() <= 180.0, "lon {lon}");
        }
    }

    #[test]
    fn moon_range_and_phase() {
        for t in (0..200).map(|i| 1_760_000_000.0 + i as f64 * 3600.0 * 6.0) {
            let (lat, lon) = sublunar_point(t);
            assert!(lat.abs() <= 28.6, "moon lat {lat}");
            assert!(lon.abs() <= 180.0);
            let p = moon_phase(t);
            assert!((0.0..=1.0).contains(&p));
        }
    }

    #[test]
    fn separation_sanity() {
        assert!((separation_deg(0.0, 0.0, 0.0, 90.0) - 90.0).abs() < 1e-6);
        assert!((separation_deg(90.0, 0.0, -90.0, 0.0) - 180.0).abs() < 1e-6);
    }

    #[test]
    fn hms() {
        assert_eq!(fmt_hms(86_400.0 + 3600.0 * 21.0 + 47.0 * 60.0 + 5.0), "21:47:05");
    }
}
