//! Embedded binary assets: multi-size anti-aliased Source Code Pro bitmap
//! faces (SIL OFL, rasterised at the orrery UI sizes 10/13/16 px - the old
//! VT323 pixel face fell apart at those sizes), a 3600x1800 1bpp
//! equirectangular land mask (Natural Earth 50m, public domain) and PB2002
//! plate-boundary segments. All baked in at compile time - the shipped
//! executable needs no sidecar files.
//! Regenerate with `tools/gen_fonts.py` and `tools/gen_assets.py`.

pub const FONT_GLYPHS: u32 = 103; // ASCII 32..127 + 7 extra symbols
pub const FONT_ATLAS_COLS: u32 = 16;

/// One bitmap face: `cw`x`ch` cells of 8-bit coverage (one byte per pixel),
/// `yoff` = row offset inside the packed GPU atlas (see `build_font_atlas`).
#[derive(Debug, Clone, Copy)]
pub struct Face {
    pub cw: u32,
    pub ch: u32,
    pub yoff: u32,
    pub bits: &'static [u8],
}

/// plot tick labels & chips (orrery uses ~8-11 px for graph annotations)
pub const FACE_XS: Face = Face {
    cw: 7,
    ch: 11,
    yoff: 0,
    bits: include_bytes!("../assets/font_scp_xs_7x11.bin"),
};
/// orrery PX_SM (13 px): labels, readouts, keys, tabs - the workhorse size
pub const FACE_SM: Face = Face {
    cw: 9,
    ch: 15,
    yoff: 7 * 11,
    bits: include_bytes!("../assets/font_scp_sm_9x15.bin"),
};
/// orrery PX (16 px): banners and body text
pub const FACE_MD: Face = Face {
    cw: 11,
    ch: 17,
    yoff: 7 * 11 + 7 * 15,
    bits: include_bytes!("../assets/font_scp_md_11x17.bin"),
};

pub const ATLAS_W: u32 = FONT_ATLAS_COLS * 11; // widest cell = 11 (MD face)
pub const ATLAS_H: u32 = 7 * 11 + 7 * 15 + 7 * 17; // 7 glyph rows per face

/// Glyph slot index: ASCII first, then the extras the HUD needs (en dash,
/// middle dot, multiplication sign, degree, plus-minus, micro, delta).
pub fn glyph_idx(ch: char) -> u32 {
    let c = ch as u32;
    match c {
        32..=127 => c - 32,
        0x2013 => 96,
        0x00B7 => 97,
        0x00D7 => 98,
        0x00B0 => 99,
        0x00B1 => 100,
        0x00B5 => 101,
        0x0394 => 102,
        _ => b'?' as u32 - 32,
    }
}

/// 10 px/degree coastlines (Natural Earth 50m) - crisp at 4K texture size.
/// Plate boundary segments (Bird 2003 PB2002, via OpenLyceum generated
/// data): `[count u32][ {lon0,lat0,lon1,lat1: i16(deg*100), kind: u16}* ]`
/// kind 1=convergent 2=divergent 3=transform.
pub static FAULTLINES: &[u8] = include_bytes!("../assets/faultlines.i16");

/// Decoded boundary segments: (lon0, lat0, lon1, lat1, kind).
pub fn fault_segments() -> Vec<(f32, f32, f32, f32, u16)> {
    let n = u32::from_le_bytes(FAULTLINES[0..4].try_into().unwrap()) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let o = 4 + i * 10;
        let v: [i16; 4] = [
            i16::from_le_bytes(FAULTLINES[o..o + 2].try_into().unwrap()),
            i16::from_le_bytes(FAULTLINES[o + 2..o + 4].try_into().unwrap()),
            i16::from_le_bytes(FAULTLINES[o + 4..o + 6].try_into().unwrap()),
            i16::from_le_bytes(FAULTLINES[o + 6..o + 8].try_into().unwrap()),
        ];
        let k = u16::from_le_bytes(FAULTLINES[o + 8..o + 10].try_into().unwrap());
        out.push((
            v[0] as f32 / 100.0,
            v[1] as f32 / 100.0,
            v[2] as f32 / 100.0,
            v[3] as f32 / 100.0,
            k,
        ));
    }
    out
}

pub const MASK_W: u32 = 3600;
pub const MASK_H: u32 = 1800;
pub static LAND_MASK: &[u8] = include_bytes!("../assets/landmask_3600x1800.bin");

fn mask_bit(x: u32, y: u32) -> bool {
    let i = y * MASK_W + x;
    LAND_MASK[(i / 8) as usize] & (1 << (7 - (i % 8))) != 0
}

/// Land test for geographic degrees.
pub fn land_at(lon_deg: f32, lat_deg: f32) -> bool {
    let x = (((lon_deg + 180.0) / 360.0) * MASK_W as f32) as u32 % MASK_W;
    let y = (((90.0 - lat_deg) / 180.0) * MASK_H as f32) as u32 % MASK_H;
    mask_bit(x, y)
}

/// Build the equirectangular planet texture (RGBA8) in the orrery HMI
/// language: flat hard-edged fields, monochrome + the three accents.
/// Near-black ocean, CELL_DIM land, GRID coastline stroke; plate boundaries
/// in ALARM (convergent) / AMBER (divergent) / CYAN (transform). No scan
/// dither, no depth banding - the reference design language is flat.
/// Graticule lines are drawn procedurally in the shader so they can toggle.
pub fn build_earth_texture(w: u32, h: u32) -> Vec<u8> {
    let mut px = vec![0u8; (w * h * 4) as usize];
    // ---- fault overlay: stamp boundary polylines into a kind buffer ----
    let mut fkind = vec![0u8; (w * h) as usize];
    {
        let segs = fault_segments();
        for (lon0, lat0, lon1, lat1, kind) in segs {
            let dx = (lon1 - lon0 + 540.0).rem_euclid(360.0) - 180.0;
            let dy = lat1 - lat0;
            let steps = (dx.abs() * w as f32 / 360.0)
                .max(dy.abs() * h as f32 / 180.0)
                .max(2.0) as u32;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let lon = lon0 + dx * t;
                let lat = (lat0 + dy * t).clamp(-89.9, 89.9);
                let cx = (((lon + 180.0) / 360.0) * w as f32) as i32;
                let cy = (((90.0 - lat) / 180.0) * h as f32) as i32;
                // plus-shaped stamp: ~2px line at 4K
                for (ox, oy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let sx = (cx + ox).rem_euclid(w as i32) as usize;
                    let sy = (cy + oy).clamp(0, h as i32 - 1) as usize;
                    fkind[sy * w as usize + sx] = kind as u8;
                }
            }
        }
    }
    // sRGB bytes of the orrery palette tokens (texture is Rgba8UnormSrgb)
    let fault_cols: [[u8; 3]; 4] = [
        [0, 0, 0],
        [210, 59, 46],   // convergent  (subduction)  ALARM
        [224, 168, 0],   // divergent   (ridge)       AMBER
        [33, 208, 197],  // transform                 CYAN
    ];
    let ocean: [u8; 3] = [5, 5, 5];
    let land: [u8; 3] = [26, 26, 26]; // CELL_DIM
    let coast: [u8; 3] = [110, 110, 110]; // GRID
    for y in 0..h {
        let lat = 90.0 - (y as f32 + 0.5) / h as f32 * 180.0;
        for x in 0..w {
            let lon = (x as f32 + 0.5) / w as f32 * 360.0 - 180.0;
            let mx = ((lon + 180.0) / 360.0 * MASK_W as f32) as u32 % MASK_W;
            let my = ((90.0 - lat) / 180.0 * MASK_H as f32) as u32 % MASK_H;
            let is_land = mask_bit(mx, my);
            // coastline: land pixel with at least one sea neighbour
            let mut edge = false;
            if is_land {
                for (dx, dy) in [(1i32, 0), (-1, 0), (0, 1), (0, -1)] {
                    let nx = (mx as i32 + dx).rem_euclid(MASK_W as i32) as u32;
                    let ny = (my as i32 + dy).clamp(0, MASK_H as i32 - 1) as u32;
                    if !mask_bit(nx, ny) {
                        edge = true;
                        break;
                    }
                }
            }
            let mut c = if is_land { land } else { ocean };
            if edge {
                c = coast;
            }
            // plate boundaries on top of coastlines
            let fk = fkind[(y * w + x) as usize];
            if fk != 0 {
                c = fault_cols[fk as usize & 3];
            }
            let o = ((y * w + x) * 4) as usize;
            px[o] = c[0];
            px[o + 1] = c[1];
            px[o + 2] = c[2];
            px[o + 3] = 255;
        }
    }
    px
}

/// R8 font atlas: the three faces stacked vertically, 16 glyph columns each,
/// cell pitch = the face's own width. One coverage byte per pixel.
pub fn build_font_atlas() -> (Vec<u8>, u32, u32) {
    let mut buf = vec![0u8; (ATLAS_W * ATLAS_H) as usize];
    for f in [FACE_XS, FACE_SM, FACE_MD] {
        for g in 0..FONT_GLYPHS {
            let gx = (g % FONT_ATLAS_COLS) * f.cw;
            let gy = f.yoff + (g / FONT_ATLAS_COLS) * f.ch;
            let cell = &f.bits[(g * f.ch * f.cw) as usize..((g + 1) * f.ch * f.cw) as usize];
            for r in 0..f.ch {
                let src = &cell[(r * f.cw) as usize..((r + 1) * f.cw) as usize];
                let dst = ((gy + r) * ATLAS_W + gx) as usize;
                buf[dst..dst + f.cw as usize].copy_from_slice(src);
            }
        }
    }
    (buf, ATLAS_W, ATLAS_H)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_sane() {
        assert_eq!(FACE_XS.bits.len(), 103 * 7 * 11);
        assert_eq!(FACE_SM.bits.len(), 103 * 9 * 15);
        assert_eq!(FACE_MD.bits.len(), 103 * 11 * 17);
        assert_eq!(LAND_MASK.len(), (3600 * 1800 / 8) as usize);
        // Amazon basin is land, mid-Pacific is ocean
        assert!(land_at(-60.0, -3.0));
        assert!(!land_at(-150.0, 0.0));
        // Sahara land, north pole ocean-ish (ice not modelled)
        assert!(land_at(13.0, 23.0));
    }

    #[test]
    fn glyph_lookup() {
        assert_eq!(glyph_idx('A'), 33);
        assert_eq!(glyph_idx('\u{2013}'), 96); // en dash
        assert_eq!(glyph_idx('\u{0394}'), 102); // delta
        assert_eq!(glyph_idx('€'), b'?' as u32 - 32); // falls back to '?'
    }

    #[test]
    fn faultlines_decode() {
        let segs = fault_segments();
        assert!(segs.len() > 1000, "PB2002 segment count");
        for (lon0, lat0, lon1, k) in segs.iter().map(|s| (s.0, s.1, s.2, s.4)) {
            assert!(lon0.abs() <= 180.0 && lon1.abs() <= 180.0);
            assert!(lat0.abs() <= 90.0);
            assert!((1..=3).contains(&k));
        }
    }

    #[test]
    fn texture_builds() {
        let t = build_earth_texture(64, 32);
        assert_eq!(t.len(), 64 * 32 * 4);
        let (a, w, h) = build_font_atlas();
        assert_eq!(a.len() as u32, w * h);
        assert!(a.iter().any(|b| *b != 0));
        // 'A' has ink in every face, ' ' has none
        for f in [FACE_XS, FACE_SM, FACE_MD] {
            let a = &f.bits[(33 * f.ch * f.cw) as usize..(34 * f.ch * f.cw) as usize];
            assert!(a.iter().any(|b| *b != 0));
            let s = &f.bits[(0 * f.ch * f.cw) as usize..(1 * f.ch * f.cw) as usize];
            assert!(s.iter().all(|b| *b == 0));
        }
    }
}

/// City night-lights map (GeoNames cities15000, CC-BY 4.0): 8-bit
/// equirectangular, baked at 2048x1024. Returns RGBA8 for upload.
pub fn build_citylights_rgba() -> (Vec<u8>, u32, u32) {
    const W: u32 = 2048;
    const H: u32 = 1024;
    let bin = include_bytes!("../assets/citylights_2048x1024.bin");
    let mut px = vec![0u8; (W * H * 4) as usize];
    for i in 0..(W * H) as usize {
        let v = bin[i];
        px[i * 4] = v;
        px[i * 4 + 1] = (v as u16 * 74 / 100) as u8; // warm sodium tint
        px[i * 4 + 2] = (v as u16 * 42 / 100) as u8;
        px[i * 4 + 3] = 255;
    }
    (px, W, H)
}

/// Capital cities (PPLC): (lat, lon, name). GeoNames, CC-BY 4.0.
pub fn capitals() -> Vec<(f32, f32, String)> {
    let bin = include_bytes!("../assets/capitals.bin");
    let n = u32::from_le_bytes([bin[0], bin[1], bin[2], bin[3]]) as usize;
    let mut o = 4usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let lat = i16::from_le_bytes([bin[o], bin[o + 1]]) as f32 / 100.0;
        let lon = i16::from_le_bytes([bin[o + 2], bin[o + 3]]) as f32 / 100.0;
        let nl = bin[o + 4] as usize;
        let name = String::from_utf8_lossy(&bin[o + 5..o + 5 + nl]).into_owned();
        o += 5 + nl;
        out.push((lat, lon, name));
    }
    out
}
