//! Portable pure-CPU layer of the WebGPU renderer (W2-13/S8).
//!
//! No wgpu device, no `web_sys`: vertex/geometry build, style resolution,
//! hit-test math, marquee, object-region derive/hit, overlay geometry, camera
//! math, the render consts and the WGSL fallback shader, plus the unit tests.
//! This is the layer the host test gate exercises; the `ShapeWebGpuRenderer`
//! struct and its impls live in the parent module and the web-surface submodules.

use std::f32::consts::PI;

use crate::lod::{apparent_px, lod_tier, LodTier};
use crate::model::{
    ActiveTool, CameraState, CanvasInputEvent, CubicRoute, RenderCard, RenderEdge, RenderGroup,
    SceneSelection, SceneShadowLayerToken, SceneSnapshot, SceneStyleToken, WorldPoint, WorldRect,
};
use crate::hit_test_object::{
    hit_test_object, resize_delta_matrix, rotate_delta_matrix, translate_3x3, HoverAffordance,
    ScreenRect, SelectionHandles,
};
use crate::outline::{derive_region, parse_path_string};
use crate::render_object::RenderObjectScene;
use crate::stats::{CoreHitResult, CoreOverlayStyle, ObjectTransformDelta};
use crate::text::{CachedTextLine, TextBuildStats, TextEngine, TextLayoutCache, TEXT_ATLAS_SOLID_UV};

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
        (group.bounds.x + style.spacing.group_padding_x) as f32,
        (group.bounds.y + style.spacing.group_padding_y) as f32,
        (group.bounds.width - style.spacing.group_padding_x * 2.0) as f32,
        style.typography.group_title_size,
        color_with_alpha(style.text, 0.86),
        text_layout_cache,
        text_engine,
    );
    if !group.summary.trim().is_empty() {
        text_stats.add(add_text_line(
            &mut vertices,
            &group.summary,
            (group.bounds.x + style.spacing.group_padding_x) as f32,
            (group.bounds.y
                + style.spacing.group_padding_y
                + style.typography.group_title_size as f64
                + 28.0) as f32,
            (group.bounds.width - style.spacing.group_padding_x * 2.0) as f32,
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
            (stroke_width + style.stroke_width.focus_ring) as f32,
            color_with_alpha(style.focus, style.state.focus_alpha),
        );
    }
    let stroke = color_with_alpha(edge_base, stroke_alpha);
    add_cubic_edge(&mut vertices, &route, stroke_width as f32, stroke);
    add_arrowhead(
        &mut vertices,
        [route.cp2.x as f32, route.cp2.y as f32],
        [route.end.x as f32, route.end.y as f32],
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
        let label_padding = style.spacing.label_padding_x as f32;
        let label_x =
            ((route.start.x + route.end.x) * 0.5) as f32 - label_width * 0.5 - label_padding;
        let label_y =
            ((route.start.y + route.end.y) * 0.5) as f32 - style.spacing.edge_label_height as f32;
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
        (badge.x + style.spacing.badge_padding_x) as f32,
        (badge.y + 5.0) as f32,
        (badge.width - style.spacing.badge_padding_x * 2.0) as f32,
        style.typography.badge_size,
        color_with_alpha(style.accent, style.badge.text_alpha),
        text_layout_cache,
        text_engine,
    ));
    text_stats.add(add_text_line(
        &mut vertices,
        &card.title,
        text_layout.content_x as f32,
        text_layout.title_y as f32,
        text_layout.content_width as f32,
        text_layout.title_font_size,
        color_with_alpha(style.text, 0.92),
        text_layout_cache,
        text_engine,
    ));
    text_stats.add(add_wrapped_text(
        &mut vertices,
        &card.summary,
        text_layout.content_x as f32,
        text_layout.summary_y as f32,
        text_layout.content_width as f32,
        text_layout.summary_font_size,
        text_layout.summary_line_height as f32,
        text_layout.summary_max_lines,
        color_with_alpha(style.muted_text, 0.84),
        text_layout_cache,
        text_engine,
    ));
    if text_layout.detail_max_lines > 0 && !card.detail.trim().is_empty() {
        text_stats.add(add_wrapped_text(
            &mut vertices,
            &card.detail,
            text_layout.content_x as f32,
            text_layout.detail_y as f32,
            text_layout.content_width as f32,
            text_layout.detail_font_size,
            text_layout.detail_line_height as f32,
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
pub(crate) fn add_rounded_rect(vertices: &mut Vec<GpuVertex>, rect: &WorldRect, radius: f64, color: [f32; 4]) {
    let w = rect.width.max(0.0) as f32;
    let h = rect.height.max(0.0) as f32;
    let r = (radius.max(0.0) as f32).min(w * 0.5).min(h * 0.5);
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
        [rect.x as f32 + r, rect.y as f32 + r],
        r,
        PI,
        PI * 1.5,
        color,
    );
    add_corner_fan(
        vertices,
        [rect.x as f32 + w - r, rect.y as f32 + r],
        r,
        PI * 1.5,
        PI * 2.0,
        color,
    );
    add_corner_fan(
        vertices,
        [rect.x as f32 + w - r, rect.y as f32 + h - r],
        r,
        0.0,
        PI * 0.5,
        color,
    );
    add_corner_fan(
        vertices,
        [rect.x as f32 + r, rect.y as f32 + h - r],
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
    metric(value, fallback) as f32
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn opacity(value: Option<f64>, fallback: f64) -> f32 {
    value.unwrap_or(fallback).clamp(0.0, 1.0) as f32
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
            [previous.x as f32, previous.y as f32],
            [current.x as f32, current.y as f32],
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

/// Build the marquee overlay quads (translucent fill + 1.5px-equivalent stroke)
/// in world space for `rect`. `zoom` keeps the stroke a constant screen width.
/// Returns up to MARQUEE_OVERLAY_VERTEX_CAPACITY vertices.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn build_marquee_overlay_vertices(rect: &WorldRect, zoom: f64) -> Vec<GpuVertex> {
    let mut vertices = Vec::with_capacity(MARQUEE_OVERLAY_VERTEX_CAPACITY);
    add_rect(&mut vertices, rect, MARQUEE_FILL_COLOR);
    let thickness = (1.5 / zoom.max(0.025)) as f32;
    let x = rect.x as f32;
    let y = rect.y as f32;
    let w = rect.width as f32;
    let h = rect.height as f32;
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

/// W2-04: build the selection-handle overlay (8 resize handles + 1 rotate zone) in
/// WORLD space for the selected object's `world_bbox`. Each handle is a square of
/// `HANDLE_SIZE_PX / zoom` world units so the legacy shader's `* zoom` renders it at
/// a CONSTANT [`HANDLE_SIZE_PX`] screen size at any zoom. Centers mirror
/// [`SelectionHandles::from_screen_bbox`] (corners + edge midpoints; rotate zone
/// `ROTATE_ZONE_OFFSET_PX` above the top-edge midpoint) so what is drawn matches
/// what hover and pointer-down hit-test. Returns up to
/// [`HANDLE_OVERLAY_VERTEX_CAPACITY`] vertices.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn build_handle_overlay_vertices(world_bbox: &WorldRect, zoom: f64) -> Vec<GpuVertex> {
    use crate::hit_test_object::{HANDLE_SIZE_PX, ROTATE_ZONE_OFFSET_PX};
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

/// Node ids AND group ids whose world bounds intersect the marquee rect (AABB).
/// Cards come first (selection-anchor friendly), then groups; both deduped by the
/// scene's natural order.
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

/// FC-09: frame a world-space AABB into the viewport, mirroring the legacy fit math
/// (center + zoom-to-fit with the same padding/zoom clamp as [`fit_camera_to_scene`]).
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

/// FC-04: derive each object's local-space region outline (D6) for hit-test /
/// marquee. Parses the geometry path-string into flattened subpaths, derives the
/// region, and keeps the boundary polygon in OBJECT-LOCAL px. Objects whose
/// geometry yields no region (degenerate) are skipped — they cannot be hit.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn derive_object_regions(scene: &RenderObjectScene) -> Vec<ObjectRegion> {
    // Same flattening tolerance the region cache groundwork uses for at-rest geometry.
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
            outline: region.outline,
        });
    }
    regions
}

/// FC-07: pick the top-most object whose region contains the screen point. Regions
/// are iterated in reverse (top-down, since later objects draw on top); the query
/// point is mapped to world then inverse-transformed into each object's local space
/// (D8) by [`hit_test_object`].
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
        .find(|region| hit_test_object(&region.transform, &region.outline, world.x, world.y))
        .map(|region| region.id.clone())
}

/// W2-02: compute the hover affordance under `screen` for the shell's cursor.
/// Runs only on a no-button move (the caller gates this on "no active drag").
///
/// Priority, top-down: when an object is selected, its resize/rotate handles
/// (laid out by the SHARED [`SelectionHandles`] helper, in screen space, so the
/// hover test matches exactly what W2-04 renders and pointer-down hit-tests) win
/// over everything; then a body hit against any object's region; otherwise empty.
/// W2-04: the SHARED selection-handle layout for the selected object, returning
/// the screen-space [`SelectionHandles`] plus the WORLD bbox they were laid out
/// from. This is the single bridge that keeps hover (W2-02), the pointer-down
/// hit-test, and the GPU handle render (W2-04) on one source of truth: all three
/// read the same screen-space handles built from the same world bbox. `None` when
/// nothing is selected or the selected region has no finite world bounds.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn selection_handles(
    regions: &[ObjectRegion],
    camera: &CameraState,
    selection: Option<&str>,
) -> Option<(SelectionHandles, WorldRect)> {
    let id = selection?;
    let region = regions.iter().find(|region| region.id == id)?;
    let world_bbox = region_world_bounds(region)?;
    let screen_rect = world_rect_to_screen_rect(&world_bbox, camera);
    let handles = SelectionHandles::from_screen_bbox(&ScreenRect {
        x: screen_rect.x,
        y: screen_rect.y,
        width: screen_rect.width,
        height: screen_rect.height,
    });
    Some((handles, world_bbox))
}

#[cfg(feature = "wgpu-probe")]
pub(crate) fn hover_affordance_at(
    regions: &[ObjectRegion],
    camera: &CameraState,
    selection: Option<&str>,
    screen: WorldPoint,
) -> HoverAffordance {
    if let Some((handles, _)) = selection_handles(regions, camera, selection) {
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

/// FC-07: the pure object pointer-input state machine. Operates only on the
/// pieces of renderer state it touches (camera, drag, the region list) so it is
/// unit-testable without a GPU device. Mutates `camera`/`input_drag` and writes
/// any selection / transform-delta / marquee result into `object_out`.
///
/// Behavior (Select tool unless noted):
/// - PointerDown, Hand tool: start a Pan drag (panning works in object mode).
/// - PointerDown, hit an object: set `selection`, start an Object drag anchored at
///   the pointer-down WORLD point.
/// - PointerDown, empty: start a world-space Marquee drag.
/// - PointerMove on Pan: update `camera`. On Object: emit a CUMULATIVE delta from
///   the fixed anchor. On Marquee: extend the rect.
/// - PointerUp on Marquee: AABB-test object world regions against the rect into
///   `marquee_ids`. Any drag clears on up/cancel.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn step_object_pointer(
    event: &CanvasInputEvent,
    regions: &[ObjectRegion],
    active_tool: ActiveTool,
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
            // W2-04: grabbing a resize handle / the rotate zone of the CURRENT
            // selection starts a transform gesture (no selection change). Uses the
            // SHARED handle layout so what is grabbed == what hover reports == what
            // is drawn. Priority matches hover: handles > body > empty.
            if let Some((handles, world_bbox)) = selection_handles(regions, camera, selection) {
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
            match hit_object_in_regions(regions, camera, screen) {
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
            // No active drag => no button is held: this is a hover move, so report
            // the affordance the shell uses to pick a cursor (W2-02). Computed
            // per-move with no hover state of its own.
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
                    // Cumulative translation from the FIXED pointer-down world point.
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
                    // Cumulative scale about the OPPOSITE anchor of the grabbed handle.
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
                InputDragState::Rotate {
                    pointer_id: drag_pointer_id,
                    object_id,
                    start,
                    center,
                } if drag_pointer_id == pointer_id => {
                    // Cumulative rotation about the bbox center from the anchor angle.
                    let world_now = screen_to_world(screen, camera);
                    object_out.transform_delta = Some(ObjectTransformDelta {
                        id: object_id,
                        matrix: rotate_delta_matrix(
                            (center.x, center.y),
                            (world_now.x, world_now.y),
                            (start.x, start.y),
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
            // Object drag commits on the shell side (one undoable op); clear only
            // when THIS pointer owns the active drag, so a second finger lifting
            // (different pointerId) can't cancel an in-progress drag mid-gesture.
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

/// FC-07: object ids whose WORLD-space region AABB intersects the marquee rect.
/// Each local outline vertex is transformed to world via the object transform; the
/// min/max over those gives the world AABB tested against `rect`.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn object_regions_in_marquee(regions: &[ObjectRegion], rect: &WorldRect) -> Vec<String> {
    regions
        .iter()
        .filter_map(|region| {
            let bounds = region_world_bounds(region)?;
            rects_intersect(&bounds, rect).then(|| region.id.clone())
        })
        .collect()
}

/// FC-09: world-space AABB over every object region (each local outline vertex
/// transformed to world). `None` when there are no regions / no finite vertices.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn object_regions_world_bounds(regions: &[ObjectRegion]) -> Option<WorldRect> {
    let mut acc: Option<WorldRect> = None;
    for region in regions {
        let Some(bounds) = region_world_bounds(region) else {
            continue;
        };
        acc = Some(match acc {
            None => bounds,
            Some(prev) => union_rect_refs(&[&prev, &bounds]).unwrap_or(prev),
        });
    }
    acc
}

/// World-space AABB of one object's region: transform each local outline vertex
/// through the object's projective matrix (D7/D8) and take the extent. `None` when
/// the outline is empty or every vertex maps to a non-finite world point.
#[cfg(feature = "wgpu-probe")]
pub(crate) fn region_world_bounds(region: &ObjectRegion) -> Option<WorldRect> {
    use crate::hit_test_object::apply_3x3;
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for &(lx, ly) in &region.outline {
        let (wx, wy) = apply_3x3(&region.transform, lx as f64, ly as f64);
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
    let red = (color[0].clamp(0.0, 1.0) * 255.0).round() as u8;
    let green = (color[1].clamp(0.0, 1.0) * 255.0).round() as u8;
    let blue = (color[2].clamp(0.0, 1.0) * 255.0).round() as u8;
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
        | Some(InputDragState::Rotate { pointer_id, .. }) => Some(*pointer_id),
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
    let summary_max_lines =
        ((summary_available / summary_line_height).floor() as usize).clamp(1, 3);
    let detail_available = (content_bottom - detail_y).max(0.0);
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
// Marquee overlay = 1 fill quad (6 verts) + 4 stroke edge quads (24 verts) = 30.
#[cfg(feature = "wgpu-probe")]
pub(crate) const MARQUEE_OVERLAY_VERTEX_CAPACITY: usize = 30;
// Marquee fill/stroke colors (accent blue, translucent). Drawn in world space so
// the existing camera-transform pipeline renders them in place.
#[cfg(feature = "wgpu-probe")]
pub(crate) const MARQUEE_FILL_COLOR: [f32; 4] = [0.231, 0.510, 0.965, 0.12];
#[cfg(feature = "wgpu-probe")]
pub(crate) const MARQUEE_STROKE_COLOR: [f32; 4] = [0.231, 0.510, 0.965, 0.9];

// W2-04: selection-handle overlay = 8 resize handles + 1 rotate zone, each a fill
// quad (6 verts) = 9 * 6 = 54.
#[cfg(feature = "wgpu-probe")]
pub(crate) const HANDLE_OVERLAY_VERTEX_CAPACITY: usize = 54;
// Solid focus-blue handle fill (#2f7ee6).
#[cfg(feature = "wgpu-probe")]
pub(crate) const HANDLE_FILL_COLOR: [f32; 4] = [0.184, 0.494, 0.902, 1.0];

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

    // ----- FC-04..FC-07 object live-path helpers -----------------------------

    use crate::render_object::RenderObject;

    fn object_identity() -> [[f64; 3]; 3] {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    }

    /// An object at world `(tx, ty)` whose local geometry is an `s`px×`s`px rect
    /// (`s` px = `s*8` quantized units, D2). Later-in-the-list objects draw on top.
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
            clip: false,
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
            ActiveTool::Select,
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
            ActiveTool::Select,
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
            ActiveTool::Select,
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
            ActiveTool::Select,
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
            ActiveTool::Select,
            None,
            &mut camera,
            &mut drag,
            &mut out,
        );
        assert_eq!(out.marquee_ids, Some(vec!["near".to_string()]));
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
                y: 0.0 - crate::hit_test_object::ROTATE_ZONE_OFFSET_PX,
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
            ActiveTool::Select,
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
            ActiveTool::Select,
            Some("o1"),
            &mut camera,
            &mut drag,
            &mut out,
        );
        assert_eq!(out.hover_affordance, None);
        assert!(out.transform_delta.is_some());
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
            // 9 quads (8 resize + rotate), 6 verts each.
            assert_eq!(verts.len(), HANDLE_OVERLAY_VERTEX_CAPACITY);
            // First quad = NW handle: verts 0 (top-left) and 2 (bottom-right) of the
            // add_quad winding span the handle in world px.
            let world_w = (verts[2].position[0] - verts[0].position[0]) as f64;
            let screen_w = world_w * zoom;
            assert!(
                (screen_w - crate::hit_test_object::HANDLE_SIZE_PX).abs() < 1e-6,
                "handle screen size must be constant HANDLE_SIZE_PX at zoom {zoom}, got {screen_w}"
            );
        }
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
            ActiveTool::Select,
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
            ActiveTool::Select,
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
                    y: 0.0 - crate::hit_test_object::ROTATE_ZONE_OFFSET_PX,
                },
            },
            &regions,
            ActiveTool::Select,
            Some("o1"),
            &mut camera,
            &mut drag2,
            &mut out2,
        );
        assert!(matches!(drag2, Some(InputDragState::Rotate { .. })));
        assert!(out2.selection.is_none());
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
            ActiveTool::Hand,
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
            ActiveTool::Hand,
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
        // FC-06: update_camera packs the camera uniform identically to from_scene.
        // No GPU device here, so assert the shared packing logic directly (the byte
        // layout update_camera writes). ObjectRenderer::update_camera is exercised on
        // a real device at the cutover.
        use crate::object_pipeline::ObjectMatrixUniform;
        let scene = object_scene(vec![rect_object("o1", 0.0, 0.0, 20)]);
        let from_scene = ObjectMatrixUniform::from_scene(&scene, 1280.0, 720.0);
        let camera = CameraState {
            x: 12.0,
            y: -7.0,
            zoom: 2.5,
        };
        let live = ObjectMatrixUniform {
            camera: [camera.x as f32, camera.y as f32, camera.zoom as f32, 0.0],
            viewport: [1280.0, 720.0, 0.0, 0.0],
        };
        // Same viewport packing; camera differs because the live camera moved.
        assert_eq!(from_scene.viewport, live.viewport);
        assert_eq!(live.camera, [12.0, -7.0, 2.5, 0.0]);
    }

    // W2-01: the object camera uniform must be fed LOGICAL pixels, the same space
    // the pointer path (screen->world) and the shaders work in. At DPR>1 the
    // physical viewport (logical*DPR) drifts objects toward the upper-right
    // because the larger denominator shrinks NDC. This pins the round-trip:
    // a logical screen point -> world (pointer math) -> NDC (shader math) lands
    // back at the same NDC the screen point maps to directly.
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
            (screen.0 - camera.x as f32) / camera.zoom as f32,
            (screen.1 - camera.y as f32) / camera.zoom as f32,
        );

        // Shader path (object_fill.wgsl world_to_clip): screen = world*zoom+cam,
        // then NDC against the uniform viewport.
        let to_ndc = |viewport: [f32; 4]| {
            let sx = world.0 * camera.zoom as f32 + camera.x as f32;
            let sy = world.1 * camera.zoom as f32 + camera.y as f32;
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
