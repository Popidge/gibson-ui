@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@group(1) @binding(0)
var label_texture: texture_2d<f32>;

@group(1) @binding(1)
var label_sampler: sampler;

struct LabelInstance {
    @location(0) center_width: vec4<f32>,
    @location(1) right_height: vec4<f32>,
    @location(2) uv_rect: vec4<f32>,
    @location(3) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

const QUAD_POSITIONS = array<vec2<f32>, 6>(
    vec2<f32>(-0.5, -0.5),
    vec2<f32>( 0.5, -0.5),
    vec2<f32>( 0.5,  0.5),
    vec2<f32>(-0.5, -0.5),
    vec2<f32>( 0.5,  0.5),
    vec2<f32>(-0.5,  0.5),
);

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, instance: LabelInstance) -> VertexOutput {
    let corner = QUAD_POSITIONS[vertex_index];
    let center = instance.center_width.xyz;
    let right = instance.right_height.xyz;
    let world = center
        + right * (corner.x * instance.center_width.w)
        + vec3<f32>(0.0, corner.y * instance.right_height.w, 0.0);
    let uv_min = instance.uv_rect.xy;
    let uv_max = instance.uv_rect.zw;
    let uv_x = mix(uv_min.x, uv_max.x, corner.x + 0.5);
    let uv_y = mix(uv_max.y, uv_min.y, corner.y + 0.5);

    var output: VertexOutput;
    output.clip_position = uniforms.view_proj * vec4<f32>(world, 1.0);
    output.uv = vec2<f32>(uv_x, uv_y);
    output.color = instance.color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let texture_size = vec2<f32>(textureDimensions(label_texture));
    let texel = 1.0 / texture_size;
    let fill = textureSample(label_texture, label_sampler, input.uv).a;
    var surround = fill;
    surround = max(surround, textureSample(label_texture, label_sampler, input.uv + vec2<f32>( texel.x, 0.0)).a);
    surround = max(surround, textureSample(label_texture, label_sampler, input.uv + vec2<f32>(-texel.x, 0.0)).a);
    surround = max(surround, textureSample(label_texture, label_sampler, input.uv + vec2<f32>(0.0,  texel.y)).a);
    surround = max(surround, textureSample(label_texture, label_sampler, input.uv + vec2<f32>(0.0, -texel.y)).a);
    let outline = max(surround - fill, 0.0);
    let alpha = clamp(fill + outline * 0.92, 0.0, 1.0) * input.color.a;
    if alpha < 0.01 {
        discard;
    }
    let rgb = input.color.rgb;
    return vec4<f32>(rgb, alpha);
}
