// TREMOR epicentre markers: instanced billboards.
// misc.y selects the object class:
//   0 = epicentre dot (magnitude-sized / magnitude-coloured)
//   1 = selected epicentre dot + pulsing ring
//   2 = sensor station crosshair
//   3 = newest event: crosshair reticle with a circle around it
//   4 = capital city: small hollow ring + centre pip

struct MarkerU {
    view_proj: mat4x4<f32>,
    right:     vec4<f32>,
    up:        vec4<f32>,
    misc:      vec4<f32>, // x time
};
@group(0) @binding(0) var<uniform> u: MarkerU;

struct Marker {
    pos:   vec4<f32>, // xyz unit epicentre
    color: vec4<f32>,
    misc:  vec4<f32>, // x world size, y class, z age, w mag
};
@group(1) @binding(0) var<storage, read> marks: array<Marker>;

struct VsOut {
    @builtin(position) clip:  vec4<f32>,
    @location(0)       uv:    vec2<f32>,
    @location(1)       color: vec4<f32>,
    @location(2)       misc:  vec4<f32>,
};

const CORNERS: array<vec2<f32>, 4> = array<vec2<f32>, 4>(
    vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0)
);
const TRI: array<u32, 6> = array<u32, 6>(0u, 1u, 2u, 1u, 3u, 2u);

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    let c = CORNERS[TRI[vi]];
    let m = marks[ii];
    let world = m.pos.xyz + (u.right.xyz * c.x + u.up.xyz * c.y) * m.misc.x;
    var o: VsOut;
    o.clip  = u.view_proj * vec4<f32>(world, 1.0);
    o.uv    = c * 0.5 + 0.5;
    o.color = m.color;
    o.misc  = m.misc;
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let kind = u32(round(in.misc.y));
    let q = in.uv - 0.5;
    let d = length(q) * 2.0;
    var a = 0.0;

    if (kind == 4u) {
        // capital city: crisp hollow ring + centre pip, no halo
        let ring = 1.0 - smoothstep(0.0, 0.16, abs(d - 0.62));
        let pip = 1.0 - smoothstep(0.10, 0.18, d);
        a = max(ring * 0.9, pip) * in.color.a;
    } else if (kind == 2u) {
        // sensor station: crisp crosshair + centre pip, no halo/pulse
        let bar = (abs(q.x) < 0.055 && abs(q.y) < 0.34) || (abs(q.y) < 0.055 && abs(q.x) < 0.34);
        let pip = 1.0 - smoothstep(0.10, 0.16, d);
        let ring = (1.0 - smoothstep(0.0, 0.10, abs(d - 0.72))) * 0.6;
        a = max(select(0.0, 1.0, bar), max(pip, ring)) * in.color.a;
    } else if (kind == 3u) {
        // newest event: crosshair arms (gapped at the centre) + circle
        let pip  = 1.0 - smoothstep(0.10, 0.20, d);
        let armx = abs(q.y) < 0.030 && abs(q.x) > 0.16 && abs(q.x) < 0.46;
        let army = abs(q.x) < 0.030 && abs(q.y) > 0.16 && abs(q.y) < 0.46;
        let ring = 1.0 - smoothstep(0.02, 0.08, abs(d - 0.78));
        let pulse = 0.72 + 0.28 * sin(u.misc.x * 4.0);
        a = max(max(pip, select(0.0, 1.0, armx || army)), ring * pulse) * in.color.a;
    } else {
        // epicentre dot: solid core + soft halo; selection adds pulse ring
        let core = 1.0 - smoothstep(0.22, 0.52, d);
        let halo = exp(-d * d * 4.5) * 0.50;
        let pulse = 0.55 + 0.45 * sin(u.misc.x * 7.0);
        let ring = (1.0 - smoothstep(0.0, 0.14, abs(d - 0.80)))
                 * select(0.0, 1.0, kind == 1u) * pulse;
        a = clamp(core + halo + ring, 0.0, 1.0) * in.color.a;
    }

    if (d > 0.95) { a = 0.0; }
    if (a < 0.004) { discard; }
    return vec4<f32>(in.color.rgb, a);
}
