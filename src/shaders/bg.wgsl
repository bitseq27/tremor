// TREMOR background sky: procedural starfield (hash cells on the view ray,
// celestial-sphere stable under camera orbit) + moon disc with phase shading
// from the sun-moon geometry. Drawn first, no depth write.

struct Globals {
    inv_vp:   mat4x4<f32>,
    cam_pos:  vec4<f32>,
    sun_dir:  vec4<f32>, // xyz subsolar direction
    moon_dir: vec4<f32>, // xyz sub-lunar direction, w angular radius (rad)
    misc:     vec4<f32>, // x time, y stars on, z moon on
};

@group(0) @binding(0) var<uniform> u: Globals;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0)       ndc:  vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // fullscreen triangle
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var o: VsOut;
    o.clip = vec4<f32>(p[vi], 0.9999, 1.0);
    o.ndc  = p[vi];
    return o;
}

fn hash3(p: vec3<f32>) -> f32 {
    let q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    let d = dot(q, q.yzx + 33.33);
    return fract((d + d) * d);
}

// value noise on the ray direction -> star cells
fn star_field(dir: vec3<f32>) -> f32 {
    var acc = 0.0;
    // three octaves of sparse cells: bright pin-stars + faint dust
    var scale = 42.0;
    var amp = 1.0;
    for (var i = 0; i < 3; i = i + 1) {
        let p = dir * scale;
        let cell = floor(p);
        let f = fract(p);
        let h = hash3(cell + f32(i) * 17.0);
        // one star per cell at a hashed offset
        let sp = vec3<f32>(hash3(cell + 1.7), hash3(cell + 3.1), hash3(cell + 5.3));
        let d = length(f - sp);
        let bright = step(0.86 - 0.06 * f32(i), h);
        let core = 1.0 - smoothstep(0.0, 0.22 + 0.10 * f32(i), d);
        acc += bright * core * amp;
        scale = scale * 2.17;
        amp = amp * 0.55;
    }
    return acc;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // reconstruct the view ray from the inverse view-projection
    let p0 = u.inv_vp * vec4<f32>(in.ndc, 1.0, 1.0);
    let p1 = u.inv_vp * vec4<f32>(in.ndc, 0.0, 1.0);
    let ray = normalize(p0.xyz / p0.w - p1.xyz / p1.w);

    var col = vec3<f32>(0.0);

    // ---- starfield: celestial-sphere stable, faint twinkle ----
    if (u.misc.y > 0.5) {
        let s = star_field(ray);
        let tw = 0.82 + 0.18 * sin(u.misc.x * 1.7 + s * 40.0);
        // slight colour temperature spread: blue-white / warm pins
        let h = hash3(floor(ray * 42.0));
        let tint = mix(vec3<f32>(0.72, 0.84, 1.0), vec3<f32>(1.0, 0.92, 0.78), step(0.7, h));
        col += tint * s * tw * 0.85;
        // faint milky-way band along a tilted great circle
        let band = exp(-pow(dot(ray, normalize(vec3<f32>(0.42, 0.82, -0.38))), 2.0) * 9.0);
        col += vec3<f32>(0.055, 0.065, 0.085) * band * 0.55;
    }

    // ---- moon: disc at the sub-lunar direction, phase from sun angle ----
    if (u.misc.z > 0.5) {
        let m = normalize(u.moon_dir.xyz);
        let ar = u.moon_dir.w;
        let ca = clamp(dot(ray, m), -1.0, 1.0);
        let ang = acos(ca);
        if (ang < ar * 1.35) {
            // sphere normal at this ray (small-angle reconstruction)
            let t = normalize(ray - m * ca);
            let n = normalize(m * cos(min(ang, ar)) + t * sin(min(ang, ar)));
            let illum = max(dot(n, normalize(u.sun_dir.xyz)), 0.0);
            // maria / crater mottling
            let cr = hash3(floor(n * 24.0)) * 0.5 + hash3(floor(n * 61.0)) * 0.5;
            var g = 0.62 + 0.30 * cr;
            // limb darkening + soft edge
            let edge = 1.0 - smoothstep(ar * 0.94, ar * 1.06, ang);
            let limb = 0.35 + 0.65 * pow(max(0.0, 1.0 - pow(ang / ar, 2.0)), 0.35);
            // phase: terminator across the disc from the sun direction
            let ph = smoothstep(-0.06, 0.10, illum);
            let glow = exp(-pow(max(0.0, ang - ar) / (ar * 0.30), 2.0)) * 0.10;
            col += vec3<f32>(0.86, 0.87, 0.82) * g * limb * ph * edge * 0.92
                 + vec3<f32>(0.30, 0.32, 0.34) * glow;
        }
    }

    return vec4<f32>(col, 1.0);
}
