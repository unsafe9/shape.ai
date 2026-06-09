//! The single-present render path (W2-13/S8): `render_frame` (object pass +
//! legacy fallback pass) plus the per-frame writers it drives — uniform upload,
//! marquee overlay geometry, glyph-atlas flush, draw-list build, viewport cull.
//! `target_arch = "wasm32"` gated; it runs against the live wgpu device.

use std::collections::HashMap;

use crate::model::{RenderCard, RenderEdge, RenderGroup, WorldRect};
use crate::serde_wasm;
use crate::stats::WebGpuFrameStats;
use crate::text::{TEXT_ATLAS_HEIGHT, TEXT_ATLAS_WIDTH};
use wasm_bindgen::prelude::*;

use super::*;

#[cfg(feature = "wgpu-probe")]
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl ShapeWebGpuRenderer {
    #[wasm_bindgen(js_name = renderFrame)]
    pub fn render_frame(&mut self) -> Result<JsValue, JsValue> {
        self.write_uniform();
        self.flush_text_atlas();
        let overlay_vertex_count = self.write_marquee_overlay();
        let handle_vertex_count = self.write_handle_overlay();
        let multi_select_vertex_count = self.write_multi_select_overlay();
        let mut draw_list = self.build_draw_list();
        // Carry this frame's per-object tiers forward so the next frame's
        // hysteresis resolves against them (T3.1 §3). Tiers are diagnostics; they
        // do not touch slot identity or vertex ranges.
        self.last_lod_tiers = std::mem::take(&mut draw_list.lod_tiers);
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
        // FC-05/FC-06: when an object scene is loaded, this single frame records the
        // OBJECT pass (clearing the surface) in place of the legacy 2D pass, so there
        // is exactly one acquire/submit/present per frame. The camera uniform is
        // refreshed every frame so pan/zoom moves objects with no scene reload.
        if let (Some(pipeline), Some(renderer)) =
            (self.object_pipeline.as_ref(), self.object_renderer.as_ref())
        {
            renderer.update_camera(
                &self.queue,
                &self.camera,
                self.width as f32,
                self.height as f32,
            );
            // W3-G8/A real drop-shadow blur: render the shadow silhouette ONCE into
            // the offscreen mask, separable-Gaussian-blur it (H then V), then
            // composite the blurred result onto the surface FIRST (it also clears the
            // surface to the canvas-bg), so the fill/stroke/text pass below draws on
            // top of the shadow. This whole block is an isolated underlay: the
            // `render(clear=false)` call still runs the fill/stroke/text regardless,
            // so a shadow fault degrades to "no shadow", never a blank canvas.
            let theme = renderer.theme();
            self.shadow_blur.set_tint(&self.queue, theme.shadow());
            renderer.render_shadow_mask(&mut encoder, pipeline, self.shadow_blur.mask_view());
            self.shadow_blur.record_blur(&mut encoder);
            self.shadow_blur.record_composite(&mut encoder, &view, theme.canvas_bg());
            // Fill/stroke/text load over the cleared + shadow-composited surface.
            renderer.render(&mut encoder, &view, pipeline, false);
            // W3-G7/#1: per-object outline highlight for the multi-select set, drawn
            // on top of the object pass with the world-space pipeline (LoadOp::Load
            // preserves the fill/stroke output). Single selection draws no outline
            // here — it keeps its 8-handle overlay below. The outline rectangles are
            // world-space quads built from each region's world bbox, so the matrix
            // transform path never re-tessellates them.
            if multi_select_vertex_count > 0 {
                let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })];
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("shape.ai multi-select outline overlay pass"),
                    color_attachments: &color_attachments,
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.multi_select_overlay_vertex_buffer.slice(..));
                pass.draw(0..multi_select_vertex_count as u32, 0..1);
            }
            // W2-04: selection-handle overlay, drawn on top of the object pass with
            // the legacy world-space pipeline (LoadOp::Load preserves the object
            // pass output). The handles are screen-fixed world quads (see
            // `build_handle_overlay_vertices`); the matrix transform W2-11 pushes to
            // the object instance path never re-tessellates these.
            if handle_vertex_count > 0 {
                let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })];
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("shape.ai selection-handle overlay pass"),
                    color_attachments: &color_attachments,
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.handle_vertex_buffer.slice(..));
                pass.draw(0..handle_vertex_count as u32, 0..1);
            }
            // RA2a (#7): the drag marquee must surface in object mode too. The object
            // pass replaces the legacy 2D pass, which is the only place the marquee
            // overlay was drawn — so without this the rubber-band never renders over
            // an object scene. Same world-space pipeline, LoadOp::Load on top.
            if overlay_vertex_count > 0 {
                let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })];
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("shape.ai object-pass marquee overlay pass"),
                    color_attachments: &color_attachments,
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.overlay_vertex_buffer.slice(..));
                pass.draw(0..overlay_vertex_count as u32, 0..1);
            }
        } else {
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
            // Draw the drag marquee on top of the scene using the same world-space
            // pipeline and bind group, from a separate dynamic vertex buffer.
            if overlay_vertex_count > 0 {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.overlay_vertex_buffer.slice(..));
                pass.draw(0..overlay_vertex_count as u32, 0..1);
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
            full_tier_count: draw_list.full_tier_count,
            compact_tier_count: draw_list.compact_tier_count,
            shape_only_tier_count: draw_list.shape_only_tier_count,
            density_tier_count: draw_list.density_tier_count,
            minimap_tier_count: draw_list.minimap_tier_count,
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
            object_count: self
                .object_scene
                .as_ref()
                .map(|scene| scene.objects.len())
                .unwrap_or(0),
            object_fill_index_count: self
                .object_renderer
                .as_ref()
                .map(|r| r.fill_index_count() as usize)
                .unwrap_or(0),
            object_stroke_vertex_count: self
                .object_renderer
                .as_ref()
                .map(|r| r.stroke_vertex_count() as usize)
                .unwrap_or(0),
            object_draw_count: self
                .object_renderer
                .as_ref()
                .map(|r| r.object_count())
                .unwrap_or(0),
            backend: "rust-wgpu-visible".to_string(),
        })
    }

}

#[cfg(feature = "wgpu-probe")]
#[cfg(target_arch = "wasm32")]
impl ShapeWebGpuRenderer {
    /// W2-04: write the selection-handle overlay (8 resize handles + rotate zone)
    /// for the current object-scene selection into the dedicated handle buffer,
    /// returning the vertex count to draw. Zero when no object is selected (or the
    /// selection has no finite world bounds). World-space quads sized
    /// `HANDLE_SIZE_PX / zoom` so the legacy pipeline draws them at a fixed screen
    /// size; the shared `selection_handles` layout keeps render == hit-test.
    fn write_handle_overlay(&mut self) -> usize {
        let selection = self
            .object_scene
            .as_ref()
            .and_then(|scene| scene.selection.clone());
        // RA1: while a transform drag is in flight the renderer holds the dragged
        // object's live preview transform; feed it so the handles track the previewed
        // bbox every frame instead of snapping only on commit (zero-rebake read).
        let preview = selection.as_deref().and_then(|id| {
            self.object_renderer
                .as_ref()
                .and_then(|renderer| renderer.preview_transform(id))
        });
        let Some((_, world_bbox)) = selection_handles(
            &self.object_regions,
            &self.camera,
            selection.as_deref(),
            preview.as_ref(),
        ) else {
            return 0;
        };
        let vertices = build_handle_overlay_vertices(&world_bbox, self.camera.zoom);
        if vertices.is_empty() {
            return 0;
        }
        self.queue
            .write_buffer(&self.handle_vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        vertices.len()
    }

    /// W3-G7/#1 + W3-G9/#2: write the per-object outline ring into its dedicated
    /// buffer, returning the vertex count to draw. Rings every multi-select member,
    /// or (when the multi-select is empty) the single selected object/group — so a
    /// grouped selection shows a continuous border, not just its 8 resize handles.
    /// Zero when nothing is selected.
    fn write_multi_select_overlay(&mut self) -> usize {
        let ids = self
            .object_scene
            .as_ref()
            .map(outline_overlay_ids)
            .unwrap_or_default();
        // W3-G10/#2: feed each id's LIVE preview transform so the outline ring tracks
        // the drag every frame like the resize handles, not just on commit. The G9
        // multi-member SameDelta preview writes every member's instance matrix, so a
        // group/multi drag rings every member live (zero-rebake read).
        let object_renderer = self.object_renderer.as_ref();
        let vertices = build_multi_select_overlay_vertices(
            &self.object_regions,
            &ids,
            self.camera.zoom,
            |id| object_renderer.and_then(|renderer| renderer.preview_transform(id)),
        );
        if vertices.is_empty() {
            return 0;
        }
        self.queue.write_buffer(
            &self.multi_select_overlay_vertex_buffer,
            0,
            bytemuck::cast_slice(&vertices),
        );
        vertices.len()
    }

    /// Write the marquee overlay quads for the active drag (if any) into the
    /// dedicated overlay vertex buffer, returning the vertex count to draw. Zero
    /// when no marquee is in flight.
    fn write_marquee_overlay(&mut self) -> usize {
        let vertices = marquee_overlay_for_drag(self.input_drag.as_ref(), self.camera.zoom);
        if vertices.is_empty() {
            return 0;
        }
        self.queue.write_buffer(
            &self.overlay_vertex_buffer,
            0,
            bytemuck::cast_slice(&vertices),
        );
        vertices.len()
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
                draw_list.record_tier(
                    &group.id,
                    &group.bounds,
                    &self.camera,
                    &self.last_lod_tiers,
                );
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
            let edge_bounds = edge_visible_bounds(source, target);
            if rects_intersect(&edge_bounds, &viewport) {
                draw_list.visible_edge_count += 1;
                draw_list.record_tier(&edge.id, &edge_bounds, &self.camera, &self.last_lod_tiers);
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
                draw_list.record_tier(&card.id, &card.bounds, &self.camera, &self.last_lod_tiers);
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

    pub(crate) fn write_uniform(&self) {
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
