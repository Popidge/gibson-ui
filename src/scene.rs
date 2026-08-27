use crate::filesystem::{FileEntry, FileMutation};
use crate::navigation::VisualState;
use crate::topology::{CityTowerPlacement, city_grid_position};
use glam::{Mat4, Quat, Vec3};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const APPEAR_TIME: Duration = Duration::from_millis(620);
const TOWER_SPACING: f32 = 5.4;
const MAX_VISIBLE_STOREYS: usize = 72;
const MAX_CONNECTIONS: usize = 96;
const MAX_TOWER_LABELS: usize = 32;
const FILE_EFFECT_TIME: Duration = Duration::from_millis(1_100);
pub const MAX_RENDER_OBJECTS: usize = 960;

#[derive(Clone, Debug)]
struct Tower {
    path: PathBuf,
    name: String,
    label: String,
    position: Vec3,
    children: Vec<FileEntry>,
    selected: Option<u128>,
    known: bool,
    entry_count_hint: usize,
    visual_scale: f32,
    height: f32,
    width: f32,
    readable: bool,
    born: Instant,
    updated: Instant,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderObject {
    pub model: Mat4,
    pub color: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct TowerLabel {
    pub world_position: Vec3,
    pub text: String,
    pub current: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct FacePanel {
    pub center: Vec3,
    pub horizontal: Vec3,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct CameraSubject {
    pub center: Vec3,
    pub height: f32,
    pub width: f32,
}

#[derive(Clone, Copy, Debug)]
struct CameraFlight {
    started: Instant,
    duration: Duration,
    from: Vec3,
    control_a: Vec3,
    control_b: Vec3,
    to: Vec3,
    distance: f32,
    curve_sign: f32,
    view_from: Vec3,
    framing_height_from: f32,
    lift_factor: f32,
}

#[derive(Clone, Copy, Debug)]
struct FlightParameters {
    route_distance: f32,
    view_from: Vec3,
    framing_height_from: f32,
    state: VisualState,
    motion_scale: f32,
}

#[derive(Clone, Copy, Debug)]
enum FileEffectKind {
    Create,
    Remove,
    Rename,
    Modify,
}

#[derive(Clone, Copy, Debug)]
struct FileEffect {
    kind: FileEffectKind,
    started: Instant,
    from_index: usize,
    to_index: usize,
}

impl CameraFlight {
    fn stationary(now: Instant, focus: Vec3, view: Vec3, framing_height: f32) -> Self {
        Self {
            started: now,
            duration: Duration::ZERO,
            from: focus,
            control_a: focus,
            control_b: focus,
            to: focus,
            distance: 0.0,
            curve_sign: 0.0,
            view_from: view,
            framing_height_from: framing_height,
            lift_factor: 0.0,
        }
    }

    fn travelling(now: Instant, from: Vec3, to: Vec3, parameters: FlightParameters) -> Self {
        let FlightParameters {
            route_distance,
            view_from,
            framing_height_from,
            state,
            motion_scale,
        } = parameters;
        if motion_scale <= 0.01 {
            return Self::stationary(now, to, view_from, framing_height_from);
        }
        let (canonical_from, canonical_to) = if position_key(from) <= position_key(to) {
            (from, to)
        } else {
            (to, from)
        };
        let canonical_delta = canonical_to - canonical_from;
        let flat = Vec3::new(canonical_delta.x, 0.0, canonical_delta.z);
        let side = Vec3::new(-flat.z, 0.0, flat.x)
            .try_normalize()
            .unwrap_or(Vec3::X);
        let variation = flight_variation(canonical_from, canonical_to);
        let curve_sign = if variation.0 & 1 == 0 { -1.0 } else { 1.0 };
        let lift_factor = variation.1;
        let motion = state.flight_scale() * motion_scale.clamp(0.05, 1.5);
        let span = flat.length().max(route_distance * 0.35);
        let bend_amount = (span * 0.10).clamp(0.35, 3.2) * motion;
        let bend = side * curve_sign * bend_amount;
        let maximum_lift = (1.2 + span * 0.23).clamp(1.4, 8.0) * motion;
        let lift_height = 0.16 + (maximum_lift - 0.16) * lift_factor.powf(1.55);
        let lift = Vec3::Y * lift_height;
        let canonical_a = canonical_from + canonical_delta * 0.22 + bend + lift;
        let canonical_b = canonical_from + canonical_delta * 0.78 + bend + lift;
        let forward = position_key(from) <= position_key(to);
        let (control_a, control_b) = if forward {
            (canonical_a, canonical_b)
        } else {
            (canonical_b, canonical_a)
        };
        Self {
            started: now,
            duration: flight_duration(route_distance, state)
                .mul_f32(motion_scale.clamp(0.05, 1.5) * (1.0 + lift_factor * 0.08)),
            from,
            control_a,
            control_b,
            to,
            distance: route_distance,
            curve_sign,
            view_from,
            framing_height_from,
            lift_factor,
        }
    }
}

fn flight_variation(from: Vec3, to: Vec3) -> (u32, f32) {
    let delta = to - from;
    let dominant_axis = if delta.x.abs() >= delta.z.abs() {
        0x9e37_79b9_u32
    } else {
        0x7f4a_7c15_u32
    };
    let mut seed = from.x.to_bits()
        ^ from.z.to_bits().rotate_left(7)
        ^ to.x.to_bits().rotate_left(13)
        ^ to.z.to_bits().rotate_left(19)
        ^ dominant_axis;
    seed ^= seed >> 16;
    seed = seed.wrapping_mul(0x7feb_352d);
    seed ^= seed >> 15;
    seed = seed.wrapping_mul(0x846c_a68b);
    seed ^= seed >> 16;
    let lift = ((seed >> 8) & 0xffff) as f32 / u16::MAX as f32;
    (seed, lift)
}

fn position_key(position: Vec3) -> (u32, u32, u32) {
    (
        position.x.to_bits(),
        position.y.to_bits(),
        position.z.to_bits(),
    )
}

pub struct Scene {
    root: PathBuf,
    towers: HashMap<PathBuf, Tower>,
    tower_order: Vec<PathBuf>,
    flight: CameraFlight,
    command_started: Option<Instant>,
    visual_state: VisualState,
    motion_scale: f32,
    file_effects: Vec<FileEffect>,
}

impl Scene {
    pub fn new(root: PathBuf) -> Self {
        let now = Instant::now();
        let initial_focus = Vec3::new(0.0, 1.8, 0.0);
        let mut scene = Self {
            root: root.clone(),
            towers: HashMap::new(),
            tower_order: Vec::new(),
            flight: CameraFlight::stationary(now, initial_focus, Vec3::Z, 3.6),
            command_started: None,
            visual_state: VisualState::Transit,
            motion_scale: 1.0,
            file_effects: Vec::new(),
        };
        scene.ensure_tower(root, now);
        scene
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn update(&mut self, root: PathBuf, entries: Vec<FileEntry>, mutations: &[FileMutation]) {
        let now = Instant::now();
        let root_changed = root != self.root;
        let previous_focus = self.focus_position(now);
        let previous_view = self.camera_direction(now);
        let previous_height = self.framing_height(now);
        let previous_root = self.root.clone();
        if !root_changed {
            self.queue_file_effects(mutations, &entries, now);
        } else {
            self.file_effects.clear();
        }
        let directory_paths = entries
            .iter()
            .filter(|entry| entry.is_directory())
            .map(|entry| entry.path.clone())
            .collect::<HashSet<_>>();

        self.towers.retain(|path, _| {
            path.parent() != Some(root.as_path()) || directory_paths.contains(path)
        });
        self.tower_order
            .retain(|path| self.towers.contains_key(path));

        self.ensure_tower(root.clone(), now);
        if let Some(parent) = root.parent() {
            self.ensure_tower(parent.to_path_buf(), now);
        }
        for entry in &entries {
            if entry.is_directory() {
                self.ensure_tower(entry.path.clone(), now);
            }
        }

        if root_changed {
            self.root.clone_from(&root);
        }

        let current = self
            .towers
            .get_mut(&root)
            .expect("the current directory tower must exist");
        let next_selected = current
            .selected
            .filter(|id| entries.iter().any(|entry| entry.id == *id))
            .or_else(|| entries.first().map(|entry| entry.id));
        current.children = entries;
        current.entry_count_hint = current.children.len();
        current.selected = next_selected;
        current.known = true;
        current.updated = now;
        refresh_tower_geometry(current);

        if root_changed {
            let target = self.focus_target();
            let world_distance = previous_focus.distance(target);
            let filesystem_distance = path_distance(&previous_root, &root) as f32 * TOWER_SPACING;
            let route_distance = world_distance.max(filesystem_distance);
            self.flight = CameraFlight::travelling(
                now,
                previous_focus,
                target,
                FlightParameters {
                    route_distance,
                    view_from: previous_view,
                    framing_height_from: previous_height,
                    state: self.visual_state,
                    motion_scale: self.motion_scale,
                },
            );
        }
    }

    pub fn set_motion_scale(&mut self, scale: f32) {
        self.motion_scale = if scale.is_finite() {
            scale.clamp(0.0, 1.5)
        } else {
            1.0
        };
    }

    pub fn set_visual_state(&mut self, state: VisualState) {
        self.visual_state = state;
    }

    pub fn begin_waypoint(&mut self, root: PathBuf, placements: &[CityTowerPlacement]) {
        let now = Instant::now();
        let previous_root = self.root.clone();
        let previous_focus = self.focus_position(now);
        let previous_view = self.camera_direction(now);
        let previous_height = self.framing_height(now);
        self.root.clone_from(&root);
        self.apply_city_layout(placements);
        self.ensure_tower(root.clone(), now);
        let target = self.focus_target();
        let world_distance = previous_focus.distance(target);
        let filesystem_distance = path_distance(&previous_root, &root) as f32 * TOWER_SPACING;
        self.flight = CameraFlight::travelling(
            now,
            previous_focus,
            target,
            FlightParameters {
                route_distance: world_distance.max(filesystem_distance),
                view_from: previous_view,
                framing_height_from: previous_height,
                state: self.visual_state,
                motion_scale: self.motion_scale,
            },
        );
    }

    pub fn flight_complete(&self) -> bool {
        self.flight_progress(Instant::now()) >= 1.0
    }

    pub fn flight_duration(&self) -> Duration {
        self.flight.duration
    }

    pub fn apply_city_layout(&mut self, placements: &[CityTowerPlacement]) {
        let now = Instant::now();
        let visible = placements
            .iter()
            .map(|placement| placement.path.clone())
            .collect::<HashSet<_>>();
        self.towers.retain(|path, _| visible.contains(path));
        self.tower_order.retain(|path| visible.contains(path));

        for placement in placements {
            self.ensure_tower(placement.path.clone(), now);
            let tower = self
                .towers
                .get_mut(&placement.path)
                .expect("a placed tower must exist");
            tower.name.clone_from(&placement.name);
            tower.label = compact_label(&tower.name, 22);
            tower.position = placement.position;
            tower.entry_count_hint = placement.entry_count;
            tower.known |= placement.known;
            tower.readable = placement.readable;
            tower.visual_scale = placement.scale;
            refresh_tower_geometry(tower);
        }

        if self.flight_progress(now) < 1.0 {
            let target = self.focus_target();
            let adjustment = target - self.flight.to;
            self.flight.control_b += adjustment * 0.72;
            self.flight.to = target;
        }
    }

    pub fn begin_command(&mut self) {
        self.command_started = Some(Instant::now());
    }

    pub fn finish_command(&mut self) {
        self.command_started = None;
    }

    pub fn command_active(&self) -> bool {
        self.command_started.is_some()
    }

    pub fn select_entry(&mut self, selected: Option<u128>) {
        let Some(tower) = self.towers.get_mut(&self.root) else {
            return;
        };
        tower.selected = selected.filter(|id| tower.children.iter().any(|entry| entry.id == *id));
    }

    pub fn focus_position(&self, now: Instant) -> Vec3 {
        let progress = smoothstep(self.flight_progress(now));
        if progress >= 1.0 {
            return self.focus_target();
        }
        cubic_bezier(
            self.flight.from,
            self.flight.control_a,
            self.flight.control_b,
            self.flight.to,
            progress,
        )
    }

    pub fn flight_progress(&self, now: Instant) -> f32 {
        if self.flight.duration.is_zero() {
            return 1.0;
        }
        (now.duration_since(self.flight.started).as_secs_f32() / self.flight.duration.as_secs_f32())
            .clamp(0.0, 1.0)
    }

    pub fn camera_direction(&self, now: Instant) -> Vec3 {
        let progress = smoothstep(self.flight_progress(now));
        self.flight
            .view_from
            .lerp(self.camera_direction_target(), progress)
            .try_normalize()
            .unwrap_or(Vec3::Z)
    }

    pub fn framing_height(&self, now: Instant) -> f32 {
        let target = self.towers.get(&self.root).map_or(3.6, tower_height);
        let progress = smoothstep(self.flight_progress(now));
        self.flight.framing_height_from + (target - self.flight.framing_height_from) * progress
    }

    pub fn flight_intensity(&self, now: Instant) -> f32 {
        let progress = self.flight_progress(now);
        if !(0.0..1.0).contains(&progress) {
            return 0.0;
        }
        (std::f32::consts::PI * progress).sin()
            * (self.flight.distance / (TOWER_SPACING * 3.0)).clamp(0.25, 1.0)
    }

    pub fn camera_roll(&self, now: Instant) -> f32 {
        let bank = 4.5 + self.flight.lift_factor * 2.0;
        self.flight.curve_sign * self.flight_intensity(now) * bank.to_radians()
    }

    pub fn camera_subject(&self, now: Instant) -> CameraSubject {
        self.towers
            .get(&self.root)
            .map(|tower| camera_subject(tower, Some(self.focus_position(now))))
            .unwrap_or_else(|| CameraSubject {
                center: self.focus_position(now),
                height: 3.6,
                width: 1.65,
            })
    }

    pub fn write_render_objects(
        &self,
        now: Instant,
        state: VisualState,
        camera_eye: Vec3,
        camera_focus: Vec3,
        max_objects: usize,
        objects: &mut Vec<RenderObject>,
    ) {
        let max_objects = max_objects.clamp(64, MAX_RENDER_OBJECTS);
        let current_path = &self.root;
        let current_position = self
            .towers
            .get(current_path)
            .map_or(Vec3::ZERO, |tower| tower.position);
        objects.clear();
        if objects.capacity() < max_objects {
            objects.reserve(max_objects - objects.capacity());
        }

        for tower in self
            .towers
            .values()
            .filter(|tower| self.tower_is_visible(tower, state, camera_eye, camera_focus))
        {
            let amount = smoothstep(
                (now.duration_since(tower.born).as_secs_f32() / APPEAR_TIME.as_secs_f32())
                    .clamp(0.0, 1.0),
            );
            let height = tower_height(tower);
            let width = tower_width(tower);
            let scale = Vec3::new(width, (height * amount).max(0.03), width);
            let position = tower.position + Vec3::new(0.0, scale.y * 0.5 - 0.05, 0.0);
            let model = Mat4::from_scale_rotation_translation(scale, Quat::IDENTITY, position);
            let age = now.duration_since(tower.updated).as_secs_f32();
            let color = if !tower.readable {
                [0.72, 0.03, 0.24, 1.15]
            } else if tower.path == *current_path {
                let pulse = (age * 4.0).sin().abs() * 0.12;
                [0.015, 0.44 + pulse, 0.68 + pulse, 1.55]
            } else if tower.known {
                [0.02, 0.72, 0.78, 1.25]
            } else {
                [0.02, 0.31, 0.58, 0.85]
            };
            objects.push(RenderObject { model, color });
        }

        if let Some(current) = self.towers.get(current_path) {
            add_tower_bands(
                objects,
                current,
                tower_height(current),
                tower_width(current),
                now,
            );
            if state.shows_storeys() {
                add_storey_slabs(objects, current, now);
                add_file_effects(objects, current, &self.file_effects, now);
            }
            if state != VisualState::Settled {
                for entry in current
                    .children
                    .iter()
                    .filter(|entry| entry.is_directory())
                    .take(MAX_CONNECTIONS)
                {
                    if let Some(target) = self.towers.get(&entry.path).filter(|tower| {
                        self.tower_is_visible(tower, state, camera_eye, camera_focus)
                    }) {
                        add_connection(
                            objects,
                            current_position,
                            target.position,
                            current.selected == Some(entry.id),
                            now.duration_since(current.born).as_secs_f32(),
                        );
                    }
                }
            }
        }
        for path in &self.tower_order {
            if objects.len() >= max_objects || path == current_path {
                continue;
            }
            let Some(tower) = self
                .towers
                .get(path)
                .filter(|tower| self.tower_is_visible(tower, state, camera_eye, camera_focus))
            else {
                continue;
            };
            add_tower_bands(objects, tower, tower_height(tower), tower_width(tower), now);
        }
        objects.truncate(max_objects);
    }

    pub fn active_face_panel(&self, outward: Vec3) -> Option<FacePanel> {
        let tower = self.towers.get(&self.root)?;
        Some(face_panel_for(tower, outward))
    }

    pub fn tower_labels(
        &self,
        face_outward: Vec3,
        state: VisualState,
        camera_eye: Vec3,
        camera_focus: Vec3,
        max_labels: usize,
    ) -> Vec<TowerLabel> {
        let max_labels = max_labels.clamp(1, MAX_TOWER_LABELS);
        let face_direction = Vec3::new(face_outward.x, 0.0, face_outward.z)
            .try_normalize()
            .unwrap_or(Vec3::NEG_Z);
        let mut labels = Vec::with_capacity(max_labels);
        let visible_towers = self
            .towers
            .values()
            .filter(|tower| self.tower_is_visible(tower, state, camera_eye, camera_focus))
            .collect::<Vec<_>>();
        let label_towers = self
            .towers
            .get(&self.root)
            .into_iter()
            .chain(
                self.tower_order
                    .iter()
                    .filter(|path| *path != &self.root)
                    .filter_map(|path| self.towers.get(path)),
            )
            .take(max_labels);

        for tower in label_towers {
            if !visible_towers
                .iter()
                .any(|visible| std::ptr::eq(*visible, tower))
            {
                continue;
            }
            let current = tower.path == self.root;
            let world_position = tower.position
                + face_direction * (tower_width(tower) * 0.52)
                + Vec3::Y * (tower_height(tower) + 0.24);
            let occluded = visible_towers.iter().any(|occluder| {
                occluder.path != tower.path
                    && tower_occludes_point(occluder, camera_eye, world_position)
            });
            if occluded {
                continue;
            }
            labels.push(TowerLabel {
                world_position,
                text: tower.label.clone(),
                current,
            });
        }

        labels
    }

    fn queue_file_effects(
        &mut self,
        mutations: &[FileMutation],
        next_entries: &[FileEntry],
        now: Instant,
    ) {
        let Some(current) = self.towers.get(&self.root) else {
            return;
        };
        self.file_effects
            .retain(|effect| now.duration_since(effect.started) < FILE_EFFECT_TIME);
        for mutation in mutations.iter().take(24) {
            let effect = match mutation {
                FileMutation::Created(entry) => FileEffect {
                    kind: FileEffectKind::Create,
                    started: now,
                    from_index: next_entries
                        .iter()
                        .position(|candidate| candidate.id == entry.id)
                        .unwrap_or(0),
                    to_index: next_entries
                        .iter()
                        .position(|candidate| candidate.id == entry.id)
                        .unwrap_or(0),
                },
                FileMutation::Removed(entry) => FileEffect {
                    kind: FileEffectKind::Remove,
                    started: now,
                    from_index: current
                        .children
                        .iter()
                        .position(|candidate| candidate.id == entry.id)
                        .unwrap_or(0),
                    to_index: current
                        .children
                        .iter()
                        .position(|candidate| candidate.id == entry.id)
                        .unwrap_or(0),
                },
                FileMutation::Renamed { before, after } => FileEffect {
                    kind: FileEffectKind::Rename,
                    started: now,
                    from_index: current
                        .children
                        .iter()
                        .position(|candidate| candidate.id == before.id)
                        .unwrap_or(0),
                    to_index: next_entries
                        .iter()
                        .position(|candidate| candidate.id == after.id)
                        .unwrap_or(0),
                },
                FileMutation::Modified(entry) => FileEffect {
                    kind: FileEffectKind::Modify,
                    started: now,
                    from_index: next_entries
                        .iter()
                        .position(|candidate| candidate.id == entry.id)
                        .unwrap_or(0),
                    to_index: next_entries
                        .iter()
                        .position(|candidate| candidate.id == entry.id)
                        .unwrap_or(0),
                },
            };
            self.file_effects.push(effect);
        }
    }

    fn tower_is_visible(
        &self,
        tower: &Tower,
        state: VisualState,
        camera_eye: Vec3,
        camera_focus: Vec3,
    ) -> bool {
        match state {
            VisualState::Transit => true,
            VisualState::Settled => {
                if tower.path == self.root {
                    return true;
                }
                // Keep the city behind and beside the active tower. Only remove a
                // nearer tower when it covers the active face.
                !tower_blocks_view(tower, camera_eye, camera_focus, state)
            }
        }
    }

    fn ensure_tower(&mut self, path: PathBuf, now: Instant) {
        if self.towers.contains_key(&path) {
            return;
        }
        let slot = self.tower_order.len();
        let position = city_grid_position(slot) * TOWER_SPACING;
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "/".into());
        let label = compact_label(&name, 22);
        self.tower_order.push(path.clone());
        self.towers.insert(
            path.clone(),
            Tower {
                path,
                name,
                label,
                position,
                children: Vec::new(),
                selected: None,
                known: false,
                entry_count_hint: 0,
                visual_scale: 1.0,
                height: 3.6,
                width: 1.65,
                readable: true,
                born: now,
                updated: now,
            },
        );
    }

    fn focus_target(&self) -> Vec3 {
        let Some(tower) = self.towers.get(&self.root) else {
            return Vec3::new(0.0, 1.8, 0.0);
        };
        tower.position + Vec3::new(0.0, tower_height(tower) * 0.42, 0.0)
    }

    fn camera_direction_target(&self) -> Vec3 {
        let Some(current) = self.towers.get(&self.root) else {
            return Vec3::Z;
        };
        let child_center = current
            .children
            .iter()
            .filter(|entry| entry.is_directory())
            .filter_map(|entry| self.towers.get(&entry.path))
            .fold(Vec3::ZERO, |total, tower| {
                total + (tower.position - current.position)
            });
        if let Some(direction) = (-child_center).try_normalize() {
            return direction;
        }
        current.position.try_normalize().unwrap_or(Vec3::Z)
    }
}

fn add_tower_bands(
    objects: &mut Vec<RenderObject>,
    tower: &Tower,
    height: f32,
    width: f32,
    now: Instant,
) {
    let band_count = ((height / 0.48).round() as usize).clamp(4, 20);
    for index in 1..band_count {
        let y = height * index as f32 / band_count as f32;
        let shimmer = (now.duration_since(tower.born).as_secs_f32() * 1.8 + index as f32 * 0.7)
            .sin()
            .abs();
        objects.push(RenderObject {
            model: Mat4::from_scale_rotation_translation(
                Vec3::new(width * 1.035, 0.025, width * 1.035),
                Quat::IDENTITY,
                tower.position + Vec3::new(0.0, y, 0.0),
            ),
            color: [0.08, 0.55 + shimmer * 0.22, 0.92, 0.8],
        });
    }
}

fn add_storey_slabs(objects: &mut Vec<RenderObject>, tower: &Tower, now: Instant) {
    if tower.children.is_empty() {
        return;
    }
    let height = tower_height(tower);
    let width = tower_width(tower);
    let selected_index = tower
        .selected
        .and_then(|id| tower.children.iter().position(|entry| entry.id == id))
        .unwrap_or(0);
    let half = MAX_VISIBLE_STOREYS / 2;
    let start = selected_index
        .saturating_sub(half)
        .min(tower.children.len().saturating_sub(MAX_VISIBLE_STOREYS));
    let end = (start + MAX_VISIBLE_STOREYS).min(tower.children.len());
    for (index, entry) in tower.children.iter().enumerate().take(end).skip(start) {
        let selected = tower.selected == Some(entry.id);
        let y = storey_height(index, tower.children.len(), height);
        let pulse = (now.duration_since(tower.born).as_secs_f32() * 5.0 + index as f32 * 0.21)
            .sin()
            .abs();
        let color = if selected {
            [1.0, 0.03 + pulse * 0.18, 0.74, 2.8]
        } else if !entry.readable {
            [1.0, 0.03, 0.12, 1.9]
        } else if entry.is_directory() {
            [0.10, 0.96, 0.94, 1.65]
        } else if entry.hidden {
            [0.08, 0.24, 0.38, 0.55]
        } else {
            [0.22, 0.62, 1.0, 1.1]
        };
        objects.push(RenderObject {
            model: Mat4::from_scale_rotation_translation(
                Vec3::new(
                    width * if selected { 1.34 } else { 1.18 },
                    0.065,
                    width * 1.18,
                ),
                Quat::IDENTITY,
                tower.position + Vec3::new(0.0, y, 0.0),
            ),
            color,
        });
    }
}

fn add_file_effects(
    objects: &mut Vec<RenderObject>,
    tower: &Tower,
    effects: &[FileEffect],
    now: Instant,
) {
    let count = tower.children.len().max(
        effects
            .iter()
            .map(|effect| effect.from_index + 1)
            .max()
            .unwrap_or(1),
    );
    let height = tower_height(tower);
    let width = tower_width(tower);
    for effect in effects {
        let progress =
            now.duration_since(effect.started).as_secs_f32() / FILE_EFFECT_TIME.as_secs_f32();
        if !(0.0..1.0).contains(&progress) {
            continue;
        }
        let envelope = (std::f32::consts::PI * progress).sin().max(0.0);
        let from_y = storey_height(effect.from_index, count, height);
        let to_y = storey_height(effect.to_index, count, height);
        match effect.kind {
            FileEffectKind::Create => {
                let rise = smoothstep(progress.min(0.72) / 0.72);
                let beam_height = (to_y + 0.35) * rise;
                objects.push(RenderObject {
                    model: Mat4::from_scale_rotation_translation(
                        Vec3::new(width * 0.09, beam_height.max(0.03), width * 0.09),
                        Quat::from_rotation_y(progress * 2.4),
                        tower.position + Vec3::new(width * 0.66, beam_height * 0.5, width * 0.66),
                    ),
                    color: [0.05, 0.95, 0.72, 2.8 * envelope],
                });
                objects.push(RenderObject {
                    model: Mat4::from_scale_rotation_translation(
                        Vec3::new(width * (1.28 + envelope * 0.30), 0.055, width * 1.28),
                        Quat::from_rotation_y(progress * 0.32),
                        tower.position + Vec3::new(0.0, to_y, 0.0),
                    ),
                    color: [0.05, 1.0, 0.68, 2.5 * envelope],
                });
            }
            FileEffectKind::Remove => {
                let collapse = 1.0 - smoothstep(progress);
                objects.push(RenderObject {
                    model: Mat4::from_scale_rotation_translation(
                        Vec3::new(
                            width * (1.25 + progress * 0.55),
                            (0.11 * collapse).max(0.012),
                            width * (1.25 + progress * 0.55),
                        ),
                        Quat::from_rotation_y(progress * 0.9),
                        tower.position + Vec3::new(0.0, from_y - progress * 0.45, 0.0),
                    ),
                    color: [1.0, 0.02, 0.34, 2.7 * envelope],
                });
            }
            FileEffectKind::Rename => {
                let y = from_y + (to_y - from_y) * smoothstep(progress);
                let vertical_span = (to_y - from_y).abs().max(0.2);
                objects.push(RenderObject {
                    model: Mat4::from_scale_rotation_translation(
                        Vec3::new(width * 0.055, vertical_span, width * 0.055),
                        Quat::IDENTITY,
                        tower.position + Vec3::new(0.0, (from_y + to_y) * 0.5, 0.0),
                    ),
                    color: [0.82, 0.08, 1.0, 2.1 * envelope],
                });
                objects.push(RenderObject {
                    model: Mat4::from_scale_rotation_translation(
                        Vec3::splat(0.16 + envelope * 0.16),
                        Quat::from_rotation_y(progress * 4.0),
                        tower.position + Vec3::new(0.0, y, width * 0.68),
                    ),
                    color: [1.0, 0.12, 0.82, 3.2 * envelope],
                });
            }
            FileEffectKind::Modify => {
                let phase = progress * std::f32::consts::TAU * 2.0;
                for offset in [0.0, std::f32::consts::PI] {
                    objects.push(RenderObject {
                        model: Mat4::from_scale_rotation_translation(
                            Vec3::splat(0.11 + envelope * 0.08),
                            Quat::from_rotation_y(phase + offset),
                            tower.position
                                + Vec3::new(
                                    (phase + offset).cos() * width * 0.72,
                                    to_y,
                                    (phase + offset).sin() * width * 0.72,
                                ),
                        ),
                        color: [0.15, 0.72, 1.0, 2.6 * envelope],
                    });
                }
            }
        }
    }
}

fn face_panel_for(tower: &Tower, outward: Vec3) -> FacePanel {
    let outward = Vec3::new(outward.x, 0.0, outward.z)
        .try_normalize()
        .unwrap_or(Vec3::NEG_Z);
    let width = tower_width(tower);
    let height = tower_height(tower);
    FacePanel {
        center: tower.position + outward * (width * 0.625 + 0.025) + Vec3::Y * (height * 0.50),
        horizontal: face_horizontal(outward),
        width: width * 0.96,
        height: (height * 0.88).clamp(3.5, 16.0),
    }
}

fn face_horizontal(outward: Vec3) -> Vec3 {
    Vec3::new(-outward.z, 0.0, outward.x)
        .try_normalize()
        .unwrap_or(Vec3::X)
}

fn add_connection(
    objects: &mut Vec<RenderObject>,
    from: Vec3,
    to: Vec3,
    selected: bool,
    elapsed: f32,
) {
    let delta = to - from;
    let flat = Vec3::new(delta.x, 0.0, delta.z);
    let length = flat.length();
    if length < 0.1 {
        return;
    }
    let midpoint = (from + to) * 0.5 + Vec3::new(0.0, 0.035, 0.0);
    let angle = -flat.z.atan2(flat.x);
    let rotation = Quat::from_rotation_y(angle);
    objects.push(RenderObject {
        model: Mat4::from_scale_rotation_translation(
            Vec3::new(length, 0.025, if selected { 0.09 } else { 0.045 }),
            rotation,
            midpoint,
        ),
        color: if selected {
            [1.0, 0.02, 0.72, 2.3]
        } else {
            [0.02, 0.46, 0.88, 0.75]
        },
    });

    if selected {
        let phase = (elapsed * 0.7).fract();
        objects.push(RenderObject {
            model: Mat4::from_scale_rotation_translation(
                Vec3::splat(0.18),
                Quat::IDENTITY,
                from.lerp(to, phase) + Vec3::new(0.0, 0.18, 0.0),
            ),
            color: [0.65, 0.96, 1.0, 3.2],
        });
    }
}

fn camera_subject(tower: &Tower, animated_center: Option<Vec3>) -> CameraSubject {
    let height = tower_height(tower);
    let width = tower_width(tower);
    CameraSubject {
        center: animated_center
            .unwrap_or_else(|| tower.position + Vec3::new(0.0, height * 0.42, 0.0)),
        height,
        width,
    }
}

fn tower_blocks_view(
    tower: &Tower,
    camera_eye: Vec3,
    camera_focus: Vec3,
    state: VisualState,
) -> bool {
    let ray = camera_focus - camera_eye;
    let distance = ray.length();
    if distance < 0.1 {
        return false;
    }
    let direction = ray / distance;
    let margin = match state {
        VisualState::Settled => 0.62,
        VisualState::Transit => 0.0,
    };
    let half_width = tower_width(tower) * 0.55 + margin;
    let bounds_min = tower.position + Vec3::new(-half_width, -0.10, -half_width);
    let bounds_max =
        tower.position + Vec3::new(half_width, tower_height(tower) + margin * 0.5, half_width);
    ray_hits_box(
        camera_eye,
        direction,
        (distance - 0.8).max(0.0),
        bounds_min,
        bounds_max,
    )
}

fn tower_occludes_point(tower: &Tower, camera_eye: Vec3, point: Vec3) -> bool {
    let ray = point - camera_eye;
    let distance = ray.length();
    if distance < 0.1 {
        return false;
    }
    let half_width = tower_width(tower) * 0.56 + 0.04;
    let bounds_min = tower.position + Vec3::new(-half_width, -0.10, -half_width);
    let bounds_max = tower.position + Vec3::new(half_width, tower_height(tower) + 0.08, half_width);
    ray_hits_box(
        camera_eye,
        ray / distance,
        (distance - 0.12).max(0.0),
        bounds_min,
        bounds_max,
    )
}

fn ray_hits_box(origin: Vec3, direction: Vec3, max_distance: f32, min: Vec3, max: Vec3) -> bool {
    let mut near = 0.0_f32;
    let mut far = max_distance;
    for axis in 0..3 {
        let origin_axis = origin[axis];
        let direction_axis = direction[axis];
        if direction_axis.abs() < 0.0001 {
            if origin_axis < min[axis] || origin_axis > max[axis] {
                return false;
            }
            continue;
        }
        let inverse = direction_axis.recip();
        let mut first = (min[axis] - origin_axis) * inverse;
        let mut second = (max[axis] - origin_axis) * inverse;
        if first > second {
            std::mem::swap(&mut first, &mut second);
        }
        near = near.max(first);
        far = far.min(second);
        if near > far {
            return false;
        }
    }
    far >= 0.0 && near < max_distance
}

fn tower_height(tower: &Tower) -> f32 {
    tower.height
}

fn tower_width(tower: &Tower) -> f32 {
    tower.width
}

fn refresh_tower_geometry(tower: &mut Tower) {
    let content_count = tower.children.len().max(tower.entry_count_hint);
    let height = if tower.known {
        (3.2 + (content_count as f32 + 1.0).ln() * 1.35).clamp(3.2, 11.5)
    } else {
        3.6
    };
    tower.height = height * tower.visual_scale;
    tower.width = if tower.known {
        let direct_bytes = tower
            .children
            .iter()
            .filter(|entry| !entry.is_directory())
            .fold(0_u64, |total, entry| total.saturating_add(entry.size));
        let child_width = (content_count as f32 + 1.0).ln() * 0.22;
        (1.65 + child_width + (direct_bytes.saturating_add(1) as f32).log2() * 0.025)
            .clamp(1.65, 3.10)
            * tower.visual_scale
    } else {
        1.65 * tower.visual_scale
    };
}

fn storey_height(index: usize, count: usize, tower_height: f32) -> f32 {
    let step = tower_height / (count.max(1) + 1) as f32;
    step * count.saturating_sub(index) as f32
}

fn cubic_bezier(from: Vec3, control_a: Vec3, control_b: Vec3, to: Vec3, t: f32) -> Vec3 {
    let inverse = 1.0 - t;
    from * inverse.powi(3)
        + control_a * (3.0 * inverse.powi(2) * t)
        + control_b * (3.0 * inverse * t.powi(2))
        + to * t.powi(3)
}

fn flight_duration(distance: f32, state: VisualState) -> Duration {
    let base = (0.72 + distance * 0.105).clamp(0.85, 4.8);
    Duration::from_secs_f32((base * state.flight_scale()).clamp(0.48, 4.8))
}

fn path_distance(from: &Path, to: &Path) -> usize {
    let from_components = from.components().collect::<Vec<_>>();
    let to_components = to.components().collect::<Vec<_>>();
    let shared = from_components
        .iter()
        .zip(&to_components)
        .take_while(|(left, right)| left == right)
        .count();
    from_components.len() + to_components.len() - shared * 2
}

fn compact_label(value: &str, limit: usize) -> String {
    let upper = value.to_uppercase();
    if upper.chars().count() <= limit {
        return upper;
    }
    let mut compact = upper
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    compact.push('~');
    compact
}

fn smoothstep(value: f32) -> f32 {
    value * value * (3.0 - 2.0 * value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::scan_directory;
    use std::fs;
    use std::os::unix::fs::symlink;

    #[test]
    fn files_are_storeys_and_directories_become_towers() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("directory")).unwrap();
        fs::write(temp.path().join("file.txt"), b"data").unwrap();
        let root = temp.path().to_path_buf();
        let entries = scan_directory(&root).unwrap();
        let mut scene = Scene::new(root.clone());
        scene.update(root, entries, &[]);

        assert_eq!(scene.towers[&scene.root].children.len(), 2);
        assert!(scene.towers.contains_key(&temp.path().join("directory")));
        assert!(!scene.towers.contains_key(&temp.path().join("file.txt")));
    }

    #[test]
    fn directory_symlinks_remain_file_entries() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("target")).unwrap();
        symlink(temp.path().join("target"), temp.path().join("link")).unwrap();
        let root = temp.path().to_path_buf();
        let mut scene = Scene::new(root.clone());
        scene.update(root, scan_directory(temp.path()).unwrap(), &[]);

        assert!(scene.towers.contains_key(&temp.path().join("target")));
        assert!(!scene.towers.contains_key(&temp.path().join("link")));
    }

    #[test]
    fn navigator_selection_is_reflected_on_the_tower() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a"), b"a").unwrap();
        fs::write(temp.path().join("b"), b"b").unwrap();
        let root = temp.path().to_path_buf();
        let entries = scan_directory(&root).unwrap();
        let mut scene = Scene::new(root.clone());
        scene.update(root, entries, &[]);

        let selected = scene
            .towers
            .get(&scene.root)
            .unwrap()
            .children
            .iter()
            .find(|entry| entry.display_name == "b")
            .unwrap()
            .id;
        scene.select_entry(Some(selected));
        assert_eq!(scene.towers[&scene.root].selected, Some(selected));
    }

    #[test]
    fn storey_selection_does_not_move_or_restart_the_camera() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a"), b"a").unwrap();
        fs::write(temp.path().join("b"), b"b").unwrap();
        let root = temp.path().to_path_buf();
        let entries = scan_directory(&root).unwrap();
        let mut scene = Scene::new(root.clone());
        scene.update(root, entries, &[]);
        let flight_started = scene.flight.started;
        let focus_target = scene.focus_target();

        let selected = scene.towers[&scene.root].children[1].id;
        scene.select_entry(Some(selected));

        assert_eq!(scene.flight.started, flight_started);
        assert_eq!(scene.focus_target(), focus_target);
    }

    #[test]
    fn storey_order_matches_downward_list_navigation() {
        let top = storey_height(0, 4, 8.0);
        let next = storey_height(1, 4, 8.0);
        let bottom = storey_height(3, 4, 8.0);

        assert!(top > next);
        assert!(next > bottom);
    }

    #[test]
    fn filesystem_mutations_create_semantic_scene_effects() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let mut scene = Scene::new(root.clone());
        scene.update(root.clone(), Vec::new(), &[]);
        fs::write(root.join("incoming"), b"data").unwrap();
        let entries = scan_directory(&root).unwrap();
        let created = entries[0].clone();
        scene.update(root, entries, &[FileMutation::Created(created)]);

        assert!(
            scene
                .file_effects
                .iter()
                .any(|effect| matches!(effect.kind, FileEffectKind::Create))
        );
    }

    #[test]
    fn zero_motion_finishes_a_flight_immediately() {
        let flight = CameraFlight::travelling(
            Instant::now(),
            Vec3::ZERO,
            Vec3::new(10.0, 1.0, 0.0),
            FlightParameters {
                route_distance: 10.0,
                view_from: Vec3::Z,
                framing_height_from: 4.0,
                state: VisualState::Transit,
                motion_scale: 0.0,
            },
        );
        assert!(flight.duration.is_zero());
        assert_eq!(flight.to, Vec3::new(10.0, 1.0, 0.0));
    }

    #[test]
    fn distant_filesystem_paths_get_longer_flights() {
        assert!(path_distance(Path::new("/a/b/c"), Path::new("/a/b/d")) == 2);
        assert!(path_distance(Path::new("/a/b/c"), Path::new("/x/y/z")) == 6);
        assert!(
            flight_duration(30.0, VisualState::Transit)
                > flight_duration(5.0, VisualState::Transit)
        );
    }

    #[test]
    fn directory_flights_use_a_curved_world_space_route() {
        let now = Instant::now();
        let from = Vec3::ZERO;
        let to = Vec3::new(10.0, 2.0, 4.0);
        let flight = CameraFlight::travelling(
            now,
            from,
            to,
            FlightParameters {
                route_distance: 12.0,
                view_from: Vec3::Z,
                framing_height_from: 4.0,
                state: VisualState::Transit,
                motion_scale: 1.0,
            },
        );
        let midpoint = cubic_bezier(
            flight.from,
            flight.control_a,
            flight.control_b,
            flight.to,
            0.5,
        );

        assert_eq!(
            cubic_bezier(from, flight.control_a, flight.control_b, to, 0.0),
            from
        );
        assert_eq!(
            cubic_bezier(from, flight.control_a, flight.control_b, to, 1.0),
            to
        );
        assert!(midpoint.distance(from.lerp(to, 0.5)) > 0.25);
    }

    #[test]
    fn reverse_flight_retraces_the_same_curve() {
        let now = Instant::now();
        let from = Vec3::new(-7.0, 1.0, 3.0);
        let to = Vec3::new(11.0, 4.0, -8.0);
        let parameters = FlightParameters {
            route_distance: 22.0,
            view_from: Vec3::Z,
            framing_height_from: 4.0,
            state: VisualState::Transit,
            motion_scale: 1.0,
        };
        let forward = CameraFlight::travelling(now, from, to, parameters);
        let reverse = CameraFlight::travelling(now, to, from, parameters);

        assert!((forward.lift_factor - reverse.lift_factor).abs() < f32::EPSILON);

        for step in 0..=10 {
            let t = step as f32 / 10.0;
            let forward_point = cubic_bezier(
                forward.from,
                forward.control_a,
                forward.control_b,
                forward.to,
                t,
            );
            let reverse_point = cubic_bezier(
                reverse.from,
                reverse.control_a,
                reverse.control_b,
                reverse.to,
                1.0 - t,
            );
            assert!(forward_point.distance(reverse_point) < 0.0001);
        }
    }

    #[test]
    fn city_vectors_receive_a_wide_range_of_stable_lift_heights() {
        let from = Vec3::ZERO;
        let lifts = (1..=24)
            .map(|index| {
                let x = (index % 6) as f32 * TOWER_SPACING;
                let z = (index / 6) as f32 * -TOWER_SPACING;
                CameraFlight::travelling(
                    Instant::now(),
                    from,
                    Vec3::new(x, 2.0, z),
                    FlightParameters {
                        route_distance: from.distance(Vec3::new(x, 2.0, z)),
                        view_from: Vec3::Z,
                        framing_height_from: 4.0,
                        state: VisualState::Transit,
                        motion_scale: 1.0,
                    },
                )
                .lift_factor
            })
            .collect::<Vec<_>>();
        let lowest = lifts.iter().copied().fold(f32::INFINITY, f32::min);
        let highest = lifts.iter().copied().fold(f32::NEG_INFINITY, f32::max);

        assert!(lowest < 0.20);
        assert!(highest > 0.80);
    }

    #[test]
    fn visited_towers_keep_their_city_position() {
        let temp = tempfile::tempdir().unwrap();
        let child = temp.path().join("child");
        fs::create_dir(&child).unwrap();
        let root = temp.path().to_path_buf();
        let mut scene = Scene::new(root.clone());
        scene.update(root, scan_directory(temp.path()).unwrap(), &[]);
        let before = scene.towers.get(&child).unwrap().position;
        scene.update(child.clone(), scan_directory(&child).unwrap(), &[]);
        assert_eq!(scene.towers.get(&child).unwrap().position, before);
    }

    #[test]
    fn removed_directories_leave_the_visible_city() {
        let temp = tempfile::tempdir().unwrap();
        let child = temp.path().join("child");
        fs::create_dir(&child).unwrap();
        let root = temp.path().to_path_buf();
        let mut scene = Scene::new(root.clone());
        scene.update(root.clone(), scan_directory(&root).unwrap(), &[]);
        assert!(scene.towers.contains_key(&child));

        fs::remove_dir(&child).unwrap();
        scene.update(root.clone(), scan_directory(&root).unwrap(), &[]);
        assert!(!scene.towers.contains_key(&child));
    }

    #[test]
    fn camera_faces_the_open_side_of_a_directory_cluster() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["a", "b", "c"] {
            fs::create_dir(temp.path().join(name)).unwrap();
        }
        let root = temp.path().to_path_buf();
        let mut scene = Scene::new(root.clone());
        scene.update(root, scan_directory(temp.path()).unwrap(), &[]);
        let current = scene.towers.get(scene.root()).unwrap();
        let children = current
            .children
            .iter()
            .filter_map(|entry| scene.towers.get(&entry.path))
            .fold(Vec3::ZERO, |total, tower| {
                total + tower.position - current.position
            });
        assert!(scene.camera_direction_target().dot(children) < 0.0);
    }

    #[test]
    fn settled_view_culls_only_a_tower_between_the_camera_and_current_tower() {
        let temp = tempfile::tempdir().unwrap();
        let blocker = temp.path().join("blocker");
        fs::create_dir(&blocker).unwrap();
        let root = temp.path().to_path_buf();
        let mut scene = Scene::new(root.clone());
        scene.update(root.clone(), scan_directory(&root).unwrap(), &[]);
        scene.towers.get_mut(&root).unwrap().position = Vec3::ZERO;
        let eye = Vec3::new(0.0, 3.0, -10.0);
        let focus = Vec3::new(0.0, 3.0, 0.0);
        scene.towers.get_mut(&blocker).unwrap().position = Vec3::new(0.0, 0.0, -5.0);
        assert!(tower_occludes_point(
            scene.towers.get(&blocker).unwrap(),
            eye,
            focus,
        ));
        assert!(!scene.tower_is_visible(
            scene.towers.get(&blocker).unwrap(),
            VisualState::Settled,
            eye,
            focus,
        ));

        scene.towers.get_mut(&blocker).unwrap().position.x = 8.0;
        assert!(!tower_occludes_point(
            scene.towers.get(&blocker).unwrap(),
            eye,
            focus,
        ));
        assert!(scene.tower_is_visible(
            scene.towers.get(&blocker).unwrap(),
            VisualState::Settled,
            eye,
            focus,
        ));

        scene.towers.get_mut(&blocker).unwrap().position = Vec3::new(0.0, 0.0, 5.0);
        assert!(!tower_occludes_point(
            scene.towers.get(&blocker).unwrap(),
            eye,
            focus,
        ));
        assert!(scene.tower_is_visible(
            scene.towers.get(&blocker).unwrap(),
            VisualState::Settled,
            eye,
            focus,
        ));
    }
}
