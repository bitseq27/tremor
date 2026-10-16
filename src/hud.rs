//! HUD composition - interface modelled 1:1 on the orrery HMI design language
//! (github.com/bitseq27/orrery, `crates/app/src/{theme,widgets,layout}.rs`):
//! every panel is a flat 1px-bordered CELL on black with an uppercase title
//! strip and a `[-]` collapse chip; readouts are label-left / value-right rows
//! with hairline ROW_LINE borders; controls are hard-edged HMI keys (active =
//! solid CYAN + black ink, AUTO = AMBER, PAUSE = ALARM); plots are scope
//! frames with dotted grids and white axes; VT323 at the orrery sizes
//! (16 / 13 / 10 px).
//!
//! Layout: header bar (SYS combo · EPOCH · FIND EVENT · FPS · camera tabs ·
//! CAM TARGET · TIPS), left rack (selected object · raw data · logged
//! events), analysis plot column, 3D viewport cell with LEGEND overlay,
//! structure column, map/residual/data-link column, and a bottom console
//! (timeline + span tabs + LIVE, key row with gain slider, status cells).

use cgmath::{Matrix4, Vector3};

use crate::analysis;
use crate::app::App;
use crate::gpu::ui::{UiBatch, MD_W, SM_H, SM_W, XS_H, XS_W};
use crate::model::Quake;
use crate::net::LinkState;
use crate::palette as pal;
use crate::palette::Rgba;
use crate::sensors::SENSORS;
use crate::timeutil;

// ---------------------------------------------------------------- regions --

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UiAction {
    ZoomIn,
    ZoomOut,
    ViewFit,
    ResetView,
    AutoSpin,
    InvX,
    InvY,
    Fullscreen,
    CamFree,
    CamTrack,
    CamTop,
    Span(f32), // hours, 0 = session-all (expanding)
    Rewind,
    Live,
    Pause,
    Simf,
    TglQuakes,
    TglStations,
    TglWaves,
    TglLabels,
    TglGrid,
    TglCities,
    Play,
    Stop,
    Collapse(u8),
    ListEvent(usize),
    Tips,
    FindFocus,
}

#[derive(Debug, Clone, Copy)]
pub struct UiRegion {
    pub r: [f32; 4],
    pub a: UiAction,
}

// ------------------------------------------------------- orrery widgets ---

const ROW_H: f32 = 20.0; // readout row height (orrery: 22 at 13px font)
const ROW_STEP: f32 = 22.0;
const TITLE_H: f32 = 24.0; // title strip + separator (orrery cell_title)
const CHIP: f32 = 16.0; // collapse chip size (orrery collapse_chip)

fn hit(p: (f32, f32), r: [f32; 4]) -> bool {
    p.0 >= r[0] && p.0 <= r[0] + r[2] && p.1 >= r[1] && p.1 <= r[1] + r[3]
}

/// Flat CELL on black with a 1px GRID border (orrery `paint_cell`).
fn cell(b: &mut UiBatch, x: f32, y: f32, w: f32, h: f32) {
    b.rect(x, y, w, h, pal::CELL);
    b.frame(x, y, w, h, pal::GRID);
}

/// Uppercase title, top-left, separator line under it (orrery `cell_title`).
fn cell_title(b: &mut UiBatch, x: f32, y: f32, w: f32, title: &str) {
    b.text_sm(x + 6.0, y + 5.0, title, pal::GRID_HI);
    b.hline(x, y + TITLE_H, w, pal::GRID);
}

/// 16x16 `[–]` / `[+]` chip (orrery `collapse_chip`); pushes its click region.
fn collapse_chip(
    b: &mut UiBatch,
    rg: &mut Vec<UiRegion>,
    x: f32,
    y: f32,
    id: u8,
    open: bool,
    hov: Option<(f32, f32)>,
) {
    let r = [x, y, CHIP, CHIP];
    let h = hov.map_or(false, |p| hit(p, r));
    b.rect(x, y, CHIP, CHIP, if h { pal::CELL_HOV } else { pal::CELL });
    b.frame(x, y, CHIP, CHIP, if h { pal::GRID_HI } else { pal::GRID });
    b.text_sm(
        x + (CHIP - SM_W) * 0.5,
        y + (CHIP - SM_H) * 0.5,
        if open { "\u{2013}" } else { "+" },
        pal::GRID_HI,
    );
    rg.push(UiRegion {
        r,
        a: UiAction::Collapse(id),
    });
}

/// orrery panel: CELL + title strip + collapse chip. When collapsed the cell
/// becomes a thin CELL_DIM strip with a centred `[+ TITLE]` chip
/// (orrery `draw_strip`). Returns (content y, open).
fn panel(
    b: &mut UiBatch,
    rg: &mut Vec<UiRegion>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    title: &str,
    id: u8,
    collapsed: u32,
    hov: Option<(f32, f32)>,
) -> (f32, bool) {
    let open = collapsed & (1 << id) == 0;
    if !open {
        b.rect(x, y, w, 22.0, pal::CELL_DIM);
        b.hline(x, y, w, pal::GRID);
        let cw = SM_W * title.chars().count() as f32 + 26.0;
        let cx = x + (w - cw) * 0.5;
        let r = [cx, y + 2.0, cw, 18.0];
        let hh = hov.map_or(false, |p| hit(p, r));
        b.rect(cx, y + 2.0, cw, 18.0, if hh { pal::CELL_HOV } else { pal::CELL });
        b.frame(cx, y + 2.0, cw, 18.0, if hh { pal::GRID_HI } else { pal::GRID });
        b.text_sm(
            cx + 8.0,
            y + 2.0 + (18.0 - SM_H) * 0.5,
            &format!("+ {title}"),
            pal::GRID_HI,
        );
        rg.push(UiRegion {
            r,
            a: UiAction::Collapse(id),
        });
        return (y + 26.0, false);
    }
    cell(b, x, y, w, h);
    cell_title(b, x, y, w, title);
    collapse_chip(b, rg, x + w - CHIP - 2.0, y + 4.0, id, true, hov);
    (y + TITLE_H + 4.0, true)
}

/// Label / value readout row: hairline ROW_LINE border, label left in GRAY,
/// value right colour-coded (orrery `readout`).
fn row(b: &mut UiBatch, x: f32, y: f32, w: f32, label: &str, value: &str, vc: Rgba) {
    b.frame(x, y, w, ROW_H, pal::ROW_LINE);
    let ty = y + (ROW_H - SM_H) * 0.5;
    b.text_sm(x + 6.0, ty, label, pal::GRAY);
    b.text_right_sm(x + w - 6.0, ty, value, vc);
}

fn key_w(label: &str) -> f32 {
    (SM_W * label.chars().count() as f32 + 14.0).max(34.0)
}

/// Flat HMI key (orrery `hmi_key`): active = accent fill + black ink + white
/// border, hovered = CELL_HOV + white, idle = CELL + GRAY on GRID border.
fn key(
    b: &mut UiBatch,
    rg: &mut Vec<UiRegion>,
    x: f32,
    y: f32,
    h: f32,
    label: &str,
    active: bool,
    accent: Option<Rgba>,
    action: UiAction,
    hov: Option<(f32, f32)>,
) -> f32 {
    let w = key_w(label);
    let r = [x, y, w, h];
    let hh = hov.map_or(false, |p| hit(p, r));
    let (fill, fg, br) = if active {
        (accent.unwrap_or(pal::CYAN), pal::INK, pal::GRID_HI)
    } else if hh {
        (pal::CELL_HOV, pal::GRID_HI, pal::GRID_HI)
    } else {
        (pal::CELL, pal::GRAY, pal::GRID)
    };
    b.rect(x, y, w, h, fill);
    b.frame(x, y, w, h, br);
    b.text_sm(
        x + (w - SM_W * label.chars().count() as f32) * 0.5,
        y + (h - SM_H) * 0.5,
        label,
        fg,
    );
    rg.push(UiRegion { r, a: action });
    x + w + 4.0
}

/// Equal-width tab strip, active tab solid CYAN with black ink
/// (orrery `tab_strip`).
fn tabs(
    b: &mut UiBatch,
    rg: &mut Vec<UiRegion>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    items: &[(&str, UiAction)],
    active: usize,
    hov: Option<(f32, f32)>,
) -> f32 {
    let mut cx = x;
    for (i, (lab, act)) in items.iter().enumerate() {
        let r = [cx, y, w, h];
        let hh = hov.map_or(false, |p| hit(p, r));
        let on = i == active;
        let (fill, fg, br) = if on {
            (pal::CYAN, pal::INK, pal::GRID_HI)
        } else if hh {
            (pal::CELL_HOV, pal::GRID_HI, pal::GRID_HI)
        } else {
            (pal::CELL, pal::GRAY, pal::GRID)
        };
        b.rect(cx, y, w, h, fill);
        b.frame(cx, y, w, h, br);
        b.text_sm(
            cx + (w - SM_W * lab.chars().count() as f32) * 0.5,
            y + (h - SM_H) * 0.5,
            lab,
            fg,
        );
        rg.push(UiRegion {
            r,
            a: *act,
        });
        cx += w + 2.0;
    }
    cx
}

/// Solid status cell with black ink (orrery `status_cell`).
fn status_cell(b: &mut UiBatch, x: f32, y: f32, h: f32, label: &str, fill: Rgba) -> f32 {
    let w = SM_W * label.chars().count() as f32 + 12.0;
    b.rect(x, y, w, h, fill);
    b.frame(x, y, w, h, pal::GRID);
    b.text_sm(
        x + (w - SM_W * label.chars().count() as f32) * 0.5,
        y + (h - SM_H) * 0.5,
        label,
        pal::INK,
    );
    x + w + 5.0
}

/// Discrete segmented bar (orrery `segment_bar`).
fn seg_bar(b: &mut UiBatch, x: f32, y: f32, w: f32, h: f32, frac: f32, segs: usize, on: Rgba) {
    let segs = segs.max(1);
    let gap = 2.0;
    let sw = (w - gap * (segs as f32 - 1.0)) / segs as f32;
    let filled = (frac.clamp(0.0, 1.0) * segs as f32).round() as usize;
    for i in 0..segs {
        let sx = x + i as f32 * (sw + gap);
        b.rect(sx, y, sw, h, if i < filled { on } else { pal::CELL_DIM });
        b.frame(sx, y, sw, h, pal::GRID);
    }
}

fn dotted_vline(b: &mut UiBatch, x: f32, y0: f32, y1: f32, c: Rgba) {
    let mut y = y0;
    while y < y1 {
        b.rect(x, y, 1.0, 2.0f32.min(y1 - y), c);
        y += 5.0;
    }
}
fn dotted_hline(b: &mut UiBatch, x0: f32, y: f32, x1: f32, c: Rgba) {
    let mut x = x0;
    while x < x1 {
        b.rect(x, y, 2.0f32.min(x1 - x), 1.0, c);
        x += 5.0;
    }
}

/// Scope-style plot cell (orrery `scope_frame` / `spec_graph`): black field,
/// GRID border, title inside top-left, dotted PLOT_DOT grid, white axes with
/// 4px ticks, axis label bottom-right. Returns the inner plot rect.
fn plot(b: &mut UiBatch, x: f32, y: f32, w: f32, h: f32, title: &str, xlab: &str) -> [f32; 4] {
    b.rect(x, y, w, h, pal::BG);
    b.frame(x, y, w, h, pal::GRID);
    b.text_xs(x + 4.0, y + 2.0, title, pal::GRID_HI);
    b.text_right_xs(x + w - 3.0, y + h - XS_H - 2.0, xlab, pal::GRAY);
    let inner = [x + 10.0, y + 14.0, w - 16.0, h - 26.0];
    for i in 1..4 {
        let f = i as f32 / 4.0;
        dotted_vline(b, inner[0] + inner[2] * f, inner[1], inner[1] + inner[3], pal::PLOT_DOT);
        dotted_hline(b, inner[0], inner[1] + inner[3] * f, inner[0] + inner[2], pal::PLOT_DOT);
    }
    b.hline(inner[0], inner[1] + inner[3], inner[2], pal::GRID_HI);
    b.vline(inner[0], inner[1], inner[3], pal::GRID_HI);
    for i in 0..=4 {
        let f = i as f32 / 4.0;
        let tx = inner[0] + inner[2] * f;
        let ty = inner[1] + inner[3] - inner[3] * f;
        b.rect(tx, inner[1] + inner[3], 1.0, 4.0, pal::GRID_HI);
        b.rect(inner[0] - 4.0, ty, 4.0, 1.0, pal::GRID_HI);
    }
    inner
}

/// Pixel triangles for glyphs VT323 does not carry (▼ ◀◀).
fn tri_down(b: &mut UiBatch, cx: f32, cy: f32, c: Rgba) {
    for (i, w) in [7.0, 5.0, 3.0, 1.0].iter().enumerate() {
        b.rect(cx - w * 0.5, cy + i as f32, *w, 1.0, c);
    }
}
fn tri_left(b: &mut UiBatch, cx: f32, cy: f32, c: Rgba) {
    for (i, w) in [1.0, 3.0, 5.0, 7.0, 5.0, 3.0, 1.0].iter().enumerate() {
        b.rect(cx - w * 0.5, cy + i as f32, *w, 1.0, c);
    }
}

// ------------------------------------------------------------------- HUD --

pub fn build_hud(a: &mut App, w: u32, h: u32) {
    let s = a.ui_scale;
    let (w, h) = (w as f32, h as f32);
    let now = a.now;
    let ph = a.playhead;
    let live = a.live;
    let fps = a.fps;
    let link = a.link;
    let tg = a.toggles;
    let sel_i = a.selected_index();
    let sel_sta = a.selected_station;
    let hov = a.drag_last.map(|(x, y)| (x as f32, y as f32));
    let tracked = a.cam.focus.is_some() && now < a.focus_until;
    let track_secs = (a.focus_until - now).max(0.0);

    let events_live: Vec<Quake> = a.store.events().iter().take(512).cloned().collect();
    let events_at: Vec<Quake> = events_live
        .iter()
        .filter(|q| q.time_s() <= ph + 0.5)
        .cloned()
        .collect();
    let stats = a.store.stats(ph);
    let zones = analysis::depth_zones(&events_at);
    let gr = analysis::gr_cumulative(&events_at);
    let waves_n = a.store.waves().iter().filter(|wv| wv.start <= ph).count();
    let gain = a.trace_gain;

    let win_end = now;
    let win_start = now - a.win_mode.max(1.0 / 6.0) as f64 * 3600.0;

    let b = &mut a.batch;
    let mut rg: Vec<UiRegion> = Vec::new();

    // ============================== row 0: header bar =====================
    let hdr_h = (30.0 * s).max(30.0);
    b.rect(0.0, 0.0, w, hdr_h, pal::BG);
    b.hline(0.0, hdr_h - 1.0, w, pal::GRID);
    let ch_h = (22.0 * s).max(20.0);
    let cy0 = (hdr_h - ch_h) * 0.5;
    let tyc = cy0 + (ch_h - SM_H) * 0.5;

    // SYS combo cell (orrery header R7)
    let sys_w = (190.0 * s).max(160.0);
    cell(b, 8.0, cy0, sys_w, ch_h);
    b.text_sm(14.0, tyc, &format!("SYS: TREMOR v{}", env!("CARGO_PKG_VERSION")), pal::CYAN);
    tri_down(b, 8.0 + sys_w - 12.0, cy0 + (ch_h - 4.0) * 0.5, pal::GRAY);
    let mut x = 8.0 + sys_w + 14.0;

    b.text_sm(x, tyc, &format!("EPOCH {}", timeutil::fmt_utc(ph)), pal::GRID_HI);
    x += SM_W * 26.0 + 14.0;
    if !live {
        x = status_cell(b, x, cy0 + (ch_h - 20.0) * 0.5, 20.0, "REVIEW", pal::AMBER);
    }
    // FIND EVENT field (orrery FIND BODY: populate-as-you-type)
    let fw = (200.0 * s).clamp(120.0, 320.0);
    let fr = [x, cy0, fw, ch_h];
    a.find_rect = fr;
    cell(b, x, cy0, fw, ch_h);
    let q = a.find_text.trim().to_uppercase();
    if q.is_empty() {
        b.text_sm(x + 6.0, tyc, "FIND EVENT...", pal::GRAY);
    } else {
        let tx = b.text_sm(x + 6.0, tyc, &a.find_text, pal::GRID_HI);
        if a.find_focused && (now * 2.0) as u64 % 2 == 0 {
            b.rect(tx, tyc, SM_W * 0.6, SM_H, pal::CYAN);
        }
    }
    rg.push(UiRegion {
        r: fr,
        a: UiAction::FindFocus,
    });
    x += fw + 12.0;

    // FPS (orrery M6 perf visibility: green > 50, amber > 25, else alarm)
    let fcol = if fps > 50.0 {
        pal::GREEN
    } else if fps > 25.0 {
        pal::AMBER
    } else {
        pal::ALARM
    };
    b.text_sm(x, tyc, &format!("FPS {:>5.1}", fps), fcol);

    // right cluster: TIPS toggle, camera tabs, CAM TARGET
    let tips_w = (70.0 * s).max(58.0);
    let trx = w - 8.0 - tips_w;
    let tr = [trx, cy0, tips_w, ch_h];
    let hh = hov.map_or(false, |p| hit(p, tr));
    let (fill, fg, br) = if a.tips {
        (pal::CYAN, pal::INK, pal::GRID_HI)
    } else if hh {
        (pal::CELL_HOV, pal::GRID_HI, pal::GRID_HI)
    } else {
        (pal::CELL, pal::GRAY, pal::GRID)
    };
    b.rect(trx, cy0, tips_w, ch_h, fill);
    b.frame(trx, cy0, tips_w, ch_h, br);
    b.text_sm(
        trx + (tips_w - SM_W * 7.0) * 0.5,
        tyc,
        if a.tips { "TIPS ON" } else { "TIPS OFF" },
        fg,
    );
    rg.push(UiRegion {
        r: tr,
        a: UiAction::Tips,
    });

    let camtxt = if tracked {
        format!("CAM TRACK: M{:.1} {:02}S", a.tracked_mag, track_secs as u32)
    } else {
        "CAM TARGET: EARTH".to_string()
    };
    let camw = SM_W * camtxt.chars().count() as f32;
    b.text_right_sm(trx - 14.0, tyc, &camtxt, pal::AMBER);
    let tab_w = (46.0 * s).max(40.0);
    let mut cx = trx - 14.0 - camw - 14.0 - tab_w * 3.0 - 4.0;
    let cam_active = if a.auto_track { 1 } else { 0 };
    let items: [(&str, UiAction); 3] = [
        ("FREE", UiAction::CamFree),
        ("TRACK", UiAction::CamTrack),
        ("TOP", UiAction::CamTop),
    ];
    for (i, (lab, act)) in items.iter().enumerate() {
        let r = [cx, cy0, tab_w, ch_h];
        let hh = hov.map_or(false, |p| hit(p, r));
        let on = i == cam_active;
        let (fill, fg, br) = if on {
            (pal::CYAN, pal::INK, pal::GRID_HI)
        } else if hh {
            (pal::CELL_HOV, pal::GRID_HI, pal::GRID_HI)
        } else {
            (pal::CELL, pal::GRAY, pal::GRID)
        };
        b.rect(cx, cy0, tab_w, ch_h, fill);
        b.frame(cx, cy0, tab_w, ch_h, br);
        b.text_sm(
            cx + (tab_w - SM_W * lab.chars().count() as f32) * 0.5,
            tyc,
            lab,
            fg,
        );
        rg.push(UiRegion { r, a: *act });
        cx += tab_w + 2.0;
    }

    // ============================== column geometry =======================
    let top = hdr_h + 6.0 * s;
    let bot_h = 46.0 * s + 56.0;
    let colw = 252.0 * s;
    let gap = 8.0 * s;
    let lx = 8.0 * s;
    let c2x = lx + colw + gap;
    let c3x = w - colw * 2.0 - gap * 2.0 - 8.0 * s;
    let c4x = w - colw - 8.0 * s;
    let colh = h - bot_h - top - 6.0;
    let vpx = c2x + colw + gap;
    let vpw = (c3x - gap) - vpx;

    // ============================== col 1: selected object =================
    let ph_h = colh * 0.44;
    let (mut y, o1) = panel(b, &mut rg, lx, top, colw, ph_h, "SELECTED OBJECT", 0, a.panels_collapsed, hov);
    if o1 {
        let ix = lx + 8.0;
        let iw = colw - 16.0;
        // name banner (orrery selected_object_cell): solid CYAN + black ink
        let banner = (ix, y, iw, 26.0);
        let name = if let Some(si) = sel_sta {
            format!("{} - {}", SENSORS[si].code, SENSORS[si].place)
        } else if let Some(q) = sel_i.and_then(|i| a.store.get(i).cloned()) {
            format!("M{:.1} {}", q.mag, q.place)
        } else {
            String::new()
        };
        if name.is_empty() {
            b.rect(banner.0, banner.1, banner.2, banner.3, pal::CELL);
            b.frame(banner.0, banner.1, banner.2, banner.3, pal::GRID);
            b.text(banner.0 + (banner.2 - MD_W * 3.0) * 0.5, banner.1 + 5.0, "---", pal::GRAY);
        } else {
            b.rect(banner.0, banner.1, banner.2, banner.3, pal::CYAN);
            b.frame(banner.0, banner.1, banner.2, banner.3, pal::GRID);
            let mut nm = name.clone();
            let maxc = (banner.2 / MD_W) as usize;
            if nm.chars().count() > maxc {
                nm = nm.chars().take(maxc.saturating_sub(1)).collect();
                nm.push('>');
            }
            b.text(
                banner.0 + (banner.2 - MD_W * nm.chars().count() as f32) * 0.5,
                banner.1 + 5.0,
                &nm,
                pal::INK,
            );
        }
        y += 32.0;
        if let Some(si) = sel_sta {
            let st = &SENSORS[si];
            row(b, ix, y, iw, "TYPE", "SENSOR STN", pal::GRID_HI); y += ROW_STEP;
            row(b, ix, y, iw, "CODE", st.code, pal::CYAN); y += ROW_STEP;
            row(b, ix, y, iw, "NETWORK", "GSN / IU CLASS", pal::GRID_HI); y += ROW_STEP;
            row(b, ix, y, iw, "PLACE", st.place, pal::CYAN); y += ROW_STEP;
            row(b, ix, y, iw, "LAT", &format!("{:.2} DEG", st.lat), pal::GRID_HI); y += ROW_STEP;
            row(b, ix, y, iw, "LON", &format!("{:.2} DEG", st.lon), pal::GRID_HI); y += ROW_STEP;
            row(b, ix, y, iw, "CHAN", "BHZ 40 SPS", pal::GRID_HI); y += ROW_STEP;
            row(b, ix, y, iw, "STATUS", "ONLINE", pal::GREEN); y += ROW_STEP;
            let d = analysis::haversine_deg(st.lat, st.lon, 0.0, 0.0);
            row(b, ix, y, iw, "GEOCENT", &format!("{:.0} DEG", d), pal::GRAY); y += ROW_STEP;
            let nev = events_at
                .iter()
                .filter(|q| analysis::haversine_deg(st.lat, st.lon, q.lat, q.lon) < 25.0)
                .count();
            row(b, ix, y, iw, "EVT <25DEG", &format!("{nev}"), pal::AMBER);
        } else if let Some(q) = sel_i.and_then(|i| a.store.get(i).cloned()) {
            let mc = pal::mag_color(q.mag);
            row(b, ix, y, iw, "TYPE", if q.simulated { "SIM QUAKE" } else { "QUAKE" }, pal::GRID_HI); y += ROW_STEP;
            row(b, ix, y, iw, "MAG", &format!("{:.1} {}", q.mag, pal::mag_tag(q.mag)), mc); y += ROW_STEP;
            row(b, ix, y, iw, "DEPTH", &format!("{:.1} KM", q.depth_km), pal::GRID_HI); y += ROW_STEP;
            row(b, ix, y, iw, "TIME", &timeutil::fmt_utc(q.time_s()), pal::GRID_HI); y += ROW_STEP;
            row(b, ix, y, iw, "LAT", &format!("{:.3} DEG", q.lat), pal::GRID_HI); y += ROW_STEP;
            row(b, ix, y, iw, "LON", &format!("{:.3} DEG", q.lon), pal::GRID_HI); y += ROW_STEP;
            row(b, ix, y, iw, "AGE@PH", &format!("{:.0} S", (ph - q.time_s()).max(0.0)), pal::AMBER); y += ROW_STEP;
            let p1: String = q.place.chars().take(24).collect();
            row(b, ix, y, iw, "REGION", &p1, pal::CYAN); y += ROW_STEP;
            let wv = a.store.waves().iter().find(|wv| wv.id == q.id && wv.start <= ph);
            let wtxt = match wv {
                Some(wv) => format!("ACTIVE {:.0}S", (ph - wv.start).max(0.0)),
                None => "DISSIPATED".to_string(),
            };
            row(b, ix, y, iw, "WAVE", &wtxt, if wv.is_some() { pal::GREEN } else { pal::GRAY });
        } else {
            b.text_sm(ix, y + 4.0, "NONE - CLICK A MARKER,", pal::GRAY);
            b.text_sm(ix, y + 4.0 + ROW_STEP, "STATION CROSS OR LIST ROW", pal::GRAY);
        }
    }

    // raw data cell (orrery RAW DATA)
    let raw_h = colh * 0.20;
    let ry0 = top + ph_h + 8.0;
    let (mut y, o2) = panel(b, &mut rg, lx, ry0, colw, raw_h, "RAW DATA", 1, a.panels_collapsed, hov);
    if o2 {
        let ix = lx + 8.0;
        let iw = colw - 16.0;
        let p = if let Some(si) = sel_sta {
            crate::model::geo_to_cartesian(SENSORS[si].lat, SENSORS[si].lon, 1.0)
        } else if let Some(q) = sel_i.and_then(|i| a.store.get(i).cloned()) {
            q.unit_pos()
        } else {
            [0.0; 3]
        };
        let has = sel_sta.is_some() || sel_i.is_some();
        let f = |v: f32| if has { format!("{v:+.6}") } else { "---.------".to_string() };
        row(b, ix, y, iw, "X", &f(p[0]), pal::GRID_HI); y += ROW_STEP;
        row(b, ix, y, iw, "Y", &f(p[1]), pal::GRID_HI); y += ROW_STEP;
        row(b, ix, y, iw, "Z", &f(p[2]), pal::GRID_HI); y += ROW_STEP;
        b.text_xs(ix, y + 2.0, "UNIT SPHERE CARTESIAN", pal::GRAY);
    }

    // logged events list (clickable -> playhead jump)
    let ly0 = ry0 + raw_h + 8.0;
    let lh = top + colh - ly0;
    let (mut y, o3) = panel(b, &mut rg, lx, ly0, colw, lh, "LOGGED EVT (CLICK=JUMP)", 2, a.panels_collapsed, hov);
    if o3 {
        let ix = lx + 8.0;
        a.log_rect = [lx, ly0, colw, lh];
        let rows = ((lh - 26.0) / 19.0).max(4.0) as usize;
        let maxs = events_live.len().saturating_sub(rows) as f32;
        a.log_scroll = a.log_scroll.clamp(0.0, maxs);
        let off = a.log_scroll.round() as usize;
        for (i, q) in events_live.iter().enumerate().skip(off).take(rows) {
            if y + 18.0 > ly0 + lh - 4.0 {
                break;
            }
            let rr = [lx + 2.0, y, colw - 4.0, 18.0];
            let hh = hov.map_or(false, |p| hit(p, rr));
            let seld = Some(i) == sel_i;
            if seld {
                b.rect(rr[0], rr[1], rr[2], rr[3], pal::CYAN);
            } else if hh {
                b.rect(rr[0], rr[1], rr[2], rr[3], pal::CELL_HOV);
            }
            let (tc, mc, pc) = if seld {
                (pal::INK, pal::INK, pal::INK)
            } else if hh {
                (pal::GRID_HI, pal::mag_color(q.mag), pal::GRID_HI)
            } else {
                (pal::GRAY, pal::mag_color(q.mag), pal::GRID)
            };
            b.text_sm(ix, y + 2.0, &timeutil::fmt_hms(q.time_s()), tc);
            b.text_sm(ix + SM_W * 9.0, y + 2.0, &format!("{:4.1}", q.mag), mc);
            let pl: String = q.place.chars().take(((colw - 16.0 - SM_W * 14.0) / XS_W) as usize).collect();
            b.text_xs(ix + SM_W * 14.0, y + 4.0, &pl, pc);
            rg.push(UiRegion {
                r: rr,
                a: UiAction::ListEvent(i),
            });
            y += 19.0;
        }
        // scrollbar: wheel over the panel scrolls (see app wheel handler)
        if maxs > 0.0 {
            let ty0 = ly0 + 16.0;
            let th0 = lh - 24.0;
            b.rect(lx + colw - 5.0, ty0, 3.0, th0, pal::CELL_DIM);
            let fh = (th0 * rows as f32 / events_live.len().max(1) as f32).max(14.0);
            let fy = ty0 + (th0 - fh) * (a.log_scroll / maxs);
            b.rect(lx + colw - 5.0, fy, 3.0, fh, pal::GRID);
            b.text_xs(lx + 6.0, ly0 + lh - 13.0, &format!("WHEEL=SCROLL {}/{}", off + rows.min(events_live.len() - off), events_live.len()), pal::GRAY);
        }
    }

    // ============================== col 2: analysis plots =================
    let st_name = sel_sta.map(|i| SENSORS[i].code).unwrap_or("ANMO");
    let mut y = top;
    let p1h = colh * 0.34;
    let inner = plot(b, c2x, y, colw, p1h, &format!("STATION TRACE BHZ - {st_name}"), "300S");
    let trace = analysis::station_trace(sel_sta.unwrap_or(0), &events_at, ph, 300.0, 160, gain);
    let pts: Vec<(f32, f32)> = trace
        .iter()
        .enumerate()
        .map(|(i, v)| {
            (
                inner[0] + inner[2] * i as f32 / trace.len().max(1) as f32,
                inner[1] + inner[3] * 0.5 - v * inner[3] * 0.45,
            )
        })
        .collect();
    dotted_hline(b, inner[0], inner[1] + inner[3] * 0.5, inner[0] + inner[2], pal::PLOT_DOT);
    b.polyline(&pts, 1.0, pal::GREEN);
    y += p1h + 8.0;
    let p2h = colh * 0.22;
    let inner = plot(b, c2x, y, colw, p2h, "GUTENBERG-RICHTER CUM", "LOG N");
    let maxc = gr.iter().map(|g| g.1).max().unwrap_or(1).max(1);
    let gpts: Vec<(f32, f32)> = gr
        .iter()
        .enumerate()
        .map(|(i, g)| {
            (
                inner[0] + inner[2] * i as f32 / 8.0,
                inner[1] + inner[3] - inner[3] * g.1 as f32 / maxc as f32,
            )
        })
        .collect();
    b.polyline(&gpts, 1.0, pal::CYAN);
    for (i, g) in gr.iter().enumerate() {
        if i % 2 == 0 {
            b.text_xs(inner[0] + inner[2] * i as f32 / 8.0 - 3.0, inner[1] + inner[3] + 5.0, &format!("{:.0}", g.0), pal::GRAY);
        }
    }
    y += p2h + 8.0;
    let p3h = colh * 0.22;
    let inner = plot(b, c2x, y, colw, p3h, "DEPTH vs TIME", "KM");
    for q in events_at.iter().take(200) {
        let fx = ((q.time_s() - win_start) / (win_end - win_start).max(1.0)).clamp(0.0, 1.0) as f32;
        let fy = (q.depth_km / 700.0).clamp(0.0, 1.0);
        b.rect(inner[0] + inner[2] * fx, inner[1] + inner[3] * fy, 2.0, 2.0, pal::mag_color(q.mag));
    }
    b.text_xs(inner[0], inner[1] + inner[3] + 5.0, "0", pal::GRAY);
    b.text_right_xs(inner[0] + inner[2], inner[1] + inner[3] + 5.0, "700", pal::GRAY);
    y += p3h + 8.0;
    let p4h = (top + colh) - y;
    let inner = plot(b, c2x, y, colw, p4h, "MAG HISTOGRAM", "N");
    let mb = stats.mag_bins;
    let mx = mb.iter().max().copied().unwrap_or(1).max(1);
    for (i, c) in mb.iter().enumerate() {
        let bw2 = inner[2] / 6.0;
        let bh = inner[3] * *c as f32 / mx as f32;
        b.rect(inner[0] + bw2 * (i as f32 + 0.5), inner[1] + inner[3] - bh, bw2 * 0.7, bh.max(1.0), pal::mag_color(4.5 + i as f32));
        b.frame(inner[0] + bw2 * (i as f32 + 0.5), inner[1] + inner[3] - bh, bw2 * 0.7, bh.max(1.0), pal::GRID);
    }

    // ============================== col 3: event structure ================
    let (mut y, o4) = panel(b, &mut rg, c3x, top, colw, colh, "EVENT STRUCTURE - EARTH", 3, a.panels_collapsed, hov);
    if o4 {
        let ix3 = c3x + 8.0;
        let iw3 = colw - 16.0;
        let max_at = events_at.iter().map(|q| q.mag).fold(0.0f32, f32::max);
        // section banner: solid CYAN + black ink (orrery status cell)
        b.rect(ix3, y, iw3, 22.0, pal::CYAN);
        b.frame(ix3, y, iw3, 22.0, pal::GRID);
        b.text_sm(ix3 + 6.0, y + 4.0, &format!("EVENTS {} \u{00b7} MAX M{:.1}", events_at.len(), max_at), pal::INK);
        y += 28.0;
        let shells: [(&str, u32, Rgba); 8] = [
            ("CRUSTAL <70 KM", zones[0], pal::GREEN),
            ("UPPER 70-300 KM", zones[1], pal::AMBER),
            ("LOWER 300-700 KM", zones[2], pal::ALARM),
            ("BIN M4.0-4.9", mb[0], pal::GREEN),
            ("BIN M5.0-5.9", mb[1], pal::AMBER),
            ("BIN M6.0-6.9", mb[2], pal::ALARM),
            ("BIN M7.0-7.9", mb[3], pal::GRID_HI),
            ("BIN M8.0+", mb[4], pal::GRID_HI),
        ];
        let maxcnt = shells.iter().map(|s| s.1).max().unwrap_or(1).max(1);
        for (n, (label, cnt, col)) in shells.iter().enumerate() {
            // numbered cell (orrery number_cell, inactive chrome)
            b.rect(ix3, y, 16.0, 16.0, pal::CELL);
            b.frame(ix3, y, 16.0, 16.0, pal::GRID);
            b.text_xs(ix3 + (16.0 - XS_W) * 0.5, y + 3.0, &format!("{}", n + 1), *col);
            b.text_sm(ix3 + 22.0, y + 1.0, label, pal::GRAY);
            b.text_right_sm(ix3 + iw3, y + 1.0, &format!("{cnt} EVT"), pal::GRID_HI);
            seg_bar(b, ix3 + 22.0, y + 17.0, iw3 - 22.0, 5.0, *cnt as f32 / maxcnt as f32, 12, *col);
            y += 28.0;
        }
        y += 4.0;
        row(b, ix3, y, iw3, "EVT 1H@PH", &format!("{}", stats.count_1h), pal::GRID_HI); y += ROW_STEP;
        row(b, ix3, y, iw3, "AVG DEPTH", &format!("{:.0} KM", stats.avg_depth_km), pal::GRID_HI); y += ROW_STEP;
        row(b, ix3, y, iw3, "WAVES@PH", &format!("{waves_n}"), pal::GREEN);
    }

    // ============================== col 4: map + data link ================
    let map_h = colh * 0.34;
    let (mut y, o5) = panel(b, &mut rg, c4x, top, colw, map_h, "MAP - LAT/LON", 4, a.panels_collapsed, hov);
    let mw = colw - 16.0;
    let area_h = map_h - (y - top) - 24.0;
    let area_y = y + 20.0;
    // equirectangular is 2:1 - letterbox the map inside the panel so land
    // keeps true proportions instead of stretching to the panel shape
    let mapw = mw.min(area_h * 2.0);
    let maph = mapw * 0.5;
    let mx0 = c4x + 8.0 + (mw - mapw) * 0.5;
    let my0 = area_y;
    if o5 {
        status_cell(b, c4x + 8.0, y, 16.0, "EPICENTRES", pal::CYAN);
        b.rect(mx0, my0, mapw, maph, pal::BG);
        b.frame(mx0, my0, mapw, maph, pal::ROW_LINE);
        // graticule: dotted PLOT_DOT lines every 60 lon / 45 lat
        for k in 1..6 {
            let gx = mx0 + mapw * k as f32 / 6.0;
            dotted_vline(b, gx, my0, my0 + maph, pal::PLOT_DOT);
        }
        for k in 1..4 {
            let gy = my0 + maph * k as f32 / 4.0;
            dotted_hline(b, mx0, gy, mx0 + mapw, pal::PLOT_DOT);
        }
        for (cx, cy) in analysis::coast_dots() {
            let px = mx0 + mapw * *cx as f32 / crate::assets::MASK_W as f32;
            let py = my0 + maph * *cy as f32 / crate::assets::MASK_H as f32;
            b.rect(px, py, 1.0, 1.0, pal::PLOT_FAINT);
        }
        for (i, st) in SENSORS.iter().enumerate() {
            let px = mx0 + mapw * (st.lon + 180.0) / 360.0;
            let py = my0 + maph * (90.0 - st.lat) / 180.0;
            let col = if Some(i) == sel_sta { pal::AMBER } else { pal::GRID_HI };
            b.rect(px - 1.0, py, 3.0, 1.0, col);
            b.rect(px, py - 1.0, 1.0, 3.0, col);
        }
        for q in events_at.iter().take(300) {
            let px = mx0 + mapw * (q.lon + 180.0) / 360.0;
            let py = my0 + maph * (90.0 - q.lat) / 180.0;
            let sz = 2.0 + (q.mag - 4.0).clamp(0.0, 4.0) * 1.6;
            b.rect(px - sz * 0.5, py - sz * 0.5, sz, sz, pal::mag_color(q.mag));
        }
        // key under the map: dot size/colour = magnitude, reticle = newest
        let ky0 = my0 + maph + 10.0;
        let mut kx = mx0;
        for (lab, m) in [("M4", 4.0f32), ("M5", 5.0), ("M6", 6.0), ("M7+", 7.5)] {
            let sz = 2.0 + (m - 4.0).clamp(0.0, 4.0) * 1.6;
            b.rect(kx, ky0 + 4.0 - sz * 0.5, sz, sz, pal::mag_color(m));
            b.text_xs(kx + 6.0, ky0, lab, pal::GRAY);
            kx += 6.0 + XS_W * lab.chars().count() as f32 + 10.0;
        }
        // newest-event reticle sample (crosshair + circle)
        let rx = kx + 6.0;
        b.rect(rx - 1.0, ky0 + 4.0, 3.0, 1.0, pal::GRID_HI);
        b.rect(rx, ky0 + 3.0, 1.0, 3.0, pal::GRID_HI);
        b.frame(rx - 4.0, ky0, 9.0, 9.0, pal::GRID_HI);
        b.text_xs(rx + 8.0, ky0, "LATEST", pal::GRAY);
        b.text_right_xs(c4x + colw - 8.0, ky0 + 14.0, "EQUIRECT 2:1", pal::GRAY);
    }

    y = area_y + area_h + 8.0;
    let res_h = colh * 0.26;
    {
        let inner = plot(b, c4x, y, colw, res_h, "RESIDUAL LOG vs BINS", &timeutil::fmt_utc(ph)[..10].to_string());
        let mx2 = mb.iter().max().copied().unwrap_or(1).max(1);
        for (i, c) in mb.iter().enumerate() {
            let bw2 = inner[2] / 5.5;
            let bh = inner[3] * *c as f32 / mx2 as f32;
            b.rect(inner[0] + bw2 * i as f32, inner[1] + inner[3] - bh, bw2 * 0.75, bh.max(1.0), pal::mag_color(4.5 + i as f32));
            b.frame(inner[0] + bw2 * i as f32, inner[1] + inner[3] - bh, bw2 * 0.75, bh.max(1.0), pal::GRID);
            b.text_xs(inner[0] + bw2 * i as f32, inner[1] + inner[3] + 5.0, &format!("{c}"), pal::GRAY);
        }
    }
    y += res_h + 8.0;
    let dl_h = (top + colh) - y;
    let (mut y, o7) = panel(b, &mut rg, c4x, y, colw, dl_h, "DATA LINK", 5, a.panels_collapsed, hov);
    if o7 {
        let ix4 = c4x + 8.0;
        let iw4 = colw - 16.0;
        let (ls, lc) = match link {
            LinkState::Live => ("LIVE", pal::GREEN),
            LinkState::Sim => ("SIM", pal::AMBER),
            LinkState::Off => ("OFF", pal::ALARM),
        };
        row(b, ix4, y, iw4, "LINK", ls, lc); y += ROW_STEP;
        row(b, ix4, y, iw4, "FEED", a.feed_src, pal::GRID_HI); y += ROW_STEP;
        row(b, ix4, y, iw4, "POLL", "WS RT + 60 S", pal::GRID_HI); y += ROW_STEP;
        row(b, ix4, y, iw4, "INGESTED", &format!("{}", a.store.ingested), pal::GRID_HI); y += ROW_STEP;
        row(b, ix4, y, iw4, "DUPES", &format!("{}", a.store.dropped_dupes), pal::GRAY); y += ROW_STEP + 4.0;
        key(b, &mut rg, ix4, y, 22.0, "SIMF", a.sim_local, None, UiAction::Simf, hov);
    }

    // ============================== centre: 3D viewport cell ==============
    b.frame(vpx, top, vpw, colh, pal::GRID);
    b.text_sm(vpx + 8.0, top + 6.0, "3D SEISMIC VIEW", pal::GRAY);
    b.text_right_sm(vpx + vpw - 8.0, top + 6.0, &format!("OBSERVER EARTH \u{00b7} PLAYHEAD {}", timeutil::fmt_hms(ph)), pal::GRAY);

    // LEGEND overlay (orrery draw_legend): translucent black + GRID border
    let lgx = vpx + 8.0;
    let lgy = top + 26.0;
    let lgw = 236.0;
    let open = a.panels_collapsed & (1 << 6) == 0;
    if open {
        let rows: [(&str, Rgba); 6] = [
            ("M 4.0-4.9 MINOR", pal::GREEN),
            ("M 5.0-5.9 MODERATE", pal::AMBER),
            ("M 6.0-6.9 STRONG", pal::ALARM),
            ("M 7.0+    MAJOR", pal::GRID_HI),
            ("SIMULATED EVENT", pal::CYAN),
            ("+ SENSOR STATION", pal::GRID_HI),
        ];
        let lgh = 26.0 + rows.len() as f32 * 14.0 + 16.0;
        b.rect(lgx, lgy, lgw, lgh, [0.0, 0.0, 0.0, 0.82]);
        b.frame(lgx, lgy, lgw, lgh, pal::GRID);
        b.text_sm(lgx + 6.0, lgy + 4.0, "LEGEND \u{2013} OBJECT CLASSES", pal::GRID_HI);
        for (i, (label, col)) in rows.iter().enumerate() {
            let ry = lgy + 24.0 + i as f32 * 14.0;
            if *label == "+ SENSOR STATION" {
                b.rect(lgx + 9.0, ry + 4.0, 7.0, 1.0, *col);
                b.rect(lgx + 12.0, ry + 1.0, 1.0, 7.0, *col);
            } else {
                b.rect(lgx + 9.0, ry + 2.0, 6.0, 6.0, *col);
            }
            b.text_xs(lgx + 22.0, ry + 1.0, label, pal::GRID);
        }
        b.text_xs(
            lgx + 6.0,
            lgy + lgh - 13.0,
            &format!("{} \u{00b7} EVT {} \u{00b7} STA {}", a.feed_src, events_live.len(), SENSORS.len()),
            pal::GRAY,
        );
        collapse_chip(b, &mut rg, lgx + lgw - CHIP - 2.0, lgy + 2.0, 6, true, hov);
    } else {
        let r = [lgx, lgy, 66.0, 16.0];
        let hh = hov.map_or(false, |p| hit(p, r));
        b.rect(lgx, lgy, 66.0, 16.0, [0.0, 0.0, 0.0, 0.82]);
        b.frame(lgx, lgy, 66.0, 16.0, if hh { pal::GRID_HI } else { pal::GRID });
        b.text_xs(lgx + 8.0, lgy + 3.0, "LEGEND +", pal::GRID_HI);
        rg.push(UiRegion {
            r,
            a: UiAction::Collapse(6),
        });
    }

    // ============================== bottom: timeline row ==================
    let ty = h - bot_h + 2.0;
    let th = 44.0 * s;
    b.rect(0.0, ty - 2.0, w, th + 4.0, pal::BG);
    b.hline(0.0, ty - 2.0, w, pal::GRID);
    // T-LINE label cell
    let tl_lab_w = SM_W * 6.0 + 16.0;
    cell(b, 8.0, ty, tl_lab_w, th);
    b.text_sm(14.0, ty + 4.0, "T-LINE", pal::CYAN);
    b.text_xs(14.0, ty + th - XS_H - 3.0, &timeutil::fmt_hms(win_start), pal::GRAY);
    let live_w = SM_W * 4.0 + 16.0;
    let tab_w2 = (40.0 * s).max(34.0);
    let rew_w = 30.0;
    let span_w = tab_w2 * 5.0 + 2.0 * 5.0 + rew_w + 6.0;
    let tlx = 8.0 + tl_lab_w + 6.0;
    let tlw = w - 8.0 - live_w - 6.0 - span_w - tlx;
    let tly = ty;
    let tlh = th;
    b.rect(tlx, tly, tlw, tlh, pal::BG);
    b.frame(tlx, tly, tlw, tlh, pal::GRID);
    a.timeline_rect = [tlx, tly, tlw, tlh];
    let span = (win_end - win_start).max(60.0);
    let steps = [60.0, 300.0, 900.0, 1800.0, 3600.0, 7200.0, 21600.0];
    let step = *steps.iter().find(|st| span / **st <= 10.0).unwrap_or(&86400.0);
    let mut t = (win_start / step).ceil() * step;
    let mut k = 0u32;
    let label_every = if span > 12.0 * 3600.0 { 2 } else { 1 };
    while t < win_end {
        let px = tlx + tlw * ((t - win_start) / span) as f32;
        b.vline(px, tly + 4.0, tlh - 8.0, pal::PLOT_DOT);
        b.rect(px, tly + tlh - 4.0, 1.0, 4.0, pal::GRID_HI);
        if k % label_every == 0 {
            b.text_xs(px + 2.0, tly + tlh - XS_H - 6.0, &timeutil::fmt_hms(t), pal::GRAY);
        }
        k += 1;
        t += step;
    }
    for m in [5.0f32, 6.0, 7.0] {
        let py = tly + 4.0 + (1.0 - (m - 4.0) / 4.0) * (tlh - 12.0);
        dotted_hline(b, tlx + 1.0, py, tlx + tlw - 1.0, pal::PLOT_DOT);
    }
    for q in events_live.iter() {
        let ts = q.time_s();
        if ts < win_start || ts > win_end {
            continue;
        }
        let px = tlx + tlw * ((ts - win_start) / span) as f32;
        let py = tly + 4.0 + (1.0 - (q.mag - 4.0).clamp(0.0, 4.0) / 4.0) * (tlh - 12.0);
        let sz = 2.0 + (q.mag - 4.0).clamp(0.0, 4.0) * 1.2;
        let col = if ts > ph { pal::dim(pal::CYAN, 0.45) } else { pal::mag_color(q.mag) };
        b.rect(px - sz * 0.5, py - sz * 0.5, sz, sz, col);
        b.rect(px, py, 1.0, tly + tlh - 3.0 - py, pal::PLOT_FAINT);
    }
    // playhead: amber line + slider-style knob (orrery speed slider knob)
    let phx = tlx + tlw * ((ph.clamp(win_start, win_end) - win_start) / span) as f32;
    b.rect(phx - 1.0, tly, 2.0, tlh, pal::AMBER);
    b.rect(phx - 4.0, tly - 2.0, 8.0, 12.0, pal::AMBER);
    b.frame(phx - 4.0, tly - 2.0, 8.0, 12.0, pal::GRID_HI);
    b.text_sm((phx + 8.0).min(tlx + tlw - SM_W * 9.0), tly + 2.0, &timeutil::fmt_hms(ph), pal::AMBER);
    // span tabs + rewind + LIVE
    let mut sx = tlx + tlw + 6.0;
    let spans: [(f32, &str); 5] = [
        (1.0 / 6.0, "10M"),
        (1.0, "1H"),
        (6.0, "6H"),
        (24.0, "24H"),
        (48.0, "48H"),
    ];
    let active = spans
        .iter()
        .position(|(hh, _)| (a.win_mode - hh).abs() < 1e-6)
        .unwrap_or(4);
    let items: Vec<(&str, UiAction)> = spans
        .iter()
        .map(|(hh, lab)| (*lab, UiAction::Span(*hh)))
        .collect();
    sx = tabs(b, &mut rg, sx, tly + 2.0, tab_w2, 24.0, &items, active, hov);
    sx += 4.0;
    let rr = [sx, tly + 2.0, rew_w, 24.0];
    let hh = hov.map_or(false, |p| hit(p, rr));
    b.rect(sx, tly + 2.0, rew_w, 24.0, if hh { pal::CELL_HOV } else { pal::CELL });
    b.frame(sx, tly + 2.0, rew_w, 24.0, if hh { pal::GRID_HI } else { pal::GRID });
    tri_left(b, sx + rew_w * 0.5 - 3.0, tly + 8.0, if hh { pal::GRID_HI } else { pal::GRAY });
    tri_left(b, sx + rew_w * 0.5 + 4.0, tly + 8.0, if hh { pal::GRID_HI } else { pal::GRAY });
    rg.push(UiRegion {
        r: rr,
        a: UiAction::Rewind,
    });
    let lvx = w - 8.0 - live_w;
    let lr = [lvx, tly + 2.0, live_w, 24.0];
    let hh = hov.map_or(false, |p| hit(p, lr));
    if live {
        b.rect(lvx, tly + 2.0, live_w, 24.0, pal::GREEN);
        b.frame(lvx, tly + 2.0, live_w, 24.0, pal::GRID_HI);
        b.text_sm(lvx + (live_w - SM_W * 4.0) * 0.5, tly + 2.0 + (24.0 - SM_H) * 0.5, "LIVE", pal::INK);
    } else {
        b.rect(lvx, tly + 2.0, live_w, 24.0, if hh { pal::CELL_HOV } else { pal::CELL });
        b.frame(lvx, tly + 2.0, live_w, 24.0, if hh { pal::GRID_HI } else { pal::GRID });
        b.text_sm(lvx + (live_w - SM_W * 4.0) * 0.5, tly + 2.0 + (24.0 - SM_H) * 0.5, "LIVE", pal::GREEN);
    }
    rg.push(UiRegion {
        r: lr,
        a: UiAction::Live,
    });

    // ============================== bottom: key row =======================
    let ky = ty + th + 6.0;
    let kh = 24.0;
    let mut x = 8.0;
    x = key(b, &mut rg, x, ky, kh, "AUTO", tg.spin, Some(pal::AMBER), UiAction::AutoSpin, hov);
    x = key(b, &mut rg, x, ky, kh, "PAUSE", tg.pause, Some(pal::ALARM), UiAction::Pause, hov);
    x = key(b, &mut rg, x, ky, kh, "QUAKES", tg.quakes, None, UiAction::TglQuakes, hov);
    x = key(b, &mut rg, x, ky, kh, "STATIONS", tg.stations, None, UiAction::TglStations, hov);
    x = key(b, &mut rg, x, ky, kh, "WAVES", tg.waves, None, UiAction::TglWaves, hov);
    x = key(b, &mut rg, x, ky, kh, "LABELS", tg.labels, None, UiAction::TglLabels, hov);
    x = key(b, &mut rg, x, ky, kh, "GRID", tg.grid, None, UiAction::TglGrid, hov);
    x = key(b, &mut rg, x, ky, kh, "CITIES", tg.cities, None, UiAction::TglCities, hov);
    // transport: PLAY replays the window from its start, STOP holds playhead
    x = key(b, &mut rg, x, ky, kh, "PLAY", a.playing, Some(pal::GREEN), UiAction::Play, hov);
    a.play_btn = [x - SM_W * 5.0 - 12.0, ky, SM_W * 5.0 + 12.0, kh];
    x = key(b, &mut rg, x, ky, kh, "STOP", !a.playing && !a.live, Some(pal::ALARM), UiAction::Stop, hov);
    a.stop_btn = [x - SM_W * 5.0 - 12.0, ky, SM_W * 5.0 + 12.0, kh];
    x = key(b, &mut rg, x, ky, kh, "FIT", false, None, UiAction::ViewFit, hov);
    x = key(b, &mut rg, x, ky, kh, "Z-", false, None, UiAction::ZoomOut, hov);
    x = key(b, &mut rg, x, ky, kh, "Z+", false, None, UiAction::ZoomIn, hov);
    x = key(b, &mut rg, x, ky, kh, "RESET", false, None, UiAction::ResetView, hov);
    x = key(b, &mut rg, x, ky, kh, "INV-X", a.inv_x, None, UiAction::InvX, hov);
    x = key(b, &mut rg, x, ky, kh, "INV-Y", a.inv_y, None, UiAction::InvY, hov);
    x = key(b, &mut rg, x, ky, kh, "FULL", a.fullscreen, None, UiAction::Fullscreen, hov);
    // trace gain slider (orrery slider: CELL track, dim CYAN fill, AMBER knob)
    x += 6.0;
    b.text_sm(x, ky + (kh - SM_H) * 0.5, "GAIN", pal::GRAY);
    x += SM_W * 5.0;
    let sw = (140.0 * s).max(90.0);
    let sr = [x, ky + (kh - 12.0) * 0.5, sw, 12.0];
    a.gain_rect = sr;
    b.rect(sr[0], sr[1], sw, 12.0, pal::CELL);
    b.frame(sr[0], sr[1], sw, 12.0, pal::GRID);
    let f = (gain.ln() - 0.3f32.ln()) / (4.0f32.ln() - 0.3f32.ln());
    let kx = sr[0] + sw * f.clamp(0.0, 1.0);
    b.rect(sr[0], sr[1], kx - sr[0], 12.0, pal::dim(pal::CYAN, 0.4));
    b.rect(kx - 3.0, sr[1] - 3.0, 6.0, 18.0, pal::AMBER);
    b.frame(kx - 3.0, sr[1] - 3.0, 6.0, 18.0, pal::GRID_HI);
    b.text_sm(x + sw + 6.0, ky + (kh - SM_H) * 0.5, &format!("\u{00d7}{gain:.1}"), pal::GRID_HI);
    // playback speed slider (log 1x..240x), orrery slider style
    x += sw + SM_W * 4.0 + 12.0;
    b.text_sm(x, ky + (kh - SM_H) * 0.5, "SPD", pal::GRAY);
    x += SM_W * 4.0;
    let pw = (96.0 * s).max(70.0);
    let pr = [x, ky + (kh - 12.0) * 0.5, pw, 12.0];
    a.speed_rect = pr;
    b.rect(pr[0], pr[1], pw, 12.0, pal::CELL);
    b.frame(pr[0], pr[1], pw, 12.0, pal::GRID);
    let pf = a.play_speed.ln() / 240.0f32.ln();
    let pkx = pr[0] + pw * pf.clamp(0.0, 1.0);
    b.rect(pr[0], pr[1], pkx - pr[0], 12.0, pal::dim(pal::AMBER, 0.4));
    b.rect(pkx - 3.0, pr[1] - 3.0, 6.0, 18.0, pal::AMBER);
    b.frame(pkx - 3.0, pr[1] - 3.0, 6.0, 18.0, pal::GRID_HI);
    b.text_sm(x + pw + 6.0, ky + (kh - SM_H) * 0.5, &format!("\u{00d7}{:.0}", a.play_speed), pal::GRID_HI);
    if a.tips {
        b.text_right_xs(w - 8.0, ky + (kh - XS_H) * 0.5 + 1.0, "DRAG ROTATE \u{00b7} WHEEL ZOOM/SCROLL LOG \u{00b7} ENTER PLAY \u{00b7} C CITIES", pal::GRAY);
    }

    // ============================== bottom: status row ====================
    let sy = ky + kh + 4.0;
    let mut x = 8.0;
    x = status_cell(b, x, sy, 18.0, &format!("EVT 24H {}", stats.count_24h), pal::CYAN);
    x = status_cell(b, x, sy, 18.0, &format!("STA {}", SENSORS.len()), pal::CYAN);
    x = status_cell(b, x, sy, 18.0, &format!("WAVES {waves_n}"), pal::CYAN);
    let _ = x;
    let status = match events_live.first() {
        Some(q) => format!(
            "QUAKE M{:.1} {} / STA {} / ON SCREEN {:03} OF {:04} BODIES",
            q.mag,
            q.place.chars().take(24).collect::<String>(),
            SENSORS.len(),
            a.marker_buf.len(),
            events_live.len() + SENSORS.len()
        ),
        None => format!("NO EVENTS LOGGED / STA {}", SENSORS.len()),
    };
    b.text_right_sm(w - 8.0, sy + 2.0, &status, pal::GRAY);

    // ============================== FIND EVENT dropdown (frontmost) =======
    if a.find_open && !q.is_empty() {
        let mut matches: Vec<(usize, &Quake)> = Vec::new();
        for (i, qq) in events_live.iter().enumerate() {
            if qq.place.to_uppercase().contains(&q) || qq.id.to_uppercase().contains(&q) {
                matches.push((i, qq));
                if matches.len() >= 10 {
                    break;
                }
            }
        }
        if !matches.is_empty() {
            let dw = fw.max(240.0 * s);
            let dh = matches.len() as f32 * 20.0 + 4.0;
            let dx = a.find_rect[0];
            let dy = a.find_rect[1] + a.find_rect[3] + 2.0;
            b.rect(dx, dy, dw, dh, pal::CELL);
            b.frame(dx, dy, dw, dh, pal::GRID);
            for (n, (i, qq)) in matches.iter().enumerate() {
                let r = [dx + 2.0, dy + 2.0 + n as f32 * 20.0, dw - 4.0, 20.0];
                let hh = hov.map_or(false, |p| hit(p, r));
                if hh {
                    b.rect(r[0], r[1], r[2], r[3], pal::CELL_HOV);
                }
                let ty2 = r[1] + (20.0 - SM_H) * 0.5;
                b.text_sm(r[0] + 6.0, ty2, &timeutil::fmt_hms(qq.time_s()), if hh { pal::GRID_HI } else { pal::GRAY });
                b.text_sm(r[0] + SM_W * 9.0, ty2, &format!("M{:.1}", qq.mag), pal::mag_color(qq.mag));
                let pl: String = qq.place.chars().take(((dw - SM_W * 15.0) / SM_W) as usize).collect();
                b.text_sm(r[0] + SM_W * 15.0, ty2, &pl, if hh { pal::GRID_HI } else { pal::GRID });
                rg.push(UiRegion {
                    r,
                    a: UiAction::ListEvent(*i),
                });
            }
        }
    }

    a.regions = rg;
}

/// Floating magnitude tags + station codes over the globe.
pub fn build_labels(a: &mut App, vp: Matrix4<f32>, size: (u32, u32)) {
    let mut jobs: Vec<(f32, f32, String, Rgba, bool)> = Vec::new();
    if a.toggles.quakes {
        for q in a.store.events().iter().take(8) {
            if q.time_s() > a.playhead + 0.5 {
                continue;
            }
            let p = q.unit_pos();
            let p = Vector3::new(p[0], p[1], p[2]);
            if let Some((x, y, facing)) = a.cam.project(p, vp, size) {
                if facing > 0.15 {
                    jobs.push((x, y, format!("{:.1}", q.mag), pal::mag_color(q.mag), false));
                }
            }
        }
    }
    if a.toggles.cities && a.cam.dist < 2.7 {
        for (lat, lon, name) in crate::assets::capitals() {
            let p = crate::model::geo_to_cartesian(lat, lon, 1.0);
            let p = Vector3::new(p[0], p[1], p[2]);
            if let Some((x, y, facing)) = a.cam.project(p, vp, size) {
                if facing > 0.30 {
                    jobs.push((x, y, name.to_string(), pal::dim(pal::AMBER, 0.85), true));
                }
            }
        }
    }
    if a.toggles.stations && a.cam.dist < 2.9 {
        for (i, st) in SENSORS.iter().enumerate() {
            let p = crate::model::geo_to_cartesian(st.lat, st.lon, 1.0);
            let p = Vector3::new(p[0], p[1], p[2]);
            if let Some((x, y, facing)) = a.cam.project(p, vp, size) {
                if facing > 0.25 {
                    let col = if Some(i) == a.selected_station { pal::AMBER } else { pal::GRID };
                    jobs.push((x, y, st.code.to_string(), col, true));
                }
            }
        }
    }
    let b = &mut a.batch;
    for (x, y, txt, col, small) in jobs {
        b.rect(x + 6.0, y - 1.0, 8.0, 1.0, col);
        if small {
            b.text_xs(x + 16.0, y - XS_H * 0.5, &txt, col);
        } else {
            b.text_sm(x + 16.0, y - SM_H * 0.5, &txt, col);
        }
    }
}
