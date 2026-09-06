@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(1) @binding(0) var bloom: texture_2d<f32>;
@group(1) @binding(1) var bloom_sampler: sampler;
@group(2) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[index];
    var output: VertexOutput;
    output.position = vec4<f32>(position, 0.0, 1.0);
    output.uv = vec2<f32>((position.x + 1.0) * 0.5, 1.0 - (position.y + 1.0) * 0.5);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let base = textureSample(scene, scene_sampler, input.uv);
    let glow = textureSample(bloom, bloom_sampler, input.uv).rgb;
    let bounds = uniforms.visual_bounds;
    let inside = all(input.uv >= bounds.xy) && all(input.uv <= bounds.xy + bounds.zw);
    // Soft headroom compression keeps bright edges coloured instead of clipping white.
    let contribution = glow * 0.65;
    let color = base.rgb + contribution * (vec3<f32>(1.0) - clamp(base.rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
    return vec4<f32>(select(base.rgb, color, inside), base.a);
}
