struct RectangleInput {
    @location(0) rect: vec4<f32>,
    @location(1) color: vec4<f32>,
}

struct RectangleOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(input: RectangleInput, @builtin(vertex_index) index: u32) -> RectangleOutput {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    let pixel = input.rect.xy + corners[index] * input.rect.zw;
    var output: RectangleOutput;
    output.position = vec4<f32>(pixel.x * 2.0 - 1.0, 1.0 - pixel.y * 2.0, 0.0, 1.0);
    output.color = input.color;
    return output;
}

@fragment
fn fs_main(input: RectangleOutput) -> @location(0) vec4<f32> {
    return input.color;
}
