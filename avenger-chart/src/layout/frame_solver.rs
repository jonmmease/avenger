//! Frame layout solver boundary.
//!
//! A frame solver lays out chart chrome around a single content rectangle:
//! margins, titles, guide overflow bands, legends, and plot/content bounds.

use std::collections::HashMap;

use datafusion::{common::ScalarValue, prelude::SessionContext};
use datafusion_proto::protobuf;
use indexmap::IndexMap;
use tracing::debug;

use avenger_chart_core::{
    AxisPosition, FrameLayout, LayoutBounds, LegendPosition, Size2D, TitleSpan,
    evaluate_string_expr, maybe::Maybe,
};

use crate::{
    error::AvengerChartError,
    guide::OverflowSpaceRequirement,
    plot::{PlotSubtitle, PlotTitle},
    render::{EvaluationContext, LayoutSolution, LegendMeasurements},
    serialization::LogicalExprNodeExt,
    theme::Theme,
};

use super::declared_frame::{DeclaredAxisSizing as FrameAxisSizing, SolvedFrameView};

use super::{
    chrome::{FrameChrome, FrameChromeBuilder, MIN_COMPONENT_SIZE},
    info::LegendLayoutInfo,
    sizing::EvaluatedLayoutSpec,
};

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

/// Avenger-native frame layout solver.
pub(crate) struct AvengerFrameLayoutSolver;

impl AvengerFrameLayoutSolver {
    pub(crate) async fn solve(
        input: FrameLayoutInput<'_>,
    ) -> Result<LayoutSolution, AvengerChartError> {
        let mut builder = FrameChromeBuilder::new();
        if input.title.is_some() {
            builder.add_title();
        }
        if input.subtitle.is_some() {
            builder.add_subtitle();
        }
        for (channel, measurement) in input.legend_measurements.iter() {
            builder.add_legend(channel.clone(), measurement.position);
        }

        let legend_sizes: HashMap<String, Size2D> = input
            .legend_measurements
            .iter()
            .map(|(key, measurement)| (key.clone(), measurement.size))
            .collect();

        let title_span = if let Some(title) = input.title {
            let title_ctx = input.theme.title_context_with_params(input.params.clone());
            evaluate_title_span(
                &title.span,
                &title_ctx,
                input.theme,
                input.ctx,
                input.params,
            )
            .await?
        } else {
            TitleSpan::default()
        };
        let subtitle_span = if let Some(subtitle) = input.subtitle {
            let subtitle_ctx = input
                .theme
                .subtitle_context_with_params(input.params.clone());
            evaluate_title_span(
                &subtitle.span,
                &subtitle_ctx,
                input.theme,
                input.ctx,
                input.params,
            )
            .await?
        } else {
            TitleSpan::default()
        };

        let (sizing_horizontal, sizing_vertical) = input.layout_spec.frame_axis_sizings();
        let chrome = builder
            .build_frame_chrome(
                input.overflow,
                sizing_horizontal,
                sizing_vertical,
                title_span,
                subtitle_span,
                input.title,
                input.subtitle,
                input.theme,
                input.layout_spec,
                &legend_sizes,
                input.ctx,
                input.params,
                input.eval_ctx,
            )
            .await?;

        solve_native_frame_layout(
            chrome,
            &builder.legends_by_position,
            input.legend_measurements,
        )
    }
}

fn solve_native_frame_layout(
    chrome: FrameChrome,
    legends_by_position: &IndexMap<LegendPosition, Vec<String>>,
    legend_measurements: &LegendMeasurements,
) -> Result<LayoutSolution, AvengerChartError> {
    let (frame_layout, legend_info, solution) =
        project_layout_rects(&chrome, legends_by_position, legend_measurements);

    let mut guide_overflow = OverflowSpaceRequirement::default();
    for (side, bounds) in &frame_layout.guide_overflows {
        match side {
            AxisPosition::Left => guide_overflow.left = guide_overflow.left.max(bounds.width),
            AxisPosition::Right => guide_overflow.right = guide_overflow.right.max(bounds.width),
            AxisPosition::Top => guide_overflow.top = guide_overflow.top.max(bounds.height),
            AxisPosition::Bottom => {
                guide_overflow.bottom = guide_overflow.bottom.max(bounds.height)
            }
        }
    }

    Ok(LayoutSolution {
        frame_layout,
        chrome,
        canvas_size: (
            solution.horizontal.extent.round().max(0.0),
            solution.vertical.extent.round().max(0.0),
        ),
        overflow: guide_overflow.clone(),
        total_overflow: guide_overflow,
        legend_info,
    })
}

/// Project every frame component rect from the declared chrome: solve the
/// frame, then derive the plot area, guide overflow, title band, and legend
/// rects with the standard rounding policies. Used by the initial solve and
/// by coordination retargets (which re-solve with updated chrome instead of
/// mutating realized rects).
fn project_layout_rects(
    chrome: &FrameChrome,
    legends_by_position: &IndexMap<LegendPosition, Vec<String>>,
    legend_measurements: &LegendMeasurements,
) -> (FrameLayout, LegendLayoutInfo, SolvedFrameView) {
    let solution = chrome.frame.solve();
    let h = &solution.horizontal;
    let v = &solution.vertical;

    let mut frame_layout = FrameLayout {
        plot_area: snap_rect_edges(LayoutBounds {
            x: h.content.start,
            y: v.content.start,
            width: h.content.size,
            height: v.content.size,
        }),
        guide_overflows: HashMap::new(),
        legends: IndexMap::new(),
        legends_by_position: legends_by_position.clone(),
        title: None,
        subtitle: None,
    };
    debug!(
        x = frame_layout.plot_area.x,
        y = frame_layout.plot_area.y,
        width = frame_layout.plot_area.width,
        height = frame_layout.plot_area.height,
        "Outer plot-area"
    );

    let guide_layers = [
        (
            AxisPosition::Top,
            chrome.guide_overflow.top,
            LayoutBounds {
                x: h.content.start,
                y: v.leading.inner.start,
                width: h.content.size,
                height: v.leading.inner.size,
            },
        ),
        (
            AxisPosition::Right,
            chrome.guide_overflow.right,
            LayoutBounds {
                x: h.trailing.inner.start,
                y: v.content.start,
                width: h.trailing.inner.size,
                height: v.content.size,
            },
        ),
        (
            AxisPosition::Bottom,
            chrome.guide_overflow.bottom,
            LayoutBounds {
                x: h.content.start,
                y: v.trailing.inner.start,
                width: h.content.size,
                height: v.trailing.inner.size,
            },
        ),
        (
            AxisPosition::Left,
            chrome.guide_overflow.left,
            LayoutBounds {
                x: h.leading.inner.start,
                y: v.content.start,
                width: h.leading.inner.size,
                height: v.content.size,
            },
        ),
    ];
    for (side, present, bounds) in guide_layers {
        if !present {
            continue;
        }
        let bounds = snap_rect_edges(bounds);
        debug!(
            position = ?side,
            x = bounds.x,
            y = bounds.y,
            width = bounds.width,
            height = bounds.height,
            rel_x = bounds.x - frame_layout.plot_area.x,
            rel_y = bounds.y - frame_layout.plot_area.y,
            "Outer guide overflow bounds"
        );
        frame_layout.guide_overflows.insert(side, bounds);
    }

    let has_right_legend = legends_by_position.contains_key(&LegendPosition::Right);
    let mut band_index = 0;
    if chrome.has_title_band {
        frame_layout.title = Some(title_band_bounds(
            &solution,
            band_index,
            chrome.title_span,
            chrome.guide_overflow.left,
            chrome.guide_overflow.right,
            has_right_legend,
        ));
        band_index += 1;
    }
    if chrome.has_subtitle_band {
        frame_layout.subtitle = Some(title_band_bounds(
            &solution,
            band_index,
            chrome.subtitle_span,
            chrome.guide_overflow.left,
            chrome.guide_overflow.right,
            has_right_legend,
        ));
    }

    for (position, legend_keys) in legends_by_position {
        let container = match position {
            LegendPosition::Left => LayoutBounds {
                x: h.leading.outer.start,
                y: v.content.start,
                width: h.leading.outer.size,
                height: v.content.size,
            },
            LegendPosition::Right => LayoutBounds {
                x: h.trailing.outer.start,
                y: v.content.start,
                width: h.trailing.outer.size,
                height: v.content.size,
            },
            LegendPosition::Top => LayoutBounds {
                x: h.content.start,
                y: v.leading.outer.start,
                width: h.content.size,
                height: v.leading.outer.size,
            },
            LegendPosition::Bottom => LayoutBounds {
                x: h.content.start,
                y: v.trailing.outer.start,
                width: h.content.size,
                height: v.trailing.outer.size,
            },
        };
        for (key, bounds) in
            layout_legend_group(*position, container, legend_keys, legend_measurements)
        {
            let bounds = snap_rect_edges(bounds);
            debug!(
                channel = key,
                x = bounds.x,
                y = bounds.y,
                width = bounds.width,
                height = bounds.height,
                "Individual legend bounds"
            );
            frame_layout.legends.insert(key, bounds);
        }
    }

    let mut legend_info = LegendLayoutInfo::default();
    for (channel, bounds) in &frame_layout.legends {
        for (position, legend_keys) in legends_by_position {
            if !legend_keys.contains(channel) {
                continue;
            }
            match position {
                LegendPosition::Right => legend_info.right_x = legend_info.right_x.max(bounds.x),
                LegendPosition::Left => {
                    if legend_info.left_x == 0.0 || bounds.x < legend_info.left_x {
                        legend_info.left_x = bounds.x;
                    }
                }
                LegendPosition::Top => {
                    if legend_info.top_y == 0.0 || bounds.y < legend_info.top_y {
                        legend_info.top_y = bounds.y;
                    }
                }
                LegendPosition::Bottom => {
                    legend_info.bottom_y = legend_info.bottom_y.max(bounds.y);
                }
            }
        }
    }

    (frame_layout, legend_info, solution)
}

/// Re-project all frame component rects of a realized layout from its
/// retained chrome, leaving the slab state (`overflow`, `total_overflow`)
/// and `canvas_size` untouched — those are owned by the caller's slab
/// algebra and sizing policy.
fn reproject_layout_rects(layout: &mut LayoutSolution, legend_measurements: &LegendMeasurements) {
    let legends_by_position = layout.frame_layout.legends_by_position.clone();
    let (frame_layout, legend_info, _) =
        project_layout_rects(&layout.chrome, &legends_by_position, legend_measurements);
    layout.frame_layout = frame_layout;
    layout.legend_info = legend_info;
}

/// Compute the rect of one title/subtitle band from the solved frame.
///
/// The band's vertical strip is `vertical.leading.bands[band_index]`. The
/// horizontal span follows the span policy: the plot content alone, or the
/// content plus the existing guide-overflow layers and a right legend
/// container (the canvas span runs from the left overflow layer through the
/// rightmost chrome component).
fn title_band_bounds(
    solution: &SolvedFrameView,
    band_index: usize,
    span: TitleSpan,
    has_left_overflow: bool,
    has_right_overflow: bool,
    has_right_legend: bool,
) -> LayoutBounds {
    let h = &solution.horizontal;
    let band = solution.vertical.leading.bands[band_index];
    let (x, width) = match span {
        TitleSpan::PlotArea => (h.content.start, h.content.size),
        TitleSpan::Canvas => {
            let x = if has_left_overflow {
                h.leading.inner.start
            } else {
                h.content.start
            };
            let mut width = 0.0f32;
            if has_left_overflow {
                width += h.leading.inner.size;
            }
            width += h.content.size;
            if has_right_overflow {
                width += h.trailing.inner.size;
            }
            if has_right_legend {
                width += h.trailing.outer.size;
            }
            (x, width)
        }
    };
    snap_rect_edges(LayoutBounds {
        x,
        y: band.start,
        width,
        height: band.size,
    })
}

async fn evaluate_title_span(
    span_field: &Maybe<Option<protobuf::LogicalExprNode>>,
    theme_context: &crate::theme::ThemeContext,
    theme: &Theme,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<TitleSpan, AvengerChartError> {
    match span_field {
        Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            let span_str = evaluate_string_expr(&expr, ctx, params).await?;
            Ok(match span_str.as_str() {
                "canvas" => TitleSpan::Canvas,
                "plot_area" | "plot-area" => TitleSpan::PlotArea,
                _ => TitleSpan::default(),
            })
        }
        _ => Ok(theme
            .query(theme_context, "width")
            .and_then(|value| value.as_string().map(|s| s.to_string()))
            .and_then(|s| match s.as_str() {
                "canvas" => Some(TitleSpan::Canvas),
                "plot-area" | "plot_area" => Some(TitleSpan::PlotArea),
                _ => None,
            })
            .unwrap_or_default()),
    }
}

/// The one pixel-snapping rule for frame component rects: round the edges
/// (`x` and `x + width`), never the sizes. Edge snapping keeps adjacent
/// rects adjacent and keeps spans from drifting; a size-rounded rect can
/// end up one pixel wider than the slab it sits in. Chrome slab inputs are
/// pixel-aligned by ceiling at measurement; the canvas extent rounds.
fn snap_rect_edges(bounds: LayoutBounds) -> LayoutBounds {
    let x = bounds.x.round();
    let y = bounds.y.round();
    let x2 = (bounds.x + bounds.width).round();
    let y2 = (bounds.y + bounds.height).round();
    LayoutBounds {
        x,
        y,
        width: (x2 - x).max(0.0),
        height: (y2 - y).max(0.0),
    }
}

fn layout_legend_group(
    position: LegendPosition,
    container: LayoutBounds,
    legend_keys: &[String],
    legend_measurements: &LegendMeasurements,
) -> IndexMap<String, LayoutBounds> {
    let axis = match position {
        LegendPosition::Top | LegendPosition::Bottom => LegendMainAxis::Horizontal,
        LegendPosition::Left | LegendPosition::Right => LegendMainAxis::Vertical,
    };
    let container_main = legend_main_axis_span(&container, axis).max(0.0);
    let mut fixed_total = 0.0f32;
    let mut flexible_count = 0usize;

    for key in legend_keys {
        let Some(measurement) = legend_measurements.get(key) else {
            continue;
        };
        if measurement.flexible {
            flexible_count += 1;
        } else {
            fixed_total += match axis {
                LegendMainAxis::Horizontal => measurement.size.width,
                LegendMainAxis::Vertical => measurement.size.height,
            }
            .max(0.0);
        }
    }

    let flexible_span = flexible_legend_span(container_main, fixed_total, flexible_count);
    let mut cursor = match axis {
        LegendMainAxis::Horizontal => container.x,
        LegendMainAxis::Vertical => container.y,
    };
    let mut result = IndexMap::new();

    for key in legend_keys {
        let Some(measurement) = legend_measurements.get(key) else {
            continue;
        };
        let mut bounds = match position {
            LegendPosition::Top | LegendPosition::Bottom => LayoutBounds {
                x: cursor,
                y: container.y,
                width: if measurement.flexible {
                    flexible_span
                } else {
                    measurement.size.width
                },
                height: measurement.size.height,
            },
            LegendPosition::Left | LegendPosition::Right => LayoutBounds {
                x: container.x,
                y: cursor,
                width: measurement.size.width,
                height: if measurement.flexible {
                    flexible_span
                } else {
                    measurement.size.height
                },
            },
        };
        bounds.width = bounds.width.max(0.0);
        bounds.height = bounds.height.max(0.0);
        cursor += legend_main_axis_span(&bounds, axis);
        result.insert(key.clone(), bounds);
    }

    result
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

fn flexible_legend_span(container_main: f32, fixed_total: f32, flexible_count: usize) -> f32 {
    if flexible_count == 0 {
        0.0
    } else {
        ((container_main - fixed_total).max(0.0) / flexible_count as f32).max(MIN_COMPONENT_SIZE)
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

fn legend_cross_axis_extent(layout: &FrameLayout, side: AxisPosition) -> f32 {
    let position = match side {
        AxisPosition::Top => LegendPosition::Top,
        AxisPosition::Right => LegendPosition::Right,
        AxisPosition::Bottom => LegendPosition::Bottom,
        AxisPosition::Left => LegendPosition::Left,
    };
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

#[cfg(test)]
mod tests {
    use avenger_chart_core::{LayoutBounds, LegendPosition, Size2D, TitleSpan};
    use avenger_chart_legend::{LegendMeasurement, LegendMeasurements};
    use avenger_layout::Edges as LayoutEdges;

    use crate::layout::declared_frame::{
        DeclaredAxis as FrameAxis, DeclaredAxisSizing as FrameAxisSizing, DeclaredFrame as Frame,
        DeclaredSide as FrameSide,
    };
    use indexmap::IndexMap;

    use super::{
        FrameChrome, MIN_COMPONENT_SIZE, layout_legend_group, snap_rect_edges,
        solve_native_frame_layout, title_band_bounds,
    };

    fn frame_side(margin: f32, bands: &[f32], outer: f32, inner: f32) -> FrameSide {
        FrameSide {
            margin,
            bands: bands.to_vec(),
            outer,
            inner,
        }
    }

    #[test]
    fn title_band_spans_plot_content_or_full_canvas_chrome() {
        // Mirrors the legacy column structure
        // [margin 10][overflow-left 35][plot 338][overflow-right 7][legend 57][margin 10]
        // with a 21px title band after a 10px top margin.
        let solution = Frame {
            horizontal: FrameAxis {
                sizing: FrameAxisSizing::ContentFixed { content: 338.0 },
                leading: frame_side(10.0, &[], 0.0, 35.0),
                trailing: frame_side(10.0, &[], 57.0, 7.0),
                content_min: MIN_COMPONENT_SIZE,
            },
            vertical: FrameAxis {
                sizing: FrameAxisSizing::ContentFixed { content: 203.0 },
                leading: frame_side(10.0, &[21.0], 0.0, 16.0),
                trailing: frame_side(10.0, &[], 0.0, 0.0),
                content_min: MIN_COMPONENT_SIZE,
            },
        }
        .solve();

        let plot_area_title =
            title_band_bounds(&solution, 0, TitleSpan::PlotArea, true, true, true);
        assert_eq!(
            plot_area_title,
            LayoutBounds {
                x: 45.0,
                y: 10.0,
                width: 338.0,
                height: 21.0,
            }
        );

        let canvas_title = title_band_bounds(&solution, 0, TitleSpan::Canvas, true, true, true);
        assert_eq!(
            canvas_title,
            LayoutBounds {
                x: 10.0,
                y: 10.0,
                width: 437.0,
                height: 21.0,
            }
        );
    }

    #[test]
    fn snap_rect_edges_rounds_edges_not_sizes() {
        let bounds = snap_rect_edges(LayoutBounds {
            x: 45.0,
            y: 53.1044,
            width: 338.0,
            height: 202.8956,
        });

        assert_eq!(bounds.x, 45.0);
        assert_eq!(bounds.y, 53.0);
        assert_eq!(bounds.width, 338.0);
        assert_eq!(bounds.height, 203.0);
    }

    #[test]
    fn snap_rect_edges_keeps_fractional_spans_anchored() {
        let bounds = snap_rect_edges(LayoutBounds {
            x: 6.0,
            y: 202.0,
            width: 278.33334,
            height: 5.5,
        });

        assert_eq!(
            bounds,
            LayoutBounds {
                x: 6.0,
                y: 202.0,
                width: 278.0,
                height: 6.0,
            }
        );
    }

    #[test]
    fn legend_group_stacks_fixed_and_flexible_legends_on_main_axis() {
        let legend_keys = vec![
            "shape".to_string(),
            "fill".to_string(),
            "opacity".to_string(),
        ];
        let measurements = legend_measurements(&[
            ("shape", Size2D::new(42.0, 20.0), false),
            ("fill", Size2D::new(14.0, 30.0), true),
            ("opacity", Size2D::new(14.0, 30.0), true),
        ]);
        let bounds = layout_legend_group(
            LegendPosition::Right,
            LayoutBounds {
                x: 410.0,
                y: 12.0,
                width: 80.0,
                height: 170.0,
            },
            &legend_keys,
            &measurements,
        );

        assert_eq!(
            bounds["shape"],
            LayoutBounds {
                x: 410.0,
                y: 12.0,
                width: 42.0,
                height: 20.0,
            }
        );
        assert_eq!(
            bounds["fill"],
            LayoutBounds {
                x: 410.0,
                y: 32.0,
                width: 14.0,
                height: 75.0,
            }
        );
        assert_eq!(
            bounds["opacity"],
            LayoutBounds {
                x: 410.0,
                y: 107.0,
                width: 14.0,
                height: 75.0,
            }
        );
    }

    #[test]
    fn layout_solution_reports_guide_only_total_overflow_from_solver() {
        let chrome = FrameChrome {
            frame: Frame {
                horizontal: FrameAxis {
                    sizing: FrameAxisSizing::EnvelopeFixed { extent: 400.0 },
                    leading: FrameSide {
                        margin: 10.0,
                        bands: Vec::new(),
                        outer: 0.0,
                        inner: 5.0,
                    },
                    trailing: FrameSide {
                        margin: 10.0,
                        bands: Vec::new(),
                        outer: 0.0,
                        inner: 7.0,
                    },
                    content_min: MIN_COMPONENT_SIZE,
                },
                vertical: FrameAxis {
                    sizing: FrameAxisSizing::EnvelopeFixed { extent: 300.0 },
                    leading: FrameSide {
                        margin: 10.0,
                        bands: Vec::new(),
                        outer: 0.0,
                        inner: 6.0,
                    },
                    trailing: FrameSide {
                        margin: 10.0,
                        bands: Vec::new(),
                        outer: 0.0,
                        inner: 8.0,
                    },
                    content_min: MIN_COMPONENT_SIZE,
                },
            },
            has_title_band: false,
            has_subtitle_band: false,
            guide_overflow: LayoutEdges::new(true, true, true, true),
            title_span: TitleSpan::PlotArea,
            subtitle_span: TitleSpan::PlotArea,
        };

        let solution =
            solve_native_frame_layout(chrome, &IndexMap::new(), &LegendMeasurements::new())
                .expect("solve native frame layout");

        assert_eq!(solution.overflow.left, 5.0);
        assert_eq!(solution.overflow.right, 7.0);
        assert_eq!(solution.overflow.top, 6.0);
        assert_eq!(solution.overflow.bottom, 8.0);
        assert_eq!(solution.total_overflow.left, solution.overflow.left);
        assert_eq!(solution.total_overflow.right, solution.overflow.right);
        assert_eq!(solution.total_overflow.top, solution.overflow.top);
        assert_eq!(solution.total_overflow.bottom, solution.overflow.bottom);
    }

    fn legend_measurements(items: &[(&str, Size2D, bool)]) -> LegendMeasurements {
        items
            .iter()
            .map(|(key, size, flexible)| {
                (
                    (*key).to_string(),
                    LegendMeasurement {
                        size: *size,
                        flexible: *flexible,
                        position: LegendPosition::Right,
                    },
                )
            })
            .collect::<IndexMap<_, _>>()
    }
}

/// Apply a known side slab to a realized frame without remeasuring guides or legends.
/// Apply a coordinated side slab to a realized frame.
///
/// The slab algebra is unchanged from the mutation-based implementation:
/// the guide layer never shrinks below its measured value, the total covers
/// the guide plus the realized legend extent, and the canvas grows by the
/// total's delta (the caller owns canvas policy beyond that). The component
/// rects then come from one re-projection of the retained chrome instead of
/// per-side translate/anchor choreography: the slab's inner layer becomes
/// the guide, the remainder becomes the outer (legend) layer, and the plot
/// content stays frozen at its realized size.
pub(crate) fn apply_frame_side_slab(
    layout: &mut LayoutSolution,
    side: AxisPosition,
    guide: f32,
    total: f32,
    legend_measurements: &LegendMeasurements,
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
        AxisPosition::Left | AxisPosition::Right => {
            layout.canvas_size.0 = (layout.canvas_size.0 + delta).max(1.0);
        }
        AxisPosition::Top | AxisPosition::Bottom => {
            layout.canvas_size.1 = (layout.canvas_size.1 + delta).max(1.0);
        }
    }

    set_overflow_side_value(&mut layout.overflow, side, guide);
    set_overflow_side_value(&mut layout.total_overflow, side, total);

    freeze_chrome_content_at_plot_size(layout);
    let frame_side = chrome_frame_side_mut(&mut layout.chrome.frame, side);
    frame_side.inner = guide;
    frame_side.outer = (total - guide).max(0.0);
    // The realized-layout threshold for an existing guide layer (the
    // initial measurement gate is MIN_GUIDE_OVERFLOW_SIZE).
    let guide_exists = guide > 0.01;
    match side {
        AxisPosition::Top => layout.chrome.guide_overflow.top = guide_exists,
        AxisPosition::Right => layout.chrome.guide_overflow.right = guide_exists,
        AxisPosition::Bottom => layout.chrome.guide_overflow.bottom = guide_exists,
        AxisPosition::Left => layout.chrome.guide_overflow.left = guide_exists,
    }

    reproject_layout_rects(layout, legend_measurements);
}

/// Retarget a realized frame to a new plot-area size without remeasuring:
/// freeze the chrome's content at the new size and re-project. Flexible
/// legends reflow under the same law as the initial solve. `canvas_size`
/// is left untouched — the callers own it (facet sizing policy can keep a
/// canvas-constrained root fixed while the plot resizes).
pub(crate) fn retarget_frame_layout_for_plot_area(
    layout: &mut LayoutSolution,
    legend_measurements: &LegendMeasurements,
    new_plot_area_width: f32,
    new_plot_area_height: f32,
) {
    layout.chrome.frame.horizontal.sizing = FrameAxisSizing::ContentFixed {
        content: new_plot_area_width,
    };
    layout.chrome.frame.vertical.sizing = FrameAxisSizing::ContentFixed {
        content: new_plot_area_height,
    };
    reproject_layout_rects(layout, legend_measurements);
}

/// Freeze both chrome axes at the realized plot-area size, so re-solving
/// derives the envelope from the plot instead of resizing the plot.
fn freeze_chrome_content_at_plot_size(layout: &mut LayoutSolution) {
    layout.chrome.frame.horizontal.sizing = FrameAxisSizing::ContentFixed {
        content: layout.frame_layout.plot_area.width,
    };
    layout.chrome.frame.vertical.sizing = FrameAxisSizing::ContentFixed {
        content: layout.frame_layout.plot_area.height,
    };
}

/// The chrome side a frame-side slab addresses.
fn chrome_frame_side_mut(
    frame: &mut super::declared_frame::DeclaredFrame,
    side: AxisPosition,
) -> &mut super::declared_frame::DeclaredSide {
    match side {
        AxisPosition::Top => &mut frame.vertical.leading,
        AxisPosition::Right => &mut frame.horizontal.trailing,
        AxisPosition::Bottom => &mut frame.vertical.trailing,
        AxisPosition::Left => &mut frame.horizontal.leading,
    }
}
