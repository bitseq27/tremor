# TREMOR — Real-Time 3D Seismic Monitor

Native Windows desktop dashboard rendered with **Rust + wgpu** (Direct3D 12 /
Vulkan), styled after the **ORRERY HMI design language**
(github.com/bitseq27/orrery): flat hard-edged monochrome cells on pure black
(#0A0A0A fills, 1px #6E6E6E rules, white highlights) with cyan #21D0C5 /
amber #E0A800 / alarm #D23B2E accents, Source Code Pro (SIL OFL) at 16/13/10 px.
No gradients, no rounding, no shadows.

Live seismicity is pulled from the EMSC-CSEM realtime socket (the same source
that drives www.emsc-csem.org): the last 48 h are back-filled at connect and
new solutions push within seconds; the USGS feed is an automatic fallback.
Seismicity is processed on an isolated tokio
worker thread; events >= M4.0 become glowing epicentre billboards and GPU-side
expanding wave fronts on a textured 3D globe.


## Running on Windows

`ship/tremor-windows-x86_64.zip` contains a self-contained
`tremor.exe` (x86_64, GNU ABI, no sidecar DLLs — imports verified to be
Windows-system DLLs only). Requirements: Windows 10/11 with any
D3D12- or Vulkan-capable GPU (the app prefers DX12, falls back to Vulkan).

```
tremor.exe                 # live EMSC-CSEM feed (USGS fallback)
tremor.exe --sim           # offline demonstration feed (no network needed)
tremor.exe --frames 240 --shot out.ppm   # headless snapshot mode (CI / validation)
```

Rendering is capped at 60 fps (`ControlFlow::WaitUntil` pacing + present
gate), so fast GPUs stay silent and the HUD FPS readout settles at ~60.

### Controls

| input            | action                                    |
|------------------|-------------------------------------------|
| left drag        | rotate globe (inertial; idle globe holds still) |
| mouse wheel      | zoom (clamped 1.45-9.0 radii)             |
| click / pen tap  | select event or station, or hit HUD keys (stylus & touch supported) |
| FIND EVENT field | type-as-you-search event list; ENTER picks best match, ESC closes |
| `[-]`/`[+]` chip | collapse / expand any panel (orrery style) |
| GAIN slider      | drag for trace gain 0.3x-4.0x (log) |
| timeline         | drag the playhead; span tabs 10M...ALL; LIVE returns to now |
| touch drag / pinch | rotate / zoom on tablets & pen displays |
| `[-]`/`[+]` box  | collapse / expand any panel (orrery style) |
| `G` / `W` / `L`  | toggle graticule / wave fronts / labels   |
| `S` / `F`        | toggle auto-spin / local simulator (SIMF chip also clickable) |
| `Space` / `P`    | pause feed ingestion                      |
| `Esc`            | close FIND list, else quit                |

## Building

### Native Windows (recommended for development)

Install the Rust toolchain (MSVC or GNU), then double-click **`launch.bat`**
(builds the release exe if stale and runs it; `launch.bat --sim` for the
offline demo feed, `launch.bat rebuild` to force a rebuild) or simply:

```
cargo run --release
```

### Cross-compile from Linux (how the shipped exe was produced)

```
rustup target add x86_64-pc-windows-gnu
sudo apt install mingw-w64            # provides x86_64-w64-mingw32-gcc
cargo build --release --target x86_64-pc-windows-gnu
```

`.cargo/config.toml` wires the MinGW linker for that target.
Low-RAM machines: build with `cargo build -j 1` (the `windows` crate at
opt-level 3 peaks above 1 GB; parallel jobs can OOM).

### Headless validation (Linux CI)

```
xvfb-run -a -s "-screen 0 1600x900x24" \
  env TREMOR_SIM=1 cargo run -- --frames 400 --shot shot.ppm
```

Uses lavapipe (software Vulkan); `--frames/--shot` renders N frames and reads
the swapchain back into a PPM without needing a display server interaction.

## Architecture (mirrors `tremor plan.md`)

| domain              | module            | contents |
|---------------------|-------------------|----------|
| network             | `src/net.rs`      | tokio worker pool, reqwest + serde GeoJSON parsing, M4.0 filter, id dedup, SIM fallback, `mpsc::sync_channel` |
| system processing   | `src/model.rs`    | unique-id collection, 512-event ring, geo→cartesian, wave TTL lifecycle & prune |
| render              | `src/gpu/*`, `src/app.rs`, `src/hud.rs` | wgpu device/swapchain (DX12-first on Windows), globe/marker/UI pipelines, WGSL shaders, HUD batcher |
| geometry            | `src/mesh.rs`, `src/camera.rs` | indexed UV sphere, bytemuck Pod layouts, orbit camera, projection/picking |
| embedded assets     | `src/assets.rs`, `assets/` | anti-aliased Source Code Pro font atlas, 3600×1800 (10 px/deg) Natural-Earth 50m land mask, 4096×2048 equirectangular planet texture built at startup |
| sensor overlay      | `src/sensors.rs` | 38 global broadband stations (GSN-style) drawn as crosshair markers, codes labelled when zoomed in |
| fault overlay       | `assets/faultlines.i16` | 1443 PB2002 plate-boundary segments (Bird 2003, via OpenLyceum generated data), kind-coloured (convergent/divergent/transform) and stamped into the planet texture at startup |
| epicentre marks     | `src/shaders/globe.wgsl` | surface-attached X + pip per event (storage buffer, 128 slots), pulsing ring on the selected one; camera auto-tracks the latest epicentre for 20 s |

Shaders (`src/shaders/`):

* `globe.wgsl` — equirectangular land texture, procedural 15° graticule,
  per-pixel evaluation of the active quake storage buffer: gaussian ring
  profile `exp(-d²/w²)` at radius `speed·t` with quadratic dissipative fade
  `(1-t/ttl)²`, magnitude-coloured, plus fresnel rim + fixed-sun lighting.
* `markers.wgsl` — instanced billboards, radial core + halo, pulsing select ring.
* `ui.wgsl` — pixel-space HUD quads + bitmap-font sampling.

Operational guardrails (plan §4): sub-4.0 events are dropped in the network
worker; events cross into the GPU queue at most once per id (worker *and*
processor dedup); every network failure is contained in the worker thread —
the render loop never blocks on I/O (after 45 s offline a clearly labelled
SIM feed keeps the display alive, `TREMOR_SIM=1` forces it).

## Tests

`cargo test` — 31 unit tests: feed parsing/filtering/garbage tolerance,
dedup, wave cleanup, geo round-trips, camera projection & occlusion, mesh
topology, asset integrity, UTC civil time.

## Regenerating style assets

`tools/gen_assets.py` rebuilds the Source Code Pro faces (SIL OFL) and
`assets/landmask_720x360.bin` (Natural Earth 110m, public domain) plus PNG
previews.
