// Output post pass: colour adjustment, gamma-correct edge blend, master and blackout.
// Mirrors lumen_render::scene::Post::apply exactly.

struct PostUniforms {
    widths: vec4<f32>,   // left, right, top, bottom
    curves: vec4<u32>,   // 0 linear, 1 smooth, 2 cosine
    params: vec4<f32>,   // blend gamma, brightness, contrast, output gamma
    params2: vec4<f32>,  // master, blackout, unused, unused
};

@group(0) @binding(0) var<uniform> u: PostUniforms;
@group(0) @binding(1) var composite: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // One triangle covering the whole target.
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    var out: VsOut;
    out.clip = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}

const PI: f32 = 3.14159265358979;

fn curve(kind: u32, t_in: f32) -> f32 {
    let t = clamp(t_in, 0.0, 1.0);
    if kind == 0u {
        return t;
    }
    if kind == 1u {
        return t * t * (3.0 - 2.0 * t);
    }
    return 0.5 - 0.5 * cos(PI * t);
}

fn edge(width: f32, kind: u32, d: f32) -> f32 {
    if width <= 0.0 {
        return 1.0;
    }
    return curve(kind, d / width);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if u.params2.y > 0.5 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let c_in = textureSample(composite, samp, in.uv).rgb;
    let x = in.uv.x;
    let y = in.uv.y;
    let lin = edge(u.widths.x, u.curves.x, x)
        * edge(u.widths.y, u.curves.y, 1.0 - x)
        * edge(u.widths.z, u.curves.z, y)
        * edge(u.widths.w, u.curves.w, 1.0 - y);
    let factor = pow(lin, 1.0 / u.params.x) * clamp(u.params2.x, 0.0, 1.0);
    var c = clamp((c_in - vec3<f32>(0.5)) * u.params.z + vec3<f32>(0.5), vec3<f32>(0.0), vec3<f32>(1.0));
    c = clamp(c * u.params.y, vec3<f32>(0.0), vec3<f32>(1.0));
    if abs(u.params.w - 1.0) >= 1e-9 {
        c = pow(c, vec3<f32>(1.0 / max(u.params.w, 1e-3)));
    }
    c = clamp(c * factor, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(c, 1.0);
}
