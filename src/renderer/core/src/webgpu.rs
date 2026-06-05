#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code, unused_imports))]

use std::{collections::HashMap, f32::consts::PI};

use crate::model::{
    CameraState, CanvasInputEvent, CubicRoute, RenderCard, RenderEdge, RenderGroup,
    RenderScenePatch, SceneSelection, SceneShadowLayerToken, SceneSnapshot, SceneStyleToken,
    WorldPoint, WorldRect,
};
use crate::serde_wasm;
use crate::stats::{
    CoreHitResult, CoreInputBatchResult, CoreOverlayRequest, CoreOverlayStyle, CoreOverlayTarget,
    WebGpuDebugSnapshot, WebGpuFrameStats, WebGpuProbeReport,
};
use crate::text::{
    CachedTextLine, TextBuildStats, TextEngine, TextLayoutCache, TEXT_ATLAS_HEIGHT,
    TEXT_ATLAS_SOLID_UV, TEXT_ATLAS_WIDTH,
};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(js_name = probeWebGpu)]
pub async fn probe_web_gpu(
    canvas: HtmlCanvasElement,
    width: f64,
    height: f64,
    device_pixel_ratio: f64,
) -> Result<JsValue, JsValue> {
    let pixel_width = ((width.max(1.0) * device_pixel_ratio.max(1.0)).round() as u32).max(1);
    let pixel_height = ((height.max(1.0) * device_pixel_ratio.max(1.0)).round() as u32).max(1);
    let enabled_backends = format!("{:?}", wgpu::Instance::enabled_backend_features());

    if !wgpu::util::is_browser_webgpu_supported().await {
        return serde_wasm(WebGpuProbeReport {
            supported: false,
            adapter_found: false,
            device_created: false,
            surface_configured: false,
            render_pass_submitted: false,
            presented: false,
            backend: "wgpu-webgpu".to_string(),
            enabled_backends,
            format: None,
            present_mode: None,
            width: pixel_width,
            height: pixel_height,
            detail: "Browser WebGPU adapter is unavailable for this canvas.".to_string(),
        });
    }

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::BROWSER_WEBGPU,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
        .map_err(|error| JsValue::from_str(&format!("WebGPU surface creation failed: {error}")))?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        })
        .await
        .map_err(|error| JsValue::from_str(&format!("WebGPU adapter request failed: {error}")))?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .map_err(|error| JsValue::from_str(&format!("WebGPU device request failed: {error}")))?;
    let config = surface
        .get_default_config(&adapter, pixel_width, pixel_height)
        .ok_or_else(|| {
            JsValue::from_str("WebGPU surface has no compatible default configuration")
        })?;
    surface.configure(&device, &config);
    let surface_texture = match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(texture)
        | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
        other => {
            return Err(JsValue::from_str(&format!(
                "WebGPU surface texture unavailable: {other:?}"
            )))
        }
    };
    let view = surface_texture
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("shape.ai WebGPU probe encoder"),
    });
    {
        let color_attachments = [Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: 0.94,
                    g: 0.98,
                    b: 0.97,
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Store,
            },
        })];
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shape.ai WebGPU probe clear pass"),
            color_attachments: &color_attachments,
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    queue.submit(Some(encoder.finish()));
    surface_texture.present();

    serde_wasm(WebGpuProbeReport {
        supported: true,
        adapter_found: true,
        device_created: true,
        surface_configured: true,
        render_pass_submitted: true,
        presented: true,
        backend: "wgpu-webgpu".to_string(),
        enabled_backends,
        format: Some(format!("{:?}", config.format)),
        present_mode: Some(format!("{:?}", config.present_mode)),
        width: config.width,
        height: config.height,
        detail: "wgpu WebGPU adapter, device, surface configuration, clear render pass, queue submit, and present succeeded.".to_string(),
    })
}

#[cfg(not(target_arch = "wasm32"))]
#[wasm_bindgen(js_name = probeWebGpu)]
pub async fn probe_web_gpu(
    _canvas: HtmlCanvasElement,
    width: f64,
    height: f64,
    device_pixel_ratio: f64,
) -> Result<JsValue, JsValue> {
    serde_wasm(WebGpuProbeReport {
        supported: false,
        adapter_found: false,
        device_created: false,
        surface_configured: false,
        render_pass_submitted: false,
        presented: false,
        backend: "wgpu-webgpu-wasm32-only".to_string(),
        enabled_backends: "native-test".to_string(),
        format: None,
        present_mode: None,
        width: ((width.max(1.0) * device_pixel_ratio.max(1.0)).round() as u32).max(1),
        height: ((height.max(1.0) * device_pixel_ratio.max(1.0)).round() as u32).max(1),
        detail: "Browser WebGPU canvas surfaces are only available in wasm32 builds.".to_string(),
    })
}

#[cfg(feature = "wgpu-probe")]
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuVertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

#[cfg(feature = "wgpu-probe")]
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ViewUniform {
    camera: [f32; 4],
    viewport: [f32; 4],
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone)]
struct ShapeRenderStyle {
    fill: [f32; 4],
    surface: [f32; 4],
    surface2: [f32; 4],
    surface3: [f32; 4],
    pastel: [f32; 4],
    stroke: [f32; 4],
    text: [f32; 4],
    muted_text: [f32; 4],
    accent: [f32; 4],
    line: [f32; 4],
    line_strong: [f32; 4],
    focus: [f32; 4],
    radius: ShapeRadius,
    stroke_width: ShapeStrokeWidth,
    typography: ShapeTypography,
    spacing: ShapeSpacing,
    shadow: Vec<ShapeShadowLayer>,
    selected_shadow: Vec<ShapeShadowLayer>,
    glow: Vec<ShapeShadowLayer>,
    gradient: ShapeGradient,
    state: ShapeState,
    badge: ShapeBadgeStyle,
    edge: ShapeEdgeStyle,
    port: ShapePortStyle,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
struct ShapeRadius {
    group: f64,
    group_selected: f64,
    card: f64,
    card_selected: f64,
    badge: f64,
    edge_label: f64,
    port: f64,
    focus_ring: f64,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
struct ShapeStrokeWidth {
    group: f64,
    group_selected: f64,
    card: f64,
    card_selected: f64,
    inner: f64,
    focus_ring: f64,
    edge: f64,
    edge_compact: f64,
    edge_selected: f64,
    separator: f64,
    port: f64,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
struct ShapeTypography {
    group_title_size: f32,
    group_summary_size: f32,
    card_title_size: f32,
    card_selected_title_size: f32,
    card_summary_size: f32,
    badge_size: f32,
    edge_label_size: f32,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
struct ShapeSpacing {
    group_padding_x: f64,
    group_padding_y: f64,
    card_padding: f64,
    card_gap: f64,
    badge_padding_x: f64,
    badge_height: f64,
    label_padding_x: f64,
    edge_label_height: f64,
    port_radius: f64,
    separator_inset: f64,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
struct ShapeShadowLayer {
    offset_x: f64,
    offset_y: f64,
    blur: f64,
    spread: f64,
    color: [f32; 4],
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
struct ShapeGradient {
    surface_top_alpha: f32,
    pastel_bottom_alpha: f32,
    accent_start_alpha: f32,
    accent_end_alpha: f32,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
struct ShapeState {
    default_fill_alpha: f32,
    default_stroke_alpha: f32,
    selected_fill_alpha: f32,
    selected_stroke_alpha: f32,
    focus_alpha: f32,
    shadow_alpha: f32,
    selected_shadow_alpha: f32,
    glow_alpha: f32,
    compact_stroke_alpha: f32,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
struct CardTextLayout {
    content_x: f64,
    content_width: f64,
    title_y: f64,
    title_font_size: f32,
    title_line_height: f64,
    summary_y: f64,
    summary_font_size: f32,
    summary_line_height: f64,
    summary_max_lines: usize,
    detail_y: f64,
    detail_font_size: f32,
    detail_line_height: f64,
    detail_max_lines: usize,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
struct ShapeBadgeStyle {
    fill_alpha: f32,
    stroke_alpha: f32,
    text_alpha: f32,
    min_width: f64,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
struct ShapeEdgeStyle {
    stroke_alpha: f32,
    selected_stroke_alpha: f32,
    compact_stroke_alpha: f32,
    label_fill_alpha: f32,
    label_stroke_alpha: f32,
    label_text_alpha: f32,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
struct ShapePortStyle {
    fill_alpha: f32,
    stroke_alpha: f32,
    selected_fill_alpha: f32,
    selected_stroke_alpha: f32,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
struct VertexSlot {
    offset: usize,
    capacity: usize,
    text_stats: TextBuildStats,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Default)]
struct VertexRanges {
    groups: HashMap<String, VertexSlot>,
    group_free_offsets: Vec<usize>,
    edges: HashMap<String, VertexSlot>,
    edge_free_offsets: Vec<usize>,
    cards: HashMap<String, VertexSlot>,
    card_free_offsets: Vec<usize>,
}

#[cfg(feature = "wgpu-probe")]
struct DrawRange {
    start: u32,
    end: u32,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Default)]
struct FrameDrawList {
    ranges: Vec<DrawRange>,
    visible_group_count: usize,
    visible_card_count: usize,
    visible_edge_count: usize,
    drawn_vertex_count: usize,
}

#[cfg(feature = "wgpu-probe")]
impl FrameDrawList {
    fn push_slot(&mut self, slot: VertexSlot) {
        let start = slot.offset as u32;
        let end = (slot.offset + slot.capacity) as u32;
        if let Some(last) = self.ranges.last_mut() {
            if last.end == start {
                last.end = end;
                self.drawn_vertex_count += slot.capacity;
                return;
            }
        }
        self.ranges.push(DrawRange { start, end });
        self.drawn_vertex_count += slot.capacity;
    }
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy, Default)]
struct VertexFitStats {
    truncation_count: usize,
    truncated_vertex_count: usize,
}

#[cfg(feature = "wgpu-probe")]
impl VertexFitStats {
    fn add(&mut self, other: VertexFitStats) {
        self.truncation_count += other.truncation_count;
        self.truncated_vertex_count += other.truncated_vertex_count;
    }
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
struct MutationCounters {
    patch_update_count: usize,
    dirty_range_write_count: usize,
    full_buffer_rebuild_count: usize,
    vertex_truncation_count: usize,
    truncated_vertex_count: usize,
    edge_capacity_grow_count: usize,
    edge_compaction_count: usize,
    card_capacity_grow_count: usize,
    card_compaction_count: usize,
    group_capacity_grow_count: usize,
    group_compaction_count: usize,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone)]
enum InputDragState {
    Pan {
        pointer_id: i32,
        start: WorldPoint,
        camera: CameraState,
    },
    Group {
        pointer_id: i32,
        group_id: String,
        start: WorldPoint,
    },
    Card {
        pointer_id: i32,
        card_id: String,
        start: WorldPoint,
        start_bounds: WorldRect,
    },
    Edge {
        pointer_id: i32,
        source_id: String,
    },
}

#[cfg(feature = "wgpu-probe")]
struct RendererRollbackState {
    scene: Option<SceneSnapshot>,
    camera: CameraState,
    input_drag: Option<InputDragState>,
    last_hit: Option<CoreHitResult>,
    text_layout_cache: TextLayoutCache,
    counters: MutationCounters,
}

#[cfg(feature = "wgpu-probe")]
#[wasm_bindgen]
pub struct ShapeWebGpuRenderer {
    canvas: HtmlCanvasElement,
    scene: Option<SceneSnapshot>,
    camera: CameraState,
    width: f64,
    height: f64,
    device_pixel_ratio: f64,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    _text_texture: wgpu::Texture,
    _text_view: wgpu::TextureView,
    _text_sampler: wgpu::Sampler,
    vertex_buffer: wgpu::Buffer,
    vertex_ranges: VertexRanges,
    text_engine: TextEngine,
    text_layout_cache: TextLayoutCache,
    vertex_count: usize,
    text_glyph_count: usize,
    fallback_text_glyph_count: usize,
    cjk_text_glyph_count: usize,
    font_fallback_run_count: usize,
    missing_text_glyph_count: usize,
    text_atlas_overflow_glyph_count: usize,
    text_missing_raster_glyph_count: usize,
    patch_update_count: usize,
    dirty_range_write_count: usize,
    full_buffer_rebuild_count: usize,
    vertex_truncation_count: usize,
    truncated_vertex_count: usize,
    edge_capacity_grow_count: usize,
    edge_compaction_count: usize,
    card_capacity_grow_count: usize,
    card_compaction_count: usize,
    group_capacity_grow_count: usize,
    group_compaction_count: usize,
    input_drag: Option<InputDragState>,
    last_hit: Option<CoreHitResult>,
}

#[cfg(feature = "wgpu-probe")]
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl ShapeWebGpuRenderer {
    #[wasm_bindgen(js_name = create)]
    pub async fn create(
        canvas: HtmlCanvasElement,
        width: f64,
        height: f64,
        device_pixel_ratio: f64,
    ) -> Result<ShapeWebGpuRenderer, JsValue> {
        let pixel_width = ((width.max(1.0) * device_pixel_ratio.max(1.0)).round() as u32).max(1);
        let pixel_height = ((height.max(1.0) * device_pixel_ratio.max(1.0)).round() as u32).max(1);
        canvas.set_width(pixel_width);
        canvas.set_height(pixel_height);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|error| {
                JsValue::from_str(&format!("WebGPU surface creation failed: {error}"))
            })?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|error| {
                JsValue::from_str(&format!("WebGPU adapter request failed: {error}"))
            })?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|error| {
                JsValue::from_str(&format!("WebGPU device request failed: {error}"))
            })?;
        let config = surface
            .get_default_config(&adapter, pixel_width, pixel_height)
            .ok_or_else(|| {
                JsValue::from_str("WebGPU surface has no compatible default configuration")
            })?;
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shape.ai visible WebGPU primitive shader"),
            source: wgpu::ShaderSource::Wgsl(SHAPE_WEBGPU_SHADER.into()),
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU view uniform"),
            size: std::mem::size_of::<ViewUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU primitive vertices"),
            size: 4,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });
        let mut text_engine = TextEngine::new().map_err(|error| {
            JsValue::from_str(&format!("Rust text font loading failed: {error}"))
        })?;
        let (text_texture, text_view, text_sampler) =
            create_text_atlas(&device, &queue, text_engine.atlas_pixels());
        text_engine.take_atlas_dirty();
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shape.ai WebGPU bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shape.ai WebGPU bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&text_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&text_sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shape.ai WebGPU pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let vertex_attributes = [
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: std::mem::size_of::<[f32; 2]>() as u64,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: (std::mem::size_of::<[f32; 2]>() * 2) as u64,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x4,
            },
        ];
        let vertex_buffers = [wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &vertex_attributes,
        }];
        let color_targets = [Some(wgpu::ColorTargetState {
            format: config.format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shape.ai WebGPU primitive pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &vertex_buffers,
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &color_targets,
            }),
            multiview_mask: None,
            cache: None,
        });

        let renderer = ShapeWebGpuRenderer {
            canvas,
            scene: None,
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            width: width.max(1.0),
            height: height.max(1.0),
            device_pixel_ratio: device_pixel_ratio.max(1.0),
            surface,
            device,
            queue,
            config,
            pipeline,
            bind_group,
            uniform_buffer,
            _text_texture: text_texture,
            _text_view: text_view,
            _text_sampler: text_sampler,
            vertex_buffer,
            vertex_ranges: VertexRanges::default(),
            text_engine,
            text_layout_cache: TextLayoutCache::default(),
            vertex_count: 0,
            text_glyph_count: 0,
            fallback_text_glyph_count: 0,
            cjk_text_glyph_count: 0,
            font_fallback_run_count: 0,
            missing_text_glyph_count: 0,
            text_atlas_overflow_glyph_count: 0,
            text_missing_raster_glyph_count: 0,
            patch_update_count: 0,
            dirty_range_write_count: 0,
            full_buffer_rebuild_count: 0,
            vertex_truncation_count: 0,
            truncated_vertex_count: 0,
            edge_capacity_grow_count: 0,
            edge_compaction_count: 0,
            card_capacity_grow_count: 0,
            card_compaction_count: 0,
            group_capacity_grow_count: 0,
            group_compaction_count: 0,
            input_drag: None,
            last_hit: None,
        };
        renderer.write_uniform();
        Ok(renderer)
    }

    pub fn resize(&mut self, width: f64, height: f64, device_pixel_ratio: f64) {
        self.width = width.max(1.0);
        self.height = height.max(1.0);
        self.device_pixel_ratio = device_pixel_ratio.max(1.0);
        self.config.width = ((self.width * self.device_pixel_ratio).round() as u32).max(1);
        self.config.height = ((self.height * self.device_pixel_ratio).round() as u32).max(1);
        self.canvas.set_width(self.config.width);
        self.canvas.set_height(self.config.height);
        self.surface.configure(&self.device, &self.config);
        self.write_uniform();
    }

    #[wasm_bindgen(js_name = loadScene)]
    pub fn load_scene(&mut self, scene_json: &str) -> Result<(), JsValue> {
        let scene = serde_json::from_str::<SceneSnapshot>(scene_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid scene snapshot: {error}")))?;
        self.camera = CameraState {
            x: scene.camera.x,
            y: scene.camera.y,
            zoom: scene.camera.zoom,
        };
        self.vertex_count = 0;
        self.scene = Some(scene);
        self.last_hit = None;
        self.rebuild_vertex_buffer();
        self.write_uniform();
        Ok(())
    }

    #[wasm_bindgen(js_name = applyPatchBatch)]
    pub fn apply_patch_batch(&mut self, patches_json: &str) -> Result<(), JsValue> {
        let patches = serde_json::from_str::<Vec<RenderScenePatch>>(patches_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid render patch batch: {error}")))?;
        let rollback = self.rollback_state();
        for patch in patches {
            if let Err(error) = self.apply_render_patch(patch) {
                self.restore_rollback_state(rollback);
                return Err(error);
            }
        }
        Ok(())
    }

    fn rollback_state(&self) -> RendererRollbackState {
        RendererRollbackState {
            scene: self.scene.clone(),
            camera: self.camera.clone(),
            input_drag: self.input_drag.clone(),
            last_hit: self.last_hit.clone(),
            text_layout_cache: self.text_layout_cache.clone(),
            counters: self.mutation_counters(),
        }
    }

    fn restore_rollback_state(&mut self, state: RendererRollbackState) {
        self.scene = state.scene;
        self.camera = state.camera;
        self.input_drag = state.input_drag;
        self.last_hit = state.last_hit;
        self.text_layout_cache = state.text_layout_cache.clone();
        self.rebuild_vertex_buffer();
        self.text_layout_cache = state.text_layout_cache;
        self.restore_mutation_counters(state.counters);
        self.write_uniform();
    }

    fn mutation_counters(&self) -> MutationCounters {
        MutationCounters {
            patch_update_count: self.patch_update_count,
            dirty_range_write_count: self.dirty_range_write_count,
            full_buffer_rebuild_count: self.full_buffer_rebuild_count,
            vertex_truncation_count: self.vertex_truncation_count,
            truncated_vertex_count: self.truncated_vertex_count,
            edge_capacity_grow_count: self.edge_capacity_grow_count,
            edge_compaction_count: self.edge_compaction_count,
            card_capacity_grow_count: self.card_capacity_grow_count,
            card_compaction_count: self.card_compaction_count,
            group_capacity_grow_count: self.group_capacity_grow_count,
            group_compaction_count: self.group_compaction_count,
        }
    }

    fn restore_mutation_counters(&mut self, counters: MutationCounters) {
        self.patch_update_count = counters.patch_update_count;
        self.dirty_range_write_count = counters.dirty_range_write_count;
        self.full_buffer_rebuild_count = counters.full_buffer_rebuild_count;
        self.vertex_truncation_count = counters.vertex_truncation_count;
        self.truncated_vertex_count = counters.truncated_vertex_count;
        self.edge_capacity_grow_count = counters.edge_capacity_grow_count;
        self.edge_compaction_count = counters.edge_compaction_count;
        self.card_capacity_grow_count = counters.card_capacity_grow_count;
        self.card_compaction_count = counters.card_compaction_count;
        self.group_capacity_grow_count = counters.group_capacity_grow_count;
        self.group_compaction_count = counters.group_compaction_count;
    }

    fn apply_render_patch(&mut self, patch: RenderScenePatch) -> Result<(), JsValue> {
        let mut dirty_card_ids = Vec::new();
        let mut dirty_edge_ids = Vec::new();
        let mut dirty_group_ids = Vec::new();
        let mut created_group_id = None;
        let mut deleted_group_id = None;
        let mut created_card_id = None;
        let mut deleted_card_id = None;
        let mut created_edge_id = None;
        let mut deleted_edge_ids = Vec::new();
        let mut deleted_card_ids = Vec::new();
        let mut rebuild_for_order = false;
        {
            let Some(scene) = &mut self.scene else {
                return Err(JsValue::from_str("No WebGPU scene loaded"));
            };
            match patch {
                RenderScenePatch::CreateGroup { group } => {
                    if group.bounds.width <= 0.0 || group.bounds.height <= 0.0 {
                        return Err(JsValue::from_str("Group bounds must be positive"));
                    }
                    if scene
                        .groups
                        .iter()
                        .any(|candidate| candidate.id == group.id)
                    {
                        return Err(JsValue::from_str(&format!(
                            "Duplicate group id: {}",
                            group.id
                        )));
                    }
                    let group_id = group.id.clone();
                    scene.groups.push(group);
                    scene.selection = SceneSelection::Group {
                        id: group_id.clone(),
                    };
                    created_group_id = Some(group_id);
                }
                RenderScenePatch::DeleteGroup { id } => {
                    let before = scene.groups.len();
                    scene.groups.retain(|group| group.id != id);
                    if scene.groups.len() == before {
                        return Err(JsValue::from_str(&format!("Unknown group id: {id}")));
                    }
                    let mut removed_card_ids = Vec::new();
                    scene.cards.retain(|card| {
                        let keep = card.group_id != id;
                        if !keep {
                            removed_card_ids.push(card.id.clone());
                        }
                        keep
                    });
                    let mut removed_edge_ids = Vec::new();
                    scene.edges.retain(|edge| {
                        let keep = edge.group_id != id
                            && !removed_card_ids
                                .iter()
                                .any(|card_id| card_id == &edge.source)
                            && !removed_card_ids
                                .iter()
                                .any(|card_id| card_id == &edge.target);
                        if !keep {
                            removed_edge_ids.push(edge.id.clone());
                        }
                        keep
                    });
                    if selection_is_group(&scene.selection, &id)
                        || removed_card_ids
                            .iter()
                            .any(|card_id| selection_is_node(&scene.selection, card_id))
                        || removed_edge_ids
                            .iter()
                            .any(|edge_id| selection_is_edge(&scene.selection, edge_id))
                    {
                        scene.selection = SceneSelection::Canvas;
                    }
                    deleted_edge_ids.extend(removed_edge_ids);
                    deleted_card_ids.extend(removed_card_ids);
                    deleted_group_id = Some(id);
                }
                RenderScenePatch::MoveGroup { id, delta } => {
                    let Some(group) = scene.groups.iter_mut().find(|group| group.id == id) else {
                        return Err(JsValue::from_str(&format!("Unknown group id: {id}")));
                    };
                    group.bounds.x += delta.x;
                    group.bounds.y += delta.y;
                    dirty_group_ids.push(id.clone());
                    let mut moved_card_ids = Vec::new();
                    for card in scene.cards.iter_mut().filter(|card| card.group_id == id) {
                        card.bounds.x += delta.x;
                        card.bounds.y += delta.y;
                        moved_card_ids.push(card.id.clone());
                    }
                    dirty_card_ids.extend(moved_card_ids.iter().cloned());
                    dirty_edge_ids.extend(
                        scene
                            .edges
                            .iter()
                            .filter(|edge| {
                                edge.group_id == id
                                    || moved_card_ids.iter().any(|card_id| {
                                        card_id == &edge.source || card_id == &edge.target
                                    })
                            })
                            .map(|edge| edge.id.clone()),
                    );
                }
                RenderScenePatch::MoveCard { id, position } => {
                    let Some(card) = scene.cards.iter_mut().find(|card| card.id == id) else {
                        return Err(JsValue::from_str(&format!("Unknown card id: {id}")));
                    };
                    card.bounds.x = position.x;
                    card.bounds.y = position.y;
                    dirty_card_ids.push(id.clone());
                    dirty_edge_ids.extend(
                        scene
                            .edges
                            .iter()
                            .filter(|edge| edge.source == id || edge.target == id)
                            .map(|edge| edge.id.clone()),
                    );
                }
                RenderScenePatch::SetCardZIndex { id, z_index } => {
                    let Some(card) = scene.cards.iter_mut().find(|card| card.id == id) else {
                        return Err(JsValue::from_str(&format!("Unknown card id: {id}")));
                    };
                    card.z_index = z_index;
                    scene.selection = SceneSelection::Node { id };
                    rebuild_for_order = true;
                }
                RenderScenePatch::EditCardText { id, field, value } => {
                    let Some(card) = scene.cards.iter_mut().find(|card| card.id == id) else {
                        return Err(JsValue::from_str(&format!("Unknown card id: {id}")));
                    };
                    match field.as_str() {
                        "title" => card.title = value,
                        "summary" => card.summary = value,
                        "detail" => card.detail = value,
                        _ => {
                            return Err(JsValue::from_str(&format!(
                                "Unsupported text field: {field}"
                            )))
                        }
                    }
                    dirty_card_ids.push(id);
                }
                RenderScenePatch::CreateCard { card } => {
                    if card.bounds.width <= 0.0 || card.bounds.height <= 0.0 {
                        return Err(JsValue::from_str("Card bounds must be positive"));
                    }
                    if !scene.groups.iter().any(|group| group.id == card.group_id) {
                        return Err(JsValue::from_str(&format!(
                            "Unknown group id: {}",
                            card.group_id
                        )));
                    }
                    if scene.cards.iter().any(|candidate| candidate.id == card.id) {
                        return Err(JsValue::from_str(&format!(
                            "Duplicate card id: {}",
                            card.id
                        )));
                    }
                    let card_id = card.id.clone();
                    scene.cards.push(card);
                    scene.selection = SceneSelection::Node {
                        id: card_id.clone(),
                    };
                    created_card_id = Some(card_id);
                }
                RenderScenePatch::DeleteCard { id } => {
                    let before = scene.cards.len();
                    scene.cards.retain(|card| card.id != id);
                    if scene.cards.len() == before {
                        return Err(JsValue::from_str(&format!("Unknown card id: {id}")));
                    }
                    let mut removed_edge_ids = Vec::new();
                    scene.edges.retain(|edge| {
                        let keep = edge.source != id && edge.target != id;
                        if !keep {
                            removed_edge_ids.push(edge.id.clone());
                        }
                        keep
                    });
                    if selection_is_node(&scene.selection, &id)
                        || removed_edge_ids
                            .iter()
                            .any(|edge_id| selection_is_edge(&scene.selection, edge_id))
                    {
                        scene.selection = SceneSelection::Canvas;
                    }
                    deleted_edge_ids.extend(removed_edge_ids);
                    deleted_card_id = Some(id);
                }
                RenderScenePatch::CreateEdge {
                    group_id,
                    source,
                    target,
                    edge_id,
                    label,
                } => {
                    if source == target {
                        return Err(JsValue::from_str("Edge source and target must differ"));
                    }
                    if !scene.cards.iter().any(|card| card.id == source) {
                        return Err(JsValue::from_str(&format!(
                            "Unknown source card id: {source}"
                        )));
                    }
                    if !scene.cards.iter().any(|card| card.id == target) {
                        return Err(JsValue::from_str(&format!(
                            "Unknown target card id: {target}"
                        )));
                    }
                    if !scene.groups.iter().any(|group| group.id == group_id) {
                        return Err(JsValue::from_str(&format!("Unknown group id: {group_id}")));
                    }
                    if scene.edges.iter().any(|edge| edge.id == edge_id) {
                        return Err(JsValue::from_str(&format!("Duplicate edge id: {edge_id}")));
                    }
                    scene.edges.push(RenderEdge {
                        id: edge_id.clone(),
                        group_id,
                        source,
                        target,
                        label: label.unwrap_or_else(|| "relates".to_string()),
                        edge_type: "supports".to_string(),
                        z_index: scene.edges.len() as f64,
                        style_key: "default".to_string(),
                    });
                    created_edge_id = Some(edge_id);
                }
                RenderScenePatch::DeleteEdge { id } => {
                    let before = scene.edges.len();
                    scene.edges.retain(|edge| edge.id != id);
                    if scene.edges.len() == before {
                        return Err(JsValue::from_str(&format!("Unknown edge id: {id}")));
                    }
                    deleted_edge_ids.push(id);
                }
                RenderScenePatch::Select { selection } => {
                    let previous = scene.selection.clone();
                    collect_selection_dirty_ids(
                        &previous,
                        &mut dirty_group_ids,
                        &mut dirty_card_ids,
                        &mut dirty_edge_ids,
                    );
                    collect_selection_dirty_ids(
                        &selection,
                        &mut dirty_group_ids,
                        &mut dirty_card_ids,
                        &mut dirty_edge_ids,
                    );
                    scene.selection = selection;
                }
            }
        }
        self.patch_update_count += 1;
        if rebuild_for_order {
            self.rebuild_vertex_buffer();
            return Ok(());
        }
        let edge_slots_deleted = !deleted_edge_ids.is_empty();
        let card_slots_deleted = deleted_card_id.is_some() || !deleted_card_ids.is_empty();
        for edge_id in deleted_edge_ids {
            if !self.clear_deleted_edge(&edge_id) {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        if edge_slots_deleted && self.should_compact_edge_slots() && !self.compact_edge_slots() {
            self.rebuild_vertex_buffer();
            return Ok(());
        }
        for card_id in deleted_card_ids {
            if !self.clear_deleted_card(&card_id) {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        if let Some(card_id) = deleted_card_id {
            if !self.clear_deleted_card(&card_id) {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        if card_slots_deleted && self.should_compact_card_slots() && !self.compact_card_slots() {
            self.rebuild_vertex_buffer();
            return Ok(());
        }
        if let Some(group_id) = deleted_group_id {
            if !self.clear_deleted_group(&group_id) {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
            if self.should_compact_group_slots() && !self.compact_group_slots() {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        if let Some(group_id) = created_group_id {
            if !self.write_new_group(&group_id)
                && !(self.grow_group_slots() && self.write_new_group(&group_id))
            {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        if let Some(card_id) = created_card_id {
            if !self.write_new_card(&card_id)
                && !(self.grow_card_slots() && self.write_new_card(&card_id))
            {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        if let Some(edge_id) = created_edge_id {
            if !self.write_new_edge(&edge_id)
                && !(self.grow_edge_slots() && self.write_new_edge(&edge_id))
            {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        for edge_id in dirty_edge_ids {
            if !self.write_dirty_edge(&edge_id) {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        for group_id in dirty_group_ids {
            if !self.write_dirty_group(&group_id) {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        for card_id in dirty_card_ids {
            if !self.write_dirty_card(&card_id) {
                self.rebuild_vertex_buffer();
                return Ok(());
            }
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = renderFrame)]
    pub fn render_frame(&mut self) -> Result<JsValue, JsValue> {
        self.write_uniform();
        self.flush_text_atlas();
        let draw_list = self.build_draw_list();
        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            other => {
                return Err(JsValue::from_str(&format!(
                    "WebGPU surface texture unavailable: {other:?}"
                )))
            }
        };
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shape.ai visible WebGPU encoder"),
            });
        {
            let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.972,
                        g: 0.982,
                        b: 0.992,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shape.ai visible WebGPU render pass"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !draw_list.ranges.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                for range in &draw_list.ranges {
                    pass.draw(range.start..range.end, 0..1);
                }
            }
        }
        self.queue.submit(Some(encoder.finish()));
        surface_texture.present();

        let (total_groups, total_cards, total_edges, style_token_count) = self
            .scene
            .as_ref()
            .map(|scene| {
                (
                    scene.groups.len(),
                    scene.cards.len(),
                    scene.edges.len(),
                    scene.styles.len(),
                )
            })
            .unwrap_or((0, 0, 0, 0));
        serde_wasm(WebGpuFrameStats {
            total_groups,
            total_cards,
            total_edges,
            visible_group_count: draw_list.visible_group_count,
            visible_card_count: draw_list.visible_card_count,
            visible_edge_count: draw_list.visible_edge_count,
            vertex_count: self.vertex_count,
            drawn_vertex_count: draw_list.drawn_vertex_count,
            draw_range_count: draw_list.ranges.len(),
            text_glyph_count: self.text_glyph_count,
            fallback_text_glyph_count: self.fallback_text_glyph_count,
            cjk_text_glyph_count: self.cjk_text_glyph_count,
            font_fallback_run_count: self.font_fallback_run_count,
            missing_text_glyph_count: self.missing_text_glyph_count,
            text_atlas_overflow_glyph_count: self.text_atlas_overflow_glyph_count,
            text_missing_raster_glyph_count: self.text_missing_raster_glyph_count,
            text_atlas_glyph_count: self.text_engine.atlas_glyph_count(),
            text_raster_cache_hits: self.text_engine.raster_cache_hits(),
            text_raster_cache_misses: self.text_engine.raster_cache_misses(),
            text_layout_cache_hits: self.text_layout_cache.hits,
            text_layout_cache_misses: self.text_layout_cache.misses,
            style_token_count,
            patch_update_count: self.patch_update_count,
            dirty_range_write_count: self.dirty_range_write_count,
            full_buffer_rebuild_count: self.full_buffer_rebuild_count,
            vertex_truncation_count: self.vertex_truncation_count,
            truncated_vertex_count: self.truncated_vertex_count,
            edge_capacity_grow_count: self.edge_capacity_grow_count,
            edge_compaction_count: self.edge_compaction_count,
            edge_slot_count: self.vertex_ranges.edges.len()
                + self.vertex_ranges.edge_free_offsets.len(),
            edge_slot_free_count: self.vertex_ranges.edge_free_offsets.len(),
            card_capacity_grow_count: self.card_capacity_grow_count,
            card_compaction_count: self.card_compaction_count,
            card_slot_count: self.vertex_ranges.cards.len()
                + self.vertex_ranges.card_free_offsets.len(),
            card_slot_free_count: self.vertex_ranges.card_free_offsets.len(),
            group_capacity_grow_count: self.group_capacity_grow_count,
            group_compaction_count: self.group_compaction_count,
            group_slot_count: self.vertex_ranges.groups.len()
                + self.vertex_ranges.group_free_offsets.len(),
            group_slot_free_count: self.vertex_ranges.group_free_offsets.len(),
            backend: "rust-wgpu-visible".to_string(),
        })
    }

    #[wasm_bindgen(js_name = inputBatch)]
    pub fn input_batch(&mut self, events_json: &str) -> Result<JsValue, JsValue> {
        let events = serde_json::from_str::<Vec<CanvasInputEvent>>(events_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid input batch: {error}")))?;
        let mut patches = Vec::new();
        let mut hit = None;
        let mut overlay = None;
        let rollback = self.rollback_state();
        for event in events {
            if let Err(error) = self.apply_input_event(event, &mut patches, &mut hit, &mut overlay)
            {
                self.restore_rollback_state(rollback);
                return Err(error);
            }
        }
        self.write_uniform();
        let selection = self
            .scene
            .as_ref()
            .map(|scene| scene.selection.clone())
            .unwrap_or_default();
        serde_wasm(CoreInputBatchResult {
            camera: self.camera.clone(),
            hit,
            selection,
            patches,
            overlay,
        })
    }

    #[wasm_bindgen(js_name = overlayRequest)]
    pub fn overlay_request(&self, card_id: &str, field: &str) -> Result<JsValue, JsValue> {
        serde_wasm(self.overlay_request_for_card(card_id, field))
    }

    #[wasm_bindgen(js_name = debugSnapshot)]
    pub fn debug_snapshot(&self) -> Result<JsValue, JsValue> {
        let (selection, selection_world_rect, total_groups, total_cards, total_edges) =
            if let Some(scene) = &self.scene {
                (
                    scene.selection.clone(),
                    selection_world_rect(scene, &scene.selection),
                    scene.groups.len(),
                    scene.cards.len(),
                    scene.edges.len(),
                )
            } else {
                (SceneSelection::Canvas, None, 0, 0, 0)
            };
        let selection_screen_rect = selection_world_rect
            .as_ref()
            .map(|rect| world_rect_to_screen_rect(rect, &self.camera));
        serde_wasm(WebGpuDebugSnapshot {
            camera: self.camera.clone(),
            selection,
            selection_world_rect,
            selection_screen_rect,
            last_hit: self.last_hit.clone(),
            total_groups,
            total_cards,
            total_edges,
            patch_update_count: self.patch_update_count,
            dirty_range_write_count: self.dirty_range_write_count,
            full_buffer_rebuild_count: self.full_buffer_rebuild_count,
        })
    }
}

#[cfg(feature = "wgpu-probe")]
#[cfg(target_arch = "wasm32")]
impl ShapeWebGpuRenderer {
    fn apply_input_event(
        &mut self,
        event: CanvasInputEvent,
        patches: &mut Vec<RenderScenePatch>,
        hit: &mut Option<CoreHitResult>,
        overlay: &mut Option<CoreOverlayRequest>,
    ) -> Result<(), JsValue> {
        match event {
            CanvasInputEvent::PointerDown { pointer_id, screen } => {
                let next_hit = self.hit_at_screen(screen);
                self.last_hit = next_hit.clone();
                *hit = next_hit.clone();
                self.push_input_patch(
                    RenderScenePatch::Select {
                        selection: selection_from_hit(next_hit.as_ref()),
                    },
                    patches,
                )?;
                self.input_drag = match next_hit.as_ref() {
                    Some(hit) if hit.kind == "port" && hit.port.as_deref() == Some("source") => {
                        Some(InputDragState::Edge {
                            pointer_id,
                            source_id: hit.id.clone(),
                        })
                    }
                    Some(hit) if hit.kind == "card" || hit.kind == "text" => self
                        .scene
                        .as_ref()
                        .and_then(|scene| scene.cards.iter().find(|card| card.id == hit.id))
                        .map(|card| InputDragState::Card {
                            pointer_id,
                            card_id: card.id.clone(),
                            start: WorldPoint {
                                x: hit.world_x,
                                y: hit.world_y,
                            },
                            start_bounds: card.bounds.clone(),
                        }),
                    Some(hit) if hit.kind == "group" => Some(InputDragState::Group {
                        pointer_id,
                        group_id: hit.id.clone(),
                        start: WorldPoint {
                            x: hit.world_x,
                            y: hit.world_y,
                        },
                    }),
                    _ => Some(InputDragState::Pan {
                        pointer_id,
                        start: screen,
                        camera: self.camera.clone(),
                    }),
                };
            }
            CanvasInputEvent::PointerMove { pointer_id, screen } => {
                let Some(drag) = self.input_drag.clone() else {
                    return Ok(());
                };
                match drag {
                    InputDragState::Pan {
                        pointer_id: drag_pointer_id,
                        start,
                        camera,
                    } if drag_pointer_id == pointer_id => {
                        self.camera = CameraState {
                            x: camera.x + screen.x - start.x,
                            y: camera.y + screen.y - start.y,
                            zoom: camera.zoom,
                        };
                    }
                    InputDragState::Group {
                        pointer_id: drag_pointer_id,
                        group_id,
                        start,
                    } if drag_pointer_id == pointer_id => {
                        let world = screen_to_world(screen, &self.camera);
                        self.input_drag = Some(InputDragState::Group {
                            pointer_id,
                            group_id: group_id.clone(),
                            start: world,
                        });
                        self.push_input_patch(
                            RenderScenePatch::MoveGroup {
                                id: group_id,
                                delta: WorldPoint {
                                    x: world.x - start.x,
                                    y: world.y - start.y,
                                },
                            },
                            patches,
                        )?;
                    }
                    InputDragState::Card {
                        pointer_id: drag_pointer_id,
                        card_id,
                        start,
                        start_bounds,
                    } if drag_pointer_id == pointer_id => {
                        let world = screen_to_world(screen, &self.camera);
                        self.push_input_patch(
                            RenderScenePatch::MoveCard {
                                id: card_id,
                                position: WorldPoint {
                                    x: start_bounds.x + world.x - start.x,
                                    y: start_bounds.y + world.y - start.y,
                                },
                            },
                            patches,
                        )?;
                    }
                    InputDragState::Edge {
                        pointer_id: drag_pointer_id,
                        ..
                    } if drag_pointer_id == pointer_id => {}
                    _ => {}
                }
            }
            CanvasInputEvent::PointerUp {
                pointer_id,
                screen,
                edge_id,
            } => {
                if let Some(InputDragState::Edge {
                    pointer_id: drag_pointer_id,
                    source_id,
                }) = self.input_drag.clone()
                {
                    if drag_pointer_id == pointer_id {
                        let next_hit = self.hit_at_screen(screen);
                        self.last_hit = next_hit.clone();
                        *hit = next_hit.clone();
                        if let Some(target_hit) = next_hit {
                            if target_hit.kind == "port"
                                && target_hit.port.as_deref() == Some("target")
                                && target_hit.id != source_id
                            {
                                let patch = self.scene.as_ref().and_then(|scene| {
                                    let source =
                                        scene.cards.iter().find(|card| card.id == source_id)?;
                                    let target =
                                        scene.cards.iter().find(|card| card.id == target_hit.id)?;
                                    Some(RenderScenePatch::CreateEdge {
                                        group_id: source.group_id.clone(),
                                        source: source.id.clone(),
                                        target: target.id.clone(),
                                        edge_id: edge_id.unwrap_or_else(|| {
                                            format!(
                                                "renderer-edge-{}",
                                                self.patch_update_count.saturating_add(1)
                                            )
                                        }),
                                        label: None,
                                    })
                                });
                                if let Some(patch) = patch {
                                    self.push_input_patch(patch, patches)?;
                                }
                            }
                        }
                    }
                }
                self.input_drag = None;
            }
            CanvasInputEvent::PointerCancel { pointer_id } => {
                if drag_pointer_id(self.input_drag.as_ref()) == Some(pointer_id) {
                    self.input_drag = None;
                }
            }
            CanvasInputEvent::Wheel { screen, delta_y } => {
                self.camera = zoom_camera_at_screen(&self.camera, screen, delta_y);
            }
            CanvasInputEvent::DoubleClick { screen } => {
                let next_hit = self.hit_at_screen(screen);
                self.last_hit = next_hit.clone();
                *hit = next_hit.clone();
                if let Some(hit) = next_hit {
                    if hit.kind == "text" {
                        if let Some(field) = hit.field.as_deref() {
                            *overlay = self.overlay_request_for_card(&hit.id, field);
                        }
                    }
                }
            }
            CanvasInputEvent::FitScene => {
                if let Some(scene) = &self.scene {
                    self.camera = fit_camera_to_scene(scene, self.width, self.height);
                }
            }
            CanvasInputEvent::FocusBounds {
                bounds,
                screen,
                zoom,
                padding,
                min_zoom,
                max_zoom,
            } => {
                self.camera = focus_camera_to_bounds(
                    &bounds,
                    screen,
                    zoom,
                    padding,
                    min_zoom,
                    max_zoom,
                    self.width,
                    self.height,
                );
            }
            CanvasInputEvent::SetCamera { camera } => {
                self.camera = clamp_camera(camera);
            }
        }
        Ok(())
    }

    fn push_input_patch(
        &mut self,
        patch: RenderScenePatch,
        patches: &mut Vec<RenderScenePatch>,
    ) -> Result<(), JsValue> {
        self.apply_render_patch(patch.clone())?;
        patches.push(patch);
        Ok(())
    }

    fn hit_at_screen(&self, screen: WorldPoint) -> Option<CoreHitResult> {
        let scene = self.scene.as_ref()?;
        hit_scene_at_screen(scene, &self.camera, screen)
    }

    fn overlay_request_for_card(&self, card_id: &str, field: &str) -> Option<CoreOverlayRequest> {
        let scene = self.scene.as_ref()?;
        let card = scene.cards.iter().find(|card| card.id == card_id)?;
        let value = match field {
            "title" => card.title.clone(),
            "summary" => card.summary.clone(),
            "detail" => card.detail.clone(),
            _ => return None,
        };
        let style = resolve_shape_style(&scene.styles, &card.style_key);
        let selected = selection_is_node(&scene.selection, &card.id);
        let text_rect = text_field_rect(&card.bounds, &style, field, selected);
        let world_rect = overlay_rect_for_text_field(&text_rect, &style, selected);
        Some(CoreOverlayRequest {
            target: CoreOverlayTarget {
                kind: "card-text".to_string(),
                id: card.id.clone(),
                field: field.to_string(),
            },
            value,
            screen_rect: world_rect_to_screen_rect(&world_rect, &self.camera),
            world_rect,
            style: overlay_style(&self.camera, &style, field, selected),
        })
    }

    fn build_draw_list(&self) -> FrameDrawList {
        let Some(scene) = &self.scene else {
            return FrameDrawList::default();
        };
        let viewport = self.padded_world_viewport();
        let mut draw_list = FrameDrawList::default();

        let mut groups: Vec<(&RenderGroup, VertexSlot)> = scene
            .groups
            .iter()
            .filter_map(|group| {
                self.vertex_ranges
                    .groups
                    .get(&group.id)
                    .copied()
                    .map(|slot| (group, slot))
            })
            .collect();
        groups.sort_by_key(|(_, slot)| slot.offset);
        for (group, slot) in groups {
            if rects_intersect(&group.bounds, &viewport) {
                draw_list.visible_group_count += 1;
                draw_list.push_slot(slot);
            }
        }

        let cards_by_id: HashMap<&str, &RenderCard> = scene
            .cards
            .iter()
            .map(|card| (card.id.as_str(), card))
            .collect();
        let mut edges: Vec<(&RenderEdge, VertexSlot)> = scene
            .edges
            .iter()
            .filter_map(|edge| {
                self.vertex_ranges
                    .edges
                    .get(&edge.id)
                    .copied()
                    .map(|slot| (edge, slot))
            })
            .collect();
        edges.sort_by_key(|(_, slot)| slot.offset);
        for (edge, slot) in edges {
            let Some(source) = cards_by_id.get(edge.source.as_str()).copied() else {
                continue;
            };
            let Some(target) = cards_by_id.get(edge.target.as_str()).copied() else {
                continue;
            };
            if rects_intersect(&edge_visible_bounds(source, target), &viewport) {
                draw_list.visible_edge_count += 1;
                draw_list.push_slot(slot);
            }
        }

        let mut cards: Vec<(&RenderCard, VertexSlot)> = scene
            .cards
            .iter()
            .filter_map(|card| {
                self.vertex_ranges
                    .cards
                    .get(&card.id)
                    .copied()
                    .map(|slot| (card, slot))
            })
            .collect();
        cards.sort_by_key(|(_, slot)| slot.offset);
        for (card, slot) in cards {
            if rects_intersect(&card.bounds, &viewport) {
                draw_list.visible_card_count += 1;
                draw_list.push_slot(slot);
            }
        }

        draw_list
    }

    fn padded_world_viewport(&self) -> WorldRect {
        let zoom = self.camera.zoom.max(0.025);
        let left = (0.0 - self.camera.x) / zoom;
        let top = (0.0 - self.camera.y) / zoom;
        let right = (self.width - self.camera.x) / zoom;
        let bottom = (self.height - self.camera.y) / zoom;
        let min_x = left.min(right) - VIEWPORT_CULL_PADDING;
        let min_y = top.min(bottom) - VIEWPORT_CULL_PADDING;
        let max_x = left.max(right) + VIEWPORT_CULL_PADDING;
        let max_y = top.max(bottom) + VIEWPORT_CULL_PADDING;
        WorldRect {
            x: min_x,
            y: min_y,
            width: (max_x - min_x).max(1.0),
            height: (max_y - min_y).max(1.0),
        }
    }

    fn write_uniform(&self) {
        let uniform = ViewUniform {
            camera: [
                self.camera.x as f32,
                self.camera.y as f32,
                self.camera.zoom as f32,
                0.0,
            ],
            viewport: [self.width as f32, self.height as f32, 0.0, 0.0],
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));
    }

    fn rebuild_vertex_buffer(&mut self) {
        let mut vertices = Vec::new();
        let mut vertex_ranges = VertexRanges::default();
        let mut text_stats = TextBuildStats::default();
        let mut vertex_fit_stats = VertexFitStats::default();
        if let Some(scene) = &self.scene {
            let mut groups: Vec<&RenderGroup> = scene.groups.iter().collect();
            groups.sort_by(|a, b| a.z_index.total_cmp(&b.z_index));
            for group in groups {
                let offset = vertices.len();
                let (group_vertices, group_text_stats) = build_group_vertices(
                    scene,
                    group,
                    &mut self.text_layout_cache,
                    &mut self.text_engine,
                );
                text_stats.add(group_text_stats);
                let (group_vertices, fit_stats) =
                    fit_vertices_to_slot_with_stats(group_vertices, GROUP_VERTEX_SLOT);
                vertex_fit_stats.add(fit_stats);
                vertices.extend(group_vertices);
                vertex_ranges.groups.insert(
                    group.id.clone(),
                    VertexSlot {
                        offset,
                        capacity: GROUP_VERTEX_SLOT,
                        text_stats: group_text_stats,
                    },
                );
            }
            let group_spare_slots = group_spare_slot_count(scene.groups.len());
            for _ in 0..group_spare_slots {
                let offset = vertices.len();
                vertices.extend(fit_vertices_to_slot(Vec::new(), GROUP_VERTEX_SLOT));
                vertex_ranges.group_free_offsets.push(offset);
            }

            for edge in &scene.edges {
                let offset = vertices.len();
                let (edge_vertices, edge_text_stats) = build_edge_vertices(
                    scene,
                    edge,
                    &mut self.text_layout_cache,
                    &mut self.text_engine,
                );
                text_stats.add(edge_text_stats);
                let (edge_vertices, fit_stats) =
                    fit_vertices_to_slot_with_stats(edge_vertices, EDGE_VERTEX_SLOT);
                vertex_fit_stats.add(fit_stats);
                vertices.extend(edge_vertices);
                vertex_ranges.edges.insert(
                    edge.id.clone(),
                    VertexSlot {
                        offset,
                        capacity: EDGE_VERTEX_SLOT,
                        text_stats: edge_text_stats,
                    },
                );
            }
            let edge_spare_slots = edge_spare_slot_count(scene.edges.len());
            for _ in 0..edge_spare_slots {
                let offset = vertices.len();
                vertices.extend(fit_vertices_to_slot(Vec::new(), EDGE_VERTEX_SLOT));
                vertex_ranges.edge_free_offsets.push(offset);
            }

            let mut cards: Vec<&RenderCard> = scene.cards.iter().collect();
            cards.sort_by(|a, b| a.z_index.total_cmp(&b.z_index));
            for card in cards {
                let offset = vertices.len();
                let (card_vertices, card_text_stats) = build_card_vertices(
                    scene,
                    card,
                    &mut self.text_layout_cache,
                    &mut self.text_engine,
                );
                text_stats.add(card_text_stats);
                let (card_vertices, fit_stats) =
                    fit_vertices_to_slot_with_stats(card_vertices, CARD_VERTEX_SLOT);
                vertex_fit_stats.add(fit_stats);
                vertices.extend(card_vertices);
                vertex_ranges.cards.insert(
                    card.id.clone(),
                    VertexSlot {
                        offset,
                        capacity: CARD_VERTEX_SLOT,
                        text_stats: card_text_stats,
                    },
                );
            }
            let card_spare_slots = card_spare_slot_count(scene.cards.len());
            for _ in 0..card_spare_slots {
                let offset = vertices.len();
                vertices.extend(fit_vertices_to_slot(Vec::new(), CARD_VERTEX_SLOT));
                vertex_ranges.card_free_offsets.push(offset);
            }
        }
        let size = (vertices.len() * std::mem::size_of::<GpuVertex>()).max(4) as u64;
        self.vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU primitive vertices"),
            size,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });
        self.vertex_ranges = vertex_ranges;
        self.vertex_count = vertices.len();
        self.text_glyph_count = text_stats.glyph_count;
        self.fallback_text_glyph_count = text_stats.fallback_glyph_count;
        self.cjk_text_glyph_count = text_stats.cjk_glyph_count;
        self.font_fallback_run_count = text_stats.fallback_run_count;
        self.missing_text_glyph_count = text_stats.missing_glyph_count;
        self.text_atlas_overflow_glyph_count = text_stats.atlas_overflow_glyph_count;
        self.text_missing_raster_glyph_count = text_stats.missing_raster_glyph_count;
        self.record_vertex_fit_stats(vertex_fit_stats);
        self.full_buffer_rebuild_count += 1;
        if !vertices.is_empty() {
            self.queue
                .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        }
    }

    fn write_dirty_card(&mut self, id: &str) -> bool {
        let Some(slot) = self.vertex_ranges.cards.get(id).copied() else {
            return false;
        };
        let (vertices, text_stats) = {
            let Some(scene) = &self.scene else {
                return false;
            };
            let Some(card) = scene.cards.iter().find(|card| card.id == id) else {
                return false;
            };
            build_card_vertices(
                scene,
                card,
                &mut self.text_layout_cache,
                &mut self.text_engine,
            )
        };
        self.write_slot_vertices(slot, vertices);
        if let Some(slot) = self.vertex_ranges.cards.get_mut(id) {
            self.text_glyph_count = self
                .text_glyph_count
                .saturating_sub(slot.text_stats.glyph_count)
                + text_stats.glyph_count;
            self.fallback_text_glyph_count = self
                .fallback_text_glyph_count
                .saturating_sub(slot.text_stats.fallback_glyph_count)
                + text_stats.fallback_glyph_count;
            self.cjk_text_glyph_count = self
                .cjk_text_glyph_count
                .saturating_sub(slot.text_stats.cjk_glyph_count)
                + text_stats.cjk_glyph_count;
            self.font_fallback_run_count = self
                .font_fallback_run_count
                .saturating_sub(slot.text_stats.fallback_run_count)
                + text_stats.fallback_run_count;
            self.missing_text_glyph_count = self
                .missing_text_glyph_count
                .saturating_sub(slot.text_stats.missing_glyph_count)
                + text_stats.missing_glyph_count;
            self.text_atlas_overflow_glyph_count = self
                .text_atlas_overflow_glyph_count
                .saturating_sub(slot.text_stats.atlas_overflow_glyph_count)
                + text_stats.atlas_overflow_glyph_count;
            self.text_missing_raster_glyph_count = self
                .text_missing_raster_glyph_count
                .saturating_sub(slot.text_stats.missing_raster_glyph_count)
                + text_stats.missing_raster_glyph_count;
            slot.text_stats = text_stats;
        }
        self.dirty_range_write_count += 1;
        true
    }

    fn write_dirty_group(&mut self, id: &str) -> bool {
        let Some(slot) = self.vertex_ranges.groups.get(id).copied() else {
            return false;
        };
        let (vertices, text_stats) = {
            let Some(scene) = &self.scene else {
                return false;
            };
            let Some(group) = scene.groups.iter().find(|group| group.id == id) else {
                return false;
            };
            build_group_vertices(
                scene,
                group,
                &mut self.text_layout_cache,
                &mut self.text_engine,
            )
        };
        self.write_slot_vertices(slot, vertices);
        if let Some(slot) = self.vertex_ranges.groups.get_mut(id) {
            self.text_glyph_count = self
                .text_glyph_count
                .saturating_sub(slot.text_stats.glyph_count)
                + text_stats.glyph_count;
            self.fallback_text_glyph_count = self
                .fallback_text_glyph_count
                .saturating_sub(slot.text_stats.fallback_glyph_count)
                + text_stats.fallback_glyph_count;
            self.cjk_text_glyph_count = self
                .cjk_text_glyph_count
                .saturating_sub(slot.text_stats.cjk_glyph_count)
                + text_stats.cjk_glyph_count;
            self.font_fallback_run_count = self
                .font_fallback_run_count
                .saturating_sub(slot.text_stats.fallback_run_count)
                + text_stats.fallback_run_count;
            self.missing_text_glyph_count = self
                .missing_text_glyph_count
                .saturating_sub(slot.text_stats.missing_glyph_count)
                + text_stats.missing_glyph_count;
            self.text_atlas_overflow_glyph_count = self
                .text_atlas_overflow_glyph_count
                .saturating_sub(slot.text_stats.atlas_overflow_glyph_count)
                + text_stats.atlas_overflow_glyph_count;
            self.text_missing_raster_glyph_count = self
                .text_missing_raster_glyph_count
                .saturating_sub(slot.text_stats.missing_raster_glyph_count)
                + text_stats.missing_raster_glyph_count;
            slot.text_stats = text_stats;
        }
        self.dirty_range_write_count += 1;
        true
    }

    fn write_dirty_edge(&mut self, id: &str) -> bool {
        let Some(slot) = self.vertex_ranges.edges.get(id).copied() else {
            return false;
        };
        let (vertices, text_stats) = {
            let Some(scene) = &self.scene else {
                return false;
            };
            let Some(edge) = scene.edges.iter().find(|edge| edge.id == id) else {
                return false;
            };
            build_edge_vertices(
                scene,
                edge,
                &mut self.text_layout_cache,
                &mut self.text_engine,
            )
        };
        self.write_slot_vertices(slot, vertices);
        if let Some(slot) = self.vertex_ranges.edges.get_mut(id) {
            self.text_glyph_count = self
                .text_glyph_count
                .saturating_sub(slot.text_stats.glyph_count)
                + text_stats.glyph_count;
            self.fallback_text_glyph_count = self
                .fallback_text_glyph_count
                .saturating_sub(slot.text_stats.fallback_glyph_count)
                + text_stats.fallback_glyph_count;
            self.cjk_text_glyph_count = self
                .cjk_text_glyph_count
                .saturating_sub(slot.text_stats.cjk_glyph_count)
                + text_stats.cjk_glyph_count;
            self.font_fallback_run_count = self
                .font_fallback_run_count
                .saturating_sub(slot.text_stats.fallback_run_count)
                + text_stats.fallback_run_count;
            self.missing_text_glyph_count = self
                .missing_text_glyph_count
                .saturating_sub(slot.text_stats.missing_glyph_count)
                + text_stats.missing_glyph_count;
            self.text_atlas_overflow_glyph_count = self
                .text_atlas_overflow_glyph_count
                .saturating_sub(slot.text_stats.atlas_overflow_glyph_count)
                + text_stats.atlas_overflow_glyph_count;
            self.text_missing_raster_glyph_count = self
                .text_missing_raster_glyph_count
                .saturating_sub(slot.text_stats.missing_raster_glyph_count)
                + text_stats.missing_raster_glyph_count;
            slot.text_stats = text_stats;
        }
        self.dirty_range_write_count += 1;
        true
    }

    fn write_new_group(&mut self, id: &str) -> bool {
        let Some(offset) = self.take_append_group_free_offset() else {
            return false;
        };
        let slot = VertexSlot {
            offset,
            capacity: GROUP_VERTEX_SLOT,
            text_stats: TextBuildStats::default(),
        };
        self.vertex_ranges.groups.insert(id.to_string(), slot);
        if !self.write_dirty_group(id) {
            self.vertex_ranges.groups.remove(id);
            self.vertex_ranges.group_free_offsets.push(offset);
            return false;
        }
        true
    }

    fn take_append_group_free_offset(&mut self) -> Option<usize> {
        let used_end = self.group_segment_used_end();
        let mut best_index = None;
        let mut best_offset = usize::MAX;
        for (index, offset) in self
            .vertex_ranges
            .group_free_offsets
            .iter()
            .copied()
            .enumerate()
        {
            if offset >= used_end && offset < best_offset {
                best_index = Some(index);
                best_offset = offset;
            }
        }
        best_index.map(|index| self.vertex_ranges.group_free_offsets.swap_remove(index))
    }

    fn write_new_edge(&mut self, id: &str) -> bool {
        let Some(offset) = self.vertex_ranges.edge_free_offsets.pop() else {
            return false;
        };
        let slot = VertexSlot {
            offset,
            capacity: EDGE_VERTEX_SLOT,
            text_stats: TextBuildStats::default(),
        };
        self.vertex_ranges.edges.insert(id.to_string(), slot);
        if !self.write_dirty_edge(id) {
            self.vertex_ranges.edges.remove(id);
            self.vertex_ranges.edge_free_offsets.push(offset);
            return false;
        }
        true
    }

    fn write_new_card(&mut self, id: &str) -> bool {
        let Some(offset) = self.take_append_card_free_offset() else {
            return false;
        };
        let slot = VertexSlot {
            offset,
            capacity: CARD_VERTEX_SLOT,
            text_stats: TextBuildStats::default(),
        };
        self.vertex_ranges.cards.insert(id.to_string(), slot);
        if !self.write_dirty_card(id) {
            self.vertex_ranges.cards.remove(id);
            self.vertex_ranges.card_free_offsets.push(offset);
            return false;
        }
        true
    }

    fn take_append_card_free_offset(&mut self) -> Option<usize> {
        let used_end = self.card_segment_used_end();
        let mut best_index = None;
        let mut best_offset = usize::MAX;
        for (index, offset) in self
            .vertex_ranges
            .card_free_offsets
            .iter()
            .copied()
            .enumerate()
        {
            if offset >= used_end && offset < best_offset {
                best_index = Some(index);
                best_offset = offset;
            }
        }
        best_index.map(|index| self.vertex_ranges.card_free_offsets.swap_remove(index))
    }

    fn grow_group_slots(&mut self) -> bool {
        let Some(scene) = &self.scene else {
            return false;
        };
        let additional_slots = group_spare_slot_count(scene.groups.len());
        let additional_vertices = additional_slots * GROUP_VERTEX_SLOT;
        if additional_vertices == 0 {
            return false;
        }

        let insert_offset = self.group_segment_end();
        let suffix_vertices = self.vertex_count.saturating_sub(insert_offset);
        let new_vertex_count = self.vertex_count + additional_vertices;
        let vertex_size = std::mem::size_of::<GpuVertex>() as u64;
        let new_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU primitive vertices grown for group slots"),
            size: (new_vertex_count * std::mem::size_of::<GpuVertex>()).max(4) as u64,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shape.ai WebGPU group slot growth encoder"),
            });
        if insert_offset > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                0,
                &new_buffer,
                0,
                insert_offset as u64 * vertex_size,
            );
        }
        if suffix_vertices > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                insert_offset as u64 * vertex_size,
                &new_buffer,
                (insert_offset + additional_vertices) as u64 * vertex_size,
                suffix_vertices as u64 * vertex_size,
            );
        }
        self.queue.submit(Some(encoder.finish()));

        let transparent_vertices = fit_vertices_to_slot(Vec::new(), additional_vertices);
        self.queue.write_buffer(
            &new_buffer,
            insert_offset as u64 * vertex_size,
            bytemuck::cast_slice(&transparent_vertices),
        );

        for slot in self.vertex_ranges.edges.values_mut() {
            slot.offset += additional_vertices;
        }
        for offset in &mut self.vertex_ranges.edge_free_offsets {
            *offset += additional_vertices;
        }
        for slot in self.vertex_ranges.cards.values_mut() {
            slot.offset += additional_vertices;
        }
        for offset in &mut self.vertex_ranges.card_free_offsets {
            *offset += additional_vertices;
        }
        for index in 0..additional_slots {
            self.vertex_ranges
                .group_free_offsets
                .push(insert_offset + index * GROUP_VERTEX_SLOT);
        }
        self.vertex_buffer = new_buffer;
        self.vertex_count = new_vertex_count;
        self.group_capacity_grow_count += 1;
        true
    }

    fn should_compact_group_slots(&self) -> bool {
        let used_slots = self.vertex_ranges.groups.len();
        let free_slots = self.vertex_ranges.group_free_offsets.len();
        let target_free_slots = group_spare_slot_count(used_slots);
        free_slots > target_free_slots * 2 && free_slots.saturating_sub(target_free_slots) >= 8
    }

    fn compact_group_slots(&mut self) -> bool {
        let target_free_slots = group_spare_slot_count(self.vertex_ranges.groups.len());
        let old_group_end = self.group_segment_end();
        if old_group_end > self.vertex_count {
            return false;
        }

        let used_groups: Vec<(String, VertexSlot)> = {
            let mut groups: Vec<(String, VertexSlot)> = self
                .vertex_ranges
                .groups
                .iter()
                .map(|(id, slot)| (id.clone(), *slot))
                .collect();
            groups.sort_by_key(|(_, slot)| slot.offset);
            groups
        };
        let compact_group_vertices = (used_groups.len() + target_free_slots) * GROUP_VERTEX_SLOT;
        let new_vertex_count =
            compact_group_vertices + self.vertex_count.saturating_sub(old_group_end);
        if new_vertex_count >= self.vertex_count {
            return false;
        }

        let vertex_size = std::mem::size_of::<GpuVertex>() as u64;
        let new_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU primitive vertices compacted for group slots"),
            size: (new_vertex_count * std::mem::size_of::<GpuVertex>()).max(4) as u64,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shape.ai WebGPU group slot compaction encoder"),
            });
        let mut compacted_groups = HashMap::new();
        for (index, (id, slot)) in used_groups.into_iter().enumerate() {
            let new_offset = index * GROUP_VERTEX_SLOT;
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                slot.offset as u64 * vertex_size,
                &new_buffer,
                new_offset as u64 * vertex_size,
                GROUP_VERTEX_SLOT as u64 * vertex_size,
            );
            compacted_groups.insert(
                id,
                VertexSlot {
                    offset: new_offset,
                    capacity: slot.capacity,
                    text_stats: slot.text_stats,
                },
            );
        }
        let free_start_offset = compacted_groups.len() * GROUP_VERTEX_SLOT;
        let new_suffix_start = free_start_offset + target_free_slots * GROUP_VERTEX_SLOT;
        let suffix_vertices = self.vertex_count - old_group_end;
        if suffix_vertices > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                old_group_end as u64 * vertex_size,
                &new_buffer,
                new_suffix_start as u64 * vertex_size,
                suffix_vertices as u64 * vertex_size,
            );
        }
        self.queue.submit(Some(encoder.finish()));

        let transparent_vertices =
            fit_vertices_to_slot(Vec::new(), target_free_slots * GROUP_VERTEX_SLOT);
        if !transparent_vertices.is_empty() {
            self.queue.write_buffer(
                &new_buffer,
                free_start_offset as u64 * vertex_size,
                bytemuck::cast_slice(&transparent_vertices),
            );
        }

        let suffix_delta = new_suffix_start as isize - old_group_end as isize;
        for slot in self.vertex_ranges.edges.values_mut() {
            slot.offset = slot.offset.saturating_add_signed(suffix_delta);
        }
        for offset in &mut self.vertex_ranges.edge_free_offsets {
            *offset = offset.saturating_add_signed(suffix_delta);
        }
        for slot in self.vertex_ranges.cards.values_mut() {
            slot.offset = slot.offset.saturating_add_signed(suffix_delta);
        }
        for offset in &mut self.vertex_ranges.card_free_offsets {
            *offset = offset.saturating_add_signed(suffix_delta);
        }
        self.vertex_ranges.groups = compacted_groups;
        self.vertex_ranges.group_free_offsets = (0..target_free_slots)
            .map(|index| free_start_offset + index * GROUP_VERTEX_SLOT)
            .collect();
        self.vertex_buffer = new_buffer;
        self.vertex_count = new_vertex_count;
        self.group_compaction_count += 1;
        true
    }

    fn grow_edge_slots(&mut self) -> bool {
        let Some(scene) = &self.scene else {
            return false;
        };
        let additional_slots = edge_spare_slot_count(scene.edges.len());
        let additional_vertices = additional_slots * EDGE_VERTEX_SLOT;
        if additional_vertices == 0 {
            return false;
        }

        let card_insert_offset = self.card_segment_start();
        let suffix_vertices = self.vertex_count.saturating_sub(card_insert_offset);
        let new_vertex_count = self.vertex_count + additional_vertices;
        let vertex_size = std::mem::size_of::<GpuVertex>() as u64;
        let new_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU primitive vertices grown for edge slots"),
            size: (new_vertex_count * std::mem::size_of::<GpuVertex>()).max(4) as u64,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shape.ai WebGPU edge slot growth encoder"),
            });
        if card_insert_offset > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                0,
                &new_buffer,
                0,
                card_insert_offset as u64 * vertex_size,
            );
        }
        if suffix_vertices > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                card_insert_offset as u64 * vertex_size,
                &new_buffer,
                (card_insert_offset + additional_vertices) as u64 * vertex_size,
                suffix_vertices as u64 * vertex_size,
            );
        }
        self.queue.submit(Some(encoder.finish()));

        let transparent_vertices = fit_vertices_to_slot(Vec::new(), additional_vertices);
        self.queue.write_buffer(
            &new_buffer,
            card_insert_offset as u64 * vertex_size,
            bytemuck::cast_slice(&transparent_vertices),
        );

        for slot in self.vertex_ranges.cards.values_mut() {
            slot.offset += additional_vertices;
        }
        for offset in &mut self.vertex_ranges.card_free_offsets {
            *offset += additional_vertices;
        }
        for index in 0..additional_slots {
            self.vertex_ranges
                .edge_free_offsets
                .push(card_insert_offset + index * EDGE_VERTEX_SLOT);
        }
        self.vertex_buffer = new_buffer;
        self.vertex_count = new_vertex_count;
        self.edge_capacity_grow_count += 1;
        true
    }

    fn should_compact_edge_slots(&self) -> bool {
        let used_slots = self.vertex_ranges.edges.len();
        let free_slots = self.vertex_ranges.edge_free_offsets.len();
        let target_free_slots = edge_spare_slot_count(used_slots);
        free_slots > target_free_slots * 2 && free_slots.saturating_sub(target_free_slots) >= 8
    }

    fn compact_edge_slots(&mut self) -> bool {
        let target_free_slots = edge_spare_slot_count(self.vertex_ranges.edges.len());
        let edge_start_offset = self.group_segment_end();
        let card_start_offset = self.card_segment_start();
        if card_start_offset < edge_start_offset || card_start_offset > self.vertex_count {
            return false;
        }

        let card_vertices = self.vertex_count - card_start_offset;
        let used_edges: Vec<(String, VertexSlot)> = {
            let mut edges: Vec<(String, VertexSlot)> = self
                .vertex_ranges
                .edges
                .iter()
                .map(|(id, slot)| (id.clone(), *slot))
                .collect();
            edges.sort_by_key(|(_, slot)| slot.offset);
            edges
        };
        let compact_edge_vertices = (used_edges.len() + target_free_slots) * EDGE_VERTEX_SLOT;
        let new_vertex_count = edge_start_offset + compact_edge_vertices + card_vertices;
        if new_vertex_count >= self.vertex_count {
            return false;
        }

        let vertex_size = std::mem::size_of::<GpuVertex>() as u64;
        let new_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU primitive vertices compacted for edge slots"),
            size: (new_vertex_count * std::mem::size_of::<GpuVertex>()).max(4) as u64,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shape.ai WebGPU edge slot compaction encoder"),
            });
        if edge_start_offset > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                0,
                &new_buffer,
                0,
                edge_start_offset as u64 * vertex_size,
            );
        }
        let mut compacted_edges = HashMap::new();
        for (index, (id, slot)) in used_edges.into_iter().enumerate() {
            let new_offset = edge_start_offset + index * EDGE_VERTEX_SLOT;
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                slot.offset as u64 * vertex_size,
                &new_buffer,
                new_offset as u64 * vertex_size,
                EDGE_VERTEX_SLOT as u64 * vertex_size,
            );
            compacted_edges.insert(
                id,
                VertexSlot {
                    offset: new_offset,
                    capacity: slot.capacity,
                    text_stats: slot.text_stats,
                },
            );
        }
        let free_start_offset = edge_start_offset + compacted_edges.len() * EDGE_VERTEX_SLOT;
        if card_vertices > 0 {
            let new_card_start_offset = free_start_offset + target_free_slots * EDGE_VERTEX_SLOT;
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                card_start_offset as u64 * vertex_size,
                &new_buffer,
                new_card_start_offset as u64 * vertex_size,
                card_vertices as u64 * vertex_size,
            );
        }
        self.queue.submit(Some(encoder.finish()));

        let transparent_vertices =
            fit_vertices_to_slot(Vec::new(), target_free_slots * EDGE_VERTEX_SLOT);
        if !transparent_vertices.is_empty() {
            self.queue.write_buffer(
                &new_buffer,
                free_start_offset as u64 * vertex_size,
                bytemuck::cast_slice(&transparent_vertices),
            );
        }

        let new_card_start_offset = free_start_offset + target_free_slots * EDGE_VERTEX_SLOT;
        let card_offset_delta = new_card_start_offset as isize - card_start_offset as isize;
        for slot in self.vertex_ranges.cards.values_mut() {
            slot.offset = slot.offset.saturating_add_signed(card_offset_delta);
        }
        for offset in &mut self.vertex_ranges.card_free_offsets {
            *offset = offset.saturating_add_signed(card_offset_delta);
        }
        self.vertex_ranges.edges = compacted_edges;
        self.vertex_ranges.edge_free_offsets = (0..target_free_slots)
            .map(|index| free_start_offset + index * EDGE_VERTEX_SLOT)
            .collect();
        self.vertex_buffer = new_buffer;
        self.vertex_count = new_vertex_count;
        self.edge_compaction_count += 1;
        true
    }

    fn grow_card_slots(&mut self) -> bool {
        let Some(scene) = &self.scene else {
            return false;
        };
        let additional_slots = card_spare_slot_count(scene.cards.len());
        let additional_vertices = additional_slots * CARD_VERTEX_SLOT;
        if additional_vertices == 0 {
            return false;
        }

        let old_vertex_count = self.vertex_count;
        let new_vertex_count = old_vertex_count + additional_vertices;
        let vertex_size = std::mem::size_of::<GpuVertex>() as u64;
        let new_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU primitive vertices grown for card slots"),
            size: (new_vertex_count * std::mem::size_of::<GpuVertex>()).max(4) as u64,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shape.ai WebGPU card slot growth encoder"),
            });
        if old_vertex_count > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                0,
                &new_buffer,
                0,
                old_vertex_count as u64 * vertex_size,
            );
        }
        self.queue.submit(Some(encoder.finish()));

        let transparent_vertices = fit_vertices_to_slot(Vec::new(), additional_vertices);
        self.queue.write_buffer(
            &new_buffer,
            old_vertex_count as u64 * vertex_size,
            bytemuck::cast_slice(&transparent_vertices),
        );

        for index in 0..additional_slots {
            self.vertex_ranges
                .card_free_offsets
                .push(old_vertex_count + index * CARD_VERTEX_SLOT);
        }
        self.vertex_buffer = new_buffer;
        self.vertex_count = new_vertex_count;
        self.card_capacity_grow_count += 1;
        true
    }

    fn should_compact_card_slots(&self) -> bool {
        let used_slots = self.vertex_ranges.cards.len();
        let free_slots = self.vertex_ranges.card_free_offsets.len();
        let target_free_slots = card_spare_slot_count(used_slots);
        free_slots > target_free_slots * 2 && free_slots.saturating_sub(target_free_slots) >= 8
    }

    fn compact_card_slots(&mut self) -> bool {
        let target_free_slots = card_spare_slot_count(self.vertex_ranges.cards.len());
        let card_start_offset = self.card_segment_start();
        if card_start_offset > self.vertex_count {
            return false;
        }

        let used_cards: Vec<(String, VertexSlot)> = {
            let mut cards: Vec<(String, VertexSlot)> = self
                .vertex_ranges
                .cards
                .iter()
                .map(|(id, slot)| (id.clone(), *slot))
                .collect();
            cards.sort_by_key(|(_, slot)| slot.offset);
            cards
        };
        let compact_card_vertices = (used_cards.len() + target_free_slots) * CARD_VERTEX_SLOT;
        let new_vertex_count = card_start_offset + compact_card_vertices;
        if new_vertex_count >= self.vertex_count {
            return false;
        }

        let vertex_size = std::mem::size_of::<GpuVertex>() as u64;
        let new_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU primitive vertices compacted for card slots"),
            size: (new_vertex_count * std::mem::size_of::<GpuVertex>()).max(4) as u64,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shape.ai WebGPU card slot compaction encoder"),
            });
        if card_start_offset > 0 {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                0,
                &new_buffer,
                0,
                card_start_offset as u64 * vertex_size,
            );
        }
        let mut compacted_cards = HashMap::new();
        for (index, (id, slot)) in used_cards.into_iter().enumerate() {
            let new_offset = card_start_offset + index * CARD_VERTEX_SLOT;
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                slot.offset as u64 * vertex_size,
                &new_buffer,
                new_offset as u64 * vertex_size,
                CARD_VERTEX_SLOT as u64 * vertex_size,
            );
            compacted_cards.insert(
                id,
                VertexSlot {
                    offset: new_offset,
                    capacity: slot.capacity,
                    text_stats: slot.text_stats,
                },
            );
        }
        self.queue.submit(Some(encoder.finish()));

        let free_start_offset = card_start_offset + compacted_cards.len() * CARD_VERTEX_SLOT;
        let transparent_vertices =
            fit_vertices_to_slot(Vec::new(), target_free_slots * CARD_VERTEX_SLOT);
        if !transparent_vertices.is_empty() {
            self.queue.write_buffer(
                &new_buffer,
                free_start_offset as u64 * vertex_size,
                bytemuck::cast_slice(&transparent_vertices),
            );
        }

        self.vertex_ranges.cards = compacted_cards;
        self.vertex_ranges.card_free_offsets = (0..target_free_slots)
            .map(|index| free_start_offset + index * CARD_VERTEX_SLOT)
            .collect();
        self.vertex_buffer = new_buffer;
        self.vertex_count = new_vertex_count;
        self.card_compaction_count += 1;
        true
    }

    fn group_segment_end(&self) -> usize {
        self.vertex_ranges
            .groups
            .values()
            .map(|slot| slot.offset + slot.capacity)
            .chain(
                self.vertex_ranges
                    .group_free_offsets
                    .iter()
                    .map(|offset| offset + GROUP_VERTEX_SLOT),
            )
            .max()
            .unwrap_or(0)
    }

    fn group_segment_used_end(&self) -> usize {
        self.vertex_ranges
            .groups
            .values()
            .map(|slot| slot.offset + slot.capacity)
            .max()
            .unwrap_or(0)
    }

    fn card_segment_start(&self) -> usize {
        self.vertex_ranges
            .cards
            .values()
            .map(|slot| slot.offset)
            .chain(self.vertex_ranges.card_free_offsets.iter().copied())
            .min()
            .unwrap_or(self.vertex_count)
    }

    fn card_segment_used_end(&self) -> usize {
        self.vertex_ranges
            .cards
            .values()
            .map(|slot| slot.offset + slot.capacity)
            .max()
            .unwrap_or_else(|| self.card_segment_start())
    }

    fn clear_deleted_edge(&mut self, id: &str) -> bool {
        let Some(slot) = self.vertex_ranges.edges.remove(id) else {
            return false;
        };
        self.write_slot_vertices(slot, Vec::new());
        self.vertex_ranges.edge_free_offsets.push(slot.offset);
        self.text_glyph_count = self
            .text_glyph_count
            .saturating_sub(slot.text_stats.glyph_count);
        self.fallback_text_glyph_count = self
            .fallback_text_glyph_count
            .saturating_sub(slot.text_stats.fallback_glyph_count);
        self.cjk_text_glyph_count = self
            .cjk_text_glyph_count
            .saturating_sub(slot.text_stats.cjk_glyph_count);
        self.font_fallback_run_count = self
            .font_fallback_run_count
            .saturating_sub(slot.text_stats.fallback_run_count);
        self.missing_text_glyph_count = self
            .missing_text_glyph_count
            .saturating_sub(slot.text_stats.missing_glyph_count);
        self.text_atlas_overflow_glyph_count = self
            .text_atlas_overflow_glyph_count
            .saturating_sub(slot.text_stats.atlas_overflow_glyph_count);
        self.text_missing_raster_glyph_count = self
            .text_missing_raster_glyph_count
            .saturating_sub(slot.text_stats.missing_raster_glyph_count);
        self.dirty_range_write_count += 1;
        true
    }

    fn clear_deleted_card(&mut self, id: &str) -> bool {
        let Some(slot) = self.vertex_ranges.cards.remove(id) else {
            return false;
        };
        self.write_slot_vertices(slot, Vec::new());
        self.vertex_ranges.card_free_offsets.push(slot.offset);
        self.text_glyph_count = self
            .text_glyph_count
            .saturating_sub(slot.text_stats.glyph_count);
        self.fallback_text_glyph_count = self
            .fallback_text_glyph_count
            .saturating_sub(slot.text_stats.fallback_glyph_count);
        self.cjk_text_glyph_count = self
            .cjk_text_glyph_count
            .saturating_sub(slot.text_stats.cjk_glyph_count);
        self.font_fallback_run_count = self
            .font_fallback_run_count
            .saturating_sub(slot.text_stats.fallback_run_count);
        self.missing_text_glyph_count = self
            .missing_text_glyph_count
            .saturating_sub(slot.text_stats.missing_glyph_count);
        self.text_atlas_overflow_glyph_count = self
            .text_atlas_overflow_glyph_count
            .saturating_sub(slot.text_stats.atlas_overflow_glyph_count);
        self.text_missing_raster_glyph_count = self
            .text_missing_raster_glyph_count
            .saturating_sub(slot.text_stats.missing_raster_glyph_count);
        self.dirty_range_write_count += 1;
        true
    }

    fn clear_deleted_group(&mut self, id: &str) -> bool {
        let Some(slot) = self.vertex_ranges.groups.remove(id) else {
            return false;
        };
        self.write_slot_vertices(slot, Vec::new());
        self.vertex_ranges.group_free_offsets.push(slot.offset);
        self.text_glyph_count = self
            .text_glyph_count
            .saturating_sub(slot.text_stats.glyph_count);
        self.fallback_text_glyph_count = self
            .fallback_text_glyph_count
            .saturating_sub(slot.text_stats.fallback_glyph_count);
        self.cjk_text_glyph_count = self
            .cjk_text_glyph_count
            .saturating_sub(slot.text_stats.cjk_glyph_count);
        self.font_fallback_run_count = self
            .font_fallback_run_count
            .saturating_sub(slot.text_stats.fallback_run_count);
        self.missing_text_glyph_count = self
            .missing_text_glyph_count
            .saturating_sub(slot.text_stats.missing_glyph_count);
        self.text_atlas_overflow_glyph_count = self
            .text_atlas_overflow_glyph_count
            .saturating_sub(slot.text_stats.atlas_overflow_glyph_count);
        self.text_missing_raster_glyph_count = self
            .text_missing_raster_glyph_count
            .saturating_sub(slot.text_stats.missing_raster_glyph_count);
        self.dirty_range_write_count += 1;
        true
    }

    fn record_vertex_fit_stats(&mut self, stats: VertexFitStats) {
        self.vertex_truncation_count += stats.truncation_count;
        self.truncated_vertex_count += stats.truncated_vertex_count;
    }

    fn write_slot_vertices(&mut self, slot: VertexSlot, vertices: Vec<GpuVertex>) {
        let (vertices, fit_stats) = fit_vertices_to_slot_with_stats(vertices, slot.capacity);
        self.record_vertex_fit_stats(fit_stats);
        let byte_offset = (slot.offset * std::mem::size_of::<GpuVertex>()) as u64;
        self.queue.write_buffer(
            &self.vertex_buffer,
            byte_offset,
            bytemuck::cast_slice(&vertices),
        );
    }

    fn flush_text_atlas(&mut self) {
        if !self.text_engine.take_atlas_dirty() {
            return;
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self._text_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            self.text_engine.atlas_pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(TEXT_ATLAS_WIDTH * 4),
                rows_per_image: Some(TEXT_ATLAS_HEIGHT),
            },
            wgpu::Extent3d {
                width: TEXT_ATLAS_WIDTH,
                height: TEXT_ATLAS_HEIGHT,
                depth_or_array_layers: 1,
            },
        );
    }
}

#[cfg(feature = "wgpu-probe")]
fn build_group_vertices(
    scene: &SceneSnapshot,
    group: &RenderGroup,
    text_layout_cache: &mut TextLayoutCache,
    text_engine: &mut TextEngine,
) -> (Vec<GpuVertex>, TextBuildStats) {
    let mut vertices = Vec::new();
    let style = resolve_shape_style(&scene.styles, &group.style_key);
    let selected = selection_is_group(&scene.selection, &group.id);
    let radius = if selected {
        style.radius.group_selected
    } else {
        style.radius.group
    };
    let stroke_width = if selected {
        style.stroke_width.group_selected
    } else {
        style.stroke_width.group
    };
    let stroke_alpha = if selected {
        style.state.selected_stroke_alpha
    } else {
        0.38
    };
    let surface_rect = inset_rect(&group.bounds, stroke_width);
    let top = mix_rgb(style.accent, style.surface, 0.86, 0.72);
    let bottom = color_with_alpha(style.surface, 0.36);
    add_layered_shadow(
        &mut vertices,
        &group.bounds,
        radius,
        if selected {
            &style.selected_shadow
        } else {
            &style.shadow
        },
        if selected {
            style.state.selected_shadow_alpha
        } else {
            style.state.shadow_alpha
        },
    );
    if selected {
        let focus_radius = style.radius.focus_ring.max(radius);
        add_focus_ring(
            &mut vertices,
            &group.bounds,
            focus_radius,
            style.stroke_width.focus_ring,
            color_with_alpha(style.focus, style.state.focus_alpha),
        );
    }
    add_rounded_rect(
        &mut vertices,
        &group.bounds,
        radius,
        color_with_alpha(style.accent, stroke_alpha),
    );
    add_gradient_banded_rect(
        &mut vertices,
        &surface_rect,
        radius - stroke_width,
        top,
        bottom,
    );
    add_inner_stroke(
        &mut vertices,
        &surface_rect,
        radius - stroke_width,
        style.stroke_width.inner,
        color_with_alpha(style.surface, 0.66),
    );
    add_separator(
        &mut vertices,
        group.bounds.x + style.spacing.group_padding_x,
        group.bounds.y
            + style.spacing.group_padding_y
            + style.typography.group_title_size as f64
            + 18.0,
        180.0,
        style.stroke_width.separator,
        color_with_alpha(style.line, 0.12),
    );
    let mut text_stats = add_text_line(
        &mut vertices,
        &group.title,
        (group.bounds.x + style.spacing.group_padding_x) as f32,
        (group.bounds.y + style.spacing.group_padding_y) as f32,
        (group.bounds.width - style.spacing.group_padding_x * 2.0) as f32,
        style.typography.group_title_size,
        color_with_alpha(style.text, 0.86),
        text_layout_cache,
        text_engine,
    );
    if !group.summary.trim().is_empty() {
        text_stats.add(add_text_line(
            &mut vertices,
            &group.summary,
            (group.bounds.x + style.spacing.group_padding_x) as f32,
            (group.bounds.y
                + style.spacing.group_padding_y
                + style.typography.group_title_size as f64
                + 28.0) as f32,
            (group.bounds.width - style.spacing.group_padding_x * 2.0) as f32,
            style.typography.group_summary_size,
            color_with_alpha(style.muted_text, 0.78),
            text_layout_cache,
            text_engine,
        ));
    }
    (vertices, text_stats)
}

#[cfg(feature = "wgpu-probe")]
fn build_edge_vertices(
    scene: &SceneSnapshot,
    edge: &RenderEdge,
    text_layout_cache: &mut TextLayoutCache,
    text_engine: &mut TextEngine,
) -> (Vec<GpuVertex>, TextBuildStats) {
    let mut vertices = Vec::new();
    let Some(source) = scene.cards.iter().find(|card| card.id == edge.source) else {
        return (vertices, TextBuildStats::default());
    };
    let Some(target) = scene.cards.iter().find(|card| card.id == edge.target) else {
        return (vertices, TextBuildStats::default());
    };
    let route = edge_route(source, target);
    let selected = selection_is_edge(&scene.selection, &edge.id);
    let style = resolve_shape_style(&scene.styles, &edge.style_key);
    let compact = edge.label.trim().is_empty();
    let stroke_width = if selected {
        style.stroke_width.edge_selected
    } else if compact {
        style.stroke_width.edge_compact
    } else {
        style.stroke_width.edge
    };
    let stroke_alpha = if selected {
        style.edge.selected_stroke_alpha
    } else if compact {
        style
            .edge
            .compact_stroke_alpha
            .min(style.state.compact_stroke_alpha)
    } else {
        style.edge.stroke_alpha
    };
    let edge_base = if edge.style_key == "default" {
        style.line_strong
    } else {
        mix_rgb(style.stroke, style.accent, 0.5, 1.0)
    };
    if selected {
        add_cubic_edge(
            &mut vertices,
            &route,
            (stroke_width + style.stroke_width.focus_ring) as f32,
            color_with_alpha(style.focus, style.state.focus_alpha),
        );
    }
    let stroke = color_with_alpha(edge_base, stroke_alpha);
    add_cubic_edge(&mut vertices, &route, stroke_width as f32, stroke);
    add_arrowhead(
        &mut vertices,
        [route.cp2.x as f32, route.cp2.y as f32],
        [route.end.x as f32, route.end.y as f32],
        color_with_alpha(edge_base, if selected { 0.98 } else { 0.62 }),
    );
    let mut text_stats = TextBuildStats::default();
    if !edge.label.trim().is_empty() {
        let label_font_size = style.typography.edge_label_size;
        let label_max_width = 180.0;
        let label_width = text_engine
            .measure_text_width(&edge.label, label_font_size)
            .min(label_max_width)
            .max(24.0);
        let label_padding = style.spacing.label_padding_x as f32;
        let label_x =
            ((route.start.x + route.end.x) * 0.5) as f32 - label_width * 0.5 - label_padding;
        let label_y =
            ((route.start.y + route.end.y) * 0.5) as f32 - style.spacing.edge_label_height as f32;
        let label_rect = WorldRect {
            x: label_x as f64,
            y: label_y as f64,
            width: (label_width + label_padding * 2.0) as f64,
            height: style.spacing.edge_label_height,
        };
        add_edge_label_capsule(&mut vertices, &label_rect, &style, selected);
        text_stats.add(add_text_line(
            &mut vertices,
            &edge.label,
            label_x + label_padding,
            label_y + 4.0,
            label_width,
            label_font_size,
            color_with_alpha(style.text, style.edge.label_text_alpha),
            text_layout_cache,
            text_engine,
        ));
    }
    (vertices, text_stats)
}

#[cfg(feature = "wgpu-probe")]
fn build_card_vertices(
    scene: &SceneSnapshot,
    card: &RenderCard,
    text_layout_cache: &mut TextLayoutCache,
    text_engine: &mut TextEngine,
) -> (Vec<GpuVertex>, TextBuildStats) {
    let mut vertices = Vec::new();
    let mut text_stats = TextBuildStats::default();
    let style = resolve_shape_style(&scene.styles, &card.style_key);
    let selected = selection_is_node(&scene.selection, &card.id);
    let text_layout = card_text_layout(&card.bounds, &style, selected);
    let radius = if selected {
        style.radius.card_selected
    } else {
        style.radius.card
    };
    let stroke_width = if selected {
        style.stroke_width.card_selected
    } else {
        style.stroke_width.card
    };
    let surface_rect = inset_rect(&card.bounds, stroke_width);
    add_layered_shadow(
        &mut vertices,
        &card.bounds,
        radius,
        if selected {
            &style.selected_shadow
        } else {
            &style.shadow
        },
        if selected {
            style.state.selected_shadow_alpha
        } else {
            style.state.shadow_alpha
        },
    );
    if selected {
        for glow in &style.glow {
            add_layered_shadow(
                &mut vertices,
                &card.bounds,
                radius,
                std::slice::from_ref(glow),
                style.state.glow_alpha,
            );
        }
        let focus_radius = style.radius.focus_ring.max(radius);
        add_focus_ring(
            &mut vertices,
            &card.bounds,
            focus_radius,
            style.stroke_width.focus_ring,
            color_with_alpha(style.accent, style.state.focus_alpha),
        );
    }
    add_rounded_rect(
        &mut vertices,
        &card.bounds,
        radius,
        color_with_alpha(
            style.accent,
            if selected {
                style.state.selected_stroke_alpha
            } else {
                style.state.default_stroke_alpha
            },
        ),
    );
    add_gradient_banded_rect(
        &mut vertices,
        &surface_rect,
        radius - stroke_width,
        color_with_alpha(
            style.surface,
            if selected {
                style.state.selected_fill_alpha
            } else {
                style
                    .gradient
                    .surface_top_alpha
                    .min(style.state.default_fill_alpha)
            },
        ),
        mix_rgb(
            style.fill,
            style.pastel,
            0.66,
            style.gradient.pastel_bottom_alpha,
        ),
    );
    add_inner_stroke(
        &mut vertices,
        &surface_rect,
        radius - stroke_width,
        style.stroke_width.inner,
        color_with_alpha(style.surface, 0.88),
    );
    add_accent_strip(&mut vertices, &card.bounds, &style);
    let node_label = node_type_label(&card.node_type);
    let badge = badge_rect(&card.bounds, &style, &node_label, text_engine);
    add_badge_pill(&mut vertices, &badge, &style);
    add_separator(
        &mut vertices,
        card.bounds.x + style.spacing.card_padding,
        card.bounds.y
            + style.spacing.card_padding
            + style.spacing.badge_height
            + style.spacing.card_gap,
        card.bounds.width - style.spacing.card_padding * 2.0,
        style.stroke_width.separator,
        color_with_alpha(style.line, 0.12),
    );
    add_port_markers(&mut vertices, &card.bounds, &style, selected);
    text_stats.add(add_text_line(
        &mut vertices,
        &node_label,
        (badge.x + style.spacing.badge_padding_x) as f32,
        (badge.y + 5.0) as f32,
        (badge.width - style.spacing.badge_padding_x * 2.0) as f32,
        style.typography.badge_size,
        color_with_alpha(style.accent, style.badge.text_alpha),
        text_layout_cache,
        text_engine,
    ));
    text_stats.add(add_text_line(
        &mut vertices,
        &card.title,
        text_layout.content_x as f32,
        text_layout.title_y as f32,
        text_layout.content_width as f32,
        text_layout.title_font_size,
        color_with_alpha(style.text, 0.92),
        text_layout_cache,
        text_engine,
    ));
    text_stats.add(add_wrapped_text(
        &mut vertices,
        &card.summary,
        text_layout.content_x as f32,
        text_layout.summary_y as f32,
        text_layout.content_width as f32,
        text_layout.summary_font_size,
        text_layout.summary_line_height as f32,
        text_layout.summary_max_lines,
        color_with_alpha(style.muted_text, 0.84),
        text_layout_cache,
        text_engine,
    ));
    if text_layout.detail_max_lines > 0 && !card.detail.trim().is_empty() {
        text_stats.add(add_wrapped_text(
            &mut vertices,
            &card.detail,
            text_layout.content_x as f32,
            text_layout.detail_y as f32,
            text_layout.content_width as f32,
            text_layout.detail_font_size,
            text_layout.detail_line_height as f32,
            text_layout.detail_max_lines,
            color_with_alpha(style.muted_text, 0.68),
            text_layout_cache,
            text_engine,
        ));
    }
    (vertices, text_stats)
}

#[cfg(feature = "wgpu-probe")]
fn fit_vertices_to_slot(vertices: Vec<GpuVertex>, capacity: usize) -> Vec<GpuVertex> {
    fit_vertices_to_slot_with_stats(vertices, capacity).0
}

#[cfg(feature = "wgpu-probe")]
fn fit_vertices_to_slot_with_stats(
    mut vertices: Vec<GpuVertex>,
    capacity: usize,
) -> (Vec<GpuVertex>, VertexFitStats) {
    let original_len = vertices.len();
    let safe_capacity = capacity - (capacity % 3);
    if vertices.len() > safe_capacity {
        vertices.truncate(safe_capacity);
    }
    let truncated_vertex_count = original_len.saturating_sub(vertices.len());
    let stats = VertexFitStats {
        truncation_count: usize::from(truncated_vertex_count > 0),
        truncated_vertex_count,
    };
    vertices.resize(capacity, transparent_vertex());
    (vertices, stats)
}

#[cfg(feature = "wgpu-probe")]
fn edge_spare_slot_count(edge_count: usize) -> usize {
    (edge_count / 8).clamp(8, 256)
}

#[cfg(feature = "wgpu-probe")]
fn group_spare_slot_count(group_count: usize) -> usize {
    (group_count / 8).clamp(4, 128)
}

#[cfg(feature = "wgpu-probe")]
fn card_spare_slot_count(card_count: usize) -> usize {
    (card_count / 8).clamp(8, 256)
}

#[cfg(feature = "wgpu-probe")]
fn collect_selection_dirty_ids(
    selection: &SceneSelection,
    groups: &mut Vec<String>,
    cards: &mut Vec<String>,
    edges: &mut Vec<String>,
) {
    match selection {
        SceneSelection::Canvas => {}
        SceneSelection::Group { id } => push_unique(groups, id),
        SceneSelection::Node { id } => push_unique(cards, id),
        SceneSelection::Edge { id } => push_unique(edges, id),
    }
}

#[cfg(feature = "wgpu-probe")]
fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|candidate| candidate == value) {
        values.push(value.to_string());
    }
}

#[cfg(feature = "wgpu-probe")]
fn selection_is_group(selection: &SceneSelection, id: &str) -> bool {
    matches!(selection, SceneSelection::Group { id: selected } if selected == id)
}

#[cfg(feature = "wgpu-probe")]
fn selection_is_node(selection: &SceneSelection, id: &str) -> bool {
    matches!(selection, SceneSelection::Node { id: selected } if selected == id)
}

#[cfg(feature = "wgpu-probe")]
fn selection_is_edge(selection: &SceneSelection, id: &str) -> bool {
    matches!(selection, SceneSelection::Edge { id: selected } if selected == id)
}

#[cfg(feature = "wgpu-probe")]
fn transparent_vertex() -> GpuVertex {
    GpuVertex {
        position: [0.0, 0.0],
        uv: SOLID_UV[0],
        color: [0.0, 0.0, 0.0, 0.0],
    }
}

#[cfg(feature = "wgpu-probe")]
fn add_rect(vertices: &mut Vec<GpuVertex>, rect: &WorldRect, color: [f32; 4]) {
    let x = rect.x as f32;
    let y = rect.y as f32;
    let w = rect.width as f32;
    let h = rect.height as f32;
    add_quad(
        vertices,
        [x, y],
        [x + w, y],
        [x + w, y + h],
        [x, y + h],
        color,
    );
}

#[cfg(feature = "wgpu-probe")]
fn add_rounded_rect(vertices: &mut Vec<GpuVertex>, rect: &WorldRect, radius: f64, color: [f32; 4]) {
    let w = rect.width.max(0.0) as f32;
    let h = rect.height.max(0.0) as f32;
    let r = (radius.max(0.0) as f32).min(w * 0.5).min(h * 0.5);
    if r <= 0.5 || w <= 1.0 || h <= 1.0 {
        add_rect(vertices, rect, color);
        return;
    }

    add_rect(
        vertices,
        &WorldRect {
            x: rect.x + r as f64,
            y: rect.y,
            width: (w - r * 2.0) as f64,
            height: h as f64,
        },
        color,
    );
    add_rect(
        vertices,
        &WorldRect {
            x: rect.x,
            y: rect.y + r as f64,
            width: r as f64,
            height: (h - r * 2.0) as f64,
        },
        color,
    );
    add_rect(
        vertices,
        &WorldRect {
            x: rect.x + (w - r) as f64,
            y: rect.y + r as f64,
            width: r as f64,
            height: (h - r * 2.0) as f64,
        },
        color,
    );
    add_corner_fan(
        vertices,
        [rect.x as f32 + r, rect.y as f32 + r],
        r,
        PI,
        PI * 1.5,
        color,
    );
    add_corner_fan(
        vertices,
        [rect.x as f32 + w - r, rect.y as f32 + r],
        r,
        PI * 1.5,
        PI * 2.0,
        color,
    );
    add_corner_fan(
        vertices,
        [rect.x as f32 + w - r, rect.y as f32 + h - r],
        r,
        0.0,
        PI * 0.5,
        color,
    );
    add_corner_fan(
        vertices,
        [rect.x as f32 + r, rect.y as f32 + h - r],
        r,
        PI * 0.5,
        PI,
        color,
    );
}

#[cfg(feature = "wgpu-probe")]
fn add_corner_fan(
    vertices: &mut Vec<GpuVertex>,
    center: [f32; 2],
    radius: f32,
    start_angle: f32,
    end_angle: f32,
    color: [f32; 4],
) {
    let step = (end_angle - start_angle) / ROUNDED_CORNER_SEGMENTS as f32;
    for index in 0..ROUNDED_CORNER_SEGMENTS {
        let a0 = start_angle + step * index as f32;
        let a1 = start_angle + step * (index + 1) as f32;
        vertices.push(GpuVertex {
            position: center,
            uv: SOLID_UV[0],
            color,
        });
        vertices.push(GpuVertex {
            position: [center[0] + a0.cos() * radius, center[1] + a0.sin() * radius],
            uv: SOLID_UV[0],
            color,
        });
        vertices.push(GpuVertex {
            position: [center[0] + a1.cos() * radius, center[1] + a1.sin() * radius],
            uv: SOLID_UV[0],
            color,
        });
    }
}

#[cfg(feature = "wgpu-probe")]
fn add_gradient_banded_rect(
    vertices: &mut Vec<GpuVertex>,
    rect: &WorldRect,
    radius: f64,
    top: [f32; 4],
    bottom: [f32; 4],
) {
    add_rounded_rect(vertices, rect, radius, bottom);
    let band_count = SOFT_GRADIENT_BANDS.max(1);
    let band_height = rect.height / band_count as f64;
    for index in 0..band_count {
        let t = index as f32 / (band_count - 1).max(1) as f32;
        let color = mix_color(top, bottom, t);
        add_rect(
            vertices,
            &WorldRect {
                x: rect.x,
                y: rect.y + band_height * index as f64,
                width: rect.width,
                height: band_height + 0.5,
            },
            color,
        );
    }
}

#[cfg(feature = "wgpu-probe")]
fn add_layered_shadow(
    vertices: &mut Vec<GpuVertex>,
    rect: &WorldRect,
    radius: f64,
    layers: &[ShapeShadowLayer],
    alpha_scale: f32,
) {
    for layer in layers {
        let spread = layer.spread + layer.blur * 0.16;
        add_rounded_rect(
            vertices,
            &WorldRect {
                x: rect.x + layer.offset_x - spread,
                y: rect.y + layer.offset_y - spread,
                width: rect.width + spread * 2.0,
                height: rect.height + spread * 2.0,
            },
            radius + spread,
            color_with_alpha(layer.color, layer.color[3] * alpha_scale),
        );
    }
}

#[cfg(feature = "wgpu-probe")]
fn add_focus_ring(
    vertices: &mut Vec<GpuVertex>,
    rect: &WorldRect,
    radius: f64,
    width: f64,
    color: [f32; 4],
) {
    add_rounded_rect(vertices, &expand_rect(rect, width), radius + width, color);
}

#[cfg(feature = "wgpu-probe")]
fn add_inner_stroke(
    vertices: &mut Vec<GpuVertex>,
    rect: &WorldRect,
    radius: f64,
    width: f64,
    color: [f32; 4],
) {
    let highlight_width = (rect.width - radius * 2.0).max(0.0);
    add_rect(
        vertices,
        &WorldRect {
            x: rect.x + radius,
            y: rect.y + width,
            width: highlight_width,
            height: width.max(1.0),
        },
        color,
    );
}

#[cfg(feature = "wgpu-probe")]
fn add_separator(
    vertices: &mut Vec<GpuVertex>,
    x: f64,
    y: f64,
    width: f64,
    thickness: f64,
    color: [f32; 4],
) {
    add_rect(
        vertices,
        &WorldRect {
            x,
            y,
            width: width.max(0.0),
            height: thickness.max(1.0),
        },
        color,
    );
}

#[cfg(feature = "wgpu-probe")]
fn add_accent_strip(vertices: &mut Vec<GpuVertex>, card: &WorldRect, style: &ShapeRenderStyle) {
    let inset = style.spacing.separator_inset;
    let strip = WorldRect {
        x: card.x + inset,
        y: card.y,
        width: (card.width - inset * 2.0).max(0.0),
        height: 4.0,
    };
    add_gradient_banded_rect(
        vertices,
        &strip,
        5.0,
        color_with_alpha(style.accent, style.gradient.accent_start_alpha),
        color_with_alpha(style.accent, style.gradient.accent_end_alpha),
    );
}

#[cfg(feature = "wgpu-probe")]
fn add_badge_pill(vertices: &mut Vec<GpuVertex>, rect: &WorldRect, style: &ShapeRenderStyle) {
    add_rounded_rect(
        vertices,
        rect,
        style.radius.badge,
        color_with_alpha(style.accent, style.badge.stroke_alpha),
    );
    add_rounded_rect(
        vertices,
        &inset_rect(rect, 1.0),
        (style.radius.badge - 1.0).max(0.0),
        color_with_alpha(style.accent, style.badge.fill_alpha),
    );
}

#[cfg(feature = "wgpu-probe")]
fn add_edge_label_capsule(
    vertices: &mut Vec<GpuVertex>,
    rect: &WorldRect,
    style: &ShapeRenderStyle,
    selected: bool,
) {
    if selected {
        add_focus_ring(
            vertices,
            rect,
            style.radius.edge_label,
            style.stroke_width.focus_ring * 0.5,
            color_with_alpha(style.focus, style.state.focus_alpha),
        );
    }
    add_rounded_rect(
        vertices,
        rect,
        style.radius.edge_label,
        color_with_alpha(style.line_strong, style.edge.label_stroke_alpha),
    );
    add_rounded_rect(
        vertices,
        &inset_rect(rect, 1.0),
        (style.radius.edge_label - 1.0).max(0.0),
        mix_rgb(
            style.surface3,
            style.surface2,
            0.32,
            style.edge.label_fill_alpha,
        ),
    );
}

#[cfg(feature = "wgpu-probe")]
fn add_port_markers(
    vertices: &mut Vec<GpuVertex>,
    card: &WorldRect,
    style: &ShapeRenderStyle,
    selected: bool,
) {
    let r = style.spacing.port_radius;
    let y = card.y + card.height / 2.0 - r;
    let ports = [
        WorldRect {
            x: card.x - r,
            y,
            width: r * 2.0,
            height: r * 2.0,
        },
        WorldRect {
            x: card.x + card.width - r,
            y,
            width: r * 2.0,
            height: r * 2.0,
        },
    ];
    for port in ports {
        add_rounded_rect(
            vertices,
            &port,
            style.radius.port,
            color_with_alpha(
                style.accent,
                if selected {
                    style.port.selected_stroke_alpha
                } else {
                    style.port.stroke_alpha
                },
            ),
        );
        add_rounded_rect(
            vertices,
            &inset_rect(&port, style.stroke_width.port),
            (style.radius.port - style.stroke_width.port).max(0.0),
            color_with_alpha(
                style.surface,
                if selected {
                    style.port.selected_fill_alpha
                } else {
                    style.port.fill_alpha
                },
            ),
        );
    }
}

#[cfg(feature = "wgpu-probe")]
fn inset_rect(rect: &WorldRect, inset: f64) -> WorldRect {
    let inset = inset.max(0.0);
    WorldRect {
        x: rect.x + inset,
        y: rect.y + inset,
        width: (rect.width - inset * 2.0).max(0.0),
        height: (rect.height - inset * 2.0).max(0.0),
    }
}

#[cfg(feature = "wgpu-probe")]
fn expand_rect(rect: &WorldRect, spread: f64) -> WorldRect {
    WorldRect {
        x: rect.x - spread,
        y: rect.y - spread,
        width: rect.width + spread * 2.0,
        height: rect.height + spread * 2.0,
    }
}

#[cfg(feature = "wgpu-probe")]
fn add_text_line(
    vertices: &mut Vec<GpuVertex>,
    value: &str,
    x: f32,
    y: f32,
    max_width: f32,
    font_size: f32,
    color: [f32; 4],
    text_layout_cache: &mut TextLayoutCache,
    text_engine: &mut TextEngine,
) -> TextBuildStats {
    let line = text_layout_cache.text_line(text_engine, value, max_width, font_size);
    add_cached_text_line(vertices, &line, x, y, color);
    line.stats
}

#[cfg(feature = "wgpu-probe")]
fn add_cached_text_line(
    vertices: &mut Vec<GpuVertex>,
    line: &CachedTextLine,
    x: f32,
    y: f32,
    color: [f32; 4],
) {
    for glyph in &line.glyphs {
        let left = x + glyph.offset_x;
        let top = y + glyph.offset_y;
        let right = left + glyph.width;
        let bottom = top + glyph.height;
        add_quad_uv(
            vertices,
            [left, top],
            [right, top],
            [right, bottom],
            [left, bottom],
            glyph.uv,
            color,
        );
    }
}

#[cfg(feature = "wgpu-probe")]
fn add_wrapped_text(
    vertices: &mut Vec<GpuVertex>,
    value: &str,
    x: f32,
    y: f32,
    max_width: f32,
    font_size: f32,
    line_height: f32,
    max_lines: usize,
    color: [f32; 4],
    text_layout_cache: &mut TextLayoutCache,
    text_engine: &mut TextEngine,
) -> TextBuildStats {
    let lines = text_layout_cache.wrap_lines(text_engine, value, max_width, font_size, max_lines);
    let mut text_stats = TextBuildStats::default();
    for (line_index, line) in lines.iter().enumerate() {
        text_stats.add(add_text_line(
            vertices,
            line,
            x,
            y + line_index as f32 * line_height,
            max_width,
            font_size,
            color,
            text_layout_cache,
            text_engine,
        ));
    }
    text_stats
}

#[cfg(all(test, feature = "wgpu-probe"))]
mod tests {
    use super::*;

    #[test]
    fn add_text_line_tracks_shaped_font_fallback_and_cjk_glyphs() {
        let mut vertices = Vec::new();
        let mut text_layout_cache = TextLayoutCache::default();
        let mut text_engine = TextEngine::new().unwrap();

        let stats = add_text_line(
            &mut vertices,
            "A한B",
            0.0,
            0.0,
            200.0,
            14.0,
            [1.0, 1.0, 1.0, 1.0],
            &mut text_layout_cache,
            &mut text_engine,
        );

        assert_eq!(stats.glyph_count, 3);
        assert_eq!(stats.fallback_glyph_count, 1);
        assert_eq!(stats.fallback_run_count, 1);
        assert_eq!(stats.cjk_glyph_count, 1);
        assert_eq!(stats.missing_glyph_count, 0);
        assert_eq!(vertices.len(), 18);
        assert_eq!(text_layout_cache.misses, 1);
    }

    #[test]
    fn wrapped_summary_uses_shaped_ellipsis() {
        let mut vertices = Vec::new();
        let mut text_layout_cache = TextLayoutCache::default();
        let mut text_engine = TextEngine::new().unwrap();

        let stats = add_wrapped_text(
            &mut vertices,
            "한글테스트문장공백없음입니다",
            0.0,
            0.0,
            72.0,
            18.0,
            22.0,
            2,
            [1.0, 1.0, 1.0, 1.0],
            &mut text_layout_cache,
            &mut text_engine,
        );

        assert!(stats.glyph_count > 0);
        assert!(stats.cjk_glyph_count > 0);
        assert!(vertices.len() > 18);
    }

    #[test]
    fn multiline_card_summary_uses_shaped_text_path() {
        let card = RenderCard {
            id: "card-a".to_string(),
            group_id: "group-a".to_string(),
            title: "Multiline summary".to_string(),
            summary: "First line, punctuation.\n둘째 줄 summary?".to_string(),
            detail: String::new(),
            status: "draft".to_string(),
            node_type: "task".to_string(),
            bounds: WorldRect {
                x: 120.0,
                y: 80.0,
                width: 320.0,
                height: 190.0,
            },
            z_index: 1.0,
            style_key: "default".to_string(),
            accessibility_label: String::new(),
        };
        let scene = SceneSnapshot {
            scene_id: "summary-test".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            groups: Vec::new(),
            cards: vec![card.clone()],
            edges: Vec::new(),
            styles: vec![minimal_style_token("default")],
            selection: SceneSelection::Node {
                id: card.id.clone(),
            },
        };
        let mut cache = TextLayoutCache::default();
        let mut text_engine = TextEngine::new().unwrap();

        let (vertices, stats) = build_card_vertices(&scene, &card, &mut cache, &mut text_engine);

        assert!(vertices.len() > 420);
        assert!(stats.glyph_count > 20);
        assert!(stats.fallback_glyph_count > 0);
        assert!(stats.cjk_glyph_count > 0);
        assert_eq!(stats.atlas_overflow_glyph_count, 0);
        assert_eq!(stats.missing_raster_glyph_count, 0);
    }

    #[test]
    fn edge_label_uses_shaped_text_path() {
        let source = text_path_card("source", 80.0, 80.0);
        let target = text_path_card("target", 480.0, 160.0);
        let edge = RenderEdge {
            id: "edge-a".to_string(),
            group_id: "group-a".to_string(),
            source: source.id.clone(),
            target: target.id.clone(),
            label: "relates: 한글?".to_string(),
            edge_type: "dependency".to_string(),
            z_index: 0.0,
            style_key: "default".to_string(),
        };
        let scene = SceneSnapshot {
            scene_id: "edge-label-test".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            groups: Vec::new(),
            cards: vec![source, target],
            edges: vec![edge.clone()],
            styles: vec![minimal_style_token("default")],
            selection: SceneSelection::Edge {
                id: edge.id.clone(),
            },
        };
        let mut cache = TextLayoutCache::default();
        let mut text_engine = TextEngine::new().unwrap();

        let (vertices, stats) = build_edge_vertices(&scene, &edge, &mut cache, &mut text_engine);

        assert!(vertices.len() > 60);
        assert!(stats.glyph_count >= 8);
        assert!(stats.fallback_glyph_count > 0);
        assert!(stats.cjk_glyph_count > 0);
        assert_eq!(stats.atlas_overflow_glyph_count, 0);
        assert_eq!(stats.missing_raster_glyph_count, 0);
    }

    #[test]
    fn slot_fit_reports_truncated_vertices() {
        let vertices = vec![transparent_vertex(); 10];

        let (fitted, stats) = fit_vertices_to_slot_with_stats(vertices, 6);

        assert_eq!(fitted.len(), 6);
        assert_eq!(stats.truncation_count, 1);
        assert_eq!(stats.truncated_vertex_count, 4);
    }

    #[test]
    fn legacy_style_token_resolves_rich_shape_defaults() {
        let token = minimal_style_token("default");

        let style = resolve_shape_style(&[token], "default");

        assert_eq!(style.radius.card, 16.0);
        assert_eq!(style.stroke_width.edge_selected, 5.0);
        assert_eq!(style.typography.card_title_size, 19.0);
        assert_eq!(style.shadow.len(), 2);
        assert_eq!(style.state.shadow_alpha, 1.0);
        assert_eq!(style.state.glow_alpha, 1.0);
        assert!((style.accent[2] - 0.9).abs() < 0.01);
    }

    #[test]
    fn custom_state_opacity_tokens_resolve_for_render_primitives() {
        let token = custom_style_token();

        let style = resolve_shape_style(&[token], "custom");

        assert_eq!(style.state.shadow_alpha, 0.25);
        assert_eq!(style.state.selected_shadow_alpha, 0.5);
        assert_eq!(style.state.glow_alpha, 0.33);
    }

    #[test]
    fn layered_shadow_applies_state_alpha_scale() {
        let mut vertices = Vec::new();
        let layer = ShapeShadowLayer {
            offset_x: 0.0,
            offset_y: 0.0,
            blur: 0.0,
            spread: 0.0,
            color: [0.0, 0.0, 0.0, 0.8],
        };

        add_layered_shadow(
            &mut vertices,
            &WorldRect {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 80.0,
            },
            12.0,
            &[layer],
            0.25,
        );

        assert!((vertices[0].color[3] - 0.2).abs() < 0.0001);
    }

    #[test]
    fn rounded_rect_generates_shape_primitive_geometry() {
        let mut vertices = Vec::new();
        add_rounded_rect(
            &mut vertices,
            &WorldRect {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 80.0,
            },
            16.0,
            [1.0, 1.0, 1.0, 1.0],
        );

        assert!(vertices.len() > 6);
        assert_eq!(vertices.len() % 3, 0);
    }

    #[test]
    fn rich_card_primitives_fit_dirty_write_slot() {
        let card = RenderCard {
            id: "card-a".to_string(),
            group_id: "group-a".to_string(),
            title: "Decision rendering".to_string(),
            summary: "Rust draws product shadows, gradient surface, badge, separator, focus ring, and ports.".to_string(),
            detail: String::new(),
            status: "draft".to_string(),
            node_type: "decision_point".to_string(),
            bounds: WorldRect {
                x: 120.0,
                y: 80.0,
                width: 320.0,
                height: 172.0,
            },
            z_index: 1.0,
            style_key: "default".to_string(),
            accessibility_label: String::new(),
        };
        let scene = SceneSnapshot {
            scene_id: "style-test".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            groups: Vec::new(),
            cards: vec![card.clone()],
            edges: Vec::new(),
            styles: vec![minimal_style_token("default")],
            selection: SceneSelection::Node {
                id: card.id.clone(),
            },
        };
        let mut cache = TextLayoutCache::default();
        let mut text_engine = TextEngine::new().unwrap();

        let (vertices, stats) = build_card_vertices(&scene, &card, &mut cache, &mut text_engine);

        assert!(vertices.len() > 420);
        assert!(vertices.len() <= CARD_VERTEX_SLOT);
        assert!(stats.glyph_count > 0);
    }

    #[test]
    fn wheel_zoom_keeps_cursor_world_point_stable() {
        let camera = CameraState {
            x: 20.0,
            y: -10.0,
            zoom: 0.5,
        };
        let screen = WorldPoint { x: 260.0, y: 180.0 };
        let before = screen_to_world(screen, &camera);

        let next = zoom_camera_at_screen(&camera, screen, -120.0);
        let after = screen_to_world(screen, &next);

        assert!(next.zoom > camera.zoom);
        assert!((before.x - after.x).abs() < 0.0001);
        assert!((before.y - after.y).abs() < 0.0001);
    }

    #[test]
    fn focus_bounds_centers_target_with_renderer_camera_math() {
        let bounds = WorldRect {
            x: 100.0,
            y: 200.0,
            width: 400.0,
            height: 300.0,
        };

        let camera = focus_camera_to_bounds(
            &bounds,
            Some(WorldPoint { x: 640.0, y: 360.0 }),
            None,
            Some(WorldPoint { x: 110.0, y: 140.0 }),
            Some(0.36),
            Some(0.58),
            1280.0,
            720.0,
        );

        assert!((camera.zoom - 0.58).abs() < 0.0001);
        assert!((camera.x - 466.0).abs() < 0.0001);
        assert!((camera.y - 157.0).abs() < 0.0001);
    }

    #[test]
    fn overlay_rect_comes_from_rust_card_geometry() {
        let card = WorldRect {
            x: 120.0,
            y: 80.0,
            width: 320.0,
            height: 172.0,
        };
        let style = resolve_shape_style(&[minimal_style_token("default")], "default");

        assert_eq!(text_field_rect(&card, &style, "title", false).x, 134.0);
        assert_eq!(text_field_rect(&card, &style, "summary", false).y, 175.0);
        let detail_rect = text_field_rect(&card, &style, "detail", false);
        assert!(detail_rect.y > text_field_rect(&card, &style, "summary", false).y);
        assert!(detail_rect.height > 18.0);
    }

    #[test]
    fn text_hit_and_overlay_geometry_follow_custom_style_tokens() {
        let token = custom_style_token();
        let style = resolve_shape_style(&[token.clone()], "custom");
        let card = RenderCard {
            id: "card-a".to_string(),
            group_id: "group-a".to_string(),
            title: "Styled card".to_string(),
            summary: "Styled summary".to_string(),
            detail: "Styled detail".to_string(),
            status: "draft".to_string(),
            node_type: "task".to_string(),
            bounds: WorldRect {
                x: 100.0,
                y: 50.0,
                width: 260.0,
                height: 190.0,
            },
            z_index: 0.0,
            style_key: "custom".to_string(),
            accessibility_label: String::new(),
        };

        let title_rect = text_field_rect(&card.bounds, &style, "title", false);
        let selected_title_rect = text_field_rect(&card.bounds, &style, "title", true);
        let summary_rect = text_field_rect(&card.bounds, &style, "summary", false);
        let detail_rect = text_field_rect(&card.bounds, &style, "detail", false);
        let selected_overlay_rect = overlay_rect_for_text_field(&selected_title_rect, &style, true);
        let overlay = overlay_style(
            &CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 2.0,
            },
            &style,
            "title",
            true,
        );

        assert_eq!(title_rect.x, 124.0);
        assert_eq!(title_rect.y, 132.0);
        assert_eq!(summary_rect.y, 170.0);
        assert!(detail_rect.y > summary_rect.y);
        assert!((selected_title_rect.height - 35.4).abs() < 0.0001);
        assert_eq!(selected_overlay_rect.x, 107.0);
        assert_eq!(selected_overlay_rect.width, 246.0);
        assert_eq!(overlay.font_size, 60.0);
        assert!((overlay.line_height - 70.8).abs() < 0.0001);
        assert_eq!(overlay.padding_x, 32.0);
        assert_eq!(overlay.border_width, 2.0);
        assert_eq!(overlay.border_radius, 14.0);
        assert_eq!(overlay.state, "selected");
        assert_eq!(overlay.max_lines, 1);
        assert_eq!(overlay.text_color, "rgba(23, 32, 38, 0.920)");
        assert_eq!(overlay.caret_color, "rgba(47, 126, 230, 0.920)");
        assert_eq!(
            text_field_at_point(
                &[token.clone()],
                &SceneSelection::Canvas,
                &card,
                &WorldPoint {
                    x: title_rect.x + 2.0,
                    y: title_rect.y + 2.0,
                },
            )
            .as_deref(),
            Some("title"),
        );
        assert_eq!(
            text_field_at_point(
                &[token],
                &SceneSelection::Canvas,
                &card,
                &WorldPoint {
                    x: detail_rect.x + 2.0,
                    y: detail_rect.y + 2.0,
                },
            )
            .as_deref(),
            Some("detail"),
        );
    }

    #[test]
    fn detail_card_text_uses_shaped_text_path_and_hit_rect() {
        let mut card = text_path_card("detail", 80.0, 60.0);
        card.summary = String::new();
        card.detail = "Detail 한글 text rendered by Rust".to_string();
        card.bounds.height = 220.0;
        let scene = SceneSnapshot {
            scene_id: "detail-text".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            groups: Vec::new(),
            cards: vec![card.clone()],
            edges: Vec::new(),
            styles: vec![minimal_style_token("default")],
            selection: SceneSelection::Canvas,
        };
        let mut cache = TextLayoutCache::default();
        let mut text_engine = TextEngine::new().unwrap();

        let (_, stats) = build_card_vertices(&scene, &card, &mut cache, &mut text_engine);
        let style = resolve_shape_style(&scene.styles, "default");
        let detail_rect = text_field_rect(&card.bounds, &style, "detail", false);

        assert!(stats.cjk_glyph_count > 0);
        assert_eq!(
            text_field_at_point(
                &scene.styles,
                &SceneSelection::Canvas,
                &card,
                &WorldPoint {
                    x: detail_rect.x + 4.0,
                    y: detail_rect.y + 4.0,
                },
            )
            .as_deref(),
            Some("detail"),
        );
    }

    fn minimal_style_token(id: &str) -> SceneStyleToken {
        serde_json::from_str(&format!(
            r##"{{
              "id": "{id}",
              "fill": "#ffffff",
              "stroke": "#2f7ee6",
              "text": "#172026",
              "mutedText": "#65717b",
              "accent": "#2f7ee6"
            }}"##
        ))
        .expect("minimal style token should deserialize")
    }

    fn custom_style_token() -> SceneStyleToken {
        serde_json::from_str(
            r##"{
              "id": "custom",
              "fill": "#ffffff",
              "stroke": "#2f7ee6",
              "text": "#172026",
              "mutedText": "#65717b",
              "accent": "#2f7ee6",
              "typography": {
                "cardTitleSize": 24,
                "cardSelectedTitleSize": 30,
                "cardSummarySize": 15
              },
              "spacing": {
                "cardPadding": 24,
                "badgeHeight": 28,
                "cardGap": 16
              },
              "states": {
                "default": { "shadowAlpha": 0.25 },
                "selected": { "shadowAlpha": 0.5, "glowAlpha": 0.33 }
              }
            }"##,
        )
        .expect("custom style token should deserialize")
    }

    fn text_path_card(id: &str, x: f64, y: f64) -> RenderCard {
        RenderCard {
            id: id.to_string(),
            group_id: "group-a".to_string(),
            title: format!("Card {id}"),
            summary: String::new(),
            detail: String::new(),
            status: "draft".to_string(),
            node_type: "task".to_string(),
            bounds: WorldRect {
                x,
                y,
                width: 240.0,
                height: 150.0,
            },
            z_index: 0.0,
            style_key: "default".to_string(),
            accessibility_label: String::new(),
        }
    }
}

#[cfg(feature = "wgpu-probe")]
fn parse_hex_color(value: &str, alpha: f32) -> Option<[f32; 4]> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some([
        red as f32 / 255.0,
        green as f32 / 255.0,
        blue as f32 / 255.0,
        alpha,
    ])
}

#[cfg(feature = "wgpu-probe")]
fn resolve_shape_style(styles: &[SceneStyleToken], style_key: &str) -> ShapeRenderStyle {
    let token = styles
        .iter()
        .find(|token| token.id == style_key)
        .or_else(|| styles.iter().find(|token| token.id == "default"));
    let fill = token_color(
        token.map(|token| token.fill.as_str()),
        0.98,
        [1.0, 1.0, 1.0, 0.98],
    );
    let surface = token_color(
        token.and_then(|token| token.surface.as_deref()),
        0.96,
        [1.0, 1.0, 1.0, 0.96],
    );
    let surface2 = token_color(
        token.and_then(|token| token.surface2.as_deref()),
        0.92,
        [0.97, 0.98, 0.99, 0.92],
    );
    let surface3 = token_color(
        token.and_then(|token| token.surface3.as_deref()),
        0.94,
        [0.99, 0.99, 1.0, 0.94],
    );
    let pastel = token_color(token.and_then(|token| token.pastel.as_deref()), 0.72, fill);
    let stroke = token_color(
        token.map(|token| token.stroke.as_str()),
        0.54,
        [0.18, 0.49, 0.9, 0.54],
    );
    let text = token_color(
        token.map(|token| token.text.as_str()),
        0.92,
        [0.09, 0.13, 0.16, 0.92],
    );
    let muted_text = token_color(
        token.map(|token| token.muted_text.as_str()),
        0.84,
        [0.35, 0.43, 0.50, 0.84],
    );
    let accent = token_color(token.map(|token| token.accent.as_str()), 0.92, stroke);
    let line = token_color(
        token.and_then(|token| token.line.as_deref()),
        0.12,
        [0.16, 0.21, 0.27, 0.12],
    );
    let line_strong = token_color(
        token.and_then(|token| token.line_strong.as_deref()),
        0.42,
        [0.12, 0.18, 0.23, 0.42],
    );
    let focus = token_color(
        token.and_then(|token| token.focus.as_deref()),
        0.82,
        [0.18, 0.49, 0.9, 0.82],
    );
    let radius = token.and_then(|token| token.radius.as_ref());
    let stroke_widths = token.and_then(|token| token.stroke_widths.as_ref());
    let typography = token.and_then(|token| token.typography.as_ref());
    let spacing = token.and_then(|token| token.spacing.as_ref());
    let gradient = token.and_then(|token| token.gradient.as_ref());
    let states = token.and_then(|token| token.states.as_ref());
    let default_state = states.and_then(|states| states.default.as_ref());
    let selected_state = states.and_then(|states| states.selected.as_ref());
    let compact_state = states.and_then(|states| states.compact.as_ref());
    let badge = token.and_then(|token| token.badge.as_ref());
    let edge = token.and_then(|token| token.edge.as_ref());
    let port = token.and_then(|token| token.port.as_ref());

    ShapeRenderStyle {
        fill,
        surface,
        surface2,
        surface3,
        pastel,
        stroke,
        text,
        muted_text,
        accent,
        line,
        line_strong,
        focus,
        radius: ShapeRadius {
            group: metric(radius.and_then(|radius| radius.group), 34.0),
            group_selected: metric(radius.and_then(|radius| radius.group_selected), 34.0),
            card: metric(radius.and_then(|radius| radius.card), 16.0),
            card_selected: metric(radius.and_then(|radius| radius.card_selected), 18.0),
            badge: metric(radius.and_then(|radius| radius.badge), 7.0),
            edge_label: metric(radius.and_then(|radius| radius.edge_label), 9.0),
            port: metric(radius.and_then(|radius| radius.port), 8.0),
            focus_ring: metric(radius.and_then(|radius| radius.focus_ring), 20.0),
        },
        stroke_width: ShapeStrokeWidth {
            group: metric(stroke_widths.and_then(|width| width.group), 2.0),
            group_selected: metric(stroke_widths.and_then(|width| width.group_selected), 2.0),
            card: metric(stroke_widths.and_then(|width| width.card), 1.0),
            card_selected: metric(stroke_widths.and_then(|width| width.card_selected), 1.0),
            inner: metric(stroke_widths.and_then(|width| width.inner), 1.0),
            focus_ring: metric(stroke_widths.and_then(|width| width.focus_ring), 4.0),
            edge: metric(stroke_widths.and_then(|width| width.edge), 3.0),
            edge_compact: metric(stroke_widths.and_then(|width| width.edge_compact), 2.2),
            edge_selected: metric(stroke_widths.and_then(|width| width.edge_selected), 5.0),
            separator: metric(stroke_widths.and_then(|width| width.separator), 1.0),
            port: metric(stroke_widths.and_then(|width| width.port), 2.0),
        },
        typography: ShapeTypography {
            group_title_size: font_metric(typography.and_then(|font| font.group_title_size), 38.0),
            group_summary_size: font_metric(
                typography.and_then(|font| font.group_summary_size),
                18.0,
            ),
            card_title_size: font_metric(typography.and_then(|font| font.card_title_size), 19.0),
            card_selected_title_size: font_metric(
                typography.and_then(|font| font.card_selected_title_size),
                22.0,
            ),
            card_summary_size: font_metric(
                typography.and_then(|font| font.card_summary_size),
                13.0,
            ),
            badge_size: font_metric(typography.and_then(|font| font.badge_size), 10.0),
            edge_label_size: font_metric(typography.and_then(|font| font.edge_label_size), 18.0),
        },
        spacing: ShapeSpacing {
            group_padding_x: metric(spacing.and_then(|spacing| spacing.group_padding_x), 28.0),
            group_padding_y: metric(spacing.and_then(|spacing| spacing.group_padding_y), 24.0),
            card_padding: metric(spacing.and_then(|spacing| spacing.card_padding), 14.0),
            card_gap: metric(spacing.and_then(|spacing| spacing.card_gap), 9.0),
            badge_padding_x: metric(spacing.and_then(|spacing| spacing.badge_padding_x), 7.0),
            badge_height: metric(spacing.and_then(|spacing| spacing.badge_height), 20.0),
            label_padding_x: metric(spacing.and_then(|spacing| spacing.label_padding_x), 8.0),
            edge_label_height: metric(spacing.and_then(|spacing| spacing.edge_label_height), 24.0),
            port_radius: metric(spacing.and_then(|spacing| spacing.port_radius), 7.0),
            separator_inset: metric(spacing.and_then(|spacing| spacing.separator_inset), 18.0),
        },
        shadow: resolve_shadow_layers(
            token
                .map(|token| token.shadow.as_slice())
                .unwrap_or_default(),
            vec![
                ShapeShadowLayer {
                    offset_x: 0.0,
                    offset_y: 18.0,
                    blur: 36.0,
                    spread: 0.0,
                    color: [0.10, 0.14, 0.19, 0.10],
                },
                ShapeShadowLayer {
                    offset_x: 0.0,
                    offset_y: 2.0,
                    blur: 7.0,
                    spread: 0.0,
                    color: [0.10, 0.14, 0.19, 0.06],
                },
            ],
        ),
        selected_shadow: resolve_shadow_layers(
            token
                .map(|token| token.selected_shadow.as_slice())
                .unwrap_or_default(),
            vec![
                ShapeShadowLayer {
                    offset_x: 0.0,
                    offset_y: 30.0,
                    blur: 64.0,
                    spread: 0.0,
                    color: color_with_alpha(accent, 0.14),
                },
                ShapeShadowLayer {
                    offset_x: 0.0,
                    offset_y: 10.0,
                    blur: 24.0,
                    spread: 0.0,
                    color: [0.10, 0.14, 0.19, 0.10],
                },
            ],
        ),
        glow: resolve_shadow_layers(
            token.map(|token| token.glow.as_slice()).unwrap_or_default(),
            vec![ShapeShadowLayer {
                offset_x: 0.0,
                offset_y: 0.0,
                blur: 0.0,
                spread: 4.0,
                color: color_with_alpha(accent, 0.12),
            }],
        ),
        gradient: ShapeGradient {
            surface_top_alpha: opacity(
                gradient.and_then(|gradient| gradient.surface_top_alpha),
                0.98,
            ),
            pastel_bottom_alpha: opacity(
                gradient.and_then(|gradient| gradient.pastel_bottom_alpha),
                0.78,
            ),
            accent_start_alpha: opacity(
                gradient.and_then(|gradient| gradient.accent_start_alpha),
                0.48,
            ),
            accent_end_alpha: opacity(
                gradient.and_then(|gradient| gradient.accent_end_alpha),
                0.22,
            ),
        },
        state: ShapeState {
            default_fill_alpha: opacity(default_state.and_then(|state| state.fill_alpha), 0.96),
            default_stroke_alpha: opacity(default_state.and_then(|state| state.stroke_alpha), 0.16),
            selected_fill_alpha: opacity(selected_state.and_then(|state| state.fill_alpha), 0.98),
            selected_stroke_alpha: opacity(
                selected_state.and_then(|state| state.stroke_alpha),
                0.52,
            ),
            focus_alpha: opacity(selected_state.and_then(|state| state.focus_alpha), 0.12),
            shadow_alpha: opacity(default_state.and_then(|state| state.shadow_alpha), 1.0),
            selected_shadow_alpha: opacity(
                selected_state.and_then(|state| state.shadow_alpha),
                opacity(default_state.and_then(|state| state.shadow_alpha), 1.0) as f64,
            ),
            glow_alpha: opacity(selected_state.and_then(|state| state.glow_alpha), 1.0),
            compact_stroke_alpha: opacity(compact_state.and_then(|state| state.stroke_alpha), 0.26),
        },
        badge: ShapeBadgeStyle {
            fill_alpha: opacity(badge.and_then(|badge| badge.fill_alpha), 0.10),
            stroke_alpha: opacity(badge.and_then(|badge| badge.stroke_alpha), 0.16),
            text_alpha: opacity(badge.and_then(|badge| badge.text_alpha), 0.94),
            min_width: metric(badge.and_then(|badge| badge.min_width), 54.0),
        },
        edge: ShapeEdgeStyle {
            stroke_alpha: opacity(edge.and_then(|edge| edge.stroke_alpha), 0.42),
            selected_stroke_alpha: opacity(edge.and_then(|edge| edge.selected_stroke_alpha), 0.82),
            compact_stroke_alpha: opacity(edge.and_then(|edge| edge.compact_stroke_alpha), 0.26),
            label_fill_alpha: opacity(edge.and_then(|edge| edge.label_fill_alpha), 0.90),
            label_stroke_alpha: opacity(edge.and_then(|edge| edge.label_stroke_alpha), 0.26),
            label_text_alpha: opacity(edge.and_then(|edge| edge.label_text_alpha), 0.72),
        },
        port: ShapePortStyle {
            fill_alpha: opacity(port.and_then(|port| port.fill_alpha), 0.94),
            stroke_alpha: opacity(port.and_then(|port| port.stroke_alpha), 0.46),
            selected_fill_alpha: opacity(port.and_then(|port| port.selected_fill_alpha), 0.18),
            selected_stroke_alpha: opacity(port.and_then(|port| port.selected_stroke_alpha), 0.82),
        },
    }
}

#[cfg(feature = "wgpu-probe")]
fn resolve_shadow_layers(
    layers: &[SceneShadowLayerToken],
    fallback: Vec<ShapeShadowLayer>,
) -> Vec<ShapeShadowLayer> {
    if layers.is_empty() {
        return fallback;
    }
    layers
        .iter()
        .map(|layer| ShapeShadowLayer {
            offset_x: layer.offset_x,
            offset_y: layer.offset_y,
            blur: layer.blur,
            spread: layer.spread,
            color: token_color(
                Some(layer.color.as_str()),
                opacity(Some(layer.alpha), 1.0),
                [0.0, 0.0, 0.0, 0.0],
            ),
        })
        .collect()
}

#[cfg(feature = "wgpu-probe")]
fn token_color(value: Option<&str>, alpha: f32, fallback: [f32; 4]) -> [f32; 4] {
    value
        .and_then(|value| parse_hex_color(value, alpha))
        .unwrap_or(fallback)
}

#[cfg(feature = "wgpu-probe")]
fn metric(value: Option<f64>, fallback: f64) -> f64 {
    value.unwrap_or(fallback).max(0.0)
}

#[cfg(feature = "wgpu-probe")]
fn font_metric(value: Option<f64>, fallback: f64) -> f32 {
    metric(value, fallback) as f32
}

#[cfg(feature = "wgpu-probe")]
fn opacity(value: Option<f64>, fallback: f64) -> f32 {
    value.unwrap_or(fallback).clamp(0.0, 1.0) as f32
}

#[cfg(feature = "wgpu-probe")]
fn color_with_alpha(color: [f32; 4], alpha: f32) -> [f32; 4] {
    [color[0], color[1], color[2], alpha.clamp(0.0, 1.0)]
}

#[cfg(feature = "wgpu-probe")]
fn mix_rgb(a: [f32; 4], b: [f32; 4], b_weight: f32, alpha: f32) -> [f32; 4] {
    let t = b_weight.clamp(0.0, 1.0);
    [
        a[0] * (1.0 - t) + b[0] * t,
        a[1] * (1.0 - t) + b[1] * t,
        a[2] * (1.0 - t) + b[2] * t,
        alpha.clamp(0.0, 1.0),
    ]
}

#[cfg(feature = "wgpu-probe")]
fn mix_color(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] * (1.0 - t) + b[0] * t,
        a[1] * (1.0 - t) + b[1] * t,
        a[2] * (1.0 - t) + b[2] * t,
        a[3] * (1.0 - t) + b[3] * t,
    ]
}

#[cfg(feature = "wgpu-probe")]
fn add_line(
    vertices: &mut Vec<GpuVertex>,
    start: [f32; 2],
    end: [f32; 2],
    thickness: f32,
    color: [f32; 4],
) {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let length = (dx * dx + dy * dy).sqrt().max(0.001);
    let nx = -dy / length * thickness * 0.5;
    let ny = dx / length * thickness * 0.5;
    add_quad(
        vertices,
        [start[0] + nx, start[1] + ny],
        [end[0] + nx, end[1] + ny],
        [end[0] - nx, end[1] - ny],
        [start[0] - nx, start[1] - ny],
        color,
    );
}

#[cfg(feature = "wgpu-probe")]
fn add_cubic_edge(
    vertices: &mut Vec<GpuVertex>,
    route: &CubicRoute,
    thickness: f32,
    color: [f32; 4],
) {
    let mut previous = route.start;
    for step in 1..=EDGE_CURVE_SEGMENTS {
        let t = step as f64 / EDGE_CURVE_SEGMENTS as f64;
        let current = cubic_point(route, t);
        add_line(
            vertices,
            [previous.x as f32, previous.y as f32],
            [current.x as f32, current.y as f32],
            thickness,
            color,
        );
        previous = current;
    }
}

#[cfg(feature = "wgpu-probe")]
fn add_arrowhead(vertices: &mut Vec<GpuVertex>, start: [f32; 2], end: [f32; 2], color: [f32; 4]) {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let length = (dx * dx + dy * dy).sqrt().max(0.001);
    let ux = dx / length;
    let uy = dy / length;
    let px = -uy;
    let py = ux;
    let arrow_length = 22.0;
    let arrow_width = 13.0;
    let base = [end[0] - ux * arrow_length, end[1] - uy * arrow_length];
    vertices.push(GpuVertex {
        position: end,
        uv: SOLID_UV[0],
        color,
    });
    vertices.push(GpuVertex {
        position: [base[0] + px * arrow_width, base[1] + py * arrow_width],
        uv: SOLID_UV[0],
        color,
    });
    vertices.push(GpuVertex {
        position: [base[0] - px * arrow_width, base[1] - py * arrow_width],
        uv: SOLID_UV[0],
        color,
    });
}

#[cfg(feature = "wgpu-probe")]
fn add_quad(
    vertices: &mut Vec<GpuVertex>,
    a: [f32; 2],
    b: [f32; 2],
    c: [f32; 2],
    d: [f32; 2],
    color: [f32; 4],
) {
    add_quad_uv(vertices, a, b, c, d, SOLID_UV, color);
}

#[cfg(feature = "wgpu-probe")]
fn add_quad_uv(
    vertices: &mut Vec<GpuVertex>,
    a: [f32; 2],
    b: [f32; 2],
    c: [f32; 2],
    d: [f32; 2],
    uv: [[f32; 2]; 4],
    color: [f32; 4],
) {
    vertices.push(GpuVertex {
        position: a,
        uv: uv[0],
        color,
    });
    vertices.push(GpuVertex {
        position: b,
        uv: uv[1],
        color,
    });
    vertices.push(GpuVertex {
        position: c,
        uv: uv[2],
        color,
    });
    vertices.push(GpuVertex {
        position: a,
        uv: uv[0],
        color,
    });
    vertices.push(GpuVertex {
        position: c,
        uv: uv[2],
        color,
    });
    vertices.push(GpuVertex {
        position: d,
        uv: uv[3],
        color,
    });
}

#[cfg(feature = "wgpu-probe")]
fn badge_rect(
    card: &WorldRect,
    style: &ShapeRenderStyle,
    label: &str,
    text_engine: &TextEngine,
) -> WorldRect {
    let width = (text_engine.measure_text_width(label, style.typography.badge_size) as f64
        + style.spacing.badge_padding_x * 2.0)
        .max(style.badge.min_width);
    WorldRect {
        x: card.x + style.spacing.card_padding,
        y: card.y + style.spacing.card_padding,
        width,
        height: style.spacing.badge_height,
    }
}

#[cfg(feature = "wgpu-probe")]
fn node_type_label(node_type: &str) -> String {
    match node_type {
        "decision_point" => "Decision".to_string(),
        "subdecision" => "Subdecision".to_string(),
        "tradeoff" => "Tradeoff".to_string(),
        "blocker" => "Blocker".to_string(),
        "proposition" => "Proposition".to_string(),
        "evidence" => "Evidence".to_string(),
        "artifact" => "Artifact".to_string(),
        "task" => "Task".to_string(),
        "option" => "Option".to_string(),
        "" => "Node".to_string(),
        other => other.replace('_', " "),
    }
}

#[cfg(feature = "wgpu-probe")]
fn point_in_rect(point: &WorldPoint, rect: &WorldRect) -> bool {
    point.x >= rect.x
        && point.x <= rect.x + rect.width
        && point.y >= rect.y
        && point.y <= rect.y + rect.height
}

#[cfg(feature = "wgpu-probe")]
fn rects_intersect(a: &WorldRect, b: &WorldRect) -> bool {
    a.x <= b.x + b.width && a.x + a.width >= b.x && a.y <= b.y + b.height && a.y + a.height >= b.y
}

#[cfg(feature = "wgpu-probe")]
fn screen_to_world(point: WorldPoint, camera: &CameraState) -> WorldPoint {
    let zoom = camera.zoom.max(0.025);
    WorldPoint {
        x: (point.x - camera.x) / zoom,
        y: (point.y - camera.y) / zoom,
    }
}

#[cfg(feature = "wgpu-probe")]
fn world_rect_to_screen_rect(rect: &WorldRect, camera: &CameraState) -> WorldRect {
    WorldRect {
        x: rect.x * camera.zoom + camera.x,
        y: rect.y * camera.zoom + camera.y,
        width: rect.width * camera.zoom,
        height: rect.height * camera.zoom,
    }
}

#[cfg(feature = "wgpu-probe")]
fn hit_scene_at_screen(
    scene: &SceneSnapshot,
    camera: &CameraState,
    screen: WorldPoint,
) -> Option<CoreHitResult> {
    let world = screen_to_world(screen, camera);

    let mut cards: Vec<&RenderCard> = scene.cards.iter().collect();
    cards.sort_by(|a, b| b.z_index.total_cmp(&a.z_index));
    for card in cards {
        if !point_in_rect(&world, &card.bounds) {
            continue;
        }
        if let Some(port) = port_at_point(card, &world) {
            return Some(CoreHitResult {
                id: card.id.clone(),
                kind: "port".to_string(),
                group_id: Some(card.group_id.clone()),
                field: None,
                port: Some(port),
                world_x: world.x,
                world_y: world.y,
                screen_x: screen.x,
                screen_y: screen.y,
            });
        }
        let field = text_field_at_point(&scene.styles, &scene.selection, card, &world);
        return Some(CoreHitResult {
            id: card.id.clone(),
            kind: if field.is_some() { "text" } else { "card" }.to_string(),
            group_id: Some(card.group_id.clone()),
            field,
            port: None,
            world_x: world.x,
            world_y: world.y,
            screen_x: screen.x,
            screen_y: screen.y,
        });
    }

    let threshold = 18.0 / camera.zoom.max(0.025);
    for edge in scene.edges.iter().rev() {
        let Some(source) = scene.cards.iter().find(|card| card.id == edge.source) else {
            continue;
        };
        let Some(target) = scene.cards.iter().find(|card| card.id == edge.target) else {
            continue;
        };
        let route = edge_route(source, target);
        if distance_to_cubic(&world, &route) <= threshold {
            return Some(CoreHitResult {
                id: edge.id.clone(),
                kind: "edge".to_string(),
                group_id: Some(edge.group_id.clone()),
                field: None,
                port: None,
                world_x: world.x,
                world_y: world.y,
                screen_x: screen.x,
                screen_y: screen.y,
            });
        }
    }

    let mut groups: Vec<&RenderGroup> = scene.groups.iter().collect();
    groups.sort_by(|a, b| b.z_index.total_cmp(&a.z_index));
    for group in groups {
        if point_in_rect(&world, &group.bounds) {
            return Some(CoreHitResult {
                id: group.id.clone(),
                kind: "group".to_string(),
                group_id: Some(group.id.clone()),
                field: None,
                port: None,
                world_x: world.x,
                world_y: world.y,
                screen_x: screen.x,
                screen_y: screen.y,
            });
        }
    }

    None
}

#[cfg(feature = "wgpu-probe")]
fn selection_from_hit(hit: Option<&CoreHitResult>) -> SceneSelection {
    let Some(hit) = hit else {
        return SceneSelection::Canvas;
    };
    if hit.kind == "group" {
        return SceneSelection::Group { id: hit.id.clone() };
    }
    if hit.kind == "edge" {
        return SceneSelection::Edge { id: hit.id.clone() };
    }
    SceneSelection::Node { id: hit.id.clone() }
}

#[cfg(feature = "wgpu-probe")]
fn selection_world_rect(scene: &SceneSnapshot, selection: &SceneSelection) -> Option<WorldRect> {
    match selection {
        SceneSelection::Canvas => None,
        SceneSelection::Group { id } => scene
            .groups
            .iter()
            .find(|group| &group.id == id)
            .map(|group| group.bounds.clone()),
        SceneSelection::Node { id } => scene
            .cards
            .iter()
            .find(|card| &card.id == id)
            .map(|card| card.bounds.clone()),
        SceneSelection::Edge { id } => {
            scene
                .edges
                .iter()
                .find(|edge| &edge.id == id)
                .and_then(|edge| {
                    let source = scene.cards.iter().find(|card| card.id == edge.source)?;
                    let target = scene.cards.iter().find(|card| card.id == edge.target)?;
                    Some(edge_visible_bounds(source, target))
                })
        }
    }
}

#[cfg(feature = "wgpu-probe")]
fn zoom_camera_at_screen(camera: &CameraState, screen: WorldPoint, delta_y: f64) -> CameraState {
    let world = screen_to_world(screen, camera);
    let zoom = clamp_camera_zoom(camera.zoom * (-delta_y * 0.0012).exp());
    CameraState {
        zoom,
        x: screen.x - world.x * zoom,
        y: screen.y - world.y * zoom,
    }
}

#[cfg(feature = "wgpu-probe")]
fn fit_camera_to_scene(
    scene: &SceneSnapshot,
    viewport_width: f64,
    viewport_height: f64,
) -> CameraState {
    let bounds = scene_world_bounds(scene);
    let padding = 96.0;
    let usable_width = (viewport_width - padding * 2.0).max(120.0);
    let usable_height = (viewport_height - padding * 2.0).max(120.0);
    let zoom = clamp(bounds_zoom(usable_width, usable_height, &bounds), 0.04, 1.6);
    CameraState {
        zoom,
        x: viewport_width / 2.0 - (bounds.x + bounds.width / 2.0) * zoom,
        y: viewport_height / 2.0 - (bounds.y + bounds.height / 2.0) * zoom,
    }
}

#[cfg(feature = "wgpu-probe")]
fn focus_camera_to_bounds(
    bounds: &WorldRect,
    screen: Option<WorldPoint>,
    zoom: Option<f64>,
    padding: Option<WorldPoint>,
    min_zoom: Option<f64>,
    max_zoom: Option<f64>,
    viewport_width: f64,
    viewport_height: f64,
) -> CameraState {
    let target = screen.unwrap_or(WorldPoint {
        x: viewport_width / 2.0,
        y: viewport_height / 2.0,
    });
    let padding = padding.unwrap_or(WorldPoint { x: 96.0, y: 96.0 });
    let usable_width = (viewport_width - padding.x * 2.0).max(120.0);
    let usable_height = (viewport_height - padding.y * 2.0).max(120.0);
    let min_zoom = min_zoom.unwrap_or(0.04);
    let max_zoom = max_zoom.unwrap_or(1.6);
    let zoom = zoom.unwrap_or_else(|| bounds_zoom(usable_width, usable_height, bounds));
    let zoom = clamp(zoom, min_zoom, max_zoom);
    CameraState {
        zoom,
        x: target.x - (bounds.x + bounds.width / 2.0) * zoom,
        y: target.y - (bounds.y + bounds.height / 2.0) * zoom,
    }
}

#[cfg(feature = "wgpu-probe")]
fn scene_world_bounds(scene: &SceneSnapshot) -> WorldRect {
    let mut rects: Vec<&WorldRect> = Vec::new();
    rects.extend(scene.groups.iter().map(|group| &group.bounds));
    rects.extend(scene.cards.iter().map(|card| &card.bounds));
    union_rect_refs(&rects).unwrap_or(WorldRect {
        x: 0.0,
        y: 0.0,
        width: 1200.0,
        height: 800.0,
    })
}

#[cfg(feature = "wgpu-probe")]
fn union_rect_refs(rects: &[&WorldRect]) -> Option<WorldRect> {
    let first = rects.first()?;
    let mut min_x = first.x;
    let mut min_y = first.y;
    let mut max_x = first.x + first.width;
    let mut max_y = first.y + first.height;
    for rect in rects.iter().skip(1) {
        min_x = min_x.min(rect.x);
        min_y = min_y.min(rect.y);
        max_x = max_x.max(rect.x + rect.width);
        max_y = max_y.max(rect.y + rect.height);
    }
    Some(WorldRect {
        x: min_x,
        y: min_y,
        width: max_x - min_x,
        height: max_y - min_y,
    })
}

#[cfg(feature = "wgpu-probe")]
fn bounds_zoom(usable_width: f64, usable_height: f64, bounds: &WorldRect) -> f64 {
    (usable_width / bounds.width).min(usable_height / bounds.height)
}

#[cfg(feature = "wgpu-probe")]
fn clamp_camera(camera: CameraState) -> CameraState {
    CameraState {
        x: camera.x,
        y: camera.y,
        zoom: clamp_camera_zoom(camera.zoom),
    }
}

#[cfg(feature = "wgpu-probe")]
fn clamp_camera_zoom(zoom: f64) -> f64 {
    clamp(zoom, 0.025, 2.8)
}

#[cfg(feature = "wgpu-probe")]
fn clamp(value: f64, min: f64, max: f64) -> f64 {
    value.min(max).max(min)
}

#[cfg(feature = "wgpu-probe")]
fn overlay_style(
    camera: &CameraState,
    style: &ShapeRenderStyle,
    field: &str,
    selected: bool,
) -> CoreOverlayStyle {
    let font_size = match field {
        "title" if selected => style.typography.card_selected_title_size,
        "title" => style.typography.card_title_size,
        _ => style.typography.card_summary_size,
    } as f64;
    let line_height = match field {
        "title" => font_size * 1.18,
        _ => font_size * 1.46,
    };
    let zoom = camera.zoom;
    let (padding_x, padding_y) = overlay_padding_world(style);
    let border_width = overlay_border_width_world(style, selected);
    let text_color = match field {
        "title" => color_with_alpha(style.text, 0.92),
        _ => color_with_alpha(style.muted_text, 0.84),
    };
    let border_color = color_with_alpha(
        style.accent,
        if selected {
            style.state.selected_stroke_alpha
        } else {
            style.state.default_stroke_alpha
        },
    );
    let focus_ring_color = color_with_alpha(style.focus, style.state.focus_alpha);
    CoreOverlayStyle {
        font_family:
            "\"Noto Sans KR\", Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", sans-serif"
                .to_string(),
        font_size: font_size * zoom,
        font_weight: 400,
        line_height: line_height * zoom,
        letter_spacing: 0.0,
        padding_x: padding_x * zoom,
        padding_y: padding_y * zoom,
        text_color: css_color(text_color),
        background_color: css_color(color_with_alpha(
            style.surface,
            if selected {
                style.state.selected_fill_alpha
            } else {
                style.state.default_fill_alpha
            },
        )),
        border_color: css_color(border_color),
        border_width: border_width * zoom,
        border_radius: style.radius.badge * zoom,
        focus_ring_color: css_color(focus_ring_color),
        focus_ring_width: style.stroke_width.focus_ring * zoom,
        box_shadow: css_shadow_layers(
            if selected { style.glow.as_slice() } else { &[] },
            zoom,
            if selected { style.state.glow_alpha } else { 0.0 },
        ),
        caret_color: css_color(color_with_alpha(style.accent, 0.92)),
        accent_color: css_color(color_with_alpha(style.accent, 0.92)),
        selection_background_color: css_color(color_with_alpha(style.accent, 0.20)),
        max_lines: overlay_max_lines(field),
        overflow_x: "hidden".to_string(),
        overflow_y: if field == "title" { "hidden" } else { "auto" }.to_string(),
        state: if selected { "selected" } else { "default" }.to_string(),
    }
}

#[cfg(feature = "wgpu-probe")]
fn overlay_rect_for_text_field(
    text_rect: &WorldRect,
    style: &ShapeRenderStyle,
    selected: bool,
) -> WorldRect {
    let (padding_x, padding_y) = overlay_padding_world(style);
    let border_width = overlay_border_width_world(style, selected);
    let x_inset = padding_x + border_width;
    let y_inset = padding_y + border_width;
    WorldRect {
        x: text_rect.x - x_inset,
        y: text_rect.y - y_inset,
        width: text_rect.width + x_inset * 2.0,
        height: text_rect.height + y_inset * 2.0,
    }
}

#[cfg(feature = "wgpu-probe")]
fn overlay_padding_world(style: &ShapeRenderStyle) -> (f64, f64) {
    (style.spacing.card_gap, style.spacing.card_gap * 0.67)
}

#[cfg(feature = "wgpu-probe")]
fn overlay_border_width_world(style: &ShapeRenderStyle, selected: bool) -> f64 {
    if selected {
        style.stroke_width.card_selected
    } else {
        style.stroke_width.card
    }
}

#[cfg(feature = "wgpu-probe")]
fn overlay_max_lines(field: &str) -> u8 {
    match field {
        "title" => 1,
        "summary" => 3,
        _ => 6,
    }
}

#[cfg(feature = "wgpu-probe")]
fn css_color(color: [f32; 4]) -> String {
    let red = (color[0].clamp(0.0, 1.0) * 255.0).round() as u8;
    let green = (color[1].clamp(0.0, 1.0) * 255.0).round() as u8;
    let blue = (color[2].clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "rgba({red}, {green}, {blue}, {:.3})",
        color[3].clamp(0.0, 1.0)
    )
}

#[cfg(feature = "wgpu-probe")]
fn css_shadow_layers(layers: &[ShapeShadowLayer], zoom: f64, alpha_scale: f32) -> String {
    if layers.is_empty() || alpha_scale <= 0.0 {
        return "none".to_string();
    }
    let shadows = layers
        .iter()
        .filter_map(|layer| {
            let alpha = layer.color[3] * alpha_scale;
            if alpha <= 0.0 {
                return None;
            }
            Some(format!(
                "{:.2}px {:.2}px {:.2}px {:.2}px {}",
                layer.offset_x * zoom,
                layer.offset_y * zoom,
                layer.blur * zoom,
                layer.spread * zoom,
                css_color(color_with_alpha(layer.color, alpha)),
            ))
        })
        .collect::<Vec<_>>();
    if shadows.is_empty() {
        "none".to_string()
    } else {
        shadows.join(", ")
    }
}

#[cfg(feature = "wgpu-probe")]
fn drag_pointer_id(drag: Option<&InputDragState>) -> Option<i32> {
    match drag {
        Some(InputDragState::Pan { pointer_id, .. })
        | Some(InputDragState::Group { pointer_id, .. })
        | Some(InputDragState::Card { pointer_id, .. })
        | Some(InputDragState::Edge { pointer_id, .. }) => Some(*pointer_id),
        None => None,
    }
}

#[cfg(feature = "wgpu-probe")]
fn text_field_at_point(
    styles: &[SceneStyleToken],
    selection: &SceneSelection,
    card: &RenderCard,
    point: &WorldPoint,
) -> Option<String> {
    let style = resolve_shape_style(styles, &card.style_key);
    let selected = selection_is_node(selection, &card.id);
    if point_in_rect(
        point,
        &text_field_rect(&card.bounds, &style, "title", selected),
    ) {
        return Some("title".to_string());
    }
    if point_in_rect(
        point,
        &text_field_rect(&card.bounds, &style, "summary", selected),
    ) {
        return Some("summary".to_string());
    }
    if point_in_rect(
        point,
        &text_field_rect(&card.bounds, &style, "detail", selected),
    ) {
        return Some("detail".to_string());
    }
    None
}

#[cfg(feature = "wgpu-probe")]
fn text_field_rect(
    card: &WorldRect,
    style: &ShapeRenderStyle,
    field: &str,
    selected: bool,
) -> WorldRect {
    let layout = card_text_layout(card, style, selected);
    if field == "title" {
        return WorldRect {
            x: layout.content_x,
            y: layout.title_y,
            width: layout.content_width,
            height: layout.title_line_height,
        };
    }
    if field == "summary" {
        return WorldRect {
            x: layout.content_x,
            y: layout.summary_y,
            width: layout.content_width,
            height: layout.summary_line_height * layout.summary_max_lines as f64,
        };
    }
    WorldRect {
        x: layout.content_x,
        y: layout.detail_y,
        width: layout.content_width,
        height: (card.y + card.height - style.spacing.card_padding - layout.detail_y)
            .max(layout.detail_line_height),
    }
}

#[cfg(feature = "wgpu-probe")]
fn card_text_layout(card: &WorldRect, style: &ShapeRenderStyle, selected: bool) -> CardTextLayout {
    let title_font_size = if selected {
        style.typography.card_selected_title_size
    } else {
        style.typography.card_title_size
    };
    let title_y = card.y
        + style.spacing.card_padding
        + style.spacing.badge_height
        + style.spacing.card_gap
        + 14.0;
    let summary_y = card.y
        + style.spacing.card_padding
        + style.spacing.badge_height
        + style.spacing.card_gap
        + 52.0;
    let summary_font_size = style.typography.card_summary_size;
    let summary_line_height = summary_font_size as f64 * 1.46;
    let detail_font_size = summary_font_size;
    let detail_line_height = detail_font_size as f64 * 1.46;
    let content_bottom = card.y + card.height - style.spacing.card_padding;
    let detail_y = (content_bottom - detail_line_height).max(summary_y + summary_line_height);
    let summary_available =
        (detail_y - summary_y - style.spacing.card_gap).max(summary_line_height);
    let summary_max_lines =
        ((summary_available / summary_line_height).floor() as usize).clamp(1, 3);
    let detail_available = (content_bottom - detail_y).max(0.0);
    let detail_max_lines =
        ((detail_available / detail_line_height + 0.001).floor() as usize).min(6);
    CardTextLayout {
        content_x: card.x + style.spacing.card_padding,
        content_width: (card.width - style.spacing.card_padding * 2.0).max(0.0),
        title_y,
        title_font_size,
        title_line_height: title_font_size as f64 * 1.18,
        summary_y,
        summary_font_size,
        summary_line_height,
        summary_max_lines,
        detail_y,
        detail_font_size,
        detail_line_height,
        detail_max_lines,
    }
}

#[cfg(feature = "wgpu-probe")]
fn port_at_point(card: &RenderCard, point: &WorldPoint) -> Option<String> {
    let source = WorldPoint {
        x: card.bounds.x + card.bounds.width,
        y: card.bounds.y + card.bounds.height / 2.0,
    };
    let target = WorldPoint {
        x: card.bounds.x,
        y: card.bounds.y + card.bounds.height / 2.0,
    };
    if distance(point, &source) <= 16.0 {
        return Some("source".to_string());
    }
    if distance(point, &target) <= 16.0 {
        return Some("target".to_string());
    }
    None
}

#[cfg(feature = "wgpu-probe")]
fn edge_route(source: &RenderCard, target: &RenderCard) -> CubicRoute {
    let start = WorldPoint {
        x: source.bounds.x + source.bounds.width,
        y: source.bounds.y + source.bounds.height / 2.0,
    };
    let end = WorldPoint {
        x: target.bounds.x,
        y: target.bounds.y + target.bounds.height / 2.0,
    };
    let curve = 80.0_f64.max((end.x - start.x).abs() * 0.34);
    CubicRoute {
        cp1: WorldPoint {
            x: start.x + curve,
            y: start.y,
        },
        cp2: WorldPoint {
            x: end.x - curve,
            y: end.y,
        },
        start,
        end,
    }
}

#[cfg(feature = "wgpu-probe")]
fn edge_visible_bounds(source: &RenderCard, target: &RenderCard) -> WorldRect {
    let route = edge_route(source, target);
    let min_x = route
        .start
        .x
        .min(route.cp1.x)
        .min(route.cp2.x)
        .min(route.end.x);
    let min_y = route
        .start
        .y
        .min(route.cp1.y)
        .min(route.cp2.y)
        .min(route.end.y);
    let max_x = route
        .start
        .x
        .max(route.cp1.x)
        .max(route.cp2.x)
        .max(route.end.x);
    let max_y = route
        .start
        .y
        .max(route.cp1.y)
        .max(route.cp2.y)
        .max(route.end.y);
    let padding = 96.0;
    WorldRect {
        x: min_x - padding,
        y: min_y - padding,
        width: (max_x - min_x) + padding * 2.0,
        height: (max_y - min_y) + padding * 2.0,
    }
}

#[cfg(feature = "wgpu-probe")]
fn distance(a: &WorldPoint, b: &WorldPoint) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

#[cfg(feature = "wgpu-probe")]
fn distance_to_segment(point: &WorldPoint, start: &WorldPoint, end: &WorldPoint) -> f64 {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared == 0.0 {
        return distance(point, start);
    }
    let t =
        (((point.x - start.x) * dx + (point.y - start.y) * dy) / length_squared).clamp(0.0, 1.0);
    distance(
        point,
        &WorldPoint {
            x: start.x + t * dx,
            y: start.y + t * dy,
        },
    )
}

#[cfg(feature = "wgpu-probe")]
fn distance_to_cubic(point: &WorldPoint, route: &CubicRoute) -> f64 {
    let mut best = f64::INFINITY;
    let mut previous = route.start;
    for step in 1..=EDGE_CURVE_SEGMENTS {
        let t = step as f64 / EDGE_CURVE_SEGMENTS as f64;
        let current = cubic_point(route, t);
        best = best.min(distance_to_segment(point, &previous, &current));
        previous = current;
    }
    best
}

#[cfg(feature = "wgpu-probe")]
fn cubic_point(route: &CubicRoute, t: f64) -> WorldPoint {
    let mt = 1.0 - t;
    WorldPoint {
        x: mt.powi(3) * route.start.x
            + 3.0 * mt.powi(2) * t * route.cp1.x
            + 3.0 * mt * t.powi(2) * route.cp2.x
            + t.powi(3) * route.end.x,
        y: mt.powi(3) * route.start.y
            + 3.0 * mt.powi(2) * t * route.cp1.y
            + 3.0 * mt * t.powi(2) * route.cp2.y
            + t.powi(3) * route.end.y,
    }
}

#[cfg(feature = "wgpu-probe")]
fn create_text_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas_pixels: &[u8],
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shape.ai WebGPU shaped glyph atlas"),
        size: wgpu::Extent3d {
            width: TEXT_ATLAS_WIDTH,
            height: TEXT_ATLAS_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        atlas_pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(TEXT_ATLAS_WIDTH * 4),
            rows_per_image: Some(TEXT_ATLAS_HEIGHT),
        },
        wgpu::Extent3d {
            width: TEXT_ATLAS_WIDTH,
            height: TEXT_ATLAS_HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("shape.ai WebGPU shaped glyph sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    (texture, view, sampler)
}

#[cfg(feature = "wgpu-probe")]
const SOLID_UV: [[f32; 2]; 4] = TEXT_ATLAS_SOLID_UV;
#[cfg(feature = "wgpu-probe")]
const EDGE_CURVE_SEGMENTS: usize = 18;
#[cfg(feature = "wgpu-probe")]
const ROUNDED_CORNER_SEGMENTS: usize = 4;
#[cfg(feature = "wgpu-probe")]
const SOFT_GRADIENT_BANDS: usize = 5;
#[cfg(feature = "wgpu-probe")]
const GROUP_VERTEX_SLOT: usize = 1536;
#[cfg(feature = "wgpu-probe")]
const EDGE_VERTEX_SLOT: usize = 768;
#[cfg(feature = "wgpu-probe")]
const CARD_VERTEX_SLOT: usize = 1536;
#[cfg(feature = "wgpu-probe")]
const VIEWPORT_CULL_PADDING: f64 = 400.0;

#[cfg(feature = "wgpu-probe")]
fn webgpu_vertex_buffer_usage() -> wgpu::BufferUsages {
    wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC
}

#[cfg(feature = "wgpu-probe")]
const SHAPE_WEBGPU_SHADER: &str = r#"
struct View {
  camera: vec4<f32>,
  viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> view: View;

@group(0) @binding(1)
var text_atlas: texture_2d<f32>;

@group(0) @binding(2)
var text_sampler: sampler;

struct VertexIn {
  @location(0) position: vec2<f32>,
  @location(1) uv: vec2<f32>,
  @location(2) color: vec4<f32>,
};

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) color: vec4<f32>,
  @location(1) uv: vec2<f32>,
};

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
  let screen = input.position * view.camera.z + view.camera.xy;
  let clip = vec2<f32>(
    (screen.x / view.viewport.x) * 2.0 - 1.0,
    1.0 - (screen.y / view.viewport.y) * 2.0
  );
  var out: VertexOut;
  out.position = vec4<f32>(clip, 0.0, 1.0);
  out.color = input.color;
  out.uv = input.uv;
  return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
  let atlas = textureSample(text_atlas, text_sampler, input.uv);
  return vec4<f32>(input.color.rgb, input.color.a * atlas.a);
}
"#;
