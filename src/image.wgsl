// CLIP_WGSL is prepended by gpu.rs. All geometry is in window device pixels.
struct Uniforms {
    rect: vec4<f32>,
    viewport: vec4<f32>, // layer origin, target size
    image: vec4<f32>, // image size, window, level
    options: vec4<f32>, // inversion, sRGB blending, horizontal flip, clockwise rotation in radians
    clip: vec4<f32>,
    fade: vec4<u32>,
    slot: vec4<f32>,
    radii: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var image: texture_2d<f32>;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex fn vs(@builtin(vertex_index) index: u32) -> Vertex {
    let corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0));
    let uv = corners[index];
    let local = (uv - vec2(0.5)) * u.rect.zw;
    let cosine = cos(u.options.w);
    let sine = sin(u.options.w);
    var rotated = vec2(cosine * local.x - sine * local.y, sine * local.x + cosine * local.y);
    // Mirror in display space, so this stays left/right after a quarter turn.
    if u.options.z > 0.5 { rotated.x = -rotated.x; }
    let center = u.rect.xy + u.rect.zw * 0.5;
    let p = (center + rotated - u.viewport.xy) / u.viewport.zw;
    return Vertex(vec4(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0), uv);
}

fn load(p: vec2<i32>) -> f32 {
    return textureLoad(image, clamp(p, vec2<i32>(0), vec2<i32>(u.image.xy) - vec2<i32>(1)), 0).r;
}
fn scalar(uv: vec2<f32>) -> f32 {
    let p = uv * u.image.xy - vec2(0.5);
    let base = vec2<i32>(floor(p));
    let f = fract(p);
    return mix(mix(load(base), load(base + vec2(1, 0)), f.x),
               mix(load(base + vec2(0, 1)), load(base + vec2(1, 1)), f.x), f.y);
}
fn linear_from_srgb(v: f32) -> f32 {
    if v <= 0.04045 { return v / 12.92; }
    return pow((v + 0.055) / 1.055, 2.4);
}
@fragment fn fs(v: Vertex) -> @location(0) vec4<f32> {
    let value = scalar(v.uv);
    let center = u.image.w - 0.5;
    let width = u.image.z - 1.0;
    var gray: f32;
    // Branching before division is essential for DICOM's width-one threshold.
    if value <= center - width * 0.5 { gray = 0.0; }
    else if value > center + width * 0.5 { gray = 1.0; }
    else { gray = clamp((value - center) / width + 0.5, 0.0, 1.0); }
    if u.options.x > 0.5 { gray = 1.0 - gray; }
    // W/L produces display-encoded gray, like Pixels RGBA. Linear-blended targets encode on
    // output (or later layer composition), so decode once before blending in those targets.
    if u.options.y < 0.5 { gray = linear_from_srgb(gray); }
    let p = v.position.xy + u.viewport.xy;
    let half = u.slot.zw * 0.5;
    let coverage = clip_alpha(p, u.clip, u.fade)
        * clamp(0.5 - sd_rounded(p - u.slot.xy - half, half, u.radii), 0.0, 1.0);
    return vec4(vec3(gray * coverage), coverage);
}
