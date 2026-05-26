//! Frame layout solver boundary.
//!
//! A frame solver lays out chart chrome around a single content rectangle:
//! margins, titles, guide overflow bands, legends, and plot/content bounds.
//! The first implementation delegates to the existing Taffy-backed
//! `ChartLayout` so callers can move to neutral frame terminology before the
//! solver implementation changes.

use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;

use avenger_chart_core::{AxisPosition, FrameLayout, LayoutBounds, LegendPosition};

use crate::{
    error::AvengerChartError,
    guide::OverflowSpaceRequirement,
    plot::{PlotSubtitle, PlotTitle},
    render::{EvaluationContext, LayoutSolution, LegendMeasurements},
    theme::Theme,
};

use super::{chart_layout::ChartLayout, sizing::EvaluatedLayoutSpec};

/// Input required to solve a chart frame layout.
pub(crate) struct FrameLayoutInput<'a> {
    pub(crate) overflow: &'a OverflowSpaceRequirement,
    pub(crate) layout_spec: &'a EvaluatedLayoutSpec,
    pub(crate) title: Option<&'a PlotTitle>,
    pub(crate) subtitle: Option<&'a PlotSubtitle>,
    pub(crate) theme: &'a Theme,
    pub(crate) legend_measurements: &'a LegendMeasurements,
    pub(crate) ctx: &'a SessionContext,
    pub(crate) params: &'a IndexMap<String, ScalarValue>,
    pub(crate) eval_ctx: Option<&'a EvaluationContext>,
}

/// Taffy-backed frame layout solver.
pub(crate) struct TaffyFrameLayoutSolver;

impl TaffyFrameLayoutSolver {
    pub(crate) async fn solve(
        input: FrameLayoutInput<'_>,
    ) -> Result<LayoutSolution, AvengerChartError> {
        let mut layout = ChartLayout::new(
            input.overflow,
            input.layout_spec,
            input.title,
            input.subtitle,
            input.theme,
            input.legend_measurements,
            input.ctx,
            input.params,
            input.eval_ctx,
        )
        .await?;
        layout.compute(input.layout_spec)
    }
}

#[derive(Clone, Copy)]
enum LegendMainAxis {
    Horizontal,
    Vertical,
}

fn legend_main_axis_span(bounds: &LayoutBounds, axis: LegendMainAxis) -> f32 {
    match axis {
        LegendMainAxis::Horizontal => bounds.width,
        LegendMainAxis::Vertical => bounds.height,
    }
}

fn set_legend_main_axis_bounds(
    bounds: &mut LayoutBounds,
    axis: LegendMainAxis,
    start: f32,
    span: f32,
) {
    match axis {
        LegendMainAxis::Horizontal => {
            bounds.x = start;
            bounds.width = span;
        }
        LegendMainAxis::Vertical => {
            bounds.y = start;
            bounds.height = span;
        }
    }
}

fn retarget_flexible_legend_group_main_axis(
    legends: &mut indexmap::IndexMap<String, LayoutBounds>,
    legend_measurements: &LegendMeasurements,
    legend_keys: &[String],
    axis: LegendMainAxis,
    origin: f32,
    new_span: f32,
) {
    let mut flexible_count = 0usize;
    let mut old_flexible_total = 0.0f32;
    let mut fixed_total = 0.0f32;

    for key in legend_keys {
        let Some(bounds) = legends.get(key) else {
            continue;
        };
        let span = legend_main_axis_span(bounds, axis).max(0.0);
        let is_flexible = legend_measurements
            .get(key)
            .map(|measurement| measurement.flexible)
            .unwrap_or(false);
        if is_flexible {
            flexible_count += 1;
            old_flexible_total += span;
        } else {
            fixed_total += span;
        }
    }

    if flexible_count == 0 {
        return;
    }

    let flexible_available = (new_span - fixed_total).max(0.0);
    let mut cursor = origin;
    for key in legend_keys {
        let Some(bounds) = legends.get_mut(key) else {
            continue;
        };
        let old_span = legend_main_axis_span(bounds, axis).max(0.0);
        let is_flexible = legend_measurements
            .get(key)
            .map(|measurement| measurement.flexible)
            .unwrap_or(false);
        let span = if is_flexible {
            if old_flexible_total > 0.01 {
                flexible_available * old_span / old_flexible_total
            } else {
                flexible_available / flexible_count as f32
            }
        } else {
            old_span
        };
        set_legend_main_axis_bounds(bounds, axis, cursor, span);
        cursor += span;
    }
}

pub(crate) fn overflow_side_value(overflow: &OverflowSpaceRequirement, side: AxisPosition) -> f32 {
    match side {
        AxisPosition::Top => overflow.top,
        AxisPosition::Right => overflow.right,
        AxisPosition::Bottom => overflow.bottom,
        AxisPosition::Left => overflow.left,
    }
}

fn set_overflow_side_value(
    overflow: &mut OverflowSpaceRequirement,
    side: AxisPosition,
    value: f32,
) {
    let value = value.max(0.0);
    match side {
        AxisPosition::Top => overflow.top = value,
        AxisPosition::Right => overflow.right = value,
        AxisPosition::Bottom => overflow.bottom = value,
        AxisPosition::Left => overflow.left = value,
    }
}

fn legend_position_for_axis_side(side: AxisPosition) -> LegendPosition {
    match side {
        AxisPosition::Top => LegendPosition::Top,
        AxisPosition::Right => LegendPosition::Right,
        AxisPosition::Bottom => LegendPosition::Bottom,
        AxisPosition::Left => LegendPosition::Left,
    }
}

fn legend_cross_axis_extent(layout: &FrameLayout, side: AxisPosition) -> f32 {
    let position = legend_position_for_axis_side(side);
    let Some(keys) = layout.legends_by_position.get(&position) else {
        return 0.0;
    };

    keys.iter()
        .filter_map(|key| layout.legends.get(key))
        .map(|bounds| match side {
            AxisPosition::Left | AxisPosition::Right => bounds.width,
            AxisPosition::Top | AxisPosition::Bottom => bounds.height,
        })
        .fold(0.0, f32::max)
}

fn translate_frame_layout(layout: &mut FrameLayout, dx: f32, dy: f32) {
    if dx.abs() <= 0.01 && dy.abs() <= 0.01 {
        return;
    }

    layout.plot_area.x += dx;
    layout.plot_area.y += dy;

    for bounds in layout.guide_overflows.values_mut() {
        bounds.x += dx;
        bounds.y += dy;
    }
    for bounds in layout.legends.values_mut() {
        bounds.x += dx;
        bounds.y += dy;
    }
    if let Some(bounds) = &mut layout.title {
        bounds.x += dx;
        bounds.y += dy;
    }
    if let Some(bounds) = &mut layout.subtitle {
        bounds.x += dx;
        bounds.y += dy;
    }
}

fn guide_overflow_bounds(plot_area: LayoutBounds, side: AxisPosition, guide: f32) -> LayoutBounds {
    let guide = guide.max(0.0);
    match side {
        AxisPosition::Top => LayoutBounds {
            x: plot_area.x,
            y: plot_area.y - guide,
            width: plot_area.width,
            height: guide,
        },
        AxisPosition::Right => LayoutBounds {
            x: plot_area.x + plot_area.width,
            y: plot_area.y,
            width: guide,
            height: plot_area.height,
        },
        AxisPosition::Bottom => LayoutBounds {
            x: plot_area.x,
            y: plot_area.y + plot_area.height,
            width: plot_area.width,
            height: guide,
        },
        AxisPosition::Left => LayoutBounds {
            x: plot_area.x - guide,
            y: plot_area.y,
            width: guide,
            height: plot_area.height,
        },
    }
}

fn set_guide_overflow_geometry(layout: &mut FrameLayout, side: AxisPosition, guide: f32) {
    if guide <= 0.01 {
        layout.guide_overflows.remove(&side);
        return;
    }

    layout
        .guide_overflows
        .insert(side, guide_overflow_bounds(layout.plot_area, side, guide));
}

fn anchor_legends_for_side(layout: &mut FrameLayout, side: AxisPosition, guide: f32) {
    let position = legend_position_for_axis_side(side);
    let Some(keys) = layout.legends_by_position.get(&position).cloned() else {
        return;
    };

    for key in keys {
        let Some(bounds) = layout.legends.get_mut(&key) else {
            continue;
        };
        match side {
            AxisPosition::Top => {
                bounds.y = layout.plot_area.y - guide.max(0.0) - bounds.height;
            }
            AxisPosition::Right => {
                bounds.x = layout.plot_area.x + layout.plot_area.width + guide.max(0.0);
            }
            AxisPosition::Bottom => {
                bounds.y = layout.plot_area.y + layout.plot_area.height + guide.max(0.0);
            }
            AxisPosition::Left => {
                bounds.x = layout.plot_area.x - guide.max(0.0) - bounds.width;
            }
        }
    }
}

/// Apply a known side slab to a realized frame without remeasuring guides or legends.
pub(crate) fn apply_frame_side_slab(
    layout: &mut LayoutSolution,
    side: AxisPosition,
    guide: f32,
    total: f32,
) {
    let guide = guide
        .max(overflow_side_value(&layout.overflow, side))
        .max(0.0);
    let total = total
        .max(guide)
        .max(guide + legend_cross_axis_extent(&layout.frame_layout, side));
    let old_total = overflow_side_value(&layout.total_overflow, side)
        .max(overflow_side_value(&layout.overflow, side));
    let delta = total - old_total;

    match side {
        AxisPosition::Left => {
            translate_frame_layout(&mut layout.frame_layout, delta, 0.0);
            layout.canvas_size.0 = (layout.canvas_size.0 + delta).max(1.0);
        }
        AxisPosition::Top => {
            translate_frame_layout(&mut layout.frame_layout, 0.0, delta);
            layout.canvas_size.1 = (layout.canvas_size.1 + delta).max(1.0);
        }
        AxisPosition::Right => {
            layout.canvas_size.0 = (layout.canvas_size.0 + delta).max(1.0);
        }
        AxisPosition::Bottom => {
            layout.canvas_size.1 = (layout.canvas_size.1 + delta).max(1.0);
        }
    }

    set_overflow_side_value(&mut layout.overflow, side, guide);
    set_overflow_side_value(&mut layout.total_overflow, side, total);
    set_guide_overflow_geometry(&mut layout.frame_layout, side, guide);
    anchor_legends_for_side(&mut layout.frame_layout, side, guide);
}

/// Retarget a realized frame to a new plot-area size without remeasuring.
pub(crate) fn retarget_frame_layout_for_plot_area(
    layout: &mut LayoutSolution,
    legend_measurements: &LegendMeasurements,
    new_plot_area_width: f32,
    new_plot_area_height: f32,
) {
    layout.frame_layout.plot_area.width = new_plot_area_width;
    layout.frame_layout.plot_area.height = new_plot_area_height;

    for (position, legend_keys) in layout.frame_layout.legends_by_position.clone() {
        match position {
            LegendPosition::Left | LegendPosition::Right => {
                retarget_flexible_legend_group_main_axis(
                    &mut layout.frame_layout.legends,
                    legend_measurements,
                    &legend_keys,
                    LegendMainAxis::Vertical,
                    layout.frame_layout.plot_area.y,
                    new_plot_area_height,
                );
            }
            LegendPosition::Top | LegendPosition::Bottom => {
                retarget_flexible_legend_group_main_axis(
                    &mut layout.frame_layout.legends,
                    legend_measurements,
                    &legend_keys,
                    LegendMainAxis::Horizontal,
                    layout.frame_layout.plot_area.x,
                    new_plot_area_width,
                );
            }
        }
    }

    for side in [
        AxisPosition::Top,
        AxisPosition::Right,
        AxisPosition::Bottom,
        AxisPosition::Left,
    ] {
        let guide = overflow_side_value(&layout.overflow, side);
        set_guide_overflow_geometry(&mut layout.frame_layout, side, guide);
        anchor_legends_for_side(&mut layout.frame_layout, side, guide);
    }
}
