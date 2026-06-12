//! Web-only WebGPU surface: the `web_sys` + wgpu device/surface/config lifecycle,
//! `target_arch = "wasm32"` gated since the renderer is browser-only.
//! `create_text_atlas` builds for the host as dead code so the pure layer keeps
//! compiling on the test gate.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code, unused_imports))]

use std::collections::HashMap;

use shape_renderer_core::model::{ActiveTool, CameraState};
use crate::serde_wasm;
use shape_renderer_core::stats::WebGpuProbeReport;
use shape_renderer_core::text::{TextEngine, TextLayoutCache, TEXT_ATLAS_HEIGHT, TEXT_ATLAS_WIDTH};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use super::*;

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
        let overlay_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU overlay vertices"),
            size: (MARQUEE_OVERLAY_VERTEX_CAPACITY * std::mem::size_of::<GpuVertex>()) as u64,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });
        let handle_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU selection-handle vertices"),
            size: (HANDLE_OVERLAY_VERTEX_CAPACITY * std::mem::size_of::<GpuVertex>()) as u64,
            usage: webgpu_vertex_buffer_usage(),
            mapped_at_creation: false,
        });
        let multi_select_overlay_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai WebGPU multi-select overlay vertices"),
            size: (MULTI_SELECT_OVERLAY_VERTEX_CAPACITY * std::mem::size_of::<GpuVertex>()) as u64,
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

        // Offscreen drop-shadow blur targets at the surface size; recreated in
        // `resize`. Isolated so a fault degrades to "no shadow", never a blank canvas.
        let shadow_blur = crate::shadow_blur::ShadowBlur::new(
            &device,
            &queue,
            config.format,
            config.width,
            config.height,
            device_pixel_ratio.max(1.0) as f32,
        );

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
            overlay_vertex_buffer,
            handle_vertex_buffer,
            multi_select_overlay_vertex_buffer,
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
            active_tool: ActiveTool::default(),
            multi_select: Vec::new(),
            last_hit: None,
            last_lod_tiers: HashMap::new(),
            object_pipeline: None,
            object_renderer: None,
            object_scene: None,
            object_regions: Vec::new(),
            object_patch_count: 0,
            object_rebuild_count: 0,
            preview_deformed: std::collections::HashSet::new(),
            endpoint_preview: None,
            object_bindings: shape_scene_core::object::move_together::BindingGraph::default(),
            object_theme: shape_renderer_core::object_theme::Theme::light(),
            shadow_blur,
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
        // The shadow targets are surface-sized, so rebuild them to the new resolution
        // only when the size changed.
        if !self.shadow_blur.matches(self.config.width, self.config.height) {
            self.shadow_blur = crate::shadow_blur::ShadowBlur::new(
                &self.device,
                &self.queue,
                self.config.format,
                self.config.width,
                self.config.height,
                self.device_pixel_ratio as f32,
            );
        }
        self.write_uniform();
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
