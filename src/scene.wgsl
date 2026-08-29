struct Uniforms {
    view_proj: mat4x4<f32>,
    camera_time: vec4<f32>,
    pulse: vec4<f32>,
    primary: vec4<f32>,
    secondary: vec4<f32>,
    accent: vec4<f32>,
    background: vec4<f32>,
    active_face: vec4<f32>,
    render_size: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@group(1) @binding(0)
var navigator_texture: texture_2d<f32>;

@group(1) @binding(1)
var navigator_sampler: sampler;

@group(2) @binding(0)
var blurred_scene_texture: texture_2d<f32>;

@group(2) @binding(1)
var blurred_scene_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) model_0: vec4<f32>,
    @location(3) model_1: vec4<f32>,
    @location(4) model_2: vec4<f32>,
    @location(5) model_3: vec4<f32>,
    @location(6) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) local_position: vec3<f32>,
    @location(4) local_normal: vec3<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let model = mat4x4<f32>(input.model_0, input.model_1, input.model_2, input.model_3);
    let world = model * vec4<f32>(input.position, 1.0);
    var output: VertexOutput;
    output.clip_position = uniforms.view_proj * world;
    output.world_position = world.xyz;
    output.world_normal = normalize((model * vec4<f32>(input.normal, 0.0)).xyz);
    output.color = input.color;
    output.local_position = input.position;
    output.local_normal = input.normal;
    return output;
}

fn hash21(value: vec2<f32>) -> f32 {
    return fract(sin(dot(value, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn pow18(value: f32) -> f32 {
    let squared = value * value;
    let fourth = squared * squared;
    let eighth = fourth * fourth;
    let sixteenth = eighth * eighth;
    return sixteenth * squared;
}

fn pow34(value: f32) -> f32 {
    let squared = value * value;
    let fourth = squared * squared;
    let eighth = fourth * fourth;
    let sixteenth = eighth * eighth;
    let thirty_second = sixteenth * sixteenth;
    return thirty_second * squared;
}

fn pow38(value: f32) -> f32 {
    let squared = value * value;
    let fourth = squared * squared;
    let eighth = fourth * fourth;
    let sixteenth = eighth * eighth;
    let thirty_second = sixteenth * sixteenth;
    return thirty_second * fourth * squared;
}

fn line_mask(distance_from_line: f32, width: f32) -> f32 {
    return 1.0 - smoothstep(
        width,
        width + max(fwidth(distance_from_line) * 1.5, 0.0015),
        distance_from_line,
    );
}

fn blurred_scene(position: vec2<f32>) -> vec3<f32> {
    let uv = clamp(position * uniforms.render_size.zw, vec2<f32>(0.0), vec2<f32>(1.0));
    return textureSample(blurred_scene_texture, blurred_scene_sampler, uv).rgb;
}

fn movie_circuit_floor(position: vec2<f32>) -> vec4<f32> {
    let cell_size = 7.2;
    let cell = floor(position / cell_size);
    let local = fract(position / cell_size) - 0.5;
    let seed = hash21(cell);
    let cross_seed = hash21(cell + vec2<f32>(43.0, 91.0));
    let x_offset = (seed - 0.5) * 0.52;
    let y_offset = (cross_seed - 0.5) * 0.52;
    let vertical = line_mask(abs(local.x - x_offset), 0.010)
        * (1.0 - smoothstep(0.34, 0.48, abs(local.y)));
    let horizontal = line_mask(abs(local.y - y_offset), 0.010)
        * (1.0 - smoothstep(0.30, 0.48, abs(local.x)));
    let elbow = max(vertical, horizontal);

    let macro_x = abs(fract(position.x / 21.6 + 0.5) - 0.5);
    let macro_y = abs(fract(position.y / 21.6 + 0.5) - 0.5);
    let macro_trace = max(line_mask(macro_x, 0.0055), line_mask(macro_y, 0.0055));

    let chip_distance = abs(max(abs(local.x), abs(local.y)) - (0.21 + seed * 0.055));
    let chip_outline = line_mask(chip_distance, 0.010) * step(0.66, cross_seed);
    let node_position = vec2<f32>(x_offset, y_offset);
    let node_distance = length(local - node_position);
    let node = line_mask(abs(node_distance - 0.034), 0.010);
    let pin_lane = line_mask(abs(abs(local.x) - 0.31), 0.009)
        * step(abs(local.y), 0.19)
        * step(0.54, seed);

    let trace = max(max(elbow, chip_outline), pin_lane);
    let pulse = pow34(max(sin(
        (position.x + position.y) * 0.44
            - uniforms.camera_time.w * (2.4 + seed * 1.8)
            + seed * 6.283,
    ), 0.0)) * trace * uniforms.pulse.z;
    let distance_to_camera = distance(position, uniforms.camera_time.xz);
    let fade = 1.0 - smoothstep(62.0, 108.0, distance_to_camera);
    let board = uniforms.background.rgb * 0.32;
    let cyan = uniforms.primary.rgb * (trace * 0.78 + node * 1.25 + pulse * 1.7);
    let magenta = uniforms.secondary.rgb * macro_trace * 0.58;
    return vec4<f32>(board + (cyan + magenta) * fade, 1.0);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if input.color.a < 0.1 {
        if uniforms.pulse.w > 0.5 {
            return movie_circuit_floor(input.world_position.xz);
        }
        let major_cell = 5.4;
        let minor_cell = 1.35;
        let major_x = abs(fract(input.world_position.x / major_cell + 0.5) - 0.5);
        let major_z = abs(fract(input.world_position.z / major_cell + 0.5) - 0.5);
        let minor_x = abs(fract(input.world_position.x / minor_cell + 0.5) - 0.5);
        let minor_z = abs(fract(input.world_position.z / minor_cell + 0.5) - 0.5);
        let major_line_x = 1.0 - smoothstep(
            0.008,
            0.008 + max(fwidth(major_x) * 1.5, 0.002),
            major_x,
        );
        let major_line_z = 1.0 - smoothstep(
            0.008,
            0.008 + max(fwidth(major_z) * 1.5, 0.002),
            major_z,
        );
        let minor_line_x = 1.0 - smoothstep(
            0.004,
            0.004 + max(fwidth(minor_x) * 1.25, 0.002),
            minor_x,
        );
        let minor_line_z = 1.0 - smoothstep(
            0.004,
            0.004 + max(fwidth(minor_z) * 1.25, 0.002),
            minor_z,
        );
        let floor_distance = distance(input.world_position.xz, uniforms.camera_time.xz);
        let major_fade = 1.0 - smoothstep(58.0, 92.0, floor_distance);
        let minor_fade = 1.0 - smoothstep(20.0, 52.0, floor_distance);
        let major = max(major_line_x, major_line_z) * major_fade;
        let minor = max(minor_line_x, minor_line_z) * minor_fade;
        let pulse_epoch = floor(uniforms.camera_time.w * 0.24);
        let z_lane = floor(input.world_position.z / major_cell + 0.5);
        let x_lane = floor(input.world_position.x / major_cell + 0.5);
        let z_seed = hash21(vec2<f32>(z_lane, pulse_epoch));
        let x_seed = hash21(vec2<f32>(x_lane + 913.0, pulse_epoch + 47.0));
        let z_gate = step(0.82, z_seed);
        let x_gate = step(0.86, x_seed);
        let along_x = pow34(max(sin(
            input.world_position.x * 0.42
                - uniforms.camera_time.w * (2.1 + z_seed * 1.8)
                + z_seed * 6.283,
        ), 0.0)) * z_gate * major_line_z * major_fade;
        let along_z = pow38(max(sin(
            input.world_position.z * 0.42
                + uniforms.camera_time.w * (1.9 + x_seed * 1.7)
                + x_seed * 6.283,
        ), 0.0)) * x_gate * major_line_x * major_fade;
        let sporadic = (along_x + along_z) * uniforms.pulse.z;
        let flight_x = pow38(max(sin(
            input.world_position.x * 0.34 - uniforms.camera_time.w * 5.8,
        ), 0.0)) * major_line_z;
        let flight_z = pow38(max(sin(
            input.world_position.z * 0.34 + uniforms.camera_time.w * 5.2,
        ), 0.0)) * major_line_x;
        let rush = (flight_x + flight_z) * uniforms.pulse.y * uniforms.pulse.z * major_fade;
        let cyan = uniforms.primary.rgb * (major * 0.72 + rush * 1.10);
        let spark = uniforms.accent.rgb * sporadic * 1.55;
        let secondary = uniforms.secondary.rgb * (minor * 0.24);
        return vec4<f32>(uniforms.background.rgb * 0.05 + cyan + spark + secondary, 1.0);
    }

    let movie_style = uniforms.pulse.w > 0.5;
    let current_tower = select(
        abs(input.color.a - 1.55) < 0.01,
        abs(input.color.a - 0.22) < 0.01,
        movie_style,
    );
    let active_normal = dot(input.world_normal, uniforms.active_face.xyz) > 0.98;
    if uniforms.active_face.w > 0.5 && current_tower && active_normal {
        let horizontal = vec3<f32>(-uniforms.active_face.z, 0.0, uniforms.active_face.x);
        let uv = vec2<f32>(
            0.5 - dot(input.local_position, horizontal),
            0.5 - input.local_position.y,
        );
        let navigator = textureSample(navigator_texture, navigator_sampler, uv);
        let edge_distance = 0.5 - max(abs(input.local_position.x), abs(input.local_position.y));
        let edge = 1.0 - smoothstep(
            0.010,
            0.010 + max(fwidth(edge_distance) * 1.6, 0.016),
            edge_distance,
        );
        let classic_face = uniforms.background.rgb * 0.24;
        var base = classic_face;
        if movie_style {
            base = blurred_scene(input.clip_position.xy) * 0.34
                + uniforms.background.rgb * 0.04
                + uniforms.primary.rgb * 0.035;
        }
        base += uniforms.primary.rgb * edge * 0.18;
        let color = base * (1.0 - navigator.a) + navigator.rgb * 1.65;
        return vec4<f32>(color, select(1.0, 0.97, movie_style));
    }

    if uniforms.pulse.w > 0.5 && input.color.a < 0.3 {
        let normal = abs(input.local_normal);
        var first = abs(input.local_position.x);
        var second = abs(input.local_position.y);
        if normal.x > 0.5 {
            first = abs(input.local_position.y);
            second = abs(input.local_position.z);
        } else if normal.y > 0.5 {
            first = abs(input.local_position.x);
            second = abs(input.local_position.z);
        }
        let edge_distance = 0.5 - max(first, second);
        let edge = 1.0 - smoothstep(
            0.006,
            0.006 + max(fwidth(edge_distance) * 1.4, 0.014),
            edge_distance,
        );
        let view_direction = normalize(uniforms.camera_time.xyz - input.world_position);
        let rim = pow(1.0 - abs(dot(view_direction, input.world_normal)), 2.0);
        let glass = mix(input.color.rgb, uniforms.primary.rgb, 0.46);
        let transmission = blurred_scene(input.clip_position.xy) * 0.78 + glass * 0.30;
        let face_light = transmission * (0.82 + rim * 0.24);
        let edge_light = uniforms.primary.rgb * 0.82 + uniforms.accent.rgb * 0.13;
        let color = face_light + edge_light * edge;
        let alpha = 0.78 + rim * 0.06 + edge * 0.15;
        return vec4<f32>(color, alpha);
    }
    let light_direction = normalize(vec3<f32>(0.35, 0.85, 0.42));
    let diffuse = max(dot(input.world_normal, light_direction), 0.0);
    let view_direction = normalize(uniforms.camera_time.xyz - input.world_position);
    let rim = pow(1.0 - max(dot(view_direction, input.world_normal), 0.0), 2.2);
    let command_wave = sin(length(input.world_position.xz) * 1.8 - uniforms.camera_time.w * 8.0);
    let command_light = max(command_wave, 0.0) * uniforms.pulse.x * 0.22;
    let flight_wave = pow18(max(sin((input.world_position.x - input.world_position.z) * 1.1 - uniforms.camera_time.w * 18.0), 0.0));
    let flight_light = flight_wave * uniforms.pulse.y * 0.38;
    let lit = input.color.rgb * (0.18 + diffuse * 0.45);
    let emissive = input.color.rgb * (input.color.a * 0.42 + rim * 0.85 + command_light + flight_light);
    let distance_to_camera = distance(input.world_position, uniforms.camera_time.xyz);
    let fog = smoothstep(18.0, 75.0, distance_to_camera);
    let themed_emissive = mix(emissive, emissive * uniforms.primary.rgb * 1.65, 0.24);
    let final_color = mix(lit + themed_emissive, uniforms.background.rgb * 0.07, fog);
    return vec4<f32>(final_color, 1.0);
}
