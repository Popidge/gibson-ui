use crate::config::{FrameRate, Settings, SettingsSnapshot, ThemeBackdrop, VisualStyle};
use crate::layout::{CockpitLayout, LayoutPreferences, PaneTarget, Rect, Splitter};
use crate::navigation::VisualState;
use crate::navigator::NavigatorSnapshot;
use crate::scene::{
    CameraSubject, FacePanel, LightningOptions, MAX_RENDER_OBJECTS, RenderObject, Scene,
    SceneRenderContext, TowerLabel,
};
use crate::system_load::SystemLoad;
use crate::terminal::{TerminalColor, TerminalSnapshot, TerminalSpan};
use crate::theme::OmarchyTheme;
use anyhow::Context;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3};
use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, Style,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight, Wrap,
};
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, warn};
use wgpu::util::DeviceExt;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

const TERMINAL_FONT_SIZE: f32 = 15.0;
const TERMINAL_LINE_HEIGHT: f32 = 19.0;
const TERMINAL_CELL_WIDTH: f32 = 9.0;
const PANE_HUD_HEIGHT: f32 = 23.0;
const TERMINAL_HORIZONTAL_PADDING: f32 = 12.0;
const TERMINAL_CONTENT_GAP: f32 = 6.0;
const TERMINAL_BOTTOM_PADDING: f32 = 10.0;
const MAX_UI_RECTS: usize = 2_048;
const MICHROMA_FAMILY: &str = "Michroma";
const FACE_SWEEP_RADIANS: f32 = std::f32::consts::FRAC_PI_4;
const NAVIGATOR_TEXTURE_WIDTH: u32 = 768;
const NAVIGATOR_TEXTURE_HEIGHT: u32 = 1_536;
const NAVIGATOR_TEXTURE_SCALE: f32 = 2.0;
const NAVIGATOR_TEXTURE_LEFT: f32 = 24.0;
const NAVIGATOR_TEXTURE_TOP: f32 = 32.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
}

impl Vertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct InstanceRaw {
    model: [[f32; 4]; 4],
    color: [f32; 4],
}

impl InstanceRaw {
    const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        2 => Float32x4,
        3 => Float32x4,
        4 => Float32x4,
        5 => Float32x4,
        6 => Float32x4
    ];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    camera_time: [f32; 4],
    pulse: [f32; 4],
    primary: [f32; 4],
    secondary: [f32; 4],
    accent: [f32; 4],
    background: [f32; 4],
    active_face: [f32; 4],
    render_size: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OverlayUniforms {
    visualiser: [f32; 4],
    terminal: [f32; 4],
    file_menu: [f32; 4],
    metadata: [f32; 4],
    settings: [f32; 4],
    hud: [f32; 4],
    terminal_hud: [f32; 4],
    terminal_focus: [f32; 4],
    primary: [f32; 4],
    secondary: [f32; 4],
    panel_background: [f32; 4],
    options: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BackgroundUniforms {
    background: [f32; 4],
    options: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UiRect {
    rect: [f32; 4],
    color: [f32; 4],
}

impl UiRect {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct CameraPose {
    focus: Vec3,
    distance: f32,
    elevation: f32,
    field_of_view: f32,
}

#[derive(Clone, Copy, Debug)]
struct CameraFrame {
    eye: Vec3,
    view_projection: Mat4,
    flight_intensity: f32,
    face_outward: Vec3,
}

#[derive(Clone, Copy, Debug)]
struct TextPlacement {
    view_projection: Mat4,
    visualiser: Rect,
    projected_panel: Option<ProjectedFacePanel>,
    metadata_rect: Option<Rect>,
    settings_rect: Option<Rect>,
}

struct HudContent<'a> {
    visualiser: &'a str,
    terminal: Option<&'a str>,
    terminal_focused: bool,
}

pub struct RenderStatus<'a> {
    pub terminal_focused: bool,
    pub navigation_pending: bool,
    pub visual_state: VisualState,
    pub settings: Option<&'a SettingsSnapshot>,
}

struct PerformanceStats {
    log_enabled: bool,
    interval_started: Instant,
    last_log: Instant,
    frames: u64,
    total_cpu: Duration,
    scene_cpu: Duration,
    labels_cpu: Duration,
    text_prepare_cpu: Duration,
    latest: PerformanceSample,
}

#[derive(Clone, Copy, Debug, Default)]
struct PerformanceSample {
    fps: f64,
    frame_cpu_ms: f64,
    scene_cpu_ms: f64,
    object_count: usize,
    label_count: usize,
}

impl PerformanceStats {
    fn new(now: Instant, configured: bool) -> Self {
        Self {
            log_enabled: configured || std::env::var_os("GIBSON_PERF").is_some(),
            interval_started: now,
            last_log: now,
            frames: 0,
            total_cpu: Duration::ZERO,
            scene_cpu: Duration::ZERO,
            labels_cpu: Duration::ZERO,
            text_prepare_cpu: Duration::ZERO,
            latest: PerformanceSample::default(),
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.log_enabled = enabled || std::env::var_os("GIBSON_PERF").is_some();
        self.interval_started = Instant::now();
        self.last_log = self.interval_started;
        self.frames = 0;
        self.total_cpu = Duration::ZERO;
        self.scene_cpu = Duration::ZERO;
        self.labels_cpu = Duration::ZERO;
        self.text_prepare_cpu = Duration::ZERO;
    }

    fn record(
        &mut self,
        total_cpu: Duration,
        scene_cpu: Duration,
        labels_cpu: Duration,
        text_prepare_cpu: Duration,
        object_count: usize,
        label_count: usize,
    ) {
        self.frames += 1;
        self.total_cpu += total_cpu;
        self.scene_cpu += scene_cpu;
        self.labels_cpu += labels_cpu;
        self.text_prepare_cpu += text_prepare_cpu;
        let elapsed = self.interval_started.elapsed();
        if elapsed < Duration::from_millis(500) {
            return;
        }

        let frames = self.frames.max(1) as f64;
        self.latest = PerformanceSample {
            fps: self.frames as f64 / elapsed.as_secs_f64(),
            frame_cpu_ms: self.total_cpu.as_secs_f64() * 1_000.0 / frames,
            scene_cpu_ms: self.scene_cpu.as_secs_f64() * 1_000.0 / frames,
            object_count,
            label_count,
        };
        if self.log_enabled && self.last_log.elapsed() >= Duration::from_secs(2) {
            eprintln!(
                "GIBSON PERF fps={:.1} frame_cpu_ms={:.3} scene_cpu_ms={:.3} \
                 labels_cpu_ms={:.3} text_cpu_ms={:.3} objects={} labels={}",
                self.latest.fps,
                self.latest.frame_cpu_ms,
                self.latest.scene_cpu_ms,
                self.labels_cpu.as_secs_f64() * 1_000.0 / frames,
                self.text_prepare_cpu.as_secs_f64() * 1_000.0 / frames,
                self.latest.object_count,
                self.latest.label_count,
            );
            self.last_log = Instant::now();
        }
        self.interval_started = Instant::now();
        self.frames = 0;
        self.total_cpu = Duration::ZERO;
        self.scene_cpu = Duration::ZERO;
        self.labels_cpu = Duration::ZERO;
        self.text_prepare_cpu = Duration::ZERO;
    }

    fn hud_text(&self, width: f32, navigation_pending: bool) -> String {
        let text = format_performance_hud(self.latest, width);
        if navigation_pending {
            text.replacen("GIBSON // ", "GIBSON // CD PENDING // ", 1)
        } else {
            text
        }
    }
}

struct SceneTargets {
    color_view: wgpu::TextureView,
    color_group: wgpu::BindGroup,
    blur_a_view: wgpu::TextureView,
    blur_a_group: wgpu::BindGroup,
    blur_b_view: wgpu::TextureView,
    blur_b_group: wgpu::BindGroup,
}

pub struct Renderer {
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    present_modes: Vec<wgpu::PresentMode>,
    pipeline: wgpu::RenderPipeline,
    glass_pipeline: wgpu::RenderPipeline,
    scene_copy_pipeline: wgpu::RenderPipeline,
    blur_horizontal_pipeline: wgpu::RenderPipeline,
    blur_vertical_pipeline: wgpu::RenderPipeline,
    background_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    ui_rect_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    instance_buffer: wgpu::Buffer,
    render_objects: Vec<RenderObject>,
    glass_objects: Vec<RenderObject>,
    instances: Vec<InstanceRaw>,
    uniform_buffer: wgpu::Buffer,
    uniform_group: wgpu::BindGroup,
    navigator_texture_view: wgpu::TextureView,
    navigator_texture_group: wgpu::BindGroup,
    postprocess_texture_layout: wgpu::BindGroupLayout,
    scene_targets: SceneTargets,
    overlay_uniform_buffer: wgpu::Buffer,
    overlay_uniform_group: wgpu::BindGroup,
    background_uniform_buffer: wgpu::Buffer,
    background_texture_group: wgpu::BindGroup,
    background_texture_layout: wgpu::BindGroupLayout,
    background_image_aspect: f32,
    background_image_loaded: bool,
    ui_rect_buffer: wgpu::Buffer,
    ui_rects: Vec<UiRect>,
    depth_view: wgpu::TextureView,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    navigator_texture_viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    scene_text_renderer: TextRenderer,
    navigator_texture_renderer: TextRenderer,
    terminal_buffer: Buffer,
    navigator_buffer: Buffer,
    metadata_buffer: Buffer,
    settings_buffer: Buffer,
    hud_buffer: Buffer,
    terminal_hud_buffer: Buffer,
    cursor_buffer: Buffer,
    tower_label_buffers: Vec<Buffer>,
    tower_label_text: Vec<String>,
    last_terminal_fingerprint: u64,
    last_navigator_fingerprint: u64,
    navigator_texture_dirty: bool,
    last_metadata_fingerprint: u64,
    last_settings_fingerprint: u64,
    last_hud_text: String,
    last_terminal_hud_text: String,
    terminal_theme: TerminalTheme,
    settings: Settings,
    theme: OmarchyTheme,
    layout_preferences: LayoutPreferences,
    layout: CockpitLayout,
    active_splitter: Option<Splitter>,
    navigator_hit_rect: Option<Rect>,
    started: Instant,
    last_frame: Instant,
    camera_focus: Vec3,
    camera_distance: f32,
    camera_elevation: f32,
    camera_field_of_view: f32,
    camera_ready: bool,
    sweep_blend: f32,
    orbit_angle: f32,
    orbit_target: f32,
    performance: PerformanceStats,
    system_load: SystemLoad,
    adapter_name: String,
    window: Arc<Window>,
}

impl Renderer {
    pub async fn new(
        window: Arc<Window>,
        event_loop: &ActiveEventLoop,
        settings: Settings,
        theme: OmarchyTheme,
    ) -> anyhow::Result<Self> {
        let theme = if settings.appearance.follow_omarchy {
            theme
        } else {
            OmarchyTheme::classic()
        };
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(event_loop.owned_display_handle()),
        ));
        let surface = instance
            .create_surface(window.clone())
            .context("create Wayland surface")?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            })
            .await
            .context("find a compatible GPU")?;
        let adapter_name = adapter.get_info().name;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("GIBSON device"),
                ..Default::default()
            })
            .await
            .context("create GPU device")?;
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| capabilities.formats.first().copied())
            .context("surface exposes no texture formats")?;
        let present_mode =
            select_present_mode(&capabilities.present_modes, settings.graphics.frame_rate)
                .context("surface exposes no presentation modes")?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene uniforms"),
            size: mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene uniform layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let uniform_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene uniform group"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let navigator_texture_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("tower navigator texture layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let (navigator_texture_view, navigator_texture_group) =
            create_navigator_texture(&device, &navigator_texture_layout, format);
        let postprocess_texture_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("scene post-process texture layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let scene_targets = create_scene_targets(
            &device,
            &postprocess_texture_layout,
            format,
            config.width,
            config.height,
        );
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene pipeline layout"),
            bind_group_layouts: &[
                Some(&uniform_layout),
                Some(&navigator_texture_layout),
                Some(&postprocess_texture_layout),
            ],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(Vertex::layout()), Some(InstanceRaw::layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let glass_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glass tower pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(Vertex::layout()), Some(InstanceRaw::layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let postprocess_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene blur shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blur.wgsl").into()),
        });
        let postprocess_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene post-process pipeline layout"),
            bind_group_layouts: &[Some(&postprocess_texture_layout)],
            immediate_size: 0,
        });
        let create_postprocess_pipeline = |label, entry_point| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&postprocess_layout),
                vertex: wgpu::VertexState {
                    module: &postprocess_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &postprocess_shader,
                    entry_point: Some(entry_point),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let scene_copy_pipeline = create_postprocess_pipeline("scene copy pipeline", "fs_copy");
        let blur_horizontal_pipeline =
            create_postprocess_pipeline("horizontal glass blur pipeline", "fs_horizontal");
        let blur_vertical_pipeline =
            create_postprocess_pipeline("vertical glass blur pipeline", "fs_vertical");

        let background_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("theme background shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("background.wgsl").into()),
        });
        let background_uniform_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("theme background uniforms"),
                contents: bytemuck::bytes_of(&BackgroundUniforms::zeroed()),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let background_texture_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("theme background layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let background_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("theme background pipeline layout"),
            bind_group_layouts: &[Some(&background_texture_layout)],
            immediate_size: 0,
        });
        let background_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("theme background pipeline"),
            layout: Some(&background_layout),
            vertex: wgpu::VertexState {
                module: &background_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &background_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let wallpaper = (settings.appearance.backdrop == ThemeBackdrop::Wallpaper)
            .then_some(theme.wallpaper.as_deref())
            .flatten();
        let (background_texture_group, background_image_aspect, background_image_loaded) =
            create_background_group(
                &device,
                &queue,
                &background_texture_layout,
                &background_uniform_buffer,
                wallpaper,
            );

        let overlay_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terminal overlay shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("overlay.wgsl").into()),
        });
        let overlay_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cockpit overlay uniforms"),
            contents: bytemuck::bytes_of(&OverlayUniforms::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let overlay_uniform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cockpit overlay uniform layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let overlay_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terminal overlay layout"),
            bind_group_layouts: &[Some(&overlay_uniform_layout)],
            immediate_size: 0,
        });
        let overlay_uniform_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cockpit overlay uniform group"),
            layout: &overlay_uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: overlay_uniform_buffer.as_entire_binding(),
            }],
        });
        let overlay_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terminal overlay pipeline"),
            layout: Some(&overlay_layout),
            vertex: wgpu::VertexState {
                module: &overlay_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &overlay_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let ui_rect_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("UI rectangle shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ui_rect.wgsl").into()),
        });
        let ui_rect_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("UI rectangle pipeline layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let ui_rect_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("UI rectangle pipeline"),
            layout: Some(&ui_rect_layout),
            vertex: wgpu::VertexState {
                module: &ui_rect_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(UiRect::layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &ui_rect_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let ui_rect_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("UI rectangles"),
            size: (MAX_UI_RECTS * mem::size_of::<UiRect>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (vertices, indices) = cube_mesh();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene instances"),
            size: ((MAX_RENDER_OBJECTS + 1) * mem::size_of::<InstanceRaw>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let depth_view = create_depth_view(&device, config.width, config.height);

        let terminal_theme = TerminalTheme::load();
        let mut font_system = FontSystem::new();
        font_system
            .db_mut()
            .load_font_data(include_bytes!("../assets/fonts/Michroma-Regular.ttf").to_vec());
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut navigator_texture_viewport = Viewport::new(&device, &cache);
        navigator_texture_viewport.update(
            &queue,
            Resolution {
                width: NAVIGATOR_TEXTURE_WIDTH,
                height: NAVIGATOR_TEXTURE_HEIGHT,
            },
        );
        let mut atlas = TextAtlas::new(&device, &queue, &cache, format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let navigator_texture_renderer =
            TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let scene_text_renderer = TextRenderer::new(
            &mut atlas,
            &device,
            wgpu::MultisampleState::default(),
            Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
        );
        let mut terminal_buffer = Buffer::new(
            &mut font_system,
            Metrics::new(TERMINAL_FONT_SIZE, TERMINAL_LINE_HEIGHT),
        );
        terminal_buffer.set_wrap(Wrap::None);
        terminal_buffer.set_monospace_width(Some(TERMINAL_CELL_WIDTH));
        let mut cursor_buffer = Buffer::new(
            &mut font_system,
            Metrics::new(TERMINAL_FONT_SIZE, TERMINAL_LINE_HEIGHT),
        );
        cursor_buffer.set_text(
            "█",
            &Attrs::new().family(Family::Monospace),
            Shaping::Basic,
            None,
        );
        cursor_buffer.set_monospace_width(Some(TERMINAL_CELL_WIDTH));
        cursor_buffer.shape_until_scroll(&mut font_system, false);
        let mut navigator_buffer = Buffer::new(&mut font_system, Metrics::new(12.0, 17.0));
        navigator_buffer.set_wrap(Wrap::None);
        navigator_buffer.set_size(Some(640.0), Some(640.0));
        navigator_buffer.set_text(
            "FILES // INITIALIZING...",
            &Attrs::new().family(Family::Monospace),
            Shaping::Basic,
            None,
        );
        navigator_buffer.shape_until_scroll(&mut font_system, false);
        let mut metadata_buffer = Buffer::new(&mut font_system, Metrics::new(12.0, 17.0));
        metadata_buffer.set_wrap(Wrap::WordOrGlyph);
        metadata_buffer.set_size(Some(300.0), Some(150.0));
        metadata_buffer.set_text(
            "METADATA // INITIALIZING...",
            &Attrs::new().family(Family::Monospace),
            Shaping::Basic,
            None,
        );
        metadata_buffer.shape_until_scroll(&mut font_system, false);
        let mut settings_buffer = Buffer::new(&mut font_system, Metrics::new(14.0, 21.0));
        settings_buffer.set_wrap(Wrap::None);
        settings_buffer.set_size(Some(720.0), Some(520.0));
        settings_buffer.set_text(
            "GIBSON // SETTINGS",
            &Attrs::new().family(Family::Name(MICHROMA_FAMILY)),
            Shaping::Advanced,
            None,
        );
        settings_buffer.shape_until_scroll(&mut font_system, false);
        let mut hud_buffer = Buffer::new(&mut font_system, Metrics::new(10.0, 15.0));
        hud_buffer.set_wrap(Wrap::None);
        hud_buffer.set_size(Some(config.width as f32), Some(20.0));
        hud_buffer.set_text(
            "GIBSON // FPS -- // L/R ORBIT // R RESET // F2 FOCUS // F3 TERMINAL // F4 FILES // F10 SETTINGS",
            &Attrs::new().family(Family::Name(MICHROMA_FAMILY)),
            Shaping::Advanced,
            None,
        );
        hud_buffer.shape_until_scroll(&mut font_system, false);
        let mut terminal_hud_buffer = Buffer::new(&mut font_system, Metrics::new(10.0, 15.0));
        terminal_hud_buffer.set_wrap(Wrap::None);
        terminal_hud_buffer.set_size(Some(config.width as f32), Some(20.0));
        terminal_hud_buffer.set_text(
            "TERMINAL // ACTIVE // F2 (FOCUS) // F3 (HIDE)",
            &Attrs::new().family(Family::Name(MICHROMA_FAMILY)),
            Shaping::Advanced,
            None,
        );
        terminal_hud_buffer.shape_until_scroll(&mut font_system, false);

        let layout_preferences = LayoutPreferences::load();
        let layout = CockpitLayout::calculate(config.width, config.height, layout_preferences);

        let started = Instant::now();
        Ok(Self {
            instance,
            surface,
            device,
            queue,
            config,
            present_modes: capabilities.present_modes,
            pipeline,
            glass_pipeline,
            scene_copy_pipeline,
            blur_horizontal_pipeline,
            blur_vertical_pipeline,
            background_pipeline,
            overlay_pipeline,
            ui_rect_pipeline,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            instance_buffer,
            render_objects: Vec::with_capacity(MAX_RENDER_OBJECTS),
            glass_objects: Vec::with_capacity(128),
            instances: Vec::with_capacity(MAX_RENDER_OBJECTS + 1),
            uniform_buffer,
            uniform_group,
            navigator_texture_view,
            navigator_texture_group,
            postprocess_texture_layout,
            scene_targets,
            overlay_uniform_buffer,
            overlay_uniform_group,
            background_uniform_buffer,
            background_texture_group,
            background_texture_layout,
            background_image_aspect,
            background_image_loaded,
            ui_rect_buffer,
            ui_rects: Vec::with_capacity(256),
            depth_view,
            font_system,
            swash_cache,
            viewport,
            navigator_texture_viewport,
            atlas,
            text_renderer,
            scene_text_renderer,
            navigator_texture_renderer,
            terminal_buffer,
            navigator_buffer,
            metadata_buffer,
            settings_buffer,
            hud_buffer,
            terminal_hud_buffer,
            cursor_buffer,
            tower_label_buffers: Vec::new(),
            tower_label_text: Vec::new(),
            last_terminal_fingerprint: 0,
            last_navigator_fingerprint: 0,
            navigator_texture_dirty: true,
            last_metadata_fingerprint: 0,
            last_settings_fingerprint: 0,
            last_hud_text: String::new(),
            last_terminal_hud_text: String::new(),
            terminal_theme,
            settings: settings.clone(),
            theme,
            layout_preferences,
            layout,
            active_splitter: None,
            navigator_hit_rect: None,
            started,
            last_frame: started,
            camera_focus: Vec3::ZERO,
            camera_distance: 18.0,
            camera_elevation: 0.52,
            camera_field_of_view: 49.0,
            camera_ready: false,
            sweep_blend: 0.0,
            orbit_angle: 0.0,
            orbit_target: 0.0,
            performance: PerformanceStats::new(started, settings.graphics.performance_log),
            system_load: SystemLoad::new(started),
            adapter_name,
            window,
        })
    }

    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    pub fn apply_settings(&mut self, settings: &Settings, omarchy_theme: &OmarchyTheme) {
        let backdrop_changed = self.settings.appearance.backdrop != settings.appearance.backdrop;
        let theme_source_changed =
            self.settings.appearance.follow_omarchy != settings.appearance.follow_omarchy;
        let next_present_mode =
            select_present_mode(&self.present_modes, settings.graphics.frame_rate)
                .unwrap_or(self.config.present_mode);
        if next_present_mode != self.config.present_mode {
            self.config.present_mode = next_present_mode;
            self.surface.configure(&self.device, &self.config);
        }
        self.performance
            .set_enabled(settings.graphics.performance_log);
        self.settings = settings.clone();
        if !self.settings.behaviour.orbit_enabled {
            self.orbit_target = 0.0;
        }
        if theme_source_changed {
            let theme = if settings.appearance.follow_omarchy {
                omarchy_theme.clone()
            } else {
                OmarchyTheme::classic()
            };
            self.set_theme(theme, true);
        } else if backdrop_changed {
            self.reload_background();
        }
    }

    pub fn apply_omarchy_theme(&mut self, theme: &OmarchyTheme) {
        self.terminal_theme = TerminalTheme::load();
        self.last_terminal_fingerprint = u64::MAX;
        if self.settings.appearance.follow_omarchy {
            self.set_theme(theme.clone(), true);
        }
    }

    pub fn rotate_orbit(&mut self, direction: isize) -> bool {
        if !self.settings.behaviour.orbit_enabled || direction == 0 {
            return false;
        }
        self.orbit_target += direction.signum() as f32 * std::f32::consts::FRAC_PI_2;
        true
    }

    pub fn reset_orbit(&mut self) {
        self.orbit_target = 0.0;
    }

    fn set_theme(&mut self, theme: OmarchyTheme, reload_background: bool) {
        self.theme = theme;
        self.last_navigator_fingerprint = u64::MAX;
        self.navigator_texture_dirty = true;
        self.last_metadata_fingerprint = u64::MAX;
        self.last_settings_fingerprint = u64::MAX;
        self.tower_label_text.fill(String::new());
        if reload_background {
            self.reload_background();
        }
    }

    fn reload_background(&mut self) {
        let wallpaper = (self.settings.appearance.backdrop == ThemeBackdrop::Wallpaper)
            .then_some(self.theme.wallpaper.as_deref())
            .flatten();
        let (group, aspect, loaded) = create_background_group(
            &self.device,
            &self.queue,
            &self.background_texture_layout,
            &self.background_uniform_buffer,
            wallpaper,
        );
        self.background_texture_group = group;
        self.background_image_aspect = aspect;
        self.background_image_loaded = loaded;
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = create_depth_view(&self.device, width, height);
        self.scene_targets = create_scene_targets(
            &self.device,
            &self.postprocess_texture_layout,
            self.config.format,
            width,
            height,
        );
        self.recalculate_layout();
    }

    pub fn sync_window_size(&mut self) -> bool {
        let size = self.window.inner_size();
        if size.width == self.config.width && size.height == self.config.height {
            return false;
        }
        self.resize(size.width, size.height);
        true
    }

    pub fn pane_at(&self, x: f32, y: f32) -> PaneTarget {
        let layout_target = self.layout.hit_test(x, y);
        if layout_target != PaneTarget::Visualiser {
            return layout_target;
        }
        if self.layout_preferences.navigator_visible && self.layout.visualiser.contains(x, y) {
            PaneTarget::Navigator
        } else {
            PaneTarget::Visualiser
        }
    }

    pub fn begin_splitter_drag(&mut self, x: f32, y: f32) -> bool {
        self.active_splitter = self.layout.splitter_at(x, y);
        self.active_splitter.is_some()
    }

    pub fn drag_splitter(&mut self, x: f32, y: f32) -> bool {
        let Some(splitter) = self.active_splitter else {
            return false;
        };
        let width = self.config.width.max(1) as f32;
        let height = self.config.height.max(1) as f32;
        match splitter {
            Splitter::Dock => {
                self.layout_preferences.dock_ratio = ((width - x) / width).clamp(0.22, 0.50);
            }
            Splitter::Drawer => {
                self.layout_preferences.drawer_ratio = ((height - y) / height).clamp(0.18, 0.48);
            }
        }
        self.recalculate_layout();
        true
    }

    pub fn end_splitter_drag(&mut self) -> bool {
        if self.active_splitter.take().is_none() {
            return false;
        }
        self.save_layout_preferences();
        true
    }

    pub fn toggle_terminal(&mut self) -> bool {
        self.layout_preferences.terminal_visible = !self.layout_preferences.terminal_visible;
        self.recalculate_layout();
        self.save_layout_preferences();
        self.layout_preferences.terminal_visible
    }

    pub fn toggle_navigator(&mut self) -> bool {
        self.layout_preferences.navigator_visible = !self.layout_preferences.navigator_visible;
        self.recalculate_layout();
        self.save_layout_preferences();
        self.layout_preferences.navigator_visible
    }

    pub fn terminal_visible(&self) -> bool {
        self.layout.terminal.is_some()
    }

    pub fn navigator_visible(&self) -> bool {
        self.layout_preferences.navigator_visible
    }

    fn save_layout_preferences(&self) {
        if let Err(error) = self.layout_preferences.save() {
            warn!(%error, "cannot save cockpit layout");
        }
    }

    pub fn navigator_dimensions(&self) -> (usize, usize) {
        if self.navigator_hit_rect.is_some() {
            (28, 42)
        } else {
            (20, 34)
        }
    }

    pub fn navigator_row_at(&self, x: f32, y: f32) -> Option<usize> {
        let pane = self.navigator_hit_rect?;
        if !pane.contains(x, y) {
            return None;
        }
        let texture_y = (y - pane.y) / pane.height.max(1.0) * NAVIGATOR_TEXTURE_HEIGHT as f32;
        let line = ((texture_y - NAVIGATOR_TEXTURE_TOP).max(0.0) / (17.0 * NAVIGATOR_TEXTURE_SCALE))
            .floor() as usize;
        line.checked_sub(2)
    }

    pub fn terminal_dimensions(&self) -> (u16, u16, u16, u16) {
        let (width, height) = self.layout.terminal.map_or((800, 456), |terminal| {
            let content = terminal_content_rect(terminal);
            (content.width as u32, content.height as u32)
        });
        let cols = (width as f32 / TERMINAL_CELL_WIDTH).floor().max(10.0) as u16;
        let rows = (height as f32 / TERMINAL_LINE_HEIGHT).floor().max(2.0) as u16;
        (
            rows,
            cols,
            width.min(u16::MAX as u32) as u16,
            height.min(u16::MAX as u32) as u16,
        )
    }

    fn recalculate_layout(&mut self) {
        self.layout = CockpitLayout::calculate(
            self.config.width,
            self.config.height,
            self.layout_preferences,
        );
        self.navigator_buffer.set_size(Some(640.0), Some(640.0));
        let metadata_width = (self.layout.visualiser.width * 0.25).clamp(220.0, 330.0);
        let metadata_height = (self.layout.visualiser.height * 0.23).clamp(150.0, 220.0);
        self.metadata_buffer.set_size(
            Some((metadata_width - 20.0).max(80.0)),
            Some((metadata_height - 18.0).max(40.0)),
        );
        self.navigator_hit_rect = None;
        self.last_terminal_fingerprint = u64::MAX;
        self.last_navigator_fingerprint = u64::MAX;
        self.last_metadata_fingerprint = u64::MAX;
        self.last_settings_fingerprint = u64::MAX;
    }

    fn update_camera(
        &mut self,
        scene: &Scene,
        visual_state: VisualState,
        selection_position: f32,
        now: Instant,
    ) -> CameraFrame {
        let frame_seconds = now
            .duration_since(self.last_frame)
            .as_secs_f32()
            .clamp(0.0, 0.1);
        self.last_frame = now;
        let visualiser = self.layout.visualiser;
        let aspect = visualiser.width / visualiser.height.max(1.0);
        let desired_pose = desired_camera_pose(
            scene.camera_subject(now),
            visual_state,
            1.0,
            selection_position,
        );
        if self.camera_ready {
            let focus_blend = 1.0 - (-frame_seconds * 4.4).exp();
            let pose_blend = 1.0 - (-frame_seconds * 3.6).exp();
            self.camera_focus = self.camera_focus.lerp(desired_pose.focus, focus_blend);
            self.camera_distance += (desired_pose.distance - self.camera_distance) * pose_blend;
            self.camera_elevation += (desired_pose.elevation - self.camera_elevation) * pose_blend;
            self.camera_field_of_view +=
                (desired_pose.field_of_view - self.camera_field_of_view) * pose_blend;
        } else {
            self.camera_focus = desired_pose.focus;
            self.camera_distance = desired_pose.distance;
            self.camera_elevation = desired_pose.elevation;
            self.camera_field_of_view = desired_pose.field_of_view;
            self.camera_ready = true;
        }
        let orbit_blend =
            1.0 - (-frame_seconds * 4.8 * self.settings.graphics.motion_scale.max(0.15)).exp();
        self.orbit_angle += (self.orbit_target - self.orbit_angle) * orbit_blend;
        let sweep_target = if self.settings.behaviour.idle_sweep
            && scene.flight_complete()
            && visual_state == VisualState::Settled
        {
            1.0
        } else {
            0.0
        };
        self.sweep_blend +=
            (sweep_target - self.sweep_blend) * (1.0 - (-frame_seconds * 5.0).exp());
        let sweep = (self.started.elapsed().as_secs_f32() * 0.18).sin()
            * (FACE_SWEEP_RADIANS * 0.24)
            * self.sweep_blend;
        let direction = Quat::from_rotation_y(self.orbit_angle + sweep) * Vec3::NEG_Z;
        let eye = self.camera_focus
            + direction * self.camera_distance
            + Vec3::Y * (self.camera_distance * self.camera_elevation);
        let forward = (self.camera_focus - eye)
            .try_normalize()
            .unwrap_or(-Vec3::Z);
        let up = Quat::from_axis_angle(forward, scene.camera_roll(now)) * Vec3::Y;
        let view = Mat4::look_at_rh(eye, self.camera_focus, up);
        let flight_intensity = scene.flight_intensity(now);
        let field_of_view = self.camera_field_of_view + flight_intensity * 7.0;
        let far_plane = (self.camera_distance * 4.0 + 120.0).max(240.0);
        let projection = Mat4::perspective_rh(field_of_view.to_radians(), aspect, 0.1, far_plane);
        CameraFrame {
            eye,
            view_projection: projection * view,
            flight_intensity,
            face_outward: nearest_tower_face(direction),
        }
    }

    pub fn render(
        &mut self,
        scene: &Scene,
        terminal: &TerminalSnapshot,
        navigator: &NavigatorSnapshot,
        status: RenderStatus<'_>,
    ) -> anyhow::Result<()> {
        let frame_started = Instant::now();
        let RenderStatus {
            terminal_focused: terminal_focus,
            navigation_pending,
            visual_state,
            settings,
        } = status;
        let now = Instant::now();
        let system_load = self.system_load.sample(now);
        let visualiser = self.layout.visualiser;
        let camera = self.update_camera(scene, visual_state, navigator.selection_position, now);
        let eye = camera.eye;
        let view_proj = camera.view_projection;
        let flight_intensity = camera.flight_intensity;
        let visual_style = self.settings.appearance.visual_style;
        let show_navigator = visual_state.shows_file_menu()
            && self.layout_preferences.navigator_visible
            && settings.is_none();
        let face_panel = show_navigator
            .then(|| scene.active_face_panel(camera.face_outward))
            .flatten();

        let scene_started = Instant::now();
        let quality = self.settings.graphics.quality;
        let lightning = if self.settings.graphics.system_lightning {
            LightningOptions {
                load: system_load,
                max_arcs: quality.lightning_arcs(),
                segments: quality.lightning_segments(),
            }
        } else {
            LightningOptions::OFF
        };
        scene.write_render_objects(
            SceneRenderContext {
                now,
                state: visual_state,
                camera_eye: eye,
                camera_focus: self.camera_focus,
                max_objects: quality.max_objects(),
                lightning,
                visual_style,
            },
            &mut self.render_objects,
        );
        self.render_objects.truncate(quality.max_objects());
        let instance_count = self.render_objects.len() + 1;
        self.instances.clear();
        self.instances.push(InstanceRaw {
            model: Mat4::from_scale_rotation_translation(
                Vec3::new(120.0, 0.02, 140.0),
                glam::Quat::IDENTITY,
                Vec3::new(self.camera_focus.x, -0.42, self.camera_focus.z),
            )
            .to_cols_array_2d(),
            color: [0.004, 0.055, 0.07, 0.04],
        });
        self.instances.extend(
            self.render_objects
                .iter()
                .copied()
                .filter(|object| !is_movie_glass(*object, visual_style))
                .map(to_instance),
        );
        let opaque_instance_count = self.instances.len();
        self.glass_objects.clear();
        self.glass_objects.extend(
            self.render_objects
                .iter()
                .copied()
                .filter(|object| is_movie_glass(*object, visual_style)),
        );
        self.glass_objects.sort_by(|left, right| {
            let left_distance = (left.model.w_axis.truncate() - eye).length_squared();
            let right_distance = (right.model.w_axis.truncate() - eye).length_squared();
            right_distance.total_cmp(&left_distance)
        });
        let glass_instance_count = self.glass_objects.len();
        self.instances
            .extend(self.glass_objects.iter().copied().map(to_instance));
        self.queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&self.instances),
        );
        let scene_cpu = scene_started.elapsed();
        let uniforms = Uniforms {
            view_proj: view_proj.to_cols_array_2d(),
            camera_time: [eye.x, eye.y, eye.z, self.started.elapsed().as_secs_f32()],
            pulse: [
                scene.command_active() as u8 as f32,
                flight_intensity,
                f32::from(self.settings.graphics.floor_pulses),
                match visual_style {
                    VisualStyle::Classic => 0.0,
                    VisualStyle::Movie1995 => 1.0,
                },
            ],
            primary: OmarchyTheme::rgba(self.theme.cyan, 1.0),
            secondary: OmarchyTheme::rgba(self.theme.magenta, 1.0),
            accent: OmarchyTheme::rgba(self.theme.accent, 1.0),
            background: OmarchyTheme::rgba(self.theme.dark_background, 1.0),
            active_face: [
                camera.face_outward.x,
                camera.face_outward.y,
                camera.face_outward.z,
                f32::from(show_navigator),
            ],
            render_size: [
                self.config.width as f32,
                self.config.height as f32,
                1.0 / self.config.width.max(1) as f32,
                1.0 / self.config.height.max(1) as f32,
            ],
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
        let projected_panel = if show_navigator {
            let panel = face_panel
                .and_then(|panel| project_face_panel(panel, view_proj, visualiser))
                .unwrap_or_else(|| ProjectedFacePanel::fallback(visualiser));
            Some(panel)
        } else {
            None
        };
        self.navigator_hit_rect = projected_panel.map(ProjectedFacePanel::rect);
        let metadata_rect =
            projected_panel.map(|panel| metadata_panel(panel, navigator.selected_line, visualiser));
        let settings_rect = settings.map(|_| settings_panel(visualiser));
        let hud_rect = performance_hud_rect(visualiser);
        let terminal_hud_rect = self.layout.terminal.map(terminal_hud_rect);
        let backdrop_mode = match self.settings.appearance.backdrop {
            ThemeBackdrop::Off => 0.0,
            ThemeBackdrop::Tint => 1.0,
            ThemeBackdrop::Wallpaper => 2.0,
        };
        let background_uniforms = BackgroundUniforms {
            background: OmarchyTheme::rgba(self.theme.background, 1.0),
            options: [
                backdrop_mode,
                visualiser.width / visualiser.height.max(1.0),
                self.background_image_aspect,
                f32::from(self.background_image_loaded),
            ],
        };
        self.queue.write_buffer(
            &self.background_uniform_buffer,
            0,
            bytemuck::bytes_of(&background_uniforms),
        );
        let overlay_uniforms = OverlayUniforms {
            visualiser: visualiser.normalized(self.config.width, self.config.height),
            terminal: normalized_optional(
                self.layout.terminal,
                self.config.width,
                self.config.height,
            ),
            file_menu: [0.0; 4],
            metadata: normalized_optional(metadata_rect, self.config.width, self.config.height),
            settings: normalized_optional(settings_rect, self.config.width, self.config.height),
            hud: hud_rect.normalized(self.config.width, self.config.height),
            terminal_hud: normalized_optional(
                terminal_hud_rect,
                self.config.width,
                self.config.height,
            ),
            terminal_focus: [f32::from(terminal_focus), 0.0, 0.0, 0.0],
            primary: OmarchyTheme::rgba(self.theme.cyan, 1.0),
            secondary: OmarchyTheme::rgba(self.theme.accent, 1.0),
            panel_background: OmarchyTheme::rgba(self.theme.dark_background, 1.0),
            options: [f32::from(self.settings.graphics.scanlines), 0.0, 0.0, 0.0],
        };
        self.queue.write_buffer(
            &self.overlay_uniform_buffer,
            0,
            bytemuck::bytes_of(&overlay_uniforms),
        );

        let labels_started = Instant::now();
        let tower_labels = scene.tower_labels(
            camera.face_outward,
            visual_state,
            eye,
            self.camera_focus,
            self.settings.graphics.quality.max_labels(),
            visual_style,
        );
        let labels_cpu = labels_started.elapsed();
        let hud_text = self
            .performance
            .hud_text(visualiser.width, navigation_pending);
        let terminal_hud_text = self
            .layout
            .terminal
            .map(|terminal| format_terminal_hud(terminal.width, terminal_focus));
        let text_prepare_cpu = self.prepare_text(
            terminal,
            navigator,
            settings,
            &tower_labels,
            TextPlacement {
                view_projection: view_proj,
                visualiser,
                projected_panel,
                metadata_rect,
                settings_rect,
            },
            HudContent {
                visualiser: &hud_text,
                terminal: terminal_hud_text.as_deref(),
                terminal_focused: terminal_focus,
            },
        )?;

        self.update_ui_rects(terminal);
        self.draw_frame(
            opaque_instance_count,
            glass_instance_count,
            self.ui_rects.len(),
            visualiser,
        )?;
        self.performance.record(
            frame_started.elapsed(),
            scene_cpu,
            labels_cpu,
            text_prepare_cpu,
            instance_count,
            tower_labels.len(),
        );
        Ok(())
    }

    fn prepare_text(
        &mut self,
        terminal: &TerminalSnapshot,
        navigator: &NavigatorSnapshot,
        settings: Option<&SettingsSnapshot>,
        tower_labels: &[TowerLabel],
        placement: TextPlacement,
        hud: HudContent<'_>,
    ) -> anyhow::Result<Duration> {
        let visualiser = placement.visualiser;
        let projected_panel = placement.projected_panel;
        let metadata_rect = placement.metadata_rect;
        let settings_rect = placement.settings_rect;
        self.update_terminal_text(terminal);
        if self.last_hud_text != hud.visualiser {
            self.hud_buffer.set_text(
                hud.visualiser,
                &Attrs::new().family(Family::Name(MICHROMA_FAMILY)),
                Shaping::Advanced,
                None,
            );
            self.hud_buffer
                .shape_until_scroll(&mut self.font_system, false);
            hud.visualiser.clone_into(&mut self.last_hud_text);
        }
        if let Some(terminal_hud_text) = hud.terminal
            && self.last_terminal_hud_text != terminal_hud_text
        {
            self.terminal_hud_buffer.set_text(
                terminal_hud_text,
                &Attrs::new().family(Family::Name(MICHROMA_FAMILY)),
                Shaping::Advanced,
                None,
            );
            self.terminal_hud_buffer
                .shape_until_scroll(&mut self.font_system, false);
            terminal_hud_text.clone_into(&mut self.last_terminal_hud_text);
        }
        if navigator.listing_fingerprint != self.last_navigator_fingerprint {
            let normal = Attrs::new()
                .family(Family::Name(MICHROMA_FAMILY))
                .color(glyph_color(self.theme.cyan))
                .metadata(0);
            let selected = Attrs::new()
                .family(Family::Name(MICHROMA_FAMILY))
                .color(glyph_color(self.theme.accent))
                .metadata(0);
            let spans = navigator.listing.split_inclusive('\n').map(|line| {
                if line.trim_start().starts_with('▶') {
                    (line, selected.clone())
                } else {
                    (line, normal.clone())
                }
            });
            self.navigator_buffer
                .set_rich_text(spans, &normal, Shaping::Advanced, None);
            self.navigator_buffer
                .shape_until_scroll(&mut self.font_system, false);
            self.last_navigator_fingerprint = navigator.listing_fingerprint;
            self.navigator_texture_dirty = true;
        }
        if navigator.metadata_fingerprint != self.last_metadata_fingerprint {
            self.metadata_buffer.set_text(
                &navigator.metadata,
                &Attrs::new().family(Family::Name(MICHROMA_FAMILY)),
                Shaping::Advanced,
                None,
            );
            self.metadata_buffer
                .shape_until_scroll(&mut self.font_system, false);
            self.last_metadata_fingerprint = navigator.metadata_fingerprint;
        }
        if let Some(settings) = settings
            && settings.fingerprint != self.last_settings_fingerprint
        {
            let normal = Attrs::new()
                .family(Family::Name(MICHROMA_FAMILY))
                .color(glyph_color(self.theme.foreground));
            let selected = Attrs::new()
                .family(Family::Name(MICHROMA_FAMILY))
                .color(glyph_color(self.theme.accent));
            let spans = settings.text.split_inclusive('\n').map(|line| {
                if line.trim_start().starts_with('▶') {
                    (line, selected.clone())
                } else {
                    (line, normal.clone())
                }
            });
            self.settings_buffer
                .set_rich_text(spans, &normal, Shaping::Advanced, None);
            self.settings_buffer
                .shape_until_scroll(&mut self.font_system, false);
            self.last_settings_fingerprint = settings.fingerprint;
        }
        self.update_tower_label_text(tower_labels);
        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );

        let terminal_rect = self.layout.terminal;
        let terminal_content = terminal_rect.map(terminal_content_rect);
        let terminal_left = terminal_content.map_or(0.0, |rect| rect.x);
        let terminal_top = terminal_content.map_or(0.0, |rect| rect.y);
        let mut ui_text_areas = Vec::new();
        let mut scene_text_areas = Vec::new();
        let hud_rect = performance_hud_rect(visualiser);
        ui_text_areas.push(TextArea {
            buffer: &self.hud_buffer,
            left: hud_rect.x + 9.0,
            top: hud_rect.y + 4.0,
            scale: 1.0,
            bounds: rect_bounds(hud_rect),
            default_color: glyph_color(self.theme.cyan),
            custom_glyphs: &[],
        });
        if settings.is_none()
            && projected_panel.is_some()
            && let Some(metadata_rect) = metadata_rect
        {
            ui_text_areas.push(TextArea {
                buffer: &self.metadata_buffer,
                left: metadata_rect.x + 10.0,
                top: metadata_rect.y + 9.0,
                scale: 1.0,
                bounds: rect_bounds(metadata_rect),
                default_color: glyph_color(self.theme.cyan),
                custom_glyphs: &[],
            });
        }
        if let (Some(terminal_rect), Some(terminal_content)) = (terminal_rect, terminal_content) {
            let terminal_hud = terminal_hud_rect(terminal_rect);
            ui_text_areas.push(TextArea {
                buffer: &self.terminal_hud_buffer,
                left: terminal_hud.x + 9.0,
                top: terminal_hud.y + 4.0,
                scale: 1.0,
                bounds: rect_bounds(terminal_hud),
                default_color: glyph_color(if hud.terminal_focused {
                    self.theme.accent
                } else {
                    self.theme.cyan
                }),
                custom_glyphs: &[],
            });
            let terminal_bounds = rect_bounds(terminal_content);
            ui_text_areas.push(TextArea {
                buffer: &self.terminal_buffer,
                left: terminal_left,
                top: terminal_top,
                scale: 1.0,
                bounds: terminal_bounds,
                default_color: self.terminal_theme.foreground_color(),
                custom_glyphs: &[],
            });
            ui_text_areas.push(TextArea {
                buffer: &self.cursor_buffer,
                left: terminal_left + f32::from(terminal.cursor.1) * TERMINAL_CELL_WIDTH,
                top: terminal_top + f32::from(terminal.cursor.0) * TERMINAL_LINE_HEIGHT,
                scale: 1.0,
                bounds: terminal_bounds,
                default_color: self.terminal_theme.cursor_color(),
                custom_glyphs: &[],
            });
        }

        if settings.is_none() {
            append_tower_label_areas(
                &mut scene_text_areas,
                &self.tower_label_buffers,
                tower_labels,
                placement,
                glyph_color(self.theme.cyan),
                glyph_color(self.theme.foreground),
                glyph_color(self.theme.dark_background),
            );
        }
        if let Some(settings_rect) = settings_rect {
            ui_text_areas.push(TextArea {
                buffer: &self.settings_buffer,
                left: settings_rect.x + 22.0,
                top: settings_rect.y + 18.0,
                scale: 1.0,
                bounds: rect_bounds(settings_rect),
                default_color: glyph_color(self.theme.foreground),
                custom_glyphs: &[],
            });
        }

        let started = Instant::now();
        let mut scene_depths = Vec::with_capacity(tower_labels.len() + 1);
        // The navigator belongs to the active face. Keep its glyphs above that face while
        // tower labels use their world depth and remain hidden by nearer geometry.
        scene_depths.push(0.0);
        scene_depths.extend(tower_labels.iter().map(|label| {
            project_world_depth(label.world_position, placement.view_projection).unwrap_or(0.0)
        }));
        if self.navigator_texture_dirty {
            self.navigator_texture_renderer.prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.navigator_texture_viewport,
                [TextArea {
                    buffer: &self.navigator_buffer,
                    left: NAVIGATOR_TEXTURE_LEFT,
                    top: NAVIGATOR_TEXTURE_TOP,
                    scale: NAVIGATOR_TEXTURE_SCALE,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: NAVIGATOR_TEXTURE_WIDTH as i32,
                        bottom: NAVIGATOR_TEXTURE_HEIGHT as i32,
                    },
                    default_color: glyph_color(self.theme.cyan),
                    custom_glyphs: &[],
                }],
                &mut self.swash_cache,
            )?;
        }
        self.scene_text_renderer.prepare_with_depth(
            &self.device,
            &self.queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            scene_text_areas,
            &mut self.swash_cache,
            |metadata| scene_depths.get(metadata).copied().unwrap_or(0.0),
        )?;
        self.text_renderer.prepare(
            &self.device,
            &self.queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            ui_text_areas,
            &mut self.swash_cache,
        )?;
        Ok(started.elapsed())
    }

    fn draw_frame(
        &mut self,
        opaque_instance_count: usize,
        glass_instance_count: usize,
        ui_rect_count: usize,
        visualiser: Rect,
    ) -> anyhow::Result<()> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Suboptimal(_) => {
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.surface = self.instance.create_surface(self.window.clone())?;
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => anyhow::bail!("surface validation error"),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("GIBSON frame"),
            });
        if self.navigator_texture_dirty {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tower navigator texture pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.navigator_texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.navigator_texture_renderer.render(
                &self.atlas,
                &self.navigator_texture_viewport,
                &mut pass,
            )?;
        }
        let movie_glass = self.settings.appearance.visual_style == VisualStyle::Movie1995;
        let (scissor_x, scissor_y, scissor_width, scissor_height) = scissor_rect(visualiser);
        let blur_scissor = (
            scissor_x / 2,
            scissor_y / 2,
            scissor_width.div_ceil(2),
            scissor_height.div_ceil(2),
        );
        let scene_target = if movie_glass {
            &self.scene_targets.color_view
        } else {
            &view
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("opaque scene pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: scene_target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.001,
                            g: 0.003,
                            b: 0.011,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: if movie_glass {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(
                visualiser.x,
                visualiser.y,
                visualiser.width,
                visualiser.height,
                0.0,
                1.0,
            );
            pass.set_scissor_rect(scissor_x, scissor_y, scissor_width, scissor_height);
            pass.set_pipeline(&self.background_pipeline);
            pass.set_bind_group(0, &self.background_texture_group, &[]);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.uniform_group, &[]);
            pass.set_bind_group(1, &self.navigator_texture_group, &[]);
            pass.set_bind_group(2, &self.scene_targets.blur_b_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..self.index_count, 0, 0..opaque_instance_count as u32);
            self.scene_text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)?;
        }
        if movie_glass {
            for (label, target, pipeline, source) in [
                (
                    "horizontal glass blur pass",
                    &self.scene_targets.blur_a_view,
                    &self.blur_horizontal_pipeline,
                    &self.scene_targets.color_group,
                ),
                (
                    "vertical glass blur pass",
                    &self.scene_targets.blur_b_view,
                    &self.blur_vertical_pipeline,
                    &self.scene_targets.blur_a_group,
                ),
            ] {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, source, &[]);
                pass.set_scissor_rect(
                    blur_scissor.0,
                    blur_scissor.1,
                    blur_scissor.2,
                    blur_scissor.3,
                );
                pass.draw(0..3, 0..1);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("scene copy pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.scene_copy_pipeline);
                pass.set_bind_group(0, &self.scene_targets.color_group, &[]);
                pass.set_scissor_rect(scissor_x, scissor_y, scissor_width, scissor_height);
                pass.draw(0..3, 0..1);
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glass composite pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(
                visualiser.x,
                visualiser.y,
                visualiser.width,
                visualiser.height,
                0.0,
                1.0,
            );
            pass.set_scissor_rect(scissor_x, scissor_y, scissor_width, scissor_height);
            if glass_instance_count > 0 {
                pass.set_pipeline(&self.glass_pipeline);
                pass.set_bind_group(0, &self.uniform_group, &[]);
                pass.set_bind_group(1, &self.navigator_texture_group, &[]);
                pass.set_bind_group(2, &self.scene_targets.blur_b_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(
                    0..self.index_count,
                    0,
                    opaque_instance_count as u32
                        ..(opaque_instance_count + glass_instance_count) as u32,
                );
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terminal and text pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.overlay_pipeline);
            pass.set_bind_group(0, &self.overlay_uniform_group, &[]);
            pass.draw(0..3, 0..1);
            if ui_rect_count > 0 {
                pass.set_pipeline(&self.ui_rect_pipeline);
                pass.set_vertex_buffer(0, self.ui_rect_buffer.slice(..));
                pass.draw(0..6, 0..ui_rect_count as u32);
            }
            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)?;
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        self.navigator_texture_dirty = false;
        self.atlas.trim();
        Ok(())
    }

    fn update_terminal_text(&mut self, terminal: &TerminalSnapshot) {
        if self.last_terminal_fingerprint != terminal.fingerprint {
            let (terminal_width, terminal_height) =
                self.layout.terminal.map_or((800.0, 456.0), |terminal| {
                    let content = terminal_content_rect(terminal);
                    (content.width.max(80.0), content.height.max(40.0))
                });
            self.terminal_buffer
                .set_size(Some(terminal_width), Some(terminal_height));
            let family_name = self.terminal_theme.font_family.clone();
            let default_attrs = Attrs::new().family(Family::Name(&family_name));
            let spans = terminal
                .spans
                .iter()
                .map(|span| {
                    (
                        span.text.as_str(),
                        self.terminal_theme.attrs(span, &family_name),
                    )
                })
                .collect::<Vec<_>>();
            self.terminal_buffer
                .set_rich_text(spans, &default_attrs, Shaping::Basic, None);
            self.terminal_buffer
                .shape_until_scroll(&mut self.font_system, false);
            self.last_terminal_fingerprint = terminal.fingerprint;
        }
    }

    fn update_ui_rects(&mut self, terminal: &TerminalSnapshot) {
        self.ui_rects.clear();
        let Some(terminal_rect) = self.layout.terminal else {
            return;
        };
        let content = terminal_content_rect(terminal_rect);
        let left = content.x;
        let top = content.y;
        let width = (f32::from(terminal.columns) * TERMINAL_CELL_WIDTH).min(content.width.max(0.0));
        let height = (f32::from(terminal.rows) * TERMINAL_LINE_HEIGHT).min(content.height.max(0.0));
        self.ui_rects.push(ui_rect(
            Rect {
                x: left,
                y: top,
                width,
                height,
            },
            self.terminal_theme.background,
            self.config.width,
            self.config.height,
        ));
        for background in terminal.backgrounds.iter().take(MAX_UI_RECTS - 1) {
            let x = left + f32::from(background.column) * TERMINAL_CELL_WIDTH;
            let y = top + f32::from(background.row) * TERMINAL_LINE_HEIGHT;
            if x >= content.right() || y >= content.bottom() {
                continue;
            }
            let color = self
                .terminal_theme
                .resolve_color(background.color, true, false);
            self.ui_rects.push(ui_rect(
                Rect {
                    x: x.floor(),
                    y: y.floor(),
                    width: (f32::from(background.cells) * TERMINAL_CELL_WIDTH + 0.5)
                        .min(content.right() - x),
                    height: (TERMINAL_LINE_HEIGHT + 0.5).min(content.bottom() - y),
                },
                color,
                self.config.width,
                self.config.height,
            ));
        }
        self.queue.write_buffer(
            &self.ui_rect_buffer,
            0,
            bytemuck::cast_slice(&self.ui_rects),
        );
    }

    fn update_tower_label_text(&mut self, labels: &[TowerLabel]) {
        while self.tower_label_buffers.len() < labels.len() {
            let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(15.0, 19.0));
            buffer.set_wrap(Wrap::None);
            self.tower_label_buffers.push(buffer);
            self.tower_label_text.push(String::new());
        }
        self.tower_label_buffers.truncate(labels.len());
        self.tower_label_text.truncate(labels.len());

        for (index, label) in labels.iter().enumerate() {
            if self.tower_label_text[index] == label.text {
                continue;
            }
            let buffer = &mut self.tower_label_buffers[index];
            buffer.set_size(Some(480.0), Some(24.0));
            buffer.set_text(
                &label.text,
                &Attrs::new()
                    .family(Family::Name(MICHROMA_FAMILY))
                    .metadata(index + 1),
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(&mut self.font_system, false);
            self.tower_label_text[index].clone_from(&label.text);
        }
    }
}

fn select_present_mode(
    supported: &[wgpu::PresentMode],
    frame_rate: FrameRate,
) -> Option<wgpu::PresentMode> {
    let preferred = if frame_rate == FrameRate::Unlimited {
        [
            wgpu::PresentMode::Immediate,
            wgpu::PresentMode::Mailbox,
            wgpu::PresentMode::Fifo,
        ]
    } else {
        [
            wgpu::PresentMode::Fifo,
            wgpu::PresentMode::Mailbox,
            wgpu::PresentMode::Immediate,
        ]
    };
    preferred
        .into_iter()
        .find(|mode| supported.contains(mode))
        .or_else(|| supported.first().copied())
}

fn create_background_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    wallpaper: Option<&Path>,
) -> (wgpu::BindGroup, f32, bool) {
    let decoded = wallpaper
        .and_then(|path| match decode_wallpaper(path) {
            Ok(image) => {
                debug!(path = %path.display(), "loaded Omarchy wallpaper");
                Some(image)
            }
            Err(error) => {
                warn!(path = %path.display(), %error, "cannot load Omarchy wallpaper");
                None
            }
        })
        .map(image::DynamicImage::into_rgba8);
    let (width, height, pixels, loaded) = decoded.map_or_else(
        || (1, 1, vec![0_u8, 0, 0, 255], false),
        |image| {
            let (width, height) = image.dimensions();
            (width.max(1), height.max(1), image.into_raw(), true)
        },
    );
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("Omarchy theme wallpaper"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &pixels,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Omarchy theme wallpaper sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Omarchy theme wallpaper group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniform_buffer.as_entire_binding(),
            },
        ],
    });
    (group, width as f32 / height.max(1) as f32, loaded)
}

fn decode_wallpaper(path: &Path) -> image::ImageResult<image::DynamicImage> {
    image::ImageReader::open(path)
        .and_then(image::ImageReader::with_guessed_format)
        .map_err(image::ImageError::IoError)?
        .decode()
}

fn create_navigator_texture(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> (wgpu::TextureView, wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tower navigator texture"),
        size: wgpu::Extent3d {
            width: NAVIGATOR_TEXTURE_WIDTH,
            height: NAVIGATOR_TEXTURE_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("tower navigator sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tower navigator texture group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    (view, group)
}

fn create_scene_targets(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> SceneTargets {
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("glass blur sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let create_target = |label: &'static str, width: u32, height: u32| {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        (view, group)
    };
    let (color_view, color_group) = create_target("opaque scene texture", width, height);
    let blur_width = width.div_ceil(2);
    let blur_height = height.div_ceil(2);
    let (blur_a_view, blur_a_group) =
        create_target("horizontal glass blur texture", blur_width, blur_height);
    let (blur_b_view, blur_b_group) =
        create_target("vertical glass blur texture", blur_width, blur_height);
    SceneTargets {
        color_view,
        color_group,
        blur_a_view,
        blur_a_group,
        blur_b_view,
        blur_b_group,
    }
}

fn ui_rect(rect: Rect, color: [u8; 3], width: u32, height: u32) -> UiRect {
    UiRect {
        rect: rect.normalized(width, height),
        color: OmarchyTheme::rgba(color, 1.0),
    }
}

fn glyph_color(color: [u8; 3]) -> Color {
    Color::rgb(color[0], color[1], color[2])
}

fn performance_hud_rect(viewport: Rect) -> Rect {
    Rect {
        x: viewport.x + 1.0,
        y: viewport.y + 1.0,
        width: (viewport.width - 2.0).max(0.0),
        height: PANE_HUD_HEIGHT,
    }
}

fn format_performance_hud(sample: PerformanceSample, width: f32) -> String {
    let fps = if sample.fps > 0.0 {
        format!("{:.1} FPS", sample.fps)
    } else {
        "FPS --".to_owned()
    };
    if width >= 900.0 {
        format!(
            "GIBSON // {fps} // L/R ORBIT // R RESET // F2 FOCUS // F3 TERMINAL // F4 FILES // F10 SETTINGS"
        )
    } else if width >= 560.0 {
        format!("GIBSON // {fps} // L/R ORBIT // F2 FOCUS // F3 TTY // F4 FILES // F10 SETTINGS")
    } else {
        format!("GIBSON // {fps} // F2 FOCUS // F10 SETTINGS")
    }
}

fn terminal_hud_rect(terminal: Rect) -> Rect {
    Rect {
        x: terminal.x + 1.0,
        y: terminal.y + 1.0,
        width: (terminal.width - 2.0).max(0.0),
        height: PANE_HUD_HEIGHT,
    }
}

fn terminal_content_rect(terminal: Rect) -> Rect {
    let top = PANE_HUD_HEIGHT + TERMINAL_CONTENT_GAP;
    Rect {
        x: terminal.x + TERMINAL_HORIZONTAL_PADDING,
        y: terminal.y + top,
        width: (terminal.width - TERMINAL_HORIZONTAL_PADDING * 2.0).max(0.0),
        height: (terminal.height - top - TERMINAL_BOTTOM_PADDING).max(0.0),
    }
}

fn format_terminal_hud(width: f32, focused: bool) -> String {
    let state = if focused { "ACTIVE" } else { "STANDBY" };
    if width >= 590.0 {
        format!("TERMINAL // {state} // F2 (FOCUS) // F3 (HIDE) // F4 (FILES) // F10 (SETTINGS)")
    } else if width >= 420.0 {
        format!("TTY // {state} // F2 (FOCUS) // F3 (HIDE) // F4 (FILES)")
    } else {
        format!("TTY // {state} // F2 FOCUS // F3 HIDE")
    }
}

fn settings_panel(viewport: Rect) -> Rect {
    let width = (viewport.width * 0.72).clamp(580.0, 780.0);
    let height = (viewport.height * 0.68).clamp(430.0, 570.0);
    Rect {
        x: viewport.x + (viewport.width - width) * 0.5,
        y: viewport.y + (viewport.height - height) * 0.5,
        width,
        height,
    }
}

fn nearest_tower_face(direction: Vec3) -> Vec3 {
    if direction.x.abs() >= direction.z.abs() {
        Vec3::new(direction.x.signum(), 0.0, 0.0)
    } else {
        Vec3::new(0.0, 0.0, direction.z.signum())
    }
}

fn desired_camera_pose(
    current: CameraSubject,
    state: VisualState,
    zoom: f32,
    selection_position: f32,
) -> CameraPose {
    let zoom = zoom.clamp(0.55, 2.2);
    match state {
        VisualState::Transit => {
            let idle_distance = (current.height * 2.0 + 8.0).clamp(18.0, 34.0);
            CameraPose {
                focus: current.center,
                distance: idle_distance * zoom,
                elevation: 0.58,
                field_of_view: 49.0,
            }
        }
        VisualState::Settled => {
            let selection_position = selection_position.clamp(0.0, 1.0);
            let edge_offset = if selection_position < 0.28 {
                (0.28 - selection_position) / 0.28
            } else if selection_position > 0.72 {
                -(selection_position - 0.72) / 0.28
            } else {
                0.0
            };
            CameraPose {
                focus: current.center + Vec3::Y * (edge_offset * current.height * 0.30),
                distance: (current.width * 3.15 + 2.2).clamp(5.4, 10.8) * zoom,
                elevation: 0.035,
                field_of_view: 40.0,
            }
        }
    }
}

fn rect_bounds(rect: Rect) -> TextBounds {
    TextBounds {
        left: rect.x as i32,
        top: rect.y as i32,
        right: rect.right() as i32,
        bottom: rect.bottom() as i32,
    }
}

fn rects_intersect(left: Rect, right: Rect) -> bool {
    left.x < right.right()
        && left.right() > right.x
        && left.y < right.bottom()
        && left.bottom() > right.y
}

fn expand_rect(rect: Rect, amount: f32) -> Rect {
    Rect {
        x: rect.x - amount,
        y: rect.y - amount,
        width: rect.width + amount * 2.0,
        height: rect.height + amount * 2.0,
    }
}

fn normalized_optional(rect: Option<Rect>, width: u32, height: u32) -> [f32; 4] {
    rect.map_or([0.0; 4], |rect| rect.normalized(width, height))
}

fn scissor_rect(rect: Rect) -> (u32, u32, u32, u32) {
    let x = rect.x.floor().max(0.0) as u32;
    let y = rect.y.floor().max(0.0) as u32;
    let right = rect.right().ceil().max(rect.x + 1.0) as u32;
    let bottom = rect.bottom().ceil().max(rect.y + 1.0) as u32;
    (x, y, right.saturating_sub(x), bottom.saturating_sub(y))
}

#[derive(Clone, Copy, Debug)]
struct ProjectedFacePanel {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl ProjectedFacePanel {
    fn fallback(viewport: Rect) -> Self {
        Self {
            left: viewport.x + viewport.width * 0.62,
            top: viewport.y + 10.0,
            right: viewport.right() - 10.0,
            bottom: viewport.bottom() - 10.0,
        }
    }

    fn rect(self) -> Rect {
        Rect {
            x: self.left,
            y: self.top,
            width: (self.right - self.left).max(0.0),
            height: (self.bottom - self.top).max(0.0),
        }
    }
}

fn metadata_panel(face: ProjectedFacePanel, selected_line: usize, viewport: Rect) -> Rect {
    let width = (viewport.width * 0.25).clamp(220.0, 330.0);
    let height = (viewport.height * 0.23).clamp(150.0, 220.0);
    let gap = 20.0;
    let x = if face.right + gap + width <= viewport.right() - 8.0 {
        face.right + gap
    } else {
        (face.left - gap - width).max(viewport.x + 8.0)
    };
    let selected_texture_y =
        NAVIGATOR_TEXTURE_TOP + (selected_line as f32 + 0.5) * 17.0 * NAVIGATOR_TEXTURE_SCALE;
    let selected_y =
        face.top + selected_texture_y / NAVIGATOR_TEXTURE_HEIGHT as f32 * (face.bottom - face.top);
    let y = (selected_y - height * 0.5).clamp(viewport.y + 8.0, viewport.bottom() - height - 8.0);
    Rect {
        x,
        y,
        width: width.min(viewport.right() - x - 8.0),
        height,
    }
}

fn project_face_panel(
    panel: FacePanel,
    view_projection: Mat4,
    viewport: Rect,
) -> Option<ProjectedFacePanel> {
    project_world_depth(panel.center, view_projection)?;
    let half_horizontal = panel.horizontal * (panel.width * 0.5);
    let half_vertical = Vec3::Y * (panel.height * 0.5);
    let corners = [
        panel.center - half_horizontal - half_vertical,
        panel.center + half_horizontal - half_vertical,
        panel.center - half_horizontal + half_vertical,
        panel.center + half_horizontal + half_vertical,
    ];
    let projected = corners
        .map(|point| project_world_point(point, view_projection, viewport))
        .into_iter()
        .collect::<Option<Vec<_>>>()?;
    let mut left = projected
        .iter()
        .map(|point| point.0)
        .fold(f32::INFINITY, f32::min);
    let mut right = projected
        .iter()
        .map(|point| point.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let mut top = projected
        .iter()
        .map(|point| point.1)
        .fold(f32::INFINITY, f32::min);
    let mut bottom = projected
        .iter()
        .map(|point| point.1)
        .fold(f32::NEG_INFINITY, f32::max);
    if right < viewport.x
        || left > viewport.right()
        || bottom < viewport.y
        || top > viewport.bottom()
    {
        return None;
    }
    left = left.clamp(viewport.x + 4.0, viewport.right() - 4.0);
    right = right.clamp(viewport.x + 4.0, viewport.right() - 4.0);
    top = top.clamp(viewport.y + 4.0, viewport.bottom() - 4.0);
    bottom = bottom.clamp(viewport.y + 4.0, viewport.bottom() - 4.0);
    if right - left < 54.0 || bottom - top < 76.0 {
        return None;
    }
    Some(ProjectedFacePanel {
        left,
        top,
        right,
        bottom,
    })
}

fn project_world_depth(point: Vec3, view_projection: Mat4) -> Option<f32> {
    let clip = view_projection * point.extend(1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let depth = clip.z / clip.w;
    (0.0..=1.0)
        .contains(&depth)
        .then_some((depth - 0.00035).max(0.0))
}

fn project_world_point(point: Vec3, view_projection: Mat4, viewport: Rect) -> Option<(f32, f32)> {
    let clip = view_projection * point.extend(1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    if !(0.0..=1.0).contains(&ndc.z) {
        return None;
    }
    Some((
        viewport.x + (ndc.x * 0.5 + 0.5) * viewport.width,
        viewport.y + (0.5 - ndc.y * 0.5) * viewport.height,
    ))
}

struct ProjectedTowerLabel {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    depth: f32,
    scale: f32,
    color: Color,
}

fn project_tower_label(
    label: &TowerLabel,
    view_projection: Mat4,
    viewport: Rect,
    primary: Color,
    foreground: Color,
) -> Option<ProjectedTowerLabel> {
    let clip = view_projection * label.world_position.extend(1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    if !(-1.12..=1.12).contains(&ndc.x)
        || !(-1.12..=1.12).contains(&ndc.y)
        || !(0.0..=1.0).contains(&ndc.z)
    {
        return None;
    }
    let x = viewport.x + (ndc.x * 0.5 + 0.5) * viewport.width;
    let y = viewport.y + (0.5 - ndc.y * 0.5) * viewport.height;
    let distance_scale = (18.0 / clip.w).clamp(0.68, 1.18);
    let (base_scale, color) = if label.current {
        (0.92, foreground)
    } else {
        (0.74, primary)
    };
    let scale = base_scale * distance_scale;
    let text_width = label.text.chars().count() as f32 * 9.5 * scale;
    Some(ProjectedTowerLabel {
        left: x - text_width * 0.5,
        top: y - 9.5 * scale,
        width: text_width,
        height: 19.0 * scale,
        depth: ndc.z,
        scale,
        color,
    })
}

fn append_tower_label_areas<'a>(
    text_areas: &mut Vec<TextArea<'a>>,
    buffers: &'a [Buffer],
    labels: &[TowerLabel],
    placement: TextPlacement,
    primary: Color,
    foreground: Color,
    shadow: Color,
) {
    let mut projected = labels
        .iter()
        .enumerate()
        .filter_map(|(index, label)| {
            project_tower_label(
                label,
                placement.view_projection,
                placement.visualiser,
                primary,
                foreground,
            )
            .map(|label| (index, label))
        })
        .collect::<Vec<_>>();
    projected.sort_by(|(left_index, left), (right_index, right)| {
        labels[*right_index]
            .current
            .cmp(&labels[*left_index].current)
            .then_with(|| left.depth.total_cmp(&right.depth))
    });

    let city_bounds = rect_bounds(placement.visualiser);
    let mut occupied = Vec::new();
    for (index, label) in projected {
        let label_rect = Rect {
            x: label.left,
            y: label.top,
            width: label.width,
            height: label.height,
        };
        if placement
            .projected_panel
            .map(ProjectedFacePanel::rect)
            .is_some_and(|panel| rects_intersect(label_rect, panel))
            || placement
                .metadata_rect
                .is_some_and(|panel| rects_intersect(label_rect, panel))
            || occupied
                .iter()
                .any(|rect| rects_intersect(expand_rect(label_rect, 3.0), *rect))
        {
            continue;
        }
        occupied.push(expand_rect(label_rect, 3.0));
        for (offset_x, offset_y) in [(-1.4, 0.0), (1.4, 0.0), (0.0, -1.4), (0.0, 1.4)] {
            text_areas.push(TextArea {
                buffer: &buffers[index],
                left: label.left + offset_x,
                top: label.top + offset_y,
                scale: label.scale,
                bounds: city_bounds,
                default_color: shadow,
                custom_glyphs: &[],
            });
        }
        text_areas.push(TextArea {
            buffer: &buffers[index],
            left: label.left,
            top: label.top,
            scale: label.scale,
            bounds: city_bounds,
            default_color: label.color,
            custom_glyphs: &[],
        });
    }
}

#[derive(Clone, Debug)]
struct TerminalTheme {
    foreground: [u8; 3],
    background: [u8; 3],
    cursor: [u8; 3],
    colors: [[u8; 3]; 16],
    font_family: String,
}

impl TerminalTheme {
    fn load() -> Self {
        let mut theme = Self::default();
        let user_config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .map(|path| path.join("foot/foot.ini"))
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".config/foot/foot.ini"))
            });
        let config = user_config
            .filter(|path| path.is_file())
            .unwrap_or_else(|| PathBuf::from("/usr/share/omarchy/config/foot/foot.ini"));
        theme.read_file(&config, 0);
        theme
    }

    fn read_file(&mut self, path: &Path, depth: usize) {
        if depth > 4 {
            return;
        }
        let Ok(contents) = fs::read_to_string(path) else {
            return;
        };
        let mut section = String::new();
        for raw_line in contents.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section.clear();
                section.push_str(&line[1..line.len() - 1]);
                continue;
            }
            let Some((raw_key, raw_value)) = line.split_once('=') else {
                continue;
            };
            let key = raw_key.trim();
            let value = raw_value.trim();
            if key == "include" {
                self.read_file(&expand_home(value), depth + 1);
                continue;
            }
            if section == "main" && key == "font" {
                let family = value
                    .split(',')
                    .next()
                    .unwrap_or(value)
                    .split(':')
                    .next()
                    .unwrap_or(value)
                    .trim();
                if !family.is_empty() {
                    self.font_family = family.into();
                }
                continue;
            }
            if section != "colors" && section != "colors-dark" {
                continue;
            }
            match key {
                "foreground" => {
                    if let Some(color) = parse_hex_color(value) {
                        self.foreground = color;
                    }
                }
                "background" => {
                    if let Some(color) = parse_hex_color(value) {
                        self.background = color;
                    }
                }
                "cursor" => {
                    if let Some(color) = value
                        .split_whitespace()
                        .next_back()
                        .and_then(parse_hex_color)
                    {
                        self.cursor = color;
                    }
                }
                _ => {
                    if let Some(index) = key
                        .strip_prefix("regular")
                        .and_then(|value| value.parse::<usize>().ok())
                        .filter(|index| *index < 8)
                        && let Some(color) = parse_hex_color(value)
                    {
                        self.colors[index] = color;
                    } else if let Some(index) = key
                        .strip_prefix("bright")
                        .and_then(|value| value.parse::<usize>().ok())
                        .filter(|index| *index < 8)
                        && let Some(color) = parse_hex_color(value)
                    {
                        self.colors[index + 8] = color;
                    }
                }
            }
        }
    }

    fn foreground_color(&self) -> Color {
        Color::rgb(self.foreground[0], self.foreground[1], self.foreground[2])
    }

    fn cursor_color(&self) -> Color {
        Color::rgb(self.cursor[0], self.cursor[1], self.cursor[2])
    }

    fn attrs<'a>(&self, span: &TerminalSpan, family_name: &'a str) -> Attrs<'a> {
        let mut rgb = self.visible_foreground(span);
        if span.dim {
            rgb = [rgb[0] / 2, rgb[1] / 2, rgb[2] / 2];
        }
        let mut attrs = Attrs::new()
            .family(Family::Name(family_name))
            .color(Color::rgb(rgb[0], rgb[1], rgb[2]));
        if span.bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        if span.italic {
            attrs = attrs.style(Style::Italic);
        }
        if span.underline {
            attrs = attrs.underline(glyphon::cosmic_text::UnderlineStyle::Single);
        }
        attrs
    }

    fn visible_foreground(&self, span: &TerminalSpan) -> [u8; 3] {
        let terminal_color = if span.inverse {
            span.background
        } else {
            span.foreground
        };
        self.resolve_color(terminal_color, span.inverse, span.bold)
    }

    fn resolve_color(&self, color: TerminalColor, background: bool, bold: bool) -> [u8; 3] {
        match color {
            TerminalColor::Default => {
                if background {
                    self.background
                } else {
                    self.foreground
                }
            }
            TerminalColor::Indexed(index) if index < 16 => {
                let index = if bold && index < 8 { index + 8 } else { index };
                self.colors[index as usize]
            }
            TerminalColor::Indexed(index @ 16..=231) => {
                let cube = index - 16;
                let red = cube / 36;
                let green = (cube % 36) / 6;
                let blue = cube % 6;
                [
                    cube_component(red),
                    cube_component(green),
                    cube_component(blue),
                ]
            }
            TerminalColor::Indexed(index) => {
                let gray = 8_u8.saturating_add(index.saturating_sub(232).saturating_mul(10));
                [gray, gray, gray]
            }
            TerminalColor::Rgb(red, green, blue) => [red, green, blue],
        }
    }
}

impl Default for TerminalTheme {
    fn default() -> Self {
        Self {
            foreground: [0xd6, 0xc9, 0xad],
            background: [0x28, 0x25, 0x21],
            cursor: [0xf1, 0xdf, 0xc0],
            colors: [
                [0x28, 0x25, 0x21],
                [0xd6, 0x6d, 0x65],
                [0x9d, 0xb3, 0x6a],
                [0xd7, 0xa9, 0x3d],
                [0x80, 0xa6, 0xb2],
                [0xb9, 0x8a, 0xa2],
                [0x7f, 0xae, 0x9e],
                [0xd6, 0xc9, 0xad],
                [0x6b, 0x5a, 0x3b],
                [0xec, 0x7f, 0x76],
                [0xb3, 0xc7, 0x7a],
                [0xf2, 0xc1, 0x4e],
                [0x94, 0xba, 0xc5],
                [0xcd, 0xa0, 0xb7],
                [0x91, 0xc0, 0xae],
                [0xf1, 0xdf, 0xc0],
            ],
            font_family: "JetBrainsMono Nerd Font".into(),
        }
    }
}

fn expand_home(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(value)
}

fn parse_hex_color(value: &str) -> Option<[u8; 3]> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 6 {
        return None;
    }
    Some([
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
    ])
}

fn cube_component(value: u8) -> u8 {
    if value == 0 { 0 } else { value * 40 + 55 }
}

fn to_instance(object: RenderObject) -> InstanceRaw {
    InstanceRaw {
        model: object.model.to_cols_array_2d(),
        color: object.color,
    }
}

fn is_movie_glass(object: RenderObject, style: VisualStyle) -> bool {
    style == VisualStyle::Movie1995 && (0.1..0.3).contains(&object.color[3])
}

fn create_depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("scene depth"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn cube_mesh() -> (Vec<Vertex>, Vec<u16>) {
    let faces = [
        (
            [0.0, 0.0, 1.0],
            [
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
        ),
        (
            [0.0, 0.0, -1.0],
            [
                [0.5, -0.5, -0.5],
                [-0.5, -0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [0.5, 0.5, -0.5],
            ],
        ),
        (
            [1.0, 0.0, 0.0],
            [
                [0.5, -0.5, 0.5],
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [0.5, 0.5, 0.5],
            ],
        ),
        (
            [-1.0, 0.0, 0.0],
            [
                [-0.5, -0.5, -0.5],
                [-0.5, -0.5, 0.5],
                [-0.5, 0.5, 0.5],
                [-0.5, 0.5, -0.5],
            ],
        ),
        (
            [0.0, 1.0, 0.0],
            [
                [-0.5, 0.5, 0.5],
                [0.5, 0.5, 0.5],
                [0.5, 0.5, -0.5],
                [-0.5, 0.5, -0.5],
            ],
        ),
        (
            [0.0, -1.0, 0.0],
            [
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, -0.5, 0.5],
                [-0.5, -0.5, 0.5],
            ],
        ),
    ];
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (face_index, (normal, positions)) in faces.into_iter().enumerate() {
        let start = (face_index * 4) as u16;
        vertices.extend(
            positions
                .into_iter()
                .map(|position| Vertex { position, normal }),
        );
        indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
    }
    (vertices, indices)
}

#[cfg(test)]
mod renderer_tests {
    use super::*;

    #[test]
    fn movie_glass_classification_does_not_capture_floor_or_ui_geometry() {
        let object = |alpha| RenderObject {
            model: Mat4::IDENTITY,
            color: [0.0, 0.5, 0.8, alpha],
        };
        assert!(is_movie_glass(object(0.18), VisualStyle::Movie1995));
        assert!(!is_movie_glass(object(0.04), VisualStyle::Movie1995));
        assert!(!is_movie_glass(object(1.0), VisualStyle::Movie1995));
        assert!(!is_movie_glass(object(0.18), VisualStyle::Classic));
    }

    #[test]
    fn wallpaper_decoder_accepts_an_extensionless_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("background");
        image::RgbaImage::from_pixel(2, 1, image::Rgba([20, 40, 60, 255]))
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();

        let decoded = decode_wallpaper(&path).unwrap();

        assert_eq!((decoded.width(), decoded.height()), (2, 1));
    }

    #[test]
    fn world_text_depth_is_biased_towards_the_camera() {
        let depth = project_world_depth(Vec3::new(0.0, 0.0, 0.5), Mat4::IDENTITY).unwrap();
        assert!(depth < 0.5);
        assert!(depth > 0.49);
    }

    #[test]
    fn parses_foot_hex_colors() {
        assert_eq!(parse_hex_color("#80a6b2"), Some([0x80, 0xa6, 0xb2]));
        assert_eq!(parse_hex_color("invalid"), None);
    }

    #[test]
    fn resolves_the_xterm_color_cube() {
        let theme = TerminalTheme::default();
        assert_eq!(
            theme.resolve_color(TerminalColor::Indexed(196), false, false),
            [255, 0, 0]
        );
    }

    #[test]
    fn powerline_text_keeps_its_declared_foreground() {
        let theme = TerminalTheme::default();
        let span = TerminalSpan {
            text: "path".into(),
            foreground: TerminalColor::Indexed(0),
            background: TerminalColor::Indexed(3),
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            inverse: false,
        };
        assert_eq!(theme.visible_foreground(&span), theme.colors[0]);
    }

    #[test]
    fn unlimited_rendering_prefers_a_non_vsync_present_mode() {
        let supported = [
            wgpu::PresentMode::Fifo,
            wgpu::PresentMode::Mailbox,
            wgpu::PresentMode::Immediate,
        ];
        assert_eq!(
            select_present_mode(&supported, FrameRate::Unlimited),
            Some(wgpu::PresentMode::Immediate)
        );
        assert_eq!(
            select_present_mode(&supported, FrameRate::Fps60),
            Some(wgpu::PresentMode::Fifo)
        );
    }

    #[test]
    fn orbit_uses_the_nearest_physical_tower_face() {
        assert_eq!(nearest_tower_face(Vec3::new(0.9, 0.0, 0.2)), Vec3::X);
        assert_eq!(nearest_tower_face(Vec3::new(-0.2, 0.0, -0.9)), Vec3::NEG_Z);
    }

    #[test]
    fn settled_view_is_closer_and_lower_than_transit() {
        let current = CameraSubject {
            center: Vec3::ZERO,
            height: 8.0,
            width: 2.0,
        };
        let transit = desired_camera_pose(current, VisualState::Transit, 1.0, 0.5);
        let settled = desired_camera_pose(current, VisualState::Settled, 1.0, 0.5);

        assert!(transit.distance > settled.distance);
        assert!(transit.elevation > settled.elevation);
    }

    #[test]
    fn settled_camera_tracks_selection_near_list_edges() {
        let current = CameraSubject {
            center: Vec3::ZERO,
            height: 10.0,
            width: 2.0,
        };
        let top = desired_camera_pose(current, VisualState::Settled, 1.0, 0.0);
        let middle = desired_camera_pose(current, VisualState::Settled, 1.0, 0.5);
        let bottom = desired_camera_pose(current, VisualState::Settled, 1.0, 1.0);

        assert!(top.focus.y > middle.focus.y);
        assert!(bottom.focus.y < middle.focus.y);
    }

    #[test]
    fn performance_hud_adapts_to_the_visualiser_width() {
        let sample = PerformanceSample {
            fps: 60.2,
            frame_cpu_ms: 1.24,
            scene_cpu_ms: 0.31,
            object_count: 801,
            label_count: 8,
        };
        let compact = format_performance_hud(sample, 480.0);
        let medium = format_performance_hud(sample, 720.0);
        let wide = format_performance_hud(sample, 1_280.0);

        assert_eq!(compact, "GIBSON // 60.2 FPS // F2 FOCUS // F10 SETTINGS");
        assert_eq!(
            medium,
            "GIBSON // 60.2 FPS // L/R ORBIT // F2 FOCUS // F3 TTY // F4 FILES // F10 SETTINGS"
        );
        assert_eq!(
            wide,
            "GIBSON // 60.2 FPS // L/R ORBIT // R RESET // F2 FOCUS // F3 TERMINAL // F4 FILES // F10 SETTINGS"
        );
    }

    #[test]
    fn terminal_hud_reports_focus_and_adapts_to_width() {
        let wide = format_terminal_hud(640.0, true);
        let compact = format_terminal_hud(360.0, false);

        assert!(wide.contains("TERMINAL // ACTIVE"));
        assert!(wide.contains("F4 (FILES) // F10 (SETTINGS)"));
        assert_eq!(compact, "TTY // STANDBY // F2 FOCUS // F3 HIDE");
    }
}
