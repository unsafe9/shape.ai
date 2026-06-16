//! Object GPU render pipeline (wgpu half): builds `ObjectPipeline` and owns the
//! buffer uploader + render-pass recorder `ObjectRenderer`, consuming the
//! device-independent CPU geometry from [`shape_renderer_core::object_pipeline`].
//! That crate's CPU vertex/instance structs are re-exported below so the sibling
//! `webgpu` modules resolve them through `crate::object_pipeline::*`.

#[cfg(feature = "wgpu-probe")]
use shape_renderer_core::model::CameraState;
#[cfg(feature = "wgpu-probe")]
use shape_renderer_core::object_theme::{resolve_token_f32, Theme};
#[cfg(feature = "wgpu-probe")]
use shape_renderer_core::plan::{
    build_frame_plan_with_text, diff_plans, FramePlan, PlanDiff, PlanPatch, StyleSlot,
};
#[cfg(feature = "wgpu-probe")]
use shape_renderer_core::render_object::RenderObjectScene;
#[cfg(feature = "wgpu-probe")]
use shape_renderer_core::text::TextEngine;
#[cfg(feature = "wgpu-probe")]
use shape_renderer_core::text_layout::{MsdfAtlasPlan, MsdfGlyphEntry};
#[cfg(feature = "wgpu-probe")]
use crate::shaders::{MSDF_TEXT_WGSL, OBJECT_FILL_WGSL, OBJECT_SHADOW_WGSL, OBJECT_STROKE_WGSL};

pub use shape_renderer_core::object_pipeline::*;

/// Fill + stroke render pipelines for the object path, plus the shared camera
/// bind group layout. Built once from a device; draw buffers live in
/// [`ObjectRenderer`].
#[cfg(feature = "wgpu-probe")]
pub struct ObjectPipeline {
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
    pub stroke_bind_group_layout: wgpu::BindGroupLayout,
    pub text_bind_group_layout: wgpu::BindGroupLayout,
    pub fill_pipeline: wgpu::RenderPipeline,
    pub shadow_pipeline: wgpu::RenderPipeline,
    pub stroke_pipeline: wgpu::RenderPipeline,
    pub text_pipeline: wgpu::RenderPipeline,
}

#[cfg(feature = "wgpu-probe")]
impl ObjectPipeline {
    /// Build the object fill and stroke pipelines for the given surface `format`.
    pub fn new(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let fill_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shape.ai object fill shader"),
            source: wgpu::ShaderSource::Wgsl(OBJECT_FILL_WGSL.into()),
        });
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shape.ai object shadow shader"),
            source: wgpu::ShaderSource::Wgsl(OBJECT_SHADOW_WGSL.into()),
        });
        let stroke_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shape.ai object stroke shader"),
            source: wgpu::ShaderSource::Wgsl(OBJECT_STROKE_WGSL.into()),
        });
        let text_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shape.ai object text shader"),
            source: wgpu::ShaderSource::Wgsl(MSDF_TEXT_WGSL.into()),
        });

        // group(0) binding(0): the affine camera uniform, shared by both shaders.
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shape.ai object camera bind group layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        // Stroke also needs binding(1): the dash params uniform (FRAGMENT).
        let stroke_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shape.ai object stroke bind group layout"),
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
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        // Text group(0), matching `msdf_text.wgsl`: b0 view uniform (VERTEX), b1
        // MSDF atlas texture, b2 sampler, b3 text params uniform (b1-b3 FRAGMENT).
        let text_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shape.ai object text bind group layout"),
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
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
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

        let color_targets = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        // ---- Fill pipeline ------------------------------------------------
        let fill_vertex_attrs = [
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: std::mem::size_of::<[f32; 2]>() as u64,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32,
            },
        ];
        let fill_instance_attrs = fill_instance_attributes();
        let fill_buffers = [
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<FillVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &fill_vertex_attrs,
            },
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<FillInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &fill_instance_attrs,
            },
        ];
        let fill_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shape.ai object fill pipeline layout"),
            bind_group_layouts: &[Some(&camera_bind_group_layout)],
            immediate_size: 0,
        });
        let fill_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shape.ai object fill pipeline"),
            layout: Some(&fill_layout),
            vertex: wgpu::VertexState {
                module: &fill_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &fill_buffers,
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fill_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &color_targets,
            }),
            multiview_mask: None,
            cache: None,
        });

        // ---- Shadow pipeline ----------------------------------------------
        // slot0: ShadowVertex (position @0, feather @1); slot1: instance-step
        // matrix columns @2..4 + shadow color @5. Matches `object_shadow.wgsl`.
        let shadow_vertex_attrs = [
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: std::mem::size_of::<[f32; 2]>() as u64,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32,
            },
        ];
        let shadow_instance_attrs = fill_instance_attributes();
        let shadow_buffers = [
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<ShadowVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &shadow_vertex_attrs,
            },
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<ShadowInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &shadow_instance_attrs,
            },
        ];
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shape.ai object shadow pipeline layout"),
            bind_group_layouts: &[Some(&camera_bind_group_layout)],
            immediate_size: 0,
        });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shape.ai object shadow pipeline"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &shadow_buffers,
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shadow_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &color_targets,
            }),
            multiview_mask: None,
            cache: None,
        });

        // ---- Stroke pipeline ----------------------------------------------
        let stroke_vertex_attrs = [
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
                format: wgpu::VertexFormat::Float32,
            },
            wgpu::VertexAttribute {
                offset: (std::mem::size_of::<[f32; 2]>() * 2 + std::mem::size_of::<f32>()) as u64,
                shader_location: 3,
                format: wgpu::VertexFormat::Float32,
            },
            wgpu::VertexAttribute {
                offset: (std::mem::size_of::<[f32; 2]>() * 2 + std::mem::size_of::<f32>() * 2)
                    as u64,
                shader_location: 4,
                format: wgpu::VertexFormat::Float32,
            },
        ];
        let stroke_instance_attrs = stroke_instance_attributes();
        let stroke_buffers = [
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<StrokeVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &stroke_vertex_attrs,
            },
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<StrokeInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &stroke_instance_attrs,
            },
        ];
        let stroke_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shape.ai object stroke pipeline layout"),
            bind_group_layouts: &[Some(&stroke_bind_group_layout)],
            immediate_size: 0,
        });
        let stroke_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shape.ai object stroke pipeline"),
            layout: Some(&stroke_layout),
            vertex: wgpu::VertexState {
                module: &stroke_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &stroke_buffers,
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &stroke_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &color_targets,
            }),
            multiview_mask: None,
            cache: None,
        });

        // ---- Text pipeline ------------------------------------------------
        // slot0: TextVertex (position @0, uv @1, color @2); slot1: instance-step
        // matrix columns @3..5. Matches `msdf_text.wgsl`.
        let text_vertex_attrs = [
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
        let text_instance_attrs = text_instance_attributes();
        let text_buffers = [
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<TextVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &text_vertex_attrs,
            },
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<TextInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &text_instance_attrs,
            },
        ];
        let text_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shape.ai object text pipeline layout"),
            bind_group_layouts: &[Some(&text_bind_group_layout)],
            immediate_size: 0,
        });
        let text_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shape.ai object text pipeline"),
            layout: Some(&text_layout),
            vertex: wgpu::VertexState {
                module: &text_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &text_buffers,
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &text_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &color_targets,
            }),
            multiview_mask: None,
            cache: None,
        });

        ObjectPipeline {
            camera_bind_group_layout,
            stroke_bind_group_layout,
            text_bind_group_layout,
            fill_pipeline,
            shadow_pipeline,
            stroke_pipeline,
            text_pipeline,
        }
    }
}

/// Instance-step attributes for fill: `m0`/`m1`/`m2` columns then inline fill
/// color, packed to match [`FillInstance`].
#[cfg(feature = "wgpu-probe")]
fn fill_instance_attributes() -> [wgpu::VertexAttribute; 4] {
    let vec3 = std::mem::size_of::<[f32; 3]>() as u64;
    [
        wgpu::VertexAttribute {
            offset: 0,
            shader_location: 2,
            format: wgpu::VertexFormat::Float32x3,
        },
        wgpu::VertexAttribute {
            offset: vec3,
            shader_location: 3,
            format: wgpu::VertexFormat::Float32x3,
        },
        wgpu::VertexAttribute {
            offset: vec3 * 2,
            shader_location: 4,
            format: wgpu::VertexFormat::Float32x3,
        },
        wgpu::VertexAttribute {
            offset: vec3 * 3,
            shader_location: 5,
            format: wgpu::VertexFormat::Float32x4,
        },
    ]
}

/// Instance-step attributes for stroke: `m0`/`m1`/`m2` at locations 5..7, stroke
/// color at 8, packed to match [`StrokeInstance`].
#[cfg(feature = "wgpu-probe")]
fn stroke_instance_attributes() -> [wgpu::VertexAttribute; 4] {
    let vec3 = std::mem::size_of::<[f32; 3]>() as u64;
    [
        wgpu::VertexAttribute {
            offset: 0,
            shader_location: 5,
            format: wgpu::VertexFormat::Float32x3,
        },
        wgpu::VertexAttribute {
            offset: vec3,
            shader_location: 6,
            format: wgpu::VertexFormat::Float32x3,
        },
        wgpu::VertexAttribute {
            offset: vec3 * 2,
            shader_location: 7,
            format: wgpu::VertexFormat::Float32x3,
        },
        wgpu::VertexAttribute {
            offset: vec3 * 3,
            shader_location: 8,
            format: wgpu::VertexFormat::Float32x4,
        },
    ]
}

/// Instance-step attributes for text: `m0`/`m1`/`m2` at locations 3..5, NO color
/// (color is per-glyph in [`TextVertex`]). Matches `msdf_text.wgsl`'s @location(3..5).
#[cfg(feature = "wgpu-probe")]
fn text_instance_attributes() -> [wgpu::VertexAttribute; 3] {
    let vec3 = std::mem::size_of::<[f32; 3]>() as u64;
    [
        wgpu::VertexAttribute {
            offset: 0,
            shader_location: 3,
            format: wgpu::VertexFormat::Float32x3,
        },
        wgpu::VertexAttribute {
            offset: vec3,
            shader_location: 4,
            format: wgpu::VertexFormat::Float32x3,
        },
        wgpu::VertexAttribute {
            offset: vec3 * 2,
            shader_location: 5,
            format: wgpu::VertexFormat::Float32x3,
        },
    ]
}

/// The ONE object-text MSDF atlas + the host text shaper that feeds it, plus the
/// GPU texture/view/sampler backing them. Completes the deferred "GPU cutover": the
/// bundled-font [`TextEngine`] rasterizes each committed glyph; the pure
/// [`MsdfAtlasPlan`] packs it into a real signed-distance slot. The dimensions match
/// the legacy text path (2048²) so the shader's `screenPxRange` math is consistent.
/// Re-population only adds NEWLY-committed glyphs (atlas grow on text-content change),
/// keeping the atlas refresh off the pan/zoom hot path.
///
/// Owned ONCE by the wrapper and shared by both the world and UI [`ObjectRenderer`]s
/// (`ui-architecture.md` decision #1: no duplicate atlas) — each renderer borrows it
/// to populate/upload and builds its own text bind group over the shared view+sampler
/// (binding 0 is the renderer's own camera uniform, so the bind group stays
/// per-renderer; the 16 MB texture + font shaper do not duplicate).
#[cfg(feature = "wgpu-probe")]
pub struct SharedObjectText {
    engine: TextEngine,
    plan: MsdfAtlasPlan,
    /// `(char, rounded px) -> slot`, the key the glyph-UV provider resolves. The
    /// pure core's `GlyphPlacement` carries only `(ch, size)`, not the run font.
    entries: std::collections::HashMap<(u32, u32), MsdfGlyphEntry>,
    /// Raster-texels-per-logical-px for the SDF source (= device pixel ratio): glyphs
    /// rasterize at `size * oversample` px so retina text stays sharp. The atlas key
    /// stays logical, so this is fixed at construction from the live dpr.
    oversample: f32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
}

/// The MSDF atlas distance-range spread (texels); matches `MsdfAtlasPlan::new`.
#[cfg(feature = "wgpu-probe")]
const OBJECT_ATLAS_DISTANCE_RANGE: f32 = 4.0;

#[cfg(feature = "wgpu-probe")]
impl SharedObjectText {
    pub fn new(device: &wgpu::Device, oversample: f32) -> Result<Self, String> {
        let plan = MsdfAtlasPlan::new(
            shape_renderer_core::text::TEXT_ATLAS_WIDTH,
            shape_renderer_core::text::TEXT_ATLAS_HEIGHT,
            OBJECT_ATLAS_DISTANCE_RANGE,
        );
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shape.ai object msdf atlas"),
            size: wgpu::Extent3d {
                width: plan.atlas_width,
                height: plan.atlas_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shape.ai object msdf sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(SharedObjectText {
            engine: TextEngine::new()?,
            plan,
            entries: std::collections::HashMap::new(),
            oversample: oversample.max(1.0),
            texture,
            view,
            sampler,
        })
    }

    /// Pack every glyph of the scene's committed text not yet in the atlas, and
    /// upload the texture only when a NEW glyph was packed (the zero-rebake /
    /// no-re-upload-on-pan-zoom contract, keyed on `(char, px)`). Delegates the
    /// populate decision to the single renderer-core seam so it lives GPU-free.
    fn populate_and_upload(&mut self, queue: &wgpu::Queue, scene: &RenderObjectScene) {
        if populate_atlas_from_scene(
            &mut self.plan,
            &mut self.entries,
            &self.engine,
            scene,
            self.oversample,
        ) {
            self.upload(queue);
        }
    }

    /// Build the scene's [`FramePlan`] with the real per-char advance + the populated
    /// atlas's per-glyph UV slots injected (the core stays pure: it calls neither).
    fn build_plan(&self, scene: &RenderObjectScene, theme: Theme) -> FramePlan {
        let measure = |ch: char, size: f32| self.engine.char_advance(ch, size);
        let glyph_uv = |ch: char, size: f32| -> Option<MsdfGlyphEntry> {
            self.entries.get(&(ch as u32, shape_renderer_core::cast::round_u32(size))).copied()
        };
        build_frame_plan_with_text(scene, theme, &measure, &glyph_uv)
    }

    /// Upload the populated atlas pixels into the shared texture (full 2048² extent).
    fn upload(&self, queue: &wgpu::Queue) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            self.plan.pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.plan.atlas_width * 4),
                rows_per_image: Some(self.plan.atlas_height),
            },
            wgpu::Extent3d {
                width: self.plan.atlas_width,
                height: self.plan.atlas_height,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// Owns the CPU-built object draw data, the GPU buffers it uploads to, and the
/// object render pass recorder.
#[cfg(feature = "wgpu-probe")]
pub struct ObjectRenderer {
    pub uniform_buffer: wgpu::Buffer,
    pub stroke_params_buffer: wgpu::Buffer,
    pub camera_bind_group: wgpu::BindGroup,
    pub stroke_bind_group: wgpu::BindGroup,
    pub fill_vertex_buffer: wgpu::Buffer,
    pub fill_index_buffer: wgpu::Buffer,
    pub fill_instance_buffer: wgpu::Buffer,
    pub shadow_vertex_buffer: wgpu::Buffer,
    pub shadow_instance_buffer: wgpu::Buffer,
    pub stroke_vertex_buffer: wgpu::Buffer,
    pub stroke_instance_buffer: wgpu::Buffer,
    pub text_vertex_buffer: wgpu::Buffer,
    pub text_instance_buffer: wgpu::Buffer,
    pub text_params_buffer: wgpu::Buffer,
    /// The per-renderer text bind group: binding 0 is this renderer's camera uniform,
    /// bindings 1/2 are the SHARED [`SharedObjectText`] atlas view + sampler (no
    /// duplicate texture), binding 3 is this renderer's text params.
    pub text_bind_group: wgpu::BindGroup,
    draws: Vec<ObjectDraw>,
    fill_index_count: u32,
    shadow_vertex_count: u32,
    stroke_vertex_count: u32,
    text_vertex_count: u32,
    /// The diffable draw-plan this renderer last uploaded. [`apply_plan_diff`]
    /// diffs a freshly-built plan against this and patches only what changed. The
    /// GPU buffers above ARE the plan's geometry store, so `self.plan.geometry`
    /// mirrors the device.
    ///
    /// [`apply_plan_diff`]: ObjectRenderer::apply_plan_diff
    plan: FramePlan,
    /// The active light/dark theme: sources the canvas clear color and is the bit
    /// [`ObjectRenderer::set_theme`] flips, so token-backed instance colors
    /// re-resolve on toggle without re-tessellation.
    theme: Theme,
    /// Per-object live preview WORLD transform (`delta * base`) under an in-flight
    /// drag, mirroring what `set_preview_transform` wrote to the GPU so the CPU side
    /// (handles / region bounds) tracks the dragged bbox. A `Vec` (not a hashed map)
    /// keeps the core free of randomness; at most a few objects are previewed at once.
    preview_transforms: Vec<(String, [[f64; 3]; 3])>,
}

#[cfg(feature = "wgpu-probe")]
impl ObjectRenderer {
    /// Build the renderer for a scene: tessellate fills into a shared megabuffer,
    /// expand strokes into ribbons, resolve per-object instance data, and upload
    /// all buffers. Records nothing — call [`ObjectRenderer::render`] in a frame.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &ObjectPipeline,
        text: &mut SharedObjectText,
        scene: &RenderObjectScene,
        pixel_width: f32,
        pixel_height: f32,
        theme: Theme,
    ) -> Self {
        // Pack the scene's committed glyphs into the SHARED MSDF atlas (re-uploaded
        // only when it grew), then build the FramePlan with the shared shaper +
        // per-glyph UV slots so glyph quads carry real atlas coverage (the GPU
        // cutover). The atlas + shaper live on the wrapper, shared by world + UI.
        text.populate_and_upload(queue, scene);
        let plan = text.build_plan(scene, theme);
        let build = &plan.geometry;

        let uniform = ObjectMatrixUniform::from_scene(scene, pixel_width, pixel_height);
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai object camera uniform"),
            size: std::mem::size_of::<ObjectMatrixUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));

        let stroke_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai object stroke params uniform"),
            size: std::mem::size_of::<StrokeParamsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &stroke_params_buffer,
            0,
            bytemuck::cast_slice(&[StrokeParamsUniform::solid()]),
        );

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shape.ai object camera bind group"),
            layout: &pipeline.camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let stroke_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shape.ai object stroke bind group"),
            layout: &pipeline.stroke_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: stroke_params_buffer.as_entire_binding(),
                },
            ],
        });

        // Widen the megabuffer positions (`[f32;2]`) to the `FillVertex` layout
        // (`position` + `edge`). `fill_edges` (the per-vertex silhouette flag, 1 on
        // the boundary / 0 interior) is index-aligned, so the zip is a pure widening.
        let fill_vertices: Vec<FillVertex> = build
            .fill
            .vertices
            .iter()
            .zip(&build.fill_edges)
            .map(|(&position, &edge)| FillVertex { position, edge })
            .collect();

        let fill_vertex_buffer = create_vertex_buffer(device, "object fill vertices", &fill_vertices);
        let fill_index_buffer = create_index_buffer(device, "object fill indices", &build.fill.indices);
        let fill_instance_buffer =
            create_vertex_buffer(device, "object fill instances", &build.fill_instances);
        let shadow_vertex_buffer =
            create_vertex_buffer(device, "object shadow vertices", &build.shadow_vertices);
        let shadow_instance_buffer =
            create_vertex_buffer(device, "object shadow instances", &build.shadow_instances);
        let stroke_vertex_buffer =
            create_vertex_buffer(device, "object stroke vertices", &build.stroke_vertices);
        let stroke_instance_buffer =
            create_vertex_buffer(device, "object stroke instances", &build.stroke_instances);
        let text_vertex_buffer =
            create_vertex_buffer(device, "object text vertices", &build.text_vertices);
        let text_instance_buffer =
            create_vertex_buffer(device, "object text instances", &build.text_instances);

        if !fill_vertices.is_empty() {
            queue.write_buffer(&fill_vertex_buffer, 0, bytemuck::cast_slice(&fill_vertices));
        }
        if !build.fill.indices.is_empty() {
            queue.write_buffer(
                &fill_index_buffer,
                0,
                bytemuck::cast_slice(&build.fill.indices),
            );
        }
        if !build.fill_instances.is_empty() {
            queue.write_buffer(
                &fill_instance_buffer,
                0,
                bytemuck::cast_slice(&build.fill_instances),
            );
        }
        if !build.shadow_vertices.is_empty() {
            queue.write_buffer(
                &shadow_vertex_buffer,
                0,
                bytemuck::cast_slice(&build.shadow_vertices),
            );
        }
        if !build.shadow_instances.is_empty() {
            queue.write_buffer(
                &shadow_instance_buffer,
                0,
                bytemuck::cast_slice(&build.shadow_instances),
            );
        }
        if !build.stroke_vertices.is_empty() {
            queue.write_buffer(
                &stroke_vertex_buffer,
                0,
                bytemuck::cast_slice(&build.stroke_vertices),
            );
        }
        if !build.stroke_instances.is_empty() {
            queue.write_buffer(
                &stroke_instance_buffer,
                0,
                bytemuck::cast_slice(&build.stroke_instances),
            );
        }
        if !build.text_vertices.is_empty() {
            queue.write_buffer(
                &text_vertex_buffer,
                0,
                bytemuck::cast_slice(&build.text_vertices),
            );
        }
        if !build.text_instances.is_empty() {
            queue.write_buffer(
                &text_instance_buffer,
                0,
                bytemuck::cast_slice(&build.text_instances),
            );
        }

        // ---- MSDF text params + bind group --------------------------------
        // The SHARED atlas texture holds the real glyph coverage; `text_params` feeds
        // the SAME dimensions/range so the shader's screenPxRange AA math matches the
        // data. The bind group is per-renderer (binding 0 = this camera uniform) but
        // points at the shared atlas view + sampler — no duplicate 16 MB texture.
        let text_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai object text params uniform"),
            size: std::mem::size_of::<TextUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let text_params = TextUniform {
            atlas: [
                text.plan.distance_range,
                text.plan.atlas_width as f32,
                text.plan.atlas_height as f32,
                0.0,
            ],
        };
        queue.write_buffer(&text_params_buffer, 0, bytemuck::cast_slice(&[text_params]));
        let text_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shape.ai object text bind group"),
            layout: &pipeline.text_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&text.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&text.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: text_params_buffer.as_entire_binding(),
                },
            ],
        });

        ObjectRenderer {
            uniform_buffer,
            stroke_params_buffer,
            camera_bind_group,
            stroke_bind_group,
            fill_vertex_buffer,
            fill_index_buffer,
            fill_instance_buffer,
            shadow_vertex_buffer,
            shadow_instance_buffer,
            stroke_vertex_buffer,
            stroke_instance_buffer,
            text_vertex_buffer,
            text_instance_buffer,
            text_params_buffer,
            text_bind_group,
            fill_index_count: shape_renderer_core::cast::len_u32(build.fill.indices.len()),
            shadow_vertex_count: shape_renderer_core::cast::len_u32(build.shadow_vertices.len()),
            stroke_vertex_count: shape_renderer_core::cast::len_u32(build.stroke_vertices.len()),
            text_vertex_count: shape_renderer_core::cast::len_u32(build.text_vertices.len()),
            draws: build.draws.clone(),
            plan,
            theme,
            preview_transforms: Vec::new(),
        }
    }

    /// Number of objects with recorded draw data.
    pub fn object_count(&self) -> usize {
        self.draws.len()
    }

    /// The retained FramePlan this renderer last uploaded — the diff base for
    /// [`apply_plan_diff`].
    ///
    /// [`apply_plan_diff`]: ObjectRenderer::apply_plan_diff
    pub fn plan(&self) -> &FramePlan {
        &self.plan
    }

    /// Re-feed `scene` by building its [`FramePlan`], diffing it against the
    /// retained plan, and applying the targeted [`PlanPatch`]es through the GPU
    /// write paths instead of reconstructing on every feed.
    ///
    /// - [`PlanPatch::TransformUpdate`] -> the 4-buffer matrix write, the SAME path
    ///   the drag preview uses.
    /// - [`PlanPatch::StyleUpdate`] -> the per-pass color-slot write, the SAME path
    ///   the theme toggle uses.
    /// - [`PlanPatch::GeometryUpdate`] -> a single object's mesh re-send over its
    ///   existing ranges when size-safe; otherwise the diff degrades to a rebuild.
    /// - [`PlanDiff::Rebuild`] (structural) -> signal a rebuild.
    ///
    /// When `needs_rebuild` is true the GPU buffers are left untouched, so the
    /// caller's fresh [`ObjectRenderer::new`] is the single clean re-upload.
    pub fn apply_plan_diff(
        &mut self,
        queue: &wgpu::Queue,
        text: &mut SharedObjectText,
        scene: &RenderObjectScene,
    ) -> PlanApplyStats {
        // Pack any NEWLY-committed glyphs into the SHARED atlas (re-uploaded only when
        // it grew — off the pan/zoom hot path). Then build the next plan with the
        // shared shaper's real per-char advance + per-glyph UV slots.
        text.populate_and_upload(queue, scene);
        let next = text.build_plan(scene, self.theme);
        let diff = diff_plans(&self.plan, &next);
        let patches = match diff {
            PlanDiff::Rebuild => {
                return PlanApplyStats {
                    patch_count: 0,
                    needs_rebuild: true,
                };
            }
            PlanDiff::Patches(patches) => patches,
        };

        // A geometry update is only safe in place when the new counts still fill the
        // retained ranges (`follower_patch_plan` guard). If ANY fails, the whole feed
        // degrades to a rebuild and we touch nothing, leaving a clean slate.
        for patch in &patches {
            if let PlanPatch::GeometryUpdate { index, entry, .. } = patch {
                let old_draw = &self.plan.entries[*index].draw;
                if follower_patch_plan(old_draw, &geometry_reexpand(&next, *index)).is_none() {
                    return PlanApplyStats {
                        patch_count: 0,
                        needs_rebuild: true,
                    };
                }
                debug_assert_eq!(entry.draw.fill_range, old_draw.fill_range);
            }
        }

        let patch_count = patches.len();
        for patch in &patches {
            self.apply_patch(queue, &next, patch);
        }
        // Adopt the new plan and refresh the mirror state the render loops read.
        self.draws = next.geometry.draws.clone();
        self.fill_index_count = shape_renderer_core::cast::len_u32(next.geometry.fill.indices.len());
        self.shadow_vertex_count = shape_renderer_core::cast::len_u32(next.geometry.shadow_vertices.len());
        self.stroke_vertex_count = shape_renderer_core::cast::len_u32(next.geometry.stroke_vertices.len());
        self.text_vertex_count = shape_renderer_core::cast::len_u32(next.geometry.text_vertices.len());
        self.plan = next;
        PlanApplyStats {
            patch_count,
            needs_rebuild: false,
        }
    }

    /// Apply one [`PlanPatch`] to the GPU buffers. `next` is the plan the patch came
    /// from (the geometry source for a `GeometryUpdate`).
    fn apply_patch(&mut self, queue: &wgpu::Queue, next: &FramePlan, patch: &PlanPatch) {
        match patch {
            PlanPatch::TransformUpdate { index, columns, .. } => {
                self.write_instance_matrix(queue, *index, columns);
            }
            PlanPatch::StyleUpdate {
                index, slot, color, ..
            } => {
                self.write_instance_color(queue, *index, *slot, color);
            }
            PlanPatch::GeometryUpdate { index, entry, .. } => {
                // Re-send only this object's mesh over its existing ranges, then the
                // new matrix + colors.
                let rebuilt = geometry_reexpand(next, *index);
                let id = entry.handle.object.clone();
                self.patch_follower_geometry(queue, &id, &rebuilt);
                let columns = [entry.instance.fill.m0, entry.instance.fill.m1, entry.instance.fill.m2];
                self.write_instance_matrix(queue, *index, &columns);
                self.write_instance_color(queue, *index, StyleSlot::Fill, &entry.instance.fill.fill);
                self.write_instance_color(queue, *index, StyleSlot::Stroke, &entry.instance.stroke.stroke);
                self.write_instance_color(queue, *index, StyleSlot::Shadow, &entry.instance.shadow.shadow);
            }
            PlanPatch::Rebuild => {
                debug_assert!(false, "PlanPatch::Rebuild should never appear inside a patch list");
            }
        }
    }

    /// Write the 36-byte matrix region (offset 0) of object `index`'s instance in
    /// EVERY per-pass buffer (fill/stroke/text/shadow), strided by
    /// [`preview_instance_strides`] — same buffers/offsets as
    /// [`set_preview_transform`]'s `delta*base` write, with absolute columns.
    ///
    /// [`set_preview_transform`]: ObjectRenderer::set_preview_transform
    fn write_instance_matrix(&self, queue: &wgpu::Queue, index: usize, columns: &[[f32; 3]; 3]) {
        let bytes = bytemuck::cast_slice(columns);
        let strides = preview_instance_strides();
        let buffers = [
            &self.fill_instance_buffer,
            &self.stroke_instance_buffer,
            &self.text_instance_buffer,
            &self.shadow_instance_buffer,
        ];
        for (buffer, stride) in buffers.iter().zip(strides) {
            queue.write_buffer(buffer, (index as u64) * stride, bytes);
        }
    }

    /// Write the 16-byte color slot (struct offset 36, past the matrix) of object
    /// `index`'s instance for one pass, leaving the matrix untouched — the same
    /// write [`set_theme`] uses.
    ///
    /// [`set_theme`]: ObjectRenderer::set_theme
    fn write_instance_color(
        &self,
        queue: &wgpu::Queue,
        index: usize,
        slot: StyleSlot,
        color: &[f32; 4],
    ) {
        let (buffer, stride) = match slot {
            StyleSlot::Fill => (&self.fill_instance_buffer, std::mem::size_of::<FillInstance>()),
            StyleSlot::Stroke => (
                &self.stroke_instance_buffer,
                std::mem::size_of::<StrokeInstance>(),
            ),
            StyleSlot::Shadow => (
                &self.shadow_instance_buffer,
                std::mem::size_of::<ShadowInstance>(),
            ),
        };
        // Color slot is at offset 36 in all three instance structs (pinned by the
        // renderer-core layout tests), strictly past the matrix.
        let offset = (index * stride + 36) as u64;
        queue.write_buffer(buffer, offset, bytemuck::cast_slice(color));
    }

    /// Rebuild + re-upload the camera uniform from the live camera + viewport.
    /// Called every frame so pan/zoom moves objects without a scene reload — the
    /// per-object instance matrices stay put. Byte-identical packing to
    /// [`ObjectMatrixUniform::from_scene`].
    pub fn update_camera(
        &self,
        queue: &wgpu::Queue,
        camera: &CameraState,
        pixel_width: f32,
        pixel_height: f32,
    ) {
        let uniform = ObjectMatrixUniform {
            camera: [shape_renderer_core::cast::narrow_f32(camera.x), shape_renderer_core::cast::narrow_f32(camera.y), shape_renderer_core::cast::narrow_f32(camera.zoom), 0.0],
            viewport: [pixel_width, pixel_height, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));
    }

    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// Zero-rebake theme toggle: flip the theme bit and re-resolve ONLY the
    /// token-backed instance colors, writing the 16-byte color slot (offset 36) of
    /// each affected instance. Tessellation is never touched. Raw hex / gradient /
    /// image paints are theme-invariant and skipped; the clear color tracks
    /// `self.theme` in [`ObjectRenderer::render`]. No-op if `dark` already matches.
    pub fn set_theme(&mut self, queue: &wgpu::Queue, dark: bool) -> Theme {
        if self.theme.dark == dark {
            return self.theme;
        }
        self.theme = Theme { dark };
        // The default drop-shadow color is the `shadow` token, so re-resolve every
        // shadow instance color from the same token table (zero rebake).
        let shadow_color = self.theme.shadow();
        for (i, draw) in self.draws.iter_mut().enumerate() {
            if !draw.shadow_range.is_empty() {
                draw.shadow_instance.shadow = shadow_color;
                let offset = (i * std::mem::size_of::<ShadowInstance>()
                    + std::mem::offset_of!(ShadowInstance, shadow)) as u64;
                queue.write_buffer(
                    &self.shadow_instance_buffer,
                    offset,
                    bytemuck::cast_slice(&shadow_color),
                );
            }
            if let Some(name) = draw.fill_token.as_deref() {
                let color = resolve_token_f32(name, dark).unwrap_or([1.0, 1.0, 1.0, 1.0]);
                draw.fill_instance.fill = color;
                let offset = (i * std::mem::size_of::<FillInstance>()
                    + std::mem::offset_of!(FillInstance, fill)) as u64;
                queue.write_buffer(&self.fill_instance_buffer, offset, bytemuck::cast_slice(&color));
            }
            if let Some(name) = draw.stroke_token.as_deref() {
                let color = resolve_token_f32(name, dark).unwrap_or([1.0, 1.0, 1.0, 1.0]);
                draw.stroke_instance.stroke = color;
                let offset = (i * std::mem::size_of::<StrokeInstance>()
                    + std::mem::offset_of!(StrokeInstance, stroke)) as u64;
                queue.write_buffer(
                    &self.stroke_instance_buffer,
                    offset,
                    bytemuck::cast_slice(&color),
                );
            }
        }
        // Mirror the moved instance colors into the plan's entries so a later
        // `apply_plan_diff` does not re-emit redundant style updates against the
        // stale pre-flip colors.
        for (entry, draw) in self.plan.entries.iter_mut().zip(&self.draws) {
            entry.instance.fill = draw.fill_instance;
            entry.instance.stroke = draw.stroke_instance;
            entry.instance.shadow = draw.shadow_instance;
        }
        self.theme
    }

    /// Zero-rebake drag: write ONLY the dragged object's instance matrix to the GPU
    /// — no re-tessellation. Looks up its instance index `i` in `self.draws`,
    /// composes `delta * base` via [`preview_instance_columns`], and overwrites the
    /// 36-byte matrix region (offset 0) of every per-pass instance so the whole
    /// visual (fill, stroke, glyphs, shadow) follows. Returns false if `id` is absent.
    pub fn set_preview_transform(
        &mut self,
        queue: &wgpu::Queue,
        id: &str,
        delta: &[[f64; 3]; 3],
        base: &[[f64; 3]; 3],
    ) -> bool {
        let Some(i) = self.draws.iter().position(|d| d.id == id) else {
            return false;
        };
        let (m0, m1, m2) = preview_instance_columns(delta, base);
        let columns: [[f32; 3]; 3] = [m0, m1, m2];
        let bytes = bytemuck::cast_slice(&columns);
        // The matrix region (offset 0) is overwritten in EVERY per-object instance
        // buffer (fill, stroke, text, shadow), each index-aligned with `draws` so the
        // write lands at `i * stride`. `preview_instance_strides` is the single source
        // pairing each buffer with its struct stride; a dropped buffer is a dropped
        // sub-visual under drag. Baked color survives (past byte 36, or per-glyph text).
        let strides = preview_instance_strides();
        let buffers = [
            &self.fill_instance_buffer,
            &self.stroke_instance_buffer,
            &self.text_instance_buffer,
            &self.shadow_instance_buffer,
        ];
        debug_assert_eq!(
            buffers.len(),
            strides.len(),
            "every previewed instance buffer has a stride (text + shadow included)"
        );
        for (buffer, stride) in buffers.iter().zip(strides) {
            queue.write_buffer(buffer, (i as u64) * stride, bytes);
        }
        // Mirror the composed WORLD transform on the CPU so handles / region bounds
        // track the dragged bbox (read-only, no rebake).
        let world = shape_renderer_core::hit_test_object::mat3_mul(delta, base);
        match self.preview_transforms.iter_mut().find(|(pid, _)| pid == id) {
            Some(entry) => entry.1 = world,
            None => self.preview_transforms.push((id.to_string(), world)),
        }
        true
    }

    /// Revert the dragged object's instance matrix to its canonical baked transform
    /// (`delta = identity`), dropping the live preview. Returns false if `id` is absent.
    pub fn clear_preview_transform(&mut self, queue: &wgpu::Queue, id: &str, base: &[[f64; 3]; 3]) -> bool {
        let written =
            self.set_preview_transform(queue, id, &shape_renderer_core::hit_test_object::identity_3x3(), base);
        // Drop the CPU preview so handles fall back to the canonical region.
        self.preview_transforms.retain(|(pid, _)| pid != id);
        written
    }

    /// Patch a follower's baked vertices in place so its anchored node tracks a
    /// moved target DURING a drag (the follower is not uniformly transformed, so the
    /// instance-matrix preview cannot express it). `rebuilt` is the follower
    /// re-expanded with the reprojected node; this writes its fill vertices, fill
    /// indices (rebased), stroke ribbon, shadow silhouette, and glyph quads over the
    /// follower's EXISTING ranges — O(one small object).
    ///
    /// A vertex/index COUNT that no longer matches the baked range makes
    /// [`follower_patch_plan`] return `None`, and this SKIPS the write entirely
    /// rather than corrupt the buffer. Returns false if `id` is absent or skipped.
    pub fn patch_follower_geometry(
        &mut self,
        queue: &wgpu::Queue,
        id: &str,
        rebuilt: &FollowerReexpand,
    ) -> bool {
        let Some(draw) = self.draws.iter().find(|d| d.id == id) else {
            return false;
        };
        let Some(plan) = follower_patch_plan(draw, rebuilt) else {
            return false;
        };
        // Fill vertices: same count as the baked range (guarded), so the write stays
        // within `[start, end)` of the shared buffer.
        if !rebuilt.fill_vertices.is_empty() {
            queue.write_buffer(
                &self.fill_vertex_buffer,
                plan.fill_vertex_byte_offset,
                bytemuck::cast_slice(&rebuilt.fill_vertices),
            );
            // Re-emit indices rebased to the follower's vertex base (corrects a
            // changed index pattern, not just positions).
            let rebased: Vec<u32> = rebuilt
                .fill_indices
                .iter()
                .map(|&i| i + plan.fill_index_rebase)
                .collect();
            queue.write_buffer(
                &self.fill_index_buffer,
                plan.fill_index_byte_offset,
                bytemuck::cast_slice(&rebased),
            );
        }
        // Stroke ribbon vertices: same count as the baked range (guarded).
        if !rebuilt.stroke_vertices.is_empty() {
            queue.write_buffer(
                &self.stroke_vertex_buffer,
                plan.stroke_vertex_byte_offset,
                bytemuck::cast_slice(&rebuilt.stroke_vertices),
            );
        }
        // Drop-shadow silhouette vertices — without this the follower's shadow stays
        // at the OLD geometry until the commit rebake.
        if !rebuilt.shadow_vertices.is_empty() {
            queue.write_buffer(
                &self.shadow_vertex_buffer,
                plan.shadow_vertex_byte_offset,
                bytemuck::cast_slice(&rebuilt.shadow_vertices),
            );
        }
        // Glyph quads — text layout depends on the region bbox, so a reprojected node
        // moves the glyphs too.
        if !rebuilt.text_vertices.is_empty() {
            queue.write_buffer(
                &self.text_vertex_buffer,
                plan.text_vertex_byte_offset,
                bytemuck::cast_slice(&rebuilt.text_vertices),
            );
        }
        true
    }

    /// The live preview WORLD transform (`delta * base`) for `id`, or `None` with no
    /// in-flight drag — the transform `set_preview_transform` pushed, so a caller can
    /// lay out handles / region bounds against the PREVIEWED bbox (a pure read).
    pub fn preview_transform(&self, id: &str) -> Option<[[f64; 3]; 3]> {
        self.preview_transforms
            .iter()
            .find(|(pid, _)| pid == id)
            .map(|(_, world)| *world)
    }

    /// Record the object draw pass into `encoder` targeting `view`: one instanced
    /// indexed fill draw per object, then one instanced stroke draw, then text.
    /// `clear` chooses whether the pass clears the color attachment first.
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        pipeline: &ObjectPipeline,
        clear: bool,
    ) {
        let load = if clear {
            // The canvas backdrop is the `canvas-bg` token, flipping with
            // `self.theme` — no buffer write needed.
            let [r, g, b, a] = self.theme.canvas_bg();
            wgpu::LoadOp::Clear(wgpu::Color {
                r: r as f64,
                g: g as f64,
                b: b as f64,
                a: a as f64,
            })
        } else {
            wgpu::LoadOp::Load
        };
        let color_attachments = [Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shape.ai object render pass"),
            color_attachments: &color_attachments,
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        // The drop shadow is rendered + blurred + composited UNDER the fill before
        // this `render` runs (see `render_shadow_mask` / `frame.rs`); this pass starts
        // with the fill so the composited shadow stays beneath fill/stroke/text.

        // Fill pass: one indexed instanced draw per object over the shared megabuffer.
        if self.fill_index_count > 0 {
            pass.set_pipeline(&pipeline.fill_pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, self.fill_vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, self.fill_instance_buffer.slice(..));
            pass.set_index_buffer(self.fill_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            for (instance, draw) in self.draws.iter().enumerate() {
                if draw.fill_range.is_empty() {
                    continue;
                }
                let instance = shape_renderer_core::cast::len_u32(instance);
                pass.draw_indexed(
                    draw.fill_range.start..draw.fill_range.end,
                    0,
                    instance..instance + 1,
                );
            }
        }

        // Stroke pass: one instanced draw per object over its ribbon range.
        if self.stroke_vertex_count > 0 {
            pass.set_pipeline(&pipeline.stroke_pipeline);
            pass.set_bind_group(0, &self.stroke_bind_group, &[]);
            pass.set_vertex_buffer(0, self.stroke_vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, self.stroke_instance_buffer.slice(..));
            for (instance, draw) in self.draws.iter().enumerate() {
                if draw.stroke_range.is_empty() {
                    continue;
                }
                let instance = shape_renderer_core::cast::len_u32(instance);
                pass.draw(
                    draw.stroke_range.start..draw.stroke_range.end,
                    instance..instance + 1,
                );
            }
        }

        // Text pass: one instanced draw per object over its glyph-quad range, OVER
        // fill+stroke. Each object's glyphs ride its own matrix instance (index `i`),
        // index-aligned with `draws` like the fill/stroke loops.
        if self.text_vertex_count > 0 {
            pass.set_pipeline(&pipeline.text_pipeline);
            pass.set_bind_group(0, &self.text_bind_group, &[]);
            pass.set_vertex_buffer(0, self.text_vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, self.text_instance_buffer.slice(..));
            for (instance, draw) in self.draws.iter().enumerate() {
                if draw.text_range.is_empty() {
                    continue;
                }
                let instance = shape_renderer_core::cast::len_u32(instance);
                pass.draw(
                    draw.text_range.start..draw.text_range.end,
                    instance..instance + 1,
                );
            }
        }
    }

    /// Record the drop-shadow silhouette ONLY into an offscreen `mask` view (cleared
    /// transparent first) — the input to [`crate::shadow_blur::ShadowBlur`]. Each
    /// shadow rides its own matrix instance, index-aligned with `draws`, so the
    /// silhouette is placed identically to the fill. Records only the clear when no
    /// object casts a shadow, leaving the mask empty.
    pub fn render_shadow_mask(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &ObjectPipeline,
        mask: &wgpu::TextureView,
    ) {
        let color_attachments = [Some(wgpu::RenderPassColorAttachment {
            view: mask,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shape.ai shadow mask pass"),
            color_attachments: &color_attachments,
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if self.shadow_vertex_count == 0 {
            return;
        }
        pass.set_pipeline(&pipeline.shadow_pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, self.shadow_vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.shadow_instance_buffer.slice(..));
        for (instance, draw) in self.draws.iter().enumerate() {
            if draw.shadow_range.is_empty() {
                continue;
            }
            let instance = shape_renderer_core::cast::len_u32(instance);
            pass.draw(
                draw.shadow_range.start..draw.shadow_range.end,
                instance..instance + 1,
            );
        }
    }

    pub fn fill_index_count(&self) -> u32 {
        self.fill_index_count
    }

    pub fn shadow_vertex_count(&self) -> u32 {
        self.shadow_vertex_count
    }

    pub fn stroke_vertex_count(&self) -> u32 {
        self.stroke_vertex_count
    }

    pub fn text_vertex_count(&self) -> u32 {
        self.text_vertex_count
    }
}

/// Result of an [`ObjectRenderer::apply_plan_diff`] feed: `patch_count` targeted
/// patches applied, `needs_rebuild` set when the diff required a full reconstruction.
#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanApplyStats {
    pub patch_count: usize,
    pub needs_rebuild: bool,
}

/// Slice object `index`'s baked geometry out of a [`FramePlan`] into a
/// [`FollowerReexpand`] for the in-place patch path. The plan's megabuffer indices
/// are rebased by the object's vertex base; this returns them object-LOCAL (0-based)
/// to match `reexpand_single_object`'s contract.
#[cfg(feature = "wgpu-probe")]
fn geometry_reexpand(plan: &FramePlan, index: usize) -> FollowerReexpand {
    let geo = &plan.geometry;
    let draw = &plan.entries[index].draw;
    let fill_v = draw.fill_vertex_range;
    let fill_i = draw.fill_range;
    let stroke = draw.stroke_range;
    let shadow = draw.shadow_range;
    let text = draw.text_range;

    let fill_vertices: Vec<FillVertex> = geo.fill.vertices[fill_v.start as usize..fill_v.end as usize]
        .iter()
        .zip(&geo.fill_edges[fill_v.start as usize..fill_v.end as usize])
        .map(|(&position, &edge)| FillVertex { position, edge })
        .collect();
    let base = fill_v.start;
    let fill_indices: Vec<u32> = geo.fill.indices[fill_i.start as usize..fill_i.end as usize]
        .iter()
        .map(|&i| i - base)
        .collect();
    FollowerReexpand {
        fill_vertices,
        fill_indices,
        stroke_vertices: geo.stroke_vertices[stroke.start as usize..stroke.end as usize].to_vec(),
        shadow_vertices: geo.shadow_vertices[shadow.start as usize..shadow.end as usize].to_vec(),
        text_vertices: geo.text_vertices[text.start as usize..text.end as usize].to_vec(),
    }
}

/// Create a `VERTEX | COPY_DST` buffer sized for `data` (min 4 bytes so an empty
/// scene still produces a valid buffer handle).
#[cfg(feature = "wgpu-probe")]
fn create_vertex_buffer<T: bytemuck::Pod>(
    device: &wgpu::Device,
    label: &str,
    data: &[T],
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (std::mem::size_of_val(data) as u64).max(4),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(feature = "wgpu-probe")]
fn create_index_buffer(device: &wgpu::Device, label: &str, data: &[u32]) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (std::mem::size_of_val(data) as u64).max(4),
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(all(test, feature = "wgpu-probe"))]
mod tests {
    use super::*;
    use shape_renderer_core::plan::build_frame_plan;

    #[test]
    fn fill_and_stroke_instance_attributes_match_shader_contract() {
        let fill_attrs = fill_instance_attributes();
        assert_eq!(fill_attrs[0].offset, 0);
        assert_eq!(fill_attrs[1].offset, 12);
        assert_eq!(fill_attrs[2].offset, 24);
        assert_eq!(fill_attrs[3].offset, 36);
        assert_eq!(fill_attrs[3].shader_location, 5);

        let stroke_attrs = stroke_instance_attributes();
        assert_eq!(stroke_attrs[0].shader_location, 5);
        assert_eq!(stroke_attrs[3].shader_location, 8);
        assert_eq!(stroke_attrs[3].offset, 36);
    }

    #[test]
    fn text_pipeline_layout_matches_msdf_shader_contract() {
        assert_eq!(std::mem::size_of::<TextVertex>(), 32);
        let attrs = text_instance_attributes();
        assert_eq!(attrs[0].offset, 0);
        assert_eq!(attrs[0].shader_location, 3);
        assert_eq!(attrs[1].offset, 12);
        assert_eq!(attrs[1].shader_location, 4);
        assert_eq!(attrs[2].offset, 24);
        assert_eq!(attrs[2].shader_location, 5);
        assert_eq!(std::mem::offset_of!(TextVertex, position), 0);
        assert_eq!(std::mem::offset_of!(TextVertex, uv), 8);
        assert_eq!(std::mem::offset_of!(TextVertex, color), 16);
    }

    use shape_renderer_core::model::CameraState;
    use shape_renderer_core::render_object::{
        RFill, RPaint, RStroke, RStrokeCap, RStrokeJoin, RenderObject,
    };

    fn rect(id: &str, d: &str) -> RenderObject {
        RenderObject {
            id: id.to_string(),
            parent: None,
            order: "a0".to_string(),
            transform: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            geometry_d: d.to_string(),
            fill: Some(RFill {
                paint: RPaint::Solid { color: "#ff0000".to_string() },
                opacity: 1.0,
            }),
            stroke: Some(RStroke {
                paint: RPaint::Solid { color: "#00ff00".to_string() },
                width: 4.0,
                opacity: 1.0,
                dash: Vec::new(),
                cap: RStrokeCap::Butt,
                join: RStrokeJoin::Miter,
            }),
            text: None,
            anchors: Vec::new(),
            clip: false,
            hidden: false,
            locked: false,
        }
    }

    fn scene(objects: Vec<RenderObject>) -> RenderObjectScene {
        RenderObjectScene {
            scene_id: "ir".to_string(),
            camera: CameraState { x: 0.0, y: 0.0, zoom: 1.0 },
            objects,
            selection: None,
            multi_select: Vec::new(),
        }
    }

    /// `geometry_reexpand` must slice an object out of a `FramePlan` into the EXACT
    /// bytes `reexpand_single_object` produces, for a NON-first object where the
    /// megabuffer index rebase is load-bearing.
    #[test]
    fn geometry_reexpand_slices_match_reexpand_single_object() {
        // Object index 1 has a non-zero megabuffer vertex base.
        let s = scene(vec![
            rect("a", "M0 0 L800 0 L800 800 L0 800 Z"),
            rect("b", "M0 0 L640 0 L640 640 L0 640 Z"),
        ]);
        let plan = build_frame_plan(&s, Theme::light());

        for index in [0usize, 1usize] {
            let sliced = geometry_reexpand(&plan, index);
            let oracle = reexpand_single_object(
                &s.objects[index],
                Theme::light(),
                s.camera.clone(),
            );
            assert_eq!(sliced.fill_vertices, oracle.fill_vertices, "fill verts parity (obj {index})");
            assert_eq!(sliced.fill_indices, oracle.fill_indices, "object-local indices parity (obj {index})");
            assert_eq!(sliced.stroke_vertices, oracle.stroke_vertices, "stroke parity (obj {index})");
            assert_eq!(sliced.shadow_vertices, oracle.shadow_vertices, "shadow parity (obj {index})");
            assert_eq!(sliced.text_vertices, oracle.text_vertices, "text parity (obj {index})");
        }

        // The sliced geometry is size-safe to patch in place (the contract
        // `apply_plan_diff`'s geometry route relies on).
        let plan_b = &plan.entries[1].draw;
        assert!(
            follower_patch_plan(plan_b, &geometry_reexpand(&plan, 1)).is_some(),
            "a plan slice fills its own object's ranges exactly"
        );
    }

    use shape_renderer_core::render_object::{RText, RTextAlign, RTextRun, RTextValign, QUANT_PER_PX};

    /// A committed text object: a wide rect frame with one run at the 16px default.
    fn text_object(id: &str, run_text: &str) -> RenderObject {
        let mut obj = rect(id, "M0 0 L1600 0 L1600 800 L0 800 Z");
        obj.text = Some(RText {
            runs: vec![RTextRun {
                text: run_text.to_string(),
                color: "#ffffff".to_string(),
                size: 16.0 * QUANT_PER_PX,
                bold: false,
                italic: false,
                font: String::new(),
            }],
            align: RTextAlign::Start,
            valign: RTextValign::Top,
        });
        obj
    }

    /// DEFECT 3 (the GPU cutover, host-side data half): the object-text atlas the GPU
    /// samples must carry REAL glyph coverage and the glyph quads must carry REAL atlas
    /// slots — the two halves that were blank/placeholder before. Drives the SAME
    /// populate + build seam `SharedObjectText` (and so `ObjectRenderer::new`/
    /// `apply_plan_diff`) delegates to, GPU-free (the texture upload needs a device but
    /// the data decision does not). FAILS against the old empty `MsdfAtlasPlan::new`
    /// upload + placeholder uv 0..1.
    #[test]
    fn object_text_atlas_populates_real_coverage_and_glyph_uvs() {
        let s = scene(vec![text_object("t", "AB")]);
        let engine = TextEngine::new().expect("bundled fonts load");
        let mut plan = MsdfAtlasPlan::new(
            shape_renderer_core::text::TEXT_ATLAS_WIDTH,
            shape_renderer_core::text::TEXT_ATLAS_HEIGHT,
            OBJECT_ATLAS_DISTANCE_RANGE,
        );
        let mut entries: std::collections::HashMap<(u32, u32), MsdfGlyphEntry> =
            std::collections::HashMap::new();

        // First populate packs new glyphs (returns grew=true, which gates the texture
        // upload); a second populate of the same scene is a no-op (off the pan/zoom hot
        // path) — the exact decision `SharedObjectText::populate_and_upload` rides.
        assert!(
            populate_atlas_from_scene(&mut plan, &mut entries, &engine, &s, 1.0),
            "committed glyphs pack into the atlas"
        );
        assert!(
            !populate_atlas_from_scene(&mut plan, &mut entries, &engine, &s, 1.0),
            "no new glyphs => no re-pack (zero-rebake on pan/zoom)"
        );
        assert!(plan.glyph_count() > 0, "atlas has packed glyphs");
        assert!(
            plan.pixels().iter().any(|&p| p > 0),
            "atlas pixels carry real coverage, not the all-zero blank atlas"
        );

        // The built plan's glyph quads carry sub-unit atlas UVs, not the full-atlas
        // placeholder (uv 0..1) the blank pipeline shipped.
        let measure = |ch: char, size: f32| engine.char_advance(ch, size);
        let glyph_uv = |ch: char, size: f32| -> Option<MsdfGlyphEntry> {
            entries.get(&(ch as u32, shape_renderer_core::cast::round_u32(size))).copied()
        };
        let built = build_frame_plan_with_text(&s, Theme::light(), &measure, &glyph_uv);
        let text_vertices = &built.geometry.text_vertices;
        assert!(!text_vertices.is_empty(), "committed text emits glyph quads");
        let all_placeholder = text_vertices.iter().all(|v| {
            (v.uv == [0.0, 0.0]) || (v.uv == [1.0, 0.0]) || (v.uv == [1.0, 1.0]) || (v.uv == [0.0, 1.0])
        });
        assert!(
            !all_placeholder,
            "glyph quads carry real sub-unit atlas UVs, not the full-atlas placeholder"
        );
    }

    /// SHARED atlas (decision #1: no duplicate atlas): a glyph packed while serving the
    /// WORLD scene must already be in the atlas when the UI scene reuses it — the UI
    /// build resolves it to a REAL sub-unit slot with NO re-populate. A duplicate
    /// per-renderer atlas would start empty for the UI, re-pack the glyph (grew=true),
    /// and the UI quad would carry the full-atlas placeholder until then. Drives the
    /// one populate/entries/build seam `SharedObjectText` shares across both renderers.
    #[test]
    fn shared_atlas_serves_ui_glyphs_committed_by_the_world_scene() {
        let engine = TextEngine::new().expect("bundled fonts load");
        let mut plan = MsdfAtlasPlan::new(
            shape_renderer_core::text::TEXT_ATLAS_WIDTH,
            shape_renderer_core::text::TEXT_ATLAS_HEIGHT,
            OBJECT_ATLAS_DISTANCE_RANGE,
        );
        let mut entries: std::collections::HashMap<(u32, u32), MsdfGlyphEntry> =
            std::collections::HashMap::new();

        // World scene commits "AB"; the UI scene reuses the SAME glyphs at the SAME size.
        let world = scene(vec![text_object("w", "AB")]);
        let ui = scene(vec![text_object("u", "AB")]);
        assert!(
            populate_atlas_from_scene(&mut plan, &mut entries, &engine, &world, 1.0),
            "world commit packs its glyphs"
        );
        // The UI feed into the SAME atlas packs nothing new: the world already did.
        assert!(
            !populate_atlas_from_scene(&mut plan, &mut entries, &engine, &ui, 1.0),
            "UI reuses the shared atlas — no re-pack of glyphs the world committed"
        );

        // The UI plan, built from the shared entries, resolves to real sub-unit slots
        // (not the full-atlas placeholder), proving the world's glyphs serve the UI.
        let measure = |ch: char, size: f32| engine.char_advance(ch, size);
        let glyph_uv = |ch: char, size: f32| -> Option<MsdfGlyphEntry> {
            entries.get(&(ch as u32, shape_renderer_core::cast::round_u32(size))).copied()
        };
        let built = build_frame_plan_with_text(&ui, Theme::light(), &measure, &glyph_uv);
        let text_vertices = &built.geometry.text_vertices;
        assert!(!text_vertices.is_empty(), "UI text emits glyph quads");
        let all_placeholder = text_vertices.iter().all(|v| {
            (v.uv == [0.0, 0.0]) || (v.uv == [1.0, 0.0]) || (v.uv == [1.0, 1.0]) || (v.uv == [0.0, 1.0])
        });
        assert!(
            !all_placeholder,
            "UI quads carry the world-committed sub-unit slots, not the empty-atlas placeholder"
        );
    }

    /// `geometry_reexpand` must un-rebase object 1's merged-megabuffer indices back
    /// to object-LOCAL (0-based), so the max index is below its own vertex count
    /// (a dropped un-rebase would write merged indices over a 0-based range).
    #[test]
    fn geometry_reexpand_returns_object_local_indices() {
        let s = scene(vec![
            rect("a", "M0 0 L800 0 L800 800 L0 800 Z"),
            rect("b", "M0 0 L640 0 L640 640 L0 640 Z"),
        ]);
        let plan = build_frame_plan(&s, Theme::light());
        let sliced = geometry_reexpand(&plan, 1);
        assert!(!sliced.fill_indices.is_empty(), "object b fills");
        let max_index = *sliced.fill_indices.iter().max().unwrap();
        assert!(
            (max_index as usize) < sliced.fill_vertices.len(),
            "indices are object-local (0-based), max {max_index} < {} verts",
            sliced.fill_vertices.len()
        );
    }
}
