struct Globals {
    screen: vec2<f32>,
    // 1 when the target is an sRGB format and colors must be linearized.
    srgb: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) kind: u32,
}

// Triangle-strip corners: (0,0) (1,0) (0,1) (1,1).
fn corner(index: u32) -> vec2<f32> {
    return vec2<f32>(f32(index & 1u), f32(index >> 1u));
}

fn to_clip(px: vec2<f32>) -> vec4<f32> {
    let ndc = px / globals.screen * 2.0 - 1.0;
    return vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
}

fn linearize(c: vec3<f32>) -> vec3<f32> {
    if globals.srgb == 0u {
        return c;
    }
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@vertex
fn vs_rect(
    @builtin(vertex_index) index: u32,
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) color: vec4<f32>,
) -> VertexOut {
    var out: VertexOut;
    out.clip = to_clip(pos + corner(index) * size);
    out.color = vec4<f32>(linearize(color.rgb), color.a);
    out.uv = vec2<f32>(0.0);
    out.kind = 0u;
    return out;
}

@fragment
fn fs_rect(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color.rgb * in.color.a, in.color.a);
}

@vertex
fn vs_glyph(
    @builtin(vertex_index) index: u32,
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv_pos: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) kind: u32,
) -> VertexOut {
    let c = corner(index);
    var out: VertexOut;
    out.clip = to_clip(pos + c * size);
    out.color = vec4<f32>(linearize(color.rgb), color.a);
    out.uv = (uv_pos + c * size) / vec2<f32>(textureDimensions(atlas));
    out.kind = kind;
    return out;
}

@fragment
fn fs_glyph(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = textureSample(atlas, atlas_sampler, in.uv);
    if in.kind == 1u {
        // Color glyph, premultiplied in the atlas.
        if texel.a <= 0.0 {
            return vec4<f32>(0.0);
        }
        return vec4<f32>(linearize(texel.rgb / texel.a) * texel.a, texel.a);
    }
    let a = texel.a * in.color.a;
    return vec4<f32>(in.color.rgb * a, a);
}

// Images: one texture per draw call, in bind group 1.
@group(1) @binding(0) var image_texture: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;

struct ImageOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_image(
    @builtin(vertex_index) index: u32,
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv: vec4<f32>,
) -> ImageOut {
    let c = corner(index);
    var out: ImageOut;
    out.clip = to_clip(pos + c * size);
    out.uv = mix(uv.xy, uv.zw, c);
    return out;
}

@fragment
fn fs_image(in: ImageOut) -> @location(0) vec4<f32> {
    // Straight alpha in the texture; the blend state expects premultiplied.
    let texel = textureSample(image_texture, image_sampler, in.uv);
    return vec4<f32>(linearize(texel.rgb) * texel.a, texel.a);
}

// The background image, mixed into the frame background color here rather
// than by the blend state: the blend unit mixes in linear space on sRGB
// targets, which makes a low opacity look far stronger than it does on a
// linear target or in CSS.
struct BackgroundOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_background(
    @builtin(vertex_index) index: u32,
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv: vec4<f32>,
    @location(3) color: vec4<f32>,
) -> BackgroundOut {
    let c = corner(index);
    var out: BackgroundOut;
    out.clip = to_clip(pos + c * size);
    out.uv = mix(uv.xy, uv.zw, c);
    out.color = color;
    return out;
}

@fragment
fn fs_background(in: BackgroundOut) -> @location(0) vec4<f32> {
    let texel = textureSample(image_texture, image_sampler, in.uv);
    let c = mix(in.color.rgb, texel.rgb, texel.a * in.color.a);
    return vec4<f32>(linearize(c), 1.0);
}
