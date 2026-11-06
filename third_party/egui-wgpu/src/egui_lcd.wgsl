
// Soyuz: per-channel blending, appended to egui.wgsl, used for all meshes.
//
// Texels with zero alpha hold LCD coverage in rgb (only the font atlas has them: a
// premultiplied image with no alpha has no colour either). Every other texel gives exactly
// the result of the normal pipeline: src1 is then the same value in all channels, and
// `src0 + dst * (1 - src1)` is premultiplied alpha blending.

struct LcdOutput {
    @location(0) @blend_src(0) color: vec4<f32>,
    @location(0) @blend_src(1) mask: vec4<f32>,
};

fn lcd_blend(in: VertexOutput) -> LcdOutput {
    let tex = sample_texture(in);
    var out: LcdOutput;
    if tex.a < 0.5 / 255.0 {
        let coverage = max(tex.r, max(tex.g, tex.b));
        out.color = vec4<f32>(in.color.rgb * tex.rgb, in.color.a * coverage);
        out.mask = vec4<f32>(in.color.a * tex.rgb, in.color.a * coverage);
    } else {
        out.color = in.color * tex;
        out.mask = vec4<f32>(in.color.a * tex.a);
    }
    if r_locals.dithering == 1 {
        out.color = vec4<f32>(dither_interleaved(out.color.rgb, 256.0, in.position), out.color.a);
    }
    return out;
}

@fragment
fn fs_lcd_gamma_framebuffer(in: VertexOutput) -> LcdOutput {
    return lcd_blend(in);
}

@fragment
fn fs_lcd_linear_framebuffer(in: VertexOutput) -> LcdOutput {
    var out = lcd_blend(in);
    out.color = vec4<f32>(linear_from_gamma_rgb(out.color.rgb), out.color.a);
    return out;
}
