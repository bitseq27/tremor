// TREMOR globe shader: equirectangular land texture + procedural graticule
// + GPU-side expanding seismic wave fronts with exponential decay.

struct Globals {
    view_proj: mat4x4<f32>,
    cam_pos:   vec4<f32>,
    light_dir: vec4<f32>,
    misc:      vec4<f32>, // x time, y graticule, z waves on, w wave count
    misc2:     vec4<f32>, // x epicentre mark count
    sun_dir:   vec4<f32>, // xyz subsolar direction (day/night terminator)
};

@group(0) @binding(0) var<uniform> u: Globals;
@group(1) @binding(0) var earth_tex: texture_2d<f32>;
@group(1) @binding(1) var earth_smp: sampler;
@group(1) @binding(2) var lights_tex: texture_2d<f32>;

struct Wave {
    center: vec4<f32>, // xyz unit epicentre, w start time
    params: vec4<f32>, // x mag, y ttl, z angular speed, w ring width
    extra:  vec4<f32>, // x kind (0 = event wave, 1 = new-event flash)
};
@group(2) @binding(0) var<storage, read> waves: array<Wave>;

struct VsOut {
    @builtin(position) clip:  vec4<f32>,
    @location(0)       norm:  vec3<f32>,
    @location(1)       world: vec3<f32>,
};

@vertex
fn vs_main(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>) -> VsOut {
    var o: VsOut;
    o.clip  = u.view_proj * vec4<f32>(position, 1.0);
    o.norm  = normal;
    o.world = position;
    return o;
}

const PI: f32 = 3.141592653589793;

// magnitude ramp in orrery HMI accents (linear values): GREEN / AMBER /
// ALARM, white-hot GRID_HI for great events
fn wave_color(mag: f32) -> vec3<f32> {
    if (mag >= 7.0) { return vec3<f32>(1.0, 1.0, 1.0); }            // GRID_HI
    if (mag >= 6.0) { return vec3<f32>(0.6445, 0.04374, 0.02732); } // ALARM
    if (mag >= 5.0) { return vec3<f32>(0.7454, 0.3916, 0.0); }      // AMBER
    return vec3<f32>(0.05613, 0.3515, 0.06301);                     // GREEN
}

struct Epic {
    center: vec4<f32>, // xyz unit epicentre, w mag
    params: vec4<f32>, // x age, y ttl, z selected
};
@group(3) @binding(0) var<storage, read> epics: array<Epic>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.norm);

    // equirectangular uv derived from the normal so the texture never slips
    let lon  = atan2(n.x, n.z);
    let lat  = asin(clamp(n.y, -1.0, 1.0));
    let uv   = vec2<f32>(lon / (2.0 * PI) + 0.5, 0.5 - lat / PI);
    var base = textureSample(earth_tex, earth_smp, uv).rgb;

    // ---- graticule (15 deg mesh, brighter equator / prime meridian) ----
    var grat = 0.0;
    if (u.misc.y > 0.5) {
        let londe = lon * 180.0 / PI;
        let late  = lat * 180.0 / PI;
        let dx = abs(fract(londe / 15.0 + 0.5) - 0.5) * 15.0;
        let dy = abs(fract(late  / 15.0 + 0.5) - 0.5) * 15.0;
        let wl = max(fwidth(londe), 0.02) * 1.1;
        let wp = max(fwidth(late),  0.02) * 1.1;
        grat = max(1.0 - smoothstep(0.0, wl, dx), 1.0 - smoothstep(0.0, wp, dy));
        let eq = max(1.0 - smoothstep(0.0, wp * 1.6, abs(late)),
                     1.0 - smoothstep(0.0, wl * 1.6, abs(londe))) * 0.6;
        grat = max(grat, eq);
    }

    // ---- active seismic wave fronts (decay equation on GPU) ----
    var wcol = vec3<f32>(0.0);
    if (u.misc.z > 0.5) {
        let count = i32(u.misc.w);
        for (var i = 0; i < count; i = i + 1) {
            let wv  = waves[i];
            let t   = u.misc.x - wv.center.w;
            let ttl = wv.params.y;
            if (t < 0.0 || t > ttl) { continue; }
            let ang = acos(clamp(dot(n, wv.center.xyz), -1.0, 1.0));
            let r   = wv.params.z * t;          // expanding angular radius
            let wd  = wv.params.w * 0.85;
            let d   = ang - r;
            let env = exp(-(d * d) / (wd * wd)); // gaussian ring profile
            var fade = 1.0 - t / ttl;            // dissipative fade
            fade = fade * fade;
            var c = wave_color(wv.params.x);
            var amp = 1.20;
            if (wv.extra.x > 0.5) {
                // NEW-EVENT FLASH: fast white-cyan ring sweeping the sphere
                c = vec3<f32>(0.62, 0.98, 0.94);
                amp = 1.90;
            }
            // young rings are tight and blinding: ramp amplitude in as the
            // front expands (energy spread), cap the newborn flash
            let ramp = clamp(0.30 + r, 0.0, 1.0);
            wcol += (c * (env * fade * amp + env * env * fade * 0.50) + vec3<f32>(0.40) * env * env * env * env * fade) * ramp;
        }
    }

    // ---- lighting: sun follows the PLAYHEAD (subsolar point) so the
    // terminator is correct for the reviewed instant; night side falls to
    // near-black and shows the baked city-lights map (GeoNames, CC-BY 4.0)
    let S    = normalize(u.sun_dir.xyz);
    let ndl  = dot(n, S);
    let day  = smoothstep(-0.12, 0.18, ndl);
    let L    = normalize(u.light_dir.xyz);
    let diff = max(dot(n, L), 0.0) * 0.62 + 0.38;
    let vdir = normalize(u.cam_pos.xyz - in.world);
    let rim  = pow(1.0 - max(dot(n, vdir), 0.0), 3.0);

    var col = base * diff * mix(0.10, 1.0, day);
    // city lights on the dark side only, warm sodium glow
    let lt = textureSample(lights_tex, earth_smp, uv).rgb;
    col += lt * (1.0 - day) * 1.35;
    col += vec3<f32>(0.1559) * grat * 0.45; // GRID-grey graticule
    col += wcol;
    // ---- persistent epicentre marks stamped on the surface ----
    let ecount = i32(u.misc2.x);
    for (var i = 0; i < ecount; i = i + 1) {
        let e = epics[i];
        let c = normalize(e.center.xyz);
        let ang = acos(clamp(dot(n, c), -1.0, 1.0));
        if (ang > 0.06) { continue; }
        let up0 = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(c.y) > 0.9);
        let t1 = normalize(cross(up0, c));
        let t2 = cross(c, t1);
        let uu = dot(n, t1);
        let vv = dot(n, t2);
        let du = uu + vv;           // 45 deg rotated frame -> X shape
        let dv = uu - vv;
        let arm = 0.020;
        let th  = 0.0032;
        let xmark = (abs(du) < th && abs(dv) < arm * 1.4) || (abs(dv) < th && abs(du) < arm * 1.4);
        let pip = ang < 0.0045;
        let sel = e.params.z;
        let ring = (1.0 - smoothstep(0.0, 0.008, abs(ang - 0.042))) * sel
                 * (0.6 + 0.4 * sin(u.misc.x * 6.0));
        let fresh = clamp(1.0 - e.params.x / max(e.params.y, 1.0), 0.0, 1.0);
        let alpha = mix(0.40, 0.95, fresh);
        let m = max(select(0.0, 1.0, xmark || pip), ring);
        col = mix(col, wave_color(e.center.w), clamp(m * alpha, 0.0, 1.0));
    }

    col += vec3<f32>(0.01521, 0.6308, 0.5583) * rim * 0.16; // CYAN fresnel rim
    return vec4<f32>(col, 1.0);
}
