//! Render domain glue: winit application handler, input -> camera, channel
//! pumping, frame orchestration, headless snapshot capture.
//! HUD composition lives in `hud.rs`.

use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Instant;

use cgmath::Vector3;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::camera::OrbitCamera;
use crate::gpu::bg::{BgPipe, BgUniforms};
use crate::gpu::globe::{GlobePipe, GlobeUniforms, WaveGpu, MAX_WAVES};
use crate::gpu::markers::{MarkerGpu, MarkerPipe, MarkerUniforms};
use crate::gpu::ui::UiPipe;
use crate::gpu::Gpu;
use crate::model::EventStore;
use crate::net::{LinkState, NetEvent};
use crate::palette as pal;
use crate::timeutil;

#[derive(Debug, Clone, Copy)]
pub struct Toggles {
    pub grid: bool,
    pub waves: bool,
    pub labels: bool,
    pub pause: bool,
    pub spin: bool,
    pub quakes: bool,
    pub stations: bool,
    pub cities: bool,
}

pub struct ShotCfg {
    pub frames: u64,
    pub path: String,
    pub done: bool,
}

pub struct App {
    pub window: Option<Arc<Window>>,
    pub gpu: Option<Gpu>,
    pub globe: Option<GlobePipe>,
    pub bg: Option<BgPipe>,
    pub markers: Option<MarkerPipe>,
    pub ui: Option<UiPipe>,
    pub rx: Receiver<NetEvent>,
    pub store: EventStore,
    pub link: LinkState,
    pub cam: OrbitCamera,
    pub toggles: Toggles,
    pub batch: crate::gpu::ui::UiBatch,
    pub marker_buf: Vec<MarkerGpu>,
    pub wave_buf: Vec<WaveGpu>,
    pub fps: f64,
    pub last: Instant,
    pub now: f64,
    pub drag_last: Option<(f64, f64)>,
    pub selected_id: Option<String>,
    pub frames: u64,
    pub shot: Option<ShotCfg>,
    pub batches: u64,
    pub last_batch_at: f64,
    pub ui_scale: f32,
    pub gpu_desc: String,
    pub t0: f64,
    pub sim_local: bool,
    pub next_sim_at: f64,
    pub sim_seq: u32,
    pub rng: crate::net::Rng,
    pub sim_btn: [f32; 4],
    pub drag_moved: f32,
    pub focus_until: f64,
    pub tracked_mag: f32,
    pub touches: Vec<(u64, f64, f64)>,
    pub pinch_dist: f32,
    pub epic_buf: Vec<crate::gpu::globe::EpicGpu>,
    pub live: bool,
    pub playing: bool,
    pub play_speed: f32,
    pub flash: Vec<(f64, [f32; 3], f32)>,
    pub log_scroll: f32,
    pub log_rect: [f32; 4],
    pub speed_rect: [f32; 4],
    pub speed_drag: bool,
    pub play_btn: [f32; 4],
    pub stop_btn: [f32; 4],
    pub mouse_pos: (f32, f32),
    pub playhead: f64,
    pub win_mode: f32,
    pub timeline_rect: [f32; 4],
    pub scrubbing: bool,
    pub regions: Vec<crate::hud::UiRegion>,
    pub auto_track: bool,
    pub selected_station: Option<usize>,
    pub inv_x: bool,
    pub inv_y: bool,
    pub fullscreen: bool,
    pub trace_gain: f32,
    /// orrery TIPS toggle: gates the plain-language hint strings
    pub tips: bool,
    /// FIND EVENT field (orrery FIND BODY): populate-as-you-type search
    pub find_text: String,
    pub find_focused: bool,
    pub find_open: bool,
    pub find_rect: [f32; 4],
    /// trace-gain slider geometry (orrery-style draggable slider)
    pub gain_rect: [f32; 4],
    pub gain_drag: bool,
    /// human-readable name of the live feed source (EMSC / USGS fallback)
    pub feed_src: &'static str,
    demo_applied: bool,
    pub present_at: std::time::Instant,
    pub panels_collapsed: u32,
}

impl App {
    pub fn new(rx: Receiver<NetEvent>, shot: Option<ShotCfg>) -> Self {
        Self {
            window: None,
            gpu: None,
            globe: None,
            bg: None,
            markers: None,
            ui: None,
            rx,
            store: EventStore::new(),
            link: LinkState::Off,
            cam: OrbitCamera::default(),
            toggles: Toggles {
                grid: true,
                waves: true,
                labels: true,
                pause: false,
                spin: false,
                quakes: true,
                stations: true,
                cities: true,
            },
            batch: Default::default(),
            marker_buf: Vec::new(),
            wave_buf: Vec::new(),
            fps: 60.0,
            last: Instant::now(),
            now: timeutil::now_secs(),
            drag_last: None,
            selected_id: None,
            frames: 0,
            shot,
            batches: 0,
            last_batch_at: 0.0,
            ui_scale: 1.0,
            gpu_desc: String::from("NONE"),
            t0: timeutil::now_secs(),
            sim_local: std::env::var("TREMOR_SIMF").is_ok(),
            next_sim_at: 0.0,
            sim_seq: 0,
            rng: crate::net::Rng::new(0xC0FF_EE15),
            sim_btn: [0.0; 4],
            drag_moved: 0.0,
            focus_until: 0.0,
            tracked_mag: 0.0,
            touches: Vec::new(),
            pinch_dist: 0.0,
            epic_buf: Vec::new(),
            live: true,
            playing: false,
            play_speed: 8.0,
            flash: Vec::new(),
            log_scroll: 0.0,
            log_rect: [0.0; 4],
            speed_rect: [0.0; 4],
            speed_drag: false,
            play_btn: [0.0; 4],
            stop_btn: [0.0; 4],
            mouse_pos: (0.0, 0.0),
            playhead: timeutil::now_secs(),
            win_mode: 48.0,
            timeline_rect: [0.0; 4],
            scrubbing: false,
            regions: Vec::new(),
            auto_track: true,
            selected_station: None,
            inv_x: false,
            inv_y: false,
            fullscreen: false,
            trace_gain: 1.0,
            tips: false,
            find_text: String::new(),
            find_focused: false,
            find_open: false,
            find_rect: [0.0; 4],
            gain_rect: [0.0; 4],
            gain_drag: false,
            feed_src: crate::net::SOURCE_EMSC,
            demo_applied: false,
            present_at: std::time::Instant::now(),
            panels_collapsed: 0,
        }
    }

    pub fn pump_net(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                NetEvent::Batch(qs) => {
                    self.batches += 1;
                    self.last_batch_at = self.now;
                    if !self.toggles.pause {
                        for q in qs {
                            if self.store.ingest(q.clone(), self.now) {
                                if self.live {
                                    self.flash.push((self.now, q.unit_pos(), q.mag));
                                }
                                if self.auto_track {
                                    self.track_event(&q);
                                }
                            }
                        }
                    }
                }
                NetEvent::Link(s) => self.link = s,
                NetEvent::Source(s) => self.feed_src = s,
            }
        }
    }

    pub fn selected_index(&self) -> Option<usize> {
        let id = self.selected_id.as_ref()?;
        self.store.events().iter().position(|q| q.id == *id)
    }

    pub fn vp_size(&self) -> (u32, u32) {
        self.gpu
            .as_ref()
            .map(|g| (g.config.width, g.config.height))
            .unwrap_or((1600, 900))
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = WindowAttributes::default()
            .with_title("TREMOR - REAL-TIME 3D SEISMIC MONITOR")
            .with_inner_size(winit::dpi::LogicalSize::new(1600u32, 900u32));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("tremor: window: {e}");
                event_loop.exit();
                return;
            }
        };
        self.ui_scale = window.scale_factor().round().clamp(1.0, 2.0) as f32;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("init runtime");
        match rt.block_on(Gpu::init(window.clone())) {
            Ok(gpu) => {
                self.gpu_desc = format!("{:?}/{}", gpu.backend, gpu.adapter_name);
                eprintln!("tremor: gpu {} ", self.gpu_desc);
                let globe = GlobePipe::new(&gpu);
                let bg = BgPipe::new(&gpu);
                let markers = MarkerPipe::new(&gpu);
                let ui = UiPipe::new(&gpu);
                self.globe = Some(globe);
                self.bg = Some(bg);
                self.markers = Some(markers);
                self.ui = Some(ui);
                self.gpu = Some(gpu);
            }
            Err(e) => {
                eprintln!("tremor: gpu init failed: {e}");
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let WindowEvent::Resized(sz) = event {
            if let Some(g) = self.gpu.as_mut() {
                g.resize(sz.width, sz.height);
            }
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(w) = self.window.as_ref() {
                    self.ui_scale = w.scale_factor().round().clamp(1.0, 2.0) as f32;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_pos = (position.x as f32, position.y as f32);
                if self.speed_drag {
                    self.speed_to(position.x as f32);
                    self.drag_last = Some((position.x, position.y));
                    return;
                }
                if self.scrubbing {
                    self.scrub_to(position.x as f32);
                    self.drag_last = Some((position.x, position.y));
                    return;
                }
                if self.gain_drag {
                    self.gain_to(position.x as f32);
                    self.drag_last = Some((position.x, position.y));
                    return;
                }
                // Orbit ONLY while a button is actually held. Hovering the
                // pointer must never rotate the globe - injecting drag
                // velocity on plain mouse motion is what made the camera
                // spin constantly and moved markers out from under clicks.
                if self.cam.dragging {
                    if let Some((lx, ly)) = self.drag_last {
                        let dx = (position.x - lx) as f32;
                        let dy = (position.y - ly) as f32;
                        self.drag_moved += dx.abs() + dy.abs();
                        // a real drag takes over from auto-spin
                        if self.drag_moved > 4.0 && self.cam.auto_spin {
                            self.cam.auto_spin = false;
                            self.toggles.spin = false;
                        }
                        let mx = if self.inv_x { -dx } else { dx };
                        let my = if self.inv_y { -dy } else { dy };
                        self.cam.drag(mx, my);
                    }
                }
                self.drag_last = Some((position.x, position.y));
            }
            WindowEvent::CursorLeft { .. } => {
                self.drag_last = None;
                self.cam.dragging = false;
                self.gain_drag = false;
                self.scrubbing = false;
            }
            WindowEvent::MouseInput { state, button, .. } => match (button, state) {
                (MouseButton::Left, ElementState::Pressed) => {
                    self.drag_moved = 0.0;
                    if let Some((x, y)) = self.drag_last {
                        if self.in_timeline(x as f32, y as f32) {
                            self.scrubbing = true;
                            self.scrub_to(x as f32);
                            return;
                        }
                        if self.in_speed(x as f32, y as f32) {
                            self.speed_drag = true;
                            self.speed_to(x as f32);
                            self.drag_last = Some((x, y));
                            return;
                        }
                        if self.in_gain(x as f32, y as f32) {
                            self.gain_drag = true;
                            self.gain_to(x as f32);
                            return;
                        }
                    }
                    self.cam.dragging = true;
                }
                (MouseButton::Left, ElementState::Released) => {
                    let was_scrub = self.scrubbing;
                    let was_gain = self.gain_drag;
                    let was_speed = self.speed_drag;
                    self.scrubbing = false;
                    self.gain_drag = false;
                    self.speed_drag = false;
                    self.cam.dragging = false;
                    if was_scrub || was_gain || was_speed {
                        return;
                    }
                    if let Some((x, y)) = self.drag_last {
                        if self.drag_moved < 5.0 {
                            self.click(x as f32, y as f32);
                        }
                    }
                }
                _ => {}
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let d = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 16.0,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                let (mx, my) = self.mouse_pos;
                let r = self.log_rect;
                if mx >= r[0] && mx <= r[0] + r[2] && my >= r[1] && my <= r[1] + r[3] {
                    self.log_scroll = (self.log_scroll - d * 0.06).max(0.0);
                    return;
                }
                self.cam.zoom(d);
            }
            WindowEvent::KeyboardInput { event: ke, .. } => {
                if ke.state != ElementState::Pressed {
                    return;
                }
                // FIND EVENT field owns the keyboard while focused
                if self.find_focused {
                    match &ke.logical_key {
                        Key::Named(NamedKey::Escape) => {
                            self.find_focused = false;
                            self.find_open = false;
                        }
                        Key::Named(NamedKey::Enter) => self.find_pick_first(),
                        Key::Named(NamedKey::Backspace) => {
                            self.find_text.pop();
                        }
                        Key::Character(c) => {
                            for ch in c.chars() {
                                if !ch.is_control() && self.find_text.chars().count() < 40 {
                                    self.find_text.push(ch);
                                }
                            }
                            self.find_open = true;
                        }
                        _ => {}
                    }
                    return;
                }
                match &ke.logical_key {
                    Key::Named(NamedKey::Escape) => {
                        if self.find_open {
                            self.find_open = false;
                        } else {
                            event_loop.exit();
                        }
                    }
                    Key::Character(c) => match c.as_str() {
                        "g" | "G" => self.toggles.grid = !self.toggles.grid,
                        "w" | "W" => self.toggles.waves = !self.toggles.waves,
                        "l" | "L" => self.toggles.labels = !self.toggles.labels,
                        "s" | "S" => {
                            self.toggles.spin = !self.toggles.spin;
                            self.cam.auto_spin = self.toggles.spin;
                        }
                        "p" | "P" | " " => self.toggles.pause = !self.toggles.pause,
                        "f" | "F" => self.toggle_simf(),
                        "c" | "C" => self.toggles.cities = !self.toggles.cities,
                        _ => {}
                    },
                    Key::Named(NamedKey::Space) => self.toggles.pause = !self.toggles.pause,
                    Key::Named(NamedKey::Enter) => self.toggle_play(),
                    Key::Named(NamedKey::Backspace) => {
                        self.playing = false;
                    }
                    _ => {}
                }
            }
            WindowEvent::Touch(t) => self.touch(t),
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
        event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
            self.present_at + Self::FRAME_DUR,
        ));
    }
}

impl App {
    /// Unified touch/pen handler: one finger/stylus = drag + tap,
    /// two fingers = pinch zoom. Stylus tip taps arrive here on Windows
    /// (WM_POINTER/WM_TOUCH), which plain mouse events never see.
    fn touch(&mut self, t: winit::event::Touch) {
        use winit::event::TouchPhase;
        let (x, y) = (t.location.x, t.location.y);
        match t.phase {
            TouchPhase::Started => {
                self.touches.retain(|e| e.0 != t.id);
                self.touches.push((t.id, x, y));
                if self.touches.len() == 1 && self.in_timeline(x as f32, y as f32) {
                    self.scrubbing = true;
                    self.scrub_to(x as f32);
                    return;
                }
                if self.touches.len() == 1 {
                    self.cam.dragging = true;
                    self.cam.auto_spin = false;
                    self.drag_last = Some((x, y));
                    self.drag_moved = 0.0;
                } else if self.touches.len() == 2 {
                    self.pinch_dist = self.touch_span();
                }
            }
            TouchPhase::Moved => {
                if let Some(e) = self.touches.iter_mut().find(|e| e.0 == t.id) {
                    e.1 = x;
                    e.2 = y;
                }
                if self.scrubbing {
                    self.scrub_to(x as f32);
                    return;
                }
                match self.touches.len() {
                    1 => {
                        if let Some((lx, ly)) = self.drag_last {
                            let dx = (x - lx) as f32;
                            let dy = (y - ly) as f32;
                            self.drag_moved += dx.abs() + dy.abs();
                            // a real drag takes over from auto-spin
                            if self.drag_moved > 4.0 && self.cam.auto_spin {
                                self.cam.auto_spin = false;
                                self.toggles.spin = false;
                            }
                            self.cam.drag(dx, dy);
                        }
                        self.drag_last = Some((x, y));
                    }
                    n if n >= 2 => {
                        let d = self.touch_span();
                        if self.pinch_dist > 1.0 {
                            self.cam.zoom((d - self.pinch_dist) * 0.9);
                        }
                        self.pinch_dist = d;
                        self.drag_moved = 99.0; // a pinch is never a tap
                    }
                    _ => {}
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let was_single = self.touches.len() == 1;
                let was_scrub = self.scrubbing;
                self.scrubbing = false;
                self.touches.retain(|e| e.0 != t.id);
                if was_scrub {
                    return;
                }
                if was_single && t.phase == TouchPhase::Ended && self.drag_moved < 5.0 {
                    self.click(x as f32, y as f32);
                }
                if self.touches.is_empty() {
                    self.cam.dragging = false;
                    self.drag_last = None;
                } else {
                    self.drag_last = self.touches.first().map(|e| (e.1, e.2));
                    self.pinch_dist = self.touch_span();
                }
            }
        }
    }

    fn touch_span(&self) -> f32 {
        if self.touches.len() < 2 {
            return 0.0;
        }
        let a = self.touches[0];
        let b = self.touches[1];
        (((a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)) as f32).sqrt()
    }

    /// Slew the camera onto a fresh epicentre and hold it there.
    pub fn track_event(&mut self, q: &crate::model::Quake) {
        let p = q.unit_pos();
        let yaw = p[0].atan2(p[2]);
        let pitch = p[1].clamp(-1.0, 1.0).asin();
        self.cam.focus = Some((yaw, pitch));
        self.focus_until = self.now + 20.0;
        self.tracked_mag = q.mag;
    }

    pub fn toggle_simf(&mut self) {
        self.sim_local = !self.sim_local;
        if self.sim_local && self.next_sim_at <= self.now {
            self.next_sim_at = self.now + 1.0;
        }
    }

    pub fn win_bounds(&self) -> (f64, f64) {
        let end = self.now;
        // spans are absolute rolling windows over the 48 h event memory
        let start = end - self.win_mode.max(1.0 / 6.0) as f64 * 3600.0;
        (start, end)
    }

    pub fn in_timeline(&self, x: f32, y: f32) -> bool {
        let [tx, ty, tw, th] = self.timeline_rect;
        tw > 0.0 && x >= tx && x <= tx + tw && y >= ty && y <= ty + th
    }

    pub fn scrub_to(&mut self, x: f32) {
        let [tx, _ty, tw, _th] = self.timeline_rect;
        if tw <= 0.0 {
            return;
        }
        let (ws, we) = self.win_bounds();
        let frac = ((x - tx) / tw).clamp(0.0, 1.0) as f64;
        self.playhead = ws + frac * (we - ws);
        self.live = false;
    }

    fn click(&mut self, x: f32, y: f32) {
        for r in self.regions.iter().rev() {
            if x >= r.r[0] && x <= r.r[0] + r.r[2] && y >= r.r[1] && y <= r.r[1] + r.r[3] {
                use crate::hud::UiAction as A;
                match r.a {
                    A::FindFocus => {
                        self.find_focused = true;
                        self.find_open = true;
                    }
                    A::ListEvent(_) => {
                        // picking from the list closes the FIND dropdown
                        self.find_focused = false;
                        self.find_open = false;
                        self.find_text.clear();
                        self.exec_action(r.a);
                    }
                    other => {
                        self.find_focused = false;
                        self.find_open = false;
                        self.exec_action(other);
                    }
                }
                return;
            }
        }
        self.find_focused = false;
        self.find_open = false;
        self.pick(x, y);
    }

    /// ENTER in the FIND EVENT field: select the best (first) match.
    fn find_pick_first(&mut self) {
        let q = self.find_text.trim().to_uppercase();
        if !q.is_empty() {
            let pick = self
                .store
                .events()
                .iter()
                .take(512)
                .enumerate()
                .find(|(_, qk)| {
                    qk.place.to_uppercase().contains(&q) || qk.id.to_uppercase().contains(&q)
                })
                .map(|(i, qk)| (i, qk.id.clone()));
            if let Some((i, id)) = pick {
                self.selected_id = Some(id);
                self.selected_station = None;
                if let Some(qk) = self.store.get(i).cloned() {
                    self.playhead = qk.time_s();
                    self.live = false;
                    self.track_event(&qk);
                }
            }
        }
        self.find_text.clear();
        self.find_open = false;
        self.find_focused = false;
    }

    pub fn in_gain(&self, x: f32, y: f32) -> bool {
        let [gx, gy, gw, gh] = self.gain_rect;
        gw > 0.0 && x >= gx - 4.0 && x <= gx + gw + 4.0 && y >= gy - 6.0 && y <= gy + gh + 6.0
    }

    pub fn in_speed(&self, x: f32, y: f32) -> bool {
        let [gx, gy, gw, gh] = self.speed_rect;
        gw > 0.0 && x >= gx - 4.0 && x <= gx + gw + 4.0 && y >= gy - 6.0 && y <= gy + gh + 6.0
    }

    /// Log-mapped slider position -> playback speed 1x..240x.
    pub fn speed_to(&mut self, x: f32) {
        let [gx, _gy, gw, _gh] = self.speed_rect;
        if gw <= 0.0 {
            return;
        }
        let f = ((x - gx) / gw).clamp(0.0, 1.0);
        self.play_speed = (240.0f32).powf(f).max(1.0);
    }

    /// PLAY: replay the visible window from its start (or resume playback);
    /// pressing again while playing stops at the current playhead.
    pub fn toggle_play(&mut self) {
        if self.playing {
            self.playing = false;
            return;
        }
        if self.live {
            let (ws, _we) = self.win_bounds();
            self.playhead = ws;
            self.live = false;
        }
        self.playing = true;
    }

    /// Log-mapped slider position -> trace gain 0.3x..4.0x.
    pub fn gain_to(&mut self, x: f32) {
        let [gx, _gy, gw, _gh] = self.gain_rect;
        if gw <= 0.0 {
            return;
        }
        let f = ((x - gx) / gw).clamp(0.0, 1.0);
        self.trace_gain = 0.3 * (4.0f32 / 0.3).powf(f);
    }

    fn exec_action(&mut self, a: crate::hud::UiAction) {
        use crate::hud::UiAction as A;
        match a {
            A::ZoomIn => self.cam.zoom(70.0),
            A::ZoomOut => self.cam.zoom(-70.0),
            A::ViewFit => self.cam.target_dist = 3.6,
            A::ResetView => {
                self.cam.yaw = 0.35;
                self.cam.pitch = 0.28;
                self.cam.target_dist = 3.6;
                self.cam.yaw_v = 0.0;
                self.cam.pitch_v = 0.0;
                self.cam.focus = None;
            }
            A::Play => self.toggle_play(),
            A::Stop => {
                self.playing = false;
            }
            A::TglCities => self.toggles.cities = !self.toggles.cities,
            A::AutoSpin => {
                self.toggles.spin = !self.toggles.spin;
                self.cam.auto_spin = self.toggles.spin;
            }
            A::InvX => self.inv_x = !self.inv_x,
            A::InvY => self.inv_y = !self.inv_y,
            A::Fullscreen => {
                if let Some(w) = self.window.as_ref() {
                    self.fullscreen = !self.fullscreen;
                    w.set_fullscreen(if self.fullscreen {
                        Some(winit::window::Fullscreen::Borderless(None))
                    } else {
                        None
                    });
                }
            }
            A::CamFree => {
                self.auto_track = false;
                self.cam.focus = None;
            }
            A::CamTrack => self.auto_track = true,
            A::CamTop => {
                self.cam.focus = Some((self.cam.yaw, 1.45));
                self.focus_until = self.now + 1.5;
            }
            A::Tips => self.tips = !self.tips,
            A::FindFocus => {
                self.find_focused = true;
                self.find_open = true;
            }
            A::Span(h) => {
                self.win_mode = h;
                let (ws, we) = self.win_bounds();
                self.playhead = self.playhead.clamp(ws, we);
            }
            A::Rewind => {
                let (ws, _) = self.win_bounds();
                self.playhead = ws;
                self.live = false;
            }
            A::Live => {
                self.live = true;
                self.playhead = self.now;
            }
            A::Pause => self.toggles.pause = !self.toggles.pause,
            A::Simf => self.toggle_simf(),
            A::TglQuakes => self.toggles.quakes = !self.toggles.quakes,
            A::TglStations => self.toggles.stations = !self.toggles.stations,
            A::TglWaves => self.toggles.waves = !self.toggles.waves,
            A::TglLabels => self.toggles.labels = !self.toggles.labels,
            A::TglGrid => self.toggles.grid = !self.toggles.grid,
            A::Collapse(id) => self.panels_collapsed ^= 1 << id,
            A::ListEvent(i) => {
                if let Some(q) = self.store.get(i).cloned() {
                    self.selected_id = Some(q.id.clone());
                    self.selected_station = None;
                    self.playhead = q.time_s();
                    self.live = false;
                    self.track_event(&q);
                }
            }
        }
    }

    /// Nearest front-facing marker within 14 px of the click becomes selected.
    fn pick(&mut self, x: f32, y: f32) {
        let aspect = self.gpu.as_ref().map(|g| g.aspect()).unwrap_or(1.777);
        let vp = self.cam.view_proj(aspect);
        let size = self.vp_size();
        let thr = 14.0 * self.ui_scale;
        if self.toggles.stations {
            let mut best: Option<(f32, usize)> = None;
            for (i, st) in crate::sensors::SENSORS.iter().enumerate() {
                let p = crate::model::geo_to_cartesian(st.lat, st.lon, 1.0);
                let p = Vector3::new(p[0], p[1], p[2]);
                if let Some((sx, sy, facing)) = self.cam.project(p, vp, size) {
                    if facing < 0.05 {
                        continue;
                    }
                    let d = ((sx - x).powi(2) + (sy - y).powi(2)).sqrt();
                    if d < thr && best.as_ref().map_or(true, |(bd, _)| d < *bd) {
                        best = Some((d, i));
                    }
                }
            }
            if let Some((_, i)) = best {
                self.selected_station = Some(i);
                self.selected_id = None;
                return;
            }
        }
        let mut best: Option<(f32, String)> = None;
        if self.toggles.quakes {
            for q in self.store.events() {
                if q.time_s() > self.playhead + 0.5 {
                    continue;
                }
                let p = q.unit_pos();
                let p = Vector3::new(p[0], p[1], p[2]);
                if let Some((sx, sy, facing)) = self.cam.project(p, vp, size) {
                    if facing < 0.05 {
                        continue;
                    }
                    let d = ((sx - x).powi(2) + (sy - y).powi(2)).sqrt();
                    if d < thr && best.as_ref().map_or(true, |(bd, _)| d < *bd) {
                        best = Some((d, q.id.clone()));
                    }
                }
            }
        }
        let picked = best.map(|(_, id)| id);
        if picked.is_some() {
            self.selected_station = None;
        }
        self.selected_id = picked;
    }

    /// 60 fps cap: skip redundant redraws between presents.
    const FRAME_DUR: std::time::Duration = std::time::Duration::from_micros(16_667);

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        if self.shot.is_none()
            && std::time::Instant::now() < self.present_at + Self::FRAME_DUR - std::time::Duration::from_millis(1)
        {
            return;
        }
        let dt = self.last.elapsed().as_secs_f32().min(0.1);
        self.last = Instant::now();
        self.now = timeutil::now_secs();
        if dt > 1e-4 {
            let inst = 1.0 / dt as f64;
            self.fps += (inst - self.fps) * 0.08;
        }
        self.pump_net();
        if !self.demo_applied && std::env::var("TREMOR_DEMO").is_ok() {
            self.demo_applied = true;
            self.live = false;
            self.playhead = self.now - 5.0;
            self.selected_station = Some(3);
        }
        if self.live {
            self.playhead = self.now;
        } else if self.playing {
            self.playhead += dt as f64 * self.play_speed as f64;
            if self.playhead >= self.now {
                self.playhead = self.now;
                self.live = true;
                self.playing = false;
            }
        }
        self.flash.retain(|(t, _, _)| self.now - t < 2.7);
        if self.sim_local && self.now >= self.next_sim_at {
            self.sim_seq += 1;
            let q = crate::net::sim_event(&mut self.rng, self.sim_seq);
            if self.store.ingest(q.clone(), self.now) {
                self.flash.push((self.now, q.unit_pos(), q.mag));
                if self.auto_track {
                    self.track_event(&q);
                }
            }
            self.next_sim_at = self.now + 10.0 + self.rng.range(0.0, 50.0) as f64;
        }
        self.store.prune_waves(self.now);
        self.store.prune_events(self.now);
        if self.now > self.focus_until {
            self.cam.focus = None;
        }
        self.cam.update(dt);

        if self.gpu.is_none() {
            return;
        }
        let (gw, gh) = self.vp_size();
        let aspect = gw as f32 / gh.max(1) as f32;

        // ---------- CPU side: instances + HUD (all &mut self work) ----------
        let vp = self.cam.view_proj(aspect);
        let sel = self.selected_index();
        self.marker_buf.clear();
        if self.toggles.quakes {
            let dist_k = self.cam.dist / 3.1;
            let win_s = crate::model::EVENT_WINDOW_H * 3600.0;
            let mut latest_at: Option<usize> = None;
            let mut latest_t = f64::NEG_INFINITY;
            for (i, q) in self.store.events().iter().take(512).enumerate() {
                if q.time_s() > self.playhead + 0.5 {
                    continue;
                }
                let p = q.unit_pos();
                let age = (self.playhead - q.time_s()).max(0.0);
                // fresh solutions pop, old ones dim across the 48 h window
                let alpha = 1.0 - 0.45 * (age / win_s).min(1.0) as f32;
                // magnitude-mapped dot size (superlinear so M7 reads big)
                let size = (0.008 + 0.0055 * (q.mag - 3.0).clamp(0.0, 6.0).powf(1.35)) * dist_k;
                let mut col = if q.simulated {
                    pal::CYAN
                } else {
                    pal::mag_color(q.mag)
                };
                col[3] = alpha;
                if q.time_s() > latest_t {
                    latest_t = q.time_s();
                    latest_at = Some(self.marker_buf.len());
                }
                self.marker_buf.push(MarkerGpu {
                    pos: [p[0] * 1.02, p[1] * 1.02, p[2] * 1.02, 0.0],
                    color: col,
                    misc: [
                        size,
                        if sel == Some(i) { 1.0 } else { 0.0 },
                        age as f32,
                        q.mag,
                    ],
                });
            }
            // newest event on the globe: crosshair reticle + circle around it
            if let Some(idx) = latest_at {
                let m = &mut self.marker_buf[idx];
                m.misc[0] = (m.misc[0] * 2.2).max(0.034 * dist_k);
                m.misc[1] = 3.0;
            }
        }
        if self.toggles.cities {
            for (lat, lon, _name) in crate::assets::capitals() {
                let p = crate::model::geo_to_cartesian(lat, lon, 1.0);
                let size = 0.0075 * (self.cam.dist / 3.1);
                self.marker_buf.push(MarkerGpu {
                    pos: [p[0] * 1.012, p[1] * 1.012, p[2] * 1.012, 0.0],
                    color: pal::with_a(pal::AMBER, 0.9),
                    misc: [size, 4.0, 0.0, 0.0],
                });
            }
        }
        if self.toggles.stations {
            for sen in crate::sensors::SENSORS {
                let p = crate::model::geo_to_cartesian(sen.lat, sen.lon, 1.0);
                let size = 0.011 * (self.cam.dist / 3.1);
                self.marker_buf.push(MarkerGpu {
                    pos: [p[0] * 1.012, p[1] * 1.012, p[2] * 1.012, 0.0],
                    color: pal::with_a(pal::GRID_HI, 0.85),
                    misc: [size, 2.0, 0.0, 0.0],
                });
            }
        }
        self.wave_buf.clear();
        // timeline-derived fronts: every event whose origin has passed the
        // playhead expands from ORIGIN time, so scrubbing replays the window
        for q in self
            .store
            .events()
            .iter()
            .filter(|q| {
                let age = self.playhead - q.time_s();
                age >= 0.0 && age <= crate::model::wave_ttl(q.mag)
            })
            .take(MAX_WAVES.saturating_sub(self.flash.len()))
        {
            self.wave_buf.push(WaveGpu::from_quake(q, self.playhead));
        }
        for (t, c, m) in self.flash.iter() {
            let mut f = WaveGpu::flash(*c, *m);
            f.center[3] = (*t - self.playhead) as f32;
            self.wave_buf.push(f);
        }
        self.epic_buf.clear();
        for (i, q) in self.store.events().iter().take(128).enumerate() {
            if q.time_s() > self.playhead + 0.5 {
                continue;
            }
            let p = q.unit_pos();
            let ttl = crate::model::wave_ttl(q.mag) as f32;
            self.epic_buf.push(crate::gpu::globe::EpicGpu {
                center: [p[0], p[1], p[2], q.mag],
                params: [
                    (self.playhead - q.time_s()).max(0.0) as f32,
                    ttl,
                    if sel == Some(i) { 1.0 } else { 0.0 },
                    0.0,
                ],
            });
        }

        self.batch.clear();
        crate::hud::build_hud(self, gw, gh);
        if self.toggles.labels {
            crate::hud::build_labels(self, vp, (gw, gh));
        }

        // ---------- GPU side (immutable borrows only) ----------
        let gpu = self.gpu.as_ref().unwrap();
        let globe = self.globe.as_ref().unwrap();
        let markers = self.markers.as_ref().unwrap();
        let ui = self.ui.as_ref().unwrap();
        let window = self.window.as_ref().unwrap();

        let surface_tex = match gpu.surface.get_current_texture() {
            Ok(t) => t,
            Err(_) => {
                window.request_redraw();
                return;
            }
        };
        let view = surface_tex
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        let eye = self.cam.eye();
        let (right, up) = self.cam.basis();
        let (slat, slon) = timeutil::subsolar_point(self.playhead);
        let sun = crate::model::geo_to_cartesian(slat as f32, slon as f32, 1.0);
        let gu = GlobeUniforms {
            view_proj: vp.into(),
            cam_pos: [eye.x, eye.y, eye.z, 0.0],
            light_dir: [sun[0], sun[1], sun[2], 0.0],
            sun_dir: [sun[0], sun[1], sun[2], 0.0],
            misc: [
                0.0, // waves carry frame-relative start times
                if self.toggles.grid { 1.0 } else { 0.0 },
                if self.toggles.waves { 1.0 } else { 0.0 },
                self.wave_buf.len() as f32,
            ],
            misc2: [self.epic_buf.len() as f32, 0.0, 0.0, 0.0],
        };
        gpu.queue
            .write_buffer(&globe.uniforms, 0, bytemuck::bytes_of(&gu));
        // background sky: starfield + moon at the playhead instant
        if let Some(bg) = self.bg.as_ref() {
            use cgmath::SquareMatrix;
            let (mlat, mlon) = timeutil::sublunar_point(self.playhead);
            let mdir = crate::model::geo_to_cartesian(mlat as f32, mlon as f32, 1.0);
            let inv = vp.invert().unwrap_or_else(|| vp);
            let bu = BgUniforms {
                inv_vp: inv.into(),
                cam_pos: [eye.x, eye.y, eye.z, 0.0],
                sun_dir: gu.sun_dir,
                moon_dir: [mdir[0], mdir[1], mdir[2], 0.085],
                misc: [0.0, 1.0, 1.0, 0.0],
            };
            gpu.queue.write_buffer(&bg.uniforms, 0, bytemuck::bytes_of(&bu));
        }
        globe.write_waves(&gpu.queue, &self.wave_buf);
        globe.write_epics(&gpu.queue, &self.epic_buf);

        let marker_count = self.marker_buf.len() as u32;
        let mu = MarkerUniforms {
            view_proj: vp.into(),
            right: [right.x, right.y, right.z, 0.0],
            up: [up.x, up.y, up.z, 0.0],
            misc: [(self.now - self.t0) as f32, 0.0, 0.0, 0.0],
        };
        gpu.queue
            .write_buffer(&markers.uniforms, 0, bytemuck::bytes_of(&mu));
        markers.write(&gpu.queue, &self.marker_buf);

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &gpu.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            if let Some(bg) = self.bg.as_ref() {
                bg.draw(&mut pass);
            }
            globe.draw(&mut pass);
            markers.draw(&mut pass, marker_count);
            ui.draw(
                &mut pass,
                &gpu.queue,
                &self.batch,
                [gw as f32, gh as f32],
            );
        }

        // ---------- optional headless snapshot ----------
        let mut capture: Option<wgpu::Buffer> = None;
        let want_shot = matches!(self.shot.as_ref(), Some(s) if !s.done && self.frames + 1 >= s.frames);
        if want_shot {
            let padded = (gw * 4 + 255) / 256 * 256;
            let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("capture"),
                size: (padded * gh) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &surface_tex.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buf,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded),
                        rows_per_image: Some(gh),
                    },
                },
                wgpu::Extent3d {
                    width: gw,
                    height: gh,
                    depth_or_array_layers: 1,
                },
            );
            capture = Some(buf);
        }

        gpu.queue.submit(Some(encoder.finish()));

        if let Some(buf) = capture {
            let fmt = gpu.config.format;
            let padded = (gw * 4 + 255) / 256 * 256;
            let slice = buf.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            let _ = gpu.device.poll(wgpu::PollType::Wait);
            let data = slice.get_mapped_range();
            let mut rgb = vec![0u8; (gw * gh * 3) as usize];
            for y in 0..gh as usize {
                let row = &data[y * padded as usize..][..gw as usize * 4];
                for x in 0..gw as usize {
                    let p = &row[x * 4..x * 4 + 4];
                    let (r, g, b) = match fmt {
                        wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm => {
                            (p[2], p[1], p[0])
                        }
                        _ => (p[0], p[1], p[2]),
                    };
                    let o = (y * gw as usize + x) * 3;
                    rgb[o] = r;
                    rgb[o + 1] = g;
                    rgb[o + 2] = b;
                }
            }
            drop(data);
            buf.unmap();
            if let Some(shot) = self.shot.as_mut() {
                let path = shot.path.clone();
                let mut file = format!("P6\n{gw} {gh}\n255\n").into_bytes();
                file.extend_from_slice(&rgb);
                match std::fs::write(&path, file) {
                    Ok(_) => eprintln!("tremor: snapshot written to {path}"),
                    Err(e) => eprintln!("tremor: snapshot failed: {e}"),
                }
                shot.done = true;
            }
            event_loop.exit();
            return;
        }

        surface_tex.present();
        self.frames += 1;
        self.present_at = std::time::Instant::now();
        window.request_redraw();
    }
}
