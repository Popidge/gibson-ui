struct BackgroundUniforms {
    background: vec4<f32>,
    options: vec4<f32>,
}

@group(0) @binding(0)
var background_texture: texture_2d<f32>;

@group(0) @binding(1)
var background_sampler: sampler;

@group(0) @binding(2)
var<uniform> uniforms: BackgroundUniforms;

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
    output.position = vec4<f32>(position, 1.0, 1.0);
    output.uv = vec2<f32>((position.x + 1.0) * 0.5, 1.0 - (position.y + 1.0) * 0.5);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let mode = uniforms.options.x;
    if mode < 0.5 {
        return vec4<f32>(0.001, 0.003, 0.011, 1.0);
    }
    if mode < 1.5 || uniforms.options.w < 0.5 {
        return vec4<f32>(uniforms.background.rgb * 0.18, 1.0);
    }

    let viewport_aspect = uniforms.options.y;
    let image_aspect = uniforms.options.z;
    var uv = input.uv;
    if image_aspect > viewport_aspect {
        let visible = viewport_aspect / image_aspect;
        uv.x = (uv.x - 0.5) * visible + 0.5;
    } else {
        let visible = image_aspect / viewport_aspect;
        uv.y = (uv.y - 0.5) * visible + 0.5;
    }
    let image = textureSample(background_texture, background_sampler, uv).rgb;
    let theme_tint = vec3<f32>(0.72) + uniforms.background.rgb * 0.55;
    let tinted = mix(image, image * theme_tint, 0.16);
    return vec4<f32>(tinted * 0.58, 1.0);
}
