//! OB-4 object GPU render pipeline (wgpu half).
//!
//! Builds the `wgpu` render pipelines (`ObjectPipeline`) and owns the buffer
//! uploader + render-pass recorder (`ObjectRenderer`) for the OB-3 object model,
//! consuming the device-independent CPU geometry built in
//! [`shape_renderer_core::object_pipeline`]. The CPU vertex/instance layout
//! structs and `build_scene_geometry*` live there and are re-exported below so the
//! sibling `webgpu` modules resolve them through `crate::object_pipeline::*`.

#[cfg(feature = "wgpu-probe")]
use shape_renderer_core::model::CameraState;
#[cfg(feature = "wgpu-probe")]
use shape_renderer_core::object_theme::{resolve_token_f32, Theme};
#[cfg(feature = "wgpu-probe")]
use shape_renderer_core::plan::{
    build_frame_plan, diff_plans, FramePlan, PlanDiff, PlanPatch, StyleSlot,
};
#[cfg(feature = "wgpu-probe")]
use shape_renderer_core::render_object::RenderObjectScene;
#[cfg(feature = "wgpu-probe")]
use shape_renderer_core::text_layout::MsdfAtlasPlan;
#[cfg(feature = "wgpu-probe")]
use crate::shaders::{MSDF_TEXT_WGSL, OBJECT_FILL_WGSL, OBJECT_SHADOW_WGSL, OBJECT_STROKE_WGSL};

// CPU geometry build + layout structs are re-exported from renderer-core so the
// `webgpu` submodules keep resolving them via `crate::object_pipeline::*`.
pub use shape_renderer_core::object_pipeline::*;

// ---------------------------------------------------------------------------
// Pipelines
// ---------------------------------------------------------------------------

/// Fill + stroke render pipelines for the object path, plus the shared camera
/// bind group layout. Built once from a device; the actual draw buffers are
/// owned by [`ObjectRenderer`].
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
    /// Build the object fill and stroke pipelines for the given surface
    /// `format`. The `queue` is unused at construction (kept in the signature to
    /// mirror the legacy `ShapeWebGpuRenderer::create` convention and to leave
    /// room for atlas uploads when MSDF text lands).
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

        // The stroke pipeline also needs binding(1): the dash params uniform,
        // visible to the fragment stage.
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

        // The text pipeline's group(0): b0 view uniform (VERTEX, the shared camera),
        // b1 MSDF atlas texture (FRAGMENT), b2 sampler (FRAGMENT), b3 text params
        // uniform (FRAGMENT, atlas distance_range + dimensions). Matches
        // `msdf_text.wgsl`'s group(0) bindings exactly.
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
            // @location(0) position: vec2<f32>
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
            // @location(1) edge: f32
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
        // matrix columns m0/m1/m2 @2..4 + shadow color @5. Matches
        // `object_shadow.wgsl`'s VertexIn.
        let shadow_vertex_attrs = [
            // @location(0) position: vec2<f32>
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
            // @location(1) feather: f32
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
            // @location(0) position: vec2<f32>
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
            // @location(1) normal: vec2<f32>
            wgpu::VertexAttribute {
                offset: std::mem::size_of::<[f32; 2]>() as u64,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x2,
            },
            // @location(2) side: f32
            wgpu::VertexAttribute {
                offset: (std::mem::size_of::<[f32; 2]>() * 2) as u64,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32,
            },
            // @location(3) width: f32
            wgpu::VertexAttribute {
                offset: (std::mem::size_of::<[f32; 2]>() * 2 + std::mem::size_of::<f32>()) as u64,
                shader_location: 3,
                format: wgpu::VertexFormat::Float32,
            },
            // @location(4) distance_along: f32
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
        // matrix columns m0/m1/m2 @3..5. Matches `msdf_text.wgsl`'s VertexIn.
        let text_vertex_attrs = [
            // @location(0) position: vec2<f32>
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
            // @location(1) uv: vec2<f32>
            wgpu::VertexAttribute {
                offset: std::mem::size_of::<[f32; 2]>() as u64,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x2,
            },
            // @location(2) color: vec4<f32>
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

/// Instance-step vertex attributes for the fill pipeline (`m0`/`m1`/`m2` columns
/// then the inline fill color), packed to match [`FillInstance`].
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

/// Instance-step vertex attributes for the stroke pipeline (`m0`/`m1`/`m2`
/// columns at locations 5..7, stroke color at 8), packed to match
/// [`StrokeInstance`].
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

/// Instance-step vertex attributes for the text pipeline (`m0`/`m1`/`m2` columns
/// at locations 3..5, NO color — color is per-glyph in [`TextVertex`]), packed to
/// match [`TextInstance`] and `msdf_text.wgsl`'s @location(3..5) contract.
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

/// Owns the CPU-built object draw data and the GPU buffers it uploads to, and
/// records the object render pass.
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
    pub msdf_atlas_texture: wgpu::Texture,
    pub text_bind_group: wgpu::BindGroup,
    draws: Vec<ObjectDraw>,
    fill_index_count: u32,
    shadow_vertex_count: u32,
    stroke_vertex_count: u32,
    text_vertex_count: u32,
    /// The retained FramePlan IR — the diffable draw-plan contract this renderer
    /// last uploaded, keyed by per-object [`ResourceHandle`]s. [`apply_plan_diff`]
    /// diffs a freshly-built plan against this one and patches only what changed
    /// (transform/style writes, or a single object's geometry re-send) instead of
    /// reconstructing on every canonical scene re-feed; on a structural change it
    /// signals a rebuild. The GPU buffers above ARE the plan's geometry store, so
    /// `self.plan.geometry` mirrors what is currently on the device.
    ///
    /// [`apply_plan_diff`]: ObjectRenderer::apply_plan_diff
    plan: FramePlan,
    /// RB1: the active light/dark theme. Sources the canvas clear color and is
    /// the bit [`ObjectRenderer::set_theme`] flips. Held so token-backed instance
    /// colors re-resolve on a toggle without re-tessellation (P4).
    theme: Theme,
    /// RA1: per-object live preview WORLD transform (`delta * base`) for objects
    /// under an in-flight drag. Mirrors what `set_preview_transform` wrote to the
    /// GPU instance buffer so the CPU side (selection handles / region bounds) can
    /// track the dragged bbox without re-tessellation. Cleared on commit/snap-back.
    /// A `Vec` (not a hashed map) keeps the pure core free of randomness; at most a
    /// handful of objects are ever previewed at once.
    preview_transforms: Vec<(String, [[f64; 3]; 3])>,
}

#[cfg(feature = "wgpu-probe")]
impl ObjectRenderer {
    /// Build the renderer for a scene: tessellate each object's fill into a
    /// shared megabuffer, expand its stroke into a ribbon, resolve per-object
    /// instance data (matrix columns + paint), and upload all buffers. Records
    /// nothing yet — call [`ObjectRenderer::render`] inside a frame.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &ObjectPipeline,
        scene: &RenderObjectScene,
        pixel_width: f32,
        pixel_height: f32,
        theme: Theme,
    ) -> Self {
        // Build the FramePlan IR (the single tessellation entry) and upload from its
        // geometry store. The plan is retained so a later canonical re-feed diffs
        // against it instead of reconstructing the whole renderer.
        let plan = build_frame_plan(scene, theme);
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

        // Widen the merged megabuffer positions (`[f32;2]`) to the pipeline's
        // `FillVertex` layout (`position` + `edge`). The analytic-AA `edge` helper
        // is the per-vertex silhouette flag built with the mesh topology (1 on the
        // boundary, 0 interior; D4) — `fill_edges` is index-aligned with the
        // megabuffer vertices, so the zip is a pure widening with no per-frame work.
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

        // ---- MSDF text atlas + params + bind group ------------------------
        // The atlas is generated CPU-side by MsdfAtlasPlan (single-channel SDF
        // replicated to RGB). The live atlas is populated from real fontdue glyph
        // coverage at the GPU cutover (alongside the injected measure); here it is
        // an empty plan giving a valid, uploadable RGBA8 texture + distance_range.
        let atlas_plan = MsdfAtlasPlan::new(256, 256, 4.0);
        let msdf_atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shape.ai object msdf atlas"),
            size: wgpu::Extent3d {
                width: atlas_plan.atlas_width,
                height: atlas_plan.atlas_height,
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
                texture: &msdf_atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            atlas_plan.pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(atlas_plan.atlas_width * 4),
                rows_per_image: Some(atlas_plan.atlas_height),
            },
            wgpu::Extent3d {
                width: atlas_plan.atlas_width,
                height: atlas_plan.atlas_height,
                depth_or_array_layers: 1,
            },
        );
        let msdf_atlas_view = msdf_atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let msdf_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shape.ai object msdf sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let text_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shape.ai object text params uniform"),
            size: std::mem::size_of::<TextUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let text_params = TextUniform {
            atlas: [
                atlas_plan.distance_range,
                atlas_plan.atlas_width as f32,
                atlas_plan.atlas_height as f32,
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
                    resource: wgpu::BindingResource::TextureView(&msdf_atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&msdf_sampler),
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
            msdf_atlas_texture,
            text_bind_group,
            fill_index_count: build.fill.indices.len() as u32,
            shadow_vertex_count: build.shadow_vertices.len() as u32,
            stroke_vertex_count: build.stroke_vertices.len() as u32,
            text_vertex_count: build.text_vertices.len() as u32,
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

    /// The retained FramePlan IR — the diffable contract this renderer last
    /// uploaded. Exposed so the canonical re-feed can diff a freshly-built plan
    /// against it (via [`apply_plan_diff`]) and so host tests can inspect the
    /// handle-keyed cache.
    ///
    /// [`apply_plan_diff`]: ObjectRenderer::apply_plan_diff
    pub fn plan(&self) -> &FramePlan {
        &self.plan
    }

    /// IR CONSUMER (the diffable seam): re-feed the canonical object `scene` by
    /// building its [`FramePlan`], diffing it against the retained plan, and
    /// applying the targeted [`PlanPatch`]es through the GPU instance/geometry
    /// write paths — instead of reconstructing the whole renderer on every feed.
    ///
    /// - [`PlanPatch::TransformUpdate`] -> the 4-buffer matrix write (fill/stroke/
    ///   text/shadow), the SAME instance-matrix path the live drag preview uses.
    /// - [`PlanPatch::StyleUpdate`] -> the per-pass color-slot write, the SAME path
    ///   the theme toggle uses.
    /// - [`PlanPatch::GeometryUpdate`] -> a single object's mesh re-send over its
    ///   existing ranges when size-safe; otherwise the whole diff degrades to a
    ///   rebuild (the caller reconstructs).
    /// - [`PlanDiff::Rebuild`] (structural: add/remove/reorder) -> signal a rebuild.
    ///
    /// Returns [`PlanApplyStats`]: `patch_count` is the number of targeted patches
    /// applied this feed; `needs_rebuild` is set when the diff (or an
    /// unfittable geometry update) requires the caller to reconstruct the renderer.
    /// When `needs_rebuild` is true the GPU buffers are left untouched, so the
    /// caller's fresh [`ObjectRenderer::new`] is the single, clean re-upload.
    pub fn apply_plan_diff(
        &mut self,
        queue: &wgpu::Queue,
        scene: &RenderObjectScene,
    ) -> PlanApplyStats {
        let next = build_frame_plan(scene, self.theme);
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

        // A geometry update is only safe in place when the new entry's vertex/index
        // counts still fill the retained ranges (the `follower_patch_plan` guard).
        // If ANY geometry update fails that guard, the buffers can't hold the new
        // mesh, so the whole feed degrades to a rebuild — and we touch nothing,
        // leaving a clean slate for the caller's `ObjectRenderer::new`.
        for patch in &patches {
            if let PlanPatch::GeometryUpdate { index, entry, .. } = patch {
                let old_draw = &self.plan.entries[*index].draw;
                if follower_patch_plan(old_draw, &geometry_reexpand(&next, *index)).is_none() {
                    return PlanApplyStats {
                        patch_count: 0,
                        needs_rebuild: true,
                    };
                }
                // The entry's own ranges must equal the retained ones too (same slot).
                debug_assert_eq!(entry.draw.fill_range, old_draw.fill_range);
            }
        }

        let patch_count = patches.len();
        for patch in &patches {
            self.apply_patch(queue, &next, patch);
        }
        // Adopt the new plan as the retained cache and refresh the mirror state the
        // render loops read (`draws` + counts). Counts are unchanged by transform/
        // style patches and by a size-safe geometry patch, but assigning keeps the
        // mirror exact regardless of which patches ran.
        self.draws = next.geometry.draws.clone();
        self.fill_index_count = next.geometry.fill.indices.len() as u32;
        self.shadow_vertex_count = next.geometry.shadow_vertices.len() as u32;
        self.stroke_vertex_count = next.geometry.stroke_vertices.len() as u32;
        self.text_vertex_count = next.geometry.text_vertices.len() as u32;
        self.plan = next;
        PlanApplyStats {
            patch_count,
            needs_rebuild: false,
        }
    }

    /// Apply one [`PlanPatch`] to the GPU buffers. `next` is the freshly-built plan
    /// the patch came from (the geometry source for a `GeometryUpdate`).
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
                // Re-send ONLY this object's mesh over its existing ranges, then the
                // new matrix + colors (the re-expand carries vertices; the instance
                // attributes ride the same per-object slot).
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
                // diff_plans only returns Rebuild via PlanDiff::Rebuild, handled by the
                // caller before reaching here; a Rebuild inside a patch list is unreachable.
                debug_assert!(false, "PlanPatch::Rebuild should never appear inside a patch list");
            }
        }
    }

    /// Write the 36-byte matrix region (`m0,m1,m2` at offset 0) of object `index`'s
    /// instance in EVERY per-pass instance buffer (fill/stroke/text/shadow), strided
    /// by [`preview_instance_strides`]. This is the absolute-columns twin of
    /// [`set_preview_transform`]'s `delta*base` write — same buffers, same offsets —
    /// so the IR transform patch lands identically to the live drag matrix push.
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

    /// Write the 16-byte color slot (past the 36-byte matrix) of object `index`'s
    /// instance for one pass. The fill/stroke/shadow color sits at struct offset 36;
    /// the matrix region before it is untouched, mirroring [`set_theme`]'s color
    /// write exactly.
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
        // The color slot is at offset 36 in all three instance structs (pinned by the
        // renderer-core layout tests), strictly past the matrix region.
        let offset = (index * stride + 36) as u64;
        queue.write_buffer(buffer, offset, bytemuck::cast_slice(color));
    }

    /// Rebuild the camera uniform from the live camera + device-pixel viewport and
    /// re-upload it (FC-06). Called every frame so pan/zoom moves objects without a
    /// scene reload — the per-object instance matrices stay put while the shared
    /// affine camera in this uniform tracks `self.camera`. Packing is byte-identical
    /// to [`ObjectMatrixUniform::from_scene`].
    pub fn update_camera(
        &self,
        queue: &wgpu::Queue,
        camera: &CameraState,
        pixel_width: f32,
        pixel_height: f32,
    ) {
        let uniform = ObjectMatrixUniform {
            camera: [camera.x as f32, camera.y as f32, camera.zoom as f32, 0.0],
            viewport: [pixel_width, pixel_height, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));
    }

    /// The active light/dark theme.
    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// RB1 ZERO-REBAKE THEME TOGGLE (D1/D2/P4): flip the renderer's theme bit and
    /// re-resolve ONLY the token-backed instance colors, writing the 16-byte color
    /// slot (offset 36, past the `m0,m1,m2` matrix) of each affected
    /// `FillInstance`/`StrokeInstance` — the exact per-instance write path W2-11
    /// uses for the drag matrix. Tessellation (the fill megabuffer, stroke ribbon
    /// vertices, and every `draws` range) is NEVER touched: a theme flip is a
    /// color refresh, not a rebuild. Objects whose paint is raw hex / gradient /
    /// image are theme-invariant and skipped. The canvas clear color tracks
    /// `self.theme` in [`ObjectRenderer::render`], so no buffer write is needed
    /// for the backdrop. No-op (returns the unchanged bit) if `dark` already
    /// matches the current theme.
    pub fn set_theme(&mut self, queue: &wgpu::Queue, dark: bool) -> Theme {
        if self.theme.dark == dark {
            return self.theme;
        }
        self.theme = Theme { dark };
        // RB3: the default drop-shadow color is the `shadow` token, so it flips with
        // the bit too — re-resolve every object's shadow instance color (zero rebake,
        // the geometry is untouched). Sourced from the same token table, never hard-
        // coded.
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
        // Keep the retained FramePlan an accurate diff base: the theme write just
        // moved instance COLORS on the GPU, so mirror them into the plan's entries
        // (index-aligned with `self.draws`). Without this a later `apply_plan_diff`
        // would re-emit redundant style updates against the stale pre-flip colors.
        for (entry, draw) in self.plan.entries.iter_mut().zip(&self.draws) {
            entry.instance.fill = draw.fill_instance;
            entry.instance.stroke = draw.stroke_instance;
            entry.instance.shadow = draw.shadow_instance;
        }
        self.theme
    }

    /// W2-11 drag zero-rebake: write ONLY the dragged object's instance model
    /// matrix to the GPU — no re-tessellation, no scene rebuild (P4). Looks up the
    /// object's instance index `i` in `self.draws` (which is built in the same loop
    /// as all instance buffers, so `draws[i]` ↔ instance `i` in fill, stroke AND
    /// text), composes `delta * base` via [`preview_instance_columns`], and overwrites
    /// the 36-byte matrix region (`m0,m1,m2` at offset 0) of `FillInstance`,
    /// `StrokeInstance`, AND `TextInstance` at offset `i * size_of::<…>()` — so the
    /// glyphs follow the drag too (G3). The baked color sits past byte 36 on fill/
    /// stroke (and per-glyph for text), so it is preserved. Returns false if `id` is
    /// absent.
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
        // The matrix region (`m0,m1,m2` at offset 0) is overwritten in EVERY per-object
        // instance buffer — fill, stroke, text (G3) AND shadow (G5) — so the dragged
        // object's whole visual (region + glyphs + drop shadow) follows the preview in
        // lockstep. Each buffer is index-aligned with `draws`, so the write lands at
        // `i * stride`. `preview_instance_strides` is the single source of truth pairing
        // each buffer with its struct stride; a dropped buffer here is a dropped
        // sub-visual under drag (the G5 bug: shadow lagging at canonical until rebake).
        // Baked color sits past byte 36 (fill/stroke/shadow) or per-glyph (text), so it
        // survives the matrix write.
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
        // RA1: mirror the composed WORLD transform on the CPU side so selection
        // handles / region bounds track the dragged bbox (read-only, no rebake).
        let world = shape_renderer_core::hit_test_object::mat3_mul(delta, base);
        match self.preview_transforms.iter_mut().find(|(pid, _)| pid == id) {
            Some(entry) => entry.1 = world,
            None => self.preview_transforms.push((id.to_string(), world)),
        }
        true
    }

    /// W2-11: revert the dragged object's instance matrix to its canonical baked
    /// transform (`delta = identity`), i.e. drop the live preview. Used by the
    /// shell as a defensive snap-back on commit-failure before the canonical scene
    /// rebake lands. Returns false if `id` is absent.
    pub fn clear_preview_transform(&mut self, queue: &wgpu::Queue, id: &str, base: &[[f64; 3]; 3]) -> bool {
        let written =
            self.set_preview_transform(queue, id, &shape_renderer_core::hit_test_object::identity_3x3(), base);
        // RA1: drop the CPU preview so handles fall back to the canonical region.
        self.preview_transforms.retain(|(pid, _)| pid != id);
        written
    }

    /// W3-G9/#4 LIVE anchor reproject: patch a follower's baked vertices in place so
    /// its anchored node tracks a moved target DURING the drag (the follower is NOT
    /// uniformly transformed — one node moves, so its shape changes and the
    /// instance-matrix preview path cannot express it). `rebuilt` is the follower
    /// re-expanded with the reprojected node ([`reexpand_single_object`]); this
    /// writes its fill vertices, fill indices (rebased into the megabuffer), stroke
    /// ribbon vertices and — W3-G13 — its drop-shadow silhouette + glyph quads over
    /// the follower's EXISTING ranges — zero full rebake, O(one small object).
    ///
    /// DEFENSIVE (GPU-blind): a vertex/index COUNT that no longer matches the baked
    /// range (a topology/LOD edge case) makes [`follower_patch_plan`] return `None`,
    /// and this SKIPS the write entirely — never a partial/mismatched range that
    /// would corrupt the buffer or bleed into a neighbour. Returns false if `id` is
    /// absent or the patch was skipped.
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
        // within `[start, end)` of the shared fill vertex buffer.
        if !rebuilt.fill_vertices.is_empty() {
            queue.write_buffer(
                &self.fill_vertex_buffer,
                plan.fill_vertex_byte_offset,
                bytemuck::cast_slice(&rebuilt.fill_vertices),
            );
            // Re-emit the indices rebased to the follower's vertex base. The count is
            // guarded equal, so a re-tessellation that kept the count but changed the
            // index pattern is still corrected (not just the positions).
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
        // W3-G13: drop-shadow silhouette vertices (fill-derived, or stroke-ribbon-
        // derived for fill-less open strokes per W3-G10/#3) — without this write the
        // follower's shadow stayed at the OLD geometry until the commit rebake.
        if !rebuilt.shadow_vertices.is_empty() {
            queue.write_buffer(
                &self.shadow_vertex_buffer,
                plan.shadow_vertex_byte_offset,
                bytemuck::cast_slice(&rebuilt.shadow_vertices),
            );
        }
        // W3-G13: glyph quads — text layout depends on the region bbox, so a
        // reprojected node moves the glyphs too.
        if !rebuilt.text_vertices.is_empty() {
            queue.write_buffer(
                &self.text_vertex_buffer,
                plan.text_vertex_byte_offset,
                bytemuck::cast_slice(&rebuilt.text_vertices),
            );
        }
        true
    }

    /// RA1: the live preview WORLD transform (`delta * base`) for `id`, or `None`
    /// when the object has no in-flight drag preview. The composed transform is the
    /// one `set_preview_transform` pushed to the instance buffer, so a caller can
    /// substitute it for the canonical `region.transform` to lay out selection
    /// handles / region bounds against the PREVIEWED bbox during a drag — a pure
    /// transform read, no re-tessellation.
    pub fn preview_transform(&self, id: &str) -> Option<[[f64; 3]; 3]> {
        self.preview_transforms
            .iter()
            .find(|(pid, _)| pid == id)
            .map(|(_, world)| *world)
    }

    /// Record the object draw pass into `encoder` targeting `view`. Each object
    /// is one instanced indexed fill draw (its megabuffer range against its
    /// instance) followed by one instanced stroke draw. `clear` chooses whether
    /// the pass clears the color attachment first (the object path owns the whole
    /// surface at the cutover).
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        pipeline: &ObjectPipeline,
        clear: bool,
    ) {
        let load = if clear {
            // RB1: the canvas backdrop is the `canvas-bg` token in the active
            // theme — flips light/dark with `self.theme`, no buffer write needed.
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

        // W3-G8/A: the drop shadow is NO LONGER drawn here. It is rendered once to an
        // offscreen mask ([`render_shadow_mask`]), separable-Gaussian-blurred, and
        // composited UNDER the fill in the visible pass before this `render` runs
        // (see `frame.rs`). This pass now starts with the fill so the blurred shadow
        // it composited stays beneath fill/stroke/text.

        // Fill pass: one indexed instanced draw per object, all sharing the merged
        // megabuffer vertex/index buffers and the per-object instance buffer.
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
                let instance = instance as u32;
                pass.draw_indexed(
                    draw.fill_range.start..draw.fill_range.end,
                    0,
                    instance..instance + 1,
                );
            }
        }

        // Stroke pass: one instanced draw per object over its ribbon vertex range.
        if self.stroke_vertex_count > 0 {
            pass.set_pipeline(&pipeline.stroke_pipeline);
            pass.set_bind_group(0, &self.stroke_bind_group, &[]);
            pass.set_vertex_buffer(0, self.stroke_vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, self.stroke_instance_buffer.slice(..));
            for (instance, draw) in self.draws.iter().enumerate() {
                if draw.stroke_range.is_empty() {
                    continue;
                }
                let instance = instance as u32;
                pass.draw(
                    draw.stroke_range.start..draw.stroke_range.end,
                    instance..instance + 1,
                );
            }
        }

        // Text pass: one instanced draw per object over its glyph-quad range, drawn
        // OVER fill+stroke (the pass already loads, never clears, between sub-passes).
        // Each object's glyphs ride its own per-object matrix instance (index `i`),
        // index-aligned with `draws` exactly like the fill/stroke loops.
        if self.text_vertex_count > 0 {
            pass.set_pipeline(&pipeline.text_pipeline);
            pass.set_bind_group(0, &self.text_bind_group, &[]);
            pass.set_vertex_buffer(0, self.text_vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, self.text_instance_buffer.slice(..));
            for (instance, draw) in self.draws.iter().enumerate() {
                if draw.text_range.is_empty() {
                    continue;
                }
                let instance = instance as u32;
                pass.draw(
                    draw.text_range.start..draw.text_range.end,
                    instance..instance + 1,
                );
            }
        }
    }

    /// W3-G8/A: record the drop-shadow silhouette ONLY into an offscreen `mask`
    /// view, clearing it to transparent first. This is the input to the separable
    /// Gaussian blur ([`crate::shadow_blur::ShadowBlur`]). Each object's shadow
    /// rides its own per-object matrix instance (index `i`), index-aligned with
    /// `draws` exactly like the fill/stroke loops, so the projective transform path
    /// places the silhouette identically to the fill. Records nothing (a single
    /// transparent clear) when no object casts a shadow, so the blurred mask stays
    /// empty and the composite is invisible.
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
            let instance = instance as u32;
            pass.draw(
                draw.shadow_range.start..draw.shadow_range.end,
                instance..instance + 1,
            );
        }
    }

    /// Number of fill indices uploaded for the loaded scene (diagnostics).
    pub fn fill_index_count(&self) -> u32 {
        self.fill_index_count
    }

    /// Number of drop-shadow quad vertices uploaded for the loaded scene (diagnostics).
    pub fn shadow_vertex_count(&self) -> u32 {
        self.shadow_vertex_count
    }

    /// Number of stroke ribbon vertices uploaded for the loaded scene (diagnostics).
    pub fn stroke_vertex_count(&self) -> u32 {
        self.stroke_vertex_count
    }

    /// Number of text glyph-quad vertices uploaded for the loaded scene (diagnostics).
    pub fn text_vertex_count(&self) -> u32 {
        self.text_vertex_count
    }
}

/// The result of an [`ObjectRenderer::apply_plan_diff`] feed, surfaced so the
/// frame-stats keep their meaning: `patch_count` is the number of targeted IR
/// patches applied (the "patch path" counter), and `needs_rebuild` tells the
/// caller the diff required a full reconstruction (the "rebuild" counter).
#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanApplyStats {
    pub patch_count: usize,
    pub needs_rebuild: bool,
}

/// Slice object `index`'s baked geometry out of a built [`FramePlan`] into a
/// [`FollowerReexpand`] — the per-object vertex/index payload the in-place patch
/// path (`patch_follower_geometry`) consumes. The fill indices in the plan's
/// megabuffer are rebased by the object's vertex base; this rebases them back to
/// object-LOCAL (0-based) so the patch can re-rebase them onto the live ranges
/// (matching `reexpand_single_object`'s object-local contract).
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
/// scene still produces a valid, non-zero-sized buffer handle).
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

    /// The fill/stroke instance attribute offsets tile each instance record with no
    /// gaps/overlap and at the shader-declared locations (fill `m0..m2`+`fill` at
    /// 2..5, stroke `m0..m2`+`stroke` at 5..8). FAILS if the `wgpu::VertexAttribute`
    /// packing drifts from the per-object matrix+color layout the WGSL expects.
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

    /// COMMIT B (packing vs shader): the text instance attribute offsets/locations
    /// match msdf_text.wgsl's @location(3..5) instance-step matrix columns —
    /// m0@offset0/loc3, m1@12/loc4, m2@24/loc5. FAILS if the instance packing drifts
    /// from the shader's expected per-object matrix layout.
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
        // TextVertex attribute formats: position Float32x2 @0, uv Float32x2 @8,
        // color Float32x4 @16 — pinned via byte offsets implicit in the 32B size.
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

    /// IR GEOMETRY-UPDATE PARITY (host, no GPU): `geometry_reexpand` slices an
    /// object out of a built `FramePlan` into the EXACT bytes
    /// `reexpand_single_object` produces for that object — the same payload the
    /// in-place `patch_follower_geometry` write consumes. Pins that the IR's
    /// geometry-update path re-sends correct, object-local geometry (fill vertices,
    /// object-LOCAL rebased indices, stroke/shadow/text vertices) for a NON-first
    /// object, where the megabuffer index rebase is load-bearing. FAILS if the slice
    /// math or the index un-rebase drifts.
    #[test]
    fn geometry_reexpand_slices_match_reexpand_single_object() {
        // Two objects so object index 1 has a non-zero megabuffer vertex base — the
        // case where slicing + un-rebasing the indices actually matters.
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

        // The sliced geometry, paired with the plan's draw record, is size-safe to
        // patch in place (the contract `apply_plan_diff`'s geometry route relies on).
        let plan_b = &plan.entries[1].draw;
        assert!(
            follower_patch_plan(plan_b, &geometry_reexpand(&plan, 1)).is_some(),
            "a plan slice fills its own object's ranges exactly"
        );
    }

    /// IR INDEX UN-REBASE: object 1's indices in the merged megabuffer are rebased
    /// by its vertex base, so a naive slice would carry merged (too-large) indices.
    /// `geometry_reexpand` must return them object-LOCAL (0-based), so the max index
    /// is below the object's own vertex count. FAILS if the un-rebase is dropped (the
    /// patch would then write merged indices over a 0-based range — corruption).
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
