//! Seismic sensor network overlay: a curated set of global broadband
//! stations (GSN-style: IU/II/GT class sites) rendered as crosshair markers
//! on the globe. Coordinates are station reference locations.

#[derive(Debug, Clone, Copy)]
pub struct Sensor {
    pub code: &'static str,
    pub place: &'static str,
    pub lat: f32,
    pub lon: f32,
}

pub const SENSORS: &[Sensor] = &[
    Sensor { code: "ANMO", place: "ALBUQUERQUE, NM, USA", lat: 34.95, lon: -106.46 },
    Sensor { code: "COLA", place: "FAIRBANKS, AK, USA", lat: 64.87, lon: -147.86 },
    Sensor { code: "HRV", place: "HARVARD, MA, USA", lat: 42.51, lon: -71.56 },
    Sensor { code: "TUCO", place: "TUCSON, AZ, USA", lat: 32.31, lon: -110.78 },
    Sensor { code: "NEWO", place: "NEWPORT, WA, USA", lat: 48.26, lon: -117.12 },
    Sensor { code: "PFO", place: "PINON FLAT, CA, USA", lat: 33.61, lon: -116.46 },
    Sensor { code: "KIP", place: "KIPAPA, HAWAII, USA", lat: 21.42, lon: -158.01 },
    Sensor { code: "BFO", place: "BLACK FOREST, GERMANY", lat: 48.33, lon: 8.33 },
    Sensor { code: "ESK", place: "ESKDALEMUIR, SCOTLAND", lat: 55.32, lon: -3.21 },
    Sensor { code: "GRF", place: "GRAFENBERG, GERMANY", lat: 49.69, lon: 11.22 },
    Sensor { code: "BORG", place: "BORGFJORDUR, ICELAND", lat: 64.75, lon: -21.32 },
    Sensor { code: "KMBO", place: "KILIMA MBOGO, KENYA", lat: -1.13, lon: 37.11 },
    Sensor { code: "TSUM", place: "TSUMEB, NAMIBIA", lat: -19.20, lon: 17.59 },
    Sensor { code: "SUR", place: "SUTHERLAND, SOUTH AFRICA", lat: -32.38, lon: 20.81 },
    Sensor { code: "ABPO", place: "ANTANANARIVO, MADAGASCAR", lat: -18.91, lon: 47.52 },
    Sensor { code: "MAJO", place: "MATSUSHIRO, JAPAN", lat: 36.54, lon: 138.20 },
    Sensor { code: "TATO", place: "TAIPEI, TAIWAN", lat: 24.97, lon: 121.50 },
    Sensor { code: "GUMO", place: "GUAM, MARIANA IS.", lat: 13.59, lon: 144.87 },
    Sensor { code: "PMG", place: "PORT MORESBY, PNG", lat: -9.41, lon: 147.15 },
    Sensor { code: "HNR", place: "HONIARA, SOLOMON IS.", lat: -9.43, lon: 160.00 },
    Sensor { code: "WRAB", place: "TENNANT CREEK, AUSTRALIA", lat: -21.08, lon: 134.75 },
    Sensor { code: "CTAO", place: "CHARTERS TOWERS, AUSTRALIA", lat: -20.09, lon: 146.25 },
    Sensor { code: "NWAO", place: "NARROGIN, AUSTRALIA", lat: -32.93, lon: 117.20 },
    Sensor { code: "SNZO", place: "WELLINGTON, NEW ZEALAND", lat: -41.31, lon: 174.70 },
    Sensor { code: "COCO", place: "COCOS ISLANDS", lat: -12.19, lon: 96.76 },
    Sensor { code: "HYB", place: "HYDERABAD, INDIA", lat: 17.41, lon: 78.55 },
    Sensor { code: "RAYN", place: "RAYN ROCK, SAUDI ARABIA", lat: 22.53, lon: 45.03 },
    Sensor { code: "POHA", place: "POHNPEI, MICRONESIA", lat: 6.95, lon: 158.21 },
    Sensor { code: "AFI", place: "AFIAMALU, SAMOA", lat: -13.90, lon: -171.78 },
    Sensor { code: "RPN", place: "EASTER ISLAND", lat: -27.13, lon: -109.33 },
    Sensor { code: "EFI", place: "EAST FALKLAND IS.", lat: -51.67, lon: -59.05 },
    Sensor { code: "PMSA", place: "PALMER STN, ANTARCTICA", lat: -64.77, lon: -64.05 },
    Sensor { code: "CASY", place: "CASEY STN, ANTARCTICA", lat: -66.28, lon: 110.53 },
    Sensor { code: "MAW", place: "MAWSON STN, ANTARCTICA", lat: -67.60, lon: 62.87 },
    Sensor { code: "SBA", place: "SCOTT BASE, ANTARCTICA", lat: -77.85, lon: 166.67 },
    Sensor { code: "VALV", place: "VALPARAISO, CHILE", lat: -33.05, lon: -71.62 },
    Sensor { code: "LPAZ", place: "LA PAZ, BOLIVIA", lat: -16.29, lon: -68.18 },
    Sensor { code: "BOGB", place: "BOGOTA, COLOMBIA", lat: 4.53, lon: -74.08 },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn stations_valid() {
        assert!(SENSORS.len() >= 30);
        let mut codes = HashSet::new();
        for s in SENSORS {
            assert!(s.lat.abs() <= 90.0 && s.lon.abs() <= 180.0, "{}", s.code);
            assert!(codes.insert(s.code), "duplicate {}", s.code);
            assert!((3..=4).contains(&s.code.len()));
        }
    }
}
