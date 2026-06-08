//! OB-4 object GPU render pipeline (additive cutover groundwork).
//!
//! This module builds the *real* `wgpu` render pipelines for the OB-3 object
//! model (`RenderObjectScene` / `RenderObject`) on top of the WGSL in
//! [`crate::shaders`]. It is **additive**: it does not touch the legacy
//! `RenderGroup`/`RenderCard`/`RenderEdge` draw path, `ViewUniform`, or
//! `load_scene` in [`crate::webgpu`]. The client flips from the legacy 2D path to
//! this object path at the full OB-4 cutover; until then both compile side by
//! side under the crate's default `wgpu-probe` feature.
//!
//! ## What is compile-verified vs GPU-runtime-deferred
//!
//! There is no GPU device in the test environment, so only construction is
//! exercised here: pipeline/layout/buffer descriptors, vertex layouts, uniform
//! byte packing, and CPU-side scene tessellation into a [`MegaBuffer`] + instance
//! data. The actual `device.create_*` / `queue.submit` calls and the recorded
//! render pass are written and **compile-checked** (they build under
//! `wgpu-probe`), but only *run* on a browser/native device at the cutover. The
//! `#[cfg(test)]` unit tests cover everything that does not need a device.
//!
//! ## Per-object 3x3 matrix strategy: instance attributes
//!
//! `wgpu`'s WebGPU backend exposes no push constants, so the per-object 3x3
//! projective transform (D7) cannot be a push constant. The WGSL contract (see
//! [`crate::shaders`]) declares the matrix as three **instance-step** `vec3`
//! columns (`m0`/`m1`/`m2`) plus the inline paint color, so we match that exactly
//! with a per-object instance buffer rather than a per-draw uniform: one instance
//! record per object carries its matrix columns + color, and each object draws as
//! a single instance (`draw_indexed(range, base_vertex, instance..instance+1)`).
//! This keeps the matrix out of the shared `view` uniform — which stays the
//! affine camera, byte-identical to the legacy `ViewUniform` — and lets the VS do
//! the projective `M * vec3(local, 1)` divide per object.

// The CPU geometry build (`build_scene_geometry` + the `bytemuck` vertex/instance
// structs) compiles for any target — it needs no `wgpu`. Only the GPU pipeline /
// uploader items (`ObjectPipeline`, `ObjectRenderer`, and their wgpu helpers) are
// gated behind `wgpu-probe` (the feature that pulls in `wgpu`), so the object
// render model + geometry build the web wasm exports stay available without it.

use crate::render_object::{resolve_visual, RPaint, RenderObject, RenderObjectScene, VisualState};
#[cfg(feature = "wgpu-probe")]
use crate::model::CameraState;
#[cfg(feature = "wgpu-probe")]
use crate::shaders::{OBJECT_FILL_WGSL, OBJECT_STROKE_WGSL};
use crate::stroke_expand::{dash_segments, expand_stroke, Cap, Join};
use crate::tessellate::{
    parse_path, quantized_to_px, tessellate_fill, DrawRange, FillRuleKind, MegaBuffer,
    PathCommand,
};

// ---------------------------------------------------------------------------
// Uniform + vertex/instance GPU layouts
// ---------------------------------------------------------------------------

/// Shared camera uniform for the object pipelines. Replaces `ViewUniform`'s
/// affine-only role for the object path: the same affine `camera`/`viewport`
/// fields the WGSL `View` struct reads, while every object's *projective* 3x3
/// matrix rides on the instance buffer (see module docs), not here. Byte-layout
/// identical to the legacy `ViewUniform` so the two pipelines share a coordinate
/// frame during the cutover.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ObjectMatrixUniform {
    /// `vec4(translate.x, translate.y, zoom, _)` — the affine camera.
    pub camera: [f32; 4],
    /// `vec4(px_w, px_h, _, _)` — the device-pixel viewport.
    pub viewport: [f32; 4],
}

impl ObjectMatrixUniform {
    /// Build the camera uniform from a scene's camera and a device-pixel viewport.
    pub fn from_scene(scene: &RenderObjectScene, pixel_width: f32, pixel_height: f32) -> Self {
        ObjectMatrixUniform {
            camera: [
                scene.camera.x as f32,
                scene.camera.y as f32,
                scene.camera.zoom as f32,
                0.0,
            ],
            viewport: [pixel_width, pixel_height, 0.0, 0.0],
        }
    }
}

/// Per-vertex fill attributes matching `object_fill.wgsl`'s `VertexIn`
/// (`position` @0, `edge` @1). Positions are object-local **pixels**
/// (`tessellate::Mesh` vertices); `edge` is the analytic-AA silhouette helper
/// (0 at interior fans for the first cutover — lyon does not emit a silhouette
/// flag, so the FS coverage term degrades gracefully to opaque interior).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FillVertex {
    pub position: [f32; 2],
    pub edge: f32,
}

/// Per-object instance attributes for the fill pipeline matching
/// `object_fill.wgsl` (`m0`/`m1`/`m2` @2..4, `fill` @5). The 3x3 projective
/// matrix is stored as three `vec3` columns; `fill` is the resolved solid paint.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FillInstance {
    pub m0: [f32; 3],
    pub m1: [f32; 3],
    pub m2: [f32; 3],
    pub fill: [f32; 4],
}

/// Per-vertex stroke attributes matching `object_stroke.wgsl`'s `VertexIn`
/// (`position` @0, `normal` @1, `side` @2, `width` @3, `distance_along` @4).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct StrokeVertex {
    pub position: [f32; 2],
    pub normal: [f32; 2],
    pub side: f32,
    pub width: f32,
    pub distance_along: f32,
}

/// Per-object instance attributes for the stroke pipeline matching
/// `object_stroke.wgsl` (`m0`/`m1`/`m2` @5..7, `stroke` @8).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct StrokeInstance {
    pub m0: [f32; 3],
    pub m1: [f32; 3],
    pub m2: [f32; 3],
    pub stroke: [f32; 4],
}

/// `object_stroke.wgsl`'s `StrokeUniform` (binding 1): dash on-length, period,
/// opacity, and a solid/dashed flag.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct StrokeParamsUniform {
    /// `vec4(dash_on_px, dash_period_px, opacity, dashed_flag)`.
    pub dash: [f32; 4],
}

impl StrokeParamsUniform {
    /// Solid (no-dash) params at full opacity. Per-object dash gating is folded
    /// into the CPU dash split for the first cutover, so the shader-side uniform
    /// stays solid; richer per-instance dashing gets its own buffer later.
    pub fn solid() -> Self {
        StrokeParamsUniform {
            dash: [0.0, 0.0, 1.0, 0.0],
        }
    }
}

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
    pub fill_pipeline: wgpu::RenderPipeline,
    pub stroke_pipeline: wgpu::RenderPipeline,
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
        let stroke_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shape.ai object stroke shader"),
            source: wgpu::ShaderSource::Wgsl(OBJECT_STROKE_WGSL.into()),
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

        ObjectPipeline {
            camera_bind_group_layout,
            stroke_bind_group_layout,
            fill_pipeline,
            stroke_pipeline,
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

// ---------------------------------------------------------------------------
// CPU scene build
// ---------------------------------------------------------------------------

/// The CPU-built draw data for one object: where its fill triangles live in the
/// megabuffer and the instance record (matrix + color) drawn against them, plus
/// its stroke vertices and stroke instance. Held by [`ObjectRenderer`] so each
/// object becomes one indexed fill draw + one stroke draw at record time.
#[derive(Clone, Debug)]
pub struct ObjectDraw {
    pub id: String,
    /// Index range into the shared fill megabuffer (`fill_indices`), or an empty
    /// range when the object has no fillable region.
    pub fill_range: DrawRange,
    pub fill_instance: FillInstance,
    /// Stroke ribbon vertices for this object (own buffer slice via `stroke_range`).
    pub stroke_range: DrawRange,
    pub stroke_instance: StrokeInstance,
    /// Whether a focus ring should be drawn for this object (selection/focus).
    pub focus_ring: bool,
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
    pub stroke_vertex_buffer: wgpu::Buffer,
    pub stroke_instance_buffer: wgpu::Buffer,
    draws: Vec<ObjectDraw>,
    fill_index_count: u32,
    stroke_vertex_count: u32,
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
    ) -> Self {
        let build = build_scene_geometry(scene);

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
        // is 0 for the first cutover: lyon does not flag silhouette vertices, so
        // the FS coverage term reduces to opaque interior fill.
        let fill_vertices: Vec<FillVertex> = build
            .fill
            .vertices
            .iter()
            .map(|&position| FillVertex { position, edge: 0.0 })
            .collect();

        let fill_vertex_buffer = create_vertex_buffer(device, "object fill vertices", &fill_vertices);
        let fill_index_buffer = create_index_buffer(device, "object fill indices", &build.fill.indices);
        let fill_instance_buffer =
            create_vertex_buffer(device, "object fill instances", &build.fill_instances);
        let stroke_vertex_buffer =
            create_vertex_buffer(device, "object stroke vertices", &build.stroke_vertices);
        let stroke_instance_buffer =
            create_vertex_buffer(device, "object stroke instances", &build.stroke_instances);

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

        ObjectRenderer {
            uniform_buffer,
            stroke_params_buffer,
            camera_bind_group,
            stroke_bind_group,
            fill_vertex_buffer,
            fill_index_buffer,
            fill_instance_buffer,
            stroke_vertex_buffer,
            stroke_instance_buffer,
            fill_index_count: build.fill.indices.len() as u32,
            stroke_vertex_count: build.stroke_vertices.len() as u32,
            draws: build.draws,
        }
    }

    /// Number of objects with recorded draw data.
    pub fn object_count(&self) -> usize {
        self.draws.len()
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

    /// W2-11 drag zero-rebake: write ONLY the dragged object's instance model
    /// matrix to the GPU — no re-tessellation, no scene rebuild (P4). Looks up the
    /// object's instance index `i` in `self.draws` (which is built in the same loop
    /// as both instance buffers, so `draws[i]` ↔ instance `i` in fill AND stroke),
    /// composes `delta * base` via [`preview_instance_columns`], and overwrites the
    /// 36-byte matrix region (`m0,m1,m2` at offset 0) of BOTH `FillInstance` and
    /// `StrokeInstance` at offset `i * size_of::<…>()`. The baked color sits past
    /// byte 36, so it is preserved. Returns false if `id` is absent.
    pub fn set_preview_transform(
        &self,
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
        let fill_offset = (i * std::mem::size_of::<FillInstance>()) as u64;
        let stroke_offset = (i * std::mem::size_of::<StrokeInstance>()) as u64;
        queue.write_buffer(&self.fill_instance_buffer, fill_offset, bytes);
        queue.write_buffer(&self.stroke_instance_buffer, stroke_offset, bytes);
        true
    }

    /// W2-11: revert the dragged object's instance matrix to its canonical baked
    /// transform (`delta = identity`), i.e. drop the live preview. Used by the
    /// shell as a defensive snap-back on commit-failure before the canonical scene
    /// rebake lands. Returns false if `id` is absent.
    pub fn clear_preview_transform(&self, queue: &wgpu::Queue, id: &str, base: &[[f64; 3]; 3]) -> bool {
        self.set_preview_transform(queue, id, &crate::hit_test_object::identity_3x3(), base)
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
            wgpu::LoadOp::Clear(wgpu::Color {
                r: 0.972,
                g: 0.982,
                b: 0.992,
                a: 1.0,
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
    }

    /// Number of fill indices uploaded for the loaded scene (diagnostics).
    pub fn fill_index_count(&self) -> u32 {
        self.fill_index_count
    }

    /// Number of stroke ribbon vertices uploaded for the loaded scene (diagnostics).
    pub fn stroke_vertex_count(&self) -> u32 {
        self.stroke_vertex_count
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

// ---------------------------------------------------------------------------
// CPU geometry build (device-independent, unit-testable)
// ---------------------------------------------------------------------------

/// The device-independent result of building a scene's GPU geometry: a merged
/// fill megabuffer + per-object fill instances, and a flat stroke ribbon vertex
/// array + per-object stroke instances, plus the per-object [`ObjectDraw`] index.
#[derive(Clone, Debug, Default)]
pub struct SceneGeometry {
    pub fill: MegaBuffer,
    pub fill_instances: Vec<FillInstance>,
    pub stroke_vertices: Vec<StrokeVertex>,
    pub stroke_instances: Vec<StrokeInstance>,
    pub draws: Vec<ObjectDraw>,
}

/// Build all CPU geometry for `scene` (no device needed): for each object,
/// tessellate its fill into the shared megabuffer, expand its stroke into a
/// ribbon, and resolve its instance data (3x3 matrix columns + paint color).
/// This is the unit-testable core of [`ObjectRenderer::new`].
pub fn build_scene_geometry(scene: &RenderObjectScene) -> SceneGeometry {
    let mut geometry = SceneGeometry::default();

    for obj in &scene.objects {
        let state = visual_state_for(scene, &obj.id);
        let resolved = resolve_visual(obj, state);

        let subpaths = flatten_object_subpaths(obj, scene.camera.zoom);

        // ---- Fill: tessellate the closed region into the megabuffer --------
        let fill_input: Vec<(bool, Vec<(f32, f32)>)> = subpaths
            .iter()
            .map(|(closed, pts)| (*closed, pts.clone()))
            .collect();
        let mesh = tessellate_fill(&fill_input, FillRuleKind::NonZero);
        let fill_range = geometry.fill.push(&mesh);
        geometry.fill_instances.push(FillInstance {
            m0: matrix_col(&obj.transform, 0),
            m1: matrix_col(&obj.transform, 1),
            m2: matrix_col(&obj.transform, 2),
            fill: paint_color(&resolved.fill.paint, resolved.fill.opacity as f32),
        });

        // ---- Stroke: expand each (dashed) subpath into a ribbon ------------
        let stroke_start = geometry.stroke_vertices.len() as u32;
        let cap = match resolved.stroke.cap {
            crate::render_object::RStrokeCap::Butt => Cap::Butt,
            crate::render_object::RStrokeCap::Round => Cap::Round,
            crate::render_object::RStrokeCap::Square => Cap::Square,
        };
        let join = match resolved.stroke.join {
            crate::render_object::RStrokeJoin::Miter => Join::Miter,
            crate::render_object::RStrokeJoin::Bevel => Join::Bevel,
            // Round joins are not yet expanded by the CPU reference; miter is the
            // closest existing geometry until a round-join arc is added.
            crate::render_object::RStrokeJoin::Round => Join::Miter,
        };
        let width = resolved.stroke.width as f32;
        for (closed, pts) in &subpaths {
            let runs = dash_segments(pts, &dash_px(&resolved.stroke.dash));
            for run in runs {
                let stroke_mesh = expand_stroke(&run, *closed, width, None, cap, join);
                append_stroke_ribbon(&mut geometry.stroke_vertices, &stroke_mesh, width);
            }
        }
        let stroke_end = geometry.stroke_vertices.len() as u32;
        geometry.stroke_instances.push(StrokeInstance {
            m0: matrix_col(&obj.transform, 0),
            m1: matrix_col(&obj.transform, 1),
            m2: matrix_col(&obj.transform, 2),
            stroke: paint_color(&resolved.stroke.paint, resolved.stroke.opacity as f32),
        });

        geometry.draws.push(ObjectDraw {
            id: obj.id.clone(),
            fill_range,
            fill_instance: *geometry
                .fill_instances
                .last()
                .expect("fill instance just pushed"),
            stroke_range: DrawRange {
                start: stroke_start,
                end: stroke_end,
            },
            stroke_instance: *geometry
                .stroke_instances
                .last()
                .expect("stroke instance just pushed"),
            focus_ring: resolved.focus_ring.is_some(),
        });
    }

    geometry
}

/// Resolve the visual state (selected via single anchor or transient
/// multi-select) for an object id, so `resolve_visual` adds a focus ring for the
/// selection set.
fn visual_state_for(scene: &RenderObjectScene, id: &str) -> VisualState {
    let selected = scene.selection.as_deref() == Some(id)
        || scene.multi_select.iter().any(|candidate| candidate == id);
    VisualState {
        selected,
        hovered: false,
        focused: false,
    }
}

/// Parse an object's geometry and flatten each subpath into an object-local
/// **pixel** polyline, flattening cubics via the zoom-bucket LOD flattener.
/// Returns `(closed, points)` per subpath.
fn flatten_object_subpaths(obj: &RenderObject, zoom: f64) -> Vec<(bool, Vec<(f32, f32)>)> {
    let bucket = crate::curve_lod::zoom_bucket(zoom);
    let flatness = crate::curve_lod::flatness_for_bucket(bucket);
    let parsed = parse_path(&obj.geometry_d);
    let mut out = Vec::with_capacity(parsed.len());
    for sub in &parsed {
        let mut pts: Vec<(f32, f32)> = Vec::new();
        let mut cursor = (0.0f32, 0.0f32);
        for cmd in &sub.commands {
            match *cmd {
                PathCommand::MoveTo { x, y } => {
                    cursor = (quantized_to_px(x), quantized_to_px(y));
                    pts.push(cursor);
                }
                PathCommand::LineTo { x, y } => {
                    cursor = (quantized_to_px(x), quantized_to_px(y));
                    pts.push(cursor);
                }
                PathCommand::Cubic {
                    c1x,
                    c1y,
                    c2x,
                    c2y,
                    x,
                    y,
                } => {
                    let c1 = (quantized_to_px(c1x), quantized_to_px(c1y));
                    let c2 = (quantized_to_px(c2x), quantized_to_px(c2y));
                    let end = (quantized_to_px(x), quantized_to_px(y));
                    let flat = crate::curve_lod::flatten_cubic(cursor, c1, c2, end, flatness);
                    // `flatten_cubic` includes both endpoints; the start duplicates
                    // the cursor already pushed, so skip it.
                    for &p in flat.iter().skip(1) {
                        pts.push(p);
                    }
                    cursor = end;
                }
                PathCommand::Close => {}
            }
        }
        out.push((sub.closed, pts));
    }
    out
}

/// Append a stroke ribbon `mesh` (triangle-list of `[x,y]` positions) as
/// expanded [`StrokeVertex`]es. The CPU ribbon already bakes the normal offset
/// into the positions, so the GPU stroke shader must not offset again: we emit a
/// zero normal and `side = 0`, carrying only the pre-offset position, with the
/// per-node `width` and a running arc-length `distance_along`.
fn append_stroke_ribbon(out: &mut Vec<StrokeVertex>, mesh: &crate::stroke_expand::Mesh, width: f32) {
    let mut distance = 0.0f32;
    let mut prev: Option<[f32; 2]> = None;
    for &index in &mesh.indices {
        let position = mesh.vertices[index as usize];
        if let Some(p) = prev {
            let dx = position[0] - p[0];
            let dy = position[1] - p[1];
            distance += (dx * dx + dy * dy).sqrt();
        }
        prev = Some(position);
        out.push(StrokeVertex {
            position,
            normal: [0.0, 0.0],
            side: 0.0,
            width,
            distance_along: distance,
        });
    }
}

/// Extract column `col` of a row-major 3x3 transform as a `vec3` for the
/// column-major `mat3x3` reconstruction in the WGSL (`M = [m0 | m1 | m2]`).
fn matrix_col(transform: &[[f64; 3]; 3], col: usize) -> [f32; 3] {
    [
        transform[0][col] as f32,
        transform[1][col] as f32,
        transform[2][col] as f32,
    ]
}

/// W2-11 drag zero-rebake: compose the preview world matrix `delta * base` and
/// return its three instance columns in the exact same packing
/// [`build_scene_geometry`] uses for [`FillInstance`]/[`StrokeInstance`]
/// (`M = [m0 | m1 | m2]`). This is the single source of truth for the preview
/// matrix: the GPU writer ([`ObjectRenderer::set_preview_transform`]) and the
/// perf-gate test both call it, so the live-drag instance push is byte-equivalent
/// to a full rebake of the same object at `compose(delta, base)`. Pure and
/// device-free so it runs under `cargo test --workspace` too.
pub fn preview_instance_columns(
    delta: &[[f64; 3]; 3],
    base: &[[f64; 3]; 3],
) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let world = crate::hit_test_object::mat3_mul(delta, base);
    (
        matrix_col(&world, 0),
        matrix_col(&world, 1),
        matrix_col(&world, 2),
    )
}

/// Resolve a paint to a single RGBA color for the inline-solid first cutover.
/// Gradient/image paints collapse to their representative color (first stop /
/// neutral) here; richer paints get their own bind group later (D7 note).
fn paint_color(paint: &RPaint, opacity: f32) -> [f32; 4] {
    let rgb = match paint {
        RPaint::Solid { color } => parse_hex_rgb(color),
        RPaint::Gradient { stops, .. } => stops
            .first()
            .map(|stop| parse_hex_rgb(&stop.color))
            .unwrap_or([1.0, 1.0, 1.0]),
        RPaint::Image { .. } => [1.0, 1.0, 1.0],
    };
    [rgb[0], rgb[1], rgb[2], opacity.clamp(0.0, 1.0)]
}

/// Parse a `#rrggbb` hex color into linear-ish 0..1 RGB. A malformed value falls
/// back to white so a bad color never poisons the draw.
fn parse_hex_rgb(value: &str) -> [f32; 3] {
    let parse = || -> Option<[f32; 3]> {
        let hex = value.strip_prefix('#')?;
        if hex.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0])
    };
    parse().unwrap_or([1.0, 1.0, 1.0])
}

/// Convert a stroke dash pattern (px lengths) for the CPU dash split. An empty
/// pattern stays empty (solid).
fn dash_px(dash: &[f64]) -> Vec<f32> {
    dash.iter().map(|&d| d as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CameraState;
    use crate::render_object::{RFill, RPaint, RStroke, RStrokeCap, RStrokeJoin};

    fn identity() -> [[f64; 3]; 3] {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    }

    fn rect_object(id: &str) -> RenderObject {
        RenderObject {
            id: id.to_string(),
            parent: None,
            order: "a0".to_string(),
            transform: identity(),
            // 100px square at 8 units/px = 800 quantized units.
            geometry_d: "M0 0 L800 0 L800 800 L0 800 Z".to_string(),
            fill: Some(RFill {
                paint: RPaint::Solid {
                    color: "#ff0000".to_string(),
                },
                opacity: 1.0,
            }),
            stroke: Some(RStroke {
                paint: RPaint::Solid {
                    color: "#00ff00".to_string(),
                },
                width: 4.0,
                opacity: 1.0,
                dash: Vec::new(),
                cap: RStrokeCap::Butt,
                join: RStrokeJoin::Miter,
            }),
            text: None,
            clip: false,
        }
    }

    fn scene_with(objects: Vec<RenderObject>, selection: Option<String>) -> RenderObjectScene {
        RenderObjectScene {
            scene_id: "test".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            objects,
            selection,
            multi_select: Vec::new(),
        }
    }

    #[test]
    fn matrix_uniform_packs_to_32_bytes_camera_then_viewport() {
        let scene = scene_with(vec![rect_object("o1")], None);
        let uniform = ObjectMatrixUniform::from_scene(&scene, 1280.0, 720.0);
        let bytes = bytemuck::bytes_of(&uniform);
        // Two vec4<f32> = 8 floats = 32 bytes, camera first then viewport.
        assert_eq!(bytes.len(), 32);
        assert_eq!(std::mem::size_of::<ObjectMatrixUniform>(), 32);
        assert_eq!(uniform.camera, [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(uniform.viewport, [1280.0, 720.0, 0.0, 0.0]);
        // Byte layout matches the legacy ViewUniform (camera xyzw then viewport).
        let floats: &[f32] = bytemuck::cast_slice(bytes);
        assert_eq!(floats, &[0.0, 0.0, 1.0, 0.0, 1280.0, 720.0, 0.0, 0.0]);
    }

    #[test]
    fn vertex_and_instance_layout_sizes_match_shader_contract() {
        // FillVertex: vec2 position + f32 edge = 3 floats = 12 bytes.
        assert_eq!(std::mem::size_of::<FillVertex>(), 12);
        // FillInstance: 3 vec3 columns + vec4 fill = 9 + 4 = 13 floats = 52 bytes.
        assert_eq!(std::mem::size_of::<FillInstance>(), 52);
        // StrokeVertex: position(2) + normal(2) + side + width + distance = 7
        // floats = 28 bytes.
        assert_eq!(std::mem::size_of::<StrokeVertex>(), 28);
        // StrokeInstance: 3 vec3 + vec4 = 52 bytes (same as fill instance).
        assert_eq!(std::mem::size_of::<StrokeInstance>(), 52);
        // StrokeParamsUniform: one vec4 = 16 bytes.
        assert_eq!(std::mem::size_of::<StrokeParamsUniform>(), 16);

        // Instance attribute offsets tile the record with no gaps/overlap.
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
    fn build_scene_geometry_tessellates_fill_and_expands_stroke() {
        let scene = scene_with(vec![rect_object("o1")], None);
        let geo = build_scene_geometry(&scene);

        assert_eq!(geo.draws.len(), 1);
        assert_eq!(geo.fill_instances.len(), 1);
        assert_eq!(geo.stroke_instances.len(), 1);

        let draw = &geo.draws[0];
        assert_eq!(draw.id, "o1");
        // The rect fill tessellates to a non-empty index range in the megabuffer.
        assert!(!draw.fill_range.is_empty(), "rect fill must tessellate");
        assert_eq!(draw.fill_range.start, 0);
        assert_eq!(draw.fill_range.end, geo.fill.indices.len() as u32);
        // The closed square stroke expands to a non-empty ribbon.
        assert!(!draw.stroke_range.is_empty(), "rect border must expand");
        assert_eq!(draw.stroke_range.end as usize, geo.stroke_vertices.len());
        // Unselected object: no focus ring.
        assert!(!draw.focus_ring);

        // Identity transform -> m0/m1/m2 are the identity columns.
        assert_eq!(geo.fill_instances[0].m0, [1.0, 0.0, 0.0]);
        assert_eq!(geo.fill_instances[0].m1, [0.0, 1.0, 0.0]);
        assert_eq!(geo.fill_instances[0].m2, [0.0, 0.0, 1.0]);
        // Inline fill #ff0000 -> red, full alpha.
        assert_eq!(geo.fill_instances[0].fill, [1.0, 0.0, 0.0, 1.0]);
        // Inline stroke #00ff00 -> green, full alpha.
        assert_eq!(geo.stroke_instances[0].stroke, [0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn megabuffer_ranges_are_contiguous_across_two_objects() {
        let scene = scene_with(vec![rect_object("a"), rect_object("b")], None);
        let geo = build_scene_geometry(&scene);

        assert_eq!(geo.draws.len(), 2);
        // Two pushes -> two contiguous ranges tiling the merged index array.
        let a = geo.draws[0].fill_range;
        let b = geo.draws[1].fill_range;
        assert_eq!(a.start, 0);
        assert_eq!(a.end, b.start, "object ranges are contiguous");
        assert_eq!(b.end, geo.fill.indices.len() as u32);
        // The second object's indices are rebased into the merged vertex array,
        // so the highest index is >= the first object's vertex count.
        let max_index = geo.fill.indices.iter().copied().max().unwrap();
        assert!(max_index as usize >= geo.draws[0].fill_range.len() as usize);
    }

    #[test]
    fn selection_adds_focus_ring_flag() {
        let scene = scene_with(vec![rect_object("o1")], Some("o1".to_string()));
        let geo = build_scene_geometry(&scene);
        assert!(geo.draws[0].focus_ring, "selected object rings");

        let unselected = scene_with(vec![rect_object("o1")], Some("other".to_string()));
        let geo2 = build_scene_geometry(&unselected);
        assert!(!geo2.draws[0].focus_ring);
    }

    #[test]
    fn multi_select_adds_focus_ring_flag() {
        let mut scene = scene_with(vec![rect_object("o1")], None);
        scene.multi_select = vec!["o1".to_string()];
        let geo = build_scene_geometry(&scene);
        assert!(
            geo.draws[0].focus_ring,
            "multi-selected object rings like the single anchor"
        );
    }

    #[test]
    fn unstyled_object_uses_structural_default_paints() {
        let mut obj = rect_object("o1");
        obj.fill = None;
        obj.stroke = None;
        let scene = scene_with(vec![obj], None);
        let geo = build_scene_geometry(&scene);
        // Default fill #ffffff -> white; default stroke #283644.
        assert_eq!(geo.fill_instances[0].fill, [1.0, 1.0, 1.0, 1.0]);
        let s = geo.stroke_instances[0].stroke;
        assert!((s[0] - 0x28 as f32 / 255.0).abs() < 1e-6);
        assert!((s[1] - 0x36 as f32 / 255.0).abs() < 1e-6);
        assert!((s[2] - 0x44 as f32 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn cubic_geometry_flattens_into_fill_and_stroke() {
        let mut obj = rect_object("curve");
        // An open cubic: M then C. No Z, so it strokes but does not fill.
        obj.geometry_d = "M0 0 C80 -160 720 -160 800 0".to_string();
        let scene = scene_with(vec![obj], None);
        let geo = build_scene_geometry(&scene);
        // Open path: stroke ribbon is non-empty.
        assert!(!geo.draws[0].stroke_range.is_empty());
        // The cubic flattened to more than the two endpoints (interior points).
        assert!(geo.stroke_vertices.len() > 6);
    }

    #[test]
    fn dashed_stroke_produces_multiple_ribbon_runs() {
        let mut obj = rect_object("dashed");
        // A straight open segment, 100px, with a [4,4] dash -> multiple on-runs.
        obj.geometry_d = "M0 0 L800 0".to_string();
        obj.stroke = Some(RStroke {
            paint: RPaint::Solid {
                color: "#000000".to_string(),
            },
            width: 2.0,
            opacity: 1.0,
            dash: vec![4.0, 4.0],
            cap: RStrokeCap::Butt,
            join: RStrokeJoin::Miter,
        });
        let scene = scene_with(vec![obj], None);
        let geo = build_scene_geometry(&scene);
        let solid = {
            let mut o = rect_object("solid");
            o.geometry_d = "M0 0 L800 0".to_string();
            o.stroke = Some(RStroke {
                paint: RPaint::Solid {
                    color: "#000000".to_string(),
                },
                width: 2.0,
                opacity: 1.0,
                dash: Vec::new(),
                cap: RStrokeCap::Butt,
                join: RStrokeJoin::Miter,
            });
            build_scene_geometry(&scene_with(vec![o], None))
        };
        // The dashed line splits into multiple ribbon quads, so it produces more
        // stroke vertices than the single solid quad.
        assert!(
            geo.stroke_vertices.len() > solid.stroke_vertices.len(),
            "dashing splits the line into more ribbon runs"
        );
    }

    /// W2-11 perf gate (the whole point of S7): a sustained drag updates ONLY each
    /// dragged object's instance model matrix via `preview_instance_columns` — a
    /// pure column compose — and NEVER re-runs `build_scene_geometry`. We bake a
    /// 10_000-object scene once (the single tessellation entry), then drive
    /// DRAG_FRAMES of per-object preview pushes and assert the bake closure ran
    /// exactly once across the whole scenario (zero re-tessellation, P4).
    #[test]
    fn perf_gate_drag_10k_objects_zero_retessellation_instance_path() {
        const N: usize = 10_000;
        const DRAG_FRAMES: usize = 60;

        let scene = scene_with((0..N).map(|i| rect_object(&format!("obj-{i}"))).collect(), None);

        // The ONLY tessellation entry: bake the whole scene once. A counter pins
        // the ground-truth `build_scene_geometry` call count; the drag must not move
        // it past 1.
        let mut bake_calls = 0usize;
        let geo = {
            bake_calls += 1;
            build_scene_geometry(&scene)
        };
        assert_eq!(geo.draws.len(), N);
        assert_eq!(bake_calls, 1, "initial bake is the sole tessellation");

        // A cumulative translate delta, advancing each frame so the matrix actually
        // moves (a real drag, not a no-op).
        for frame in 0..DRAG_FRAMES {
            let delta = crate::hit_test_object::translate_3x3((frame as f64) + 1.0, -(frame as f64));
            for obj in &scene.objects {
                let (m0, m1, m2) = preview_instance_columns(&delta, &obj.transform);
                // Finite, no NaN/inf on the hot path.
                for v in m0.iter().chain(m1.iter()).chain(m2.iter()) {
                    assert!(v.is_finite(), "preview columns stay finite");
                }
                // Correctness: the pushed columns equal `delta * base`'s columns,
                // exactly the packing `build_scene_geometry` would have baked.
                let world = crate::hit_test_object::mat3_mul(&delta, &obj.transform);
                assert_eq!(m0, matrix_col(&world, 0));
                assert_eq!(m1, matrix_col(&world, 1));
                assert_eq!(m2, matrix_col(&world, 2));
            }
        }

        // THE 0-REBAKE GATE: the entire N * DRAG_FRAMES drag re-tessellated nothing.
        assert_eq!(
            bake_calls, 1,
            "1만 object 드래그 = 재tessellation 0: the instance-matrix push must never re-bake (P4)"
        );
    }

    /// W2-11: the GPU instance shortcut is geometry-equivalent to the old
    /// full-rebake feed. `preview_instance_columns(delta, base)` must yield the SAME
    /// instance columns as a full `build_scene_geometry` of a scene whose object
    /// transform was set to `compose(delta, base)` — proving the per-move push is
    /// pixel-equivalent to the W2-05 `sceneWithObjectTransformed` rebake it replaces.
    #[test]
    fn preview_columns_equal_full_rebake_at_composed_transform() {
        let base = [[2.0, 0.0, 30.0], [0.0, 2.0, -10.0], [0.0, 0.0, 1.0]];
        let delta = crate::hit_test_object::rotate_about_3x3(std::f64::consts::FRAC_PI_3, 5.0, 7.0);

        let mut obj = rect_object("o1");
        obj.transform = base;
        let preview = preview_instance_columns(&delta, &base);

        // Full rebake: bake the object at the composed world transform and read its
        // baked instance columns.
        let composed = crate::hit_test_object::mat3_mul(&delta, &base);
        let mut rebaked_obj = rect_object("o1");
        rebaked_obj.transform = composed;
        let geo = build_scene_geometry(&scene_with(vec![rebaked_obj], None));
        let inst = geo.fill_instances[0];

        assert_eq!(preview.0, inst.m0);
        assert_eq!(preview.1, inst.m1);
        assert_eq!(preview.2, inst.m2);
        // And the stroke instance carries the same matrix columns.
        let stroke = geo.stroke_instances[0];
        assert_eq!(preview.0, stroke.m0);
        assert_eq!(preview.1, stroke.m1);
        assert_eq!(preview.2, stroke.m2);
    }

    /// W2-11 index==offset invariant: `draws[i].id` ↔ instance `i` in BOTH the fill
    /// and stroke instance buffers. `set_preview_transform` looks up `i` via
    /// `draws.position(id)` and writes `i * size_of` in both buffers, so this
    /// alignment is load-bearing. Pin it so a future reorder of draws-vs-instances
    /// can't silently corrupt the preview write.
    #[test]
    fn draws_index_aligns_with_both_instance_buffers() {
        let scene = scene_with(
            vec![rect_object("a"), rect_object("b"), rect_object("c")],
            None,
        );
        let geo = build_scene_geometry(&scene);
        assert_eq!(geo.draws.len(), geo.fill_instances.len());
        assert_eq!(geo.draws.len(), geo.stroke_instances.len());
        for (i, draw) in geo.draws.iter().enumerate() {
            assert_eq!(draw.id, scene.objects[i].id, "draws stay in scene order");
            assert_eq!(draw.fill_instance, geo.fill_instances[i]);
            assert_eq!(draw.stroke_instance, geo.stroke_instances[i]);
        }
    }
}
