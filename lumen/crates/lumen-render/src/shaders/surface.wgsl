// Draws one warped surface into the composite buffer with premultiplied alpha.

struct Uniforms {
    crop: vec4<f32>,    // x, y, w, h in normalised source space
    opacity: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var src_tex: texture_2d<f32>;
@group(0) @binding(2) var src_samp: sampler;
@group(0) @binding(3) var mask_tex: texture_2d<f32>;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uvw: vec3<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uvw: vec3<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = vec4<f32>(in.pos.x * 2.0 - 1.0, 1.0 - in.pos.y * 2.0, 0.0, 1.0);
    // Clip w is 1 for every vertex, so the rasteriser interpolates uvw linearly in screen
    // space; dividing by uvw.z in the fragment stage then yields perspective-correct uv.
    out.uvw = in.uvw;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = clamp(in.uvw.xy / in.uvw.z, vec2<f32>(0.0), vec2<f32>(1.0));
    let mask = textureSample(mask_tex, src_samp, uv).r;
    let suv = u.crop.xy + uv * u.crop.zw;
    let s = textureSample(src_tex, src_samp, suv);
    let a = s.a * u.opacity * mask;
    return vec4<f32>(s.rgb * a, a);
}
