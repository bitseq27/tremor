//! Software-rendered HUD preview (no GPU required): composes the orrery-
//! styled HUD batch for a synthetic session and rasterises it on the CPU to
//! `shots/hud_preview.ppm`, so the interface can be eyeballed in CI-less
//! sandboxes. Colours are sRGB-encoded on output exactly like the sRGB
//! swapchain would, so the PNG shows the true orrery palette bytes.
//!
//! usage: cargo run --example hud_preview

use tremor::app::App;
use tremor::assets;
use tremor::gpu::ui::UiVertex;
use tremor::net;
use tremor::timeutil;

fn srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let v = if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (v * 255.0).round() as u8
}

fn edge(a: &UiVertex, b: &UiVertex, x: f32, y: f32) -> f32 {
    (b.pos[0] - a.pos[0]) * (y - a.pos[1]) - (b.pos[1] - a.pos[1]) * (x - a.pos[0])
}

fn main() {
    let (w, h) = (1600usize, 900usize);
    let (_tx, rx) = std::sync::mpsc::sync_channel(4);
    let mut a = App::new(rx, None);
    a.ui_scale = 1.0;
    let now = timeutil::now_secs();
    a.now = now;
    a.playhead = now;
    a.t0 = now - 48.0 * 3600.0;
    a.win_mode = 48.0;
    // a realistic 48 h EMSC-shaped session: ~150 events, newest first
    let mut rng = net::Rng::new(7);
    for i in 0..150u32 {
        let mut q = net::sim_event(&mut rng, i);
        q.time_ms = ((now - i as f64 * 1100.0) * 1000.0) as i64;
        q.simulated = i % 9 == 4; // a few SIM-tinted events among live ones
        q.id = format!("prev-{i:03}");
        a.store.ingest(q, now);
    }
    a.link = net::LinkState::Sim;
    a.sim_local = true;
    a.fps = 59.94;
    a.tips = true;
    a.drag_last = Some((420.0, 320.0)); // hover point -> hover states visible
    let variant = std::env::args().any(|x| x == "--alt");
    if variant {
        // second state: selection, collapsed panels, find dropdown, review
        let id = a.store.events().front().map(|q| q.id.clone());
        a.selected_id = id;
        a.selected_station = None;
        a.panels_collapsed = (1 << 2) | (1 << 6); // LOGGED EVENTS + LEGEND collapsed
        a.find_text = "JAPAN".into();
        a.find_focused = true;
        a.find_open = true;
        a.live = false;
        a.drag_last = Some((300.0, 40.0)); // hover over the find field
    }
    tremor::hud::build_hud(&mut a, w as u32, h as u32);
    eprintln!("hud quads: {} (cap 65536)", a.batch.verts.len() / 6);

    let (atlas, aw, ah) = assets::build_font_atlas();
    let mut fb = vec![0.0f32; w * h * 3];
    for ch in a.batch.verts.chunks(6) {
        let tris: [[&UiVertex; 3]; 2] = [[&ch[0], &ch[1], &ch[2]], [&ch[1], &ch[4], &ch[2]]];
        for t in tris.iter() {
            let (v0, v1, v2) = (t[0], t[1], t[2]);
            let xs = [v0.pos[0], v1.pos[0], v2.pos[0]];
            let ys = [v0.pos[1], v1.pos[1], v2.pos[1]];
            let minx = xs.iter().cloned().fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
            let maxx = (xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max).ceil().min(w as f32) as usize).min(w);
            let miny = ys.iter().cloned().fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
            let maxy = (ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max).ceil().min(h as f32) as usize).min(h);
            let d = edge(v0, v1, v2.pos[0], v2.pos[1]);
            if d.abs() < 1e-6 {
                continue;
            }
            for py in miny..maxy {
                for px in minx..maxx {
                    let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
                    let w0 = edge(v1, v2, x, y) / d;
                    let w1 = edge(v2, v0, x, y) / d;
                    let w2 = edge(v0, v1, x, y) / d;
                    if w0 < -1e-3 || w1 < -1e-3 || w2 < -1e-3 {
                        continue;
                    }
                    let col = v0.color;
                    let alpha = if v0.mode > 0.5 {
                        let u = w0 * v0.uv[0] + w1 * v1.uv[0] + w2 * v2.uv[0];
                        let vv = w0 * v0.uv[1] + w1 * v1.uv[1] + w2 * v2.uv[1];
                        let tx = (u * aw as f32) as usize % aw as usize;
                        let ty = (vv * ah as f32) as usize % ah as usize;
                        atlas[ty * aw as usize + tx] as f32 / 255.0 * col[3]
                    } else {
                        col[3]
                    };
                    if alpha < 0.01 {
                        continue;
                    }
                    let o = (py * w + px) * 3;
                    fb[o] = col[0] * alpha + fb[o] * (1.0 - alpha);
                    fb[o + 1] = col[1] * alpha + fb[o + 1] * (1.0 - alpha);
                    fb[o + 2] = col[2] * alpha + fb[o + 2] * (1.0 - alpha);
                }
            }
        }
    }

    let path = if variant { "shots/hud_preview_alt.ppm".to_string() } else { "shots/hud_preview.ppm".to_string() };
    let mut file = format!("P6\n{w} {h}\n255\n").into_bytes();
    for px in fb.chunks(3) {
        file.extend_from_slice(&[srgb(px[0]), srgb(px[1]), srgb(px[2])]);
    }
    std::fs::write(&path, file).expect("write ppm");
    eprintln!("hud preview written to {path}");
}
