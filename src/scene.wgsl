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

@group(3) @binding(0)
var reflection_texture: texture_2d<f32>;
@group(3) @binding(1)
var reflection_sampler: sampler;

const FLOOR_HEIGHT: f32 = -0.41;

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
    @location(5) @interpolate(flat) center: vec3<f32>,
}

fn scene_vertex(input: VertexInput) -> VertexOutput {
    let model = mat4x4<f32>(input.model_0, input.model_1, input.model_2, input.model_3);
    let world = model * vec4<f32>(input.position, 1.0);
    var output: VertexOutput;
    output.clip_position = uniforms.view_proj * world;
    output.world_position = world.xyz;
    output.world_normal = normalize((model * vec4<f32>(input.normal, 0.0)).xyz);
    output.color = input.color;
    output.local_position = input.position;
    output.local_normal = input.normal;
    output.center = model[3].xyz;
    return output;
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    return scene_vertex(input);
}

@vertex
fn vs_reflection(input: VertexInput) -> VertexOutput {
    var output = scene_vertex(input);
    let reflected = vec3<f32>(output.world_position.x, 2.0 * FLOOR_HEIGHT - output.world_position.y, output.world_position.z);
    output.clip_position = uniforms.view_proj * vec4<f32>(reflected, 1.0);
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

fn surface_color(input: VertexOutput) -> vec4<f32> {
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

// Cinematic keeps light emission separate from scene colour. Text, wallpaper and
// transmitted background never enter the bloom buffer.
fn face_edge(input: VertexOutput) -> f32 {
    let n = abs(input.local_normal);
    var uv = input.local_position.xy;
    if n.x > 0.5 { uv = input.local_position.zy; }
    if n.y > 0.5 { uv = input.local_position.xz; }
    return line_mask(0.5 - max(abs(uv.x), abs(uv.y)), 0.005);
}

fn is_navigator(input: VertexOutput) -> bool {
    let current = select(abs(input.color.a - 1.55) < 0.01, abs(input.color.a - 0.22) < 0.01, uniforms.pulse.w > 0.5);
    return current && uniforms.active_face.w > 0.5 && dot(input.world_normal, uniforms.active_face.xyz) > 0.98;
}

fn arrival_light() -> f32 {
    let phase = uniforms.cinematic.y;
    return select(0.0, sin(clamp(phase, 0.0, 1.0) * 3.141593), phase >= 0.0);
}

fn journey_floor(position: vec2<f32>) -> vec3<f32> {
    let destination = uniforms.destination.xz;
    let radius = distance(position, destination);
    let phase = uniforms.cinematic.y;
    let ring = exp(-pow((radius - (0.8 + max(phase, 0.0) * 10.0)) / 0.28, 2.0)) * arrival_light();
    let base_pool = exp(-radius * radius / 9.0) * arrival_light() * 0.28;
    // A short luminous wake follows the same Bezier as the camera, projected
    // onto the floor. Sampling only the wake keeps the per-pixel work bounded.
    let progress = smoothstep(0.0, 1.0, uniforms.cinematic.z);
    var route_pulse = 0.0;
    for (var i = 0; i < 10; i = i + 1) {
        let t = clamp(progress + 0.055 - f32(i) * 0.018, 0.0, 1.0);
        let u = 1.0 - t;
        let point = u*u*u * uniforms.departure.xz
            + 3.0*u*u*t * uniforms.route_controls.xy
            + 3.0*u*t*t * uniforms.route_controls.zw + t*t*t * destination;
        let delta = position - point;
        route_pulse = max(route_pulse, exp(-dot(delta, delta) / 0.16) * (1.0 - f32(i) / 10.0));
    }
    route_pulse *= uniforms.pulse.y * uniforms.pulse.z;
    return uniforms.primary.rgb * (base_pool + ring * 1.4 * uniforms.pulse.z + route_pulse * 1.8);
}

fn glass_tint(input: VertexOutput) -> vec3<f32> {
    // Material variation must preserve the danger colour of unreadable towers.
    if abs(input.color.a - 0.24) < 0.01 { return input.color.rgb; }
    let seed = hash21(input.center.xz + vec2<f32>(19.0, 71.0));
    let tint = mix(uniforms.primary.rgb, uniforms.secondary.rgb, seed * 0.24);
    return tint * select(1.0, 0.72, abs(input.color.a - 0.14) < 0.01);
}

fn glass_detail(input: VertexOutput) -> vec3<f32> {
    let n = abs(input.local_normal);
    var uv = input.local_position.xy;
    var view = normalize(uniforms.camera_time.xyz - input.world_position).xy;
    if n.x > 0.5 {
        uv = input.local_position.zy;
        view = normalize(uniforms.camera_time.xyz - input.world_position).zy;
    }
    if n.y > 0.5 { uv = input.local_position.xz; }
    let seed = hash21(input.center.xz + vec2<f32>(19.0, 71.0));
    let etched = step(0.35, seed) * (1.0 - step(0.73, seed));
    // Two etched planes suggest internal depth without extra scene objects.
    let front = uv + view * 0.025;
    let back = uv + view * 0.075;
    let lane_a = line_mask(abs(fract(front.x * (4.0 + floor(seed * 6.0)) + 0.5) - 0.5), 0.012);
    let lane_b = line_mask(abs(fract(back.y * 13.0 + 0.5) - 0.5), 0.015);
    let border_fade = smoothstep(0.0, 0.10, 0.5 - max(abs(uv.x), abs(uv.y)));
    let distance_fade = 1.0 - smoothstep(12.0, 36.0, distance(input.world_position, uniforms.camera_time.xyz));
    let light = 0.7 + 0.3 * sin(input.world_position.y * 1.5 - uniforms.camera_time.w * 0.55 * uniforms.cinematic.w);
    let core = line_mask(abs(back.x - (seed - 0.5) * 0.35), 0.009);
    let slots = line_mask(abs(fract(back.y * 28.0) - 0.5), 0.06) * core;
    let detail = (lane_a * 0.055 + lane_b * 0.035) * (0.55 + etched * 1.8) + core * 0.08 + slots * 0.18;
    return glass_tint(input) * detail * border_fade * distance_fade * light;
}

fn lightning_illumination(position: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    var light = vec3<f32>(0.0);
    for (var i = 0u; i < 4u; i += 1u) {
        let source = uniforms.lightning_positions[i];
        if source.w <= 0.001 { continue; }
        let delta = source.xyz - position;
        let distance_squared = dot(delta, delta);
        let direction = delta / sqrt(max(distance_squared, 0.001));
        // A broad local wash, including a little transmission through glass.
        let facing = 0.25 + max(dot(normal, direction), 0.0) * 0.75;
        light += uniforms.lightning_colors[i].rgb * source.w * facing / (1.0 + distance_squared * 0.16);
    }
    return light * 0.42;
}

fn floor_reflection(input: VertexOutput) -> vec3<f32> {
    if uniforms.cinematic.x < 0.5 { return vec3<f32>(0.0); }
    let uv = input.clip_position.xy * uniforms.render_size.zw;
    let bounds = uniforms.visual_bounds;
    // World-anchored, very shallow surface waviness: no swimming screen noise.
    let position = input.world_position.xz;
    let grain = sin(position.x * 1.7 + sin(position.y * 0.6)) * sin(position.y * 2.1);
    let offset = vec2<f32>(grain * 0.7, sin(position.y * 3.2) * 0.35) * uniforms.render_size.zw;
    let sample_uv = clamp(uv + offset, bounds.xy + uniforms.render_size.zw, bounds.xy + bounds.zw - uniforms.render_size.zw);
    let reflection = textureSample(reflection_texture, reflection_sampler, sample_uv).rgb;
    let direction = normalize(uniforms.camera_time.xyz - input.world_position);
    let fresnel = 0.36 + 0.40 * pow(1.0 - max(direction.y, 0.0), 3.0);
    let fade = 1.0 - smoothstep(22.0, 78.0, distance(input.world_position, uniforms.camera_time.xyz));
    let roughness = 0.93 + grain * 0.07;
    return reflection * fresnel * fade * roughness * uniforms.cinematic.x;
}

// The mirror pass deliberately contains materials only: no wallpaper, floor,
// navigator, labels, or decorative geometry can reflect recursively.
@fragment
fn fs_reflection(input: VertexOutput) -> @location(0) vec4<f32> {
    let edge = face_edge(input);
    let seed = hash21(input.center.xz + vec2<f32>(19.0, 71.0));
    let smoked = step(0.73, seed);
    let tint = glass_tint(input);
    let height_fade = exp(-max(input.world_position.y - FLOOR_HEIGHT, 0.0) * 0.075);
    var material = tint * (0.10 + edge * 1.3) * (1.0 - smoked * 0.28);
    if uniforms.pulse.w < 0.5 { material = input.color.rgb * (0.28 + edge * 0.75); }
    let glow = lightning_illumination(input.world_position, input.world_normal);
    return vec4<f32>((material + glow * 0.45) * height_fade, 1.0);
}

struct CinematicOutput {
    @location(0) color: vec4<f32>,
    @location(1) emission: vec4<f32>,
}

// Light lives on the pane, so tower depth and glass compositing naturally
// occlude it. Event groups share one bounded wave under sustained writes.
fn filesystem_light(input: VertexOutput) -> vec3<f32> {
    if uniforms.pulse.w < 0.5 || input.color.a < 0.1 || input.color.a >= 0.3 { return vec3<f32>(0.0); }
    var light = vec3<f32>(0.0);
    let navigator = is_navigator(input);
    let horizontal = vec3<f32>(-uniforms.active_face.z, 0.0, uniforms.active_face.x);
    let uv = vec2<f32>(0.5 - dot(input.local_position, horizontal), 0.5 - input.local_position.y);
    // Two softly antialiased rails, confined to the clear outside margin.
    let rail_distance = min(uv.x, 1.0 - uv.x);
    let rail_width = max(fwidth(uv.x), 0.002);
    let rails = 1.0 - smoothstep(0.007, 0.007 + rail_width, rail_distance);
    for (var i = 0; i < 8; i = i + 1) {
        let position = uniforms.activity_positions[i];
        if position.w <= 0.0 { break; }
        let tower_delta = input.center.xz - position.xz;
        if dot(tower_delta, tower_delta) > 0.0025 { continue; }
        let event = uniforms.activity_events[i];
        let phase = clamp(position.y, 0.0, 1.0);
        let envelope = sin(phase * 3.141593) * select(0.0, 1.0, position.y < 1.0);
        let y = input.local_position.y;
        var level = event.y;
        var color = uniforms.secondary.rgb;
        var shape = 0.0;
        if event.z < 0.5 {
            // Creation rises from the foot, then settles at the new file level.
            level = mix(-0.5, event.y, smoothstep(0.0, 0.7, phase));
            color = uniforms.primary.rgb;
        } else if event.z < 1.5 {
            level = event.x - phase * 0.16;
            color = mix(uniforms.secondary.rgb, uniforms.accent.rgb, 0.45);
        } else if event.z < 2.5 {
            level = mix(event.x, event.y, smoothstep(0.0, 1.0, phase));
            color = uniforms.accent.rgb;
        } else {
            // A save sends two small ripples away from its file level.
            level = event.y;
        }
        let width = 0.012 + event.w * 0.008;
        var delta = abs(y - level);
        if event.z > 2.5 { delta = abs(delta - phase * 0.18); }
        shape = exp(-pow(delta / width, 2.0));
        let edge = face_edge(input);
        let residual = position.w * (0.018 + edge * 0.09);
        if navigator {
            let travelling = exp(-pow((y - level) / 0.08, 2.0));
            light += color * rails * (envelope * (0.16 + travelling * 0.95) + position.w * 0.035);
        } else {
            light += color * (shape * envelope * (0.30 + event.w * 0.10) + residual);
        }
    }
    if navigator {
        // A narrow dash sits before the text's left padding, at the actual
        // visible row. Scrolling and the parent-directory row are accounted for.
        let metrics = uniforms.navigator_metrics;
        let marker_x = metrics.z * 0.55;
        let marker_width = metrics.w * 3.0;
        let x_mask = 1.0 - smoothstep(marker_width, marker_width + max(fwidth(uv.x), 0.001), abs(uv.x - marker_x));
        for (var i = 0; i < 4; i = i + 1) {
            let row = uniforms.activity_rows[i];
            if row.w < 0.5 { continue; }
            let center_y = metrics.x + (row.x + 0.5) * metrics.y;
            let y_mask = 1.0 - smoothstep(metrics.y * 0.15, metrics.y * 0.25, abs(uv.y - center_y));
            var color = uniforms.secondary.rgb;
            if row.z < 0.5 { color = uniforms.primary.rgb; }
            if row.z > 1.5 && row.z < 2.5 { color = uniforms.accent.rgb; }
            light += color * x_mask * y_mask * sin(row.y * 3.141593) * 1.1;
        }
    }
    return min(light, vec3<f32>(1.2));
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let base = surface_color(input);
    return vec4<f32>(base.rgb + filesystem_light(input), base.a);
}

@fragment
fn fs_cinematic(input: VertexOutput) -> CinematicOutput {
    var base = vec4<f32>(0.0);
    var glow = vec3<f32>(0.0);
    let floor = input.color.a < 0.1;
    let glass = uniforms.pulse.w > 0.5 && input.color.a >= 0.1 && input.color.a < 0.3;
    let navigator = is_navigator(input);
    // Cinematic glass replaces the ordinary glass material completely.
    if !glass || navigator { base = surface_color(input); }
    if floor {
        // Keep moving floor light out of the glass transmission source. It is
        // rendered later against solid tower depth, independently of glass opacity.
        let radius = distance(input.world_position.xz, uniforms.destination.xz);
        let base_pool = uniforms.primary.rgb * exp(-radius * radius / 9.0) * 0.12;
        let reflected = floor_reflection(input);
        let wash = lightning_illumination(input.world_position, vec3<f32>(0.0, 1.0, 0.0));
        base = vec4<f32>(base.rgb * 0.78 + reflected + base_pool + wash, base.a);
        glow = max(base.rgb - uniforms.background.rgb * 0.32 - vec3<f32>(0.22), vec3<f32>(0.0)) * 0.55 + base_pool * 0.65;
    } else if !navigator {
        let edge = face_edge(input);
        let proximity = exp(-distance(input.world_position.xz, uniforms.destination.xz) * 0.8);
        let destination_tower = exp(-distance(input.center.xz, uniforms.destination.xz) * 2.0);
        let anticipation = sin(clamp(uniforms.cinematic.z / 0.3, 0.0, 1.0) * 3.141593)
            * select(0.0, 1.0, uniforms.cinematic.w > 0.01);
        let sweep_height = -0.5 + clamp(uniforms.cinematic.y * 1.25, 0.0, 1.0);
        let sweep = exp(-pow((input.local_position.y - sweep_height) / 0.12, 2.0));
        let arrival = arrival_light() * proximity * 0.3
            + destination_tower * (anticipation * edge * 0.9 + sweep * arrival_light() * 1.2);
        if glass {
            let direction = normalize(uniforms.camera_time.xyz - input.world_position);
            let fresnel = pow(1.0 - abs(dot(direction, input.world_normal)), 3.0);
            let seed = hash21(input.center.xz + vec2<f32>(19.0, 71.0));
            let smoked = step(0.73, seed);
            let clear = 1.0 - step(0.35, seed);
            let tint = glass_tint(input);
            let n = abs(input.local_normal);
            var plane = input.local_position.xy;
            if n.x > 0.5 { plane = input.local_position.zy; }
            if n.y > 0.5 { plane = input.local_position.xz; }
            let bevel_width = 0.5 - max(abs(plane.x), abs(plane.y));
            let shoulder = 1.0 - smoothstep(0.004, 0.045, bevel_width);
            // A fixed studio light catches the bevel as the camera moves.
            // No timer: a settled view stays quiet.
            let half_vector = normalize(direction + normalize(vec3<f32>(-0.4, 0.8, 0.3)));
            let glint = pow(max(dot(input.world_normal, half_vector), 0.0), 48.0)
                * shoulder * (0.16 + seed * 0.12);
            let bevel = tint * (edge * (0.70 + fresnel * 0.55) + shoulder * fresnel * 0.18)
                + uniforms.accent.rgb * glint;
            let detail = glass_detail(input);
            // A restrained screen-space offset at grazing angles gives the pane thickness.
            let offset = input.world_normal.xy * (1.0 + fresnel * 4.0 + shoulder * 2.0);
            let transmission = blurred_scene(input.clip_position.xy + offset);
            let body = transmission * (0.38 + clear * 0.16 - smoked * 0.18 + fresnel * 0.14) + tint * (0.065 + smoked * 0.025);
            let inner = uniforms.primary.rgb * arrival * (0.32 + edge * 0.8);
            let wash = lightning_illumination(input.world_position, input.world_normal);
            base = vec4<f32>(body + bevel + detail + inner + wash, min(0.97, 0.82 - clear * 0.12 + smoked * 0.12 + edge * 0.10));
            glow = bevel * 1.25 + detail * 0.35 + inner + wash * 0.35;
        } else {
            let seam = uniforms.primary.rgb * edge * 0.18;
            let inner = uniforms.primary.rgb * arrival * 0.25 + lightning_illumination(input.world_position, input.world_normal);
            base = vec4<f32>(base.rgb + seam + inner, base.a);
            glow = max(base.rgb - vec3<f32>(0.65), vec3<f32>(0.0)) * 0.55 + seam + inner;
        }
    }
    let activity = filesystem_light(input);
    base = vec4<f32>(base.rgb + activity, base.a);
    glow += activity * 0.8;
    // Height fog gives distant towers separation while keeping the working face clear.
    if !navigator {
        let distance_to_eye = distance(input.world_position, uniforms.camera_time.xyz);
        let low_haze = exp(-max(input.world_position.y + 0.42, 0.0) * 0.32);
        let haze = (1.0 - exp(-distance_to_eye * 0.014)) * (0.20 + low_haze * 0.52);
        let atmosphere = uniforms.background.rgb * 0.22 + uniforms.primary.rgb * 0.035;
        base = vec4<f32>(mix(base.rgb, atmosphere, haze), base.a);
        glow *= 1.0 - haze;
    }
    var output: CinematicOutput;
    output.color = base;
    output.emission = vec4<f32>(glow, base.a);
    return output;
}

// Floor light is composited only after glass, with its own tower depth mask.
// Neither the sharp ring nor its emission can leak through the transmission blur.
@fragment
fn fs_floor_journey(input: VertexOutput) -> CinematicOutput {
    if input.world_normal.y < 0.5 { discard; }
    let distance_to_eye = distance(input.world_position, uniforms.camera_time.xyz);
    let fade = 1.0 - (1.0 - exp(-distance_to_eye * 0.014)) * 0.72;
    let light = journey_floor(input.world_position.xz) * fade;
    var output: CinematicOutput;
    output.color = vec4<f32>(light, 0.0);
    output.emission = vec4<f32>(light * 1.2, 0.0);
    return output;
}
