//! Debug utilities for visualizing layout bounds

use std::sync::Arc;

use avenger_chart_core::AxisPosition;
use avenger_common::types::ColorOrGradient;
use avenger_scales::color::parse_color_string;
use avenger_scenegraph::marks::{mark::SceneMark, rect::SceneRectMark, text::SceneTextMark};
use avenger_text::types::{TextAlign, TextBaseline};

use crate::layout::{ContentLayout, EdgeSlabs, FrameLayout, LayoutBounds, OverflowSide};

const MIN_DEBUG_RECT_SIZE: f32 = 0.01;
const DEBUG_OVERLAY_LABEL_FONT_SIZE: f32 = 7.0;
const DEBUG_OVERLAY_LABEL_PAD: f32 = 3.0;
const DEBUG_OVERLAY_LABEL_AVG_CHAR_WIDTH: f32 = DEBUG_OVERLAY_LABEL_FONT_SIZE * 0.55;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutDebugLayer {
    FrameAllocation,
    ContentRect,
    OwnedSlab,
    ResidualOverflow,
    ChildFrameAllocation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutDebugRect {
    pub bounds: LayoutBounds,
    pub label: String,
    pub layer: LayoutDebugLayer,
    pub side: Option<OverflowSide>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameDebugOverlay {
    pub rects: Vec<LayoutDebugRect>,
}

impl FrameDebugOverlay {
    pub fn from_content_layout(content_layout: &ContentLayout, origin: [f32; 2]) -> Self {
        let child_rects = content_layout
            .child_frame_allocations
            .iter()
            .map(|allocation| allocation.rect)
            .collect::<Vec<_>>();
        Self::from_content_layout_with_child_rects(content_layout, origin, child_rects)
    }

    pub fn from_content_layout_with_child_rects(
        content_layout: &ContentLayout,
        origin: [f32; 2],
        child_frame_rects: Vec<LayoutBounds>,
    ) -> Self {
        let mut overlay = Self::default();
        let frame_allocation = content_layout.allocation.frame;
        let content_rect = content_layout.allocation.content_rect;
        overlay.push_rect(
            translate_bounds(frame_allocation.rect, origin),
            "frame",
            LayoutDebugLayer::FrameAllocation,
        );
        overlay.push_rect(
            translate_bounds(content_rect, origin),
            "content",
            LayoutDebugLayer::ContentRect,
        );
        overlay.push_slab_rects(
            content_rect,
            frame_allocation.owned_slabs,
            origin,
            "owned",
            LayoutDebugLayer::OwnedSlab,
        );
        overlay.push_slab_rects(
            content_rect,
            content_layout
                .frame_demand
                .residual_overflow(frame_allocation.owned_slabs),
            origin,
            "resid",
            LayoutDebugLayer::ResidualOverflow,
        );
        for (idx, bounds) in child_frame_rects.into_iter().enumerate() {
            overlay.push_rect(
                translate_bounds(bounds, origin),
                format!("child-{idx}"),
                LayoutDebugLayer::ChildFrameAllocation,
            );
        }
        overlay
    }

    fn push_slab_rects(
        &mut self,
        content_rect: LayoutBounds,
        slabs: EdgeSlabs,
        origin: [f32; 2],
        label_prefix: &str,
        layer: LayoutDebugLayer,
    ) {
        for (side, bounds) in slab_rects(content_rect, slabs) {
            self.push_slab_rect(
                translate_bounds(bounds, origin),
                side,
                format!("{label_prefix}-{}", side_label(side)),
                layer,
            );
        }
    }

    fn push_rect(
        &mut self,
        bounds: LayoutBounds,
        label: impl Into<String>,
        layer: LayoutDebugLayer,
    ) {
        if bounds.width <= MIN_DEBUG_RECT_SIZE || bounds.height <= MIN_DEBUG_RECT_SIZE {
            return;
        }
        self.rects.push(LayoutDebugRect {
            bounds,
            label: label.into(),
            layer,
            side: None,
        });
    }

    fn push_slab_rect(
        &mut self,
        bounds: LayoutBounds,
        side: OverflowSide,
        label: impl Into<String>,
        layer: LayoutDebugLayer,
    ) {
        if bounds.width <= MIN_DEBUG_RECT_SIZE || bounds.height <= MIN_DEBUG_RECT_SIZE {
            return;
        }
        self.rects.push(LayoutDebugRect {
            bounds,
            label: label.into(),
            layer,
            side: Some(side),
        });
    }
}

fn translate_bounds(bounds: LayoutBounds, origin: [f32; 2]) -> LayoutBounds {
    LayoutBounds {
        x: bounds.x + origin[0],
        y: bounds.y + origin[1],
        ..bounds
    }
}

fn slab_rects(content_rect: LayoutBounds, slabs: EdgeSlabs) -> Vec<(OverflowSide, LayoutBounds)> {
    let mut rects = Vec::with_capacity(4);
    if slabs.top > MIN_DEBUG_RECT_SIZE {
        rects.push((
            OverflowSide::Top,
            LayoutBounds {
                x: content_rect.x,
                y: content_rect.y - slabs.top,
                width: content_rect.width,
                height: slabs.top,
            },
        ));
    }
    if slabs.right > MIN_DEBUG_RECT_SIZE {
        rects.push((
            OverflowSide::Right,
            LayoutBounds {
                x: content_rect.x + content_rect.width,
                y: content_rect.y,
                width: slabs.right,
                height: content_rect.height,
            },
        ));
    }
    if slabs.bottom > MIN_DEBUG_RECT_SIZE {
        rects.push((
            OverflowSide::Bottom,
            LayoutBounds {
                x: content_rect.x,
                y: content_rect.y + content_rect.height,
                width: content_rect.width,
                height: slabs.bottom,
            },
        ));
    }
    if slabs.left > MIN_DEBUG_RECT_SIZE {
        rects.push((
            OverflowSide::Left,
            LayoutBounds {
                x: content_rect.x - slabs.left,
                y: content_rect.y,
                width: slabs.left,
                height: content_rect.height,
            },
        ));
    }
    rects
}

fn side_label(side: OverflowSide) -> &'static str {
    match side {
        OverflowSide::Top => "top",
        OverflowSide::Right => "right",
        OverflowSide::Bottom => "bottom",
        OverflowSide::Left => "left",
    }
}

fn debug_rect_mark(
    bounds: LayoutBounds,
    stroke_color: [f32; 4],
    fill_color: [f32; 4],
    stroke_width: f32,
    zindex: i32,
) -> SceneMark {
    SceneMark::Rect(SceneRectMark {
        x: bounds.x.into(),
        y: bounds.y.into(),
        width: Some(bounds.width.into()),
        height: Some(bounds.height.into()),
        fill: ColorOrGradient::Color(fill_color).into(),
        stroke: ColorOrGradient::Color(stroke_color).into(),
        stroke_width: stroke_width.into(),
        zindex: Some(zindex),
        clip: false,
        ..Default::default()
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct OverlayLabelPlacement {
    x: f32,
    y: f32,
    angle: f32,
    align: TextAlign,
    baseline: TextBaseline,
}

fn estimated_debug_label_width(text: &str) -> f32 {
    text.chars().count() as f32 * DEBUG_OVERLAY_LABEL_AVG_CHAR_WIDTH
}

fn slab_label_fits(bounds: LayoutBounds, side: OverflowSide, text: &str) -> bool {
    let text_width = estimated_debug_label_width(text);
    match side {
        OverflowSide::Left | OverflowSide::Right => {
            bounds.width >= DEBUG_OVERLAY_LABEL_FONT_SIZE + DEBUG_OVERLAY_LABEL_PAD
                && bounds.height >= text_width + DEBUG_OVERLAY_LABEL_PAD
        }
        OverflowSide::Top | OverflowSide::Bottom => {
            bounds.width >= text_width + DEBUG_OVERLAY_LABEL_PAD
                && bounds.height >= DEBUG_OVERLAY_LABEL_FONT_SIZE + DEBUG_OVERLAY_LABEL_PAD
        }
    }
}

fn slab_label_placement(
    bounds: LayoutBounds,
    side: OverflowSide,
    text: &str,
) -> Option<OverlayLabelPlacement> {
    if !slab_label_fits(bounds, side, text) {
        return None;
    }

    let placement = match side {
        OverflowSide::Left => OverlayLabelPlacement {
            x: bounds.x + DEBUG_OVERLAY_LABEL_PAD,
            y: bounds.y + bounds.height * 0.25,
            angle: -90.0,
            align: TextAlign::Center,
            baseline: TextBaseline::Middle,
        },
        OverflowSide::Right => OverlayLabelPlacement {
            x: bounds.x + bounds.width - DEBUG_OVERLAY_LABEL_PAD,
            y: bounds.y + bounds.height * 0.75,
            angle: 90.0,
            align: TextAlign::Center,
            baseline: TextBaseline::Middle,
        },
        OverflowSide::Top => OverlayLabelPlacement {
            x: bounds.x + bounds.width / 2.0,
            y: bounds.y + DEBUG_OVERLAY_LABEL_PAD / 2.0,
            angle: 0.0,
            align: TextAlign::Center,
            baseline: TextBaseline::Top,
        },
        OverflowSide::Bottom => OverlayLabelPlacement {
            x: bounds.x + bounds.width * 0.25,
            y: bounds.y + bounds.height - DEBUG_OVERLAY_LABEL_PAD / 2.0,
            angle: 0.0,
            align: TextAlign::Center,
            baseline: TextBaseline::Bottom,
        },
    };
    Some(placement)
}

fn box_label_placement(
    bounds: LayoutBounds,
    layer: LayoutDebugLayer,
    flip_label_align: bool,
) -> OverlayLabelPlacement {
    let (x, align) = match layer {
        LayoutDebugLayer::FrameAllocation => (bounds.x + DEBUG_OVERLAY_LABEL_PAD, TextAlign::Left),
        _ if flip_label_align => (
            bounds.x + bounds.width - DEBUG_OVERLAY_LABEL_PAD,
            TextAlign::Right,
        ),
        _ => (bounds.x + DEBUG_OVERLAY_LABEL_PAD, TextAlign::Left),
    };
    let (y, baseline) = match layer {
        LayoutDebugLayer::ChildFrameAllocation => {
            (bounds.y + bounds.height - 1.0, TextBaseline::Bottom)
        }
        LayoutDebugLayer::FrameAllocation | LayoutDebugLayer::ContentRect => {
            (bounds.y + DEBUG_OVERLAY_LABEL_PAD, TextBaseline::Top)
        }
        LayoutDebugLayer::OwnedSlab | LayoutDebugLayer::ResidualOverflow => {
            unreachable!("slab labels use side-aware placement")
        }
    };
    OverlayLabelPlacement {
        x,
        y,
        angle: 0.0,
        align,
        baseline,
    }
}

fn overlay_label_placement(
    rect: &LayoutDebugRect,
    flip_label_align: bool,
) -> Option<OverlayLabelPlacement> {
    rect.side
        .map(|side| slab_label_placement(rect.bounds, side, &rect.label))
        .unwrap_or_else(|| {
            Some(box_label_placement(
                rect.bounds,
                rect.layer,
                flip_label_align,
            ))
        })
}

fn debug_overlay_label_mark(
    rect: &LayoutDebugRect,
    color: [f32; 4],
    zindex: i32,
    flip_label_align: bool,
) -> Option<SceneMark> {
    let placement = overlay_label_placement(rect, flip_label_align)?;
    Some(SceneMark::Text(Arc::new(SceneTextMark {
        text: rect.label.clone().into(),
        x: placement.x.into(),
        y: placement.y.into(),
        font_size: DEBUG_OVERLAY_LABEL_FONT_SIZE.into(),
        color: ColorOrGradient::Color(color).into(),
        angle: placement.angle.into(),
        align: placement.align.into(),
        baseline: placement.baseline.into(),
        zindex: Some(zindex),
        clip: false,
        ..Default::default()
    })))
}

fn color_with_alpha(mut color: [f32; 4], alpha: f32) -> [f32; 4] {
    color[3] = alpha;
    color
}

fn overlay_layer_color(layer: LayoutDebugLayer, child_color: Option<[f32; 4]>) -> [f32; 4] {
    match layer {
        LayoutDebugLayer::FrameAllocation => [0.08, 0.08, 0.08, 0.65],
        LayoutDebugLayer::ContentRect => [0.0, 0.45, 0.85, 0.7],
        LayoutDebugLayer::OwnedSlab => [0.0, 0.55, 0.32, 0.7],
        LayoutDebugLayer::ResidualOverflow => [0.86, 0.25, 0.05, 0.75],
        LayoutDebugLayer::ChildFrameAllocation => child_color.unwrap_or([0.35, 0.35, 0.35, 0.7]),
    }
}

fn overlay_layer_stroke_width(layer: LayoutDebugLayer, base: f32) -> f32 {
    match layer {
        LayoutDebugLayer::FrameAllocation => base,
        LayoutDebugLayer::ContentRect => base,
        LayoutDebugLayer::OwnedSlab => base * 0.8,
        LayoutDebugLayer::ResidualOverflow => base * 0.8,
        LayoutDebugLayer::ChildFrameAllocation => base,
    }
}

fn overlay_layer_fill_color(layer: LayoutDebugLayer, color: [f32; 4]) -> [f32; 4] {
    match layer {
        LayoutDebugLayer::OwnedSlab | LayoutDebugLayer::ResidualOverflow => {
            color_with_alpha(color, 0.08)
        }
        _ => [0.0, 0.0, 0.0, 0.0],
    }
}

/// Render frame/content allocation and demand overlay marks.
pub fn create_debug_overlay_rects(
    overlay: &FrameDebugOverlay,
    child_color: Option<String>,
    stroke_width: Option<f32>,
    zindex: Option<i32>,
    flip_label_align: bool,
) -> Vec<SceneMark> {
    let child_color = child_color.as_deref().and_then(parse_color_string);
    let stroke = stroke_width.unwrap_or(1.0);
    let z = zindex.unwrap_or(30);
    let mut marks = Vec::with_capacity(overlay.rects.len() * 2);

    for rect in &overlay.rects {
        let color = overlay_layer_color(rect.layer, child_color);
        marks.push(debug_rect_mark(
            rect.bounds,
            color,
            overlay_layer_fill_color(rect.layer, color),
            overlay_layer_stroke_width(rect.layer, stroke),
            z,
        ));
        if let Some(label_mark) = debug_overlay_label_mark(rect, color, z, flip_label_align) {
            marks.push(label_mark);
        }
    }
    marks
}

/// Create debug rectangles to visualize frame layout bounds.
///
/// # Arguments
/// * `layout` - The layout result to visualize
/// * `color` - Optional color string (HSL format like "hsl(15 65% 60%)") for debug marks. Defaults to magenta
/// * `stroke_width` - Optional stroke width. Defaults to 1.0
/// * `zindex` - Optional z-index. Defaults to 20
/// * `flip_label_align` - If true, align labels on the opposite edge of each debug region.
pub fn create_debug_layout_rects(
    layout: &FrameLayout,
    color: Option<String>,
    stroke_width: Option<f32>,
    zindex: Option<i32>,
    flip_label_align: bool,
) -> Vec<SceneMark> {
    let mut debug_marks = Vec::new();

    // Parse color string to RGBA, defaulting to magenta
    let debug_color = if let Some(color_str) = color {
        parse_color_string(&color_str).unwrap_or([1.0, 0.0, 1.0, 0.5])
    } else {
        [1.0, 0.0, 1.0, 0.7] // Default magenta
    };
    let stroke = stroke_width.unwrap_or(1.0);
    let z = zindex.unwrap_or(20);

    // Plot area outline
    let plot_rect = SceneRectMark {
        x: layout.plot_area.x.into(),
        y: layout.plot_area.y.into(),
        width: Some(layout.plot_area.width.into()),
        height: Some(layout.plot_area.height.into()),
        fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(), // Transparent
        stroke: ColorOrGradient::Color(debug_color).into(),
        stroke_width: stroke.into(),
        zindex: Some(z),
        clip: false, // Don't clip debug marks
        ..Default::default()
    };
    debug_marks.push(SceneMark::Rect(plot_rect));

    // Plot area label - align right if flip_label_align is true
    let (label_x, label_align) = if flip_label_align {
        (
            layout.plot_area.x + layout.plot_area.width - 2.0,
            TextAlign::Right,
        )
    } else {
        (layout.plot_area.x + 2.0, TextAlign::Left)
    };

    let plot_label = SceneTextMark {
        text: "plot-area".into(),
        x: label_x.into(),
        y: (layout.plot_area.y + 10.0).into(),
        font_size: 8.0.into(),
        color: ColorOrGradient::Color(debug_color).into(),
        align: label_align.into(),
        zindex: Some(z),
        clip: false, // Don't clip debug marks
        ..Default::default()
    };
    debug_marks.push(SceneMark::Text(Arc::new(plot_label)));

    // Guide overflows - outlines with labels
    for (position, bounds) in &layout.guide_overflows {
        let overflow_rect = SceneRectMark {
            x: bounds.x.into(),
            y: bounds.y.into(),
            width: Some(bounds.width.into()),
            height: Some(bounds.height.into()),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
            stroke: ColorOrGradient::Color(debug_color).into(),
            stroke_width: stroke.into(),
            zindex: Some(z),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Rect(overflow_rect));

        // Add overflow region label - flip alignment if needed
        let (overflow_label, label_x, label_y, angle, align, baseline): (
            &str,
            f32,
            f32,
            f32,
            _,
            _,
        ) = if flip_label_align {
            match position {
                AxisPosition::Left => (
                    "of-left",
                    bounds.x + 2.0,
                    bounds.y + bounds.height - 2.0,
                    -90.0,
                    TextAlign::Left,
                    TextBaseline::Top,
                ),
                AxisPosition::Right => (
                    "of-right",
                    bounds.x + bounds.width - 2.0,
                    bounds.y + bounds.height - 2.0,
                    90.0,
                    TextAlign::Right,
                    TextBaseline::Top,
                ),
                AxisPosition::Top => (
                    "of-top",
                    bounds.x + bounds.width - 2.0, // Right side instead of left
                    bounds.y,
                    0.0,
                    TextAlign::Right, // Flipped from Left
                    TextBaseline::Top,
                ),
                AxisPosition::Bottom => (
                    "of-bottom",
                    bounds.x + bounds.width - 2.0,
                    bounds.y + bounds.height - 2.0,
                    0.0,
                    TextAlign::Right,
                    TextBaseline::Bottom,
                ),
            }
        } else {
            match position {
                AxisPosition::Left => (
                    "of-left",
                    bounds.x + 2.0,
                    bounds.y + 2.0,
                    -90.0,
                    TextAlign::Right,
                    TextBaseline::Top,
                ),
                AxisPosition::Right => (
                    "of-right",
                    bounds.x + bounds.width - 2.0,
                    bounds.y + 2.0,
                    90.0,
                    TextAlign::Left,
                    TextBaseline::Top,
                ),
                AxisPosition::Top => (
                    "of-top",
                    bounds.x + 2.0,
                    bounds.y,
                    0.0,
                    TextAlign::Left,
                    TextBaseline::Top,
                ),
                AxisPosition::Bottom => (
                    "of-bottom",
                    bounds.x + 2.0,
                    bounds.y + bounds.height - 2.0,
                    0.0,
                    TextAlign::Left,
                    TextBaseline::Bottom,
                ),
            }
        };

        let overflow_label_mark = SceneTextMark {
            text: overflow_label.into(),
            x: label_x.into(),
            y: label_y.into(),
            font_size: 8.0.into(),
            color: ColorOrGradient::Color(debug_color).into(),
            angle: angle.into(),
            align: align.into(),
            baseline: baseline.into(),
            zindex: Some(z),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Text(Arc::new(overflow_label_mark)));
    }

    // Legends - outlines
    for (channel, bounds) in &layout.legends {
        let legend_rect = SceneRectMark {
            x: bounds.x.into(),
            y: bounds.y.into(),
            width: Some(bounds.width.into()),
            height: Some(bounds.height.into()),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
            stroke: ColorOrGradient::Color(debug_color).into(),
            stroke_width: stroke.into(),
            zindex: Some(z),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Rect(legend_rect));

        // Add label for legend channel
        let label_text = channel
            .split_once('@')
            .map(|(primary, _)| primary)
            .unwrap_or(channel);
        let label = SceneTextMark {
            text: label_text.to_string().into(),
            x: (bounds.x + 2.0).into(),
            y: (bounds.y + 10.0).into(),
            font_size: 8.0.into(),
            color: ColorOrGradient::Color(debug_color).into(),
            zindex: Some(z),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Text(Arc::new(label)));
    }

    // Title - outline
    if let Some(bounds) = &layout.title {
        let title_rect = SceneRectMark {
            x: bounds.x.into(),
            y: bounds.y.into(),
            width: Some(bounds.width.into()),
            height: Some(bounds.height.into()),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
            stroke: ColorOrGradient::Color(debug_color).into(),
            stroke_width: stroke.into(),
            zindex: Some(z),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Rect(title_rect));

        // Add title label - right aligned to avoid overlapping with text
        let title_label = SceneTextMark {
            text: "title".into(),
            x: (bounds.x + bounds.width - 5.0).into(),
            y: (bounds.y + 10.0).into(),
            font_size: 8.0.into(),
            color: ColorOrGradient::Color(debug_color).into(),
            align: TextAlign::Right.into(),
            zindex: Some(z),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Text(Arc::new(title_label)));
    }

    // Subtitle - outline
    if let Some(bounds) = &layout.subtitle {
        let subtitle_rect = SceneRectMark {
            x: bounds.x.into(),
            y: bounds.y.into(),
            width: Some(bounds.width.into()),
            height: Some(bounds.height.into()),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
            stroke: ColorOrGradient::Color(debug_color).into(),
            stroke_width: stroke.into(),
            zindex: Some(z),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Rect(subtitle_rect));

        // Add subtitle label - right aligned to avoid overlapping with text
        let subtitle_label = SceneTextMark {
            text: "subtitle".into(),
            x: (bounds.x + bounds.width - 5.0).into(),
            y: (bounds.y + 10.0).into(),
            font_size: 8.0.into(),
            color: ColorOrGradient::Color(debug_color).into(),
            align: TextAlign::Right.into(),
            zindex: Some(z),
            clip: false, // Don't clip debug marks
            ..Default::default()
        };
        debug_marks.push(SceneMark::Text(Arc::new(subtitle_label)));
    }

    debug_marks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{
        ContentAllocation, FrameAllocation, FrameDemand, FrameDimensionSizing, FrameSizingPolicy,
    };

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> LayoutBounds {
        LayoutBounds {
            x,
            y,
            width,
            height,
        }
    }

    fn sizing() -> FrameSizingPolicy {
        FrameSizingPolicy {
            width: FrameDimensionSizing::CanvasConstrained { canvas_size: 200.0 },
            height: FrameDimensionSizing::CanvasConstrained { canvas_size: 160.0 },
        }
    }

    fn frame_allocation(owned_slabs: EdgeSlabs) -> FrameAllocation {
        FrameAllocation {
            rect: bounds(0.0, 0.0, 200.0, 160.0),
            sizing: sizing(),
            owned_slabs,
        }
    }

    fn content_layout(
        owned_slabs: EdgeSlabs,
        frame_demand: FrameDemand,
        child_rects: Vec<LayoutBounds>,
    ) -> ContentLayout {
        let frame = frame_allocation(owned_slabs);
        let child_frame_allocations = child_rects
            .into_iter()
            .map(|rect| FrameAllocation {
                rect,
                sizing: sizing(),
                owned_slabs: EdgeSlabs::default(),
            })
            .collect();
        ContentLayout::new(
            ContentAllocation::new(frame, bounds(20.0, 30.0, 120.0, 80.0)),
            frame_demand,
            child_frame_allocations,
        )
    }

    fn rect<'a>(overlay: &'a FrameDebugOverlay, label: &str) -> &'a LayoutDebugRect {
        overlay
            .rects
            .iter()
            .find(|rect| rect.label == label)
            .unwrap_or_else(|| panic!("missing debug rect {label}"))
    }

    fn label_rect(
        bounds: LayoutBounds,
        label: &str,
        layer: LayoutDebugLayer,
        side: Option<OverflowSide>,
    ) -> LayoutDebugRect {
        LayoutDebugRect {
            bounds,
            label: label.to_string(),
            layer,
            side,
        }
    }

    #[test]
    fn overlay_model_omits_child_allocations_for_single_plot() {
        let layout = content_layout(EdgeSlabs::default(), FrameDemand::default(), Vec::new());

        let overlay = FrameDebugOverlay::from_content_layout(&layout, [0.0, 0.0]);

        assert!(
            overlay
                .rects
                .iter()
                .all(|rect| rect.layer != LayoutDebugLayer::ChildFrameAllocation)
        );
    }

    #[test]
    fn overlay_model_includes_frame_content_and_child_allocations() {
        let layout = content_layout(
            EdgeSlabs::default(),
            FrameDemand::default(),
            vec![bounds(25.0, 35.0, 40.0, 30.0)],
        );

        let overlay = FrameDebugOverlay::from_content_layout(&layout, [0.0, 0.0]);

        assert_eq!(
            rect(&overlay, "frame").layer,
            LayoutDebugLayer::FrameAllocation
        );
        assert_eq!(
            rect(&overlay, "content").layer,
            LayoutDebugLayer::ContentRect
        );
        assert_eq!(
            rect(&overlay, "child-0").layer,
            LayoutDebugLayer::ChildFrameAllocation
        );
        assert_eq!(
            rect(&overlay, "child-0").bounds,
            bounds(25.0, 35.0, 40.0, 30.0)
        );
    }

    #[test]
    fn overlay_model_converts_owned_slabs_to_rects() {
        let layout = content_layout(
            EdgeSlabs::new(10.0, 20.0, 30.0, 40.0),
            FrameDemand::default(),
            Vec::new(),
        );

        let overlay = FrameDebugOverlay::from_content_layout(&layout, [0.0, 0.0]);

        assert_eq!(
            rect(&overlay, "owned-top").bounds,
            bounds(20.0, 20.0, 120.0, 10.0)
        );
        assert_eq!(
            rect(&overlay, "owned-right").bounds,
            bounds(140.0, 30.0, 20.0, 80.0)
        );
        assert_eq!(
            rect(&overlay, "owned-bottom").bounds,
            bounds(20.0, 110.0, 120.0, 30.0)
        );
        assert_eq!(
            rect(&overlay, "owned-left").bounds,
            bounds(-20.0, 30.0, 40.0, 80.0)
        );
    }

    #[test]
    fn overlay_model_converts_residual_overflow_to_rects() {
        let frame_demand = FrameDemand {
            rendered_envelope: EdgeSlabs::new(15.0, 35.0, 30.0, 12.0),
            ..Default::default()
        };
        let layout = content_layout(
            EdgeSlabs::new(10.0, 20.0, 50.0, 5.0),
            frame_demand,
            Vec::new(),
        );

        let overlay = FrameDebugOverlay::from_content_layout(&layout, [0.0, 0.0]);

        assert_eq!(
            rect(&overlay, "resid-top").bounds,
            bounds(20.0, 25.0, 120.0, 5.0)
        );
        assert_eq!(
            rect(&overlay, "resid-right").bounds,
            bounds(140.0, 30.0, 15.0, 80.0)
        );
        assert_eq!(
            rect(&overlay, "resid-right").side,
            Some(OverflowSide::Right)
        );
        assert_eq!(
            rect(&overlay, "resid-left").bounds,
            bounds(13.0, 30.0, 7.0, 80.0)
        );
        assert!(
            overlay
                .rects
                .iter()
                .all(|rect| rect.label != "resid-bottom")
        );
    }

    #[test]
    fn overlay_model_applies_origin_translation() {
        let layout = content_layout(
            EdgeSlabs::default(),
            FrameDemand::default(),
            vec![bounds(25.0, 35.0, 40.0, 30.0)],
        );

        let overlay = FrameDebugOverlay::from_content_layout(&layout, [-20.0, -30.0]);

        assert_eq!(
            rect(&overlay, "content").bounds,
            bounds(0.0, 0.0, 120.0, 80.0)
        );
        assert_eq!(
            rect(&overlay, "child-0").bounds,
            bounds(5.0, 5.0, 40.0, 30.0)
        );
    }

    #[test]
    fn overlay_label_places_vertical_slab_labels_at_outer_quarters() {
        let rect = label_rect(
            bounds(100.0, 20.0, 18.0, 120.0),
            "resid-right",
            LayoutDebugLayer::ResidualOverflow,
            Some(OverflowSide::Right),
        );

        let placement = overlay_label_placement(&rect, false).expect("label should fit");

        assert_eq!(
            placement,
            OverlayLabelPlacement {
                x: 115.0,
                y: 110.0,
                angle: 90.0,
                align: TextAlign::Center,
                baseline: TextBaseline::Middle,
            }
        );

        let rect = label_rect(
            bounds(100.0, 20.0, 18.0, 120.0),
            "resid-left",
            LayoutDebugLayer::ResidualOverflow,
            Some(OverflowSide::Left),
        );

        let placement = overlay_label_placement(&rect, false).expect("label should fit");

        assert_eq!(
            placement,
            OverlayLabelPlacement {
                x: 103.0,
                y: 50.0,
                angle: -90.0,
                align: TextAlign::Center,
                baseline: TextBaseline::Middle,
            }
        );
    }

    #[test]
    fn overlay_label_suppresses_tiny_slab_labels() {
        let rect = label_rect(
            bounds(100.0, 20.0, 5.0, 120.0),
            "resid-right",
            LayoutDebugLayer::ResidualOverflow,
            Some(OverflowSide::Right),
        );

        assert_eq!(overlay_label_placement(&rect, false), None);
    }

    #[test]
    fn overlay_label_places_bottom_slab_labels_at_first_quarter() {
        let rect = label_rect(
            bounds(20.0, 100.0, 120.0, 20.0),
            "resid-bottom",
            LayoutDebugLayer::ResidualOverflow,
            Some(OverflowSide::Bottom),
        );

        let placement = overlay_label_placement(&rect, false).expect("label should fit");

        assert_eq!(
            placement,
            OverlayLabelPlacement {
                x: 50.0,
                y: 118.5,
                angle: 0.0,
                align: TextAlign::Center,
                baseline: TextBaseline::Bottom,
            }
        );
    }

    #[test]
    fn overlay_label_places_child_labels_on_bottom_edge() {
        let rect = label_rect(
            bounds(10.0, 20.0, 80.0, 40.0),
            "child-0",
            LayoutDebugLayer::ChildFrameAllocation,
            None,
        );

        let placement = overlay_label_placement(&rect, false).expect("label should be present");

        assert_eq!(
            placement,
            OverlayLabelPlacement {
                x: 13.0,
                y: 59.0,
                angle: 0.0,
                align: TextAlign::Left,
                baseline: TextBaseline::Bottom,
            }
        );
    }

    #[test]
    fn overlay_label_keeps_frame_labels_on_left_when_flipped() {
        let rect = label_rect(
            bounds(10.0, 20.0, 80.0, 40.0),
            "frame",
            LayoutDebugLayer::FrameAllocation,
            None,
        );

        let placement = overlay_label_placement(&rect, true).expect("label should be present");

        assert_eq!(
            placement,
            OverlayLabelPlacement {
                x: 13.0,
                y: 23.0,
                angle: 0.0,
                align: TextAlign::Left,
                baseline: TextBaseline::Top,
            }
        );
    }
}
