//! W3-G8/A real drop-shadow blur: offscreen separable Gaussian.
//!
//! The G7 shadow faked softness by stacking scaled silhouette tiers. This module
//! replaces that with a TRUE macOS-style soft shadow: the per-object shadow
//! silhouette (the offset fill mesh) is rendered ONCE into an offscreen mask, blur
//! ed with a separable Gaussian (a horizontal pass then a vertical pass), and the
//! blurred result composites UNDER the fill in the visible object pass, tinted by
//! the theme `shadow` token. The shadow GEOMETRY is still baked once with the scene
//! (zero per-frame re-tessellation); only the two fixed-cost fullscreen blur passes
//! and the composite run per frame.
//!
//! ## Isolation contract
//!
//! A bug in this path must NEVER blank the canvas. The mask/blur/composite are an
//! ADDITIVE underlay: the fill/stroke/text passes draw on top into the same surface
//! regardless. The worst acceptable failure mode is "shadow missing/weak". The
//! composite reads only the blurred mask's ALPHA as coverage and multiplies it by
//! the theme shadow color, so an empty mask -> zero coverage -> invisible (never a
//! dark wash over the canvas).
//!
//! ## What is host-testable vs GPU-runtime-deferred
//!
//! [`gaussian_kernel`] is a pure, device-free function (normalized, symmetric
//! weights) with falsifiable unit tests below — it runs on the host test gate. The
//! WGSL blur/composite shaders, the offscreen render/sample wiring, bind-group /
//! pipeline-layout match, and the resize re-creation all compile at build time but
//! only EXECUTE on a browser device; the look is user-validated.

/// Maximum one-sided blur radius in taps. The kernel is `2*radius+1` taps wide;
/// the WGSL blur shader's fixed loop bound (`SHADOW_BLUR_MAX_RADIUS`) must match.
pub const SHADOW_BLUR_MAX_RADIUS: usize = 12;

/// Default blur radius in PHYSICAL pixels (zoom-independent screen blur). Scaled by
/// the device-pixel ratio at upload so the screen feather is constant across DPRs.
pub const SHADOW_BLUR_RADIUS_PX: f32 = 6.0;

/// Compute a normalized, symmetric 1-D Gaussian kernel of `2*radius+1` taps. The
/// `sigma` controls the spread; passing `radius == 0` yields the trivial `[1.0]`
/// kernel (no blur). Weights are normalized to sum to 1.0 and are symmetric about
/// the center tap, so the separable H-then-V passes preserve total energy and a
/// wider sigma pushes more weight into the tails.
///
/// Pure and device-free: this is the falsifiable core of the blur (the WGSL shader
/// reads these exact weights from a uniform), so it carries the host unit tests.
pub fn gaussian_kernel(radius: usize, sigma: f32) -> Vec<f32> {
    if radius == 0 {
        return vec![1.0];
    }
    // A non-positive sigma would divide by zero; clamp to a tiny positive so the
    // kernel stays a valid (sharply-peaked) distribution instead of NaN.
    let sigma = sigma.max(1e-4);
    let two_sigma_sq = 2.0 * sigma * sigma;
    let n = 2 * radius + 1;
    let mut weights = Vec::with_capacity(n);
    for i in 0..n {
        // Offset from the center tap, in [-radius, radius].
        let x = i as f32 - radius as f32;
        weights.push((-(x * x) / two_sigma_sq).exp());
    }
    let sum: f32 = weights.iter().sum();
    for w in &mut weights {
        *w /= sum;
    }
    weights
}

#[cfg(feature = "wgpu-probe")]
mod gpu {
    use super::{gaussian_kernel, SHADOW_BLUR_MAX_RADIUS, SHADOW_BLUR_RADIUS_PX};
    use crate::shaders::{SHADOW_BLUR_WGSL, SHADOW_COMPOSITE_WGSL};

    /// Blur-pass uniform matching `shadow_blur.wgsl`'s `BlurParams`. `direction` is
    /// `(1,0)` for the horizontal pass and `(0,1)` for the vertical pass, in
    /// TEXEL units (the shader multiplies by `texel` to step one pixel). `texel` is
    /// `(1/width, 1/height)` so the sample offsets are resolution-correct. `params`
    /// is `vec4(radius_taps, _, _, _)`; the active tap count is read from `.x`.
    /// `weights` holds the normalized one-sided+center Gaussian taps (index 0 is the
    /// center, then taps 1..=radius), padded to the fixed max so the layout is
    /// constant. std140 requires 16-byte alignment, so each weight occupies a full
    /// `vec4` slot (`.x` carries the value).
    #[repr(C)]
    #[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
    pub struct BlurParams {
        pub direction: [f32; 2],
        pub texel: [f32; 2],
        pub params: [f32; 4],
        // One weight per vec4 slot (std140): `[w, 0, 0, 0]`. Index 0 = center tap.
        pub weights: [[f32; 4]; SHADOW_BLUR_MAX_RADIUS + 1],
    }

    /// Composite uniform matching `shadow_composite.wgsl`'s `CompositeParams`:
    /// `tint` is the theme `shadow` token color (resolved through the token path,
    /// never hardcoded). The shader multiplies the blurred mask's coverage (its
    /// alpha) by this tint, so a theme flip is a single uniform write.
    #[repr(C)]
    #[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
    pub struct CompositeParams {
        pub tint: [f32; 4],
    }

    /// Offscreen targets + pipelines for the separable-Gaussian drop-shadow blur.
    /// Sized to the surface (`config.width` x `config.height`, PHYSICAL px); rebuilt
    /// on resize. Owns three surface-format render targets — `mask` (the silhouette
    /// rendered once), `ping`/`pong` (H then V blur scratch) — and the blur +
    /// composite pipelines.
    pub struct ShadowBlur {
        width: u32,
        height: u32,

        mask_view: wgpu::TextureView,
        ping_view: wgpu::TextureView,
        pong_view: wgpu::TextureView,
        // Held so the views stay valid; the textures/sampler/layouts/format are not
        // read again after construction, but must outlive the views/pipelines/bind
        // groups they back — hence the `_` prefix to silence the unused-field lint.
        _format: wgpu::TextureFormat,
        _mask: wgpu::Texture,
        _ping: wgpu::Texture,
        _pong: wgpu::Texture,
        _sampler: wgpu::Sampler,
        _blur_bind_group_layout: wgpu::BindGroupLayout,
        _composite_bind_group_layout: wgpu::BindGroupLayout,

        blur_pipeline: wgpu::RenderPipeline,
        composite_pipeline: wgpu::RenderPipeline,

        // Two blur-param buffers (H then V) so both directions can be bound in one
        // frame without a mid-frame overwrite race.
        blur_h_buffer: wgpu::Buffer,
        blur_v_buffer: wgpu::Buffer,
        composite_buffer: wgpu::Buffer,

        // Bind groups are stable for the lifetime of the targets (views don't
        // change between frames), so they are built once here.
        blur_h_bind_group: wgpu::BindGroup,
        blur_v_bind_group: wgpu::BindGroup,
        composite_bind_group: wgpu::BindGroup,
    }

    impl ShadowBlur {
        /// Build the offscreen targets + blur/composite pipelines for a surface of
        /// `width` x `height` PHYSICAL px and `format` (the surface format, so the
        /// mask blends identically to the visible pass). `dpr` scales the blur
        /// radius so the screen feather is DPR-independent.
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

            let make_target = |label: &str| -> (wgpu::Texture, wgpu::TextureView) {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
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

            // group(0): b0 source texture (FRAGMENT), b1 sampler (FRAGMENT), b2
            // params uniform (FRAGMENT). Shared shape for blur + composite (the
            // composite uniform differs in contents only).
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

            // The blur passes WRITE the full (premultiplied-ish) source through, so
            // they REPLACE the target (no blend) — the source already carries the
            // silhouette coverage. The composite blends src-over onto the surface.
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

            // H pass samples the MASK -> writes PING. V pass samples PING -> writes
            // PONG. Composite samples PONG. The bind groups capture those source
            // views; they are stable for these targets' lifetime.
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
            // The Gaussian kernel + texel only depend on size/dpr, both fixed for
            // this target — so compute + upload the blur params ONCE here (the only
            // place the kernel Vec is allocated), never per frame.
            blur.upload_blur_params(queue, dpr);
            blur
        }

        /// Whether the targets already match a surface of `width` x `height`. The
        /// caller (resize) rebuilds only on a mismatch.
        pub fn matches(&self, width: u32, height: u32) -> bool {
            self.width == width.max(1) && self.height == height.max(1)
        }

        /// Compute + upload the H/V Gaussian blur params (taps + texel + direction)
        /// for this target's size and `dpr`-scaled blur radius. Called ONCE from
        /// `new` (the kernel/texel are size-fixed); the per-frame path never touches
        /// this, so the kernel `Vec` allocation stays off the hot path.
        fn upload_blur_params(&self, queue: &wgpu::Queue, dpr: f32) {
            let radius_px = (SHADOW_BLUR_RADIUS_PX * dpr.max(1.0)).round();
            let radius = (radius_px as usize).clamp(1, SHADOW_BLUR_MAX_RADIUS);
            // A common rule of thumb: sigma ~= radius/3 keeps the tails inside the
            // kernel support so the truncation error stays small.
            let sigma = (radius as f32 / 3.0).max(0.5);
            let kernel = gaussian_kernel(radius, sigma);
            // `gaussian_kernel` returns the full `2*radius+1` symmetric kernel; the
            // shader reads one-sided taps (center + positive side) and mirrors them,
            // so pack `weights[0] = center`, `weights[k] = kernel[radius + k]`.
            let mut weights = [[0.0f32; 4]; SHADOW_BLUR_MAX_RADIUS + 1];
            for k in 0..=radius {
                weights[k][0] = kernel[radius + k];
            }
            let texel = [1.0 / self.width as f32, 1.0 / self.height as f32];
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

        /// Upload the composite tint (the theme `shadow` token color). A single
        /// 16-byte uniform write with NO allocation — cheap enough to call per frame,
        /// though in practice only the theme flip changes it.
        pub fn set_tint(&self, queue: &wgpu::Queue, tint: [f32; 4]) {
            queue.write_buffer(
                &self.composite_buffer,
                0,
                bytemuck::cast_slice(&[CompositeParams { tint }]),
            );
        }

        /// Record the H then V separable blur passes (mask -> ping -> pong), each a
        /// fullscreen triangle. The MASK must already hold the rendered shadow
        /// silhouette (see [`record_mask_into`]); this only blurs it.
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
        /// This is the FIRST draw into the visible surface, so it both CLEARS the
        /// surface to `clear` (the canvas-bg) and draws the shadow on top — the
        /// subsequent fill/stroke/text pass then loads and draws over the shadow. By
        /// owning the clear here the shadow is guaranteed to sit beneath the fill in
        /// a single, well-ordered surface pass.
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

        /// The mask view the shadow silhouette renders into (cleared to transparent
        /// by the caller's render pass). Exposed so `ObjectRenderer` can record the
        /// shadow-only sub-pass into it.
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
                    // REPLACE blend + a clear here are equivalent for a full-coverage
                    // triangle; clear keeps it well-defined even outside the tri.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The kernel is symmetric about the center tap and sums to ~1.0. FAILS if the
    /// kernel is left unnormalized (sum != 1) or built asymmetrically.
    #[test]
    fn gaussian_kernel_is_normalized_and_symmetric() {
        for radius in 1..=8usize {
            let sigma = radius as f32 / 3.0;
            let k = gaussian_kernel(radius, sigma);
            assert_eq!(k.len(), 2 * radius + 1, "kernel is 2*radius+1 taps wide");

            // Sums to 1.0 (normalized) within tight tolerance.
            let sum: f32 = k.iter().sum();
            assert!(
                (sum - 1.0).abs() < 1e-5,
                "radius {radius}: kernel sums to ~1.0 (got {sum})"
            );

            // Symmetric: weight at +i equals weight at -i.
            for i in 0..=radius {
                let lo = k[radius - i];
                let hi = k[radius + i];
                assert!(
                    (lo - hi).abs() < 1e-6,
                    "radius {radius}: tap +{i} ({hi}) != tap -{i} ({lo})"
                );
            }
        }
    }

    /// The center tap is the maximum (a Gaussian peaks at the center) and weights
    /// fall off monotonically toward the tails. FAILS if the distribution is flat
    /// (a box blur) or inverted.
    #[test]
    fn gaussian_kernel_peaks_at_center_and_falls_off() {
        let radius = 6;
        let k = gaussian_kernel(radius, radius as f32 / 3.0);
        for i in 0..radius {
            // Strictly decreasing from center out to the tail (one-sided).
            assert!(
                k[radius + i] > k[radius + i + 1],
                "tap {i} ({}) must exceed tap {} ({})",
                k[radius + i],
                i + 1,
                k[radius + i + 1]
            );
        }
        // The center is the global max.
        let center = k[radius];
        assert!(
            k.iter().all(|&w| w <= center + 1e-9),
            "center tap is the maximum weight"
        );
    }

    /// A WIDER sigma spreads more weight into the tails: the center weight DROPS and
    /// the tail weight RISES as sigma grows (the blur softens). This is the property
    /// that makes the blur actually blur. FAILS if sigma is ignored (e.g. a fixed
    /// box kernel) because the center/tail ratio would not move.
    #[test]
    fn wider_sigma_spreads_weight_to_tails() {
        let radius = 8;
        let narrow = gaussian_kernel(radius, 1.0);
        let wide = gaussian_kernel(radius, 4.0);

        // Center weight is monotonically smaller for the wider kernel.
        assert!(
            wide[radius] < narrow[radius],
            "wider sigma lowers the center weight ({} !< {})",
            wide[radius],
            narrow[radius]
        );
        // Tail (outermost) weight is larger for the wider kernel.
        assert!(
            wide[0] > narrow[0],
            "wider sigma raises the tail weight ({} !> {})",
            wide[0],
            narrow[0]
        );
        // Both still normalized.
        assert!((narrow.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert!((wide.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }

    /// `radius == 0` is the identity kernel (`[1.0]`) — no blur, still normalized.
    #[test]
    fn zero_radius_is_identity_kernel() {
        let k = gaussian_kernel(0, 1.0);
        assert_eq!(k, vec![1.0]);
    }

    /// The packed one-sided tap count never exceeds the shader's fixed loop bound.
    /// FAILS if `SHADOW_BLUR_MAX_RADIUS` and the WGSL `MAX_RADIUS` drift apart.
    #[test]
    fn blur_radius_fits_shader_loop_bound() {
        let radius_px = (SHADOW_BLUR_RADIUS_PX * 3.0).round() as usize; // 3x DPR
        let radius = radius_px.clamp(1, SHADOW_BLUR_MAX_RADIUS);
        assert!(radius <= SHADOW_BLUR_MAX_RADIUS);
        // The one-sided weights array (center + radius) fits the fixed slot count.
        assert!(radius + 1 <= SHADOW_BLUR_MAX_RADIUS + 1);
    }
}
