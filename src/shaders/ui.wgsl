// TREMOR HUD shader: solid quads + 1bpp font atlas sampling in pixel space.

struct UiU {
    screen: vec4<f32>, // x width, y height
};
@group(0) @binding(0) var<uniform> u: UiU;
@group(1) @binding(0) var font_tex: texture_2d<f32>;
@group(1) @binding(1) var font_smp: sampler;

struct VsOut {
    @builtin(position) clip:  vec4<f32>,
    @location(0)       uv:    vec2<f32>,
    @location(1)       color: vec4<f32>,
    @location(2)       mode:  f32,
};

@vertex
fn vs_main(
    @location(0) pos:   vec2<f32>,
    @location(1) uv:    vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) mmode: vec2<f32>,
) -> VsOut {
    var o: VsOut;
    let ndc = vec2<f32>(pos.x / u.screen.x * 2.0 - 1.0, 1.0 - pos.y / u.screen.y * 2.0);
    o.clip  = vec4<f32>(ndc, 0.0, 1.0);
    o.uv    = uv;
    o.color = color;
    o.mode  = mmode.x;
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (in.mode < 0.5) {
        return in.color;
    }
    let a = textureSample(font_tex, font_smp, in.uv).r * in.color.a;
    if (a < 0.02) { discard; }
    return vec4<f32>(in.color.rgb, a);
}
