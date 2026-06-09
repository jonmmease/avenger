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

use super::{
    ComponentType, MIN_GUIDE_OVERFLOW_SIZE,
    grid::{FrameTrackSize, GridBuilder, GridLayout},
    info::LegendLayoutInfo,
    sizing::{EvaluatedLayoutSpec, EvaluatedSizeMode},
};

const DEFAULT_CANVAS_WIDTH: f32 = 400.0;
const DEFAULT_CANVAS_HEIGHT: f32 = 300.0;
const MIN_COMPONENT_SIZE: f32 = 50.0;

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
        let mut builder = GridBuilder::new();
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

        let grid = builder
            .build_with_overflow(
                input.overflow,
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
            input.overflow,
            input.layout_spec,
            &grid,
            &builder.legends_by_position,
            input.legend_measurements,
            title_span,
            subtitle_span,
        )
    }
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

fn solve_native_frame_layout(
    overflow: &OverflowSpaceRequirement,
    layout_spec: &EvaluatedLayoutSpec,
    grid: &GridLayout,
    legends_by_position: &IndexMap<LegendPosition, Vec<String>>,
    legend_measurements: &LegendMeasurements,
    title_span: TitleSpan,
    subtitle_span: TitleSpan,
) -> Result<LayoutSolution, AvengerChartError> {
    let normalized_spec = normalize_layout_spec(layout_spec);
    let plot_position = grid
        .find_component_position(&ComponentType::PlotArea)
        .ok_or_else(|| AvengerChartError::LayoutError("Frame grid is missing plot area".into()))?;

    let col_sizes = solve_tracks(
        &grid.cols,
        definite_width(&normalized_spec.canvas),
        Some(plot_position.1),
    );
    let row_sizes = solve_tracks(
        &grid.rows,
        definite_height(&normalized_spec.canvas),
        Some(plot_position.0),
    );
    let col_starts = track_starts(&col_sizes);
    let row_starts = track_starts(&row_sizes);

    let mut frame_layout = FrameLayout {
        plot_area: rounded_plot_bounds(bounds_for_cell(
            &row_starts,
            &row_sizes,
            &col_starts,
            &col_sizes,
            plot_position.0,
            plot_position.1,
            1,
            1,
        )),
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

    for (side, component) in [
        (
            AxisPosition::Top,
            ComponentType::GuideOverflow(avenger_chart_core::OverflowSide::Top),
        ),
        (
            AxisPosition::Right,
            ComponentType::GuideOverflow(avenger_chart_core::OverflowSide::Right),
        ),
        (
            AxisPosition::Bottom,
            ComponentType::GuideOverflow(avenger_chart_core::OverflowSide::Bottom),
        ),
        (
            AxisPosition::Left,
            ComponentType::GuideOverflow(avenger_chart_core::OverflowSide::Left),
        ),
    ] {
        if overflow_side_value(overflow, side) > MIN_GUIDE_OVERFLOW_SIZE
            && let Some((row, col)) = grid.find_component_position(&component)
        {
            frame_layout.guide_overflows.insert(
                side,
                rounded_guide_bounds(bounds_for_cell(
                    &row_starts,
                    &row_sizes,
                    &col_starts,
                    &col_sizes,
                    row,
                    col,
                    1,
                    1,
                )),
            );
            if let Some(bounds) = frame_layout.guide_overflows.get(&side) {
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
            }
        }
    }

    frame_layout.title = title_or_subtitle_bounds(
        grid,
        ComponentType::Title,
        title_span,
        &row_starts,
        &row_sizes,
        &col_starts,
        &col_sizes,
    );
    frame_layout.subtitle = title_or_subtitle_bounds(
        grid,
        ComponentType::Subtitle,
        subtitle_span,
        &row_starts,
        &row_sizes,
        &col_starts,
        &col_sizes,
    );

    for (position, legend_keys) in legends_by_position {
        let component = ComponentType::LegendContainer(*position);
        let Some((row, col)) = grid.find_component_position(&component) else {
            continue;
        };
        let container = bounds_for_cell(
            &row_starts,
            &row_sizes,
            &col_starts,
            &col_sizes,
            row,
            col,
            1,
            1,
        );
        for (key, bounds) in
            layout_legend_group(*position, container, legend_keys, legend_measurements)
        {
            let bounds = rounded_bounds(bounds);
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

    Ok(LayoutSolution {
        frame_layout,
        canvas_size: (col_sizes.iter().sum(), row_sizes.iter().sum()),
        overflow: guide_overflow.clone(),
        total_overflow: guide_overflow,
        legend_info,
    })
}

fn normalize_layout_spec(layout_spec: &EvaluatedLayoutSpec) -> EvaluatedLayoutSpec {
    match (&layout_spec.canvas, &layout_spec.plot_area) {
        (EvaluatedSizeMode::Auto, EvaluatedSizeMode::Auto) => {
            let mut spec = layout_spec.clone();
            spec.canvas = EvaluatedSizeMode::Fixed {
                width: DEFAULT_CANVAS_WIDTH,
                height: DEFAULT_CANVAS_HEIGHT,
            };
            spec
        }
        _ => layout_spec.clone(),
    }
}

fn definite_width(mode: &EvaluatedSizeMode) -> Option<f32> {
    match mode {
        EvaluatedSizeMode::Fixed { width, .. } | EvaluatedSizeMode::Width(width) => Some(*width),
        EvaluatedSizeMode::Height(_) | EvaluatedSizeMode::Auto => None,
    }
}

fn definite_height(mode: &EvaluatedSizeMode) -> Option<f32> {
    match mode {
        EvaluatedSizeMode::Fixed { height, .. } | EvaluatedSizeMode::Height(height) => {
            Some(*height)
        }
        EvaluatedSizeMode::Width(_) | EvaluatedSizeMode::Auto => None,
    }
}

fn solve_tracks(
    tracks: &[FrameTrackSize],
    definite_available: Option<f32>,
    plot_track: Option<usize>,
) -> Vec<f32> {
    let mut sizes = vec![0.0f32; tracks.len()];
    let mut fixed_total = 0.0f32;
    let mut fr_weight_total = 0.0f32;
    let mut fr_min_total = 0.0f32;
    let mut fr_tracks = Vec::new();

    for (index, track) in tracks.iter().copied().enumerate() {
        match track {
            FrameTrackSize::Fixed(value) => {
                sizes[index] = value.max(0.0);
                fixed_total += sizes[index];
            }
            FrameTrackSize::Fr(weight) => {
                let weight = weight.max(0.0);
                let min = if Some(index) == plot_track {
                    MIN_COMPONENT_SIZE
                } else {
                    0.0
                };
                fr_weight_total += weight;
                fr_min_total += min;
                fr_tracks.push((index, weight, min));
            }
        }
    }

    if fr_tracks.is_empty() {
        return sizes;
    }

    let fr_available = definite_available
        .map(|available| (available - fixed_total).max(fr_min_total))
        .unwrap_or(fr_min_total);
    let extra = (fr_available - fr_min_total).max(0.0);
    for (index, weight, min) in fr_tracks {
        let share = if fr_weight_total > 0.0 {
            extra * weight / fr_weight_total
        } else {
            0.0
        };
        sizes[index] = min + share;
    }

    sizes
}

fn track_starts(sizes: &[f32]) -> Vec<f32> {
    let mut cursor = 0.0f32;
    sizes
        .iter()
        .map(|size| {
            let start = cursor;
            cursor += *size;
            start
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn bounds_for_cell(
    row_starts: &[f32],
    row_sizes: &[f32],
    col_starts: &[f32],
    col_sizes: &[f32],
    row: usize,
    col: usize,
    row_span: usize,
    col_span: usize,
) -> LayoutBounds {
    LayoutBounds {
        x: col_starts[col],
        y: row_starts[row],
        width: col_sizes[col..col + col_span].iter().sum(),
        height: row_sizes[row..row + row_span].iter().sum(),
    }
}

fn rounded_plot_bounds(bounds: LayoutBounds) -> LayoutBounds {
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

fn rounded_guide_bounds(bounds: LayoutBounds) -> LayoutBounds {
    LayoutBounds {
        x: bounds.x.round(),
        y: bounds.y.round(),
        width: bounds.width.ceil(),
        height: bounds.height.ceil(),
    }
}

fn rounded_bounds(bounds: LayoutBounds) -> LayoutBounds {
    LayoutBounds {
        x: bounds.x.round(),
        y: bounds.y.round(),
        width: bounds.width.round(),
        height: bounds.height.round(),
    }
}

fn title_or_subtitle_bounds(
    grid: &GridLayout,
    component: ComponentType,
    span: TitleSpan,
    row_starts: &[f32],
    row_sizes: &[f32],
    col_starts: &[f32],
    col_sizes: &[f32],
) -> Option<LayoutBounds> {
    let (row, col) = grid.find_component_position(&component)?;
    let plot_col = grid
        .find_component_position(&ComponentType::PlotArea)
        .map(|(_, col)| col)
        .unwrap_or(col);
    let (start_col, col_span) = match span {
        TitleSpan::PlotArea => (plot_col, 1),
        TitleSpan::Canvas => {
            let end_col = grid
                .component_cells
                .keys()
                .map(|(_, col)| *col)
                .max()
                .unwrap_or(col);
            (col, end_col.saturating_sub(col) + 1)
        }
    };
    Some(rounded_bounds(bounds_for_cell(
        row_starts, row_sizes, col_starts, col_sizes, row, start_col, 1, col_span,
    )))
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

    let flexible_span = if flexible_count > 0 {
        ((container_main - fixed_total).max(0.0) / flexible_count as f32).max(MIN_COMPONENT_SIZE)
    } else {
        0.0
    };
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

#[cfg(test)]
mod tests {
    use avenger_chart_core::{LayoutBounds, LegendPosition, OverflowSide, TitleSpan};

    use super::{
        ComponentType, FrameTrackSize, GridLayout, MIN_COMPONENT_SIZE, rounded_plot_bounds,
        solve_tracks, title_or_subtitle_bounds,
    };

    #[test]
    fn track_solver_distributes_remaining_space_to_fr_tracks() {
        let sizes = solve_tracks(
            &[
                FrameTrackSize::fixed(10.0),
                FrameTrackSize::fr(1.0),
                FrameTrackSize::fr(2.0),
                FrameTrackSize::fixed(20.0),
            ],
            Some(330.0),
            Some(1),
        );

        assert_eq!(sizes[0], 10.0);
        assert_eq!(sizes[3], 20.0);
        assert!((sizes[1] - 133.33334).abs() < 0.001);
        assert!((sizes[2] - 166.66667).abs() < 0.001);
    }

    #[test]
    fn track_solver_uses_plot_minimum_for_min_content_axis() {
        let sizes = solve_tracks(
            &[
                FrameTrackSize::fixed(10.0),
                FrameTrackSize::fr(1.0),
                FrameTrackSize::fr(1.0),
            ],
            None,
            Some(1),
        );

        assert_eq!(sizes[0], 10.0);
        assert_eq!(sizes[1], MIN_COMPONENT_SIZE);
        assert_eq!(sizes[2], 0.0);
    }

    #[test]
    fn rounded_plot_bounds_uses_rounded_track_edges() {
        let bounds = rounded_plot_bounds(LayoutBounds {
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
    fn title_plot_area_span_uses_only_plot_track_with_overflow_and_legend() {
        let mut grid = GridLayout::new();
        grid.add_component(ComponentType::Title, 1, 1);
        grid.add_component(ComponentType::PlotArea, 3, 2);
        grid.add_component(ComponentType::GuideOverflow(OverflowSide::Left), 3, 1);
        grid.add_component(ComponentType::GuideOverflow(OverflowSide::Right), 3, 3);
        grid.add_component(ComponentType::LegendContainer(LegendPosition::Right), 3, 4);

        let row_starts = vec![0.0, 10.0, 31.0, 47.0];
        let row_sizes = vec![10.0, 21.0, 16.0, 203.0];
        let col_starts = vec![0.0, 10.0, 45.0, 383.0, 390.0, 447.0];
        let col_sizes = vec![10.0, 35.0, 338.0, 7.0, 57.0, 10.0];

        let plot_area_title = title_or_subtitle_bounds(
            &grid,
            ComponentType::Title,
            TitleSpan::PlotArea,
            &row_starts,
            &row_sizes,
            &col_starts,
            &col_sizes,
        )
        .expect("plot-area title bounds");

        assert_eq!(
            plot_area_title,
            LayoutBounds {
                x: 45.0,
                y: 10.0,
                width: 338.0,
                height: 21.0,
            }
        );

        let canvas_title = title_or_subtitle_bounds(
            &grid,
            ComponentType::Title,
            TitleSpan::Canvas,
            &row_starts,
            &row_sizes,
            &col_starts,
            &col_sizes,
        )
        .expect("canvas title bounds");

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
