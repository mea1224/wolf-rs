struct Camera { origin: vec2<f32>, viewport: vec2<f32> }
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var atlas: texture_2d_array<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

struct Input {
    @location(0) origin: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv_min: vec2<f32>,
    @location(3) uv_max: vec2<f32>,
    @location(4) tint: vec4<f32>,
    @location(5) page: u32,
    @location(6) flags: u32,
}
struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) @interpolate(flat) page: u32,
}
@vertex
fn vs_main(input: Input, @builtin(vertex_index) vertex: u32) -> Varyings {
    let corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0));
    let local = corners[vertex];
    var sample_local = local;
    if ((input.flags & 1u) != 0u) { sample_local.x = 1.0 - sample_local.x; }
    if ((input.flags & 2u) != 0u) { sample_local.y = 1.0 - sample_local.y; }
    let screen = (input.origin + local * input.size - camera.origin) / camera.viewport;
    var out: Varyings;
    out.position = vec4(screen.x * 2.0 - 1.0, 1.0 - screen.y * 2.0, 0.0, 1.0);
    out.uv = mix(input.uv_min, input.uv_max, sample_local);
    out.tint = input.tint;
    out.page = input.page;
    return out;
}
@fragment
fn fs_main(input: Varyings) -> @location(0) vec4<f32> {
    let straight = textureSample(atlas, atlas_sampler, input.uv, i32(input.page)) * input.tint;
    return vec4(straight.rgb * straight.a, straight.a);
}
