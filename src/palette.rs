//! ORRERY HMI palette - the exact design-language tokens of the reference
//! orrery dashboard (github.com/bitseq27/orrery, `crates/app/src/theme.rs`):
//! flat hard-edged monochrome + three accents, no gradients, no rounding.
//!
//! Values are stored sRGB-decoded (linear) because TREMOR presents to an
//! sRGB swapchain: writing these floats makes the display show precisely the
//! orrery `Color32` bytes below (egui is sRGB-correct in the same way).

pub type Rgba = [f32; 4];

pub const BG: Rgba = [0.0, 0.0, 0.0, 1.0]; // #000000
pub const CELL: Rgba = [0.003035, 0.003035, 0.003035, 1.0]; // #0A0A0A
pub const CELL_DIM: Rgba = [0.01033, 0.01033, 0.01033, 1.0]; // #1A1A1A
pub const CELL_HOV: Rgba = [0.008023, 0.008023, 0.008023, 1.0]; // #161616
pub const GRID: Rgba = [0.1559, 0.1559, 0.1559, 1.0]; // #6E6E6E
pub const GRID_HI: Rgba = [1.0, 1.0, 1.0, 1.0]; // #FFFFFF
pub const CYAN: Rgba = [0.01521, 0.6308, 0.5583, 1.0]; // #21D0C5
pub const GREEN: Rgba = [0.05613, 0.3515, 0.06301, 1.0]; // #43A047
pub const AMBER: Rgba = [0.7454, 0.3916, 0.0, 1.0]; // #E0A800
pub const ALARM: Rgba = [0.6445, 0.04374, 0.02732, 1.0]; // #D23B2E
pub const GRAY: Rgba = [0.09531, 0.09531, 0.06848, 1.0]; // #57574A
pub const INK: Rgba = [0.0, 0.0, 0.0, 1.0]; // #000000
pub const ROW_LINE: Rgba = [0.01161, 0.01161, 0.01161, 1.0]; // #1C1C1C
pub const PLOT_DOT: Rgba = [0.02732, 0.02732, 0.02732, 1.0]; // #2E2E2E
pub const PLOT_FAINT: Rgba = [0.01938, 0.01938, 0.01938, 1.0]; // #262626

/// egui-style `linear_multiply`: scale a linear colour towards black.
pub fn dim(c: Rgba, f: f32) -> Rgba {
    [c[0] * f, c[1] * f, c[2] * f, c[3]]
}

/// Override alpha (markers, overlays).
pub fn with_a(mut c: Rgba, a: f32) -> Rgba {
    c[3] = a;
    c
}

/// Magnitude -> HMI accent (green calm .. white-hot great). Uses only the
/// orrery accent set: GREEN / AMBER / ALARM plus monochrome white.
pub fn mag_color(m: f32) -> Rgba {
    match m {
        m if m < 5.0 => GREEN,
        m if m < 6.0 => AMBER,
        m if m < 7.0 => ALARM,
        _ => GRID_HI,
    }
}

pub fn mag_tag(m: f32) -> &'static str {
    match m {
        m if m < 5.0 => "MINOR",
        m if m < 6.0 => "MODERATE",
        m if m < 7.0 => "STRONG",
        m if m < 8.0 => "MAJOR",
        _ => "GREAT",
    }
}
