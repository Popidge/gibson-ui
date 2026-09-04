struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

struct OverlayUniforms {
    visualiser: vec4<f32>,
    terminal: vec4<f32>,
    metadata: vec4<f32>,
    settings: vec4<f32>,
    hud: vec4<f32>,
    terminal_hud: vec4<f32>,
    terminal_focus: vec4<f32>,
    primary: vec4<f32>,
    secondary: vec4<f32>,
    panel_background: vec4<f32>,
    options: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> overlay: OverlayUniforms;

fn inside(point: vec2<f32>, rect: vec4<f32>) -> bool {
    return rect.z > 0.0
        && rect.w > 0.0
        && point.x >= rect.x
        && point.x <= rect.x + rect.z
        && point.y >= rect.y
        && point.y <= rect.y + rect.w;
}

fn rect_edge(point: vec2<f32>, rect: vec4<f32>) -> f32 {
    let distance = min(
        min(point.x - rect.x, rect.x + rect.z - point.x),
        min(point.y - rect.y, rect.y + rect.w - point.y),
    );
    return 1.0 - smoothstep(0.0, 0.004, distance);
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
    if inside(input.uv, overlay.hud) {
        let scan = overlay.options.x * 0.006 * sin(input.uv.y * 1150.0);
        return vec4<f32>(
            overlay.panel_background.rgb * 0.34 + vec3<f32>(scan),
            0.82,
        );
    }

    if inside(input.uv, overlay.terminal_hud) {
        let border = rect_edge(input.uv, overlay.terminal_hud);
        let focus = overlay.terminal_focus.x;
        let scan = overlay.options.x * 0.006 * sin(input.uv.y * 1150.0);
        return vec4<f32>(
            overlay.panel_background.rgb * 0.34
                + overlay.primary.rgb * border * 0.08
                + overlay.secondary.rgb * focus * 0.06
                + vec3<f32>(scan),
            0.86,
        );
    }

    if inside(input.uv, overlay.settings) {
        let border = rect_edge(input.uv, overlay.settings);
        let scan = overlay.options.x * 0.010 * sin(input.uv.y * 1150.0);
        return vec4<f32>(
            overlay.panel_background.rgb * 0.28
                + overlay.primary.rgb * border * 0.34
                + vec3<f32>(scan),
            0.985,
        );
    }

    if inside(input.uv, overlay.metadata) {
        let border = rect_edge(input.uv, overlay.metadata);
        let scan = overlay.options.x * 0.010 * sin(input.uv.y * 1050.0);
        return vec4<f32>(
            overlay.panel_background.rgb * 0.24
                + overlay.secondary.rgb * border * 0.25
                + vec3<f32>(scan),
            0.96,
        );
    }

    if inside(input.uv, overlay.terminal) {
        let border = rect_edge(input.uv, overlay.terminal);
        let focused = overlay.terminal_focus.x;
        let amber = focused * border;
        let scan = overlay.options.x * 0.012 * sin(input.uv.y * 1200.0);
        return vec4<f32>(
            overlay.panel_background.rgb * 0.34
                + overlay.primary.rgb * border * 0.08
                + overlay.secondary.rgb * amber * 0.26
                + vec3<f32>(scan),
            0.96,
        );
    }

    if inside(input.uv, overlay.visualiser) {
        let border = rect_edge(input.uv, overlay.visualiser);
        if border > 0.01 {
            return vec4<f32>(overlay.primary.rgb * 0.42, border * 0.70);
        }
    }

    discard;
}
