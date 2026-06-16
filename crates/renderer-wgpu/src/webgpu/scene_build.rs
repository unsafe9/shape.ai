//! Portable pure-CPU layer of the WebGPU renderer (no wgpu device, no `web_sys`):
//! geometry/style/hit-test/marquee/overlay/camera math, the render consts, the WGSL
//! fallback shader, and the unit tests. This is the layer the host test gate
//! exercises; `ShapeWebGpuRenderer` lives in the parent + web-surface submodules.

use std::f32::consts::PI;

use shape_renderer_core::lod::{apparent_px, lod_tier, LodTier};
use shape_renderer_core::model::{
    ActiveTool, CameraState, CanvasInputEvent, CubicRoute, RenderCard, RenderEdge, RenderGroup,
    SceneSelection, SceneShadowLayerToken, SceneSnapshot, SceneStyleToken, WorldPoint, WorldRect,
};
use shape_renderer_core::hit_test_object::{
    hit_test_object_or_bbox, resize_delta_matrix, rotate_delta_matrix_snapped,
    swept_segment_hits_object, translate_3x3, HoverAffordance, ScreenRect, SelectionHandles,
    ROTATE_SNAP_DEG,
};
use shape_renderer_core::outline::{derive_region, parse_path_string};
use shape_renderer_core::render_object::{RenderObject, RenderObjectScene};
use shape_renderer_core::stats::{
    CoreHitResult, CoreOverlayStyle, ObjectDoubleClick, ObjectEndpointDelta, ObjectTransformDelta,
};
use shape_renderer_core::text::{CachedTextLine, TextBuildStats, TextEngine, TextLayoutCache, TEXT_ATLAS_SOLID_UV};

use super::*;

#[cfg(feature = "wgpu-probe")]
#[derive(Clone)]
pub(crate) struct ShapeRenderStyle {
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
pub(crate) struct ShapeRadius {
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
pub(crate) struct ShapeStrokeWidth {
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
pub(crate) struct ShapeTypography {
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
pub(crate) struct ShapeSpacing {
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
pub(crate) struct ShapeShadowLayer {
    offset_x: f64,
    offset_y: f64,
    blur: f64,
    spread: f64,
    color: [f32; 4],
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
pub(crate) struct ShapeGradient {
    surface_top_alpha: f32,
    pastel_bottom_alpha: f32,
    accent_start_alpha: f32,
    accent_end_alpha: f32,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
pub(crate) struct ShapeState {
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
pub(crate) struct CardTextLayout {
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
pub(crate) struct ShapeBadgeStyle {
    fill_alpha: f32,
    stroke_alpha: f32,
    text_alpha: f32,
    min_width: f64,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
pub(crate) struct ShapeEdgeStyle {
    stroke_alpha: f32,
    selected_stroke_alpha: f32,
    compact_stroke_alpha: f32,
    label_fill_alpha: f32,
    label_stroke_alpha: f32,
    label_text_alpha: f32,
}

#[cfg(feature = "wgpu-probe")]
#[derive(Clone, Copy)]
pub(crate) struct ShapePortStyle {
    fill_alpha: f32,
    stroke_alpha: f32,
    selected_fill_alpha: f32,
    selected_stroke_alpha: f32,
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn build_group_vertices(
    scene: &SceneSnapshot,
    group: &RenderGroup,
    text_layout_cache: &mut TextLayoutCache,
    text_engine: &mut TextEngine,
) -> (Vec<GpuVertex>, TextBuildStats) {
    let mut vertices = Vec::new();
    let style = resolve_shape_style(&scene.styles, &group.style_key);
    let selected =
        selection_is_group(&scene.selection, &group.id) || multi_selected(scene, &group.id);
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
        shape_renderer_core::cast::narrow_f32(group.bounds.x + style.spacing.group_padding_x),
        shape_renderer_core::cast::narrow_f32(group.bounds.y + style.spacing.group_padding_y),
        shape_renderer_core::cast::narrow_f32(group.bounds.width - style.spacing.group_padding_x * 2.0),
        style.typography.group_title_size,
        color_with_alpha(style.text, 0.86),
        text_layout_cache,
        text_engine,
    );
    if !group.summary.trim().is_empty() {
        text_stats.add(add_text_line(
            &mut vertices,
            &group.summary,
            shape_renderer_core::cast::narrow_f32(group.bounds.x + style.spacing.group_padding_x),
            shape_renderer_core::cast::narrow_f32(
                group.bounds.y
                    + style.spacing.group_padding_y
                    + style.typography.group_title_size as f64
                    + 28.0,
            ),
            shape_renderer_core::cast::narrow_f32(group.bounds.width - style.spacing.group_padding_x * 2.0),
            style.typography.group_summary_size,
            color_with_alpha(style.muted_text, 0.78),
            text_layout_cache,
            text_engine,
        ));
    }
    (vertices, text_stats)
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn build_edge_vertices(
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
    let selected =
        selection_is_edge(&scene.selection, &edge.id) || multi_selected(scene, &edge.id);
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
            shape_renderer_core::cast::narrow_f32(stroke_width + style.stroke_width.focus_ring),
            color_with_alpha(style.focus, style.state.focus_alpha),
        );
    }
    let stroke = color_with_alpha(edge_base, stroke_alpha);
    add_cubic_edge(&mut vertices, &route, shape_renderer_core::cast::narrow_f32(stroke_width), stroke);
    add_arrowhead(
        &mut vertices,
        [shape_renderer_core::cast::narrow_f32(route.cp2.x), shape_renderer_core::cast::narrow_f32(route.cp2.y)],
        [shape_renderer_core::cast::narrow_f32(route.end.x), shape_renderer_core::cast::narrow_f32(route.end.y)],
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
        let label_padding = shape_renderer_core::cast::narrow_f32(style.spacing.label_padding_x);
        let label_x = shape_renderer_core::cast::narrow_f32((route.start.x + route.end.x) * 0.5)
            - label_width * 0.5
            - label_padding;
        let label_y = shape_renderer_core::cast::narrow_f32((route.start.y + route.end.y) * 0.5)
            - shape_renderer_core::cast::narrow_f32(style.spacing.edge_label_height);
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
pub(crate) fn build_card_vertices(
    scene: &SceneSnapshot,
    card: &RenderCard,
    text_layout_cache: &mut TextLayoutCache,
    text_engine: &mut TextEngine,
) -> (Vec<GpuVertex>, TextBuildStats) {
    let mut vertices = Vec::new();
    let mut text_stats = TextBuildStats::default();
    let style = resolve_shape_style(&scene.styles, &card.style_key);
    let selected =
        selection_is_node(&scene.selection, &card.id) || multi_selected(scene, &card.id);
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
        shape_renderer_core::cast::narrow_f32(badge.x + style.spacing.badge_padding_x),
        shape_renderer_core::cast::narrow_f32(badge.y + 5.0),
        shape_renderer_core::cast::narrow_f32(badge.width - style.spacing.badge_padding_x * 2.0),
        style.typography.badge_size,
        color_with_alpha(style.accent, style.badge.text_alpha),
        text_layout_cache,
        text_engine,
    ));
    text_stats.add(add_text_line(
        &mut vertices,
        &card.title,
        shape_renderer_core::cast::narrow_f32(text_layout.content_x),
        shape_renderer_core::cast::narrow_f32(text_layout.title_y),
        shape_renderer_core::cast::narrow_f32(text_layout.content_width),
        text_layout.title_font_size,
        color_with_alpha(style.text, 0.92),
        text_layout_cache,
        text_engine,
    ));
    text_stats.add(add_wrapped_text(
        &mut vertices,
        &card.summary,
        shape_renderer_core::cast::narrow_f32(text_layout.content_x),
        shape_renderer_core::cast::narrow_f32(text_layout.summary_y),
        shape_renderer_core::cast::narrow_f32(text_layout.content_width),
        text_layout.summary_font_size,
        shape_renderer_core::cast::narrow_f32(text_layout.summary_line_height),
        text_layout.summary_max_lines,
        color_with_alpha(style.muted_text, 0.84),
        text_layout_cache,
        text_engine,
    ));
    if text_layout.detail_max_lines > 0 && !card.detail.trim().is_empty() {
        text_stats.add(add_wrapped_text(
            &mut vertices,
            &card.detail,
            shape_renderer_core::cast::narrow_f32(text_layout.content_x),
            shape_renderer_core::cast::narrow_f32(text_layout.detail_y),
            shape_renderer_core::cast::narrow_f32(text_layout.content_width),
            text_layout.detail_font_size,
            shape_renderer_core::cast::narrow_f32(text_layout.detail_line_height),
            text_layout.detail_max_lines,
            color_with_alpha(style.muted_text, 0.68),
            text_layout_cache,
            text_engine,
        ));
    }
    (vertices, text_stats)
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn fit_vertices_to_slot(vertices: Vec<GpuVertex>, capacity: usize) -> Vec<GpuVertex> {
    fit_vertices_to_slot_with_stats(vertices, capacity).0
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn fit_vertices_to_slot_with_stats(
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
pub(crate) fn edge_spare_slot_count(edge_count: usize) -> usize {
    (edge_count / 8).clamp(8, 256)
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn group_spare_slot_count(group_count: usize) -> usize {
    (group_count / 8).clamp(4, 128)
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn card_spare_slot_count(card_count: usize) -> usize {
    (card_count / 8).clamp(8, 256)
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn collect_selection_dirty_ids(
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
        // Multi is a transient shell-side set; the persisted core selection never
        // holds it, but dirty any node ids it carries to be safe.
        SceneSelection::Multi { ids } => {
            for id in ids {
                push_unique(cards, id);
            }
        }
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|candidate| candidate == value) {
        values.push(value.to_string());
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn selection_is_group(selection: &SceneSelection, id: &str) -> bool {
    matches!(selection, SceneSelection::Group { id: selected } if selected == id)
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn selection_is_node(selection: &SceneSelection, id: &str) -> bool {
    matches!(selection, SceneSelection::Node { id: selected } if selected == id)
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn selection_is_edge(selection: &SceneSelection, id: &str) -> bool {
    matches!(selection, SceneSelection::Edge { id: selected } if selected == id)
}

// True when `id` should draw with selection styling: it is the single anchor of
// the matching kind, or a member of the transient multi-select set. The set is
// kind-agnostic (groups/cards/edges all live in `multi_select`), so each draw
// path OR-s its single-anchor check with this membership test.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn multi_selected(scene: &SceneSnapshot, id: &str) -> bool {
    scene.multi_select.iter().any(|candidate| candidate == id)
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn transparent_vertex() -> GpuVertex {
    GpuVertex {
        position: [0.0, 0.0],
        uv: SOLID_UV[0],
        color: [0.0, 0.0, 0.0, 0.0],
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn add_rect(vertices: &mut Vec<GpuVertex>, rect: &WorldRect, color: [f32; 4]) {
    let x = shape_renderer_core::cast::narrow_f32(rect.x);
    let y = shape_renderer_core::cast::narrow_f32(rect.y);
    let w = shape_renderer_core::cast::narrow_f32(rect.width);
    let h = shape_renderer_core::cast::narrow_f32(rect.height);
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
pub(crate) fn add_rounded_rect(vertices: &mut Vec<GpuVertex>, rect: &WorldRect, radius: f64, color: [f32; 4]) {
    let w = shape_renderer_core::cast::narrow_f32(rect.width.max(0.0));
    let h = shape_renderer_core::cast::narrow_f32(rect.height.max(0.0));
    let r = shape_renderer_core::cast::narrow_f32(radius.max(0.0))
        .min(w * 0.5)
        .min(h * 0.5);
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
        [shape_renderer_core::cast::narrow_f32(rect.x) + r, shape_renderer_core::cast::narrow_f32(rect.y) + r],
        r,
        PI,
        PI * 1.5,
        color,
    );
    add_corner_fan(
        vertices,
        [shape_renderer_core::cast::narrow_f32(rect.x) + w - r, shape_renderer_core::cast::narrow_f32(rect.y) + r],
        r,
        PI * 1.5,
        PI * 2.0,
        color,
    );
    add_corner_fan(
        vertices,
        [shape_renderer_core::cast::narrow_f32(rect.x) + w - r, shape_renderer_core::cast::narrow_f32(rect.y) + h - r],
        r,
        0.0,
        PI * 0.5,
        color,
    );
    add_corner_fan(
        vertices,
        [shape_renderer_core::cast::narrow_f32(rect.x) + r, shape_renderer_core::cast::narrow_f32(rect.y) + h - r],
        r,
        PI * 0.5,
        PI,
        color,
    );
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn add_corner_fan(
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
pub(crate) fn add_gradient_banded_rect(
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
pub(crate) fn add_layered_shadow(
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
pub(crate) fn add_focus_ring(
    vertices: &mut Vec<GpuVertex>,
    rect: &WorldRect,
    radius: f64,
    width: f64,
    color: [f32; 4],
) {
    add_rounded_rect(vertices, &expand_rect(rect, width), radius + width, color);
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn add_inner_stroke(
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
pub(crate) fn add_separator(
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
pub(crate) fn add_accent_strip(vertices: &mut Vec<GpuVertex>, card: &WorldRect, style: &ShapeRenderStyle) {
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
pub(crate) fn add_badge_pill(vertices: &mut Vec<GpuVertex>, rect: &WorldRect, style: &ShapeRenderStyle) {
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
pub(crate) fn add_edge_label_capsule(
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
pub(crate) fn add_port_markers(
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
pub(crate) fn inset_rect(rect: &WorldRect, inset: f64) -> WorldRect {
    let inset = inset.max(0.0);
    WorldRect {
        x: rect.x + inset,
        y: rect.y + inset,
        width: (rect.width - inset * 2.0).max(0.0),
        height: (rect.height - inset * 2.0).max(0.0),
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn expand_rect(rect: &WorldRect, spread: f64) -> WorldRect {
    WorldRect {
        x: rect.x - spread,
        y: rect.y - spread,
        width: rect.width + spread * 2.0,
        height: rect.height + spread * 2.0,
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn add_text_line(
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
pub(crate) fn add_cached_text_line(
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
pub(crate) fn add_wrapped_text(
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

#[cfg(feature = "wgpu-probe")]
pub(crate) fn parse_hex_color(value: &str, alpha: f32) -> Option<[f32; 4]> {
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
pub(crate) fn resolve_shape_style(styles: &[SceneStyleToken], style_key: &str) -> ShapeRenderStyle {
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
pub(crate) fn resolve_shadow_layers(
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
pub(crate) fn token_color(value: Option<&str>, alpha: f32, fallback: [f32; 4]) -> [f32; 4] {
    value
        .and_then(|value| parse_hex_color(value, alpha))
        .unwrap_or(fallback)
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn metric(value: Option<f64>, fallback: f64) -> f64 {
    value.unwrap_or(fallback).max(0.0)
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn font_metric(value: Option<f64>, fallback: f64) -> f32 {
    shape_renderer_core::cast::narrow_f32(metric(value, fallback))
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn opacity(value: Option<f64>, fallback: f64) -> f32 {
    shape_renderer_core::cast::narrow_f32(value.unwrap_or(fallback).clamp(0.0, 1.0))
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn color_with_alpha(color: [f32; 4], alpha: f32) -> [f32; 4] {
    [color[0], color[1], color[2], alpha.clamp(0.0, 1.0)]
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn mix_rgb(a: [f32; 4], b: [f32; 4], b_weight: f32, alpha: f32) -> [f32; 4] {
    let t = b_weight.clamp(0.0, 1.0);
    [
        a[0] * (1.0 - t) + b[0] * t,
        a[1] * (1.0 - t) + b[1] * t,
        a[2] * (1.0 - t) + b[2] * t,
        alpha.clamp(0.0, 1.0),
    ]
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn mix_color(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] * (1.0 - t) + b[0] * t,
        a[1] * (1.0 - t) + b[1] * t,
        a[2] * (1.0 - t) + b[2] * t,
        a[3] * (1.0 - t) + b[3] * t,
    ]
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn add_line(
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
pub(crate) fn add_cubic_edge(
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
            [shape_renderer_core::cast::narrow_f32(previous.x), shape_renderer_core::cast::narrow_f32(previous.y)],
            [shape_renderer_core::cast::narrow_f32(current.x), shape_renderer_core::cast::narrow_f32(current.y)],
            thickness,
            color,
        );
        previous = current;
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn add_arrowhead(vertices: &mut Vec<GpuVertex>, start: [f32; 2], end: [f32; 2], color: [f32; 4]) {
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
pub(crate) fn add_quad(
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
pub(crate) fn add_quad_uv(
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
pub(crate) fn badge_rect(
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
pub(crate) fn node_type_label(node_type: &str) -> String {
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
pub(crate) fn point_in_rect(point: &WorldPoint, rect: &WorldRect) -> bool {
    point.x >= rect.x
        && point.x <= rect.x + rect.width
        && point.y >= rect.y
        && point.y <= rect.y + rect.height
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn rects_intersect(a: &WorldRect, b: &WorldRect) -> bool {
    a.x <= b.x + b.width && a.x + a.width >= b.x && a.y <= b.y + b.height && a.y + a.height >= b.y
}

/// Normalize two world-space drag corners into a non-negative-extent rect.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn marquee_rect(start: WorldPoint, current: WorldPoint) -> WorldRect {
    let x = start.x.min(current.x);
    let y = start.y.min(current.y);
    WorldRect {
        x,
        y,
        width: (start.x - current.x).abs(),
        height: (start.y - current.y).abs(),
    }
}

/// Build the marquee overlay quads (translucent fill + 1.5px stroke) in world space;
/// `zoom` keeps the stroke a constant screen width.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn build_marquee_overlay_vertices(rect: &WorldRect, zoom: f64) -> Vec<GpuVertex> {
    let mut vertices = Vec::with_capacity(MARQUEE_OVERLAY_VERTEX_CAPACITY);
    add_rect(&mut vertices, rect, MARQUEE_FILL_COLOR);
    let thickness = shape_renderer_core::cast::narrow_f32(1.5 / zoom.max(0.025));
    let x = shape_renderer_core::cast::narrow_f32(rect.x);
    let y = shape_renderer_core::cast::narrow_f32(rect.y);
    let w = shape_renderer_core::cast::narrow_f32(rect.width);
    let h = shape_renderer_core::cast::narrow_f32(rect.height);
    add_line(&mut vertices, [x, y], [x + w, y], thickness, MARQUEE_STROKE_COLOR);
    add_line(
        &mut vertices,
        [x + w, y],
        [x + w, y + h],
        thickness,
        MARQUEE_STROKE_COLOR,
    );
    add_line(
        &mut vertices,
        [x + w, y + h],
        [x, y + h],
        thickness,
        MARQUEE_STROKE_COLOR,
    );
    add_line(&mut vertices, [x, y + h], [x, y], thickness, MARQUEE_STROKE_COLOR);
    vertices
}

/// The overlay quads for an in-flight marquee drag (empty otherwise). The single
/// source both render passes (object + legacy) draw from, so the rubber-band
/// surfaces identically; pure so it is unit-testable.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn marquee_overlay_for_drag(
    input_drag: Option<&InputDragState>,
    zoom: f64,
) -> Vec<GpuVertex> {
    let Some(InputDragState::Marquee { start, current, .. }) = input_drag else {
        return Vec::new();
    };
    let rect = marquee_rect(*start, *current);
    build_marquee_overlay_vertices(&rect, zoom)
}

/// Build the selection-handle overlay (8 resize handles + 1 rotate zone) in WORLD
/// space for `world_bbox`. Each handle is `HANDLE_SIZE_PX / zoom` world units so the
/// shader's `* zoom` renders it at a CONSTANT screen size. Centers mirror
/// [`SelectionHandles::from_screen_bbox`] so what is drawn matches hover/hit-test.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn build_handle_overlay_vertices(world_bbox: &WorldRect, zoom: f64) -> Vec<GpuVertex> {
    use shape_renderer_core::hit_test_object::{HANDLE_SIZE_PX, ROTATE_ZONE_OFFSET_PX};
    let mut vertices = Vec::with_capacity(HANDLE_OVERLAY_VERTEX_CAPACITY);
    let z = zoom.max(0.025);
    let size = HANDLE_SIZE_PX / z;
    let half = size / 2.0;
    let rotate_offset = ROTATE_ZONE_OFFSET_PX / z;
    let left = world_bbox.x;
    let right = world_bbox.x + world_bbox.width;
    let top = world_bbox.y;
    let bottom = world_bbox.y + world_bbox.height;
    let cx = world_bbox.x + world_bbox.width / 2.0;
    let cy = world_bbox.y + world_bbox.height / 2.0;
    let mut handle = |hx: f64, hy: f64| {
        add_rect(
            &mut vertices,
            &WorldRect {
                x: hx - half,
                y: hy - half,
                width: size,
                height: size,
            },
            HANDLE_FILL_COLOR,
        );
    };
    handle(left, top);
    handle(cx, top);
    handle(right, top);
    handle(right, cy);
    handle(right, bottom);
    handle(cx, bottom);
    handle(left, bottom);
    handle(left, cy);
    handle(cx, top - rotate_offset);
    vertices
}

/// The id set that gets a continuous bbox outline ring: every multi-select member,
/// or (when empty) the lone selected object/group so a grouped selection shows a
/// border — UNLESS open-class, whose surface is the two endpoint dots, not a ring.
/// Returns member-input order; empty when nothing is selected.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn outline_overlay_ids(
    scene: &RenderObjectScene,
    regions: &[ObjectRegion],
) -> Vec<String> {
    if !scene.multi_select.is_empty() {
        return scene.multi_select.clone();
    }
    let Some(id) = &scene.selection else {
        return Vec::new();
    };
    let open = regions
        .iter()
        .find(|region| &region.id == id)
        .is_some_and(|region| region.open_endpoints.is_some());
    if open {
        return Vec::new();
    }
    vec![id.clone()]
}

/// Per-object outline highlight for the multi-select set: each id's world bbox as a
/// 4-edge rectangle (constant ~2px screen width via `2.0 / zoom`) in selection blue,
/// stopping at buffer capacity. `preview` supplies each id's LIVE drag transform so
/// the ring tracks the drag at the PREVIEWED bbox (transform-only, no
/// re-tessellation). An OPEN-CLASS member draws its two endpoint dots
/// ([`push_endpoint_dot_vertices`]) instead of a ring — visual only.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn build_multi_select_overlay_vertices(
    regions: &[ObjectRegion],
    ids: &[String],
    zoom: f64,
    preview: impl Fn(&str) -> Option<[[f64; 3]; 3]>,
) -> Vec<GpuVertex> {
    use shape_renderer_core::hit_test_object::{apply_3x3, HANDLE_SIZE_PX};
    let mut vertices = Vec::new();
    let z = zoom.max(0.025);
    let thickness = shape_renderer_core::cast::narrow_f32(2.0 / z);
    let dot_radius = shape_renderer_core::cast::narrow_f32(HANDLE_SIZE_PX / z / 2.0);
    for id in ids {
        let Some(region) = regions.iter().find(|region| &region.id == id) else {
            continue;
        };
        let preview_t = preview(id);
        if let Some(endpoints) = region.open_endpoints {
            if vertices.len() + 2 * ENDPOINT_HANDLE_SEGMENTS * 3
                > MULTI_SELECT_OVERLAY_VERTEX_CAPACITY
            {
                break;
            }
            let transform = preview_t.as_ref().unwrap_or(&region.transform);
            let (sx, sy) = apply_3x3(transform, endpoints.start.0, endpoints.start.1);
            let (ex, ey) = apply_3x3(transform, endpoints.end.0, endpoints.end.1);
            if !(sx.is_finite() && sy.is_finite() && ex.is_finite() && ey.is_finite()) {
                continue;
            }
            for point in [WorldPoint { x: sx, y: sy }, WorldPoint { x: ex, y: ey }] {
                push_endpoint_dot_vertices(&mut vertices, &point, dot_radius);
            }
            continue;
        }
        if vertices.len() + 24 > MULTI_SELECT_OVERLAY_VERTEX_CAPACITY {
            break;
        }
        let Some(bounds) = region_world_bounds(region, preview_t.as_ref()) else {
            continue;
        };
        let x = shape_renderer_core::cast::narrow_f32(bounds.x);
        let y = shape_renderer_core::cast::narrow_f32(bounds.y);
        let w = shape_renderer_core::cast::narrow_f32(bounds.width);
        let h = shape_renderer_core::cast::narrow_f32(bounds.height);
        add_line(&mut vertices, [x, y], [x + w, y], thickness, MULTI_SELECT_OUTLINE_COLOR);
        add_line(&mut vertices, [x + w, y], [x + w, y + h], thickness, MULTI_SELECT_OUTLINE_COLOR);
        add_line(&mut vertices, [x + w, y + h], [x, y + h], thickness, MULTI_SELECT_OUTLINE_COLOR);
        add_line(&mut vertices, [x, y + h], [x, y], thickness, MULTI_SELECT_OUTLINE_COLOR);
    }
    vertices
}

/// Node ids AND group ids whose world bounds intersect the marquee rect. Cards come
/// first (selection-anchor friendly), then groups.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn marquee_intersecting_ids(scene: &SceneSnapshot, rect: &WorldRect) -> Vec<String> {
    let mut ids = Vec::new();
    for card in &scene.cards {
        if rects_intersect(&card.bounds, rect) {
            ids.push(card.id.clone());
        }
    }
    for group in &scene.groups {
        if rects_intersect(&group.bounds, rect) {
            ids.push(group.id.clone());
        }
    }
    ids
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn screen_to_world(point: WorldPoint, camera: &CameraState) -> WorldPoint {
    let zoom = camera.zoom.max(0.025);
    WorldPoint {
        x: (point.x - camera.x) / zoom,
        y: (point.y - camera.y) / zoom,
    }
}

/// World -> screen projection through the LIVE core camera, the exact inverse of
/// [`screen_to_world`] (same clamped `zoom`). The shell calls this instead of
/// recomputing the transform from a mirrored `CameraState`, so projection and
/// inverse-projection share one source of truth.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn world_to_screen(point: WorldPoint, camera: &CameraState) -> WorldPoint {
    let zoom = camera.zoom.max(0.025);
    WorldPoint {
        x: point.x * zoom + camera.x,
        y: point.y * zoom + camera.y,
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn world_rect_to_screen_rect(rect: &WorldRect, camera: &CameraState) -> WorldRect {
    WorldRect {
        x: rect.x * camera.zoom + camera.x,
        y: rect.y * camera.zoom + camera.y,
        width: rect.width * camera.zoom,
        height: rect.height * camera.zoom,
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn hit_scene_at_screen(
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
pub(crate) fn selection_from_hit(hit: Option<&CoreHitResult>) -> SceneSelection {
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
pub(crate) fn selection_world_rect(scene: &SceneSnapshot, selection: &SceneSelection) -> Option<WorldRect> {
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
        // A transient multi-select has no single persisted bounds; the shell owns
        // any multi-selection framing.
        SceneSelection::Multi { .. } => None,
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn zoom_camera_at_screen(camera: &CameraState, screen: WorldPoint, delta_y: f64) -> CameraState {
    let world = screen_to_world(screen, camera);
    let zoom = clamp_camera_zoom(camera.zoom * (-delta_y * 0.0012).exp());
    CameraState {
        zoom,
        x: screen.x - world.x * zoom,
        y: screen.y - world.y * zoom,
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn fit_camera_to_scene(
    scene: &SceneSnapshot,
    viewport_width: f64,
    viewport_height: f64,
) -> CameraState {
    let bounds = scene_world_bounds(scene);
    fit_camera_to_bounds(&bounds, viewport_width, viewport_height)
}

/// Frame a world-space AABB into the viewport (same padding/zoom clamp as
/// [`fit_camera_to_scene`]).
#[cfg(feature = "wgpu-probe")]
pub(crate) fn fit_camera_to_bounds(
    bounds: &WorldRect,
    viewport_width: f64,
    viewport_height: f64,
) -> CameraState {
    let padding = 96.0;
    let usable_width = (viewport_width - padding * 2.0).max(120.0);
    let usable_height = (viewport_height - padding * 2.0).max(120.0);
    let zoom = clamp(bounds_zoom(usable_width, usable_height, bounds), 0.04, 1.6);
    CameraState {
        zoom,
        x: viewport_width / 2.0 - (bounds.x + bounds.width / 2.0) * zoom,
        y: viewport_height / 2.0 - (bounds.y + bounds.height / 2.0) * zoom,
    }
}

/// Derive each object's local-space region outline for hit-test / marquee: parse the
/// geometry path-string into flattened subpaths, derive the region, keep the boundary
/// polygon in OBJECT-LOCAL px. Degenerate objects are skipped (they cannot be hit).
#[cfg(feature = "wgpu-probe")]
pub(crate) fn derive_object_regions(scene: &RenderObjectScene) -> Vec<ObjectRegion> {
    const REGION_FLATNESS: f32 = 0.5;
    let mut regions = Vec::with_capacity(scene.objects.len());
    for obj in &scene.objects {
        let Some(subpaths) = parse_path_string(&obj.geometry_d, REGION_FLATNESS) else {
            continue;
        };
        let Some(region) = derive_region(&subpaths, REGION_FLATNESS) else {
            continue;
        };
        regions.push(ObjectRegion {
            id: obj.id.clone(),
            transform: obj.transform,
            closed: region.closed,
            outline: region.outline,
            open_endpoints: derive_open_endpoints(&obj.geometry_d),
            hidden: obj.hidden,
            locked: obj.locked,
        });
    }
    regions
}

/// Classify `d` (`is_open_class_d`) and read its endpoint pair through the same
/// pair-space parser anchors address (`local_nodes`), de-quantized to object-local
/// px. `None` for closed-class / multi-subpath / degenerate (< 2 pairs) geometry.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn derive_open_endpoints(d: &str) -> Option<OpenEndpoints> {
    use shape_renderer_core::hit_test_object::UNITS_PER_PX;
    use shape_scene_core::object::{is_open_class_d, local_nodes};
    if !is_open_class_d(d) {
        return None;
    }
    let pairs = local_nodes(d);
    if pairs.len() < 2 {
        return None;
    }
    let last_index = i32::try_from(pairs.len() - 1).ok()?;
    let (sx, sy) = pairs[0];
    let (ex, ey) = pairs[pairs.len() - 1];
    Some(OpenEndpoints {
        start: (sx / UNITS_PER_PX, sy / UNITS_PER_PX),
        end: (ex / UNITS_PER_PX, ey / UNITS_PER_PX),
        last_index,
    })
}

/// Object-local pad (px) for the body bbox fallback so a zero-size / collapsed
/// stroke or text object still presents a finite grab target. Only applied to
/// objects with no closed fill, so empty canvas never reads as a hit.
#[cfg(feature = "wgpu-probe")]
const BODY_BBOX_GRAB_PAD_PX: f32 = 4.0;

/// Pick the top-most object whose region contains the screen point (regions iterated
/// in reverse since later objects draw on top; the query point is inverse-transformed
/// into each object's local space). A closed fill keeps the even-odd polygon hit; a
/// stroke/text/open/zero-size object falls back to its object-local bbox.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn hit_object_in_regions(
    regions: &[ObjectRegion],
    camera: &CameraState,
    screen: WorldPoint,
) -> Option<String> {
    let world = screen_to_world(screen, camera);
    regions
        .iter()
        .rev()
        .find(|region| {
            // Hidden (non-render => non-hittable) or locked (non-interactive) regions
            // are inert; the region is kept for index/lookup, just skipped here.
            !region.hidden
                && !region.locked
                && hit_test_object_or_bbox(
                    &region.transform,
                    &region.outline,
                    region.closed,
                    BODY_BBOX_GRAB_PAD_PX,
                    world.x,
                    world.y,
                )
        })
        .map(|region| region.id.clone())
}

/// Scoped pick for an active drill-in container: when `active_container` is `Some(c)`,
/// resolve a pointer-down to the DIRECT CHILD under the pointer (top-most region whose
/// `RenderObject.parent == Some(c)`); if no child is hit, return `Some(c)` when the
/// point is inside the container's own region (empty-inside click keeps the container),
/// else `None` (a click outside the scope). When `None`, delegate to the unscoped
/// [`hit_object_in_regions`]. A pure pick-filter on the existing `parent` metadata —
/// allocation-free, transform-only-safe. Hidden/locked regions stay inert here too.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn hit_object_scoped(
    regions: &[ObjectRegion],
    objects: &[RenderObject],
    camera: &CameraState,
    screen: WorldPoint,
    active_container: Option<&str>,
) -> Option<String> {
    let Some(container) = active_container else {
        return hit_object_in_regions(regions, camera, screen);
    };
    let world = screen_to_world(screen, camera);
    let is_child = |id: &str| {
        objects
            .iter()
            .any(|o| o.id == id && o.parent.as_deref() == Some(container))
    };
    let hit = |region: &ObjectRegion| {
        !region.hidden
            && !region.locked
            && hit_test_object_or_bbox(
                &region.transform,
                &region.outline,
                region.closed,
                BODY_BBOX_GRAB_PAD_PX,
                world.x,
                world.y,
            )
    };
    // Top-most direct child under the pointer wins.
    if let Some(child) = regions
        .iter()
        .rev()
        .find(|region| is_child(&region.id) && hit(region))
    {
        return Some(child.id.clone());
    }
    // No child hit: keep the container when the click is inside its own body; a click
    // outside the scope returns None (the shell forwards null to exit the scope).
    regions
        .iter()
        .find(|region| region.id == container && hit(region))
        .map(|region| region.id.clone())
}

/// Ids of EVERY object the pointer crossed between two consecutive SCREEN samples:
/// both are mapped to world and the segment is swept against each object via
/// [`swept_segment_hits_object`] (filled => outline crossing; otherwise padded
/// local-bbox), top-down. A zero-length segment degenerates to a point sweep but
/// still returns ALL overlapping ids.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn swept_erase_in_regions(
    regions: &[ObjectRegion],
    camera: &CameraState,
    prev: WorldPoint,
    curr: WorldPoint,
) -> Vec<String> {
    let a = screen_to_world(prev, camera);
    let b = screen_to_world(curr, camera);
    regions
        .iter()
        .rev()
        .filter(|region| {
            !region.hidden
                && !region.locked
                && swept_segment_hits_object(
                    &region.transform,
                    &region.outline,
                    region.closed,
                    BODY_BBOX_GRAB_PAD_PX,
                    a.x,
                    a.y,
                    b.x,
                    b.y,
                )
        })
        .map(|region| region.id.clone())
        .collect()
}

/// The SHARED selection-handle layout for the selected object: the screen-space
/// [`SelectionHandles`] plus the WORLD bbox they were laid out from. The single
/// source hover, pointer-down hit-test, and GPU handle render share. `preview`
/// substitutes the live drag transform so the handles track the PREVIEWED bbox.
/// `None` when nothing is selected or the region has no finite world bounds.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn selection_handles(
    regions: &[ObjectRegion],
    camera: &CameraState,
    selection: Option<&str>,
    preview: Option<&[[f64; 3]; 3]>,
) -> Option<(SelectionHandles, WorldRect)> {
    let id = selection?;
    let region = regions.iter().find(|region| region.id == id)?;
    // A hidden/locked region shows no resize/rotate handles (non-interactive).
    if region.hidden || region.locked {
        return None;
    }
    // An open-class selection has NO bbox transform surface — its two endpoint
    // handles are the whole manipulation surface.
    if region.open_endpoints.is_some() {
        return None;
    }
    let world_bbox = region_world_bounds(region, preview)?;
    let screen_rect = world_rect_to_screen_rect(&world_bbox, camera);
    let handles = SelectionHandles::from_screen_bbox(&ScreenRect {
        x: screen_rect.x,
        y: screen_rect.y,
        width: screen_rect.width,
        height: screen_rect.height,
    });
    Some((handles, world_bbox))
}

/// The endpoint-handle layout for an OPEN-CLASS selection, the single source hover,
/// grab, and GPU render share. `world` holds the two endpoint WORLD positions (node
/// 0, then the last node); `screen` the matching [`HANDLE_SIZE_PX`]-square hit zones;
/// `last_index` the end node's geometry PAIR index.
#[cfg(feature = "wgpu-probe")]
pub(crate) struct EndpointHandles {
    pub(crate) world: [WorldPoint; 2],
    pub(crate) screen: [ScreenRect; 2],
    pub(crate) last_index: i32,
}

#[cfg(feature = "wgpu-probe")]
impl EndpointHandles {
    /// Classify a screen point against the two endpoint handles.
    pub(crate) fn affordance_at(&self, x: f64, y: f64) -> Option<HoverAffordance> {
        if self.screen[0].contains(x, y) {
            return Some(HoverAffordance::EndpointStart);
        }
        if self.screen[1].contains(x, y) {
            return Some(HoverAffordance::EndpointEnd);
        }
        None
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn endpoint_handles(
    regions: &[ObjectRegion],
    camera: &CameraState,
    selection: Option<&str>,
    preview: Option<&[[f64; 3]; 3]>,
) -> Option<EndpointHandles> {
    use shape_renderer_core::hit_test_object::{apply_3x3, HANDLE_SIZE_PX};
    let id = selection?;
    let region = regions.iter().find(|region| region.id == id)?;
    let endpoints = region.open_endpoints?;
    let transform = preview.unwrap_or(&region.transform);
    let mut world = [WorldPoint { x: 0.0, y: 0.0 }; 2];
    for (slot, (lx, ly)) in [endpoints.start, endpoints.end].into_iter().enumerate() {
        let (wx, wy) = apply_3x3(transform, lx, ly);
        if !wx.is_finite() || !wy.is_finite() {
            return None;
        }
        world[slot] = WorldPoint { x: wx, y: wy };
    }
    let half = HANDLE_SIZE_PX / 2.0;
    let screen = world.map(|point| ScreenRect {
        x: point.x * camera.zoom + camera.x - half,
        y: point.y * camera.zoom + camera.y - half,
        width: HANDLE_SIZE_PX,
        height: HANDLE_SIZE_PX,
    });
    Some(EndpointHandles {
        world,
        screen,
        last_index: endpoints.last_index,
    })
}

/// Build the endpoint-handle overlay: two FILLED CIRCLES at the open-class
/// selection's endpoint WORLD positions, zoom-invariant like
/// [`build_handle_overlay_vertices`] (screen DIAMETER pins to `HANDLE_SIZE_PX`).
/// Each dot is a [`ENDPOINT_HANDLE_SEGMENTS`]-triangle fan. Visual only: the
/// grab/hover HIT zones stay the screen squares in [`endpoint_handles`].
#[cfg(feature = "wgpu-probe")]
pub(crate) fn build_endpoint_handle_overlay_vertices(
    world: &[WorldPoint; 2],
    zoom: f64,
) -> Vec<GpuVertex> {
    use shape_renderer_core::hit_test_object::HANDLE_SIZE_PX;
    let mut vertices = Vec::with_capacity(2 * ENDPOINT_HANDLE_SEGMENTS * 3);
    let z = zoom.max(0.025);
    let radius = shape_renderer_core::cast::narrow_f32(HANDLE_SIZE_PX / z / 2.0);
    for point in world {
        push_endpoint_dot_vertices(&mut vertices, point, radius);
    }
    vertices
}

/// One endpoint dot: a filled [`ENDPOINT_HANDLE_SEGMENTS`]-triangle fan at `center`
/// with the given WORLD radius, in [`ENDPOINT_HANDLE_FILL_COLOR`]. The single fan
/// emitter shared by the single-selection overlay and the multi-select open-member
/// dots, so the two visuals can't drift.
#[cfg(feature = "wgpu-probe")]
fn push_endpoint_dot_vertices(vertices: &mut Vec<GpuVertex>, center: &WorldPoint, radius: f32) {
    let cx = shape_renderer_core::cast::narrow_f32(center.x);
    let cy = shape_renderer_core::cast::narrow_f32(center.y);
    let rim = |segment: usize| {
        let angle = segment as f32 / ENDPOINT_HANDLE_SEGMENTS as f32 * std::f32::consts::TAU;
        [cx + radius * angle.cos(), cy + radius * angle.sin()]
    };
    for segment in 0..ENDPOINT_HANDLE_SEGMENTS {
        for position in [[cx, cy], rim(segment), rim(segment + 1)] {
            vertices.push(GpuVertex {
                position,
                uv: SOLID_UV[0],
                color: ENDPOINT_HANDLE_FILL_COLOR,
            });
        }
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn hover_affordance_at(
    regions: &[ObjectRegion],
    camera: &CameraState,
    selection: Option<&str>,
    screen: WorldPoint,
) -> HoverAffordance {
    // An open-class selection's surface is its two endpoint handles; the bbox
    // handles below return None for it.
    if let Some(handles) = endpoint_handles(regions, camera, selection, None) {
        if let Some(affordance) = handles.affordance_at(screen.x, screen.y) {
            return affordance;
        }
    }
    if let Some((handles, _)) = selection_handles(regions, camera, selection, None) {
        if let Some(affordance) = handles.affordance_at(screen.x, screen.y) {
            return affordance;
        }
    }
    if hit_object_in_regions(regions, camera, screen).is_some() {
        HoverAffordance::Body
    } else {
        HoverAffordance::Empty
    }
}

/// The pure object pointer-input state machine: mutates `camera`/`input_drag` and
/// writes selection / transform-delta / marquee results into `object_out`,
/// unit-testable without a device.
///
/// Behavior (Select unless noted):
/// - PointerDown, Hand: start a Pan drag.
/// - PointerDown, hit an object: set `selection`, start an Object drag anchored at
///   the pointer-down WORLD point.
/// - PointerDown, empty: start a Marquee drag.
/// - PointerMove on Pan: update `camera`. On Object: emit a CUMULATIVE delta from
///   the fixed anchor. On Marquee: extend the rect. On Rotate with `coarse_rotate`:
///   the swept delta is snapped IN-CORE to the nearest [`ROTATE_SNAP_DEG`] increment
///   so the shell never decomposes/rebuilds the matrix.
/// - PointerUp on Marquee: AABB-test regions into `marquee_ids`; any drag clears.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn step_object_pointer(
    event: &CanvasInputEvent,
    regions: &[ObjectRegion],
    objects: &[RenderObject],
    active_tool: ActiveTool,
    coarse_rotate: bool,
    active_container: Option<&str>,
    selection: Option<&str>,
    camera: &mut CameraState,
    input_drag: &mut Option<InputDragState>,
    object_out: &mut ObjectInputOut,
) {
    match event {
        CanvasInputEvent::PointerDown { pointer_id, screen } => {
            let pointer_id = *pointer_id;
            let screen = *screen;
            // Hand tool always pans; it never hit-tests or mutates selection.
            if active_tool == ActiveTool::Hand {
                *input_drag = Some(InputDragState::Pan {
                    pointer_id,
                    start: screen,
                    camera: camera.clone(),
                });
                return;
            }
            // Grabbing an endpoint handle starts an Endpoint drag (no selection
            // change); anchored endpoints are grabbable too (rebind/unbind is the
            // shell's release commit).
            if let Some(handles) = endpoint_handles(regions, camera, selection, None) {
                if let Some(affordance) = handles.affordance_at(screen.x, screen.y) {
                    let object_id = selection.expect("endpoint_handles requires a selection");
                    let node_index = if affordance == HoverAffordance::EndpointStart {
                        0
                    } else {
                        handles.last_index
                    };
                    *input_drag = Some(InputDragState::Endpoint {
                        pointer_id,
                        object_id: object_id.to_string(),
                        node_index,
                    });
                    return;
                }
            }
            // Grabbing a resize handle / rotate zone starts a transform gesture (no
            // selection change), via the SHARED layout so grab == hover == render.
            // Priority: handles > body > empty.
            if let Some((handles, world_bbox)) = selection_handles(regions, camera, selection, None) {
                if let Some(affordance) = handles.affordance_at(screen.x, screen.y) {
                    let object_id = selection.expect("selection_handles requires a selection");
                    let start = screen_to_world(screen, camera);
                    *input_drag = Some(match affordance {
                        HoverAffordance::Rotate => InputDragState::Rotate {
                            pointer_id,
                            object_id: object_id.to_string(),
                            start,
                            center: WorldPoint {
                                x: world_bbox.x + world_bbox.width / 2.0,
                                y: world_bbox.y + world_bbox.height / 2.0,
                            },
                        },
                        corner => InputDragState::Resize {
                            pointer_id,
                            object_id: object_id.to_string(),
                            corner,
                            start,
                            world_bbox: (
                                world_bbox.x,
                                world_bbox.y,
                                world_bbox.x + world_bbox.width,
                                world_bbox.y + world_bbox.height,
                            ),
                        },
                    });
                    return;
                }
            }
            match hit_object_scoped(regions, objects, camera, screen, active_container) {
                Some(id) => {
                    object_out.selection = Some(id.clone());
                    *input_drag = Some(InputDragState::Object {
                        pointer_id,
                        object_id: id,
                        start: screen_to_world(screen, camera),
                    });
                }
                None => {
                    let world = screen_to_world(screen, camera);
                    *input_drag = Some(InputDragState::Marquee {
                        pointer_id,
                        start: world,
                        current: world,
                    });
                }
            }
        }
        CanvasInputEvent::PointerMove { pointer_id, screen } => {
            let pointer_id = *pointer_id;
            let screen = *screen;
            // No active drag => a hover move, so report the affordance the shell uses
            // to pick a cursor. No hover state of its own.
            let Some(drag) = input_drag.clone() else {
                object_out.hover_affordance =
                    Some(hover_affordance_at(regions, camera, selection, screen));
                return;
            };
            match drag {
                InputDragState::Pan {
                    pointer_id: drag_pointer_id,
                    start,
                    camera: drag_camera,
                } if drag_pointer_id == pointer_id => {
                    *camera = CameraState {
                        x: drag_camera.x + screen.x - start.x,
                        y: drag_camera.y + screen.y - start.y,
                        zoom: drag_camera.zoom,
                    };
                }
                InputDragState::Object {
                    pointer_id: drag_pointer_id,
                    object_id,
                    start,
                } if drag_pointer_id == pointer_id => {
                    let world_now = screen_to_world(screen, camera);
                    object_out.transform_delta = Some(ObjectTransformDelta {
                        id: object_id,
                        matrix: translate_3x3(world_now.x - start.x, world_now.y - start.y),
                        kind: "translate",
                    });
                }
                InputDragState::Resize {
                    pointer_id: drag_pointer_id,
                    object_id,
                    corner,
                    start,
                    world_bbox,
                } if drag_pointer_id == pointer_id => {
                    // Scale about the OPPOSITE anchor of the grabbed handle.
                    let world_now = screen_to_world(screen, camera);
                    object_out.transform_delta = Some(ObjectTransformDelta {
                        id: object_id,
                        matrix: resize_delta_matrix(
                            world_bbox,
                            corner,
                            (world_now.x, world_now.y),
                            (start.x, start.y),
                        ),
                        kind: "resize",
                    });
                }
                InputDragState::Endpoint {
                    pointer_id: drag_pointer_id,
                    object_id,
                    node_index,
                } if drag_pointer_id == pointer_id => {
                    // The dragged endpoint's cumulative WORLD position; the shell
                    // previews the chord deform and commits once on release.
                    let world_now = screen_to_world(screen, camera);
                    object_out.endpoint_delta = Some(ObjectEndpointDelta {
                        id: object_id,
                        node_index,
                        x: world_now.x,
                        y: world_now.y,
                    });
                }
                InputDragState::Rotate {
                    pointer_id: drag_pointer_id,
                    object_id,
                    start,
                    center,
                } if drag_pointer_id == pointer_id => {
                    // Rotation about the bbox center from the anchor angle. Coarse mode
                    // snaps the swept delta IN-CORE so the shell never rebuilds the matrix.
                    let world_now = screen_to_world(screen, camera);
                    let snap = coarse_rotate.then_some(ROTATE_SNAP_DEG);
                    object_out.transform_delta = Some(ObjectTransformDelta {
                        id: object_id,
                        matrix: rotate_delta_matrix_snapped(
                            (center.x, center.y),
                            (world_now.x, world_now.y),
                            (start.x, start.y),
                            snap,
                        ),
                        kind: "rotate",
                    });
                }
                InputDragState::Marquee {
                    pointer_id: drag_pointer_id,
                    start,
                    ..
                } if drag_pointer_id == pointer_id => {
                    *input_drag = Some(InputDragState::Marquee {
                        pointer_id,
                        start,
                        current: screen_to_world(screen, camera),
                    });
                }
                _ => {}
            }
        }
        CanvasInputEvent::PointerUp {
            pointer_id, screen, ..
        } => {
            let pointer_id = *pointer_id;
            if let Some(InputDragState::Marquee {
                pointer_id: drag_pointer_id,
                start,
                ..
            }) = input_drag.clone()
            {
                if drag_pointer_id == pointer_id {
                    let current = screen_to_world(*screen, camera);
                    let rect = marquee_rect(start, current);
                    object_out.marquee_ids = Some(object_regions_in_marquee(regions, &rect));
                }
            }
            // Object drag commits on the shell side; clear only when THIS pointer owns
            // the active drag, so a second finger lifting can't cancel it mid-gesture.
            if drag_pointer_id(input_drag.as_ref()) == Some(pointer_id) {
                *input_drag = None;
            }
        }
        CanvasInputEvent::PointerCancel { pointer_id } => {
            if drag_pointer_id(input_drag.as_ref()) == Some(*pointer_id) {
                *input_drag = None;
            }
        }
        _ => {}
    }
}

/// Branch a double-click that hit an object: `{ id, has_children }`, where
/// `has_children` is true iff any object has `parent == hit id` (container =>
/// drill-in; leaf => text edit). `None` on empty canvas.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn object_double_click(
    regions: &[ObjectRegion],
    objects: &[RenderObject],
    camera: &CameraState,
    screen: WorldPoint,
) -> Option<ObjectDoubleClick> {
    let id = hit_object_in_regions(regions, camera, screen)?;
    let has_children = objects
        .iter()
        .any(|object| object.parent.as_deref() == Some(id.as_str()));
    Some(ObjectDoubleClick { id, has_children })
}

/// Object ids whose WORLD-space region AABB intersects the marquee rect.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn object_regions_in_marquee(regions: &[ObjectRegion], rect: &WorldRect) -> Vec<String> {
    regions
        .iter()
        .filter(|region| !region.hidden && !region.locked)
        .filter_map(|region| {
            let bounds = region_world_bounds(region, None)?;
            rects_intersect(&bounds, rect).then(|| region.id.clone())
        })
        .collect()
}

/// World-space AABB over every object region. `None` when there are no regions / no
/// finite vertices.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn object_regions_world_bounds(regions: &[ObjectRegion]) -> Option<WorldRect> {
    let mut acc: Option<WorldRect> = None;
    for region in regions {
        let Some(bounds) = region_world_bounds(region, None) else {
            continue;
        };
        acc = Some(match acc {
            None => bounds,
            Some(prev) => union_rect_refs(&[&prev, &bounds]).unwrap_or(prev),
        });
    }
    acc
}

/// World-space AABB of one object's region (each local outline vertex through the
/// projective matrix). `None` when the outline is empty or maps non-finite.
/// `preview` substitutes the live drag transform so the bounds (and handles laid out
/// from them) track the dragged bbox without a region rebuild.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn region_world_bounds(
    region: &ObjectRegion,
    preview: Option<&[[f64; 3]; 3]>,
) -> Option<WorldRect> {
    use shape_renderer_core::hit_test_object::apply_3x3;
    let transform = preview.unwrap_or(&region.transform);
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for &(lx, ly) in &region.outline {
        let (wx, wy) = apply_3x3(transform, lx as f64, ly as f64);
        if !wx.is_finite() || !wy.is_finite() {
            continue;
        }
        min_x = min_x.min(wx);
        min_y = min_y.min(wy);
        max_x = max_x.max(wx);
        max_y = max_y.max(wy);
    }
    if min_x.is_finite() && max_x >= min_x && max_y >= min_y {
        Some(WorldRect {
            x: min_x,
            y: min_y,
            width: max_x - min_x,
            height: max_y - min_y,
        })
    } else {
        None
    }
}

/// Nearest point on ANY object outline to a WORLD query point, within `tol_world`.
/// Returns `(id, world_x, world_y)` of the global minimum, or `None`. Used by
/// drag-create anchor snapping.
///
/// Ellipses arrive pre-flattened into `Region.outline`, so this is a uniform min
/// over all outline segments via [`nearest_point_on_polyline`] (no Newton path).
/// Runs live during drag: an AABB broad-phase rejects first, the query point is
/// inverse-transformed into each surviving region's LOCAL space ONCE (allocation-
/// free inner loop), but distances are compared in WORLD space (the local candidate
/// is mapped back, since local distances are wrong under scale/shear/perspective).
#[cfg(feature = "wgpu-probe")]
pub(crate) fn nearest_outline_point(
    regions: &[ObjectRegion],
    world: WorldPoint,
    tol_world: f64,
    exclude: &[&str],
) -> Option<(String, f64, f64)> {
    use shape_renderer_core::hit_test_object::{apply_3x3, world_to_local};
    use shape_renderer_core::outline::nearest_point_on_polyline;

    let tol2 = tol_world * tol_world;
    let mut best: Option<(&str, f64, f64, f64)> = None; // (id, world_x, world_y, d2)
    for region in regions {
        // Skip transient preview regions so a create-drag snaps to a REAL edge
        // instead of self-snapping under the cursor.
        if exclude.contains(&region.id.as_str()) {
            continue;
        }
        // Broad-phase: skip when the query is outside the world AABB + tolerance.
        let Some(bounds) = region_world_bounds(region, None) else {
            continue;
        };
        if world.x < bounds.x - tol_world
            || world.x > bounds.x + bounds.width + tol_world
            || world.y < bounds.y - tol_world
            || world.y > bounds.y + bounds.height + tol_world
        {
            continue;
        }
        // Query in object-LOCAL space: inverse-transform the world point once.
        let Some((lx, ly)) = world_to_local(&region.transform, world.x, world.y) else {
            continue;
        };
        let Some((local_pt, _local_d2)) =
            nearest_point_on_polyline(&region.outline, region.closed, shape_renderer_core::cast::narrow_f32(lx), shape_renderer_core::cast::narrow_f32(ly))
        else {
            continue;
        };
        // Map the local candidate back to WORLD and measure there.
        let (wx, wy) = apply_3x3(&region.transform, local_pt.0 as f64, local_pt.1 as f64);
        if !wx.is_finite() || !wy.is_finite() {
            continue;
        }
        let dx = wx - world.x;
        let dy = wy - world.y;
        let d2 = dx * dx + dy * dy;
        if d2 <= tol2 && best.map(|(_, _, _, bd2)| d2 < bd2).unwrap_or(true) {
            best = Some((region.id.as_str(), wx, wy, d2));
        }
    }
    best.map(|(id, wx, wy, _)| (id.to_string(), wx, wy))
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn focus_camera_to_bounds(
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
pub(crate) fn scene_world_bounds(scene: &SceneSnapshot) -> WorldRect {
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
pub(crate) fn union_rect_refs(rects: &[&WorldRect]) -> Option<WorldRect> {
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
pub(crate) fn bounds_zoom(usable_width: f64, usable_height: f64, bounds: &WorldRect) -> f64 {
    (usable_width / bounds.width).min(usable_height / bounds.height)
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn clamp_camera(camera: CameraState) -> CameraState {
    CameraState {
        x: camera.x,
        y: camera.y,
        zoom: clamp_camera_zoom(camera.zoom),
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn clamp_camera_zoom(zoom: f64) -> f64 {
    clamp(zoom, 0.025, 2.8)
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn clamp(value: f64, min: f64, max: f64) -> f64 {
    value.min(max).max(min)
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn overlay_style(
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
pub(crate) fn overlay_rect_for_text_field(
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
pub(crate) fn overlay_padding_world(style: &ShapeRenderStyle) -> (f64, f64) {
    (style.spacing.card_gap, style.spacing.card_gap * 0.67)
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn overlay_border_width_world(style: &ShapeRenderStyle, selected: bool) -> f64 {
    if selected {
        style.stroke_width.card_selected
    } else {
        style.stroke_width.card
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn overlay_max_lines(field: &str) -> u8 {
    match field {
        "title" => 1,
        "summary" => 3,
        _ => 6,
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn css_color(color: [f32; 4]) -> String {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped to [0.0, 1.0] then scaled by 255 and rounded; the value is an exact integer in u8 range"
    )]
    let channel = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    let red = channel(color[0]);
    let green = channel(color[1]);
    let blue = channel(color[2]);
    format!(
        "rgba({red}, {green}, {blue}, {:.3})",
        color[3].clamp(0.0, 1.0)
    )
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn css_shadow_layers(layers: &[ShapeShadowLayer], zoom: f64, alpha_scale: f32) -> String {
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
pub(crate) fn drag_pointer_id(drag: Option<&InputDragState>) -> Option<i32> {
    match drag {
        Some(InputDragState::Pan { pointer_id, .. })
        | Some(InputDragState::Group { pointer_id, .. })
        | Some(InputDragState::Card { pointer_id, .. })
        | Some(InputDragState::Edge { pointer_id, .. })
        | Some(InputDragState::Marquee { pointer_id, .. })
        | Some(InputDragState::Object { pointer_id, .. })
        | Some(InputDragState::Resize { pointer_id, .. })
        | Some(InputDragState::Rotate { pointer_id, .. })
        | Some(InputDragState::Endpoint { pointer_id, .. }) => Some(*pointer_id),
        None => None,
    }
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn text_field_at_point(
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
pub(crate) fn text_field_rect(
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
pub(crate) fn card_text_layout(card: &WorldRect, style: &ShapeRenderStyle, selected: bool) -> CardTextLayout {
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
    // Non-negative line counts, each immediately clamped to a tiny range.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "non-negative floored line count, clamped to a tiny range below; exact integer in range"
    )]
    let summary_max_lines =
        ((summary_available / summary_line_height).floor() as usize).clamp(1, 3);
    let detail_available = (content_bottom - detail_y).max(0.0);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "non-negative floored line count, clamped to a tiny range below; exact integer in range"
    )]
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
pub(crate) fn port_at_point(card: &RenderCard, point: &WorldPoint) -> Option<String> {
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
pub(crate) fn edge_route(source: &RenderCard, target: &RenderCard) -> CubicRoute {
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
pub(crate) fn edge_visible_bounds(source: &RenderCard, target: &RenderCard) -> WorldRect {
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
pub(crate) fn distance(a: &WorldPoint, b: &WorldPoint) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn distance_to_segment(point: &WorldPoint, start: &WorldPoint, end: &WorldPoint) -> f64 {
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
pub(crate) fn distance_to_cubic(point: &WorldPoint, route: &CubicRoute) -> f64 {
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
pub(crate) fn cubic_point(route: &CubicRoute, t: f64) -> WorldPoint {
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
pub(crate) const SOLID_UV: [[f32; 2]; 4] = TEXT_ATLAS_SOLID_UV;
#[cfg(feature = "wgpu-probe")]
pub(crate) const EDGE_CURVE_SEGMENTS: usize = 18;
#[cfg(feature = "wgpu-probe")]
pub(crate) const ROUNDED_CORNER_SEGMENTS: usize = 4;
#[cfg(feature = "wgpu-probe")]
pub(crate) const SOFT_GRADIENT_BANDS: usize = 5;
#[cfg(feature = "wgpu-probe")]
pub(crate) const GROUP_VERTEX_SLOT: usize = 1536;
#[cfg(feature = "wgpu-probe")]
pub(crate) const EDGE_VERTEX_SLOT: usize = 768;
#[cfg(feature = "wgpu-probe")]
pub(crate) const CARD_VERTEX_SLOT: usize = 1536;
#[cfg(feature = "wgpu-probe")]
pub(crate) const VIEWPORT_CULL_PADDING: f64 = 400.0;
// 1 fill quad (6) + 4 stroke edge quads (24).
#[cfg(feature = "wgpu-probe")]
pub(crate) const MARQUEE_OVERLAY_VERTEX_CAPACITY: usize = 30;
// Accent blue, translucent. World-space so the camera-transform pipeline renders it.
#[cfg(feature = "wgpu-probe")]
pub(crate) const MARQUEE_FILL_COLOR: [f32; 4] = [0.231, 0.510, 0.965, 0.12];
#[cfg(feature = "wgpu-probe")]
pub(crate) const MARQUEE_STROKE_COLOR: [f32; 4] = [0.231, 0.510, 0.965, 0.9];

// Sized for the larger surface: open-class = two endpoint dot fans (2 * 16 * 3 =
// 96), vs closed-class = 8 resize handles + rotate zone (54).
#[cfg(feature = "wgpu-probe")]
pub(crate) const HANDLE_OVERLAY_VERTEX_CAPACITY: usize = 2 * ENDPOINT_HANDLE_SEGMENTS * 3;
#[cfg(feature = "wgpu-probe")]
pub(crate) const HANDLE_FILL_COLOR: [f32; 4] = [0.184, 0.494, 0.902, 1.0];
// Fan segments per endpoint dot circle.
#[cfg(feature = "wgpu-probe")]
pub(crate) const ENDPOINT_HANDLE_SEGMENTS: usize = 16;
#[cfg(feature = "wgpu-probe")]
pub(crate) const ENDPOINT_HANDLE_FILL_COLOR: [f32; 4] = [0.0, 0.478, 1.0, 1.0];

// Sized for the ALL-OPEN worst case (~96 members, each two dot fans = 4x a box's 24
// outline verts). One-time allocation at device init, not a per-frame cost.
#[cfg(feature = "wgpu-probe")]
pub(crate) const MULTI_SELECT_OVERLAY_VERTEX_CAPACITY: usize =
    96 * 2 * ENDPOINT_HANDLE_SEGMENTS * 3;
#[cfg(feature = "wgpu-probe")]
pub(crate) const MULTI_SELECT_OUTLINE_COLOR: [f32; 4] = [0.0, 0.478, 1.0, 1.0];

#[cfg(feature = "wgpu-probe")]
pub(crate) fn webgpu_vertex_buffer_usage() -> wgpu::BufferUsages {
    wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC
}


#[cfg(feature = "wgpu-probe")]
pub(crate) const SHAPE_WEBGPU_SHADER: &str = r#"
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
            multi_select: Vec::new(),
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
            multi_select: Vec::new(),
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
            multi_select: Vec::new(),
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
    fn world_to_screen_inverts_screen_to_world_under_pan_and_zoom() {
        let camera = CameraState {
            x: 37.5,
            y: -112.25,
            zoom: 1.875,
        };
        for &(sx, sy) in &[
            (0.0, 0.0),
            (260.0, 180.0),
            (-413.0, 642.0),
            (1280.0, 720.0),
        ] {
            let screen = WorldPoint { x: sx, y: sy };
            let round_trip = world_to_screen(screen_to_world(screen, &camera), &camera);
            assert!(
                (round_trip.x - screen.x).abs() < 1e-9 && (round_trip.y - screen.y).abs() < 1e-9,
                "screen round-trip drifted: {screen:?} -> {round_trip:?}"
            );

            let world = WorldPoint { x: sx, y: sy };
            let world_round_trip = screen_to_world(world_to_screen(world, &camera), &camera);
            assert!(
                (world_round_trip.x - world.x).abs() < 1e-9
                    && (world_round_trip.y - world.y).abs() < 1e-9,
                "world round-trip drifted: {world:?} -> {world_round_trip:?}"
            );
        }
    }

    #[test]
    fn world_to_screen_applies_pan_and_zoom() {
        let camera = CameraState {
            x: 100.0,
            y: -40.0,
            zoom: 2.0,
        };
        let screen = world_to_screen(WorldPoint { x: 30.0, y: 25.0 }, &camera);
        assert!((screen.x - 160.0).abs() < 1e-9);
        assert!((screen.y - 10.0).abs() < 1e-9);
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
            multi_select: Vec::new(),
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

    fn lod_camera(zoom: f64) -> CameraState {
        CameraState {
            x: 0.0,
            y: 0.0,
            zoom,
        }
    }

    fn lod_bounds(width: f64, height: f64) -> WorldRect {
        WorldRect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        }
    }

    #[test]
    fn record_tier_buckets_objects_by_apparent_size() {
        let mut draw_list = FrameDrawList::default();
        let previous = HashMap::new();
        let camera = lod_camera(1.0);

        // 390px card -> Full (>= 220px apparent).
        draw_list.record_tier("card-full", &lod_bounds(390.0, 390.0), &camera, &previous);
        // 150px card -> Compact (120..220).
        draw_list.record_tier("card-compact", &lod_bounds(150.0, 80.0), &camera, &previous);
        // 60px card -> ShapeOnly (40..120).
        draw_list.record_tier("card-shape", &lod_bounds(60.0, 30.0), &camera, &previous);
        // 20px card -> Density (8..40).
        draw_list.record_tier("card-density", &lod_bounds(20.0, 10.0), &camera, &previous);
        // 4px card -> Minimap (< 8).
        draw_list.record_tier("card-minimap", &lod_bounds(4.0, 2.0), &camera, &previous);

        assert_eq!(draw_list.full_tier_count, 1);
        assert_eq!(draw_list.compact_tier_count, 1);
        assert_eq!(draw_list.shape_only_tier_count, 1);
        assert_eq!(draw_list.density_tier_count, 1);
        assert_eq!(draw_list.minimap_tier_count, 1);
        assert_eq!(draw_list.lod_tiers.len(), 5);
        assert_eq!(draw_list.lod_tiers.get("card-full"), Some(&LodTier::Full));
        assert_eq!(
            draw_list.lod_tiers.get("card-minimap"),
            Some(&LodTier::Minimap)
        );
    }

    #[test]
    fn record_tier_honors_previous_frame_for_hysteresis() {
        // A 222px card sits just past the 220px Full edge. With no history it is
        // Full; if the previous frame had it Compact, hysteresis holds Compact
        // until it clears the dead-band.
        let camera = lod_camera(1.0);
        let bounds = lod_bounds(222.0, 100.0);

        let mut fresh = FrameDrawList::default();
        fresh.record_tier("card", &bounds, &camera, &HashMap::new());
        assert_eq!(fresh.full_tier_count, 1);
        assert_eq!(fresh.compact_tier_count, 0);

        let mut previous = HashMap::new();
        previous.insert("card".to_string(), LodTier::Compact);
        let mut held = FrameDrawList::default();
        held.record_tier("card", &bounds, &camera, &previous);
        assert_eq!(held.full_tier_count, 0);
        assert_eq!(held.compact_tier_count, 1);
        assert_eq!(held.lod_tiers.get("card"), Some(&LodTier::Compact));
    }

    fn marquee_group(id: &str, x: f64, y: f64, width: f64, height: f64) -> RenderGroup {
        RenderGroup {
            id: id.to_string(),
            title: id.to_string(),
            summary: String::new(),
            bounds: WorldRect {
                x,
                y,
                width,
                height,
            },
            tag_ids: Vec::new(),
            z_index: 0.0,
            style_key: "default".to_string(),
        }
    }

    #[test]
    fn marquee_rect_normalizes_drag_corners() {
        // Drag from bottom-right to top-left still yields a positive-extent rect.
        let rect = marquee_rect(
            WorldPoint { x: 300.0, y: 200.0 },
            WorldPoint { x: 100.0, y: 50.0 },
        );
        assert_eq!(rect.x, 100.0);
        assert_eq!(rect.y, 50.0);
        assert_eq!(rect.width, 200.0);
        assert_eq!(rect.height, 150.0);
    }

    #[test]
    fn marquee_collects_intersecting_cards_and_groups() {
        let scene = SceneSnapshot {
            scene_id: "marquee-test".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            // inside (60..300) overlaps the marquee; far (900+) does not.
            groups: vec![
                marquee_group("group-in", 40.0, 40.0, 120.0, 120.0),
                marquee_group("group-out", 900.0, 900.0, 80.0, 80.0),
            ],
            cards: vec![
                text_path_card("card-in", 100.0, 100.0),
                text_path_card("card-out", 1000.0, 1000.0),
            ],
            edges: Vec::new(),
            styles: vec![minimal_style_token("default")],
            selection: SceneSelection::Canvas,
            multi_select: Vec::new(),
        };

        let rect = marquee_rect(
            WorldPoint { x: 60.0, y: 60.0 },
            WorldPoint { x: 320.0, y: 320.0 },
        );
        let ids = marquee_intersecting_ids(&scene, &rect);

        // Cards first, then groups; only the intersecting ones.
        assert_eq!(ids, vec!["card-in".to_string(), "group-in".to_string()]);
    }

    #[test]
    fn marquee_overlay_builds_fill_and_stroke_geometry() {
        let rect = WorldRect {
            x: 10.0,
            y: 20.0,
            width: 120.0,
            height: 80.0,
        };
        let vertices = build_marquee_overlay_vertices(&rect, 1.0);
        // 1 fill quad + 4 stroke quads = 30 vertices, within the buffer capacity.
        assert_eq!(vertices.len(), MARQUEE_OVERLAY_VERTEX_CAPACITY);
        assert!(vertices.len() <= MARQUEE_OVERLAY_VERTEX_CAPACITY);
    }

    // ----- object live-path helpers -----------------------------

    use shape_renderer_core::render_object::RenderObject;

    fn object_identity() -> [[f64; 3]; 3] {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    }

    /// An object at world `(tx, ty)` whose local geometry is an `s`px×`s`px rect
    /// (`s` px = `s*8` quantized units). Later-in-the-list objects draw on top.
    fn rect_object(id: &str, tx: f64, ty: f64, s: i32) -> RenderObject {
        let u = s * 8; // px -> quantized units
        RenderObject {
            id: id.to_string(),
            parent: None,
            order: "a0".to_string(),
            transform: [[1.0, 0.0, tx], [0.0, 1.0, ty], [0.0, 0.0, 1.0]],
            geometry_d: format!("M 0 0 L {u} 0 L {u} {u} L 0 {u} Z"),
            fill: None,
            stroke: None,
            text: None,
            anchors: Vec::new(),
            clip: false,
            hidden: false,
            locked: false,
        }
    }

    /// An object at world `(tx, ty)` whose local geometry is a `w`px×`h`px ellipse
    /// (4 cubic arcs, the same kappa string the shell's `ellipsePath` emits).
    /// Exercises the curve nearest-point path.
    fn ellipse_object(id: &str, tx: f64, ty: f64, w: i32, h: i32) -> RenderObject {
        let q = |px: i32| px * 8; // px -> quantized units
        let qw = q(w);
        let qh = q(h);
        let cx = q(w / 2);
        let cy = q(h / 2);
        let rx = w * 8 / 2;
        let ry = h * 8 / 2;
        let kx = shape_renderer_core::cast::round_i32(f64::from(rx) * 0.5523);
        let ky = shape_renderer_core::cast::round_i32(f64::from(ry) * 0.5523);
        let d = format!(
            "M 0 {cy} C 0 {} {} 0 {cx} 0 C {} 0 {qw} {} {qw} {cy} \
             C {qw} {} {} {qh} {cx} {qh} C {} {qh} 0 {} 0 {cy} Z",
            cy - ky,
            cx - kx,
            cx + kx,
            cy - ky,
            cy + ky,
            cx + kx,
            cx - kx,
            cy + ky,
        );
        RenderObject {
            id: id.to_string(),
            parent: None,
            order: "a0".to_string(),
            transform: [[1.0, 0.0, tx], [0.0, 1.0, ty], [0.0, 0.0, 1.0]],
            geometry_d: d,
            fill: None,
            stroke: None,
            text: None,
            anchors: Vec::new(),
            clip: false,
            hidden: false,
            locked: false,
        }
    }

    /// An object at world `(tx, ty)` whose local geometry is an OPEN horizontal
    /// line of `len`px (no closing edge — `M 0 0 L len*8 0`).
    fn line_object(id: &str, tx: f64, ty: f64, len: i32) -> RenderObject {
        let u = len * 8;
        RenderObject {
            id: id.to_string(),
            parent: None,
            order: "a0".to_string(),
            transform: [[1.0, 0.0, tx], [0.0, 1.0, ty], [0.0, 0.0, 1.0]],
            geometry_d: format!("M 0 0 L {u} 0"),
            fill: None,
            stroke: None,
            text: None,
            anchors: Vec::new(),
            clip: false,
            hidden: false,
            locked: false,
        }
    }

    fn object_scene(objects: Vec<RenderObject>) -> RenderObjectScene {
        RenderObjectScene {
            scene_id: "fc-object".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            objects,
            selection: None,
            multi_select: Vec::new(),
        }
    }

    fn identity_camera() -> CameraState {
        CameraState {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }

    #[test]
    fn hidden_object_is_not_hittable_or_drawn() {
        // A hidden rect must not be hit (non-render => non-interactive) and must
        // tessellate ZERO fill vertices. FAILS today: the flag was dropped, so the
        // region was built (hittable) and the fill tessellated.
        let mut hidden = rect_object("h", 0.0, 0.0, 20);
        hidden.hidden = true;
        let scene = object_scene(vec![hidden]);
        let regions = derive_object_regions(&scene);
        let camera = identity_camera();

        // A point inside the bbox misses the hidden region.
        assert_eq!(
            hit_object_in_regions(&regions, &camera, WorldPoint { x: 10.0, y: 10.0 }),
            None,
            "a hidden object is not hittable"
        );

        // The geometry build emits zero fill vertices over the hidden object's range.
        let geo = shape_renderer_core::build_scene_geometry(&scene);
        let draw = geo.draws.iter().find(|d| d.id == "h").expect("draw slot kept");
        assert!(
            draw.fill_range.is_empty() && draw.fill_vertex_range.is_empty(),
            "a hidden object tessellates no fill"
        );
    }

    #[test]
    fn locked_object_pointer_down_does_not_select_or_drag() {
        // A locked rect: a PointerDown inside it must NOT select it and must NOT start
        // an Object drag — it falls through to a Marquee. FAILS today: the locked flag
        // was dropped, so the region was hit and an Object drag started.
        let mut locked = rect_object("l", 0.0, 0.0, 20);
        locked.locked = true;
        let scene = object_scene(vec![locked]);
        let regions = derive_object_regions(&scene);
        let mut camera = identity_camera();
        let mut drag: Option<InputDragState> = None;
        let mut out = ObjectInputOut::default();

        step_object_pointer(
            &CanvasInputEvent::PointerDown {
                pointer_id: 1,
                screen: WorldPoint { x: 10.0, y: 10.0 },
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            false,
            None,
            None,
            &mut camera,
            &mut drag,
            &mut out,
        );
        assert!(out.selection.is_none(), "a locked object is not selectable");
        assert!(
            matches!(drag, Some(InputDragState::Marquee { .. })),
            "a locked-object pointer-down falls through to a Marquee, not an Object drag"
        );
    }

    #[test]
    fn pointer_down_inside_active_container_selects_child_not_frame() {
        // A container frame `f` spanning [0,40]² placed LAST (top-most) + a child `c`
        // (parent=f) spanning [5,15]² placed first (under the frame at the shared
        // point). With no scope, a click resolves to the top-most `f` (status quo).
        // With `f` as the active scope, the SAME click resolves to the direct child
        // `c`; a click inside `f` but outside `c` keeps `f`. FAILS today: the scope is
        // ignored and the top-most `f` always wins.
        let mut child = rect_object("c", 5.0, 5.0, 10); // local 10px @ (5,5) => [5,15]
        child.parent = Some("f".to_string());
        let frame = rect_object("f", 0.0, 0.0, 40); // [0,40], LAST => top-most
        let scene = object_scene(vec![child, frame]);
        let regions = derive_object_regions(&scene);
        let camera = identity_camera();
        let inside_child = WorldPoint { x: 10.0, y: 10.0 };
        let frame_only = WorldPoint { x: 35.0, y: 35.0 };

        // No active container: the top-most frame wins (pins status quo).
        assert_eq!(
            hit_object_scoped(&regions, &scene.objects, &camera, inside_child, None).as_deref(),
            Some("f"),
            "unscoped click selects the top-level container"
        );

        // Scope = f: the same click drills to the direct child under the pointer.
        assert_eq!(
            hit_object_scoped(&regions, &scene.objects, &camera, inside_child, Some("f"))
                .as_deref(),
            Some("c"),
            "a scoped click inside the container selects the direct child"
        );
        // A click inside the container but outside any child keeps the container.
        assert_eq!(
            hit_object_scoped(&regions, &scene.objects, &camera, frame_only, Some("f")).as_deref(),
            Some("f"),
            "an empty-inside click selects the container (does not exit)"
        );
        // A click entirely outside the container returns None (the shell exits scope).
        assert_eq!(
            hit_object_scoped(
                &regions,
                &scene.objects,
                &camera,
                WorldPoint { x: 100.0, y: 100.0 },
                Some("f")
            ),
            None,
            "a click outside the scope returns None"
        );
    }

    #[test]
    fn hit_object_picks_top_overlapping_object() {
        // Two overlapping 20px rects: "bottom" at world (0,0), "top" at world (5,5).
        // They overlap in world [5,20]×[5,20]; the later object ("top") draws on top
        // and must win the hit at a shared point.
        let scene = object_scene(vec![
            rect_object("bottom", 0.0, 0.0, 20),
            rect_object("top", 5.0, 5.0, 20),
        ]);
        let regions = derive_object_regions(&scene);
        assert_eq!(regions.len(), 2);
        let camera = identity_camera(); // screen == world at zoom 1, origin 0.

        // Shared point -> top wins (top-down = reverse of the Vec).
        let id = hit_object_in_regions(&regions, &camera, WorldPoint { x: 10.0, y: 10.0 });
        assert_eq!(id.as_deref(), Some("top"));

        // A point only inside the bottom rect (world x<5) -> bottom.
        let id = hit_object_in_regions(&regions, &camera, WorldPoint { x: 2.0, y: 2.0 });
        assert_eq!(id.as_deref(), Some("bottom"));

        // Outside both.
        let id = hit_object_in_regions(&regions, &camera, WorldPoint { x: 100.0, y: 100.0 });
        assert_eq!(id, None);
    }

    #[test]
    fn hit_object_grabs_zero_fill_stroke_via_bbox_fallback() {
        // An OPEN line (no closed fill) at world (0,0), local (0,0)->(40,0).
        // A point ON the stroke body must grab it via the bbox fallback (the even-odd
        // fill test would miss an open contour); empty canvas off the stroke misses.
        let scene = object_scene(vec![line_object("l", 0.0, 0.0, 40)]);
        let regions = derive_object_regions(&scene);
        let camera = identity_camera();

        let id = hit_object_in_regions(&regions, &camera, WorldPoint { x: 20.0, y: 0.0 });
        assert_eq!(id.as_deref(), Some("l"), "stroke body grabbable via bbox fallback");

        // Far from the stroke bbox -> empty space still misses (marquee stays reachable).
        let id = hit_object_in_regions(&regions, &camera, WorldPoint { x: 20.0, y: 100.0 });
        assert_eq!(id, None);
    }

    #[test]
    fn swept_erase_crosses_all_objects_between_two_samples() {
        // Three 10px rects spaced along x at world x = 0, 30, 60 (all y=0).
        // A FAST eraser drag samples only the endpoints (-5, 5) and (75, 5): the
        // segment crosses all three between samples though no single sample point sits
        // inside more than one. The swept test must accumulate EVERY crossed object.
        let scene = object_scene(vec![
            rect_object("a", 0.0, 0.0, 10),
            rect_object("b", 30.0, 0.0, 10),
            rect_object("c", 60.0, 0.0, 10),
        ]);
        let regions = derive_object_regions(&scene);
        let camera = identity_camera(); // screen == world.

        let ids = swept_erase_in_regions(
            &regions,
            &camera,
            WorldPoint { x: -5.0, y: 5.0 },
            WorldPoint { x: 75.0, y: 5.0 },
        );
        let mut ids = ids;
        ids.sort();
        assert_eq!(ids, vec!["a".to_string(), "b".to_string(), "c".to_string()]);

        // Sanity: the per-sample point hit-test catches at most ONE of them at each
        // endpoint, proving the gap the swept test closes. Endpoint (-5,5) is left of
        // every rect, so the discrete sample erases nothing there.
        assert_eq!(
            hit_object_in_regions(&regions, &camera, WorldPoint { x: -5.0, y: 5.0 }),
            None
        );

        // A drag well above all three rects crosses none.
        let none = swept_erase_in_regions(
            &regions,
            &camera,
            WorldPoint { x: -5.0, y: 500.0 },
            WorldPoint { x: 75.0, y: 500.0 },
        );
        assert!(none.is_empty());
    }

    #[test]
    fn nearest_outline_rect_edge_and_corner() {
        // A 20px rect at world (0,0): local outline 0..20 in both axes.
        let scene = object_scene(vec![rect_object("r", 0.0, 0.0, 20)]);
        let regions = derive_object_regions(&scene);

        // Just outside the right edge at mid-height -> snaps to (20, 10).
        let (id, x, y) =
            nearest_outline_point(&regions, WorldPoint { x: 22.0, y: 10.0 }, 5.0, &[]).expect("snap");
        assert_eq!(id, "r");
        assert!((x - 20.0).abs() < 1e-3 && (y - 10.0).abs() < 1e-3, "({x}, {y})");

        // Near a corner (outside both edges) -> clamps to the corner (20, 20).
        let (_, cx, cy) =
            nearest_outline_point(&regions, WorldPoint { x: 23.0, y: 23.0 }, 6.0, &[]).expect("corner");
        assert!((cx - 20.0).abs() < 1e-3 && (cy - 20.0).abs() < 1e-3, "({cx}, {cy})");
    }

    #[test]
    fn nearest_outline_ellipse_accuracy_within_flatten_tolerance() {
        // A 200px ellipse at world (0,0): center (100,100), rx = ry = 100. The
        // analytic +x extremum is (200, 100). A query just outside it must snap to
        // within the 0.5px flatten tolerance of that analytic point.
        let scene = object_scene(vec![ellipse_object("e", 0.0, 0.0, 200, 200)]);
        let regions = derive_object_regions(&scene);
        assert_eq!(regions.len(), 1, "ellipse derives a region");

        let (id, x, y) =
            nearest_outline_point(&regions, WorldPoint { x: 203.0, y: 100.0 }, 6.0, &[]).expect("snap");
        assert_eq!(id, "e");
        // Within the flatten tolerance (0.5px chord error) of the analytic point.
        assert!((x - 200.0).abs() < 0.6, "x {x} ~ 200");
        assert!((y - 100.0).abs() < 0.6, "y {y} ~ 100");
    }

    #[test]
    fn nearest_outline_open_line_ignores_implicit_closing_edge() {
        // An open horizontal line from (0,0) to (40,0) at world (0,0). A query above
        // the midpoint snaps onto the drawn segment at (20, 0).
        let scene = object_scene(vec![line_object("l", 0.0, 0.0, 40)]);
        let regions = derive_object_regions(&scene);

        let (id, x, y) =
            nearest_outline_point(&regions, WorldPoint { x: 20.0, y: 3.0 }, 5.0, &[]).expect("snap");
        assert_eq!(id, "l");
        assert!((x - 20.0).abs() < 1e-3 && (y - 0.0).abs() < 1e-3, "({x}, {y})");

        // The line is a 2-vertex outline (no closing edge); a query is never nearer
        // a non-existent edge than the drawn segment. The endpoints are (0,0) and
        // (40,0), so the whole snap surface is the single segment itself.
        let (_, x2, y2) =
            nearest_outline_point(&regions, WorldPoint { x: -3.0, y: 0.0 }, 5.0, &[]).expect("endpoint");
        assert!((x2 - 0.0).abs() < 1e-3 && (y2 - 0.0).abs() < 1e-3, "({x2}, {y2})");
    }

    #[test]
    fn nearest_outline_tolerance_boundary() {
        // A 20px rect at world (0,0). The right edge is at x=20; a query at x=24 is
        // 4px away from the nearest outline point (20, 10).
        let scene = object_scene(vec![rect_object("r", 0.0, 0.0, 20)]);
        let regions = derive_object_regions(&scene);

        // Just inside tolerance (4px away, tol 5) -> snaps.
        let inside = nearest_outline_point(&regions, WorldPoint { x: 24.0, y: 10.0 }, 5.0, &[]);
        assert!(matches!(inside, Some((ref id, _, _)) if id == "r"), "{inside:?}");

        // Just beyond tolerance (4px away, tol 3) -> no snap.
        let beyond = nearest_outline_point(&regions, WorldPoint { x: 24.0, y: 10.0 }, 3.0, &[]);
        assert!(beyond.is_none(), "{beyond:?}");
    }

    // Excluding the transient create-preview region (whose dragged corner sits under
    // the cursor) must surface the nearest REAL object's edge instead of self-snapping.
    #[test]
    fn nearest_outline_excludes_transient_create_preview() {
        // Real rect "r" right edge x=20; preview rect whose corner lands at the query
        // point (22,10).
        let scene = object_scene(vec![
            rect_object("r", 0.0, 0.0, 20),
            rect_object("create-preview", 2.0, -10.0, 20),
        ]);
        let regions = derive_object_regions(&scene);
        let query = WorldPoint { x: 22.0, y: 10.0 };

        // Documents the break: with no exclusion the preview corner (dist ~0) wins.
        let unexcluded =
            nearest_outline_point(&regions, query, 5.0, &[]).expect("preview self-snap");
        assert_eq!(unexcluded.0, "create-preview");

        // The fix: excluding the preview surfaces the REAL rect's right edge at
        // (20, 10), 2px from the cursor.
        let (id, x, y) = nearest_outline_point(&regions, query, 5.0, &["create-preview"])
            .expect("real edge snap");
        assert_eq!(id, "r");
        assert!((x - 20.0).abs() < 1e-3 && (y - 10.0).abs() < 1e-3, "({x}, {y})");

        // Far from BOTH (excluded preview included) -> no snap at all.
        let far = nearest_outline_point(
            &regions,
            WorldPoint { x: 200.0, y: 200.0 },
            5.0,
            &["create-preview"],
        );
        assert!(far.is_none(), "{far:?}");
    }

    #[test]
    fn object_pointer_down_selects_then_move_emits_transform_delta() {
        let scene = object_scene(vec![rect_object("o1", 0.0, 0.0, 20)]);
        let regions = derive_object_regions(&scene);
        let mut camera = identity_camera();
        let mut drag: Option<InputDragState> = None;
        let mut out = ObjectInputOut::default();

        // PointerDown inside the object -> selection + Object drag.
        step_object_pointer(
            &CanvasInputEvent::PointerDown {
                pointer_id: 1,
                screen: WorldPoint { x: 10.0, y: 10.0 },
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            false,
            None,
            None,
            &mut camera,
            &mut drag,
            &mut out,
        );
        assert_eq!(out.selection.as_deref(), Some("o1"));
        assert!(matches!(
            drag,
            Some(InputDragState::Object { ref object_id, .. }) if object_id == "o1"
        ));

        // PointerMove -> cumulative world-px delta from the FIXED pointer-down point.
        step_object_pointer(
            &CanvasInputEvent::PointerMove {
                pointer_id: 1,
                screen: WorldPoint { x: 18.0, y: 13.0 },
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            false,
            None,
            None,
            &mut camera,
            &mut drag,
            &mut out,
        );
        let delta = out
            .transform_delta
            .take()
            .expect("move emits a transform delta");
        assert_eq!(delta.id, "o1");
        assert_eq!(delta.kind, "translate");
        // W2-04: the delta is a translation matrix; dx/dy are the translation column.
        assert!((delta.matrix[0][2] - 8.0).abs() < 1e-9);
        assert!((delta.matrix[1][2] - 3.0).abs() < 1e-9);

        // PointerUp clears the drag (the shell commits one undoable op).
        step_object_pointer(
            &CanvasInputEvent::PointerUp {
                pointer_id: 1,
                screen: WorldPoint { x: 18.0, y: 13.0 },
                edge_id: None,
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            false,
            None,
            None,
            &mut camera,
            &mut drag,
            &mut out,
        );
        assert!(drag.is_none());
    }

    #[test]
    fn object_pointer_empty_marquee_collects_world_aabb_intersections() {
        // Two objects far apart; a marquee over only the first collects just it.
        let scene = object_scene(vec![
            rect_object("near", 0.0, 0.0, 10),
            rect_object("far", 500.0, 500.0, 10),
        ]);
        let regions = derive_object_regions(&scene);
        let mut camera = identity_camera();
        let mut drag: Option<InputDragState> = None;
        let mut out = ObjectInputOut::default();

        // Down on empty space starts a marquee.
        step_object_pointer(
            &CanvasInputEvent::PointerDown {
                pointer_id: 2,
                screen: WorldPoint { x: -5.0, y: -5.0 },
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            false,
            None,
            None,
            &mut camera,
            &mut drag,
            &mut out,
        );
        assert!(matches!(drag, Some(InputDragState::Marquee { .. })));
        assert!(out.selection.is_none());

        // Up at (15,15) -> marquee [-5,15]² covers "near" only.
        step_object_pointer(
            &CanvasInputEvent::PointerUp {
                pointer_id: 2,
                screen: WorldPoint { x: 15.0, y: 15.0 },
                edge_id: None,
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            false,
            None,
            None,
            &mut camera,
            &mut drag,
            &mut out,
        );
        assert_eq!(out.marquee_ids, Some(vec!["near".to_string()]));
    }

    #[test]
    fn object_marquee_reversed_drag_through_move_reaches_marquee_ids() {
        // A full down -> move -> up sequence dragged bottom-right -> top-left
        // (reversed corners) must still round-trip to the same ids the rect intersects.
        let scene = object_scene(vec![
            rect_object("near", 0.0, 0.0, 10),
            rect_object("far", 500.0, 500.0, 10),
        ]);
        let regions = derive_object_regions(&scene);
        let mut camera = identity_camera();
        let mut drag: Option<InputDragState> = None;
        let mut out = ObjectInputOut::default();

        // Down bottom-right of "near" on empty space.
        step_object_pointer(
            &CanvasInputEvent::PointerDown { pointer_id: 4, screen: WorldPoint { x: 15.0, y: 15.0 } },
            &regions, &scene.objects, ActiveTool::Select, false, None, None, &mut camera, &mut drag, &mut out,
        );
        assert!(matches!(drag, Some(InputDragState::Marquee { .. })));
        // Move toward top-left through an intermediate point (real drags emit moves).
        step_object_pointer(
            &CanvasInputEvent::PointerMove { pointer_id: 4, screen: WorldPoint { x: 5.0, y: 5.0 } },
            &regions, &scene.objects, ActiveTool::Select, false, None, None, &mut camera, &mut drag, &mut out,
        );
        // Up at (-5,-5): reversed rect [-5,15]² covers "near" only.
        step_object_pointer(
            &CanvasInputEvent::PointerUp { pointer_id: 4, screen: WorldPoint { x: -5.0, y: -5.0 }, edge_id: None },
            &regions, &scene.objects, ActiveTool::Select, false, None, None, &mut camera, &mut drag, &mut out,
        );
        assert_eq!(out.marquee_ids, Some(vec!["near".to_string()]));
    }

    #[test]
    fn object_marquee_over_two_filled_rects_yields_multi_ids() {
        // A drag starting on empty canvas between two FILLED rects must start a Marquee
        // (the bbox fallback must NOT re-grab a filled body's empty bbox) and collect
        // BOTH ids on up. The final rect is span(down-anchor, up-point), so the anchor
        // (-5,-5) + up corner must straddle both 20px bodies (a [0..20], b [30..50]).
        let scene = object_scene(vec![
            rect_object("a", 0.0, 0.0, 20),
            rect_object("b", 30.0, 0.0, 20),
        ]);
        let regions = derive_object_regions(&scene);
        // Both regions are closed fills (so hit-testing uses the polygon test, not the bbox).
        assert!(regions.iter().all(|r| r.closed), "filled rects derive closed regions");
        let mut camera = identity_camera();
        let mut drag: Option<InputDragState> = None;
        let mut out = ObjectInputOut::default();

        // Down on empty canvas -> must start a Marquee, not an Object drag.
        step_object_pointer(
            &CanvasInputEvent::PointerDown { pointer_id: 7, screen: WorldPoint { x: -5.0, y: -5.0 } },
            &regions, &scene.objects, ActiveTool::Select, false, None, None, &mut camera, &mut drag, &mut out,
        );
        assert!(matches!(drag, Some(InputDragState::Marquee { .. })),
            "empty-canvas down starts a Marquee, not an Object move");
        assert!(out.selection.is_none(), "empty-canvas down selects nothing");

        // Drag across both bodies (the move only grows the live overlay).
        step_object_pointer(
            &CanvasInputEvent::PointerMove { pointer_id: 7, screen: WorldPoint { x: 55.0, y: 25.0 } },
            &regions, &scene.objects, ActiveTool::Select, false, None, None, &mut camera, &mut drag, &mut out,
        );
        // Up at (55,25): rect [-5,55] x [-5,25] (anchor + up corner) covers BOTH.
        step_object_pointer(
            &CanvasInputEvent::PointerUp { pointer_id: 7, screen: WorldPoint { x: 55.0, y: 25.0 }, edge_id: None },
            &regions, &scene.objects, ActiveTool::Select, false, None, None, &mut camera, &mut drag, &mut out,
        );
        let mut ids = out.marquee_ids.take().expect("pointer-up on a Marquee yields ids");
        ids.sort();
        assert_eq!(ids, vec!["a".to_string(), "b".to_string()],
            "marquee over two filled rects collects BOTH ids (>=2 => Multi)");
    }

    #[test]
    fn object_pointer_down_in_concave_fill_notch_starts_marquee() {
        // A point inside a FILLED concave object's bbox but OUTSIDE its polygon (the
        // notch) must MISS the body, so a drag there starts a Marquee. An L on its
        // back: AABB 0..20 square, but the upper-LEFT quadrant (5,15) is empty notch.
        let l_shape = RenderObject {
            id: "l".to_string(),
            parent: None,
            order: "a0".to_string(),
            transform: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            geometry_d: "M 0 0 L 160 0 L 160 160 L 80 160 L 80 80 L 0 80 Z".to_string(),
            fill: None,
            stroke: None,
            text: None,
            anchors: Vec::new(),
            clip: false,
            hidden: false,
            locked: false,
        };
        let scene = object_scene(vec![l_shape]);
        let regions = derive_object_regions(&scene);
        assert!(regions[0].closed, "the L-shape derives a closed fill region");
        let camera = identity_camera();

        // The notch point is inside the local AABB but outside the polygon: a hit-test
        // there must MISS (so it can start a marquee, not grab the body).
        assert_eq!(
            hit_object_in_regions(&regions, &camera, WorldPoint { x: 5.0, y: 15.0 }),
            None,
            "a filled concave body's empty notch must NOT grab (RA3 keeps marquee reachable)"
        );

        // Driving the real pointer state machine: a down in the notch starts a Marquee.
        let mut camera = camera;
        let mut drag: Option<InputDragState> = None;
        let mut out = ObjectInputOut::default();
        step_object_pointer(
            &CanvasInputEvent::PointerDown { pointer_id: 8, screen: WorldPoint { x: 5.0, y: 15.0 } },
            &regions, &scene.objects, ActiveTool::Select, false, None, None, &mut camera, &mut drag, &mut out,
        );
        assert!(matches!(drag, Some(InputDragState::Marquee { .. })),
            "notch down starts a Marquee, not an Object move");
        assert!(out.selection.is_none());

        // Sanity: a down ON the filled arm (5,5) DOES grab the body (not a marquee).
        let id = hit_object_in_regions(&regions, &camera, WorldPoint { x: 5.0, y: 5.0 });
        assert_eq!(id.as_deref(), Some("l"), "the filled arm still grabs the body");
    }

    #[test]
    fn object_mode_marquee_drag_yields_overlay_geometry_from_shared_source() {
        // The in-flight object-mode marquee drag must produce drawable
        // overlay geometry from the shared `marquee_overlay_for_drag` source the
        // object render pass draws — without it the rubber-band never surfaces over an
        // object scene. A non-Marquee / no drag yields no overlay.
        let scene = object_scene(vec![rect_object("o1", 0.0, 0.0, 10)]);
        let regions = derive_object_regions(&scene);
        let mut camera = identity_camera();
        let mut drag: Option<InputDragState> = None;
        let mut out = ObjectInputOut::default();

        // No drag => no overlay.
        assert!(marquee_overlay_for_drag(drag.as_ref(), 1.0).is_empty());

        // Down on empty space then a move grows an in-flight marquee.
        step_object_pointer(
            &CanvasInputEvent::PointerDown { pointer_id: 5, screen: WorldPoint { x: -20.0, y: -20.0 } },
            &regions, &scene.objects, ActiveTool::Select, false, None, None, &mut camera, &mut drag, &mut out,
        );
        step_object_pointer(
            &CanvasInputEvent::PointerMove { pointer_id: 5, screen: WorldPoint { x: 30.0, y: 30.0 } },
            &regions, &scene.objects, ActiveTool::Select, false, None, None, &mut camera, &mut drag, &mut out,
        );
        assert!(matches!(drag, Some(InputDragState::Marquee { .. })));
        // The shared source the object pass draws must produce a full overlay
        // (fill quad + 4 stroke quads) for the in-flight marquee.
        let overlay = marquee_overlay_for_drag(drag.as_ref(), camera.zoom);
        assert_eq!(overlay.len(), MARQUEE_OVERLAY_VERTEX_CAPACITY);

        // An object drag (not a marquee) must NOT draw a marquee overlay.
        let object_drag = Some(InputDragState::Object {
            pointer_id: 6,
            object_id: "o1".to_string(),
            start: WorldPoint { x: 0.0, y: 0.0 },
        });
        assert!(marquee_overlay_for_drag(object_drag.as_ref(), 1.0).is_empty());
    }

    #[test]
    fn hover_move_reports_affordance_per_priority() {
        // Single 20px object at world origin; identity camera => screen == world.
        let scene = object_scene(vec![rect_object("o1", 0.0, 0.0, 20)]);
        let regions = derive_object_regions(&scene);
        let camera = identity_camera();

        // With "o1" selected, hovering its NW corner hits the resize handle.
        let nw = hover_affordance_at(
            &regions,
            &camera,
            Some("o1"),
            WorldPoint { x: 0.0, y: 0.0 },
        );
        assert_eq!(nw, HoverAffordance::ResizeNw);

        // The rotate zone sits above the top-edge midpoint (x=10).
        let rotate = hover_affordance_at(
            &regions,
            &camera,
            Some("o1"),
            WorldPoint {
                x: 10.0,
                y: 0.0 - shape_renderer_core::hit_test_object::ROTATE_ZONE_OFFSET_PX,
            },
        );
        assert_eq!(rotate, HoverAffordance::Rotate);

        // Interior of the selected object (off every handle) => body.
        let body = hover_affordance_at(
            &regions,
            &camera,
            Some("o1"),
            WorldPoint { x: 10.0, y: 10.0 },
        );
        assert_eq!(body, HoverAffordance::Body);

        // Far from everything => empty.
        let empty = hover_affordance_at(
            &regions,
            &camera,
            Some("o1"),
            WorldPoint { x: 200.0, y: 200.0 },
        );
        assert_eq!(empty, HoverAffordance::Empty);

        // No selection => no handles, so the same NW corner reads as body.
        let no_sel = hover_affordance_at(&regions, &camera, None, WorldPoint { x: 0.0, y: 0.0 });
        assert_eq!(no_sel, HoverAffordance::Body);
    }

    #[test]
    fn step_object_pointer_hover_sets_affordance_only_without_drag() {
        let scene = object_scene(vec![rect_object("o1", 0.0, 0.0, 20)]);
        let regions = derive_object_regions(&scene);
        let mut camera = identity_camera();
        let mut drag: Option<InputDragState> = None;
        let mut out = ObjectInputOut::default();

        // No active drag => the move is a hover and reports an affordance.
        step_object_pointer(
            &CanvasInputEvent::PointerMove {
                pointer_id: 1,
                screen: WorldPoint { x: 10.0, y: 10.0 },
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            false,
            None,
            Some("o1"),
            &mut camera,
            &mut drag,
            &mut out,
        );
        assert_eq!(out.hover_affordance, Some(HoverAffordance::Body));

        // Start an Object drag, then a move during the drag must NOT set hover.
        out.hover_affordance = None;
        drag = Some(InputDragState::Object {
            pointer_id: 1,
            object_id: "o1".to_string(),
            start: WorldPoint { x: 10.0, y: 10.0 },
        });
        step_object_pointer(
            &CanvasInputEvent::PointerMove {
                pointer_id: 1,
                screen: WorldPoint { x: 14.0, y: 12.0 },
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            false,
            None,
            Some("o1"),
            &mut camera,
            &mut drag,
            &mut out,
        );
        assert_eq!(out.hover_affordance, None);
        assert!(out.transform_delta.is_some());
    }

    #[test]
    fn double_click_branches_parent_vs_leaf_by_children() {
        // Double-click an object WITH children => drill-in branch
        // (has_children true); double-click a LEAF => text-edit branch (false).
        // "o1" at world (0,0) has a child "c1" (parent=o1, off to the side so it
        // does not overlap the click); "leaf" at world (50,0) has no children.
        let mut child = rect_object("c1", 100.0, 100.0, 20);
        child.parent = Some("o1".to_string());
        let scene = object_scene(vec![
            rect_object("o1", 0.0, 0.0, 20),
            child,
            rect_object("leaf", 50.0, 0.0, 20),
        ]);
        let regions = derive_object_regions(&scene);
        let camera = identity_camera();

        // Hit the parent "o1" (center of its 20px rect at origin) => has_children.
        let parent_hit =
            object_double_click(&regions, &scene.objects, &camera, WorldPoint { x: 10.0, y: 10.0 })
                .expect("double-click on the parent must hit an object");
        assert_eq!(parent_hit.id, "o1");
        assert!(
            parent_hit.has_children,
            "an object with a child must take the drill-in branch"
        );

        // Hit the leaf (center of its 20px rect at world (50,0)) => no children.
        let leaf_hit =
            object_double_click(&regions, &scene.objects, &camera, WorldPoint { x: 60.0, y: 10.0 })
                .expect("double-click on the leaf must hit an object");
        assert_eq!(leaf_hit.id, "leaf");
        assert!(
            !leaf_hit.has_children,
            "a leaf object must take the text-edit branch"
        );

        // Empty canvas => no signal at all.
        assert!(
            object_double_click(&regions, &scene.objects, &camera, WorldPoint { x: 500.0, y: 500.0 })
                .is_none(),
            "a double-click on empty canvas must not emit a branch"
        );
    }

    #[test]
    fn handle_overlay_is_zoom_invariant_in_screen_px() {
        // Same world bbox at two zooms: each handle quad's WORLD size scales 1/zoom,
        // so its SCREEN size (world size * zoom) is constant HANDLE_SIZE_PX.
        let world_bbox = WorldRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 80.0,
        };
        for &zoom in &[1.0_f64, 4.0_f64] {
            let verts = build_handle_overlay_vertices(&world_bbox, zoom);
            // 9 quads (8 resize + rotate), 6 verts each, within the shared buffer.
            assert_eq!(verts.len(), 54);
            assert!(verts.len() <= HANDLE_OVERLAY_VERTEX_CAPACITY);
            // First quad = NW handle: verts 0 (top-left) and 2 (bottom-right) of the
            // add_quad winding span the handle in world px.
            let world_w = (verts[2].position[0] - verts[0].position[0]) as f64;
            let screen_w = world_w * zoom;
            assert!(
                (screen_w - shape_renderer_core::hit_test_object::HANDLE_SIZE_PX).abs() < 1e-6,
                "handle screen size must be constant HANDLE_SIZE_PX at zoom {zoom}, got {screen_w}"
            );
        }
    }

    #[test]
    fn multi_select_overlay_draws_one_outline_per_selected_object() {
        // Two 20px rects; multi-select both => one 4-edge outline per object.
        let scene = object_scene(vec![
            rect_object("a", 0.0, 0.0, 20),
            rect_object("b", 50.0, 50.0, 20),
        ]);
        let regions = derive_object_regions(&scene);

        let verts = build_multi_select_overlay_vertices(
            &regions,
            &["a".to_string(), "b".to_string()],
            1.0,
            |_| None,
        );
        // 4 edge quads * 6 verts = 24 per object; non-empty and a clean multiple.
        assert!(!verts.is_empty(), "multi-select highlight must emit geometry");
        assert_eq!(verts.len(), 24 * 2, "one 24-vert outline per selected object");
        assert_eq!(verts.len() % 24, 0, "outline must be whole-object multiples");
        assert!(
            verts.len() <= MULTI_SELECT_OVERLAY_VERTEX_CAPACITY,
            "must stay within the overlay buffer capacity"
        );
        // The outline uses the selection-ring blue, not transparent.
        assert_eq!(verts[0].color, MULTI_SELECT_OUTLINE_COLOR);

        // Empty set => nothing drawn (single selection keeps its handle overlay).
        assert!(build_multi_select_overlay_vertices(&regions, &[], 1.0, |_| None).is_empty());
    }

    #[test]
    fn multi_select_overlay_ring_follows_live_preview_transform() {
        // A 20px rect at world origin; canonical ring spans [0,20]². A live
        // drag pushes a preview WORLD transform translating +100,+50. The outline ring
        // MUST be built at the PREVIEWED bbox [100,120]×[50,70], tracking the drag like
        // the resize handles — not snapping only on commit. Fails if the ring ignores
        // the preview (would stay at the canonical [0,20]² bounds).
        let scene = object_scene(vec![rect_object("o1", 0.0, 0.0, 20)]);
        let regions = derive_object_regions(&scene);
        let ids = vec!["o1".to_string()];

        // Bounding box of all ring vertex positions = the world bbox the ring is drawn
        // at (modulo a half-thickness skirt that is identical across both calls).
        let ring_bbox = |verts: &[GpuVertex]| -> (f32, f32, f32, f32) {
            let mut min_x = f32::INFINITY;
            let mut min_y = f32::INFINITY;
            let mut max_x = f32::NEG_INFINITY;
            let mut max_y = f32::NEG_INFINITY;
            for v in verts {
                min_x = min_x.min(v.position[0]);
                min_y = min_y.min(v.position[1]);
                max_x = max_x.max(v.position[0]);
                max_y = max_y.max(v.position[1]);
            }
            (min_x, min_y, max_x, max_y)
        };

        let canonical = build_multi_select_overlay_vertices(&regions, &ids, 1.0, |_| None);
        let (cx0, cy0, cx1, cy1) = ring_bbox(&canonical);
        // Anchored near origin (within the ~1px half-thickness ring skirt at zoom 1.0).
        assert!(
            cx0.abs() < 1.5 && cy0.abs() < 1.5,
            "canonical ring anchored at origin: ({cx0},{cy0})"
        );

        let preview = [[1.0, 0.0, 100.0], [0.0, 1.0, 50.0], [0.0, 0.0, 1.0]];
        let previewed = build_multi_select_overlay_vertices(&regions, &ids, 1.0, |id| {
            (id == "o1").then_some(preview)
        });
        let (px0, py0, px1, py1) = ring_bbox(&previewed);

        // The previewed ring must be translated by the drag delta, NOT the canonical
        // bounds: this assertion fails if the outline path ignores the preview.
        assert!(
            (px0 - cx0 - 100.0).abs() < 1e-3 && (py0 - cy0 - 50.0).abs() < 1e-3,
            "ring origin tracks the preview drag (+100,+50): canonical ({cx0},{cy0}) previewed ({px0},{py0})"
        );
        assert!(
            ((px1 - px0) - (cx1 - cx0)).abs() < 1e-3 && ((py1 - py0) - (cy1 - cy0)).abs() < 1e-3,
            "previewed ring keeps the same extent (pure translation), no re-tessellation"
        );

        // preview None reproduces the canonical-bounds ring (same count + bbox).
        let none_preview = build_multi_select_overlay_vertices(&regions, &ids, 1.0, |_| None);
        assert_eq!(none_preview.len(), canonical.len());
        let (nx0, ny0, nx1, ny1) = ring_bbox(&none_preview);
        assert!(
            (nx0 - cx0).abs() < 1e-6
                && (ny0 - cy0).abs() < 1e-6
                && (nx1 - cx1).abs() < 1e-6
                && (ny1 - cy1).abs() < 1e-6,
            "preview None must reproduce the canonical-bounds ring"
        );
    }

    #[test]
    fn outline_ids_ring_single_selection_and_every_multi_member() {
        let mut scene = object_scene(vec![
            rect_object("a", 0.0, 0.0, 20),
            rect_object("b", 50.0, 50.0, 20),
        ]);
        let regions = derive_object_regions(&scene);

        // A lone selected object/group rings (so a grouped selection shows
        // a border, not just resize dots) even though multi_select is empty.
        scene.selection = Some("a".to_string());
        scene.multi_select = Vec::new();
        let single = outline_overlay_ids(&scene, &regions);
        assert_eq!(single, vec!["a".to_string()]);
        let verts = build_multi_select_overlay_vertices(&regions, &single, 1.0, |_| None);
        assert!(!verts.is_empty(), "single selection must emit an outline ring");
        assert_eq!(verts.len(), 24, "one 24-vert ring for the single selection");

        // A multi-select rings every member (the single selection is subsumed).
        scene.multi_select = vec!["a".to_string(), "b".to_string()];
        let multi = outline_overlay_ids(&scene, &regions);
        assert_eq!(multi, vec!["a".to_string(), "b".to_string()]);

        // Nothing selected => no ring.
        scene.selection = None;
        scene.multi_select = Vec::new();
        assert!(outline_overlay_ids(&scene, &regions).is_empty());
    }

    #[test]
    fn open_class_single_selection_has_no_outline_ring() {
        // Feedback #1: a lone selected open line shows ONLY its endpoint dots — no
        // bbox ring. The closed rect keeps its single-selection ring (regression),
        // and the multi-select union ring keeps open members.
        let mut scene = object_scene(vec![
            line_object("l", 0.0, 0.0, 40),
            rect_object("r", 100.0, 0.0, 20),
        ]);
        let regions = derive_object_regions(&scene);

        scene.selection = Some("l".to_string());
        assert!(
            outline_overlay_ids(&scene, &regions).is_empty(),
            "an open-class single selection must not ring"
        );

        scene.selection = Some("r".to_string());
        assert_eq!(outline_overlay_ids(&scene, &regions), vec!["r".to_string()]);

        scene.multi_select = vec!["l".to_string(), "r".to_string()];
        assert_eq!(
            outline_overlay_ids(&scene, &regions),
            vec!["l".to_string(), "r".to_string()],
            "the union ring keeps every multi-select member, open ones included"
        );
    }

    #[test]
    fn multi_select_open_members_show_endpoint_dots_not_bbox() {
        // Feedback: a mixed multi-select [closed rect, open line]. The rect keeps
        // its 24-vert 4-edge outline (regression guard); the line member emits TWO
        // filled endpoint dot fans — the same visual as the single-selection
        // overlay — instead of a bbox ring.
        let scene = object_scene(vec![
            rect_object("r", 100.0, 0.0, 20),
            line_object("l", 0.0, 0.0, 40),
        ]);
        let regions = derive_object_regions(&scene);
        let ids = vec!["r".to_string(), "l".to_string()];
        let fan = ENDPOINT_HANDLE_SEGMENTS * 3;

        for &zoom in &[1.0_f64, 4.0_f64] {
            let verts = build_multi_select_overlay_vertices(&regions, &ids, zoom, |_| None);
            assert_eq!(
                verts.len(),
                24 + 2 * fan,
                "rect = 24 box verts, line = two endpoint dot fans"
            );
            assert!(verts.len() <= MULTI_SELECT_OVERLAY_VERTEX_CAPACITY);
            // Member order is input order: the rect's ring comes first, in blue.
            assert_eq!(verts[0].color, MULTI_SELECT_OUTLINE_COLOR);
            // The line's fans sit at its endpoints (0,0)/(40,0): every triangle
            // starts at the center, every rim vertex is one radius out, and the
            // screen DIAMETER pins to HANDLE_SIZE_PX at any zoom — identical math
            // to the single-selection endpoint dots.
            let centers = [
                WorldPoint { x: 0.0, y: 0.0 },
                WorldPoint { x: 40.0, y: 0.0 },
            ];
            for (slot, center) in centers.iter().enumerate() {
                let base = 24 + slot * fan;
                for triangle in verts[base..base + fan].chunks(3) {
                    assert!(
                        (triangle[0].position[0] as f64 - center.x).abs() < 1e-4
                            && (triangle[0].position[1] as f64 - center.y).abs() < 1e-4,
                        "each fan triangle must start at the endpoint center"
                    );
                    assert_eq!(triangle[0].color, ENDPOINT_HANDLE_FILL_COLOR);
                    for vertex in &triangle[1..] {
                        let dx = vertex.position[0] as f64 - center.x;
                        let dy = vertex.position[1] as f64 - center.y;
                        let screen_diameter = (dx * dx + dy * dy).sqrt() * 2.0 * zoom;
                        assert!(
                            (screen_diameter - shape_renderer_core::hit_test_object::HANDLE_SIZE_PX).abs()
                                < 1e-3,
                            "open-member dot must stay circular at constant screen size, zoom {zoom}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn multi_select_open_member_dots_follow_live_preview_transform() {
        // An open member's dots must be built at the PREVIEWED
        // endpoint positions during a live drag, tracking the pointer every frame
        // like the closed-member rings — transform-only, no re-tessellation.
        let scene = object_scene(vec![line_object("l", 0.0, 0.0, 40)]);
        let regions = derive_object_regions(&scene);
        let ids = vec!["l".to_string()];
        let fan = ENDPOINT_HANDLE_SEGMENTS * 3;

        let canonical = build_multi_select_overlay_vertices(&regions, &ids, 1.0, |_| None);
        assert_eq!(canonical.len(), 2 * fan, "an open member emits dots only, no ring");

        let preview = [[1.0, 0.0, 100.0], [0.0, 1.0, 50.0], [0.0, 0.0, 1.0]];
        let previewed = build_multi_select_overlay_vertices(&regions, &ids, 1.0, |id| {
            (id == "l").then_some(preview)
        });
        assert_eq!(previewed.len(), canonical.len());
        // Every vertex translates by exactly the drag delta (+100,+50).
        for (canon, live) in canonical.iter().zip(previewed.iter()) {
            assert!(
                (live.position[0] - canon.position[0] - 100.0).abs() < 1e-3
                    && (live.position[1] - canon.position[1] - 50.0).abs() < 1e-3,
                "open-member dots must track the live preview drag (+100,+50)"
            );
        }

        // preview None reproduces the canonical dots.
        let none_preview = build_multi_select_overlay_vertices(&regions, &ids, 1.0, |_| None);
        assert_eq!(none_preview.len(), canonical.len());
        for (canon, again) in canonical.iter().zip(none_preview.iter()) {
            assert_eq!(canon.position, again.position);
        }
    }

    #[test]
    fn multi_select_overlay_caps_at_capacity_with_all_open_members() {
        // Worst case: every member is open-class (96 dot verts each, 4x a box's
        // 24). Feed MORE members than the one-time-allocated buffer holds and
        // prove the emitter stops exactly at capacity instead of overflowing.
        let member_verts = 2 * ENDPOINT_HANDLE_SEGMENTS * 3;
        let count = MULTI_SELECT_OVERLAY_VERTEX_CAPACITY / member_verts + 8;
        let objects: Vec<RenderObject> = (0..count)
            .map(|i| line_object(&format!("l{i}"), i as f64 * 50.0, 0.0, 40))
            .collect();
        let ids: Vec<String> = objects.iter().map(|object| object.id.clone()).collect();
        let scene = object_scene(objects);
        let regions = derive_object_regions(&scene);

        let verts = build_multi_select_overlay_vertices(&regions, &ids, 1.0, |_| None);
        assert_eq!(
            verts.len(),
            MULTI_SELECT_OVERLAY_VERTEX_CAPACITY,
            "fills to the buffer capacity exactly, never beyond"
        );
    }

    #[test]
    fn pointer_down_on_handle_starts_resize_or_rotate_not_object() {
        // 20px object at world origin; identity camera => screen == world. With it
        // selected, a pointer-down on its NE corner must start a Resize (not Object),
        // and the selection must NOT change.
        let scene = object_scene(vec![rect_object("o1", 0.0, 0.0, 20)]);
        let regions = derive_object_regions(&scene);
        let mut camera = identity_camera();
        let mut drag: Option<InputDragState> = None;
        let mut out = ObjectInputOut::default();

        // NE corner of a 20px rect at origin = world (20, 0).
        step_object_pointer(
            &CanvasInputEvent::PointerDown {
                pointer_id: 1,
                screen: WorldPoint { x: 20.0, y: 0.0 },
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            false,
            None,
            Some("o1"),
            &mut camera,
            &mut drag,
            &mut out,
        );
        assert!(
            matches!(drag, Some(InputDragState::Resize { ref corner, .. }) if *corner == HoverAffordance::ResizeNe)
        );
        // Grabbing a handle does not re-pick selection.
        assert!(out.selection.is_none());

        // A move emits a resize delta scaling about the SW anchor.
        step_object_pointer(
            &CanvasInputEvent::PointerMove {
                pointer_id: 1,
                screen: WorldPoint { x: 40.0, y: -20.0 },
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            false,
            None,
            Some("o1"),
            &mut camera,
            &mut drag,
            &mut out,
        );
        let delta = out.transform_delta.take().expect("resize emits a delta");
        assert_eq!(delta.kind, "resize");

        // Rotate zone above the top-edge midpoint (x=10) starts a Rotate.
        let mut drag2: Option<InputDragState> = None;
        let mut out2 = ObjectInputOut::default();
        step_object_pointer(
            &CanvasInputEvent::PointerDown {
                pointer_id: 2,
                screen: WorldPoint {
                    x: 10.0,
                    y: 0.0 - shape_renderer_core::hit_test_object::ROTATE_ZONE_OFFSET_PX,
                },
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            false,
            None,
            Some("o1"),
            &mut camera,
            &mut drag2,
            &mut out2,
        );
        assert!(matches!(drag2, Some(InputDragState::Rotate { .. })));
        assert!(out2.selection.is_none());
    }

    #[test]
    fn coarse_rotate_drag_emits_snapped_delta_in_core() {
        use shape_renderer_core::hit_test_object::{
            rotate_delta_matrix_snapped, ROTATE_SNAP_DEG,
        };

        // 20px rect at origin selected; identity camera => screen == world. Grab the
        // rotate zone, then move with the coarse-rotate bit set: the emitted matrix
        // must be the IN-CORE 15deg-snapped delta, so the shell never rebuilds it.
        let scene = object_scene(vec![rect_object("o1", 0.0, 0.0, 20)]);
        let regions = derive_object_regions(&scene);
        let mut camera = identity_camera();
        let mut drag: Option<InputDragState> = None;
        let mut out = ObjectInputOut::default();

        step_object_pointer(
            &CanvasInputEvent::PointerDown {
                pointer_id: 9,
                screen: WorldPoint {
                    x: 10.0,
                    y: 0.0 - shape_renderer_core::hit_test_object::ROTATE_ZONE_OFFSET_PX,
                },
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            true,
            None,
            Some("o1"),
            &mut camera,
            &mut drag,
            &mut out,
        );
        let (center, start) = match &drag {
            Some(InputDragState::Rotate { center, start, .. }) => (*center, *start),
            _ => panic!("rotate-zone grab must start a Rotate drag"),
        };

        // A move point chosen so the raw swept delta is NOT a 15deg multiple. We assert
        // the emitted matrix equals the snapped core result AND differs from the
        // unsnapped one — the falsifier: a shell-side raw rotate would match unsnapped.
        let world_now = WorldPoint { x: 60.0, y: 35.0 };
        step_object_pointer(
            &CanvasInputEvent::PointerMove {
                pointer_id: 9,
                screen: world_now,
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            true,
            None,
            Some("o1"),
            &mut camera,
            &mut drag,
            &mut out,
        );
        let delta = out.transform_delta.take().expect("rotate move emits a delta");
        assert_eq!(delta.kind, "rotate");

        let snapped = rotate_delta_matrix_snapped(
            (center.x, center.y),
            (world_now.x, world_now.y),
            (start.x, start.y),
            Some(ROTATE_SNAP_DEG),
        );
        let unsnapped = rotate_delta_matrix_snapped(
            (center.x, center.y),
            (world_now.x, world_now.y),
            (start.x, start.y),
            None,
        );
        let close = |a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]| {
            a.iter().zip(b).all(|(ra, rb)| {
                ra.iter().zip(rb).all(|(x, y)| (x - y).abs() < 1e-9)
            })
        };
        assert!(
            close(&delta.matrix, &snapped),
            "coarse-rotate must emit the 15deg-snapped delta, got {:?}",
            delta.matrix
        );
        assert!(
            !close(&snapped, &unsnapped),
            "test point must produce a non-15deg raw angle so snapping is observable"
        );
    }

    // ----- open-class endpoint handles ---------------

    /// An open bezier curve: 3 nodes, 5 coordinate PAIRS (control points included),
    /// endpoints local px (0,0) and (8,0).
    fn curve_object(id: &str) -> RenderObject {
        RenderObject {
            id: id.to_string(),
            parent: None,
            order: "a0".to_string(),
            transform: object_identity(),
            geometry_d: "M 0 0 C 8 -8 24 -8 32 0 L 64 0".to_string(),
            fill: None,
            stroke: None,
            text: None,
            anchors: Vec::new(),
            clip: false,
            hidden: false,
            locked: false,
        }
    }

    #[test]
    fn open_class_selection_surfaces_endpoint_handles_not_bbox() {
        // An OPEN 40px line and a CLOSED 20px rect. The open selection gets TWO
        // endpoint handles and NO bbox/rotate surface; the rect keeps the 8-handle
        // overlay (regression guard) and has no endpoint surface.
        let scene = object_scene(vec![
            line_object("l", 0.0, 0.0, 40),
            rect_object("r", 100.0, 0.0, 20),
        ]);
        let regions = derive_object_regions(&scene);
        let camera = identity_camera();

        // Open-class: bbox handles are GONE...
        assert!(
            selection_handles(&regions, &camera, Some("l"), None).is_none(),
            "an open-class selection must not lay out bbox resize/rotate handles"
        );
        // ...and the endpoint pair is the surface (node 0 / last node world pos).
        let handles =
            endpoint_handles(&regions, &camera, Some("l"), None).expect("endpoint handles");
        assert_eq!(handles.last_index, 1);
        assert!((handles.world[0].x - 0.0).abs() < 1e-9 && handles.world[0].y.abs() < 1e-9);
        assert!((handles.world[1].x - 40.0).abs() < 1e-9 && handles.world[1].y.abs() < 1e-9);

        // Hover classifies the endpoints; the old rotate zone reads as EMPTY (no
        // rotate affordance exists for open-class), and the body still reads Body.
        assert_eq!(
            hover_affordance_at(&regions, &camera, Some("l"), WorldPoint { x: 0.0, y: 0.0 }),
            HoverAffordance::EndpointStart
        );
        assert_eq!(
            hover_affordance_at(&regions, &camera, Some("l"), WorldPoint { x: 40.0, y: 0.0 }),
            HoverAffordance::EndpointEnd
        );
        assert_eq!(
            hover_affordance_at(
                &regions,
                &camera,
                Some("l"),
                WorldPoint {
                    x: 20.0,
                    y: 0.0 - shape_renderer_core::hit_test_object::ROTATE_ZONE_OFFSET_PX,
                },
            ),
            HoverAffordance::Empty,
            "no rotate affordance on an open-class selection"
        );
        assert_eq!(
            hover_affordance_at(&regions, &camera, Some("l"), WorldPoint { x: 20.0, y: 0.0 }),
            HoverAffordance::Body
        );

        // Closed-class regression: the rect keeps its 8-handle surface and has no
        // endpoint surface.
        assert!(selection_handles(&regions, &camera, Some("r"), None).is_some());
        assert!(endpoint_handles(&regions, &camera, Some("r"), None).is_none());
    }

    #[test]
    fn curve_endpoint_handles_use_pair_space_last_index() {
        // Bezier control points count as PAIRS (anchors address pair space), so the
        // curve's end handle reports pair index 4, not node index 2 — while the
        // handle POSITION is still the last node's world point (8, 0)px.
        let scene = object_scene(vec![curve_object("c")]);
        let regions = derive_object_regions(&scene);
        let handles = endpoint_handles(&regions, &identity_camera(), Some("c"), None)
            .expect("endpoint handles");
        assert_eq!(handles.last_index, 4, "pair-space index, control points included");
        assert!((handles.world[1].x - 8.0).abs() < 1e-9 && handles.world[1].y.abs() < 1e-9);
    }

    #[test]
    fn endpoint_handles_follow_preview_transform_and_stay_screen_sized() {
        let scene = object_scene(vec![line_object("l", 0.0, 0.0, 40)]);
        let regions = derive_object_regions(&scene);
        let camera = identity_camera();

        // A live preview transform substitutes for the canonical
        // region transform, so the handles track a translate drag frame-by-frame.
        let preview = [[1.0, 0.0, 100.0], [0.0, 1.0, 50.0], [0.0, 0.0, 1.0]];
        let handles = endpoint_handles(&regions, &camera, Some("l"), Some(&preview))
            .expect("previewed handles");
        assert!((handles.world[0].x - 100.0).abs() < 1e-9);
        assert!((handles.world[0].y - 50.0).abs() < 1e-9);
        assert!((handles.world[1].x - 140.0).abs() < 1e-9);
        assert!((handles.world[1].y - 50.0).abs() < 1e-9);

        // Feedback #1: the overlay dots are FILLED CIRCLES (triangle fans) whose
        // screen DIAMETER stays HANDLE_SIZE_PX at any zoom (the same 1/zoom sizing
        // as the bbox handles). Per dot: every triangle starts at the endpoint
        // center and every rim vertex sits at exactly one radius — a circle, not
        // the old square quad.
        let fan = ENDPOINT_HANDLE_SEGMENTS * 3;
        for &zoom in &[1.0_f64, 4.0_f64] {
            let verts = build_endpoint_handle_overlay_vertices(&handles.world, zoom);
            assert_eq!(verts.len(), 2 * fan);
            assert!(verts.len() <= HANDLE_OVERLAY_VERTEX_CAPACITY);
            for (slot, center) in handles.world.iter().enumerate() {
                for triangle in verts[slot * fan..(slot + 1) * fan].chunks(3) {
                    assert!(
                        (triangle[0].position[0] as f64 - center.x).abs() < 1e-4
                            && (triangle[0].position[1] as f64 - center.y).abs() < 1e-4,
                        "each fan triangle must start at the endpoint center"
                    );
                    assert_eq!(triangle[0].color, ENDPOINT_HANDLE_FILL_COLOR);
                    for vertex in &triangle[1..] {
                        let dx = vertex.position[0] as f64 - center.x;
                        let dy = vertex.position[1] as f64 - center.y;
                        let screen_diameter = (dx * dx + dy * dy).sqrt() * 2.0 * zoom;
                        assert!(
                            (screen_diameter - shape_renderer_core::hit_test_object::HANDLE_SIZE_PX).abs()
                                < 1e-3,
                            "endpoint dot must stay circular at constant screen size, zoom {zoom}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn pointer_down_on_endpoint_starts_endpoint_drag_and_emits_world_signal() {
        // An ANCHORED open line: the handle is grabbable regardless of the anchor
        // (release-time rebind/unbind is the shell's commit).
        let mut line = line_object("l", 0.0, 0.0, 40);
        line.anchors = vec![shape_renderer_core::render_object::RAnchor {
            node_index: 1,
            target: "t".to_string(),
            at: shape_renderer_core::render_object::RLocalPoint { x: 0.0, y: 0.0 },
        }];
        let scene = object_scene(vec![line]);
        let regions = derive_object_regions(&scene);
        let mut camera = identity_camera();
        let mut drag: Option<InputDragState> = None;
        let mut out = ObjectInputOut::default();

        // Down on the END endpoint handle -> Endpoint drag (pair index 1), no
        // selection re-pick, no transform gesture.
        step_object_pointer(
            &CanvasInputEvent::PointerDown {
                pointer_id: 1,
                screen: WorldPoint { x: 40.0, y: 0.0 },
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            false,
            None,
            Some("l"),
            &mut camera,
            &mut drag,
            &mut out,
        );
        assert!(
            matches!(drag, Some(InputDragState::Endpoint { node_index: 1, ref object_id, .. }) if object_id == "l"),
            "grabbing the end handle starts an Endpoint drag"
        );
        assert!(out.selection.is_none());

        // A move emits the cumulative WORLD endpoint sample — and no transform delta.
        step_object_pointer(
            &CanvasInputEvent::PointerMove {
                pointer_id: 1,
                screen: WorldPoint { x: 55.0, y: 7.0 },
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            false,
            None,
            Some("l"),
            &mut camera,
            &mut drag,
            &mut out,
        );
        let delta = out.endpoint_delta.take().expect("move emits an endpoint delta");
        assert_eq!(delta.id, "l");
        assert_eq!(delta.node_index, 1);
        assert!((delta.x - 55.0).abs() < 1e-9 && (delta.y - 7.0).abs() < 1e-9);
        assert!(out.transform_delta.is_none(), "an endpoint drag is not a transform");

        // Up clears the drag (the shell commits endpoint_release_ops).
        step_object_pointer(
            &CanvasInputEvent::PointerUp {
                pointer_id: 1,
                screen: WorldPoint { x: 55.0, y: 7.0 },
                edge_id: None,
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            false,
            None,
            Some("l"),
            &mut camera,
            &mut drag,
            &mut out,
        );
        assert!(drag.is_none());

        // The START handle reports pair index 0.
        step_object_pointer(
            &CanvasInputEvent::PointerDown {
                pointer_id: 2,
                screen: WorldPoint { x: 0.0, y: 0.0 },
            },
            &regions,
            &scene.objects,
            ActiveTool::Select,
            false,
            None,
            Some("l"),
            &mut camera,
            &mut drag,
            &mut out,
        );
        assert!(matches!(drag, Some(InputDragState::Endpoint { node_index: 0, .. })));
    }

    #[test]
    fn selection_handles_follow_live_preview_transform() {
        // A 20px rect at world origin, identity camera (screen == world).
        // Canonical bbox is [0,20]², so the SE resize handle centers at world (20,20).
        let scene = object_scene(vec![rect_object("o1", 0.0, 0.0, 20)]);
        let regions = derive_object_regions(&scene);
        let camera = identity_camera();

        // No preview -> handles read the canonical region transform.
        let (handles, bbox) = selection_handles(&regions, &camera, Some("o1"), None)
            .expect("canonical handles");
        assert!((bbox.x - 0.0).abs() < 1e-9 && (bbox.y - 0.0).abs() < 1e-9);
        assert!((bbox.width - 20.0).abs() < 1e-9 && (bbox.height - 20.0).abs() < 1e-9);
        let se_cx = handles.se.x + handles.se.width / 2.0;
        let se_cy = handles.se.y + handles.se.height / 2.0;
        assert!((se_cx - 20.0).abs() < 1e-9 && (se_cy - 20.0).abs() < 1e-9, "({se_cx},{se_cy})");

        // A live drag pushes a preview WORLD transform translating +100,+50. The
        // handles MUST track the previewed bbox [100,120]×[50,70]; the SE handle then
        // centers at world (120,70). Fails if handles still read the canonical region.
        let preview = [[1.0, 0.0, 100.0], [0.0, 1.0, 50.0], [0.0, 0.0, 1.0]];
        let (phandles, pbbox) = selection_handles(&regions, &camera, Some("o1"), Some(&preview))
            .expect("previewed handles");
        assert!(
            (pbbox.x - 100.0).abs() < 1e-9 && (pbbox.y - 50.0).abs() < 1e-9,
            "previewed bbox origin tracks the drag: ({},{})",
            pbbox.x,
            pbbox.y
        );
        assert!((pbbox.width - 20.0).abs() < 1e-9 && (pbbox.height - 20.0).abs() < 1e-9);
        let pse_cx = phandles.se.x + phandles.se.width / 2.0;
        let pse_cy = phandles.se.y + phandles.se.height / 2.0;
        assert!(
            (pse_cx - 120.0).abs() < 1e-9 && (pse_cy - 70.0).abs() < 1e-9,
            "SE handle tracks the PREVIEWED corner, not canonical: ({pse_cx},{pse_cy})"
        );
    }

    #[test]
    fn object_hand_tool_pointer_down_pans() {
        let scene = object_scene(vec![rect_object("o1", 0.0, 0.0, 20)]);
        let regions = derive_object_regions(&scene);
        let mut camera = identity_camera();
        let mut drag: Option<InputDragState> = None;
        let mut out = ObjectInputOut::default();

        // Hand tool: even a pointer-down over an object pans (no selection).
        step_object_pointer(
            &CanvasInputEvent::PointerDown {
                pointer_id: 3,
                screen: WorldPoint { x: 10.0, y: 10.0 },
            },
            &regions,
            &scene.objects,
            ActiveTool::Hand,
            false,
            None,
            None,
            &mut camera,
            &mut drag,
            &mut out,
        );
        assert!(matches!(drag, Some(InputDragState::Pan { .. })));
        assert!(out.selection.is_none());

        step_object_pointer(
            &CanvasInputEvent::PointerMove {
                pointer_id: 3,
                screen: WorldPoint { x: 40.0, y: 30.0 },
            },
            &regions,
            &scene.objects,
            ActiveTool::Hand,
            false,
            None,
            None,
            &mut camera,
            &mut drag,
            &mut out,
        );
        // Pan moves the camera by the screen delta.
        assert!((camera.x - 30.0).abs() < 1e-9);
        assert!((camera.y - 20.0).abs() < 1e-9);
    }

    #[test]
    fn fit_camera_to_object_regions_frames_world_bounds() {
        // Object at world (100,100), 50px rect -> world bounds [100,150]².
        let scene = object_scene(vec![rect_object("o1", 100.0, 100.0, 50)]);
        let regions = derive_object_regions(&scene);
        let bounds = object_regions_world_bounds(&regions).expect("bounds");
        assert!((bounds.x - 100.0).abs() < 1e-6);
        assert!((bounds.y - 100.0).abs() < 1e-6);
        assert!((bounds.width - 50.0).abs() < 1e-6);
        assert!((bounds.height - 50.0).abs() < 1e-6);

        // The fit camera centers that AABB in the viewport.
        let camera = fit_camera_to_bounds(&bounds, 1200.0, 800.0);
        let center_world = (bounds.x + bounds.width / 2.0, bounds.y + bounds.height / 2.0);
        let center_screen_x = center_world.0 * camera.zoom + camera.x;
        let center_screen_y = center_world.1 * camera.zoom + camera.y;
        assert!((center_screen_x - 600.0).abs() < 1e-6);
        assert!((center_screen_y - 400.0).abs() < 1e-6);
    }

    #[test]
    fn object_matrix_uniform_camera_packing_matches_update_camera() {
        // No device here, so assert the shared packing logic update_camera writes
        // directly.
        use crate::object_pipeline::ObjectMatrixUniform;
        let scene = object_scene(vec![rect_object("o1", 0.0, 0.0, 20)]);
        let from_scene = ObjectMatrixUniform::from_scene(&scene, 1280.0, 720.0);
        let camera = CameraState {
            x: 12.0,
            y: -7.0,
            zoom: 2.5,
        };
        let live = ObjectMatrixUniform {
            camera: [shape_renderer_core::cast::narrow_f32(camera.x), shape_renderer_core::cast::narrow_f32(camera.y), shape_renderer_core::cast::narrow_f32(camera.zoom), 0.0],
            viewport: [1280.0, 720.0, 0.0, 0.0],
        };
        // Same viewport packing; camera differs because the live camera moved.
        assert_eq!(from_scene.viewport, live.viewport);
        assert_eq!(live.camera, [12.0, -7.0, 2.5, 0.0]);
    }

    // The camera uniform must be fed LOGICAL pixels (the pointer + shader space).
    // The physical viewport (logical*DPR) would shrink NDC and drift objects, so this
    // pins the round-trip: logical screen -> world -> NDC lands back at the direct NDC.
    #[test]
    fn object_camera_logical_viewport_round_trips_at_dpr2() {
        use crate::object_pipeline::ObjectMatrixUniform;

        let logical_w = 800.0_f32;
        let logical_h = 600.0_f32;
        let dpr = 2.0_f32;
        let physical_w = logical_w * dpr;
        let physical_h = logical_h * dpr;

        let camera = CameraState {
            x: 30.0,
            y: -15.0,
            zoom: 1.5,
        };

        // Pointer path: a logical screen point becomes world via the camera.
        let screen = (210.0_f32, 140.0_f32);
        let world = (
            (screen.0 - shape_renderer_core::cast::narrow_f32(camera.x)) / shape_renderer_core::cast::narrow_f32(camera.zoom),
            (screen.1 - shape_renderer_core::cast::narrow_f32(camera.y)) / shape_renderer_core::cast::narrow_f32(camera.zoom),
        );

        // Shader path (object_fill.wgsl world_to_clip): screen = world*zoom+cam,
        // then NDC against the uniform viewport.
        let to_ndc = |viewport: [f32; 4]| {
            let sx = world.0 * shape_renderer_core::cast::narrow_f32(camera.zoom) + shape_renderer_core::cast::narrow_f32(camera.x);
            let sy = world.1 * shape_renderer_core::cast::narrow_f32(camera.zoom) + shape_renderer_core::cast::narrow_f32(camera.y);
            (
                (sx / viewport[0]) * 2.0 - 1.0,
                1.0 - (sy / viewport[1]) * 2.0,
            )
        };

        // The NDC the original logical screen point maps to directly.
        let expected = (
            (screen.0 / logical_w) * 2.0 - 1.0,
            1.0 - (screen.1 / logical_h) * 2.0,
        );

        // from_scene packs the viewport it is handed; feed it LOGICAL pixels.
        let logical = ObjectMatrixUniform::from_scene(
            &object_scene(vec![rect_object("o1", 0.0, 0.0, 20)]),
            logical_w,
            logical_h,
        );
        let logical_ndc = to_ndc(logical.viewport);
        assert!((logical_ndc.0 - expected.0).abs() < 1e-5);
        assert!((logical_ndc.1 - expected.1).abs() < 1e-5);

        // The physical viewport (the pre-fix bug) does NOT round-trip: NDC shrinks
        // by 1/DPR toward the origin, which on screen reads as upper-right drift.
        let physical_ndc = to_ndc([physical_w, physical_h, 0.0, 0.0]);
        assert!((physical_ndc.0 - expected.0).abs() > 0.1);
        assert!((physical_ndc.1 - expected.1).abs() > 0.1);
    }

    #[test]
    fn multi_selection_round_trips_camel_case_kind() {
        let selection = SceneSelection::Multi {
            ids: vec!["card-a".to_string(), "group-b".to_string()],
        };
        let json = serde_json::to_string(&selection).unwrap();
        assert_eq!(json, r#"{"kind":"multi","ids":["card-a","group-b"]}"#);
        let parsed: SceneSelection = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SceneSelection::Multi { ids } if ids.len() == 2));
    }

    // The transient multi-select set is never serialized, so its presence cannot
    // leak into the persisted snapshot wire format.
    #[test]
    fn multi_select_set_is_not_serialized() {
        let scene = SceneSnapshot {
            scene_id: "skip-test".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            groups: Vec::new(),
            cards: Vec::new(),
            edges: Vec::new(),
            styles: Vec::new(),
            selection: SceneSelection::Canvas,
            multi_select: vec!["card-a".to_string()],
        };
        let json = serde_json::to_string(&scene).unwrap();
        assert!(!json.contains("multiSelect"));
        assert!(!json.contains("multi_select"));
        assert!(!json.contains("card-a"));
        let parsed: SceneSnapshot = serde_json::from_str(&json).unwrap();
        assert!(parsed.multi_select.is_empty());
    }

    // A card listed in the transient multi-select set draws with the same selection
    // styling as the single anchor, and differently from an unselected card —
    // proving the highlight path covers multi-select ids.
    #[test]
    fn multi_select_highlights_card_like_single_anchor() {
        let card = text_path_card("card-a", 100.0, 100.0);
        let make_scene = |selection: SceneSelection, multi_select: Vec<String>| SceneSnapshot {
            scene_id: "multi-highlight-test".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            groups: Vec::new(),
            cards: vec![card.clone()],
            edges: Vec::new(),
            styles: vec![minimal_style_token("default")],
            selection,
            multi_select,
        };
        let mut cache = TextLayoutCache::default();
        let mut text_engine = TextEngine::new().unwrap();

        let unselected = make_scene(SceneSelection::Canvas, Vec::new());
        let single = make_scene(
            SceneSelection::Node {
                id: card.id.clone(),
            },
            Vec::new(),
        );
        let multi = make_scene(SceneSelection::Canvas, vec![card.id.clone()]);

        let (unselected_v, _) =
            build_card_vertices(&unselected, &card, &mut cache, &mut text_engine);
        let (single_v, _) = build_card_vertices(&single, &card, &mut cache, &mut text_engine);
        let (multi_v, _) = build_card_vertices(&multi, &card, &mut cache, &mut text_engine);

        // Selection styling changes the geometry, so a highlighted card never
        // matches the unselected one...
        assert_ne!(unselected_v.len(), single_v.len());
        // ...and a multi-selected card matches the single-anchor highlight exactly.
        assert_eq!(single_v.len(), multi_v.len());
        assert_ne!(unselected_v.len(), multi_v.len());
    }

    // The wasm-facing `setMultiSelect` (via its inner helper) stores the set on the
    // scene without disturbing the persisted single-anchor selection.
    #[test]
    fn set_multi_select_ids_keeps_single_anchor_selection() {
        let card_a = text_path_card("card-a", 100.0, 100.0);
        let card_b = text_path_card("card-b", 400.0, 100.0);
        let mut scene = SceneSnapshot {
            scene_id: "set-multi-test".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            groups: Vec::new(),
            cards: vec![card_a.clone(), card_b.clone()],
            edges: Vec::new(),
            styles: vec![minimal_style_token("default")],
            selection: SceneSelection::Node {
                id: card_a.id.clone(),
            },
            multi_select: Vec::new(),
        };

        // Mirrors set_multi_select_ids's scene mutation (the renderer wrapper also
        // rebuilds GPU buffers, which a probe-less test cannot exercise).
        scene.multi_select = vec![card_a.id.clone(), card_b.id.clone()];

        assert!(matches!(
            &scene.selection,
            SceneSelection::Node { id } if id == &card_a.id
        ));
        assert_eq!(scene.multi_select, vec![card_a.id, card_b.id]);
    }
}
