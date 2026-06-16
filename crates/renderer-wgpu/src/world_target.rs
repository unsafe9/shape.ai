//! The offscreen world color target: the sampleable backbuffer the world pass can
//! render into so panels can frost the content behind them (backdrop blur).
//!
//! Today the world renders straight to the swapchain (`RENDER_ATTACHMENT` only),
//! which a sampler cannot read. A frosted panel must SAMPLE the world, so the world
//! must land in a `RENDER_ATTACHMENT | TEXTURE_BINDING` texture first, then blit to
//! the swapchain. This module owns that target's descriptor (host-testable) and its
//! allocation + resize. All `wgpu`-typed code is behind `wgpu-probe` (the only
//! feature that pulls in `wgpu`), matching `shadow_blur.rs`.

#[cfg(feature = "wgpu-probe")]
mod gpu {
    /// The descriptor for the offscreen world color target at `width` x `height`
    /// PHYSICAL px in the surface `format`. Carries `TEXTURE_BINDING` (so the panel
    /// frost can sample it) on top of `RENDER_ATTACHMENT` (so the world pass can draw
    /// into it). `COPY_SRC` lets the cheapest backbuffer→swapchain hand-off be a raw
    /// texture copy when formats match.
    pub fn world_target_descriptor(
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("shape.ai offscreen world color target"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        }
    }

    /// The offscreen world color target + its view, sized to the surface and rebuilt
    /// on resize. The world pass renders here (sampleable), then the frame blits it
    /// to the swapchain; panels sample `view()` for their frosted backdrop.
    pub struct WorldTarget {
        width: u32,
        height: u32,
        view: wgpu::TextureView,
        // Held so `view` stays valid; not read again after construction.
        _texture: wgpu::Texture,
    }

    impl WorldTarget {
        pub fn new(
            device: &wgpu::Device,
            width: u32,
            height: u32,
            format: wgpu::TextureFormat,
        ) -> Self {
            let width = width.max(1);
            let height = height.max(1);
            let texture = device.create_texture(&world_target_descriptor(width, height, format));
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            Self {
                width,
                height,
                view,
                _texture: texture,
            }
        }

        /// Whether the target already matches a surface of `width` x `height`.
        pub fn matches(&self, width: u32, height: u32) -> bool {
            self.width == width.max(1) && self.height == height.max(1)
        }

        /// The sampleable world view — the world pass renders into it; panels sample it.
        pub fn view(&self) -> &wgpu::TextureView {
            &self.view
        }
    }

    #[cfg(test)]
    mod tests {
        use super::world_target_descriptor;

        /// The offscreen world target MUST be sampleable (so a frosted panel can read
        /// the content behind it) AND a render attachment (so the world pass draws
        /// into it). Missing `TEXTURE_BINDING` is the exact failure that makes
        /// backdrop blur impossible — this pins both bits.
        #[test]
        fn world_target_is_a_sampleable_render_attachment() {
            let desc = world_target_descriptor(1600, 900, wgpu::TextureFormat::Bgra8Unorm);
            assert!(
                desc.usage.contains(wgpu::TextureUsages::TEXTURE_BINDING),
                "the offscreen world target must be sampleable for the panel frost"
            );
            assert!(
                desc.usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT),
                "the offscreen world target must be a render attachment for the world pass"
            );
            assert_eq!(desc.size.width, 1600);
            assert_eq!(desc.size.height, 900);
            assert_eq!(
                desc.sample_count, 1,
                "single-sampled so it binds as a plain texture_2d"
            );
        }

        #[test]
        fn world_target_clamps_a_zero_dimension_to_one() {
            let desc = world_target_descriptor(0, 0, wgpu::TextureFormat::Bgra8Unorm);
            assert_eq!(desc.size.width, 1, "a zero width clamps to a non-zero texture");
            assert_eq!(desc.size.height, 1, "a zero height clamps to a non-zero texture");
        }
    }
}

#[cfg(feature = "wgpu-probe")]
pub use gpu::WorldTarget;
