// YUV -> RGB for the video player (Phase V4b; `video_gpu.rs`). Mirrors
// `soyuz_media::yuv::Conversion::apply` (the CPU reference the tests compare with).
//
// Planes are integer textures (R8 / RG8 / R16 / RG16 `Uint`), read with `textureLoad` and
// filtered by hand (bilinear on texel centres), so 8- and 16-bit samples share one path.
// The quad fills the paint callback's viewport; `rotation` turns the picture clockwise.

struct Params {
    // Rows of the affine map from 8-bit codes (Y, Cb, Cr, 1) to non-linear R'G'B'.
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    // Raw sample -> 8-bit code units.
    scale: f32,
    // 0 = SDR, 1 = PQ, 2 = HLG.
    transfer: u32,
    // 1: BT.2020 -> BT.709 primaries after linearising (HDR).
    gamut: u32,
    // 1: the chroma plane holds U / V pairs (NV12).
    nv12: u32,
    // Quarter turns clockwise.
    rotation: u32,
    // 1: the target is an sRGB format (write linear values).
    linear_out: u32,
    // Tone-mapping peak relative to reference white.
    peak: f32,
    // Reference white in nits.
    white: f32,
    // Luma and chroma plane sizes in texels.
    size: vec2<f32>,
    csize: vec2<f32>,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var tex_y: texture_2d<u32>;
@group(0) @binding(2) var tex_u: texture_2d<u32>;
@group(0) @binding(3) var tex_v: texture_2d<u32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let t = corners[i];
    var o: VsOut;
    o.pos = vec4<f32>(t.x * 2.0 - 1.0, 1.0 - t.y * 2.0, 0.0, 1.0);
    o.uv = t;
    return o;
}

// Shown position -> position in the decoded picture (inverse of the clockwise turn).
fn source_uv(uv: vec2<f32>) -> vec2<f32> {
    switch p.rotation {
        case 1u: { return vec2<f32>(uv.y, 1.0 - uv.x); }
        case 2u: { return vec2<f32>(1.0 - uv.x, 1.0 - uv.y); }
        case 3u: { return vec2<f32>(1.0 - uv.y, uv.x); }
        default: { return uv; }
    }
}

fn texel(t: texture_2d<u32>, c: vec2<i32>, size: vec2<i32>) -> vec2<f32> {
    let v = textureLoad(t, clamp(c, vec2<i32>(0, 0), size - vec2<i32>(1, 1)), 0);
    return vec2<f32>(f32(v.x), f32(v.y));
}

fn bilinear(t: texture_2d<u32>, uv: vec2<f32>, size: vec2<f32>) -> vec2<f32> {
    let at = uv * size - vec2<f32>(0.5, 0.5);
    let base = floor(at);
    let f = at - base;
    let b = vec2<i32>(base);
    let s = vec2<i32>(size);
    let a = texel(t, b, s);
    let bx = texel(t, b + vec2<i32>(1, 0), s);
    let by = texel(t, b + vec2<i32>(0, 1), s);
    let bxy = texel(t, b + vec2<i32>(1, 1), s);
    return mix(mix(a, bx, f.x), mix(by, bxy, f.x), f.y);
}

fn pq_eotf(e: f32) -> f32 {
    let m1 = 2610.0 / 16384.0;
    let m2 = 2523.0 / 4096.0 * 128.0;
    let c1 = 3424.0 / 4096.0;
    let c2 = 2413.0 / 4096.0 * 32.0;
    let c3 = 2392.0 / 4096.0 * 32.0;
    let q = pow(clamp(e, 0.0, 1.0), 1.0 / m2);
    return 10000.0 * pow(max(q - c1, 0.0) / (c2 - c3 * q), 1.0 / m1);
}

fn hlg_to_nits(e0: f32) -> f32 {
    let a = 0.17883277;
    let b = 0.28466892;
    let c = 0.5599107;
    let e = clamp(e0, 0.0, 1.0);
    var scene: f32;
    if e <= 0.5 {
        scene = e * e / 3.0;
    } else {
        scene = (exp((e - c) / a) + b) / 12.0;
    }
    return 1000.0 * pow(scene, 1.2);
}

fn srgb_oetf(v0: f32) -> f32 {
    let v = clamp(v0, 0.0, 1.0);
    if v <= 0.0031308 {
        return v * 12.92;
    }
    return 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

fn srgb_eotf(v: f32) -> f32 {
    if v <= 0.04045 {
        return v / 12.92;
    }
    return pow((v + 0.055) / 1.055, 2.4);
}

fn tone_map(c0: vec3<f32>, peak: f32) -> vec3<f32> {
    let c = max(c0, vec3<f32>(0.0));
    let m = max(c.r, max(c.g, c.b));
    if m <= 1e-6 {
        return vec3<f32>(0.0);
    }
    let w = max(peak, 1.0);
    let mapped = m * (1.0 + m / (w * w)) / (1.0 + m);
    return c * (mapped / m);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = source_uv(in.uv);
    let y = bilinear(tex_y, uv, p.size).x;
    var cb: f32;
    var cr: f32;
    if p.nv12 == 1u {
        let c = bilinear(tex_u, uv, p.csize);
        cb = c.x;
        cr = c.y;
    } else {
        cb = bilinear(tex_u, uv, p.csize).x;
        cr = bilinear(tex_v, uv, p.csize).x;
    }
    let code = vec4<f32>(vec3<f32>(y, cb, cr) * p.scale, 1.0);
    var rgb = vec3<f32>(dot(p.m0, code), dot(p.m1, code), dot(p.m2, code));
    if p.transfer != 0u {
        var lin: vec3<f32>;
        if p.transfer == 1u {
            lin = vec3<f32>(pq_eotf(rgb.r), pq_eotf(rgb.g), pq_eotf(rgb.b)) / p.white;
        } else {
            lin = vec3<f32>(hlg_to_nits(rgb.r), hlg_to_nits(rgb.g), hlg_to_nits(rgb.b)) / p.white;
        }
        if p.gamut == 1u {
            lin = vec3<f32>(
                1.6605 * lin.r - 0.5876 * lin.g - 0.0728 * lin.b,
                -0.1246 * lin.r + 1.1329 * lin.g - 0.0083 * lin.b,
                -0.0182 * lin.r - 0.1006 * lin.g + 1.1187 * lin.b,
            );
        }
        let m = tone_map(lin, p.peak);
        rgb = vec3<f32>(srgb_oetf(m.r), srgb_oetf(m.g), srgb_oetf(m.b));
    }
    rgb = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    if p.linear_out == 1u {
        rgb = vec3<f32>(srgb_eotf(rgb.r), srgb_eotf(rgb.g), srgb_eotf(rgb.b));
    }
    return vec4<f32>(rgb, 1.0);
}
