//! Offscreen separable-Gaussian drop-shadow blur (GPU passes). The pure kernel/
//! constants live in [`shape_renderer_core::shadow_blur`].

#[cfg(feature = "wgpu-probe")]
mod gpu {
    use shape_renderer_core::shadow_blur::{quarter_dim, gaussian_kernel, SHADOW_BLUR_MAX_RADIUS, SHADOW_BLUR_RADIUS_PX};
    use crate::shaders::{SHADOW_BLUR_WGSL, SHADOW_COMPOSITE_WGSL};

    /// Matches `shadow_blur.wgsl`'s `BlurParams`. `direction` is `(1,0)` H / `(0,1)`
    /// V in TEXEL units; `texel` is `(1/width, 1/height)`; `params.x` is the active
    /// tap count. std140 16-byte alignment forces each weight into a full `vec4`
    /// slot (`.x` carries the value, index 0 = center tap).
    #[repr(C)]
    #[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
    pub struct BlurParams {
        pub direction: [f32; 2],
        pub texel: [f32; 2],
        pub params: [f32; 4],
        pub weights: [[f32; 4]; SHADOW_BLUR_MAX_RADIUS + 1],
    }

    /// Matches `shadow_composite.wgsl`'s `CompositeParams`: `tint` is the theme
    /// `shadow` token color (resolved through the token path, never hardcoded).
    #[repr(C)]
    #[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
    pub struct CompositeParams {
        pub tint: [f32; 4],
    }

    /// Offscreen targets + pipelines for the separable-Gaussian drop-shadow blur.
    /// Sized to the surface (PHYSICAL px), rebuilt on resize.
    pub struct ShadowBlur {
        width: u32,
        height: u32,

        mask_view: wgpu::TextureView,
        ping_view: wgpu::TextureView,
        pong_view: wgpu::TextureView,
        // Held so the views stay valid: not read again after construction, but must
        // outlive the views/pipelines/bind groups they back.
        _format: wgpu::TextureFormat,
        _mask: wgpu::Texture,
        _ping: wgpu::Texture,
        _pong: wgpu::Texture,
        _sampler: wgpu::Sampler,
        _blur_bind_group_layout: wgpu::BindGroupLayout,
        _composite_bind_group_layout: wgpu::BindGroupLayout,

        blur_pipeline: wgpu::RenderPipeline,
        composite_pipeline: wgpu::RenderPipeline,

        // Two blur-param buffers (H then V) so both directions bind in one frame
        // without a mid-frame overwrite race.
        blur_h_buffer: wgpu::Buffer,
        blur_v_buffer: wgpu::Buffer,
        composite_buffer: wgpu::Buffer,

        blur_h_bind_group: wgpu::BindGroup,
        blur_v_bind_group: wgpu::BindGroup,
        composite_bind_group: wgpu::BindGroup,
    }

    impl ShadowBlur {
        /// Build the offscreen targets + pipelines for a surface of `width` x
        /// `height` PHYSICAL px and `format` (the surface format, so the mask blends
        /// identically to the visible pass). `dpr` scales the blur radius so the
        /// screen feather is DPR-independent.
        pub fn new(
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            format: wgpu::TextureFormat,
            width: u32,
            height: u32,
            dpr: f32,
        ) -> Self {
            let width = width.max(1);
            let height = height.max(1);
            // Blur targets are QUARTER-res; the stored `width`/`height` stay full so
            // `matches` compares against the surface.
            let qw = quarter_dim(width);
            let qh = quarter_dim(height);

            let make_target = |label: &str| -> (wgpu::Texture, wgpu::TextureView) {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: qw,
                        height: qh,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                (texture, view)
            };
            let (mask, mask_view) = make_target("shape.ai shadow mask");
            let (ping, ping_view) = make_target("shape.ai shadow blur ping");
            let (pong, pong_view) = make_target("shape.ai shadow blur pong");

            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("shape.ai shadow blur sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            });

            // group(0): b0 source texture, b1 sampler, b2 params uniform (all
            // FRAGMENT). Shared by blur + composite (composite differs in contents).
            let sampled_layout_entries = [
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
            ];
            let blur_bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("shape.ai shadow blur bind group layout"),
                    entries: &sampled_layout_entries,
                });
            let composite_bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("shape.ai shadow composite bind group layout"),
                    entries: &sampled_layout_entries,
                });

            let blur_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("shape.ai shadow blur shader"),
                source: wgpu::ShaderSource::Wgsl(SHADOW_BLUR_WGSL.into()),
            });
            let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("shape.ai shadow composite shader"),
                source: wgpu::ShaderSource::Wgsl(SHADOW_COMPOSITE_WGSL.into()),
            });

            // Blur passes REPLACE the target (the source already carries silhouette
            // coverage); the composite blends src-over onto the surface.
            let replace_targets = [Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })];
            let over_targets = [Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })];

            let blur_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shape.ai shadow blur pipeline layout"),
                bind_group_layouts: &[Some(&blur_bind_group_layout)],
                immediate_size: 0,
            });
            let blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("shape.ai shadow blur pipeline"),
                layout: Some(&blur_layout),
                vertex: wgpu::VertexState {
                    module: &blur_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &blur_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &replace_targets,
                }),
                multiview_mask: None,
                cache: None,
            });

            let composite_layout =
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("shape.ai shadow composite pipeline layout"),
                    bind_group_layouts: &[Some(&composite_bind_group_layout)],
                    immediate_size: 0,
                });
            let composite_pipeline =
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("shape.ai shadow composite pipeline"),
                    layout: Some(&composite_layout),
                    vertex: wgpu::VertexState {
                        module: &composite_shader,
                        entry_point: Some("vs_main"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &composite_shader,
                        entry_point: Some("fs_main"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: &over_targets,
                    }),
                    multiview_mask: None,
                    cache: None,
                });

            let make_uniform = |label: &str, size: u64| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            let blur_h_buffer = make_uniform(
                "shape.ai shadow blur-h params",
                std::mem::size_of::<BlurParams>() as u64,
            );
            let blur_v_buffer = make_uniform(
                "shape.ai shadow blur-v params",
                std::mem::size_of::<BlurParams>() as u64,
            );
            let composite_buffer = make_uniform(
                "shape.ai shadow composite params",
                std::mem::size_of::<CompositeParams>() as u64,
            );

            // H samples MASK -> PING; V samples PING -> PONG; composite samples PONG.
            // The bind groups capture those source views, stable for the targets' life.
            let make_bind_group = |label: &str,
                                   layout: &wgpu::BindGroupLayout,
                                   src: &wgpu::TextureView,
                                   uniform: &wgpu::Buffer| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(src),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: uniform.as_entire_binding(),
                        },
                    ],
                })
            };
            let blur_h_bind_group = make_bind_group(
                "shape.ai shadow blur-h bind group",
                &blur_bind_group_layout,
                &mask_view,
                &blur_h_buffer,
            );
            let blur_v_bind_group = make_bind_group(
                "shape.ai shadow blur-v bind group",
                &blur_bind_group_layout,
                &ping_view,
                &blur_v_buffer,
            );
            let composite_bind_group = make_bind_group(
                "shape.ai shadow composite bind group",
                &composite_bind_group_layout,
                &pong_view,
                &composite_buffer,
            );

            let blur = ShadowBlur {
                width,
                height,
                mask_view,
                ping_view,
                pong_view,
                _format: format,
                _mask: mask,
                _ping: ping,
                _pong: pong,
                _sampler: sampler,
                _blur_bind_group_layout: blur_bind_group_layout,
                _composite_bind_group_layout: composite_bind_group_layout,
                blur_pipeline,
                composite_pipeline,
                blur_h_buffer,
                blur_v_buffer,
                composite_buffer,
                blur_h_bind_group,
                blur_v_bind_group,
                composite_bind_group,
            };
            // Kernel + texel depend only on size/dpr (fixed for this target), so
            // compute + upload ONCE here — the only place the kernel Vec is allocated,
            // never per frame.
            blur.upload_blur_params(queue, dpr);
            blur
        }

        /// Whether the targets already match a surface of `width` x `height`.
        pub fn matches(&self, width: u32, height: u32) -> bool {
            self.width == width.max(1) && self.height == height.max(1)
        }

        fn upload_blur_params(&self, queue: &wgpu::Queue, dpr: f32) {
            let radius_px = (SHADOW_BLUR_RADIUS_PX * dpr.max(1.0)).round();
            #[allow(
                clippy::cast_possible_truncation,
                reason = "small clamped-positive blur radius (px); rounds to an exact in-range integer"
            )]
            let radius_usize = radius_px.max(0.0) as usize;
            let radius = radius_usize.clamp(1, SHADOW_BLUR_MAX_RADIUS);
            // sigma ~= radius/3 keeps the tails inside the kernel support.
            let sigma = (radius as f32 / 3.0).max(0.5);
            let kernel = gaussian_kernel(radius, sigma);
            // `gaussian_kernel` returns the full `2*radius+1` symmetric kernel; the
            // shader reads one-sided taps and mirrors them, so pack `weights[0] =
            // center`, `weights[k] = kernel[radius + k]`.
            let mut weights = [[0.0f32; 4]; SHADOW_BLUR_MAX_RADIUS + 1];
            for k in 0..=radius {
                weights[k][0] = kernel[radius + k];
            }
            // Quarter-res target: one texel is `4 / full_dim`, so each tap steps 4
            // physical px (~48-physical-px one-sided reach at the same tap budget).
            let texel = [4.0 / self.width as f32, 4.0 / self.height as f32];
            let h = BlurParams {
                direction: [1.0, 0.0],
                texel,
                params: [radius as f32, 0.0, 0.0, 0.0],
                weights,
            };
            let v = BlurParams {
                direction: [0.0, 1.0],
                texel,
                params: [radius as f32, 0.0, 0.0, 0.0],
                weights,
            };
            queue.write_buffer(&self.blur_h_buffer, 0, bytemuck::cast_slice(&[h]));
            queue.write_buffer(&self.blur_v_buffer, 0, bytemuck::cast_slice(&[v]));
        }

        /// Upload the composite tint (the theme `shadow` token color): a single
        /// 16-byte uniform write, no allocation.
        pub fn set_tint(&self, queue: &wgpu::Queue, tint: [f32; 4]) {
            queue.write_buffer(
                &self.composite_buffer,
                0,
                bytemuck::cast_slice(&[CompositeParams { tint }]),
            );
        }

        /// Record the H then V separable blur passes (mask -> ping -> pong), each a
        /// fullscreen triangle. The MASK must already hold the shadow silhouette.
        pub fn record_blur(&self, encoder: &mut wgpu::CommandEncoder) {
            self.record_fullscreen(
                encoder,
                "shape.ai shadow blur-h pass",
                &self.ping_view,
                &self.blur_pipeline,
                &self.blur_h_bind_group,
            );
            self.record_fullscreen(
                encoder,
                "shape.ai shadow blur-v pass",
                &self.pong_view,
                &self.blur_pipeline,
                &self.blur_v_bind_group,
            );
        }

        /// Record the composite pass: a fullscreen triangle sampling the blurred
        /// mask (pong), tinted by the theme shadow color, src-over into `target`.
        /// This is the FIRST draw into the visible surface, so it CLEARS to `clear`
        /// (the canvas-bg) here, guaranteeing the shadow sits beneath the later
        /// fill/stroke/text in one well-ordered pass.
        pub fn record_composite(
            &self,
            encoder: &mut wgpu::CommandEncoder,
            target: &wgpu::TextureView,
            clear: [f32; 4],
        ) {
            let [r, g, b, a] = clear;
            let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: r as f64,
                        g: g as f64,
                        b: b as f64,
                        a: a as f64,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shape.ai shadow composite pass"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &self.composite_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        /// The mask view the shadow silhouette renders into, exposed so
        /// `ObjectRenderer` can record the shadow-only sub-pass.
        pub fn mask_view(&self) -> &wgpu::TextureView {
            &self.mask_view
        }

        fn record_fullscreen(
            &self,
            encoder: &mut wgpu::CommandEncoder,
            label: &str,
            target: &wgpu::TextureView,
            pipeline: &wgpu::RenderPipeline,
            bind_group: &wgpu::BindGroup,
        ) {
            let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &color_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(feature = "wgpu-probe")]
pub use gpu::ShadowBlur;
