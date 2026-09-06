@group(0) @binding(0)
var source_texture: texture_2d<f32>;

@group(0) @binding(1)
var source_sampler: sampler;

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

fn gaussian_blur(uv: vec2<f32>, direction: vec2<f32>) -> vec4<f32> {
    let dimensions = vec2<f32>(textureDimensions(source_texture));
    let texel = direction / dimensions * 1.65;
    var color = textureSample(source_texture, source_sampler, uv) * 0.227027;
    color += textureSample(source_texture, source_sampler, uv + texel * 1.384615) * 0.316216;
    color += textureSample(source_texture, source_sampler, uv - texel * 1.384615) * 0.316216;
    color += textureSample(source_texture, source_sampler, uv + texel * 3.230769) * 0.070270;
    color += textureSample(source_texture, source_sampler, uv - texel * 3.230769) * 0.070270;
    return color;
}

@fragment
fn fs_copy(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(source_texture, source_sampler, input.uv);
}

@fragment
fn fs_horizontal(input: VertexOutput) -> @location(0) vec4<f32> {
    // Match the vertical radius while this pass downsamples the full-resolution scene.
    return gaussian_blur(input.uv, vec2<f32>(2.0, 0.0));
}

@fragment
fn fs_vertical(input: VertexOutput) -> @location(0) vec4<f32> {
    return gaussian_blur(input.uv, vec2<f32>(0.0, 1.0));
}

// Dense taps avoid repeated silhouettes around fine emissive edges. Horizontal
// sampling also downsamples 2:1; vertical sampling uses the half-size result.
fn bloom_blur(uv: vec2<f32>, direction: vec2<f32>) -> vec4<f32> {
    let texel = direction / vec2<f32>(textureDimensions(source_texture));
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = -8; i <= 8; i += 1) {
        let weight = exp(-f32(i * i) / 18.0);
        sum += textureSample(source_texture, source_sampler, uv + texel * f32(i)) * weight;
        total += weight;
    }
    return sum / total;
}
// Pair adjacent texels through bilinear filtering. This is the same 17-tap
// Gaussian at unit spacing, with nine texture reads. Do not use it for the
// horizontal downsample or the fractional reflection radius.
fn unit_blur(uv: vec2<f32>, direction: vec2<f32>) -> vec4<f32> {
    let texel = direction / vec2<f32>(textureDimensions(source_texture));
    var color = textureSample(source_texture, source_sampler, uv) * 0.1335712203;
    let offsets = array<f32, 4>(1.458429517, 3.403984807, 5.351805780, 7.302940716);
    let weights = array<f32, 4>(0.233308433, 0.135927811, 0.051383178, 0.012594969);
    for (var i = 0; i < 4; i += 1) {
        color += (textureSample(source_texture, source_sampler, uv + texel * offsets[i])
                + textureSample(source_texture, source_sampler, uv - texel * offsets[i])) * weights[i];
    }
    return color;
}

@fragment
fn fs_bloom_horizontal(input: VertexOutput) -> @location(0) vec4<f32> {
    return bloom_blur(input.uv, vec2<f32>(2.0, 0.0));
}
@fragment
fn fs_bloom_vertical(input: VertexOutput) -> @location(0) vec4<f32> {
    return unit_blur(input.uv, vec2<f32>(0.0, 1.0));
}

// Reflection roughness stretches softly down the polished floor.
@fragment
fn fs_reflection_horizontal(input: VertexOutput) -> @location(0) vec4<f32> {
    return bloom_blur(input.uv, vec2<f32>(0.65, 0.0));
}
@fragment
fn fs_reflection_vertical(input: VertexOutput) -> @location(0) vec4<f32> {
    return unit_blur(input.uv, vec2<f32>(0.0, 1.0));
}
