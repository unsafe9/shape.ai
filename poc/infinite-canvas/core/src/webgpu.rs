use std::collections::HashMap;

use crate::model::{
    CameraState, CubicRoute, RenderCard, RenderEdge, RenderGroup, RenderScenePatch, SceneSelection,
    SceneSnapshot, SceneStyleToken, WorldPoint, WorldRect,
};
use crate::serde_wasm;
use crate::stats::{CoreHitResult, WebGpuFrameStats, WebGpuProbeReport};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

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
#[derive(Clone, Copy)]
struct TextBuildStats {
    glyph_count: usize,
    fallback_glyph_count: usize,
    cjk_glyph_count: usize,
}

#[cfg(feature = "wgpu-probe")]
impl TextBuildStats {
    fn add(&mut self, other: TextBuildStats) {
        self.glyph_count += other.glyph_count;
        self.fallback_glyph_count += other.fallback_glyph_count;
        self.cjk_glyph_count += other.cjk_glyph_count;
    }
}

#[cfg(feature = "wgpu-probe")]
impl Default for TextBuildStats {
    fn default() -> Self {
        TextBuildStats {
            glyph_count: 0,
            fallback_glyph_count: 0,
            cjk_glyph_count: 0,
        }
    }
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
    vertex_count: usize,
    text_glyph_count: usize,
    fallback_text_glyph_count: usize,
    cjk_text_glyph_count: usize,
    patch_update_count: usize,
    dirty_range_write_count: usize,
    full_buffer_rebuild_count: usize,
    edge_capacity_grow_count: usize,
    edge_compaction_count: usize,
    card_capacity_grow_count: usize,
    card_compaction_count: usize,
    group_capacity_grow_count: usize,
    group_compaction_count: usize,
}

#[cfg(feature = "wgpu-probe")]
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
        let (text_texture, text_view, text_sampler) = create_text_atlas(&device, &queue);
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
            vertex_count: 0,
            text_glyph_count: 0,
            fallback_text_glyph_count: 0,
            cjk_text_glyph_count: 0,
            patch_update_count: 0,
            dirty_range_write_count: 0,
            full_buffer_rebuild_count: 0,
            edge_capacity_grow_count: 0,
            edge_compaction_count: 0,
            card_capacity_grow_count: 0,
            card_compaction_count: 0,
            group_capacity_grow_count: 0,
            group_compaction_count: 0,
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
        self.rebuild_vertex_buffer();
        self.write_uniform();
        Ok(())
    }

    #[wasm_bindgen(js_name = applyPatch)]
    pub fn apply_patch(&mut self, patch_json: &str) -> Result<(), JsValue> {
        let patch = serde_json::from_str::<RenderScenePatch>(patch_json)
            .map_err(|error| JsValue::from_str(&format!("Invalid render patch: {error}")))?;
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

    #[wasm_bindgen(js_name = setCamera)]
    pub fn set_camera(&mut self, x: f64, y: f64, zoom: f64) {
        self.camera = CameraState { x, y, zoom };
        self.write_uniform();
    }

    #[wasm_bindgen(js_name = renderFrame)]
    pub fn render_frame(&mut self) -> Result<JsValue, JsValue> {
        self.write_uniform();
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
            if self.vertex_count > 0 {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.draw(0..self.vertex_count as u32, 0..1);
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
            vertex_count: self.vertex_count,
            text_glyph_count: self.text_glyph_count,
            fallback_text_glyph_count: self.fallback_text_glyph_count,
            cjk_text_glyph_count: self.cjk_text_glyph_count,
            style_token_count,
            patch_update_count: self.patch_update_count,
            dirty_range_write_count: self.dirty_range_write_count,
            full_buffer_rebuild_count: self.full_buffer_rebuild_count,
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

    #[wasm_bindgen(js_name = hitTest)]
    pub fn hit_test(&self, screen_x: f64, screen_y: f64) -> Result<JsValue, JsValue> {
        let Some(scene) = &self.scene else {
            return Ok(JsValue::NULL);
        };
        let world_x = (screen_x - self.camera.x) / self.camera.zoom;
        let world_y = (screen_y - self.camera.y) / self.camera.zoom;
        let world = WorldPoint {
            x: world_x,
            y: world_y,
        };

        let mut cards: Vec<&RenderCard> = scene.cards.iter().collect();
        cards.sort_by(|a, b| b.z_index.total_cmp(&a.z_index));
        for card in cards {
            if !point_in_rect(&world, &card.bounds) {
                continue;
            }
            if let Some(port) = port_at_point(card, &world) {
                return serde_wasm(CoreHitResult {
                    id: card.id.clone(),
                    kind: "port".to_string(),
                    group_id: Some(card.group_id.clone()),
                    field: None,
                    port: Some(port),
                    world_x,
                    world_y,
                });
            }
            let field = text_field_at_point(card, &world);
            return serde_wasm(CoreHitResult {
                id: card.id.clone(),
                kind: if field.is_some() { "text" } else { "card" }.to_string(),
                group_id: Some(card.group_id.clone()),
                field,
                port: None,
                world_x,
                world_y,
            });
        }

        let threshold = 18.0 / self.camera.zoom.max(0.025);
        for edge in scene.edges.iter().rev() {
            let Some(source) = scene.cards.iter().find(|card| card.id == edge.source) else {
                continue;
            };
            let Some(target) = scene.cards.iter().find(|card| card.id == edge.target) else {
                continue;
            };
            let route = edge_route(source, target);
            if distance_to_cubic(&world, &route) <= threshold {
                return serde_wasm(CoreHitResult {
                    id: edge.id.clone(),
                    kind: "edge".to_string(),
                    group_id: Some(edge.group_id.clone()),
                    field: None,
                    port: None,
                    world_x,
                    world_y,
                });
            }
        }

        let mut groups: Vec<&RenderGroup> = scene.groups.iter().collect();
        groups.sort_by(|a, b| b.z_index.total_cmp(&a.z_index));
        for group in groups {
            if point_in_rect(&world, &group.bounds) {
                return serde_wasm(CoreHitResult {
                    id: group.id.clone(),
                    kind: "group".to_string(),
                    group_id: Some(group.id.clone()),
                    field: None,
                    port: None,
                    world_x,
                    world_y,
                });
            }
        }

        Ok(JsValue::NULL)
    }
}

#[cfg(feature = "wgpu-probe")]
impl ShapeWebGpuRenderer {
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
        if let Some(scene) = &self.scene {
            let mut groups: Vec<&RenderGroup> = scene.groups.iter().collect();
            groups.sort_by(|a, b| a.z_index.total_cmp(&b.z_index));
            for group in groups {
                let offset = vertices.len();
                let (group_vertices, group_text_stats) = build_group_vertices(scene, group);
                text_stats.add(group_text_stats);
                vertices.extend(fit_vertices_to_slot(group_vertices, GROUP_VERTEX_SLOT));
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
                let (edge_vertices, edge_text_stats) = build_edge_vertices(scene, edge);
                text_stats.add(edge_text_stats);
                vertices.extend(fit_vertices_to_slot(edge_vertices, EDGE_VERTEX_SLOT));
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
                let (card_vertices, card_text_stats) = build_card_vertices(scene, card);
                text_stats.add(card_text_stats);
                vertices.extend(fit_vertices_to_slot(card_vertices, CARD_VERTEX_SLOT));
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
        self.full_buffer_rebuild_count += 1;
        if !vertices.is_empty() {
            self.queue
                .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        }
    }

    fn write_dirty_card(&mut self, id: &str) -> bool {
        let Some(scene) = &self.scene else {
            return false;
        };
        let Some(slot) = self.vertex_ranges.cards.get(id).copied() else {
            return false;
        };
        let Some(card) = scene.cards.iter().find(|card| card.id == id) else {
            return false;
        };
        let (vertices, text_stats) = build_card_vertices(scene, card);
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
            slot.text_stats = text_stats;
        }
        self.dirty_range_write_count += 1;
        true
    }

    fn write_dirty_group(&mut self, id: &str) -> bool {
        let Some(scene) = &self.scene else {
            return false;
        };
        let Some(slot) = self.vertex_ranges.groups.get(id).copied() else {
            return false;
        };
        let Some(group) = scene.groups.iter().find(|group| group.id == id) else {
            return false;
        };
        let (vertices, text_stats) = build_group_vertices(scene, group);
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
            slot.text_stats = text_stats;
        }
        self.dirty_range_write_count += 1;
        true
    }

    fn write_dirty_edge(&mut self, id: &str) -> bool {
        let Some(scene) = &self.scene else {
            return false;
        };
        let Some(slot) = self.vertex_ranges.edges.get(id).copied() else {
            return false;
        };
        let Some(edge) = scene.edges.iter().find(|edge| edge.id == id) else {
            return false;
        };
        let (vertices, text_stats) = build_edge_vertices(scene, edge);
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
        self.dirty_range_write_count += 1;
        true
    }

    fn write_slot_vertices(&self, slot: VertexSlot, vertices: Vec<GpuVertex>) {
        let vertices = fit_vertices_to_slot(vertices, slot.capacity);
        let byte_offset = (slot.offset * std::mem::size_of::<GpuVertex>()) as u64;
        self.queue.write_buffer(
            &self.vertex_buffer,
            byte_offset,
            bytemuck::cast_slice(&vertices),
        );
    }
}

#[cfg(feature = "wgpu-probe")]
fn build_group_vertices(
    scene: &SceneSnapshot,
    group: &RenderGroup,
) -> (Vec<GpuVertex>, TextBuildStats) {
    let mut vertices = Vec::new();
    let fill = style_color(&scene.styles, &group.style_key, StyleColor::Fill, 0.62);
    let stroke = style_color(&scene.styles, &group.style_key, StyleColor::Stroke, 0.38);
    let accent = style_color(&scene.styles, &group.style_key, StyleColor::Accent, 0.86);
    let text = style_color(&scene.styles, &group.style_key, StyleColor::Text, 0.72);
    add_rect(&mut vertices, &group.bounds, fill);
    add_rect_border(&mut vertices, &group.bounds, 10.0, stroke);
    if selection_is_group(&scene.selection, &group.id) {
        add_rect_border(&mut vertices, &group.bounds, 18.0, accent);
    }
    let text_stats = add_text_line(
        &mut vertices,
        &group.title,
        group.bounds.x as f32 + 28.0,
        group.bounds.y as f32 + 24.0,
        group.bounds.width as f32 - 56.0,
        18.0,
        text,
    );
    (vertices, text_stats)
}

#[cfg(feature = "wgpu-probe")]
fn build_edge_vertices(
    scene: &SceneSnapshot,
    edge: &RenderEdge,
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
    let stroke = style_color(
        &scene.styles,
        &edge.style_key,
        if selected {
            StyleColor::Accent
        } else {
            StyleColor::Stroke
        },
        if selected { 0.92 } else { 0.48 },
    );
    let arrow = style_color(
        &scene.styles,
        &edge.style_key,
        if selected {
            StyleColor::Accent
        } else {
            StyleColor::Stroke
        },
        if selected { 0.98 } else { 0.56 },
    );
    add_cubic_edge(
        &mut vertices,
        &route,
        if selected { 7.0 } else { 4.0 },
        stroke,
    );
    add_arrowhead(
        &mut vertices,
        [route.cp2.x as f32, route.cp2.y as f32],
        [route.end.x as f32, route.end.y as f32],
        arrow,
    );
    let mut text_stats = TextBuildStats::default();
    if !edge.label.trim().is_empty() {
        let fill = style_color(&scene.styles, &edge.style_key, StyleColor::Fill, 0.88);
        let label_stroke = style_color(&scene.styles, &edge.style_key, StyleColor::Stroke, 0.28);
        let label_text = style_color(&scene.styles, &edge.style_key, StyleColor::Text, 0.72);
        let label_font_size = 11.0;
        let label_max_width = 180.0;
        let label_width = text_line_width(&edge.label, label_font_size)
            .min(label_max_width)
            .max(24.0);
        let label_x = ((route.start.x + route.end.x) * 0.5) as f32 - label_width * 0.5 - 6.0;
        let label_y = ((route.start.y + route.end.y) * 0.5) as f32 - 22.0;
        let label_rect = WorldRect {
            x: label_x as f64,
            y: label_y as f64,
            width: (label_width + 12.0) as f64,
            height: 17.0,
        };
        add_rect(&mut vertices, &label_rect, fill);
        add_rect_border(&mut vertices, &label_rect, 1.0, label_stroke);
        text_stats.add(add_text_line(
            &mut vertices,
            &edge.label,
            label_x + 6.0,
            label_y + 4.0,
            label_width,
            label_font_size,
            label_text,
        ));
    }
    (vertices, text_stats)
}

#[cfg(feature = "wgpu-probe")]
fn build_card_vertices(
    scene: &SceneSnapshot,
    card: &RenderCard,
) -> (Vec<GpuVertex>, TextBuildStats) {
    let mut vertices = Vec::new();
    let mut text_stats = TextBuildStats::default();
    let fill = style_color(&scene.styles, &card.style_key, StyleColor::Fill, 0.98);
    let stroke = style_color(&scene.styles, &card.style_key, StyleColor::Stroke, 0.54);
    let accent = style_color(&scene.styles, &card.style_key, StyleColor::Accent, 0.92);
    let title = style_color(&scene.styles, &card.style_key, StyleColor::Text, 0.92);
    let muted = style_color(&scene.styles, &card.style_key, StyleColor::MutedText, 0.84);
    add_rect(&mut vertices, &card.bounds, fill);
    add_rect(&mut vertices, &badge_rect(&card.bounds), accent);
    add_rect_border(&mut vertices, &card.bounds, 4.0, stroke);
    if selection_is_node(&scene.selection, &card.id) {
        add_rect_border(&mut vertices, &card.bounds, 9.0, accent);
    }
    text_stats.add(add_text_line(
        &mut vertices,
        &card.title,
        card.bounds.x as f32 + 18.0,
        card.bounds.y as f32 + 50.0,
        card.bounds.width as f32 - 36.0,
        17.0,
        title,
    ));
    text_stats.add(add_wrapped_text(
        &mut vertices,
        &card.summary,
        card.bounds.x as f32 + 18.0,
        card.bounds.y as f32 + 92.0,
        card.bounds.width as f32 - 36.0,
        11.0,
        15.0,
        3,
        muted,
    ));
    (vertices, text_stats)
}

#[cfg(feature = "wgpu-probe")]
fn fit_vertices_to_slot(mut vertices: Vec<GpuVertex>, capacity: usize) -> Vec<GpuVertex> {
    let safe_capacity = capacity - (capacity % 3);
    if vertices.len() > safe_capacity {
        vertices.truncate(safe_capacity);
    }
    vertices.resize(capacity, transparent_vertex());
    vertices
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
fn add_text_line(
    vertices: &mut Vec<GpuVertex>,
    value: &str,
    x: f32,
    y: f32,
    max_width: f32,
    font_size: f32,
    color: [f32; 4],
) -> TextBuildStats {
    let scale = font_size / GLYPH_HEIGHT as f32;
    let glyph_width = GLYPH_WIDTH as f32 * scale;
    let glyph_height = GLYPH_HEIGHT as f32 * scale;
    let mut cursor_x = x;
    let right_limit = x + max_width.max(0.0);
    let mut text_stats = TextBuildStats::default();

    for grapheme in value.graphemes(true) {
        if grapheme == "\n" {
            break;
        }
        let advance = grapheme_advance(grapheme, font_size);
        if cursor_x + advance > right_limit {
            break;
        }
        if is_whitespace_grapheme(grapheme) {
            cursor_x += advance;
            continue;
        }

        let render_char = glyph_render_char(grapheme);
        let Some(uv) = glyph_uv(render_char) else {
            cursor_x += advance;
            continue;
        };
        let left = cursor_x;
        let top = y;
        let right = left + glyph_width;
        let bottom = top + glyph_height;
        add_quad_uv(
            vertices,
            [left, top],
            [right, top],
            [right, bottom],
            [left, bottom],
            uv,
            color,
        );
        text_stats.glyph_count += 1;
        if grapheme_uses_fallback_glyph(grapheme) {
            text_stats.fallback_glyph_count += 1;
        }
        if is_cjk_grapheme(grapheme) {
            text_stats.cjk_glyph_count += 1;
        }
        cursor_x += advance;
    }
    text_stats
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
) -> TextBuildStats {
    let lines = wrap_text_lines(value, max_width, font_size, max_lines);
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
        ));
    }
    text_stats
}

#[cfg(feature = "wgpu-probe")]
fn wrap_text_lines(value: &str, max_width: f32, font_size: f32, max_lines: usize) -> Vec<String> {
    let max_lines = max_lines.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0.0;

    for segment in value.split_word_bounds() {
        append_segment_with_newlines(
            &mut lines,
            &mut line,
            &mut line_width,
            segment,
            max_width,
            font_size,
            max_lines,
        );
        if lines.len() >= max_lines {
            return lines;
        }
    }

    push_current_line(&mut lines, &mut line, &mut line_width, max_lines);
    lines
}

#[cfg(feature = "wgpu-probe")]
fn append_segment_with_newlines(
    lines: &mut Vec<String>,
    line: &mut String,
    line_width: &mut f32,
    segment: &str,
    max_width: f32,
    font_size: f32,
    max_lines: usize,
) {
    let mut start = 0;
    for (index, ch) in segment.char_indices() {
        if ch == '\n' {
            append_wrapped_segment(
                lines,
                line,
                line_width,
                &segment[start..index],
                max_width,
                font_size,
                max_lines,
            );
            push_current_line(lines, line, line_width, max_lines);
            if lines.len() >= max_lines {
                return;
            }
            start = index + ch.len_utf8();
        }
    }
    append_wrapped_segment(
        lines,
        line,
        line_width,
        &segment[start..],
        max_width,
        font_size,
        max_lines,
    );
}

#[cfg(feature = "wgpu-probe")]
fn append_wrapped_segment(
    lines: &mut Vec<String>,
    line: &mut String,
    line_width: &mut f32,
    segment: &str,
    max_width: f32,
    font_size: f32,
    max_lines: usize,
) {
    if segment.is_empty() || lines.len() >= max_lines {
        return;
    }
    if segment.chars().all(|ch| ch.is_whitespace()) {
        append_space(lines, line, line_width, max_width, font_size, max_lines);
        return;
    }

    let segment_width = text_line_width(segment, font_size);
    if segment_width <= max_width {
        if !line.is_empty() && *line_width + segment_width > max_width {
            push_current_line(lines, line, line_width, max_lines);
        }
        if lines.len() < max_lines {
            line.push_str(segment);
            *line_width += segment_width;
        }
        return;
    }

    for grapheme in segment.graphemes(true) {
        append_grapheme(
            lines, line, line_width, grapheme, max_width, font_size, max_lines,
        );
        if lines.len() >= max_lines {
            return;
        }
    }
}

#[cfg(feature = "wgpu-probe")]
fn append_grapheme(
    lines: &mut Vec<String>,
    line: &mut String,
    line_width: &mut f32,
    grapheme: &str,
    max_width: f32,
    font_size: f32,
    max_lines: usize,
) {
    if grapheme == "\n" {
        push_current_line(lines, line, line_width, max_lines);
        return;
    }
    if is_whitespace_grapheme(grapheme) {
        append_space(lines, line, line_width, max_width, font_size, max_lines);
        return;
    }

    let width = grapheme_advance(grapheme, font_size);
    if !line.is_empty() && *line_width + width > max_width {
        push_current_line(lines, line, line_width, max_lines);
    }
    if lines.len() < max_lines {
        line.push_str(grapheme);
        *line_width += width;
    }
}

#[cfg(feature = "wgpu-probe")]
fn append_space(
    lines: &mut Vec<String>,
    line: &mut String,
    line_width: &mut f32,
    max_width: f32,
    font_size: f32,
    max_lines: usize,
) {
    if line.is_empty() || lines.len() >= max_lines {
        return;
    }
    let width = grapheme_advance(" ", font_size);
    if *line_width + width > max_width {
        push_current_line(lines, line, line_width, max_lines);
        return;
    }
    line.push(' ');
    *line_width += width;
}

#[cfg(feature = "wgpu-probe")]
fn push_current_line(
    lines: &mut Vec<String>,
    line: &mut String,
    line_width: &mut f32,
    max_lines: usize,
) {
    if lines.len() < max_lines {
        let trimmed = line.trim_end();
        if !trimmed.is_empty() {
            lines.push(trimmed.to_string());
        }
    }
    line.clear();
    *line_width = 0.0;
}

#[cfg(feature = "wgpu-probe")]
fn text_line_width(value: &str, font_size: f32) -> f32 {
    UnicodeWidthStr::width(value) as f32 * text_char_advance(font_size)
}

#[cfg(feature = "wgpu-probe")]
fn grapheme_advance(grapheme: &str, font_size: f32) -> f32 {
    grapheme_width_units(grapheme) as f32 * text_char_advance(font_size)
}

#[cfg(feature = "wgpu-probe")]
fn grapheme_width_units(grapheme: &str) -> usize {
    if grapheme == "\t" {
        return 2;
    }
    UnicodeWidthStr::width(grapheme).max(1)
}

#[cfg(feature = "wgpu-probe")]
fn is_whitespace_grapheme(grapheme: &str) -> bool {
    grapheme.chars().all(|ch| ch.is_whitespace())
}

#[cfg(feature = "wgpu-probe")]
fn glyph_render_char(grapheme: &str) -> char {
    let Some(ch) = grapheme.chars().next() else {
        return '?';
    };
    if ch.is_ascii() && (FIRST_GLYPH..=LAST_GLYPH).contains(&(ch as u32)) {
        ch
    } else {
        '?'
    }
}

#[cfg(feature = "wgpu-probe")]
fn grapheme_uses_fallback_glyph(grapheme: &str) -> bool {
    grapheme
        .chars()
        .any(|ch| !ch.is_ascii() || !(FIRST_GLYPH..=LAST_GLYPH).contains(&(ch as u32)))
}

#[cfg(feature = "wgpu-probe")]
fn is_cjk_grapheme(grapheme: &str) -> bool {
    grapheme.chars().any(is_cjk_char)
}

#[cfg(feature = "wgpu-probe")]
fn is_cjk_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{1100}'..='\u{11ff}'
            | '\u{2e80}'..='\u{2eff}'
            | '\u{3000}'..='\u{303f}'
            | '\u{3040}'..='\u{30ff}'
            | '\u{3130}'..='\u{318f}'
            | '\u{31f0}'..='\u{31ff}'
            | '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{a960}'..='\u{a97f}'
            | '\u{ac00}'..='\u{d7af}'
            | '\u{d7b0}'..='\u{d7ff}'
            | '\u{f900}'..='\u{faff}'
            | '\u{ff00}'..='\u{ffef}'
    )
}

#[cfg(feature = "wgpu-probe")]
fn text_char_advance(font_size: f32) -> f32 {
    let scale = font_size / GLYPH_HEIGHT as f32;
    (GLYPH_WIDTH + 1) as f32 * scale
}

#[cfg(all(test, feature = "wgpu-probe"))]
mod tests {
    use super::*;

    #[test]
    fn text_width_uses_unicode_display_width() {
        let font_size = 14.0;

        assert!(text_line_width("한글", font_size) > text_line_width("AB", font_size));
    }

    #[test]
    fn wrap_text_lines_breaks_no_space_cjk_by_grapheme_width() {
        let font_size = 14.0;
        let max_width = text_char_advance(font_size) * 4.0;

        let lines = wrap_text_lines("한글테스트문장", max_width, font_size, 3);

        assert!(lines.len() > 1);
        assert!(text_line_width(&lines[0], font_size) <= max_width);
    }

    #[test]
    fn add_text_line_tracks_fallback_and_cjk_glyphs() {
        let mut vertices = Vec::new();

        let stats = add_text_line(
            &mut vertices,
            "A한B",
            0.0,
            0.0,
            200.0,
            14.0,
            [1.0, 1.0, 1.0, 1.0],
        );

        assert_eq!(stats.glyph_count, 3);
        assert_eq!(stats.fallback_glyph_count, 1);
        assert_eq!(stats.cjk_glyph_count, 1);
        assert_eq!(vertices.len(), 18);
    }
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
enum StyleColor {
    Fill,
    Stroke,
    Text,
    MutedText,
    Accent,
}

#[cfg(feature = "wgpu-probe")]
fn style_color(
    styles: &[SceneStyleToken],
    style_key: &str,
    channel: StyleColor,
    alpha: f32,
) -> [f32; 4] {
    let token = styles
        .iter()
        .find(|token| token.id == style_key)
        .or_else(|| styles.iter().find(|token| token.id == "default"));
    let parsed = token.and_then(|token| {
        let value = match channel {
            StyleColor::Fill => &token.fill,
            StyleColor::Stroke => &token.stroke,
            StyleColor::Text => &token.text,
            StyleColor::MutedText => &token.muted_text,
            StyleColor::Accent => &token.accent,
        };
        parse_hex_color(value, alpha)
    });
    parsed.unwrap_or_else(|| fallback_style_color(channel, alpha))
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
fn fallback_style_color(channel: StyleColor, alpha: f32) -> [f32; 4] {
    let [red, green, blue] = match channel {
        StyleColor::Fill => [1.0, 1.0, 1.0],
        StyleColor::Stroke => [0.18, 0.49, 0.9],
        StyleColor::Text => [0.09, 0.13, 0.16],
        StyleColor::MutedText => [0.35, 0.43, 0.50],
        StyleColor::Accent => [0.18, 0.49, 0.9],
    };
    [red, green, blue, alpha]
}

#[cfg(feature = "wgpu-probe")]
fn add_rect_border(
    vertices: &mut Vec<GpuVertex>,
    rect: &WorldRect,
    thickness: f64,
    color: [f32; 4],
) {
    let t = thickness.max(1.0);
    add_rect(
        vertices,
        &WorldRect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: t,
        },
        color,
    );
    add_rect(
        vertices,
        &WorldRect {
            x: rect.x,
            y: rect.y + rect.height - t,
            width: rect.width,
            height: t,
        },
        color,
    );
    add_rect(
        vertices,
        &WorldRect {
            x: rect.x,
            y: rect.y,
            width: t,
            height: rect.height,
        },
        color,
    );
    add_rect(
        vertices,
        &WorldRect {
            x: rect.x + rect.width - t,
            y: rect.y,
            width: t,
            height: rect.height,
        },
        color,
    );
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
fn badge_rect(card: &WorldRect) -> WorldRect {
    WorldRect {
        x: card.x + 18.0,
        y: card.y + 16.0,
        width: 68.0,
        height: 22.0,
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
fn text_field_at_point(card: &RenderCard, point: &WorldPoint) -> Option<String> {
    if point_in_rect(point, &text_field_rect(&card.bounds, "title")) {
        return Some("title".to_string());
    }
    if point_in_rect(point, &text_field_rect(&card.bounds, "summary")) {
        return Some("summary".to_string());
    }
    None
}

#[cfg(feature = "wgpu-probe")]
fn text_field_rect(card: &WorldRect, field: &str) -> WorldRect {
    if field == "title" {
        return WorldRect {
            x: card.x + 16.0,
            y: card.y + 44.0,
            width: card.width - 32.0,
            height: 48.0,
        };
    }
    WorldRect {
        x: card.x + 16.0,
        y: card.y + 96.0,
        width: card.width - 32.0,
        height: 58.0,
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
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    let mut pixels = vec![0_u8; (ATLAS_WIDTH * ATLAS_HEIGHT * 4) as usize];
    write_atlas_pixel(&mut pixels, 0, 0, 255);
    for code in FIRST_GLYPH..=LAST_GLYPH {
        let rows = glyph_rows(char::from_u32(code).unwrap_or('?'));
        let index = code - FIRST_GLYPH;
        let origin_x = (index % ATLAS_COLS) * GLYPH_CELL_WIDTH;
        let origin_y = (index / ATLAS_COLS) * GLYPH_CELL_HEIGHT;
        for (row_index, row_bits) in rows.iter().enumerate() {
            for col in 0..GLYPH_WIDTH {
                let bit = (row_bits >> (GLYPH_WIDTH - 1 - col)) & 1;
                if bit == 1 {
                    write_atlas_pixel(
                        &mut pixels,
                        origin_x + col,
                        origin_y + row_index as u32,
                        255,
                    );
                }
            }
        }
    }

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shape.ai WebGPU bitmap text atlas"),
        size: wgpu::Extent3d {
            width: ATLAS_WIDTH,
            height: ATLAS_HEIGHT,
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
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(ATLAS_WIDTH * 4),
            rows_per_image: Some(ATLAS_HEIGHT),
        },
        wgpu::Extent3d {
            width: ATLAS_WIDTH,
            height: ATLAS_HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("shape.ai WebGPU bitmap text sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    (texture, view, sampler)
}

#[cfg(feature = "wgpu-probe")]
fn write_atlas_pixel(pixels: &mut [u8], x: u32, y: u32, alpha: u8) {
    let index = ((y * ATLAS_WIDTH + x) * 4) as usize;
    pixels[index] = 255;
    pixels[index + 1] = 255;
    pixels[index + 2] = 255;
    pixels[index + 3] = alpha;
}

#[cfg(feature = "wgpu-probe")]
fn glyph_uv(ch: char) -> Option<[[f32; 2]; 4]> {
    let normalized = if ch.is_ascii_lowercase() {
        ch.to_ascii_uppercase()
    } else if ch.is_ascii() {
        ch
    } else {
        '?'
    };
    let code = normalized as u32;
    if !(FIRST_GLYPH..=LAST_GLYPH).contains(&code) {
        return glyph_uv('?');
    }
    if normalized == ' ' {
        return None;
    }
    let index = code - FIRST_GLYPH;
    let x = (index % ATLAS_COLS) * GLYPH_CELL_WIDTH;
    let y = (index / ATLAS_COLS) * GLYPH_CELL_HEIGHT;
    let left = x as f32 / ATLAS_WIDTH as f32;
    let right = (x + GLYPH_WIDTH) as f32 / ATLAS_WIDTH as f32;
    let top = y as f32 / ATLAS_HEIGHT as f32;
    let bottom = (y + GLYPH_HEIGHT) as f32 / ATLAS_HEIGHT as f32;
    Some([[left, top], [right, top], [right, bottom], [left, bottom]])
}

#[cfg(feature = "wgpu-probe")]
fn glyph_rows(ch: char) -> [u8; GLYPH_HEIGHT as usize] {
    match ch.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        '6' => [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        '-' => [
            0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000,
        ],
        '_' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b11111,
        ],
        '.' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100,
        ],
        ':' => [
            0b00000, 0b01100, 0b01100, 0b00000, 0b01100, 0b01100, 0b00000,
        ],
        '/' => [
            0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000,
        ],
        '?' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b00000, 0b00100,
        ],
        _ => [0b00000; GLYPH_HEIGHT as usize],
    }
}

#[cfg(feature = "wgpu-probe")]
const FIRST_GLYPH: u32 = 32;
#[cfg(feature = "wgpu-probe")]
const LAST_GLYPH: u32 = 126;
#[cfg(feature = "wgpu-probe")]
const ATLAS_COLS: u32 = 16;
#[cfg(feature = "wgpu-probe")]
const GLYPH_WIDTH: u32 = 5;
#[cfg(feature = "wgpu-probe")]
const GLYPH_HEIGHT: u32 = 7;
#[cfg(feature = "wgpu-probe")]
const GLYPH_CELL_WIDTH: u32 = 6;
#[cfg(feature = "wgpu-probe")]
const GLYPH_CELL_HEIGHT: u32 = 8;
#[cfg(feature = "wgpu-probe")]
const ATLAS_WIDTH: u32 = ATLAS_COLS * GLYPH_CELL_WIDTH;
#[cfg(feature = "wgpu-probe")]
const ATLAS_HEIGHT: u32 = 6 * GLYPH_CELL_HEIGHT;
#[cfg(feature = "wgpu-probe")]
const SOLID_UV: [[f32; 2]; 4] = [
    [0.0052083335, 0.010416667],
    [0.0052083335, 0.010416667],
    [0.0052083335, 0.010416667],
    [0.0052083335, 0.010416667],
];
#[cfg(feature = "wgpu-probe")]
const EDGE_CURVE_SEGMENTS: usize = 18;
#[cfg(feature = "wgpu-probe")]
const GROUP_VERTEX_SLOT: usize = 768;
#[cfg(feature = "wgpu-probe")]
const EDGE_VERTEX_SLOT: usize = 384;
#[cfg(feature = "wgpu-probe")]
const CARD_VERTEX_SLOT: usize = 768;

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
