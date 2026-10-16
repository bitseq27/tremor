//! TREMOR - Real-Time 3D Seismic Monitor
//!
//! Crate layout follows `tremor plan.md`:
//!   * network domain   -> [`net`]      (tokio worker thread, reqwest + serde)
//!   * processing domain-> [`model`]    (dedup, geo->cartesian, wave lifecycle)
//!   * render domain    -> [`gpu`], [`app`] (wgpu D3D12/Vulkan, winit event loop)
//!
//! Geometry / mesh math lives in [`mesh`], embedded binary assets (multi-size
//! VT323 bitmap fonts, land mask) in [`assets`], and the orrery-HMI palette
//! tokens in [`palette`].

pub mod analysis;
pub mod app;
pub mod assets;
pub mod camera;
pub mod gpu;
pub mod hud;
pub mod mesh;
pub mod model;
pub mod net;
pub mod palette;
pub mod sensors;
pub mod timeutil;
pub mod ws;

pub const APP_NAME: &str = "TREMOR";
pub const APP_VERSION: &str = "0.1.0";
/// Network-level guardrail: events below this magnitude are dropped at the
/// worker thread, before they ever reach the GPU queue.
pub const MAG_FILTER: f32 = 4.0;
