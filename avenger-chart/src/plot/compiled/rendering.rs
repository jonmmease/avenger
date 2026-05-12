//! Rendering pipeline for CompiledPlot

use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::{Arc, Mutex},
    time::Instant,
};

use arrow::array::{Float32Array, Float64Array};
use avenger_common::types::ColorOrGradient;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        mark::SceneMark,
        rect::SceneRectMark,
    },
    scene_graph::SceneGraph,
};
use datafusion::{
    arrow::{
        array::Int32Array,
        compute::concat_batches,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    logical_expr::{Expr, lit, when},
    prelude::SessionContext,
};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use tracing::{Level, debug, trace};

use crate::{
    channel::{
        resolution::resolve_all_channel_refs,
        value::{ChannelValue, ConditionalValue, strip_trailing_numbers},
    },
    coords::{
        CoordMeasurement, FacetCoordinationMode, coordinate_overflow_for_guides_with_mode,
        coordinate_overflow_for_guides_with_mode_until,
    },
    error::AvengerChartError,
    facet::{
        coord::{
            FacetBandCoordMeasurement, FacetBandCoordMeasurementFixed, FacetCellRuntime,
            retarget_scale_ranges_for_plot_area,
        },
        debug as facet_debug,
        empty_cell_policy::FacetEmptyCellPolicy,
        evaluated_facet_tree::{EvaluatedFacetTree, PartitionContent, PartitionNode},
        layout_slabs::LayoutSlabs,
        marks::facet::{FacetMarkRef, facet_mark_ref},
    },
    guide::OverflowSpaceRequirement,
    layout::{
        ChartLayout, EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode, LayoutBounds,
        LayoutSpec, Margins, SizeMode,
    },
    legend::LegendPosition,
    marks::CompiledMark,
    maybe::Maybe,
    render::context::{FacetRuntimeSizingMode, FacetSubtreeSnapshotCapture},
    render::{
        CoordinationCheckpoint, EvaluatedPlot, EvaluationContext, EvaluationMetrics,
        EvaluationOptions, FacetSubtreeCheckpoint, FacetSubtreeSelector, FacetSubtreeSnapshot,
        LayoutSnapshot, LayoutSolution, RefinementCheckpoint, RenderContext, RenderState,
        WholeChartSnapshot, debug::create_debug_layout_rects,
    },
    scales::{ConfiguredScaleDataFusionExt, ConfiguredScaleWithSpec},
    serialization::{LogicalExprNodeExt, LogicalPlanNodeExt},
    theme::{Theme, ThemeContext},
    utils::{params_to_datafusion, parse_color_string},
};

use super::{
    CompiledPlot, ComponentsMeasurement, PlotComponents,
    expr_eval::evaluate_f32_expr,
    legends::{LegendPlanScope, PreparedLegendPlan},
    scale_provider::{DynamicScaleProvider, ScaleProvider},
    scales::build_scale_builder_from_marks,
};

/// Prepared data for mark evaluation (measure or render pass)
struct PreparedMarkData {
    /// Array data batch (multiple rows), or None if all channels are scalar
    data_batch: Option<RecordBatch>,
    /// Scalar data batch (single row) for channels that don't vary per mark
    scalar_batch: RecordBatch,
    /// Render state with plot dimensions and scales
    render_state: RenderState,
}

#[derive(Clone, Copy, Debug)]
struct RelativeBounds {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

impl RelativeBounds {
    fn empty() -> Self {
        Self {
            min_x: f32::INFINITY,
            min_y: f32::INFINITY,
            max_x: f32::NEG_INFINITY,
            max_y: f32::NEG_INFINITY,
        }
    }

    fn from_plot_and_overflow(
        plot_width: f32,
        plot_height: f32,
        overflow: &OverflowSpaceRequirement,
    ) -> Self {
        Self {
            min_x: -overflow.left,
            min_y: -overflow.top,
            max_x: plot_width + overflow.right,
            max_y: plot_height + overflow.bottom,
        }
    }

    fn is_empty(self) -> bool {
        !self.min_x.is_finite()
            || !self.min_y.is_finite()
            || !self.max_x.is_finite()
            || !self.max_y.is_finite()
    }

    fn union_with(&mut self, other: Self) {
        if other.is_empty() {
            return;
        }
        if self.is_empty() {
            *self = other;
            return;
        }
        self.min_x = self.min_x.min(other.min_x);
        self.min_y = self.min_y.min(other.min_y);
        self.max_x = self.max_x.max(other.max_x);
        self.max_y = self.max_y.max(other.max_y);
    }

    fn translated(self, dx: f32, dy: f32) -> Self {
        if self.is_empty() {
            return self;
        }
        Self {
            min_x: self.min_x + dx,
            min_y: self.min_y + dy,
            max_x: self.max_x + dx,
            max_y: self.max_y + dy,
        }
    }
}

#[derive(Clone, Debug)]
struct RecursiveOverflowSnapshot {
    guide: OverflowSpaceRequirement,
    total: OverflowSpaceRequirement,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum FacetSizingStrategy {
    CanvasFit,
    FixedSubplot {
        leaf_plot_width: f32,
        leaf_plot_height: f32,
    },
}

impl FacetSizingStrategy {
    fn coordination_mode(self) -> FacetCoordinationMode {
        match self {
            Self::CanvasFit => FacetCoordinationMode::CanvasFullCycle,
            Self::FixedSubplot { .. } => FacetCoordinationMode::FixedFullCycle,
        }
    }

    fn runtime_sizing_mode(self) -> FacetRuntimeSizingMode {
        match self {
            Self::CanvasFit => FacetRuntimeSizingMode::CanvasFit,
            Self::FixedSubplot {
                leaf_plot_width,
                leaf_plot_height,
            } => FacetRuntimeSizingMode::FixedSubplot {
                leaf_plot_width,
                leaf_plot_height,
            },
        }
    }
}

struct SelectedFacetSubtree<'a> {
    plot: &'a CompiledPlot,
    measurement: &'a ComponentsMeasurement,
    data_override: Option<&'a DataFrame>,
    facet_path: Vec<ScalarValue>,
    dimensions_are_plot_area: bool,
}

fn facet_band_children(
    measurement: &ComponentsMeasurement,
) -> Option<(&Arc<CompiledPlot>, &[FacetCellRuntime])> {
    if let Some(facet_band) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
    {
        return Some((&facet_band.compiled_subplot, &facet_band.cells));
    }

    measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurementFixed>()
        .map(|facet_band| (&facet_band.compiled_subplot, facet_band.cells.as_slice()))
}

struct CanvasRefinementIterationOutcome {
    reached_snapshot_checkpoint: bool,
    overflow_grew: Option<bool>,
}

impl CompiledPlot {
    /// Initial estimate for plot area as ratio of total canvas size
    const INITIAL_PLOT_AREA_RATIO: f32 = 0.8;

    /// Default margin in pixels when not specified in theme or expression
    const DEFAULT_MARGIN: f32 = 10.0;

    /// Stop canvas refinement once dimensions are effectively stable.
    const LAYOUT_REFINEMENT_EPSILON: f32 = 0.5;

    /// Extra whitespace around standalone facet-subtree snapshot renders.
    const FACET_SUBTREE_SNAPSHOT_PADDING: f32 = 24.0;

    fn overflow_increased(
        previous: &OverflowSpaceRequirement,
        next: &OverflowSpaceRequirement,
        epsilon: f32,
    ) -> bool {
        next.top > previous.top + epsilon
            || next.right > previous.right + epsilon
            || next.bottom > previous.bottom + epsilon
            || next.left > previous.left + epsilon
    }

    fn collect_recursive_overflow_snapshots(
        measurement: &ComponentsMeasurement,
        snapshots: &mut Vec<RecursiveOverflowSnapshot>,
    ) {
        snapshots.push(RecursiveOverflowSnapshot {
            guide: measurement.layout.overflow.clone(),
            total: measurement.layout.total_overflow.clone(),
        });

        if let Some(facet_measurement) = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
        {
            for child in facet_measurement.child_measurements_iter() {
                Self::collect_recursive_overflow_snapshots(child, snapshots);
            }
            return;
        }

        if let Some(facet_measurement) = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurementFixed>()
        {
            for child in facet_measurement.child_measurements_iter() {
                Self::collect_recursive_overflow_snapshots(child, snapshots);
            }
        }
    }

    fn recursive_overflow_snapshot(
        measurement: &ComponentsMeasurement,
    ) -> Vec<RecursiveOverflowSnapshot> {
        let mut snapshots = Vec::new();
        Self::collect_recursive_overflow_snapshots(measurement, &mut snapshots);
        snapshots
    }

    fn recursive_overflow_increased(
        previous: &[RecursiveOverflowSnapshot],
        next: &[RecursiveOverflowSnapshot],
        epsilon: f32,
    ) -> bool {
        let empty = RecursiveOverflowSnapshot {
            guide: OverflowSpaceRequirement::default(),
            total: OverflowSpaceRequirement::default(),
        };
        for idx in 0..previous.len().max(next.len()) {
            let previous = previous.get(idx).unwrap_or(&empty);
            let next = next.get(idx).unwrap_or(&empty);
            if Self::overflow_increased(&previous.guide, &next.guide, epsilon)
                || Self::overflow_increased(&previous.total, &next.total, epsilon)
            {
                return true;
            }
        }
        false
    }

    fn collect_facet_probe_size_overrides(
        measurement: &ComponentsMeasurement,
        overrides: &mut HashMap<Vec<ScalarValue>, (f32, f32)>,
    ) {
        let Some((_compiled_subplot, cells)) = facet_band_children(measurement) else {
            return;
        };

        for cell in cells {
            overrides.insert(
                cell.plan.full_path.clone(),
                (
                    cell.measurement.plot_area_width,
                    cell.measurement.plot_area_height,
                ),
            );
            Self::collect_facet_probe_size_overrides(&cell.measurement, overrides);
        }
    }

    fn canvas_facet_cell_positions(facet_measurement: &FacetBandCoordMeasurement) -> Vec<f32> {
        let layout = facet_measurement
            .coordinated_layout
            .as_ref()
            .unwrap_or(&facet_measurement.local_layout);
        let mut positions = Vec::with_capacity(facet_measurement.cells.len());
        let mut cursor = layout.outer_start.max(0.0);
        let gap = layout.padding_inner_px.max(0.0);
        for (idx, cell) in facet_measurement.cells.iter().enumerate() {
            positions.push(cursor);
            let main_size = match facet_measurement.axis {
                crate::coords::FacetAxis::Column => cell.measurement.plot_area_width,
                crate::coords::FacetAxis::Row => cell.measurement.plot_area_height,
            }
            .max(0.0);
            cursor += main_size;
            if idx + 1 < facet_measurement.cells.len() {
                cursor += gap;
            }
        }
        positions
    }

    fn fixed_facet_cell_positions(facet_measurement: &FacetBandCoordMeasurementFixed) -> Vec<f32> {
        facet_measurement.fixed_main_axis_positions.clone()
    }

    fn component_relative_bounds(measurement: &ComponentsMeasurement) -> RelativeBounds {
        let mut bounds = RelativeBounds::from_plot_and_overflow(
            measurement.plot_area_width,
            measurement.plot_area_height,
            &measurement.layout.total_overflow,
        );

        if let Some(facet_measurement_fixed) = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurementFixed>()
        {
            let facet_measurement = &facet_measurement_fixed.base;
            let cell_positions = Self::fixed_facet_cell_positions(facet_measurement_fixed);
            let slabs = LayoutSlabs::from_coordinated(&facet_measurement.coordinated_overflow);
            let (origin_offset_x, origin_offset_y) = match facet_measurement.axis {
                crate::coords::FacetAxis::Column => (0.0, slabs.legend.top),
                crate::coords::FacetAxis::Row => (slabs.legend.left, 0.0),
            };

            let mut facet_bounds = RelativeBounds::empty();
            for (idx, cell) in facet_measurement.cells.iter().enumerate() {
                if idx >= cell_positions.len() {
                    continue;
                }
                let child_bounds = Self::component_relative_bounds(&cell.measurement);
                let translated = match facet_measurement.axis {
                    crate::coords::FacetAxis::Column => child_bounds
                        .translated(cell_positions[idx] + origin_offset_x, origin_offset_y),
                    crate::coords::FacetAxis::Row => child_bounds
                        .translated(origin_offset_x, cell_positions[idx] + origin_offset_y),
                };
                facet_bounds.union_with(translated);
            }

            bounds.union_with(facet_bounds);
        } else if let Some(facet_measurement) = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
        {
            let cell_positions = Self::canvas_facet_cell_positions(facet_measurement);
            let slabs = LayoutSlabs::from_coordinated(&facet_measurement.coordinated_overflow);
            let (origin_offset_x, origin_offset_y) = match facet_measurement.axis {
                crate::coords::FacetAxis::Column => (0.0, slabs.legend.top),
                crate::coords::FacetAxis::Row => (slabs.legend.left, 0.0),
            };

            let mut facet_bounds = RelativeBounds::empty();
            for (idx, cell) in facet_measurement.cells.iter().enumerate() {
                if idx >= cell_positions.len() {
                    continue;
                }
                let child_bounds = Self::component_relative_bounds(&cell.measurement);
                let translated = match facet_measurement.axis {
                    crate::coords::FacetAxis::Column => child_bounds
                        .translated(cell_positions[idx] + origin_offset_x, origin_offset_y),
                    crate::coords::FacetAxis::Row => child_bounds
                        .translated(origin_offset_x, cell_positions[idx] + origin_offset_y),
                };
                facet_bounds.union_with(translated);
            }

            bounds.union_with(facet_bounds);
        }

        bounds
    }

    fn fixed_subplot_required_canvas_size(
        measurement: &ComponentsMeasurement,
    ) -> Option<(f32, f32)> {
        let relative_bounds = Self::component_relative_bounds(measurement);
        if relative_bounds.is_empty() {
            return None;
        }
        let plot_bounds = measurement.layout.plot_area_bounds();
        let abs_min_x = plot_bounds.x + relative_bounds.min_x;
        let abs_min_y = plot_bounds.y + relative_bounds.min_y;
        let abs_max_x = plot_bounds.x + relative_bounds.max_x;
        let abs_max_y = plot_bounds.y + relative_bounds.max_y;

        // We do not shift plot-area origin in fixed-subplot realization. If marks extend
        // left/top of origin, grow canvas conservatively and log the residual.
        let required_width = abs_max_x.max(measurement.layout.canvas_size.0).max(1.0);
        let required_height = abs_max_y.max(measurement.layout.canvas_size.1).max(1.0);

        if abs_min_x < -Self::LAYOUT_REFINEMENT_EPSILON
            || abs_min_y < -Self::LAYOUT_REFINEMENT_EPSILON
        {
            debug!(
                abs_min_x,
                abs_min_y, "fixed-subplot required bounds extend left/top of canvas origin"
            );
        }

        Some((required_width, required_height))
    }

    fn legends_within_canvas_recursive(measurement: &ComponentsMeasurement) -> bool {
        let canvas_width = measurement.canvas_size.0;
        let canvas_height = measurement.canvas_size.1;
        for bounds in measurement.layout.taffy_layout.legends.values() {
            if bounds.x < -0.5
                || bounds.y < -0.5
                || bounds.x + bounds.width > canvas_width + 0.5
                || bounds.y + bounds.height > canvas_height + 0.5
            {
                return false;
            }
        }

        if let Some(facet_measurement_fixed) = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurementFixed>()
        {
            return facet_measurement_fixed
                .child_measurements_iter()
                .all(Self::legends_within_canvas_recursive);
        }
        if let Some(facet_measurement) = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
        {
            return facet_measurement
                .child_measurements_iter()
                .all(Self::legends_within_canvas_recursive);
        }

        true
    }

    fn marks_use_auto_empty_cell_policy(marks: &[Arc<dyn CompiledMark>]) -> bool {
        marks.iter().any(|mark| {
            if let Some(facet_mark) = facet_mark_ref(mark.as_ref()) {
                return match facet_mark {
                    FacetMarkRef::Row(facet_row) => {
                        matches!(
                            facet_row.facet_empty_cell_policy(),
                            FacetEmptyCellPolicy::Auto
                        ) || Self::marks_use_auto_empty_cell_policy(
                            &facet_row.compiled_subplot().marks,
                        )
                    }
                    FacetMarkRef::Col(facet_col) => {
                        matches!(
                            facet_col.facet_empty_cell_policy(),
                            FacetEmptyCellPolicy::Auto
                        ) || Self::marks_use_auto_empty_cell_policy(
                            &facet_col.compiled_subplot().marks,
                        )
                    }
                };
            }
            false
        })
    }

    fn marks_contain_facet(marks: &[Arc<dyn CompiledMark>]) -> bool {
        marks
            .iter()
            .any(|mark| facet_mark_ref(mark.as_ref()).is_some())
    }

    fn contains_partial_size_mode(mode: &SizeMode) -> bool {
        matches!(mode, SizeMode::Width(_) | SizeMode::Height(_))
    }

    fn validate_no_nested_subplot_plot_size_under_facet(
        marks: &[Arc<dyn CompiledMark>],
        path: &mut Vec<String>,
    ) -> Result<(), AvengerChartError> {
        for (idx, mark) in marks.iter().enumerate() {
            let Some(facet_mark) = facet_mark_ref(mark.as_ref()) else {
                continue;
            };

            path.push(format!("facet[{idx}]"));
            let subplot = facet_mark.compiled_subplot();
            if !matches!(subplot.layout_spec.plot_area, SizeMode::Auto) {
                let facet_path = if path.is_empty() {
                    "facet-root".to_string()
                } else {
                    path.join(" -> ")
                };
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Nested subplot at {facet_path} cannot set `plot_size(...)` when used under a facet. \
                     Set `plot_size(...)` only on the top-level faceted plot."
                )));
            }
            Self::validate_no_nested_subplot_plot_size_under_facet(&subplot.marks, path)?;
            path.pop();
        }
        Ok(())
    }

    fn resolve_facet_sizing_strategy(
        &self,
        evaluated_layout_spec: &EvaluatedLayoutSpec,
    ) -> Result<FacetSizingStrategy, AvengerChartError> {
        if !Self::marks_contain_facet(&self.marks) {
            return Ok(FacetSizingStrategy::CanvasFit);
        }

        Self::validate_no_nested_subplot_plot_size_under_facet(&self.marks, &mut Vec::new())?;

        if Self::contains_partial_size_mode(&self.layout_spec.canvas)
            || Self::contains_partial_size_mode(&self.layout_spec.plot_area)
        {
            return Err(AvengerChartError::InvalidArgument(
                "Faceted charts do not support partial `canvas_constraint`/`plot_constraint` in this mode. \
                 Use `canvas_size(...)` (canvas-fit mode) or `plot_size(width, height)` (fixed-subplot mode)."
                    .to_string(),
            ));
        }

        if let EvaluatedSizeMode::Fixed {
            width: leaf_plot_width,
            height: leaf_plot_height,
        } = evaluated_layout_spec.plot_area
        {
            if matches!(
                evaluated_layout_spec.canvas,
                EvaluatedSizeMode::Fixed { .. }
            ) {
                return Err(AvengerChartError::InvalidArgument(
                    "Faceted charts cannot combine `canvas_size(...)` and `plot_size(...)`. \
                     Use exactly one sizing mode."
                        .to_string(),
                ));
            }
            return Ok(FacetSizingStrategy::FixedSubplot {
                leaf_plot_width,
                leaf_plot_height,
            });
        }

        Ok(FacetSizingStrategy::CanvasFit)
    }

    fn synthesize_subtree_plot_area_from_leaf_size(
        node: &PartitionNode,
        leaf_plot_width: f32,
        leaf_plot_height: f32,
    ) -> (f32, f32) {
        match &node.content {
            PartitionContent::Leaf { values } => {
                let count = values.len().max(1) as f32;
                match node.direction {
                    crate::guide::FacetDirection::Column => {
                        (leaf_plot_width * count, leaf_plot_height)
                    }
                    crate::guide::FacetDirection::Row => {
                        (leaf_plot_width, leaf_plot_height * count)
                    }
                }
            }
            PartitionContent::Branch { children } => {
                let mut child_sizes = children.values().map(|child| {
                    Self::synthesize_subtree_plot_area_from_leaf_size(
                        child.as_ref(),
                        leaf_plot_width,
                        leaf_plot_height,
                    )
                });

                let Some((first_w, first_h)) = child_sizes.next() else {
                    return (leaf_plot_width, leaf_plot_height);
                };

                match node.direction {
                    crate::guide::FacetDirection::Column => {
                        let mut total_w = first_w;
                        let mut max_h = first_h;
                        for (w, h) in child_sizes {
                            total_w += w;
                            max_h = max_h.max(h);
                        }
                        (total_w, max_h)
                    }
                    crate::guide::FacetDirection::Row => {
                        let mut max_w = first_w;
                        let mut total_h = first_h;
                        for (w, h) in child_sizes {
                            max_w = max_w.max(w);
                            total_h += h;
                        }
                        (max_w, total_h)
                    }
                }
            }
        }
    }

    fn derive_fixed_subplot_plot_area(
        facet_tree: &EvaluatedFacetTree,
        leaf_plot_width: f32,
        leaf_plot_height: f32,
    ) -> (f32, f32) {
        if let Some(root) = facet_tree.root() {
            Self::synthesize_subtree_plot_area_from_leaf_size(
                root,
                leaf_plot_width,
                leaf_plot_height,
            )
        } else {
            (leaf_plot_width.max(1.0), leaf_plot_height.max(1.0))
        }
    }

    fn layout_spec_for_facet_sizing_strategy(
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        facet_tree: &EvaluatedFacetTree,
        strategy: FacetSizingStrategy,
    ) -> EvaluatedLayoutSpec {
        match strategy {
            FacetSizingStrategy::CanvasFit => evaluated_layout_spec.clone(),
            FacetSizingStrategy::FixedSubplot {
                leaf_plot_width,
                leaf_plot_height,
            } => {
                let (required_plot_area_width, required_plot_area_height) =
                    Self::derive_fixed_subplot_plot_area(
                        facet_tree,
                        leaf_plot_width,
                        leaf_plot_height,
                    );
                let mut adjusted = evaluated_layout_spec.clone();
                adjusted.canvas = EvaluatedSizeMode::Auto;
                adjusted.plot_area = EvaluatedSizeMode::Fixed {
                    width: required_plot_area_width.max(1.0),
                    height: required_plot_area_height.max(1.0),
                };
                adjusted
            }
        }
    }
}

/// Evaluate a SizeMode to get an EvaluatedSizeMode with concrete f32 values
async fn evaluate_size_mode(
    size_mode: &SizeMode,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<EvaluatedSizeMode, AvengerChartError> {
    match size_mode {
        SizeMode::Fixed { width, height } => {
            let width_node: LogicalExprNode = width.clone().into();
            let height_node: LogicalExprNode = height.clone().into();
            let width_expr = width_node.to_expr(ctx)?;
            let height_expr = height_node.to_expr(ctx)?;
            let w = evaluate_f32_expr(&width_expr, ctx, params).await?;
            let h = evaluate_f32_expr(&height_expr, ctx, params).await?;
            Ok(EvaluatedSizeMode::Fixed {
                width: w,
                height: h,
            })
        }
        SizeMode::Width(width) => {
            let width_node: LogicalExprNode = width.clone().into();
            let width_expr = width_node.to_expr(ctx)?;
            let w = evaluate_f32_expr(&width_expr, ctx, params).await?;
            Ok(EvaluatedSizeMode::Width(w))
        }
        SizeMode::Height(height) => {
            let height_node: LogicalExprNode = height.clone().into();
            let height_expr = height_node.to_expr(ctx)?;
            let h = evaluate_f32_expr(&height_expr, ctx, params).await?;
            Ok(EvaluatedSizeMode::Height(h))
        }
        SizeMode::Auto => Ok(EvaluatedSizeMode::Auto),
    }
}

/// Evaluate Margins to get concrete f32 values (from expression, theme, or default)
async fn evaluate_margins(
    margins: &Margins,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    theme: &Theme,
) -> Result<EvaluatedMargins, AvengerChartError> {
    // Helper to query margin from theme
    let query_margin = |property: &str| -> f32 {
        let canvas_ctx = ThemeContext::new("canvas", params.clone());
        theme
            .query(&canvas_ctx, property)
            .and_then(|v| v.as_font_size(params, theme.get_base_font_size(params)))
            .unwrap_or(CompiledPlot::DEFAULT_MARGIN)
    };

    // Evaluate each margin field, checking expression → theme → default
    let top = match margins.top.as_ref() {
        Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            evaluate_f32_expr(&expr, ctx, params).await?
        }
        _ => query_margin("margin-top"),
    };

    let right = match margins.right.as_ref() {
        Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            evaluate_f32_expr(&expr, ctx, params).await?
        }
        _ => query_margin("margin-right"),
    };

    let bottom = match margins.bottom.as_ref() {
        Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            evaluate_f32_expr(&expr, ctx, params).await?
        }
        _ => query_margin("margin-bottom"),
    };

    let left = match margins.left.as_ref() {
        Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            evaluate_f32_expr(&expr, ctx, params).await?
        }
        _ => query_margin("margin-left"),
    };

    Ok(EvaluatedMargins {
        top,
        right,
        bottom,
        left,
    })
}

/// Evaluate a LayoutSpec to get an EvaluatedLayoutSpec with concrete f32 values
async fn evaluate_layout_spec(
    layout_spec: &LayoutSpec,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    theme: &Theme,
) -> Result<EvaluatedLayoutSpec, AvengerChartError> {
    let canvas = evaluate_size_mode(&layout_spec.canvas, ctx, params).await?;
    let plot_area = evaluate_size_mode(&layout_spec.plot_area, ctx, params).await?;
    let margins = evaluate_margins(&layout_spec.margins, ctx, params, theme).await?;

    Ok(EvaluatedLayoutSpec {
        canvas,
        plot_area,
        margins,
    })
}

impl CompiledPlot {
    /// Apply scaling transformation to a channel expression
    fn apply_channel_scale(
        &self,
        channel_name: &str,
        channel_value: &ChannelValue,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
    ) -> Result<Expr, AvengerChartError> {
        match channel_value {
            ChannelValue::Value { expr } => {
                // No scaling requested, return expression as-is - convert to Expr
                expr.to_expr(ctx)
            }
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => {
                // Build a CASE WHEN expression from the conditions

                // Helper to convert color string literals to the proper ScalarValue format
                let convert_color_literal = |expr: &Expr| -> Expr {
                    // Check if this is a string literal that might be a color
                    if let Expr::Literal(scalar_value, _) = expr
                        && let ScalarValue::Utf8(Some(s)) = scalar_value
                    {
                        // Use our existing color parsing utility
                        if let Some(color_or_gradient) = parse_color_string(s)
                            && let ColorOrGradient::Color(rgba) = color_or_gradient
                        {
                            // Convert to List ScalarValue with Float32 values
                            let values: Vec<ScalarValue> = rgba
                                .into_iter()
                                .map(|v| ScalarValue::Float32(Some(v)))
                                .collect();

                            // Create the list array and wrap in ScalarValue
                            let list_array =
                                ScalarValue::new_list_nullable(&values, &DataType::Float32);
                            let scalar_list = ScalarValue::List(list_array);
                            return lit(scalar_list);
                        }
                    }
                    // Not a color literal or failed to parse, return as-is
                    expr.clone()
                };

                // For conditional values, the scale name is derived from the channel name
                let scale_key = strip_trailing_numbers(channel_name).to_string();

                // Check if we need color conversion (for color channels)
                let needs_color_conversion = matches!(channel_name, "fill" | "stroke" | "color");

                // Helper to apply scale to a conditional value
                let apply_to_conditional =
                    |cond_val: &ConditionalValue| -> Result<Expr, AvengerChartError> {
                        match cond_val {
                            ConditionalValue::Scaled { expr } => {
                                // Apply scale transformation
                                if let Some(scale) = scales.get(&scale_key) {
                                    // Convert SerializableExpr to Expr first
                                    expr.to_expr(ctx).and_then(|e| scale.to_expr(e))
                                } else {
                                    // No scale found, return expression as-is - convert to Expr
                                    expr.to_expr(ctx)
                                }
                            }
                            ConditionalValue::Value { expr } => {
                                // Pass through literal values unchanged - convert to Expr
                                let expr_df = expr.to_expr(ctx)?;
                                if needs_color_conversion {
                                    Ok(convert_color_literal(&expr_df))
                                } else {
                                    Ok(expr_df)
                                }
                            }
                        }
                    };

                // Start with the first condition
                let first_cond = &conditions[0];
                let first_value = apply_to_conditional(&first_cond.1)?;
                // Convert SerializableExpr to Expr
                let first_cond_expr = first_cond.0.to_expr(ctx)?;
                let mut case_expr = when(first_cond_expr, first_value);

                // Add remaining conditions
                for (condition, value) in &conditions[1..] {
                    let scaled_value = apply_to_conditional(value)?;
                    // Convert SerializableExpr to Expr
                    let condition_expr = condition.to_expr(ctx)?;
                    case_expr = case_expr.when(condition_expr, scaled_value);
                }

                // Add the otherwise clause
                let otherwise_value = apply_to_conditional(otherwise)?;

                Ok(case_expr.otherwise(otherwise_value)?)
            }
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                ..
            } => {
                // Determine the scale to use
                let default_scale_name = strip_trailing_numbers(channel_name).to_string();
                let scale_key = scale_name.as_ref().unwrap_or(&default_scale_name);

                // Look up the configured scale
                let scale = scales.get(scale_key).ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Scale '{}' not found for channel '{}'",
                        scale_key, channel_name
                    ))
                })?;

                // Optional debugging for size scale behavior
                // Apply the scale transformation
                // Convert SerializableExpr to Expr first
                let expr_df = expr.to_expr(ctx)?;
                if let Some(band) = band {
                    scale.to_expr_with_band(expr_df.clone(), *band)
                } else {
                    scale.to_expr(expr_df)
                }
            }
        }
    }

    /// Evaluate a single mark with an optional provided plot-level DataFrame fallback.
    /// If `provided_plot_df` is Some, it is used when the mark has no explicit data and
    /// the channels reference columns. Otherwise, falls back to this CompiledPlot's plot-level data.
    /// Prepare data batches and context for mark evaluation.
    /// This is shared between measure and render passes.
    async fn prepare_mark_data(
        &self,
        mark: &dyn CompiledMark,
        eval_ctx: &EvaluationContext,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        provided_plot_df: Option<&DataFrame>,
    ) -> Result<Option<PreparedMarkData>, AvengerChartError> {
        let ctx = &*eval_ctx.session_context;
        let params = &eval_ctx.params;

        // Get channel mappings from DataContext
        let channels = mark.data_context().channels();

        // Resolve channel references
        let channels = resolve_all_channel_refs(channels, ctx)?;

        // Check if any channel expressions reference columns
        let references_columns = channels.values().any(|channel_value| match channel_value {
            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => expr
                .to_expr(ctx)
                .map(|e| !e.column_refs().is_empty())
                .unwrap_or(false),
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => {
                conditions.iter().any(|(condition, value)| {
                    let cond_has_refs = condition
                        .to_expr(ctx)
                        .map(|e| !e.column_refs().is_empty())
                        .unwrap_or(false);
                    let value_has_refs = value
                        .expr(ctx)
                        .map(|e| !e.column_refs().is_empty())
                        .unwrap_or(false);
                    cond_has_refs || value_has_refs
                }) || otherwise
                    .expr(ctx)
                    .map(|e| !e.column_refs().is_empty())
                    .unwrap_or(false)
            }
        });

        // Determine data source
        // Priority 1: Mark's own data (e.g., reference lines) - should be used in full for all facets
        // Priority 2: Parent facet's filtered data - for nested marks without their own data
        // Priority 3: Plot-level data - fallback for top-level marks
        let df_ref = if let Some(mark_df) = mark.data_context().dataframe_with_context(ctx) {
            Some(mark_df)
        } else if let Some(df_override) = provided_plot_df.cloned() {
            Some(df_override)
        } else if !references_columns {
            None
        } else if let Some(df) = self.data.as_ref().and_then(|node| {
            node.to_logical_plan(ctx)
                .ok()
                .map(|plan| DataFrame::new(ctx.state().clone(), plan))
        }) {
            Some(df)
        } else {
            return Err(AvengerChartError::InternalError(
                "Mark expressions reference columns but no data is available".to_string(),
            ));
        };

        // Sorting
        let df = if let Some(df_ref) = df_ref {
            if let Some(sort_channel_name) = mark.sorting_channel() {
                if let Some(sort_channel) = channels.get(sort_channel_name) {
                    let sort_expr =
                        self.apply_channel_scale(sort_channel_name, sort_channel, scales, ctx)?;
                    let sorted_df = df_ref.sort(vec![sort_expr.sort(true, false)])?;
                    Arc::new(sorted_df)
                } else {
                    Arc::new(df_ref)
                }
            } else {
                Arc::new(df_ref)
            }
        } else {
            let empty_df = ctx
                .sql("SELECT 1 as _dummy")
                .await
                .map_err(AvengerChartError::DataFusionError)?;
            Arc::new(empty_df)
        };

        // Channels split
        let supported_channels = mark.supported_channels();
        let mut array_channels = Vec::new();
        let mut scalar_channels = Vec::new();
        let mut has_array_data = false;
        for channel_desc in &supported_channels {
            if let Some(channel_value) = channels.get(channel_desc.name) {
                let scaled_expr =
                    self.apply_channel_scale(channel_desc.name, channel_value, scales, ctx)?;
                if channel_desc.allow_column_ref && scaled_expr.any_column_refs() {
                    array_channels.push((channel_desc.name, scaled_expr));
                    has_array_data = true;
                } else {
                    scalar_channels.push((channel_desc.name, scaled_expr));
                }
            }
        }

        // Build array data batch
        let data_batch = if mark.wants_full_data_batch() {
            // For container marks (facets): preserve ALL columns for nested marks
            // This works whether data comes from provided_plot_df (nested) or self.data (top-level)
            let datafusion_params = params_to_datafusion(params);
            let batch = if let Some(param_values) = datafusion_params {
                (*df)
                    .clone()
                    .with_param_values(param_values)?
                    .collect()
                    .await?
            } else {
                (*df).clone().collect().await?
            };

            if batch.is_empty() {
                // Return empty batch WITH SCHEMA for facets (enables key extraction)
                let arrow_schema = std::sync::Arc::new(df.schema().as_arrow().clone());
                Some(RecordBatch::new_empty(arrow_schema))
            } else {
                let schema = batch[0].schema();
                Some(concat_batches(&schema, &batch)?)
            }
        } else if has_array_data && !mark.wants_full_data_batch() {
            // Normal path (non-facet marks): select only needed channels
            let mut select_exprs = vec![];
            for (name, expr) in &array_channels {
                select_exprs.push(expr.clone().alias(*name));
            }
            let datafusion_params = params_to_datafusion(params);
            let batch = if let Some(param_values) = datafusion_params {
                (*df)
                    .clone()
                    .select(select_exprs)?
                    .with_param_values(param_values)?
                    .collect()
                    .await?
            } else {
                (*df).clone().select(select_exprs)?.collect().await?
            };
            if batch.is_empty() {
                None
            } else {
                let schema = batch[0].schema();
                let combined = concat_batches(&schema, &batch)?;
                Some(combined)
            }
        } else {
            None
        };

        // Scalar data batch
        let mut scalar_select_exprs = vec![];
        for (name, expr) in &scalar_channels {
            scalar_select_exprs.push(expr.clone().alias(*name));
        }
        let scalar_batch = if !scalar_select_exprs.is_empty() {
            let datafusion_params = params_to_datafusion(params);
            let batch = if let Some(param_values) = datafusion_params {
                (*df)
                    .clone()
                    .select(scalar_select_exprs)?
                    .with_param_values(param_values)?
                    .collect()
                    .await?
            } else {
                (*df).clone().select(scalar_select_exprs)?.collect().await?
            };
            if batch.is_empty() {
                return Ok(None);
            } else {
                let schema = batch[0].schema();
                concat_batches(&schema, &batch)?
            }
        } else {
            RecordBatch::try_new(
                Arc::new(Schema::new(vec![Field::new(
                    "_dummy",
                    DataType::Int32,
                    false,
                )])),
                vec![Arc::new(Int32Array::from(vec![0]))],
            )?
        };

        self.validate_positional_channel_types(&data_batch, &scalar_batch)?;

        let render_state = RenderState::new(plot_width, plot_height, scales.clone());

        Ok(Some(PreparedMarkData {
            data_batch,
            scalar_batch,
            render_state,
        }))
    }

    /// Render a single mark to scene marks.
    pub(super) async fn render_mark_with_plot_df(
        &self,
        mark: &dyn CompiledMark,
        eval_ctx: &EvaluationContext,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        provided_plot_df: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let prepared = self
            .prepare_mark_data(
                mark,
                eval_ctx,
                scales,
                plot_width,
                plot_height,
                provided_plot_df,
            )
            .await?;

        let Some(prepared) = prepared else {
            return Ok(vec![]);
        };

        let render_ctx = RenderContext::new(
            eval_ctx,
            &prepared.render_state,
            facet_path,
            coord_measurement,
        );
        let coord_transform = self.coord_transform.clone_box();
        mark.render_from_data(
            prepared.data_batch.as_ref(),
            &prepared.scalar_batch,
            &render_ctx,
            coord_transform,
        )
        .await
    }

    /// Create guide marks (axes, grids) for the coordinate system
    pub(super) async fn create_guide_marks(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        guide_overflow: &OverflowSpaceRequirement,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
        data_override: Option<&DataFrame>,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Use the pre-built guide renderer if available
        if let Some(compiled_guide) = &self.compiled_guide {
            let theme = self.get_theme();
            // Extract ConfiguredScale from ConfiguredScaleWithSpec for guide renderer
            let configured_scales: HashMap<String, ConfiguredScale> = scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();

            if tracing::enabled!(Level::DEBUG)
                && let Some(y_scale) = configured_scales.get("y")
            {
                let domain = y_scale.domain();
                if let Some(float_arr) = domain.as_any().downcast_ref::<Float32Array>() {
                    let vals: Vec<f32> = float_arr.iter().flatten().collect();
                    debug!(domain = ?vals, "create_guide_marks y scale domain");
                } else if let Some(float_arr) = domain.as_any().downcast_ref::<Float64Array>() {
                    let vals: Vec<f64> = float_arr.iter().flatten().collect();
                    debug!(domain = ?vals, "create_guide_marks y scale domain");
                } else {
                    debug!(domain_type = ?domain.data_type(), "create_guide_marks y scale domain type");
                }
            }

            compiled_guide
                .evaluate(
                    &configured_scales,
                    plot_width,
                    plot_height,
                    plot_bounds,
                    guide_overflow,
                    theme.as_ref(),
                    params,
                    ctx,
                    data_override,
                    facet_tree,
                    facet_path,
                    coord_measurement,
                )
                .await
        } else {
            // No guide renderer available
            Ok(vec![])
        }
    }

    /// Compute layout with an evaluated layout specification.
    ///
    /// This unified method handles both canvas-mode and plot-area-mode layouts
    /// based on the provided EvaluatedLayoutSpec.
    pub(super) async fn compute_layout_with_spec(
        &self,
        layout_spec: &EvaluatedLayoutSpec,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        data_override: Option<&DataFrame>,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
    ) -> Result<(LayoutSolution, PreparedLegendPlan), AvengerChartError> {
        // Check for required positional scales before measuring overflow
        self.validate_positional_scales_exist(scales)?;

        // Determine if this is plot-area mode (for overflow estimation dimensions)
        let is_plot_area_mode = matches!(
            (&layout_spec.canvas, &layout_spec.plot_area),
            (EvaluatedSizeMode::Auto, EvaluatedSizeMode::Fixed { .. })
        );

        // Get dimensions for overflow estimation
        let (estimate_width, estimate_height) = if is_plot_area_mode {
            // Plot area mode: use exact plot dimensions
            match &layout_spec.plot_area {
                EvaluatedSizeMode::Fixed { width, height } => (*width, *height),
                _ => (400.0, 300.0), // Fallback
            }
        } else {
            // Canvas mode: estimate plot area as fraction of canvas
            let (canvas_w, canvas_h) = match &layout_spec.canvas {
                EvaluatedSizeMode::Fixed { width, height } => (*width, *height),
                EvaluatedSizeMode::Width(w) => (*w, 300.0),
                EvaluatedSizeMode::Height(h) => (400.0, *h),
                EvaluatedSizeMode::Auto => (400.0, 300.0),
            };
            (
                canvas_w * Self::INITIAL_PLOT_AREA_RATIO,
                canvas_h * Self::INITIAL_PLOT_AREA_RATIO,
            )
        };

        // Measure guide overflow (axis tick labels, titles, etc.)
        let overflow = if let Some(compiled_guide) = &self.compiled_guide {
            let theme = self.get_theme();
            let configured_scales: HashMap<String, ConfiguredScale> = scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();
            compiled_guide
                .measure_overflow(
                    &configured_scales,
                    estimate_width,
                    estimate_height,
                    theme.as_ref(),
                    params,
                    data_override,
                    ctx,
                    facet_tree,
                    facet_path,
                    None, // No coord_measurement yet (first pass)
                )
                .await?
        } else {
            OverflowSpaceRequirement::default()
        };

        let available_size = taffy::Size {
            width: estimate_width,
            height: estimate_height,
        };
        let scope = if facet_path.is_empty() {
            LegendPlanScope::TopLevel
        } else {
            LegendPlanScope::FacetCell
        };
        self.compute_layout_with_precomputed_overflow(
            &overflow,
            layout_spec,
            scales,
            available_size,
            ctx,
            params,
            facet_tree,
            facet_path,
            scope,
        )
        .await
    }

    async fn compute_layout_with_precomputed_overflow(
        &self,
        overflow: &OverflowSpaceRequirement,
        layout_spec: &EvaluatedLayoutSpec,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        available_size: taffy::Size<f32>,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        scope: LegendPlanScope,
    ) -> Result<(LayoutSolution, PreparedLegendPlan), AvengerChartError> {
        let legend_plan = self
            .prepare_legend_plan(
                scales,
                available_size,
                ctx,
                params,
                facet_tree,
                facet_path,
                scope,
            )
            .await?;

        let mut layout = ChartLayout::new(
            overflow,
            layout_spec,
            self.get_title(),
            self.get_subtitle(),
            self.get_theme().as_ref(),
            &legend_plan.measurements,
            ctx,
            params,
        )
        .await?;
        let mut result = layout.compute(layout_spec)?;

        // ChartLayout.compute() sets total_overflow to guide-only; add legend dimensions.
        for measurement in legend_plan.measurements.values() {
            match measurement.position {
                LegendPosition::Left => {
                    result.total_overflow.left += measurement.size.width;
                }
                LegendPosition::Right => {
                    result.total_overflow.right += measurement.size.width;
                }
                LegendPosition::Top => {
                    result.total_overflow.top += measurement.size.height;
                }
                LegendPosition::Bottom => {
                    result.total_overflow.bottom += measurement.size.height;
                }
            }
        }

        Ok((result, legend_plan))
    }

    #[cfg(test)]
    pub(crate) async fn total_overflow_from_precomputed_guide_overflow(
        &self,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_area_width: f32,
        plot_area_height: f32,
        guide_overflow: &OverflowSpaceRequirement,
        facet_path: &[ScalarValue],
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        let params_with_dims = eval_ctx.with_dimension_params(plot_area_width, plot_area_height);
        let (layout, _) = self
            .compute_layout_with_precomputed_overflow(
                guide_overflow,
                layout_spec,
                scales,
                taffy::Size {
                    width: plot_area_width,
                    height: plot_area_height,
                },
                params_with_dims.session_context.as_ref(),
                &params_with_dims.params,
                params_with_dims.facet_tree.as_ref(),
                facet_path,
                Self::legend_scope_for_facet_path(facet_path),
            )
            .await?;
        Ok(layout.total_overflow)
    }

    #[inline]
    fn legend_scope_for_facet_path(facet_path: &[ScalarValue]) -> LegendPlanScope {
        if facet_path.is_empty() {
            LegendPlanScope::TopLevel
        } else {
            LegendPlanScope::FacetCell
        }
    }

    async fn measure_overflow_with_coord(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        params: &IndexMap<String, ScalarValue>,
        data_override: Option<&DataFrame>,
        ctx: &SessionContext,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        let Some(compiled_guide) = &self.compiled_guide else {
            return Ok(OverflowSpaceRequirement::default());
        };

        let configured_scales: HashMap<String, ConfiguredScale> = scales
            .iter()
            .map(|(k, v)| (k.clone(), v.configured().clone()))
            .collect();

        compiled_guide
            .measure_overflow(
                &configured_scales,
                plot_width,
                plot_height,
                self.get_theme().as_ref(),
                params,
                data_override,
                ctx,
                facet_tree,
                facet_path,
                coord_measurement,
            )
            .await
    }

    async fn rebuild_layout_with_coord_overflow(
        &self,
        layout_spec: &EvaluatedLayoutSpec,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_area_width: f32,
        plot_area_height: f32,
        params: &IndexMap<String, ScalarValue>,
        data_override: Option<&DataFrame>,
        ctx: &SessionContext,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<(OverflowSpaceRequirement, LayoutSolution, PreparedLegendPlan), AvengerChartError>
    {
        let overflow = self
            .measure_overflow_with_coord(
                scales,
                plot_area_width,
                plot_area_height,
                params,
                data_override,
                ctx,
                facet_tree,
                facet_path,
                coord_measurement,
            )
            .await?;

        let (layout, legend_plan) = self
            .compute_layout_with_precomputed_overflow(
                &overflow,
                layout_spec,
                scales,
                taffy::Size {
                    width: plot_area_width,
                    height: plot_area_height,
                },
                ctx,
                params,
                facet_tree,
                facet_path,
                Self::legend_scope_for_facet_path(facet_path),
            )
            .await?;

        Ok((overflow, layout, legend_plan))
    }

    fn realize_canvas_plot_area_no_overflow_remeasure(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        facet_path: &[ScalarValue],
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> Result<(), AvengerChartError> {
        let plot_area_width = plot_area_width.max(1.0);
        let plot_area_height = plot_area_height.max(1.0);

        retarget_scale_ranges_for_plot_area(
            &mut measurement.scales,
            plot_area_width,
            plot_area_height,
        );

        if let Some(facet_band) = measurement
            .coord_measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
        {
            facet_band.retarget_parent_plot_area_no_remeasure(
                &mut measurement.scales,
                eval_ctx,
                plot_area_width,
                plot_area_height,
            )?;
        } else {
            measurement
                .coord_measurement
                .apply_scale_adjustments(&mut measurement.scales);
        }

        measurement.plot_area_width = plot_area_width;
        measurement.plot_area_height = plot_area_height;
        // Canvas-mode params carry the evaluated canvas dimensions for media
        // queries and user expressions. Retargeting the plot area must not
        // turn `width`/`height` into inner plot dimensions.
        measurement.clip = self.resolved_clip_region(
            eval_ctx,
            facet_path,
            &measurement.scales,
            plot_area_width,
            plot_area_height,
        );
        measurement.legend_plan.retarget_scales(&measurement.scales);

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn measure_canvas_candidate_layout_from_current_measurement(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        iter: usize,
        trace_label: &'static str,
    ) -> Result<LayoutBounds, AvengerChartError> {
        let ctx = &*eval_ctx.session_context;
        let facet_tree = eval_ctx.facet_tree.as_ref();
        let (_, candidate_layout, candidate_legend_plan) = self
            .rebuild_layout_with_coord_overflow(
                layout_spec,
                &measurement.scales,
                measurement.plot_area_width,
                measurement.plot_area_height,
                &measurement.params,
                data_override,
                ctx,
                facet_tree,
                facet_path,
                Some(measurement.coord_measurement.as_ref()),
            )
            .await?;

        let candidate_bounds = *candidate_layout.plot_area_bounds();
        let delta_w = (candidate_bounds.width - measurement.plot_area_width).abs();
        let delta_h = (candidate_bounds.height - measurement.plot_area_height).abs();
        trace!(
            iter,
            current_width = measurement.plot_area_width,
            current_height = measurement.plot_area_height,
            candidate_width = candidate_bounds.width,
            candidate_height = candidate_bounds.height,
            delta_w,
            delta_h,
            trace_label
        );

        measurement.layout = candidate_layout;
        measurement.legend_plan = candidate_legend_plan;
        measurement.canvas_size = measurement.layout.canvas_size;
        Ok(candidate_bounds)
    }

    async fn remeasure_canvas_coord_at_current_plot_area(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        scale_provider: &dyn ScaleProvider,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
    ) -> Result<(), AvengerChartError> {
        let ctx = &*eval_ctx.session_context;
        let next_width = measurement.plot_area_width.max(1.0);
        let next_height = measurement.plot_area_height.max(1.0);
        let mut probe_size_overrides = HashMap::new();
        Self::collect_facet_probe_size_overrides(measurement, &mut probe_size_overrides);
        let params_with_canvas_dims = eval_ctx
            .with_params(measurement.params.clone())
            .with_facet_probe_size_overrides(Arc::new(probe_size_overrides));

        let mut final_scales = scale_provider
            .build_scales(
                next_width,
                next_height,
                ctx,
                &params_with_canvas_dims.params,
            )
            .await?;
        let coord_measurement = self
            .measure_coord_system(
                &final_scales,
                next_width,
                next_height,
                &params_with_canvas_dims,
                data_override,
                facet_path,
                ctx,
            )
            .await?;
        coord_measurement.apply_scale_adjustments(&mut final_scales);

        measurement.plot_area_width = next_width;
        measurement.plot_area_height = next_height;
        measurement.scales = final_scales;
        measurement.coord_measurement = coord_measurement;
        measurement.clip = self.resolved_clip_region(
            eval_ctx,
            facet_path,
            &measurement.scales,
            next_width,
            next_height,
        );
        measurement.legend_plan.retarget_scales(&measurement.scales);

        Ok(())
    }

    async fn refine_measurement_after_coordination(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scale_provider: &dyn ScaleProvider,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        coordination_mode: FacetCoordinationMode,
        allow_plot_area_resize: bool,
        max_iters: usize,
        trace_label: &'static str,
    ) -> Result<(), AvengerChartError> {
        let ctx = &*eval_ctx.session_context;
        let facet_tree = eval_ctx.facet_tree.as_ref();

        for iter in 0..=max_iters {
            let (_, candidate_layout, candidate_legend_plan) = self
                .rebuild_layout_with_coord_overflow(
                    layout_spec,
                    &measurement.scales,
                    measurement.plot_area_width,
                    measurement.plot_area_height,
                    &measurement.params,
                    data_override,
                    ctx,
                    facet_tree,
                    facet_path,
                    Some(measurement.coord_measurement.as_ref()),
                )
                .await?;

            let candidate_bounds = *candidate_layout.plot_area_bounds();
            let delta_w = (candidate_bounds.width - measurement.plot_area_width).abs();
            let delta_h = (candidate_bounds.height - measurement.plot_area_height).abs();
            let overflow_grew = Self::overflow_increased(
                &measurement.layout.total_overflow,
                &candidate_layout.total_overflow,
                eval_ctx.facet_layout_refinement().overflow_growth_epsilon,
            );

            trace!(
                iter,
                current_width = measurement.plot_area_width,
                current_height = measurement.plot_area_height,
                candidate_width = candidate_bounds.width,
                candidate_height = candidate_bounds.height,
                delta_w,
                delta_h,
                overflow_grew,
                trace_label
            );

            let previous_canvas_size = measurement.canvas_size;
            measurement.layout = candidate_layout;
            measurement.legend_plan = candidate_legend_plan;
            measurement.canvas_size = measurement.layout.canvas_size;
            if !allow_plot_area_resize
                && let Some((required_width, required_height)) =
                    Self::fixed_subplot_required_canvas_size(measurement)
                && (required_width > measurement.canvas_size.0
                    || required_height > measurement.canvas_size.1)
            {
                trace!(
                    iter,
                    required_width,
                    required_height,
                    old_canvas_width = measurement.canvas_size.0,
                    old_canvas_height = measurement.canvas_size.1,
                    trace_label,
                    "fixed-subplot realization expanded canvas from coordinated subtree bounds"
                );
                measurement.canvas_size = (required_width, required_height);
                measurement.layout.canvas_size = measurement.canvas_size;
            }

            if allow_plot_area_resize {
                let next_width = candidate_bounds.width.max(1.0);
                let next_height = candidate_bounds.height.max(1.0);
                self.realize_canvas_plot_area_no_overflow_remeasure(
                    measurement,
                    eval_ctx,
                    facet_path,
                    next_width,
                    next_height,
                )?;

                if !overflow_grew {
                    eval_ctx.record_facet_refinement_converged();
                    trace!(iter, trace_label, "layout refinement converged");
                    return Ok(());
                }
                if iter == max_iters {
                    eval_ctx.record_facet_refinement_hit_max_passes();
                    debug!(
                        iter,
                        delta_w,
                        delta_h,
                        epsilon = Self::LAYOUT_REFINEMENT_EPSILON,
                        trace_label,
                        "layout refinement reached iteration cap"
                    );
                    return Ok(());
                }

                eval_ctx.record_facet_refinement_pass();
                coordinate_overflow_for_guides_with_mode(measurement, eval_ctx, coordination_mode)
                    .await?;
                continue;
            } else {
                let canvas_delta_w = (measurement.canvas_size.0 - previous_canvas_size.0).abs();
                let canvas_delta_h = (measurement.canvas_size.1 - previous_canvas_size.1).abs();
                if !overflow_grew {
                    eval_ctx.record_facet_refinement_converged();
                    debug_assert!(
                        Self::legends_within_canvas_recursive(measurement),
                        "fixed-subplot realization invariant: legends must remain within canvas bounds after convergence"
                    );
                    trace!(
                        iter,
                        canvas_delta_w,
                        canvas_delta_h,
                        trace_label,
                        "fixed-subplot layout realization converged"
                    );
                    return Ok(());
                }
                if iter == max_iters {
                    eval_ctx.record_facet_refinement_hit_max_passes();
                    debug_assert!(
                        Self::legends_within_canvas_recursive(measurement),
                        "fixed-subplot realization invariant: legends must remain within canvas bounds at iteration cap"
                    );
                    debug!(
                        iter,
                        canvas_delta_w,
                        canvas_delta_h,
                        epsilon = Self::LAYOUT_REFINEMENT_EPSILON,
                        trace_label,
                        "fixed-subplot layout realization reached iteration cap"
                    );
                    return Ok(());
                }
            }

            let (next_width, next_height) = (
                measurement.plot_area_width.max(1.0),
                measurement.plot_area_height.max(1.0),
            );
            let params_with_dims = eval_ctx.with_dimension_params(next_width, next_height);
            let merged_params = params_with_dims.params.clone();
            eval_ctx.record_facet_refinement_pass();

            let mut final_scales = scale_provider
                .build_scales(next_width, next_height, ctx, &merged_params)
                .await?;
            let coord_measurement = self
                .measure_coord_system(
                    &final_scales,
                    next_width,
                    next_height,
                    &params_with_dims,
                    data_override,
                    facet_path,
                    ctx,
                )
                .await?;
            coord_measurement.apply_scale_adjustments(&mut final_scales);

            measurement.plot_area_width = next_width;
            measurement.plot_area_height = next_height;
            measurement.scales = final_scales;
            measurement.coord_measurement = coord_measurement;
            measurement.params = merged_params;
            measurement.clip = self.resolved_clip_region(
                eval_ctx,
                facet_path,
                &measurement.scales,
                next_width,
                next_height,
            );
            measurement.legend_plan.retarget_scales(&measurement.scales);

            coordinate_overflow_for_guides_with_mode(measurement, eval_ctx, coordination_mode)
                .await?;
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_canvas_refinement_iteration(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scale_provider: &dyn ScaleProvider,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        iteration: usize,
        target_checkpoint: Option<RefinementCheckpoint>,
        trace_label: &'static str,
    ) -> Result<CanvasRefinementIterationOutcome, AvengerChartError> {
        let before = if iteration == 0 {
            None
        } else {
            Some(Self::recursive_overflow_snapshot(measurement))
        };

        if iteration > 0 {
            self.remeasure_canvas_coord_at_current_plot_area(
                measurement,
                eval_ctx,
                scale_provider,
                data_override,
                facet_path,
            )
            .await?;
            coordinate_overflow_for_guides_with_mode(
                measurement,
                eval_ctx,
                FacetCoordinationMode::CanvasFullCycle,
            )
            .await?;

            if target_checkpoint == Some(RefinementCheckpoint::Recoordinated) {
                return Ok(CanvasRefinementIterationOutcome {
                    reached_snapshot_checkpoint: true,
                    overflow_grew: None,
                });
            }
        }

        let candidate_bounds = self
            .measure_canvas_candidate_layout_from_current_measurement(
                measurement,
                eval_ctx,
                layout_spec,
                data_override,
                facet_path,
                iteration,
                trace_label,
            )
            .await?;

        if target_checkpoint == Some(RefinementCheckpoint::CandidateLayoutMeasured) {
            return Ok(CanvasRefinementIterationOutcome {
                reached_snapshot_checkpoint: true,
                overflow_grew: None,
            });
        }

        self.realize_canvas_plot_area_no_overflow_remeasure(
            measurement,
            eval_ctx,
            facet_path,
            candidate_bounds.width.max(1.0),
            candidate_bounds.height.max(1.0),
        )?;

        if target_checkpoint == Some(RefinementCheckpoint::PlotAreaRetargeted) {
            return Ok(CanvasRefinementIterationOutcome {
                reached_snapshot_checkpoint: true,
                overflow_grew: None,
            });
        }

        let overflow_grew = before.map(|before| {
            let after = Self::recursive_overflow_snapshot(measurement);
            Self::recursive_overflow_increased(
                &before,
                &after,
                eval_ctx.facet_layout_refinement().overflow_growth_epsilon,
            )
        });

        Ok(CanvasRefinementIterationOutcome {
            reached_snapshot_checkpoint: false,
            overflow_grew,
        })
    }

    async fn refine_canvas_measurement_after_coordination(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scale_provider: &dyn ScaleProvider,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        max_refinement_passes: usize,
    ) -> Result<(), AvengerChartError> {
        self.run_canvas_refinement_iteration(
            measurement,
            eval_ctx,
            layout_spec,
            scale_provider,
            data_override,
            facet_path,
            0,
            None,
            "canvas layout mandatory realization",
        )
        .await?;

        if max_refinement_passes == 0 {
            eval_ctx.record_facet_refinement_converged();
            trace!("canvas layout refinement disabled after mandatory realization");
            return Ok(());
        }

        for pass in 1..=max_refinement_passes {
            let outcome = self
                .run_canvas_refinement_iteration(
                    measurement,
                    eval_ctx,
                    layout_spec,
                    scale_provider,
                    data_override,
                    facet_path,
                    pass,
                    None,
                    "canvas layout refinement realization",
                )
                .await?;

            eval_ctx.record_facet_refinement_pass();
            let overflow_grew = outcome.overflow_grew.unwrap_or(false);
            trace!(
                pass,
                overflow_grew, "canvas layout refinement pass completed"
            );

            if !overflow_grew {
                eval_ctx.record_facet_refinement_converged();
                return Ok(());
            }
        }

        eval_ctx.record_facet_refinement_hit_max_passes();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn refine_canvas_measurement_after_coordination_until(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scale_provider: &dyn ScaleProvider,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        max_refinement_passes: usize,
        target_iteration: usize,
        target_checkpoint: RefinementCheckpoint,
    ) -> Result<(), AvengerChartError> {
        if target_iteration > max_refinement_passes {
            return Err(AvengerChartError::InternalError(format!(
                "Requested refinement snapshot iteration {} exceeds configured max iteration {}",
                target_iteration, max_refinement_passes
            )));
        }

        let target = (target_iteration == 0).then_some(target_checkpoint);
        let outcome = self
            .run_canvas_refinement_iteration(
                measurement,
                eval_ctx,
                layout_spec,
                scale_provider,
                data_override,
                facet_path,
                0,
                target,
                "canvas layout refinement snapshot",
            )
            .await?;

        if outcome.reached_snapshot_checkpoint {
            return Ok(());
        }

        if max_refinement_passes == 0 {
            return Err(AvengerChartError::InternalError(format!(
                "Requested refinement snapshot {:?} at iteration {} was not reached (canvas layout refinement snapshot)",
                target_checkpoint, target_iteration
            )));
        }

        for pass in 1..=max_refinement_passes {
            let target = (pass == target_iteration).then_some(target_checkpoint);
            let outcome = self
                .run_canvas_refinement_iteration(
                    measurement,
                    eval_ctx,
                    layout_spec,
                    scale_provider,
                    data_override,
                    facet_path,
                    pass,
                    target,
                    "canvas layout refinement snapshot",
                )
                .await?;

            if outcome.reached_snapshot_checkpoint {
                return Ok(());
            }

            if !outcome.overflow_grew.unwrap_or(false) {
                break;
            }
        }

        Err(AvengerChartError::InternalError(format!(
            "Requested refinement snapshot {:?} at iteration {} was not reached (canvas layout refinement snapshot)",
            target_checkpoint, target_iteration
        )))
    }

    #[allow(clippy::too_many_arguments)]
    async fn refine_measurement_after_coordination_until(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scale_provider: &dyn ScaleProvider,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        coordination_mode: FacetCoordinationMode,
        allow_plot_area_resize: bool,
        max_iters: usize,
        target_iteration: usize,
        target_checkpoint: RefinementCheckpoint,
        trace_label: &'static str,
    ) -> Result<(), AvengerChartError> {
        if target_iteration > max_iters {
            return Err(AvengerChartError::InternalError(format!(
                "Requested refinement snapshot iteration {} exceeds configured max iteration {}",
                target_iteration, max_iters
            )));
        }

        let ctx = &*eval_ctx.session_context;
        let facet_tree = eval_ctx.facet_tree.as_ref();

        for iter in 0..=max_iters {
            let (_, candidate_layout, candidate_legend_plan) = self
                .rebuild_layout_with_coord_overflow(
                    layout_spec,
                    &measurement.scales,
                    measurement.plot_area_width,
                    measurement.plot_area_height,
                    &measurement.params,
                    data_override,
                    ctx,
                    facet_tree,
                    facet_path,
                    Some(measurement.coord_measurement.as_ref()),
                )
                .await?;

            let candidate_bounds = *candidate_layout.plot_area_bounds();
            let overflow_grew = Self::overflow_increased(
                &measurement.layout.total_overflow,
                &candidate_layout.total_overflow,
                eval_ctx.facet_layout_refinement().overflow_growth_epsilon,
            );

            measurement.layout = candidate_layout;
            measurement.legend_plan = candidate_legend_plan;
            measurement.canvas_size = measurement.layout.canvas_size;

            if iter == target_iteration
                && target_checkpoint == RefinementCheckpoint::CandidateLayoutMeasured
            {
                return Ok(());
            }

            if !allow_plot_area_resize
                && let Some((required_width, required_height)) =
                    Self::fixed_subplot_required_canvas_size(measurement)
                && (required_width > measurement.canvas_size.0
                    || required_height > measurement.canvas_size.1)
            {
                measurement.canvas_size = (required_width, required_height);
                measurement.layout.canvas_size = measurement.canvas_size;
            }

            if allow_plot_area_resize {
                self.realize_canvas_plot_area_no_overflow_remeasure(
                    measurement,
                    eval_ctx,
                    facet_path,
                    candidate_bounds.width.max(1.0),
                    candidate_bounds.height.max(1.0),
                )?;

                if iter == target_iteration
                    && target_checkpoint == RefinementCheckpoint::PlotAreaRetargeted
                {
                    return Ok(());
                }

                if !overflow_grew || iter == max_iters {
                    break;
                }

                coordinate_overflow_for_guides_with_mode(measurement, eval_ctx, coordination_mode)
                    .await?;

                if iter == target_iteration
                    && target_checkpoint == RefinementCheckpoint::Recoordinated
                {
                    return Ok(());
                }
                continue;
            }

            if iter == target_iteration
                && target_checkpoint == RefinementCheckpoint::PlotAreaRetargeted
            {
                return Ok(());
            }

            if !overflow_grew || iter == max_iters {
                break;
            }

            let (next_width, next_height) = (
                measurement.plot_area_width.max(1.0),
                measurement.plot_area_height.max(1.0),
            );
            let params_with_dims = eval_ctx.with_dimension_params(next_width, next_height);
            let merged_params = params_with_dims.params.clone();

            let mut final_scales = scale_provider
                .build_scales(next_width, next_height, ctx, &merged_params)
                .await?;
            let coord_measurement = self
                .measure_coord_system(
                    &final_scales,
                    next_width,
                    next_height,
                    &params_with_dims,
                    data_override,
                    facet_path,
                    ctx,
                )
                .await?;
            coord_measurement.apply_scale_adjustments(&mut final_scales);

            measurement.plot_area_width = next_width;
            measurement.plot_area_height = next_height;
            measurement.scales = final_scales;
            measurement.coord_measurement = coord_measurement;
            measurement.params = merged_params;
            measurement.clip = self.resolved_clip_region(
                eval_ctx,
                facet_path,
                &measurement.scales,
                next_width,
                next_height,
            );
            measurement.legend_plan.retarget_scales(&measurement.scales);

            coordinate_overflow_for_guides_with_mode(measurement, eval_ctx, coordination_mode)
                .await?;

            if iter == target_iteration && target_checkpoint == RefinementCheckpoint::Recoordinated
            {
                return Ok(());
            }
        }

        Err(AvengerChartError::InternalError(format!(
            "Requested refinement snapshot {:?} at iteration {} was not reached ({})",
            target_checkpoint, target_iteration, trace_label
        )))
    }

    async fn realize_fixed_subplot_layout_after_coordination(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scale_provider: &dyn ScaleProvider,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        max_refinement_passes: usize,
    ) -> Result<(), AvengerChartError> {
        self.refine_measurement_after_coordination(
            measurement,
            eval_ctx,
            layout_spec,
            scale_provider,
            data_override,
            facet_path,
            FacetCoordinationMode::FixedFullCycle,
            false,
            max_refinement_passes,
            "fixed-subplot layout realization candidate",
        )
        .await?;

        // Final guardrail: derive required canvas from the rendered scene envelope.
        // This catches nested facet subtree extents that are difficult to infer from
        // top-level overflow summaries alone.
        self.expand_fixed_subplot_canvas_from_rendered_envelope(
            measurement,
            eval_ctx,
            data_override,
            facet_path,
        )
        .await?;

        debug_assert!(
            Self::legends_within_canvas_recursive(measurement),
            "fixed-subplot realization invariant: legends must be within canvas after final envelope expansion"
        );
        Ok(())
    }

    async fn expand_fixed_subplot_canvas_from_rendered_envelope(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
    ) -> Result<(), AvengerChartError> {
        let components = self
            .build_plot_components(
                eval_ctx,
                measurement,
                data_override,
                !facet_path.is_empty(),
                facet_path,
            )
            .await?;

        let data_marks_group = SceneGroup {
            origin: [components.plot_bounds.x, components.plot_bounds.y],
            marks: components.data_marks,
            clip: components.clip,
            zindex: Some(0),
            ..Default::default()
        };

        let mut all_marks = Vec::new();
        all_marks.push(SceneMark::Group(data_marks_group));
        all_marks.extend(components.guide_marks);
        all_marks.extend(components.legend_marks);
        all_marks.extend(components.title_marks);
        all_marks.extend(components.subtitle_marks);
        all_marks.extend(components.debug_marks);

        let scene_graph = SceneGraph {
            marks: vec![SceneMark::Group(SceneGroup {
                marks: all_marks,
                ..Default::default()
            })],
            width: components.size.0,
            height: components.size.1,
            origin: [0.0, 0.0],
        };
        let envelope = *SceneGraphRTree::from_scene_graph(&scene_graph).envelope();
        let required_width = envelope.upper()[0].max(measurement.canvas_size.0).max(1.0);
        let required_height = envelope.upper()[1].max(measurement.canvas_size.1).max(1.0);
        if required_width > measurement.canvas_size.0 || required_height > measurement.canvas_size.1
        {
            trace!(
                required_width,
                required_height,
                old_canvas_width = measurement.canvas_size.0,
                old_canvas_height = measurement.canvas_size.1,
                "fixed-subplot canvas expanded from rendered scene envelope"
            );
            measurement.canvas_size = (required_width, required_height);
            measurement.layout.canvas_size = measurement.canvas_size;
        }
        Ok(())
    }

    /// Extract dimensions and determine layout mode from evaluated layout spec.
    ///
    /// Returns (width, height, is_plot_area_mode) where:
    /// - Plot area mode (`is_plot_area_mode=true`): canvas Auto + plot_area Fixed
    /// - Canvas mode (`is_plot_area_mode=false`): canvas specified, compute plot area later
    fn resolve_dimensions_from_spec(layout_spec: &EvaluatedLayoutSpec) -> (f32, f32, bool) {
        const DEFAULT_WIDTH: f32 = 400.0;
        const DEFAULT_HEIGHT: f32 = 300.0;

        // Determine if this is plot area mode (canvas Auto + plot_area Fixed)
        // vs canvas mode (canvas specified, plot_area may or may not be)
        let is_plot_area_mode = matches!(
            (&layout_spec.canvas, &layout_spec.plot_area),
            (EvaluatedSizeMode::Auto, EvaluatedSizeMode::Fixed { .. })
        );

        let (width, height) = if is_plot_area_mode {
            // Plot area mode: use plot_area dimensions
            match &layout_spec.plot_area {
                EvaluatedSizeMode::Fixed { width, height } => (*width, *height),
                _ => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
            }
        } else {
            // Canvas mode: use canvas dimensions (or defaults)
            match &layout_spec.canvas {
                EvaluatedSizeMode::Fixed { width, height } => (*width, *height),
                EvaluatedSizeMode::Width(w) => (*w, DEFAULT_HEIGHT),
                EvaluatedSizeMode::Height(h) => (DEFAULT_WIDTH, *h),
                EvaluatedSizeMode::Auto => {
                    // Canvas auto but not plot_area fixed - use plot_area or defaults
                    match &layout_spec.plot_area {
                        EvaluatedSizeMode::Fixed { width, height } => (*width, *height),
                        EvaluatedSizeMode::Width(w) => (*w, DEFAULT_HEIGHT),
                        EvaluatedSizeMode::Height(h) => (DEFAULT_WIDTH, *h),
                        EvaluatedSizeMode::Auto => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
                    }
                }
            }
        };

        (width, height, is_plot_area_mode)
    }

    /// Determine clip region from guide or default to plot area rect.
    fn get_clip_region(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> Clip {
        if let Some(ref guide) = self.compiled_guide {
            let configured_scales: HashMap<String, ConfiguredScale> = scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();
            guide.get_clip(plot_area_width, plot_area_height, &configured_scales)
        } else {
            Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: plot_area_width,
                height: plot_area_height,
            }
        }
    }

    pub(crate) fn resolved_clip_region(
        &self,
        eval_ctx: &EvaluationContext,
        facet_path: &[ScalarValue],
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> Clip {
        if facet_path.is_empty()
            && matches!(
                eval_ctx.facet_runtime_sizing_mode(),
                FacetRuntimeSizingMode::FixedSubplot { .. }
            )
            && Self::marks_contain_facet(&self.marks)
        {
            // Fixed-subplot top-level facet content can legitimately extend past the
            // synthesized root plot-area rectangle; avoid clipping at the root.
            Clip::None
        } else {
            self.get_clip_region(scales, plot_area_width, plot_area_height)
        }
    }

    /// Compute layout and determine plot area dimensions.
    ///
    /// Handles two modes:
    /// - **Plot area mode** (`is_plot_area_mode=true`): Uses provided dimensions as plot area,
    ///   computes layout to determine canvas size
    /// - **Canvas mode** (`is_plot_area_mode=false`): Uses provided dimensions as canvas,
    ///   computes layout to determine plot area from overflow
    ///
    /// Returns (plot_area_width, plot_area_height, canvas_size, layout, legend_plan)
    async fn compute_layout_and_dimensions(
        &self,
        is_plot_area_mode: bool,
        width: f32,
        height: f32,
        layout_spec: &EvaluatedLayoutSpec,
        scale_provider: &dyn ScaleProvider,
        ctx: &SessionContext,
        merged_params: &IndexMap<String, ScalarValue>,
        data_override: Option<&DataFrame>,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
    ) -> Result<(f32, f32, (f32, f32), LayoutSolution, PreparedLegendPlan), AvengerChartError> {
        if is_plot_area_mode {
            // Plot area mode: dimensions specify the plot area size
            let plot_area_width = width;
            let plot_area_height = height;

            let initial_scales = scale_provider
                .build_scales(plot_area_width, plot_area_height, ctx, merged_params)
                .await?;

            let (layout, legend_plan) = self
                .compute_layout_with_spec(
                    layout_spec,
                    &initial_scales,
                    ctx,
                    merged_params,
                    data_override,
                    facet_tree,
                    facet_path,
                )
                .await?;

            Ok((
                plot_area_width,
                plot_area_height,
                layout.canvas_size,
                layout,
                legend_plan,
            ))
        } else {
            // Canvas mode: dimensions are canvas size, compute layout to determine plot area
            let initial_plot_width = width * Self::INITIAL_PLOT_AREA_RATIO;
            let initial_plot_height = height * Self::INITIAL_PLOT_AREA_RATIO;

            let initial_scales = scale_provider
                .build_scales(initial_plot_width, initial_plot_height, ctx, merged_params)
                .await?;

            let (layout, legend_plan) = self
                .compute_layout_with_spec(
                    layout_spec,
                    &initial_scales,
                    ctx,
                    merged_params,
                    data_override,
                    facet_tree,
                    facet_path,
                )
                .await?;

            let plot_bounds = layout.plot_area_bounds();
            Ok((
                plot_bounds.width,
                plot_bounds.height,
                layout.canvas_size,
                layout,
                legend_plan,
            ))
        }
    }

    /// Measure coordinate system layout (e.g., facet cell positioning).
    ///
    /// This allows coordinate systems to compute layout data that's available
    /// to both guides and marks during rendering.
    ///
    /// Note: The caller is responsible for calling `apply_scale_adjustments()`
    /// on the returned measurement to update scales with coord-derived values.
    async fn measure_coord_system(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_area_width: f32,
        plot_area_height: f32,
        params_with_dims: &EvaluationContext,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        ctx: &SessionContext,
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        // Get data for coord measurement: use data_override if provided (nested facets),
        // otherwise use the plot's own data (top-level).
        let plot_data = if data_override.is_some() {
            None
        } else {
            self.data.as_ref().and_then(|node| {
                node.to_logical_plan(ctx)
                    .ok()
                    .map(|plan| DataFrame::new(ctx.state().clone(), plan))
            })
        };
        let coord_data = data_override.or(plot_data.as_ref());

        self.coord_transform
            .measure(
                scales,
                plot_area_width,
                plot_area_height,
                params_with_dims,
                coord_data,
                &self.marks,
                facet_path,
            )
            .await
    }

    /// Measure plot components without rendering (for layout coordination).
    ///
    /// This method performs all setup and measurement needed for layout coordination:
    /// - Resolves plot area dimensions from layout_spec (see Layout Modes below)
    /// - Builds scales with the provided scale_provider
    /// - Calls coordinate system measure (for facet cell layout)
    /// - Computes overflow requirements for guide elements
    ///
    /// Returns `ComponentsMeasurement` for parent layout coordination and rendering.
    ///
    /// # Layout Modes
    /// The `layout_spec` determines how dimensions are resolved:
    /// - **Canvas mode** (`canvas: Fixed`, `plot_area: Auto`): Plot area is computed
    ///   by subtracting legend/title overflow from canvas dimensions
    /// - **Plot area mode** (`canvas: Auto`, `plot_area: Fixed`): Plot area dimensions
    ///   are used directly; facet subplots always use this mode
    ///
    /// # Arguments
    /// * `eval_ctx` - Evaluation context with theme, session, params, and facet tree
    /// * `layout_spec` - Evaluated layout specification determining canvas vs plot area mode
    /// * `scale_provider` - Provider for building scales (prebuilt for shared scales, or from-data)
    /// * `data_override` - Optional data override for faceted subplots (filtered data)
    /// * `facet_path` - Path of values identifying current cell in facet hierarchy (e.g., `["East", "Eng"]`).
    ///   Used for tree navigation, data filtering, and axis visibility checks.
    pub(crate) async fn measure_plot_components(
        &self,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scale_provider: &dyn ScaleProvider,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
    ) -> Result<ComponentsMeasurement, AvengerChartError> {
        eval_ctx.record_plot_component_measure_call(facet_path.len());

        let ctx = &*eval_ctx.session_context;
        let facet_tree = &*eval_ctx.facet_tree;

        // Phase 1: Extract dimensions from layout spec
        let (width, height, is_plot_area_mode) = Self::resolve_dimensions_from_spec(layout_spec);

        debug!(
            width,
            height, is_plot_area_mode, "measure_plot_components dimensions"
        );

        // Add dimensions to params for media queries
        let params_with_dims = eval_ctx.with_dimension_params(width, height);
        let merged_params = params_with_dims.params.clone();

        // Phase 2: Compute layout and determine plot area dimensions
        let (plot_area_width, plot_area_height, mut canvas_size, mut layout, mut legend_plan) =
            self.compute_layout_and_dimensions(
                is_plot_area_mode,
                width,
                height,
                layout_spec,
                scale_provider,
                ctx,
                &merged_params,
                data_override,
                facet_tree,
                facet_path,
            )
            .await?;

        // Phase 3: Build final scales with actual plot area dimensions
        let mut final_scales = scale_provider
            .build_scales(plot_area_width, plot_area_height, ctx, &merged_params)
            .await?;

        // Phase 4: Coordinate system measurement
        let coord_measurement = self
            .measure_coord_system(
                &final_scales,
                plot_area_width,
                plot_area_height,
                &params_with_dims,
                data_override,
                facet_path,
                ctx,
            )
            .await?;

        // Apply scale adjustments from coord measurement (stays in main function
        // because it mutates final_scales which is used by later phases)
        coord_measurement.apply_scale_adjustments(&mut final_scales);

        if is_plot_area_mode {
            let initial_plot_bounds = *layout.plot_area_bounds();
            let initial_overflow = layout.overflow;
            let initial_total_overflow = layout.total_overflow;
            let (_, refined_layout, refined_legend_plan) = self
                .rebuild_layout_with_coord_overflow(
                    layout_spec,
                    &final_scales,
                    plot_area_width,
                    plot_area_height,
                    &merged_params,
                    data_override,
                    ctx,
                    facet_tree,
                    facet_path,
                    Some(coord_measurement.as_ref()),
                )
                .await?;
            let refined_plot_bounds = refined_layout.plot_area_bounds();
            trace!(
                initial_width = initial_plot_bounds.width,
                initial_height = initial_plot_bounds.height,
                refined_width = refined_plot_bounds.width,
                refined_height = refined_plot_bounds.height,
                delta_w = (refined_plot_bounds.width - initial_plot_bounds.width).abs(),
                delta_h = (refined_plot_bounds.height - initial_plot_bounds.height).abs(),
                initial_overflow_top = initial_overflow.top,
                initial_overflow_right = initial_overflow.right,
                initial_overflow_bottom = initial_overflow.bottom,
                initial_overflow_left = initial_overflow.left,
                refined_overflow_top = refined_layout.overflow.top,
                refined_overflow_right = refined_layout.overflow.right,
                refined_overflow_bottom = refined_layout.overflow.bottom,
                refined_overflow_left = refined_layout.overflow.left,
                initial_total_overflow_top = initial_total_overflow.top,
                initial_total_overflow_right = initial_total_overflow.right,
                initial_total_overflow_bottom = initial_total_overflow.bottom,
                initial_total_overflow_left = initial_total_overflow.left,
                refined_total_overflow_top = refined_layout.total_overflow.top,
                refined_total_overflow_right = refined_layout.total_overflow.right,
                refined_total_overflow_bottom = refined_layout.total_overflow.bottom,
                refined_total_overflow_left = refined_layout.total_overflow.left,
                "plot-area measurement refined with coord-aware overflow"
            );
            layout = refined_layout;
            legend_plan = refined_legend_plan;
            canvas_size = layout.canvas_size;
        }

        // Phase 5: Get clip region from guide
        let clip = self.resolved_clip_region(
            eval_ctx,
            facet_path,
            &final_scales,
            plot_area_width,
            plot_area_height,
        );

        // Note: Overflow info is available via layout.overflow (guide only) and
        // layout.total_overflow (guide + legends), computed during layout phase.

        Ok(ComponentsMeasurement {
            coord_measurement,
            scales: final_scales,
            plot_area_width,
            plot_area_height,
            canvas_size,
            clip,
            layout,
            params: merged_params,
            legend_plan,
        })
    }

    /// Build plot components with explicit dimensions and scale provider (recursive entry point)
    ///
    /// This method supports both top-level plots and subplots by accepting:
    /// - Explicit dimensions (canvas size or plot area size, controlled by `dimensions_are_plot_area`)
    /// - A scale provider (build new scales or use shared scales from parent)
    /// - Evaluation mode (measure overflow or full render)
    /// - Optional data override (for faceted subplots)
    ///
    /// When `dimensions_are_plot_area` is false (canvas mode), the dimensions represent the full
    /// canvas and layout is computed to determine the plot area. When true (plot area mode), the
    /// dimensions represent the already-determined plot area size.
    ///
    /// This enables true recursive rendering where the same logic works at all nesting levels.
    /// Build plot components using pre-computed measurement
    ///
    /// This method renders all marks using the provided measurement results.
    /// Call `measure_plot_components()` first to get the measurement.
    ///
    /// # Arguments
    /// * `eval_ctx` - Evaluation context
    /// * `measurement` - Pre-computed measurement from `measure_plot_components()`
    /// * `data_override` - Optional data override for faceted subplots
    /// * `dimensions_are_plot_area` - If true, dimensions are plot area; if false, canvas
    /// * `facet_path` - Current cell path in facet hierarchy as values (for axis visibility).
    ///   Empty slice when not in a facet cell.
    pub async fn build_plot_components(
        &self,
        eval_ctx: &EvaluationContext,
        measurement: &ComponentsMeasurement,
        data_override: Option<&DataFrame>,
        dimensions_are_plot_area: bool,
        facet_path: &[ScalarValue],
    ) -> Result<PlotComponents, AvengerChartError> {
        let ctx = &*eval_ctx.session_context;

        debug!(
            plot_area_width = measurement.plot_area_width,
            plot_area_height = measurement.plot_area_height,
            dimensions_are_plot_area,
            "build_plot_components start"
        );

        // Render components using measurement
        let layout_solution = measurement.layout.clone();

        // Extract values from measurement for convenience
        let plot_area_width = measurement.plot_area_width;
        let plot_area_height = measurement.plot_area_height;
        let canvas_size = measurement.canvas_size;
        let clip = measurement.clip.clone();
        let merged_params = measurement.params.clone();
        let merged_scales = measurement.scales.clone();
        let legend_plan_initial = measurement.legend_plan.clone();

        // Create context with measurement's params (which include dimensions)
        let mark_eval_ctx = eval_ctx.with_params(merged_params.clone());

        // Render marks using pre-computed measurements from measurement
        let coord_measurement_ref: &dyn CoordMeasurement = measurement.coord_measurement.as_ref();

        let mut data_marks = Vec::new();
        for mark in &self.marks {
            let marks = self
                .render_mark_with_plot_df(
                    mark.as_ref(),
                    &mark_eval_ctx,
                    &merged_scales,
                    plot_area_width,
                    plot_area_height,
                    data_override,
                    facet_path,
                    coord_measurement_ref,
                )
                .await?;
            data_marks.extend(marks);
        }

        // Create guide marks and other components
        let (
            plot_bounds_struct,
            guide_marks,
            legend_marks,
            title_marks,
            subtitle_marks,
            debug_marks,
        ) = {
            let layout_initial = layout_solution;
            // Check if this is a top-level plot (canvas mode) or subplot (plot area mode)
            if !dimensions_are_plot_area {
                // Canvas mode: measurement already carries the finalized layout/overflow.
                let plot_bounds = layout_initial.plot_area_bounds();
                let plot_bounds_struct = LayoutBounds {
                    x: plot_bounds.x,
                    y: plot_bounds.y,
                    width: plot_area_width,
                    height: plot_area_height,
                };

                let guide_marks = self
                    .create_guide_marks(
                        &merged_scales,
                        plot_area_width,
                        plot_area_height,
                        &plot_bounds_struct,
                        &layout_initial.overflow,
                        &merged_params,
                        ctx,
                        data_override,
                        eval_ctx.facet_tree.as_ref(),
                        facet_path,
                        coord_measurement_ref,
                    )
                    .await?;

                let legend_marks = self
                    .render_legends_from_plan(
                        &legend_plan_initial,
                        &layout_initial.taffy_layout,
                        ctx,
                        &merged_params,
                    )
                    .await?;

                let title_marks = if let Some(title_bounds) = &layout_initial.taffy_layout.title {
                    self.create_title(Some(*title_bounds), ctx, &merged_params)
                        .await?
                } else {
                    Vec::new()
                };

                let subtitle_marks =
                    if let Some(subtitle_bounds) = &layout_initial.taffy_layout.subtitle {
                        self.create_subtitle(Some(*subtitle_bounds), ctx, &merged_params)
                            .await?
                    } else {
                        Vec::new()
                    };

                let mut debug_marks = vec![];
                if eval_ctx.debug_layout_lines_enabled() {
                    debug_marks.extend(create_debug_layout_rects(
                        &layout_initial.taffy_layout,
                        None,
                        None,
                        None,
                        false,
                    ));
                }

                (
                    plot_bounds_struct,
                    guide_marks,
                    legend_marks,
                    title_marks,
                    subtitle_marks,
                    debug_marks,
                )
            } else {
                // Plot area mode (subplots): plot_bounds.y = 0
                // For nested facets, the FacetColGuide computes adjusted_plot_bounds.y as
                // a NEGATIVE value, placing labels ABOVE the subplot origin (in the overflow
                // region). Data marks render at y = 0 to plot_height.
                // The subplot's overflow region is at negative y, not positive.
                let plot_bounds_struct = LayoutBounds {
                    x: 0.0,
                    y: 0.0,
                    width: plot_area_width,
                    height: plot_area_height,
                };

                // Create guide marks
                // Pass data_override so nested facets use filtered data
                let guide_marks = self
                    .create_guide_marks(
                        &merged_scales,
                        plot_area_width,
                        plot_area_height,
                        &plot_bounds_struct,
                        &layout_initial.overflow,
                        &merged_params,
                        ctx,
                        data_override,
                        eval_ctx.facet_tree.as_ref(),
                        facet_path,
                        coord_measurement_ref,
                    )
                    .await?;

                // Create legend marks from the computed layout
                // Legend positions from layout include the plot area offset, but we need them at (0,0)
                let plot_bounds = layout_initial.plot_area_bounds();
                let legend_marks_raw = self
                    .render_legends_from_plan(
                        &legend_plan_initial,
                        &layout_initial.taffy_layout,
                        ctx,
                        &merged_params,
                    )
                    .await?;

                // Translate legend marks to be relative to (0, 0) instead of plot area offset
                let legend_marks: Vec<_> = legend_marks_raw
                    .into_iter()
                    .map(|mark| {
                        match mark {
                            SceneMark::Group(mut group) => {
                                // Adjust group origin by subtracting plot area offset
                                group.origin = [
                                    group.origin[0] - plot_bounds.x,
                                    group.origin[1] - plot_bounds.y,
                                ];
                                SceneMark::Group(group)
                            }
                            _ => mark, // Other mark types shouldn't be at this level
                        }
                    })
                    .collect();

                // Create title/subtitle marks if they exist in layout
                let title_marks = vec![];
                let subtitle_marks = vec![];
                // (Subplots typically don't have titles, but the layout might include them)

                let mut debug_marks = vec![];
                if eval_ctx.debug_layout_lines_enabled() {
                    // Use the actual computed layout which includes legends
                    // The layout has plot area at an offset due to overflow/legends
                    // We need to translate it to (0,0) for subplot coordinates
                    let plot_bounds = layout_initial.plot_area_bounds();

                    // Create a translated copy of the layout with plot area at (0,0)
                    let mut subplot_layout = layout_initial.taffy_layout.clone();

                    // Translate plot_area
                    subplot_layout.plot_area.x -= plot_bounds.x;
                    subplot_layout.plot_area.y -= plot_bounds.y;

                    // Translate guide_overflows
                    for (_, bounds) in subplot_layout.guide_overflows.iter_mut() {
                        bounds.x -= plot_bounds.x;
                        bounds.y -= plot_bounds.y;
                    }

                    // Translate legends
                    for (_, bounds) in subplot_layout.legends.iter_mut() {
                        bounds.x -= plot_bounds.x;
                        bounds.y -= plot_bounds.y;
                    }

                    // Compute unique color for each subplot using a simple hash
                    // of params to differentiate subplots without FacetContext
                    let subplot_color_string = {
                        // Use a simple hash based on params count for deterministic coloring
                        let mut hasher = DefaultHasher::new();
                        merged_params.len().hash(&mut hasher);
                        // Include some param values for more variation
                        for (key, _) in merged_params.iter().take(3) {
                            key.hash(&mut hasher);
                        }
                        let hash = hasher.finish();
                        let index = (hash % 6) as usize;

                        let colors = [
                            "hsla(15, 65%, 60%, 0.8)",  // Orange-red
                            "hsla(75, 65%, 60%, 0.8)",  // Yellow-green
                            "hsla(135, 65%, 60%, 0.8)", // Green
                            "hsla(195, 65%, 60%, 0.8)", // Cyan
                            "hsla(255, 65%, 60%, 0.8)", // Blue-purple
                            "hsla(315, 65%, 60%, 0.8)", // Magenta
                        ];

                        colors[index].to_string()
                    };

                    debug_marks.extend(create_debug_layout_rects(
                        &subplot_layout,
                        Some(subplot_color_string),
                        Some(1.0), // Same width as outer lines
                        Some(100), // Higher z-index to render on top
                        true,      // Flip label alignment to avoid overlap with outer plot labels
                    ));
                }

                (
                    plot_bounds_struct,
                    guide_marks,
                    legend_marks,
                    title_marks,
                    subtitle_marks,
                    debug_marks,
                )
            }
        };

        // Debug marks are kept separate - they're in absolute canvas coordinates
        // and should not be translated with the data marks group

        Ok(PlotComponents {
            data_marks,
            guide_marks,
            legend_marks,
            title_marks,
            subtitle_marks,
            plot_bounds: plot_bounds_struct,
            clip,
            size: canvas_size,
            size_is_canvas: !dimensions_are_plot_area,
            debug_marks,
        })
    }

    pub(crate) fn components_to_evaluated_plot(
        &self,
        eval_ctx: &EvaluationContext,
        components: PlotComponents,
    ) -> EvaluatedPlot {
        let plot_bounds = components.plot_bounds;
        let (final_width, final_height) = components.size;

        let data_marks_group = SceneGroup {
            origin: [plot_bounds.x, plot_bounds.y],
            marks: components.data_marks,
            clip: components.clip,
            zindex: Some(0),
            ..Default::default()
        };

        let mut all_marks = Vec::new();

        let theme = self.get_theme();
        let canvas_ctx = ThemeContext::new("canvas", eval_ctx.params.clone());
        if let Some(color) = theme
            .query(&canvas_ctx, "background-color")
            .and_then(|v| v.as_color_array())
        {
            let background_rect = SceneRectMark {
                x: 0.0.into(),
                y: 0.0.into(),
                width: Some(final_width.into()),
                height: Some(final_height.into()),
                fill: ColorOrGradient::Color(color).into(),
                stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
                stroke_width: 0.0.into(),
                zindex: Some(-100),
                ..Default::default()
            };
            all_marks.push(SceneMark::Rect(background_rect));
        }

        all_marks.push(SceneMark::Group(data_marks_group));
        all_marks.extend(components.guide_marks);
        all_marks.extend(components.legend_marks);
        all_marks.extend(components.title_marks);
        all_marks.extend(components.subtitle_marks);
        all_marks.extend(components.debug_marks);

        let root_group = SceneGroup {
            marks: all_marks,
            ..Default::default()
        };

        let scene_graph = SceneGraph {
            marks: vec![SceneMark::Group(root_group)],
            width: final_width,
            height: final_height,
            origin: [0.0, 0.0],
        };

        let rtree = SceneGraphRTree::from_scene_graph(&scene_graph);

        EvaluatedPlot {
            scene_graph,
            rtree: Some(rtree),
        }
    }

    fn select_facet_subtree_by_facet_path<'a>(
        &'a self,
        measurement: &'a ComponentsMeasurement,
        data_override: Option<&'a DataFrame>,
        remaining_path: &[ScalarValue],
        current_path: Vec<ScalarValue>,
        dimensions_are_plot_area: bool,
    ) -> Result<SelectedFacetSubtree<'a>, AvengerChartError> {
        if remaining_path.is_empty() {
            return Ok(SelectedFacetSubtree {
                plot: self,
                measurement,
                data_override,
                facet_path: current_path,
                dimensions_are_plot_area,
            });
        }

        let Some((compiled_subplot, cells)) = facet_band_children(measurement) else {
            return Err(AvengerChartError::InternalError(format!(
                "Facet subtree path {:?} continues through a non-facet measurement",
                remaining_path
            )));
        };

        let target = &remaining_path[0];
        let cell = cells
            .iter()
            .find(|cell| &cell.plan.value == target)
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Facet subtree path value {:?} not found",
                    target
                ))
            })?;

        compiled_subplot.select_facet_subtree_by_facet_path(
            &cell.measurement,
            Some(&cell.data_override),
            &remaining_path[1..],
            cell.plan.full_path.clone(),
            true,
        )
    }

    fn select_facet_subtree_by_coord_node_path<'a>(
        &'a self,
        measurement: &'a ComponentsMeasurement,
        data_override: Option<&'a DataFrame>,
        remaining_path: &[usize],
        current_path: Vec<ScalarValue>,
        dimensions_are_plot_area: bool,
    ) -> Result<SelectedFacetSubtree<'a>, AvengerChartError> {
        if remaining_path.is_empty() {
            return Ok(SelectedFacetSubtree {
                plot: self,
                measurement,
                data_override,
                facet_path: current_path,
                dimensions_are_plot_area,
            });
        }

        let Some((compiled_subplot, cells)) = facet_band_children(measurement) else {
            return Err(AvengerChartError::InternalError(format!(
                "Facet subtree coord-node path {:?} continues through a non-facet measurement",
                remaining_path
            )));
        };

        let child_index = remaining_path[0];
        let cell = cells.get(child_index).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Facet subtree coord-node child index {} out of range",
                child_index
            ))
        })?;

        compiled_subplot.select_facet_subtree_by_coord_node_path(
            &cell.measurement,
            Some(&cell.data_override),
            &remaining_path[1..],
            cell.plan.full_path.clone(),
            true,
        )
    }

    async fn render_facet_subtree_snapshot(
        &self,
        eval_ctx: &EvaluationContext,
        measurement: &ComponentsMeasurement,
        snapshot: &FacetSubtreeSnapshot,
    ) -> Result<EvaluatedPlot, AvengerChartError> {
        let selected = match &snapshot.selector {
            FacetSubtreeSelector::ByFacetPath(path) => {
                self.select_facet_subtree_by_facet_path(measurement, None, path, Vec::new(), false)?
            }
            FacetSubtreeSelector::ByCoordNodePath(path) => self
                .select_facet_subtree_by_coord_node_path(
                    measurement,
                    None,
                    path,
                    Vec::new(),
                    false,
                )?,
        };

        let components = selected
            .plot
            .build_plot_components(
                eval_ctx,
                selected.measurement,
                selected.data_override,
                selected.dimensions_are_plot_area,
                &selected.facet_path,
            )
            .await?;
        let evaluated = selected
            .plot
            .components_to_evaluated_plot(eval_ctx, components);
        Ok(Self::pad_facet_subtree_snapshot(evaluated))
    }

    fn pad_facet_subtree_snapshot(evaluated: EvaluatedPlot) -> EvaluatedPlot {
        let original_scene = evaluated.scene_graph;
        let original_width = original_scene.width.max(1.0);
        let original_height = original_scene.height.max(1.0);
        let envelope = evaluated
            .rtree
            .as_ref()
            .map(|rtree| *rtree.envelope())
            .unwrap_or_else(|| *SceneGraphRTree::from_scene_graph(&original_scene).envelope());

        let min_x = envelope.lower()[0].min(0.0);
        let min_y = envelope.lower()[1].min(0.0);
        let max_x = envelope.upper()[0].max(original_width);
        let max_y = envelope.upper()[1].max(original_height);
        let padding = Self::FACET_SUBTREE_SNAPSHOT_PADDING;
        let shift_x = padding - min_x;
        let shift_y = padding - min_y;
        let final_width = (max_x - min_x + 2.0 * padding).ceil().max(1.0);
        let final_height = (max_y - min_y + 2.0 * padding).ceil().max(1.0);

        let content_group = SceneGroup {
            name: "facet_subtree_snapshot_content".to_string(),
            origin: [shift_x, shift_y],
            marks: original_scene.marks,
            ..Default::default()
        };

        let scene_graph = SceneGraph {
            marks: vec![SceneMark::Group(SceneGroup {
                marks: vec![SceneMark::Group(content_group)],
                ..Default::default()
            })],
            width: final_width,
            height: final_height,
            origin: [0.0, 0.0],
        };
        let rtree = SceneGraphRTree::from_scene_graph(&scene_graph);
        EvaluatedPlot {
            scene_graph,
            rtree: Some(rtree),
        }
    }

    async fn apply_layout_snapshot(
        &self,
        snapshot: &LayoutSnapshot,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        provider: &dyn ScaleProvider,
        facet_sizing_strategy: FacetSizingStrategy,
        coordination_mode: FacetCoordinationMode,
    ) -> Result<(), AvengerChartError> {
        match snapshot {
            LayoutSnapshot::Final => {
                self.apply_final_layout_snapshot(
                    measurement,
                    eval_ctx,
                    evaluated_layout_spec,
                    provider,
                    facet_sizing_strategy,
                    coordination_mode,
                )
                .await
            }
            LayoutSnapshot::Whole(WholeChartSnapshot::LocalMeasured) => Ok(()),
            LayoutSnapshot::Whole(WholeChartSnapshot::Coordination(checkpoint)) => {
                coordinate_overflow_for_guides_with_mode_until(
                    measurement,
                    eval_ctx,
                    coordination_mode,
                    *checkpoint,
                )
                .await
            }
            LayoutSnapshot::Whole(WholeChartSnapshot::Refinement {
                iteration,
                checkpoint,
            }) => {
                coordinate_overflow_for_guides_with_mode_until(
                    measurement,
                    eval_ctx,
                    coordination_mode,
                    CoordinationCheckpoint::FinalPropagationComplete,
                )
                .await?;
                self.apply_refinement_snapshot(
                    measurement,
                    eval_ctx,
                    evaluated_layout_spec,
                    provider,
                    facet_sizing_strategy,
                    coordination_mode,
                    *iteration,
                    *checkpoint,
                )
                .await
            }
            LayoutSnapshot::FacetSubtree(_) => Ok(()),
        }
    }

    async fn apply_final_layout_snapshot(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        provider: &dyn ScaleProvider,
        facet_sizing_strategy: FacetSizingStrategy,
        coordination_mode: FacetCoordinationMode,
    ) -> Result<(), AvengerChartError> {
        // `coordinate_overflow_for_guides` currently maps to top-level phases 7-10:
        // - phase 7 attributes build (snapshot/aggregate/distribution) + executor apply,
        // - phase 8 executor apply/remeasure + immutable execution trace,
        // - phase 9 attributes build (post-remeasure reconcile distribution) + executor apply,
        // - phase 10 executor scale retarget/adjustment propagation + immutable trace.
        coordinate_overflow_for_guides_with_mode(measurement, eval_ctx, coordination_mode).await?;

        let (_, _, is_plot_area_mode) = Self::resolve_dimensions_from_spec(evaluated_layout_spec);
        match facet_sizing_strategy {
            FacetSizingStrategy::CanvasFit => {
                if !is_plot_area_mode {
                    let refinement = eval_ctx.facet_layout_refinement();
                    self.refine_canvas_measurement_after_coordination(
                        measurement,
                        eval_ctx,
                        evaluated_layout_spec,
                        provider,
                        None,
                        &[],
                        refinement.max_refinement_passes,
                    )
                    .await?;
                }
            }
            FacetSizingStrategy::FixedSubplot { .. } => {
                let refinement = eval_ctx.facet_layout_refinement();
                self.realize_fixed_subplot_layout_after_coordination(
                    measurement,
                    eval_ctx,
                    evaluated_layout_spec,
                    provider,
                    None,
                    &[],
                    refinement.max_refinement_passes,
                )
                .await?;
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_refinement_snapshot(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        provider: &dyn ScaleProvider,
        facet_sizing_strategy: FacetSizingStrategy,
        coordination_mode: FacetCoordinationMode,
        iteration: usize,
        checkpoint: RefinementCheckpoint,
    ) -> Result<(), AvengerChartError> {
        let refinement = eval_ctx.facet_layout_refinement();
        let (_, _, is_plot_area_mode) = Self::resolve_dimensions_from_spec(evaluated_layout_spec);
        match facet_sizing_strategy {
            FacetSizingStrategy::CanvasFit => {
                if is_plot_area_mode {
                    return Err(AvengerChartError::InternalError(
                        "Refinement snapshots are not available for plot-area mode canvas-fit layouts"
                            .to_string(),
                    ));
                }
                self.refine_canvas_measurement_after_coordination_until(
                    measurement,
                    eval_ctx,
                    evaluated_layout_spec,
                    provider,
                    None,
                    &[],
                    refinement.max_refinement_passes,
                    iteration,
                    checkpoint,
                )
                .await
            }
            FacetSizingStrategy::FixedSubplot { .. } => {
                self.refine_measurement_after_coordination_until(
                    measurement,
                    eval_ctx,
                    evaluated_layout_spec,
                    provider,
                    None,
                    &[],
                    coordination_mode,
                    false,
                    refinement.max_refinement_passes,
                    iteration,
                    checkpoint,
                    "fixed-subplot layout realization snapshot",
                )
                .await
            }
        }
    }

    /// Evaluate the plot to a scene graph
    pub async fn evaluate(
        &self,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
    ) -> Result<EvaluatedPlot, AvengerChartError> {
        self.evaluate_with_options(ctx, params, EvaluationOptions::default())
            .await
    }

    /// Evaluate the plot to a scene graph with explicit layout snapshot and debug options.
    pub async fn evaluate_with_options(
        &self,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        options: EvaluationOptions,
    ) -> Result<EvaluatedPlot, AvengerChartError> {
        self.evaluate_with_options_internal(ctx, params, options, None)
            .await
    }

    /// Evaluate the plot while collecting focused performance diagnostics.
    #[doc(hidden)]
    pub async fn evaluate_with_options_and_metrics(
        &self,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        options: EvaluationOptions,
    ) -> Result<(EvaluatedPlot, EvaluationMetrics), AvengerChartError> {
        let metrics = Arc::new(Mutex::new(EvaluationMetrics::default()));
        let evaluated = self
            .evaluate_with_options_internal(ctx, params, options, Some(metrics.clone()))
            .await?;
        let metrics = metrics
            .lock()
            .expect("evaluation metrics lock poisoned")
            .clone();
        Ok((evaluated, metrics))
    }

    async fn evaluate_with_options_internal(
        &self,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        options: EvaluationOptions,
        evaluation_metrics: Option<Arc<Mutex<EvaluationMetrics>>>,
    ) -> Result<EvaluatedPlot, AvengerChartError> {
        // Merge provided params with defaults
        let merged_params = if let Some(provided) = params {
            let mut merged = self.default_params.clone();
            merged.extend(provided);
            merged
        } else {
            self.default_params.clone()
        };

        // Build evaluated facet spec (pre-pass to discover partition structure)
        // This queries distinct values for each facet level, respecting scale sharing settings.
        // Used for efficient domain lookups in nested facet coordination.
        let facet_tree_start = Instant::now();
        let facet_tree = Arc::new(EvaluatedFacetTree::from_compiled_plot(self, ctx).await?);
        debug!(
            elapsed_ms = facet_tree_start.elapsed().as_secs_f64() * 1000.0,
            depth = facet_tree.depth(),
            "evaluated facet tree construction completed"
        );

        // Evaluate layout spec to get concrete dimensions
        let evaluated_layout_spec = evaluate_layout_spec(
            &self.layout_spec,
            ctx,
            &merged_params,
            self.get_theme().as_ref(),
        )
        .await?;
        let facet_sizing_strategy = self.resolve_facet_sizing_strategy(&evaluated_layout_spec)?;
        let measured_layout_spec = Self::layout_spec_for_facet_sizing_strategy(
            &evaluated_layout_spec,
            facet_tree.as_ref(),
            facet_sizing_strategy,
        );

        // Build scale provider
        let scale_builder = build_scale_builder_from_marks(
            &self.marks,
            &self.scale_specs,
            &self.coord_transform,
            &self.data,
            None,
            ctx,
            &merged_params,
            self.get_theme().as_ref(),
        )
        .await?;

        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: self,
        };

        // Create EvaluationContext for the entire evaluation
        let mut eval_ctx = EvaluationContext::new(
            self.get_theme(),
            Arc::new(ctx.clone()),
            merged_params,
            facet_tree.clone(),
        )
        .with_facet_runtime_sizing_mode(facet_sizing_strategy.runtime_sizing_mode())
        .with_facet_layout_refinement(options.facet_layout_refinement)
        .with_debug_layout_lines(facet_debug::resolve_layout_overlay_enabled(
            options.debug_layout_lines,
        ));
        if let LayoutSnapshot::FacetSubtree(snapshot) = &options.layout_snapshot
            && snapshot.checkpoint == FacetSubtreeCheckpoint::EstimatedOverflowProbe
        {
            eval_ctx = eval_ctx.with_facet_subtree_snapshot_capture(Arc::new(Mutex::new(
                FacetSubtreeSnapshotCapture {
                    request: snapshot.clone(),
                    result: None,
                },
            )));
        }
        if let Some(metrics) = evaluation_metrics {
            eval_ctx = eval_ctx.with_evaluation_metrics(metrics);
        }

        if Self::marks_use_auto_empty_cell_policy(&self.marks) {
            trace!("Facet empty-cell policy `auto` resolved to `hole` for this evaluation");
        }

        // Measure plot components
        let measurement = self
            .measure_plot_components(
                &eval_ctx,
                &measured_layout_spec,
                &provider,
                None, // No data override for top-level plots
                &[],  // Empty facet path for top-level plots
            )
            .await?;

        let mut measurement = measurement;
        if let LayoutSnapshot::FacetSubtree(snapshot) = &options.layout_snapshot {
            match snapshot.checkpoint {
                FacetSubtreeCheckpoint::EstimatedOverflowProbe => {
                    let evaluated = eval_ctx.take_facet_subtree_snapshot().ok_or_else(|| {
                        AvengerChartError::InternalError(format!(
                            "Requested estimated overflow probe snapshot was not captured: {:?}",
                            snapshot
                        ))
                    })?;
                    return Ok(Self::pad_facet_subtree_snapshot(evaluated));
                }
                FacetSubtreeCheckpoint::LocalRetargetedLayout => {}
                FacetSubtreeCheckpoint::CoordinatedLayout => {
                    self.apply_layout_snapshot(
                        &LayoutSnapshot::Whole(WholeChartSnapshot::Coordination(
                            CoordinationCheckpoint::FinalPropagationComplete,
                        )),
                        &mut measurement,
                        &eval_ctx,
                        &measured_layout_spec,
                        &provider,
                        facet_sizing_strategy,
                        facet_sizing_strategy.coordination_mode(),
                    )
                    .await?;
                }
                FacetSubtreeCheckpoint::FinalLayout => {
                    self.apply_layout_snapshot(
                        &LayoutSnapshot::Final,
                        &mut measurement,
                        &eval_ctx,
                        &measured_layout_spec,
                        &provider,
                        facet_sizing_strategy,
                        facet_sizing_strategy.coordination_mode(),
                    )
                    .await?;
                }
            }
            return self
                .render_facet_subtree_snapshot(&eval_ctx, &measurement, snapshot)
                .await;
        }

        self.apply_layout_snapshot(
            &options.layout_snapshot,
            &mut measurement,
            &eval_ctx,
            &measured_layout_spec,
            &provider,
            facet_sizing_strategy,
            facet_sizing_strategy.coordination_mode(),
        )
        .await?;

        // Build plot components using measurement
        let components = self
            .build_plot_components(
                &eval_ctx,
                &measurement,
                None,  // No data override for top-level plots
                false, // Canvas mode: dimensions are canvas size
                &[],   // Empty path for top-level plots (not in a facet cell)
            )
            .await?;

        Ok(self.components_to_evaluated_plot(&eval_ctx, components))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        coords::{CoordinatedOverflow, FacetAxis},
        facet::{
            band_positions::BandPositionIterator,
            coord::{FacetBandCoordMeasurement, facet_band_ref as facet_band_ref_from_coord},
            layout_slabs::LayoutSlabs,
        },
        layout::PlotConstraint,
        legend::LegendPosition,
        prelude::*,
        render::FacetLayoutRefinement,
    };
    use datafusion::{
        arrow::{
            array::{Float64Array, StringArray},
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        dataframe::DataFrame,
        prelude::SessionContext,
    };
    use std::future::Future;

    fn facet_band_ref(measurement: &ComponentsMeasurement) -> Option<&FacetBandCoordMeasurement> {
        facet_band_ref_from_coord(measurement.coord_measurement.as_ref())
    }

    fn run_with_large_stack<F, Fut>(f: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), AvengerChartError>> + Send + 'static,
    {
        std::thread::Builder::new()
            .name("rendering-test-large-stack".to_string())
            .stack_size(64 * 1024 * 1024)
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build tokio runtime for large-stack rendering test");
                runtime
                    .block_on(f())
                    .expect("large-stack rendering test future should succeed");
            })
            .expect("spawn large-stack rendering test thread")
            .join()
            .expect("join large-stack rendering test thread");
    }

    fn deeply_nested_dataframe(ctx: &SessionContext) -> DataFrame {
        let outer_groups = StringArray::from(vec![
            "G1", "G1", "G1", "G1", // S1: A,B; S2: B,C
            "G2", "G2", "G2", "G2", // S1: C,D; S2: A,D
        ]);
        let sub_groups = StringArray::from(vec![
            "S1", "S1", "S2", "S2", // G1
            "S1", "S1", "S2", "S2", // G2
        ]);
        let categories = StringArray::from(vec![
            "A", "B", "B", "C", // G1 (S1: A,B; S2: B,C)
            "C", "D", "A", "D", // G2 (S1: C,D; S2: A,D)
        ]);
        let values = Float64Array::from(vec![
            10.0, 20.0, 15.0, 25.0, // G1
            30.0, 40.0, 35.0, 45.0, // G2
        ]);

        let schema = Arc::new(Schema::new(vec![
            Field::new("outer_group", DataType::Utf8, false),
            Field::new("sub_group", DataType::Utf8, false),
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(outer_groups),
                Arc::new(sub_groups),
                Arc::new(categories),
                Arc::new(values),
            ],
        )
        .expect("create deeply nested categorical sharing batch");

        ctx.read_batch(batch)
            .expect("read deeply nested test batch")
    }

    fn build_deeply_nested_plot(df: DataFrame) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(700, 500)
            .mark(
                Facet::new()
                    .col_with(col("outer_group"), |c| c.facet(|f| f.title("Outer Group")))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("sub_group"), |c| c.facet(|f| f.title("Sub Group")))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Rect::new()
                                            .x_with(col("category"), |c| {
                                                c.scale_with::<Band>(|s| s)
                                                    .with_scale_sharing(ScaleSharing::Shared)
                                                    .axis(|a| a.title("Category"))
                                            })
                                            .x2_with(col(":x"), |c| c.band(1.0))
                                            .y(0.0)
                                            .y2_with(col("value"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Shared)
                                                    .axis(|a| a.title("Value"))
                                            })
                                            .fill("#4682b4"),
                                    ),
                                ),
                        ),
                    ),
            )
    }

    fn build_simple_facet_col_plot(df: DataFrame) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new().data(df).mark(
            Facet::new().column(col("outer_group")).subplot(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("value"))
                        .y(col("value"))
                        .size(24.0)
                        .fill("#4682b4"),
                ),
            ),
        )
    }

    async fn legend_sharing_dataframe(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "CREATE TABLE legend_sharing AS VALUES
            ('DivA', 'Dept1', 1.0, 1.0, 'Low'),
            ('DivA', 'Dept1', 1.4, 1.3, 'High'),
            ('DivA', 'Dept2', 2.0, 1.1, 'Low'),
            ('DivA', 'Dept2', 2.3, 1.4, 'High'),
            ('DivB', 'Dept1', 3.0, 1.0, 'Low'),
            ('DivB', 'Dept1', 3.4, 1.3, 'High'),
            ('DivB', 'Dept2', 4.0, 1.1, 'Low'),
            ('DivB', 'Dept2', 4.3, 1.4, 'High')",
        )
        .await
        .expect("create legend sharing test data");

        ctx.sql(
            "SELECT
                column1 AS division,
                column2 AS department,
                column3 AS x_val,
                column4 AS y_val,
                column5 AS category
             FROM legend_sharing",
        )
        .await
        .expect("read legend sharing test data")
    }

    async fn legend_sharing_three_level_dataframe(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "CREATE TABLE legend_sharing_three_level AS VALUES
            ('DivA', 'Dept1', 'Team1', 1.0, 1.0, 'Low'),
            ('DivA', 'Dept1', 'Team1', 1.4, 1.3, 'High'),
            ('DivA', 'Dept1', 'Team2', 2.0, 1.1, 'Low'),
            ('DivA', 'Dept1', 'Team2', 2.3, 1.4, 'High'),
            ('DivA', 'Dept2', 'Team1', 1.1, 2.0, 'Low'),
            ('DivA', 'Dept2', 'Team1', 1.5, 2.3, 'High'),
            ('DivA', 'Dept2', 'Team2', 2.1, 2.1, 'Low'),
            ('DivA', 'Dept2', 'Team2', 2.4, 2.4, 'High'),
            ('DivB', 'Dept1', 'Team1', 3.0, 1.0, 'Low'),
            ('DivB', 'Dept1', 'Team1', 3.4, 1.3, 'High'),
            ('DivB', 'Dept1', 'Team2', 4.0, 1.1, 'Low'),
            ('DivB', 'Dept1', 'Team2', 4.3, 1.4, 'High'),
            ('DivB', 'Dept2', 'Team1', 3.1, 2.0, 'Low'),
            ('DivB', 'Dept2', 'Team1', 3.5, 2.3, 'High'),
            ('DivB', 'Dept2', 'Team2', 4.1, 2.1, 'Low'),
            ('DivB', 'Dept2', 'Team2', 4.4, 2.4, 'High')",
        )
        .await
        .expect("create three-level legend sharing test data");

        ctx.sql(
            "SELECT
                column1 AS division,
                column2 AS department,
                column3 AS team,
                column4 AS x_val,
                column5 AS y_val,
                column6 AS category
             FROM legend_sharing_three_level",
        )
        .await
        .expect("read three-level legend sharing test data")
    }

    async fn two_level_col_col_refinement_dataframe(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "CREATE TABLE two_level_col_col_refinement AS VALUES
            ('Eng', 'Frontend', 1.0, 90.0),
            ('Eng', 'Frontend', 2.0, 95.0),
            ('Eng', 'Backend', 1.0, 110.0),
            ('Eng', 'Backend', 2.0, 115.0),
            ('Ops', 'Support', 1.0, 40.0),
            ('Ops', 'Support', 2.0, 45.0),
            ('Ops', 'DevOps', 1.0, 60.0),
            ('Ops', 'DevOps', 2.0, 65.0)",
        )
        .await
        .expect("create two-level col-col refinement test data");

        ctx.sql(
            "SELECT
                column1 AS division,
                column2 AS department,
                column3 AS x_val,
                column4 AS y_val
             FROM two_level_col_col_refinement",
        )
        .await
        .expect("read two-level col-col refinement test data")
    }

    async fn nested_sparse_row_dataframe(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "CREATE TABLE nested_sparse_row AS VALUES
            ('Iris-setosa', 'narrow', 4.8, 3.1),
            ('Iris-setosa', 'narrow', 5.1, 3.4),
            ('Iris-versicolor', 'medium', 5.8, 2.8),
            ('Iris-versicolor', 'wide', 7.2, 3.0),
            ('Iris-virginica', 'medium', 6.3, 2.9),
            ('Iris-virginica', 'wide', 7.8, 3.1)",
        )
        .await
        .expect("create nested sparse row test data");

        ctx.sql(
            "SELECT
                column1 AS species,
                column2 AS petal_width_bin,
                column3 AS sepal_length,
                column4 AS sepal_width
             FROM nested_sparse_row",
        )
        .await
        .expect("read nested sparse row test data")
    }

    async fn shared_row_basic_dataframe(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "CREATE TABLE shared_row_basic AS VALUES
            ('C1', 'R1', 1.0, 1.0),
            ('C1', 'R2', 1.2, 1.3),
            ('C2', 'R1', 2.0, 1.1),
            ('C2', 'R3', 2.3, 1.4),
            ('C3', 'R2', 3.0, 1.2),
            ('C3', 'R3', 3.4, 1.5)",
        )
        .await
        .expect("create shared row basic test data");

        ctx.sql(
            "SELECT
                column1 AS col_group,
                column2 AS row_group,
                column3 AS x_val,
                column4 AS y_val
             FROM shared_row_basic",
        )
        .await
        .expect("read shared row basic test data")
    }

    async fn jagged_group_local_dataframe(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "CREATE TABLE jagged_group_local AS VALUES
            ('A', 'C1', 'A1', 1.0, 1.0),
            ('A', 'C1', 'A2', 1.2, 1.2),
            ('A', 'C1', 'A3', 1.4, 1.4),
            ('A', 'C2', 'A1', 2.0, 1.1),
            ('A', 'C2', 'A2', 2.2, 1.3),
            ('A', 'C3', 'A2', 3.0, 1.2),
            ('A', 'C3', 'A3', 3.2, 1.4),
            ('B', 'C1', 'B1', 1.0, 2.0),
            ('B', 'C1', 'B2', 1.3, 2.2),
            ('B', 'C2', 'B1', 2.0, 2.1),
            ('B', 'C2', 'B2', 2.3, 2.3)",
        )
        .await
        .expect("create jagged group-local test data");

        ctx.sql(
            "SELECT
                column1 AS outer_group,
                column2 AS inner_col,
                column3 AS row_group,
                column4 AS x_val,
                column5 AS y_val
             FROM jagged_group_local",
        )
        .await
        .expect("read jagged group-local test data")
    }

    fn build_level1_fill_legend_symbol(position: LegendPosition) -> Symbol<Cartesian> {
        Symbol::new()
            .x_with(col("x_val"), |c| c.with_scale_sharing(ScaleSharing::Shared))
            .y_with(col("y_val"), |c| c.with_scale_sharing(ScaleSharing::Shared))
            .fill_with(col("category"), move |c| {
                c.with_scale_sharing(ScaleSharing::Level(1))
                    .legend(|legend| legend.title("Category").position(position))
            })
            .size(70.0)
    }

    fn build_level2_fill_legend_symbol(position: LegendPosition) -> Symbol<Cartesian> {
        Symbol::new()
            .x_with(col("x_val"), |c| c.with_scale_sharing(ScaleSharing::Shared))
            .y_with(col("y_val"), |c| c.with_scale_sharing(ScaleSharing::Shared))
            .fill_with(col("category"), move |c| {
                c.with_scale_sharing(ScaleSharing::Level(2))
                    .legend(|legend| legend.title("Category").position(position))
            })
            .size(70.0)
    }

    fn build_unshared_fill_legend_symbol(position: LegendPosition) -> Symbol<Cartesian> {
        Symbol::new()
            .x(col("x_val"))
            .y(col("y_val"))
            .fill_with(col("category"), move |c| {
                c.legend(|legend| legend.title("Category").position(position))
            })
            .size(70.0)
    }

    fn build_two_level_col_legend_sharing_plot(
        df: DataFrame,
        position: LegendPosition,
    ) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(960.0, 420.0)
            .mark(
                Facet::new()
                    .column(col("division"))
                    .subplot(
                        Plot::<FacetColumn>::new().mark(
                            Facet::new().column(col("department")).subplot(
                                Plot::<Cartesian>::new()
                                    .mark(build_level1_fill_legend_symbol(position)),
                            ),
                        ),
                    ),
            )
    }

    fn build_three_level_col_legend_sharing_plot(
        df: DataFrame,
        position: LegendPosition,
    ) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(1500.0, 380.0)
            .mark(Facet::new().column(col("division")).subplot(
                Plot::<FacetColumn>::new().mark(Facet::new().column(col("department")).subplot(
                    Plot::<FacetColumn>::new().mark(Facet::new().column(col("team")).subplot(
                        Plot::<Cartesian>::new().mark(build_level2_fill_legend_symbol(position)),
                    )),
                )),
            ))
    }

    fn build_three_level_col_legend_sharing_plot_fixed_subplot(
        df: DataFrame,
        position: LegendPosition,
    ) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .plot_size(120.0, 90.0)
            .mark(Facet::new().column(col("division")).subplot(
                Plot::<FacetColumn>::new().mark(Facet::new().column(col("department")).subplot(
                    Plot::<FacetColumn>::new().mark(Facet::new().column(col("team")).subplot(
                        Plot::<Cartesian>::new().mark(build_level2_fill_legend_symbol(position)),
                    )),
                )),
            ))
    }

    fn build_nested_col_row_col_continuous_legend_plot_fixed_subplot(
        df: DataFrame,
    ) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .plot_size(110.0, 80.0)
            .mark(
                Facet::new().column(col("division")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("department")).subplot(
                            Plot::<FacetColumn>::new().mark(
                                Facet::new().column(col("team")).subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Shared)
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Shared)
                                            })
                                            .fill_with(col("y_val"), |c| {
                                                c.scale_with::<crate::scales::Linear>(|s| s).legend(
                                                    |legend| {
                                                        legend
                                                            .title("Score")
                                                            .position(LegendPosition::Right)
                                                    },
                                                )
                                            })
                                            .size(58.0),
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
            )
    }

    fn build_nested_sparse_row_plot(df: DataFrame) -> Plot<FacetRow> {
        Plot::<FacetRow>::new().data(df).canvas_size(600, 600).mark(
            Facet::new().row(col("species")).subplot(
                Plot::<FacetColumn>::new().mark(
                    Facet::new().column(col("petal_width_bin")).subplot(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("sepal_length"), |c| {
                                    c.with_scale_sharing(ScaleSharing::Shared)
                                })
                                .y_with(col("sepal_width"), |c| {
                                    c.with_scale_sharing(ScaleSharing::Level(1))
                                })
                                .size(25.0)
                                .fill("#4682b4"),
                        ),
                    ),
                ),
            ),
        )
    }

    fn build_nested_shared_row_basic_plot(df: DataFrame) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(760.0, 560.0)
            .mark(
                Facet::new().column(col("col_group")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new()
                            .row_with(col("row_group"), |c| {
                                c.facet(|f| {
                                    f.with_scale_sharing(ScaleSharing::Shared).position("right")
                                })
                            })
                            .subplot(
                                Plot::<Cartesian>::new()
                                    .mark(Symbol::new().x(col("x_val")).y(col("y_val")).size(35.0)),
                            ),
                    ),
                ),
            )
    }

    fn build_nested_shared_row_shared_both_plot(df: DataFrame) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(760.0, 560.0)
            .mark(
                Facet::new().column(col("col_group")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new()
                            .row_with(col("row_group"), |c| {
                                c.facet(|f| f.with_scale_sharing(ScaleSharing::Shared))
                            })
                            .subplot(
                                Plot::<Cartesian>::new().mark(
                                    Symbol::new()
                                        .x_with(col("x_val"), |c| {
                                            c.with_scale_sharing(ScaleSharing::Shared)
                                        })
                                        .y_with(col("y_val"), |c| {
                                            c.with_scale_sharing(ScaleSharing::Shared)
                                        })
                                        .size(35.0),
                                ),
                            ),
                    ),
                ),
            )
    }

    fn build_nested_shared_row_shared_both_plot_with_empty_policy(
        df: DataFrame,
        policy: FacetEmptyCellPolicy,
    ) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(760.0, 560.0)
            .mark(
                Facet::new().column(col("col_group")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new()
                            .row_with(col("row_group"), move |c| {
                                c.facet(|f| {
                                    f.with_scale_sharing(ScaleSharing::Shared)
                                        .empty_cell_policy(policy)
                                })
                            })
                            .subplot(
                                Plot::<Cartesian>::new().mark(
                                    Symbol::new()
                                        .x_with(col("x_val"), |c| {
                                            c.with_scale_sharing(ScaleSharing::Shared)
                                        })
                                        .y_with(col("y_val"), |c| {
                                            c.with_scale_sharing(ScaleSharing::Shared)
                                        })
                                        .size(35.0),
                                ),
                            ),
                    ),
                ),
            )
    }

    fn build_jagged_group_local_shared_row_plot(df: DataFrame) -> Plot<FacetRow> {
        Plot::<FacetRow>::new()
            .data(df)
            .canvas_size(980.0, 700.0)
            .mark(
                Facet::new().row(col("outer_group")).subplot(
                    Plot::<FacetColumn>::new().mark(
                        Facet::new().column(col("inner_col")).subplot(
                            Plot::<FacetRow>::new().mark(
                                Facet::new()
                                    .row_with(col("row_group"), |c| {
                                        c.facet(|f| {
                                            f.with_scale_sharing(ScaleSharing::Level(1))
                                                .position("right")
                                        })
                                    })
                                    .subplot(
                                        Plot::<Cartesian>::new().mark(
                                            Symbol::new()
                                                .x(col("x_val"))
                                                .y(col("y_val"))
                                                .size(25.0)
                                                .fill("#4682b4"),
                                        ),
                                    ),
                            ),
                        ),
                    ),
                ),
            )
    }

    fn build_single_level_row_legend_plot(
        df: DataFrame,
        position: LegendPosition,
    ) -> Plot<FacetRow> {
        Plot::<FacetRow>::new()
            .data(df)
            .canvas_size(960.0, 420.0)
            .mark(Facet::new().row(col("department")).subplot(
                Plot::<Cartesian>::new().mark(build_unshared_fill_legend_symbol(position)),
            ))
    }

    fn build_single_level_col_legend_plot(
        df: DataFrame,
        position: LegendPosition,
    ) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(960.0, 420.0)
            .mark(Facet::new().column(col("division")).subplot(
                Plot::<Cartesian>::new().mark(build_unshared_fill_legend_symbol(position)),
            ))
    }

    async fn compile_two_level_col_legend_sharing_plot(
        ctx: &SessionContext,
        position: LegendPosition,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = legend_sharing_dataframe(ctx).await;
        build_two_level_col_legend_sharing_plot(df, position)
            .compile(ctx)
            .await
    }

    async fn compile_three_level_col_legend_sharing_plot(
        ctx: &SessionContext,
        position: LegendPosition,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = legend_sharing_three_level_dataframe(ctx).await;
        build_three_level_col_legend_sharing_plot(df, position)
            .compile(ctx)
            .await
    }

    async fn compile_three_level_col_legend_sharing_plot_fixed_subplot(
        ctx: &SessionContext,
        position: LegendPosition,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = legend_sharing_three_level_dataframe(ctx).await;
        build_three_level_col_legend_sharing_plot_fixed_subplot(df, position)
            .compile(ctx)
            .await
    }

    async fn compile_two_level_col_col_refinement_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = two_level_col_col_refinement_dataframe(ctx).await;
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(1800, 500)
            .mark(
                Facet::new()
                    .col_with(col("division"), |c| c.facet(|f| f.title("Division")))
                    .subplot(
                        Plot::<FacetColumn>::new().mark(
                            Facet::new()
                                .col_with(col("department"), |c| c.facet(|f| f.title("Dept")))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Level(2))
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Level(2))
                                            })
                                            .size(40.0)
                                            .fill("#9b59b6"),
                                    ),
                                ),
                        ),
                    ),
            )
            .compile(ctx)
            .await
    }

    async fn compile_nested_col_row_col_continuous_legend_plot_fixed_subplot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = legend_sharing_three_level_dataframe(ctx).await;
        build_nested_col_row_col_continuous_legend_plot_fixed_subplot(df)
            .compile(ctx)
            .await
    }

    async fn compile_nested_sparse_row_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = nested_sparse_row_dataframe(ctx).await;
        build_nested_sparse_row_plot(df).compile(ctx).await
    }

    async fn compile_nested_shared_row_basic_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = shared_row_basic_dataframe(ctx).await;
        build_nested_shared_row_basic_plot(df).compile(ctx).await
    }

    async fn compile_nested_shared_row_shared_both_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = shared_row_basic_dataframe(ctx).await;
        build_nested_shared_row_shared_both_plot(df)
            .compile(ctx)
            .await
    }

    async fn compile_nested_shared_row_shared_both_plot_with_empty_policy(
        ctx: &SessionContext,
        policy: FacetEmptyCellPolicy,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = shared_row_basic_dataframe(ctx).await;
        build_nested_shared_row_shared_both_plot_with_empty_policy(df, policy)
            .compile(ctx)
            .await
    }

    async fn compile_jagged_group_local_shared_row_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = jagged_group_local_dataframe(ctx).await;
        build_jagged_group_local_shared_row_plot(df)
            .compile(ctx)
            .await
    }

    async fn compile_single_level_row_legend_plot(
        ctx: &SessionContext,
        position: LegendPosition,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = legend_sharing_dataframe(ctx).await;
        build_single_level_row_legend_plot(df, position)
            .compile(ctx)
            .await
    }

    async fn compile_single_level_col_legend_plot(
        ctx: &SessionContext,
        position: LegendPosition,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = legend_sharing_dataframe(ctx).await;
        build_single_level_col_legend_plot(df, position)
            .compile(ctx)
            .await
    }

    async fn compile_deeply_nested_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = deeply_nested_dataframe(ctx);
        let plot = build_deeply_nested_plot(df);
        plot.compile(ctx).await
    }

    async fn compile_simple_facet_plot_with_plot_size(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = deeply_nested_dataframe(ctx);
        build_simple_facet_col_plot(df)
            .plot_size(120.0, 90.0)
            .compile(ctx)
            .await
    }

    async fn prepare_top_level_measurement(
        compiled: &CompiledPlot,
        ctx: &SessionContext,
    ) -> Result<
        (
            EvaluationContext,
            EvaluatedLayoutSpec,
            crate::scales::ScaleBuilder,
            ComponentsMeasurement,
        ),
        AvengerChartError,
    > {
        let merged_params = compiled.get_default_params().clone();
        let facet_tree = Arc::new(EvaluatedFacetTree::from_compiled_plot(compiled, ctx).await?);
        let evaluated_layout_spec = evaluate_layout_spec(
            compiled.get_layout_spec(),
            ctx,
            &merged_params,
            compiled.get_theme().as_ref(),
        )
        .await?;
        let facet_sizing_strategy =
            compiled.resolve_facet_sizing_strategy(&evaluated_layout_spec)?;
        let measured_layout_spec = CompiledPlot::layout_spec_for_facet_sizing_strategy(
            &evaluated_layout_spec,
            facet_tree.as_ref(),
            facet_sizing_strategy,
        );

        let scale_builder = build_scale_builder_from_marks(
            &compiled.marks,
            &compiled.scale_specs,
            &compiled.coord_transform,
            &compiled.data,
            None,
            ctx,
            &merged_params,
            compiled.get_theme().as_ref(),
        )
        .await?;
        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: compiled,
        };

        let eval_ctx = EvaluationContext::new(
            compiled.get_theme(),
            Arc::new(ctx.clone()),
            merged_params,
            facet_tree,
        )
        .with_facet_runtime_sizing_mode(facet_sizing_strategy.runtime_sizing_mode());

        let measurement = compiled
            .measure_plot_components(&eval_ctx, &measured_layout_spec, &provider, None, &[])
            .await?;

        Ok((eval_ctx, measured_layout_spec, scale_builder, measurement))
    }

    async fn prepare_refined_top_level_measurement(
        compiled: &CompiledPlot,
        ctx: &SessionContext,
    ) -> Result<
        (
            EvaluationContext,
            EvaluatedLayoutSpec,
            ComponentsMeasurement,
        ),
        AvengerChartError,
    > {
        let (eval_ctx, evaluated_layout_spec, scale_builder, mut measurement) =
            prepare_top_level_measurement(compiled, ctx).await?;

        let facet_sizing_strategy =
            compiled.resolve_facet_sizing_strategy(&evaluated_layout_spec)?;
        let coordination_mode = facet_sizing_strategy.coordination_mode();
        coordinate_overflow_for_guides_with_mode(&mut measurement, &eval_ctx, coordination_mode)
            .await?;
        let (_, _, is_plot_area_mode) =
            CompiledPlot::resolve_dimensions_from_spec(&evaluated_layout_spec);
        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: compiled,
        };
        match facet_sizing_strategy {
            FacetSizingStrategy::CanvasFit => {
                if !is_plot_area_mode {
                    compiled
                        .refine_canvas_measurement_after_coordination(
                            &mut measurement,
                            &eval_ctx,
                            &evaluated_layout_spec,
                            &provider,
                            None,
                            &[],
                            eval_ctx.facet_layout_refinement().max_refinement_passes,
                        )
                        .await?;
                }
            }
            FacetSizingStrategy::FixedSubplot { .. } => {
                compiled
                    .realize_fixed_subplot_layout_after_coordination(
                        &mut measurement,
                        &eval_ctx,
                        &evaluated_layout_spec,
                        &provider,
                        None,
                        &[],
                        eval_ctx.facet_layout_refinement().max_refinement_passes,
                    )
                    .await?;
            }
        }

        Ok((eval_ctx, evaluated_layout_spec, measurement))
    }

    fn max_overflow_abs_delta(a: &OverflowSpaceRequirement, b: &OverflowSpaceRequirement) -> f32 {
        let top = (a.top - b.top).abs();
        let right = (a.right - b.right).abs();
        let bottom = (a.bottom - b.bottom).abs();
        let left = (a.left - b.left).abs();
        top.max(right).max(bottom).max(left)
    }

    fn legend_slab_for_position(overflow: &CoordinatedOverflow, position: LegendPosition) -> f32 {
        match position {
            LegendPosition::Top => (overflow.total.top - overflow.guide.top).max(0.0),
            LegendPosition::Right => (overflow.total.right - overflow.guide.right).max(0.0),
            LegendPosition::Bottom => (overflow.total.bottom - overflow.guide.bottom).max(0.0),
            LegendPosition::Left => (overflow.total.left - overflow.guide.left).max(0.0),
        }
    }

    fn absolute_origins_for_named_groups(
        scene_graph: &SceneGraph,
        prefix: &str,
    ) -> Vec<(String, [f32; 2])> {
        let mut groups = scene_graph
            .group_paths()
            .into_iter()
            .filter_map(|path| {
                let SceneMark::Group(group) = scene_graph.get_mark(&path)? else {
                    return None;
                };
                if !group.name.starts_with(prefix) || group.name.ends_with("_empty") {
                    return None;
                }
                let origin = scene_graph.get_absolute_origin(&path)?;
                Some((group.name.clone(), origin))
            })
            .collect::<Vec<_>>();
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        groups
    }

    fn count_groups_with_name(scene_graph: &SceneGraph, prefix: &str, ends_with: &str) -> usize {
        scene_graph
            .group_paths()
            .into_iter()
            .filter_map(|path| scene_graph.get_mark(&path))
            .filter_map(|mark| match mark {
                SceneMark::Group(group) => Some(group),
                _ => None,
            })
            .filter(|group| group.name.starts_with(prefix) && group.name.ends_with(ends_with))
            .count()
    }

    fn count_groups_with_prefix_excluding_suffix(
        scene_graph: &SceneGraph,
        prefix: &str,
        excluded_suffix: &str,
    ) -> usize {
        scene_graph
            .group_paths()
            .into_iter()
            .filter_map(|path| scene_graph.get_mark(&path))
            .filter_map(|mark| match mark {
                SceneMark::Group(group) => Some(group),
                _ => None,
            })
            .filter(|group| {
                group.name.starts_with(prefix) && !group.name.ends_with(excluded_suffix)
            })
            .count()
    }

    fn collect_text_x_positions(scene_graph: &SceneGraph, text: &str) -> Vec<f32> {
        fn collect_from_mark(mark: &SceneMark, origin: [f32; 2], text: &str, xs: &mut Vec<f32>) {
            match mark {
                SceneMark::Group(group) => {
                    let next_origin = [origin[0] + group.origin[0], origin[1] + group.origin[1]];
                    for child in &group.marks {
                        collect_from_mark(child, next_origin, text, xs);
                    }
                }
                SceneMark::Text(text_mark) => {
                    let matches = text_mark.text_iter().any(|value| value == text);
                    if !matches {
                        return;
                    }
                    if let Some(x) = text_mark.x_iter().next() {
                        xs.push(origin[0] + *x);
                    }
                }
                _ => {}
            }
        }

        let mut xs = Vec::new();
        for mark in scene_graph.children() {
            collect_from_mark(mark, [0.0, 0.0], text, &mut xs);
        }
        xs.sort_by(f32::total_cmp);
        xs
    }

    fn collect_leaf_plot_areas(
        measurement: &ComponentsMeasurement,
        leaf_widths: &mut Vec<f32>,
        leaf_heights: &mut Vec<f32>,
    ) {
        if let Some(facet_band) = facet_band_ref(measurement) {
            for child in facet_band.child_measurements_iter() {
                collect_leaf_plot_areas(child, leaf_widths, leaf_heights);
            }
        } else {
            leaf_widths.push(measurement.plot_area_width);
            leaf_heights.push(measurement.plot_area_height);
        }
    }

    fn assert_no_fixed_subplot_main_axis_overlap(measurement: &ComponentsMeasurement) {
        if let Some(facet_band_fixed) = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurementFixed>()
        {
            let facet_band = &facet_band_fixed.base;
            let positions = &facet_band_fixed.fixed_main_axis_positions;
            for (idx, window) in positions.windows(2).enumerate() {
                let current_start = window[0];
                let next_start = window[1];
                let current_span = match facet_band.axis {
                    FacetAxis::Column => facet_band.cells[idx].measurement.plot_area_width,
                    FacetAxis::Row => facet_band.cells[idx].measurement.plot_area_height,
                };
                assert!(
                    next_start + 0.01 >= current_start + current_span,
                    "fixed-subplot main-axis overlap at axis {:?}, cell_index {}, current_start={}, current_span={}, next_start={}",
                    facet_band.axis,
                    idx,
                    current_start,
                    current_span,
                    next_start
                );
            }

            for child in facet_band.child_measurements_iter() {
                assert_no_fixed_subplot_main_axis_overlap(child);
            }
        }
    }

    fn count_coordinated_layout_patches(measurement: &ComponentsMeasurement) -> usize {
        if let Some(facet_band) = facet_band_ref(measurement) {
            let local = usize::from(facet_band.coordinated_layout.is_some());
            local
                + facet_band
                    .child_measurements_iter()
                    .map(count_coordinated_layout_patches)
                    .sum::<usize>()
        } else {
            0
        }
    }

    fn assert_legends_within_canvas(measurement: &ComponentsMeasurement) {
        let canvas_width = measurement.canvas_size.0;
        let canvas_height = measurement.canvas_size.1;
        for (legend_key, bounds) in &measurement.layout.taffy_layout.legends {
            assert!(
                bounds.x >= -0.5,
                "legend {legend_key} starts left of canvas: x={}, canvas_width={canvas_width}",
                bounds.x
            );
            assert!(
                bounds.y >= -0.5,
                "legend {legend_key} starts above canvas: y={}, canvas_height={canvas_height}",
                bounds.y
            );
            assert!(
                bounds.x + bounds.width <= canvas_width + 0.5,
                "legend {legend_key} exceeds canvas width: right={}, canvas_width={canvas_width}",
                bounds.x + bounds.width
            );
            assert!(
                bounds.y + bounds.height <= canvas_height + 0.5,
                "legend {legend_key} exceeds canvas height: bottom={}, canvas_height={canvas_height}",
                bounds.y + bounds.height
            );
        }

        if let Some(facet_band) = facet_band_ref(measurement) {
            for child in facet_band.child_measurements_iter() {
                assert_legends_within_canvas(child);
            }
        }
    }

    fn count_legend_measurements_recursive(measurement: &ComponentsMeasurement) -> usize {
        let mut count = measurement.legend_plan.measurements.len();
        if let Some(facet_band) = facet_band_ref(measurement) {
            for child in facet_band.child_measurements_iter() {
                count += count_legend_measurements_recursive(child);
            }
        }
        count
    }

    fn assert_fixed_positions_match_recomputed(measurement: &ComponentsMeasurement) {
        if let Some(fixed_facet) = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<crate::facet::coord::FacetBandCoordMeasurementFixed>(
        ) {
            let layout = fixed_facet
                .coordinated_layout
                .as_ref()
                .unwrap_or(&fixed_facet.local_layout);
            let expected = crate::facet::coord::compute_fixed_main_axis_positions(
                fixed_facet.axis,
                &fixed_facet.cells,
                layout,
            );
            assert_eq!(
                fixed_facet.fixed_main_axis_positions.len(),
                expected.len(),
                "fixed facet position count mismatch when validating recomputed positions"
            );
            for (idx, (actual, expected)) in fixed_facet
                .fixed_main_axis_positions
                .iter()
                .zip(expected.iter())
                .enumerate()
            {
                assert!(
                    (actual - expected).abs() <= 0.01,
                    "fixed position mismatch at index {idx}: actual={actual}, expected={expected}"
                );
            }

            for child in fixed_facet.child_measurements_iter() {
                assert_fixed_positions_match_recomputed(child);
            }
        }
    }

    fn collect_team_level_fixed_apply_signals(
        measurement: &ComponentsMeasurement,
        out: &mut Vec<(f32, bool, bool)>,
    ) {
        if let Some(facet_band) = facet_band_ref(measurement) {
            if facet_band.coordination_field_identity == "team" {
                let slabs = LayoutSlabs::from_coordinated(&facet_band.coordinated_overflow);
                let apply_plan = facet_band.derive_coordinated_apply_plan();
                out.push((
                    slabs.legend.right.max(0.0),
                    apply_plan.has_coordinated_layout,
                    apply_plan.remeasure_required,
                ));
            }

            for child in facet_band.child_measurements_iter() {
                collect_team_level_fixed_apply_signals(child, out);
            }
        }
    }

    #[tokio::test]
    async fn evaluate_default_matches_with_options_final() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;

        let default_eval = compiled.evaluate(&ctx, None).await?;
        let options_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Final,
                    debug_layout_lines: false,
                    ..EvaluationOptions::default()
                },
            )
            .await?;

        assert_eq!(
            default_eval.scene_graph.width,
            options_eval.scene_graph.width
        );
        assert_eq!(
            default_eval.scene_graph.height,
            options_eval.scene_graph.height
        );

        let default_col_origins =
            absolute_origins_for_named_groups(&default_eval.scene_graph, "facet_col_");
        let options_col_origins =
            absolute_origins_for_named_groups(&options_eval.scene_graph, "facet_col_");
        assert_eq!(default_col_origins, options_col_origins);

        let default_row_origins =
            absolute_origins_for_named_groups(&default_eval.scene_graph, "facet_row_");
        let options_row_origins =
            absolute_origins_for_named_groups(&options_eval.scene_graph, "facet_row_");
        assert_eq!(default_row_origins, options_row_origins);

        Ok(())
    }

    #[tokio::test]
    async fn evaluate_with_options_initial_and_coordinated_execute_for_nested_facets()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;

        let initial_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::LocalMeasured),
                    debug_layout_lines: false,
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let coordinated_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::Coordination(
                        CoordinationCheckpoint::FinalPropagationComplete,
                    )),
                    debug_layout_lines: false,
                    ..EvaluationOptions::default()
                },
            )
            .await?;

        assert!(initial_eval.scene_graph.width > 0.0);
        assert!(initial_eval.scene_graph.height > 0.0);
        assert!(coordinated_eval.scene_graph.width > 0.0);
        assert!(coordinated_eval.scene_graph.height > 0.0);

        let initial_col_origins =
            absolute_origins_for_named_groups(&initial_eval.scene_graph, "facet_col_");
        let coordinated_col_origins =
            absolute_origins_for_named_groups(&coordinated_eval.scene_graph, "facet_col_");
        let initial_row_origins =
            absolute_origins_for_named_groups(&initial_eval.scene_graph, "facet_row_");
        let coordinated_row_origins =
            absolute_origins_for_named_groups(&coordinated_eval.scene_graph, "facet_row_");

        assert!(!initial_col_origins.is_empty());
        assert!(!coordinated_col_origins.is_empty());
        assert_eq!(initial_col_origins.len(), coordinated_col_origins.len());
        assert_eq!(initial_row_origins.len(), coordinated_row_origins.len());

        Ok(())
    }

    #[tokio::test]
    async fn facet_subtree_snapshots_render_probe_and_local_layout() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;
        let selector = FacetSubtreeSelector::ByFacetPath(vec![
            ScalarValue::Utf8(Some("G1".to_string())),
            ScalarValue::Utf8(Some("S1".to_string())),
        ]);

        let probe_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::FacetSubtree(FacetSubtreeSnapshot {
                        selector: selector.clone(),
                        checkpoint: FacetSubtreeCheckpoint::EstimatedOverflowProbe,
                    }),
                    debug_layout_lines: true,
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let probe_by_index_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::FacetSubtree(FacetSubtreeSnapshot {
                        selector: FacetSubtreeSelector::ByCoordNodePath(vec![0, 0]),
                        checkpoint: FacetSubtreeCheckpoint::EstimatedOverflowProbe,
                    }),
                    debug_layout_lines: true,
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let local_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::FacetSubtree(FacetSubtreeSnapshot {
                        selector,
                        checkpoint: FacetSubtreeCheckpoint::LocalRetargetedLayout,
                    }),
                    debug_layout_lines: true,
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let by_index_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::FacetSubtree(FacetSubtreeSnapshot {
                        selector: FacetSubtreeSelector::ByCoordNodePath(vec![0, 0]),
                        checkpoint: FacetSubtreeCheckpoint::LocalRetargetedLayout,
                    }),
                    debug_layout_lines: true,
                    ..EvaluationOptions::default()
                },
            )
            .await?;

        assert!(probe_eval.scene_graph.width > 0.0);
        assert!(probe_eval.scene_graph.height > 0.0);
        assert_eq!(
            probe_eval.scene_graph.width,
            probe_by_index_eval.scene_graph.width
        );
        assert_eq!(
            probe_eval.scene_graph.height,
            probe_by_index_eval.scene_graph.height
        );
        assert!(local_eval.scene_graph.width > 0.0);
        assert!(local_eval.scene_graph.height > 0.0);
        assert_eq!(
            local_eval.scene_graph.width,
            by_index_eval.scene_graph.width
        );
        assert_eq!(
            local_eval.scene_graph.height,
            by_index_eval.scene_graph.height
        );

        Ok(())
    }

    #[test]
    fn evaluation_metrics_capture_facet_recursive_counts() {
        run_with_large_stack(|| async {
            let ctx = SessionContext::new();
            let compiled =
                compile_three_level_col_legend_sharing_plot(&ctx, LegendPosition::Right).await?;
            let (_evaluated, metrics) = compiled
                .evaluate_with_options_and_metrics(
                    &ctx,
                    None,
                    EvaluationOptions {
                        layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::LocalMeasured),
                        debug_layout_lines: false,
                        ..EvaluationOptions::default()
                    },
                )
                .await?;
            let facet_metrics = &metrics.facet_layout;

            assert_eq!(
                facet_metrics
                    .plot_component_measure_calls_by_facet_depth
                    .first()
                    .copied(),
                Some(1),
                "expected one top-level measurement: {metrics:?}"
            );
            assert!(
                facet_metrics
                    .plot_component_measure_calls_by_facet_depth
                    .len()
                    >= 4,
                "expected top-level plus three facet path depths: {metrics:?}"
            );
            assert!(
                facet_metrics.plot_component_measure_calls_by_facet_depth[3] > 0,
                "expected leaf-depth measurements: {metrics:?}"
            );
            assert!(
                facet_metrics.facet_band_measure_runs > 0,
                "expected facet-band measurement pipelines: {metrics:?}"
            );
            assert!(
                facet_metrics.estimated_overflow_non_leaf_aggregate_count > 0,
                "expected non-leaf estimated-overflow probe aggregates: {metrics:?}"
            );
            assert!(
                facet_metrics.estimated_overflow_non_leaf_full_measure_count > 0,
                "expected non-leaf estimated-overflow probes to require full subtree measurement: {metrics:?}"
            );
            assert!(
                facet_metrics.plot_component_measure_calls <= 128,
                "unexpected recursive measurement regression: {metrics:?}"
            );

            Ok(())
        });
    }

    #[test]
    fn final_snapshot_fast_measure_once_has_no_refinement_passes() {
        run_with_large_stack(|| async {
            let ctx = SessionContext::new();
            let compiled =
                compile_three_level_col_legend_sharing_plot(&ctx, LegendPosition::Right).await?;
            let (_evaluated, metrics) = compiled
                .evaluate_with_options_and_metrics(
                    &ctx,
                    None,
                    EvaluationOptions {
                        layout_snapshot: LayoutSnapshot::Final,
                        debug_layout_lines: false,
                        facet_layout_refinement: FacetLayoutRefinement {
                            max_refinement_passes: 0,
                            overflow_growth_epsilon: 0.5,
                        },
                    },
                )
                .await?;
            let facet_metrics = &metrics.facet_layout;

            assert_eq!(
                facet_metrics.refinement_pass_count, 0,
                "max_refinement_passes=0 should not run extra measurement passes: {metrics:?}"
            );

            Ok(())
        });
    }

    #[test]
    fn canvas_refinement_remeasures_after_plot_area_retarget() {
        run_with_large_stack(|| async {
            let ctx = SessionContext::new();
            let compiled = compile_two_level_col_col_refinement_plot(&ctx).await?;
            let selector = FacetSubtreeSelector::ByFacetPath(vec![
                ScalarValue::Utf8(Some("Eng".to_string())),
                ScalarValue::Utf8(Some("Frontend".to_string())),
            ]);

            let (fast_eval, fast_metrics) = compiled
                .evaluate_with_options_and_metrics(
                    &ctx,
                    None,
                    EvaluationOptions {
                        layout_snapshot: LayoutSnapshot::FacetSubtree(FacetSubtreeSnapshot {
                            selector: selector.clone(),
                            checkpoint: FacetSubtreeCheckpoint::FinalLayout,
                        }),
                        debug_layout_lines: true,
                        facet_layout_refinement: FacetLayoutRefinement {
                            max_refinement_passes: 0,
                            overflow_growth_epsilon: 0.5,
                        },
                    },
                )
                .await?;
            let (refined_eval, refined_metrics) = compiled
                .evaluate_with_options_and_metrics(
                    &ctx,
                    None,
                    EvaluationOptions {
                        layout_snapshot: LayoutSnapshot::FacetSubtree(FacetSubtreeSnapshot {
                            selector,
                            checkpoint: FacetSubtreeCheckpoint::FinalLayout,
                        }),
                        debug_layout_lines: true,
                        facet_layout_refinement: FacetLayoutRefinement {
                            max_refinement_passes: 1,
                            overflow_growth_epsilon: 0.5,
                        },
                    },
                )
                .await?;

            assert_eq!(fast_metrics.facet_layout.refinement_pass_count, 0);
            assert!(
                refined_metrics.facet_layout.refinement_pass_count >= 1,
                "expected an additional canvas refinement pass: {refined_metrics:?}"
            );
            assert!(
                refined_metrics.facet_layout.plot_component_measure_calls
                    > fast_metrics.facet_layout.plot_component_measure_calls,
                "refinement should remeasure at the retargeted subplot size; fast={fast_metrics:?}, refined={refined_metrics:?}"
            );
            assert_eq!(
                collect_text_x_positions(&fast_eval.scene_graph, "of-right").len(),
                0,
                "fast one-shot subtree should document the missing right overflow"
            );
            assert!(
                !collect_text_x_positions(&refined_eval.scene_graph, "of-right").is_empty(),
                "refined subtree should allocate the late right overflow"
            );

            Ok(())
        });
    }

    #[test]
    fn canvas_refinement_snapshot_reuses_iteration_remeasure_path() {
        run_with_large_stack(|| async {
            let ctx = SessionContext::new();
            let compiled = compile_two_level_col_col_refinement_plot(&ctx).await?;
            let (_evaluated, metrics) = compiled
                .evaluate_with_options_and_metrics(
                    &ctx,
                    None,
                    EvaluationOptions {
                        layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::Refinement {
                            iteration: 1,
                            checkpoint: RefinementCheckpoint::Recoordinated,
                        }),
                        debug_layout_lines: false,
                        facet_layout_refinement: FacetLayoutRefinement {
                            max_refinement_passes: 1,
                            overflow_growth_epsilon: 0.5,
                        },
                    },
                )
                .await?;

            assert!(
                metrics.facet_layout.plot_component_measure_calls > 7,
                "iteration-1 snapshot should run the refinement remeasure path: {metrics:?}"
            );

            Ok(())
        });
    }

    #[test]
    fn initial_snapshot_ignores_facet_refinement_budget() {
        run_with_large_stack(|| async {
            let ctx = SessionContext::new();
            let compiled =
                compile_three_level_col_legend_sharing_plot(&ctx, LegendPosition::Right).await?;
            let (_evaluated, metrics) = compiled
                .evaluate_with_options_and_metrics(
                    &ctx,
                    None,
                    EvaluationOptions {
                        layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::LocalMeasured),
                        debug_layout_lines: false,
                        facet_layout_refinement: FacetLayoutRefinement {
                            max_refinement_passes: 3,
                            overflow_growth_epsilon: 0.5,
                        },
                    },
                )
                .await?;

            assert_eq!(
                metrics.facet_layout.refinement_pass_count, 0,
                "Initial snapshot should stop before final refinement: {metrics:?}"
            );

            Ok(())
        });
    }

    #[tokio::test]
    async fn evaluate_with_options_coordinated_vs_final_canvas_mode()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;
        let coordinated_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::Coordination(
                        CoordinationCheckpoint::FinalPropagationComplete,
                    )),
                    debug_layout_lines: false,
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let final_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Final,
                    debug_layout_lines: false,
                    ..EvaluationOptions::default()
                },
            )
            .await?;

        assert!(coordinated_eval.scene_graph.width > 0.0);
        assert!(coordinated_eval.scene_graph.height > 0.0);
        assert!(final_eval.scene_graph.width > 0.0);
        assert!(final_eval.scene_graph.height > 0.0);

        let (eval_ctx, evaluated_layout_spec, scale_builder, mut coordinated_measurement) =
            prepare_top_level_measurement(&compiled, &ctx).await?;
        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: &compiled,
        };
        compiled
            .apply_layout_snapshot(
                &LayoutSnapshot::Whole(WholeChartSnapshot::Coordination(
                    CoordinationCheckpoint::FinalPropagationComplete,
                )),
                &mut coordinated_measurement,
                &eval_ctx,
                &evaluated_layout_spec,
                &provider,
                FacetSizingStrategy::CanvasFit,
                FacetCoordinationMode::CanvasFullCycle,
            )
            .await?;

        let coordinated_bounds = coordinated_measurement.layout.plot_area_bounds();
        let coordinated_delta_w =
            (coordinated_measurement.plot_area_width - coordinated_bounds.width).abs();
        let coordinated_delta_h =
            (coordinated_measurement.plot_area_height - coordinated_bounds.height).abs();

        let (eval_ctx, evaluated_layout_spec, scale_builder, mut final_measurement) =
            prepare_top_level_measurement(&compiled, &ctx).await?;
        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: &compiled,
        };
        compiled
            .apply_layout_snapshot(
                &LayoutSnapshot::Final,
                &mut final_measurement,
                &eval_ctx,
                &evaluated_layout_spec,
                &provider,
                FacetSizingStrategy::CanvasFit,
                FacetCoordinationMode::CanvasFullCycle,
            )
            .await?;

        let final_bounds = final_measurement.layout.plot_area_bounds();
        let final_delta_w = (final_measurement.plot_area_width - final_bounds.width).abs();
        let final_delta_h = (final_measurement.plot_area_height - final_bounds.height).abs();

        assert!(final_delta_w <= CompiledPlot::LAYOUT_REFINEMENT_EPSILON);
        assert!(final_delta_h <= CompiledPlot::LAYOUT_REFINEMENT_EPSILON);
        assert!(final_delta_w <= coordinated_delta_w + 1e-6);
        assert!(final_delta_h <= coordinated_delta_h + 1e-6);

        Ok(())
    }

    #[tokio::test]
    async fn facet_plot_size_mode_detected_on_top_level_facet_root() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = compile_simple_facet_plot_with_plot_size(&ctx).await?;
        let params = compiled.get_default_params().clone();
        let evaluated_layout_spec = evaluate_layout_spec(
            compiled.get_layout_spec(),
            &ctx,
            &params,
            compiled.get_theme().as_ref(),
        )
        .await?;

        let strategy = compiled.resolve_facet_sizing_strategy(&evaluated_layout_spec)?;
        assert!(
            matches!(
                strategy,
                FacetSizingStrategy::FixedSubplot {
                    leaf_plot_width: 120.0,
                    leaf_plot_height: 90.0
                }
            ),
            "expected fixed-subplot strategy for faceted plot_size"
        );
        Ok(())
    }

    #[tokio::test]
    async fn facet_plot_size_mode_evaluates_successfully() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_simple_facet_plot_with_plot_size(&ctx).await?;
        let evaluated = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Final,
                    debug_layout_lines: false,
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        assert!(evaluated.scene_graph.width > 0.0);
        assert!(evaluated.scene_graph.height > 0.0);
        Ok(())
    }

    #[tokio::test]
    async fn facet_canvas_and_plot_size_combination_errors() {
        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let compiled = build_simple_facet_col_plot(df)
            .canvas_size(640.0, 420.0)
            .plot_size(120.0, 90.0)
            .compile(&ctx)
            .await
            .expect("compile facet plot with both canvas_size and plot_size");

        let err = match compiled
            .evaluate_with_options(&ctx, None, EvaluationOptions::default())
            .await
        {
            Ok(_) => panic!("facet chart with canvas_size + plot_size should error"),
            Err(err) => err,
        };
        let message = err.to_string();
        assert!(
            message.contains("cannot combine `canvas_size(...)` and `plot_size(...)`"),
            "unexpected error: {message}"
        );
    }

    #[tokio::test]
    async fn facet_partial_constraints_error_in_fixed_subplot_mode() {
        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let compiled = build_simple_facet_col_plot(df)
            .plot_constraint(PlotConstraint::width(200.0))
            .compile(&ctx)
            .await
            .expect("compile facet plot with plot_constraint");

        let err = match compiled
            .evaluate_with_options(&ctx, None, EvaluationOptions::default())
            .await
        {
            Ok(_) => panic!("faceted partial constraints should error"),
            Err(err) => err,
        };
        let message = err.to_string();
        assert!(
            message.contains("do not support partial `canvas_constraint`/`plot_constraint`"),
            "unexpected error: {message}"
        );
    }

    #[tokio::test]
    async fn nested_subplot_plot_size_under_facet_errors() {
        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let plot = Plot::<FacetColumn>::new()
            .data(df)
            .plot_size(120.0, 90.0)
            .mark(
                Facet::new().column(col("outer_group")).subplot(
                    Plot::<Cartesian>::new()
                        .plot_size(80.0, 60.0)
                        .mark(Symbol::new().x(col("value")).y(col("value")).size(24.0)),
                ),
            );

        let compiled = plot
            .compile(&ctx)
            .await
            .expect("compile nested subplot plot_size test");
        let err = match compiled
            .evaluate_with_options(&ctx, None, EvaluationOptions::default())
            .await
        {
            Ok(_) => panic!("nested subplot plot_size under facet should error"),
            Err(err) => err,
        };
        let message = err.to_string();
        assert!(
            message.contains("cannot set `plot_size(...)` when used under a facet"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn fixed_subplot_mode_leaf_plot_area_uniform_for_three_level_col_col_col_with_level2_right_legend()
     {
        run_with_large_stack(|| async {
            let ctx = SessionContext::new();
            let compiled = compile_three_level_col_legend_sharing_plot_fixed_subplot(
                &ctx,
                LegendPosition::Right,
            )
            .await?;
            let (_, _, measurement) =
                prepare_refined_top_level_measurement(&compiled, &ctx).await?;

            let mut leaf_widths = Vec::new();
            let mut leaf_heights = Vec::new();
            collect_leaf_plot_areas(&measurement, &mut leaf_widths, &mut leaf_heights);
            assert!(
                !leaf_widths.is_empty(),
                "expected at least one leaf measurement"
            );
            assert_eq!(leaf_widths.len(), leaf_heights.len());

            let first_width = leaf_widths[0];
            let first_height = leaf_heights[0];
            for (idx, width) in leaf_widths.iter().enumerate() {
                assert!(
                    (width - first_width).abs() <= 0.01,
                    "leaf width mismatch at index {idx}: width={width}, baseline={first_width}"
                );
                assert!(
                    (*width - 120.0).abs() <= 0.01,
                    "leaf width should match fixed plot_size width: width={width}"
                );
            }
            for (idx, height) in leaf_heights.iter().enumerate() {
                assert!(
                    (height - first_height).abs() <= 0.01,
                    "leaf height mismatch at index {idx}: height={height}, baseline={first_height}"
                );
                assert!(
                    (*height - 90.0).abs() <= 0.01,
                    "leaf height should match fixed plot_size height: height={height}"
                );
            }
            Ok(())
        });
    }

    #[test]
    fn fixed_level2_right_detects_legend_overflow_on_team_nodes() {
        run_with_large_stack(|| async {
            let ctx = SessionContext::new();
            let compiled = compile_three_level_col_legend_sharing_plot_fixed_subplot(
                &ctx,
                LegendPosition::Right,
            )
            .await?;
            let (_, _, measurement) =
                prepare_refined_top_level_measurement(&compiled, &ctx).await?;

            let mut team_signals = Vec::new();
            collect_team_level_fixed_apply_signals(&measurement, &mut team_signals);
            assert!(
                !team_signals.is_empty(),
                "expected at least one team-level facet node in level2-right scenario"
            );

            let nodes_with_right_legend_slab = team_signals
                .iter()
                .filter(|(right_slab, _, _)| *right_slab > 0.0)
                .count();
            assert!(
                nodes_with_right_legend_slab > 0,
                "expected at least one team-level node to own right-side legend slab"
            );

            for (right_slab, has_coordinated_layout, remeasure_required) in team_signals
                .into_iter()
                .filter(|(right_slab, _, _)| *right_slab > 0.0)
            {
                assert!(
                    !has_coordinated_layout,
                    "team-level fixed leaf nodes should keep coordinated-layout cross-size rewrites disabled (right_slab={right_slab})"
                );
                assert!(
                    remeasure_required,
                    "team-level node with right legend slab should require retarget remeasure semantics (right_slab={right_slab})"
                );
            }
            Ok(())
        });
    }

    #[test]
    fn fixed_subplot_mode_collection_only_applies_layout_patches_without_resizing_leaves() {
        run_with_large_stack(|| async {
            let ctx = SessionContext::new();
            let compiled = compile_three_level_col_legend_sharing_plot_fixed_subplot(
                &ctx,
                LegendPosition::Right,
            )
            .await?;
            let (eval_ctx, evaluated_layout_spec, scale_builder, mut measurement) =
                prepare_top_level_measurement(&compiled, &ctx).await?;
            let provider = DynamicScaleProvider {
                builder: &scale_builder,
                plot: &compiled,
            };
            compiled
                .apply_layout_snapshot(
                    &LayoutSnapshot::Whole(WholeChartSnapshot::Coordination(
                        CoordinationCheckpoint::FinalPropagationComplete,
                    )),
                    &mut measurement,
                    &eval_ctx,
                    &evaluated_layout_spec,
                    &provider,
                    FacetSizingStrategy::FixedSubplot {
                        leaf_plot_width: 120.0,
                        leaf_plot_height: 90.0,
                    },
                    FacetCoordinationMode::FixedFullCycle,
                )
                .await?;

            let patch_count = count_coordinated_layout_patches(&measurement);
            assert!(
                patch_count > 0,
                "expected fixed full-cycle mode to apply at least one coordinated layout patch"
            );

            let mut leaf_widths = Vec::new();
            let mut leaf_heights = Vec::new();
            collect_leaf_plot_areas(&measurement, &mut leaf_widths, &mut leaf_heights);
            assert!(
                leaf_widths.iter().all(|w| (*w - 120.0).abs() <= 0.01),
                "fixed full-cycle path must keep leaf plot widths locked"
            );
            assert!(
                leaf_heights.iter().all(|h| (*h - 90.0).abs() <= 0.01),
                "fixed full-cycle path must keep leaf plot heights locked"
            );
            Ok(())
        });
    }

    #[test]
    fn fixed_subplot_mode_final_realization_applies_layout_patches_without_resizing_leaves() {
        run_with_large_stack(|| async {
            let ctx = SessionContext::new();
            let compiled = compile_three_level_col_legend_sharing_plot_fixed_subplot(
                &ctx,
                LegendPosition::Right,
            )
            .await?;
            let (eval_ctx, evaluated_layout_spec, scale_builder, mut measurement) =
                prepare_top_level_measurement(&compiled, &ctx).await?;
            let provider = DynamicScaleProvider {
                builder: &scale_builder,
                plot: &compiled,
            };
            compiled
                .apply_layout_snapshot(
                    &LayoutSnapshot::Final,
                    &mut measurement,
                    &eval_ctx,
                    &evaluated_layout_spec,
                    &provider,
                    FacetSizingStrategy::FixedSubplot {
                        leaf_plot_width: 120.0,
                        leaf_plot_height: 90.0,
                    },
                    FacetCoordinationMode::FixedFullCycle,
                )
                .await?;

            let patch_count = count_coordinated_layout_patches(&measurement);
            assert!(
                patch_count > 0,
                "expected fixed final realization to preserve coordinated layout patches"
            );

            let mut leaf_widths = Vec::new();
            let mut leaf_heights = Vec::new();
            collect_leaf_plot_areas(&measurement, &mut leaf_widths, &mut leaf_heights);
            assert!(
                leaf_widths.iter().all(|w| (*w - 120.0).abs() <= 0.01),
                "fixed final realization must keep leaf plot widths locked"
            );
            assert!(
                leaf_heights.iter().all(|h| (*h - 90.0).abs() <= 0.01),
                "fixed final realization must keep leaf plot heights locked"
            );
            Ok(())
        });
    }

    #[test]
    fn fixed_subplot_mode_has_no_main_axis_overlap_for_three_level_col_col_col_with_level2_right_legend()
     {
        run_with_large_stack(|| async {
            let ctx = SessionContext::new();
            let compiled = compile_three_level_col_legend_sharing_plot_fixed_subplot(
                &ctx,
                LegendPosition::Right,
            )
            .await?;
            let (_, _, measurement) =
                prepare_refined_top_level_measurement(&compiled, &ctx).await?;
            assert_no_fixed_subplot_main_axis_overlap(&measurement);
            Ok(())
        });
    }

    #[test]
    fn fixed_level2_right_legends_within_canvas() {
        run_with_large_stack(|| async {
            let ctx = SessionContext::new();
            let compiled = compile_three_level_col_legend_sharing_plot_fixed_subplot(
                &ctx,
                LegendPosition::Right,
            )
            .await?;
            let (_, _, measurement) =
                prepare_refined_top_level_measurement(&compiled, &ctx).await?;
            assert_legends_within_canvas(&measurement);
            Ok(())
        });
    }

    #[test]
    fn fixed_retarget_trace_recomputes_positions_after_remeasure() {
        run_with_large_stack(|| async {
            let ctx = SessionContext::new();
            let compiled = compile_three_level_col_legend_sharing_plot_fixed_subplot(
                &ctx,
                LegendPosition::Right,
            )
            .await?;
            let (_, _, measurement) =
                prepare_refined_top_level_measurement(&compiled, &ctx).await?;
            assert_fixed_positions_match_recomputed(&measurement);
            Ok(())
        });
    }

    #[test]
    fn fixed_subplot_mode_continuous_legend_is_visible_with_uniform_leaf_sizes() {
        run_with_large_stack(|| async {
            let ctx = SessionContext::new();
            let compiled =
                compile_nested_col_row_col_continuous_legend_plot_fixed_subplot(&ctx).await?;
            let (_, _, measurement) =
                prepare_refined_top_level_measurement(&compiled, &ctx).await?;

            let legend_measurement_count = count_legend_measurements_recursive(&measurement);
            assert!(
                legend_measurement_count > 0,
                "expected at least one legend measurement in fixed-subplot continuous legend scenario"
            );
            assert_legends_within_canvas(&measurement);

            let mut leaf_widths = Vec::new();
            let mut leaf_heights = Vec::new();
            collect_leaf_plot_areas(&measurement, &mut leaf_widths, &mut leaf_heights);
            assert!(
                !leaf_widths.is_empty(),
                "expected non-empty leaf measurements"
            );
            for width in leaf_widths {
                assert!(
                    (width - 110.0).abs() <= 0.01,
                    "fixed-subplot leaf width drifted from configured width: {width}"
                );
            }
            for height in leaf_heights {
                assert!(
                    (height - 80.0).abs() <= 0.01,
                    "fixed-subplot leaf height drifted from configured height: {height}"
                );
            }
            Ok(())
        });
    }

    #[tokio::test]
    async fn debug_layout_lines_option_enables_overlay_without_env() -> Result<(), AvengerChartError>
    {
        if crate::facet::debug::env_layout_overlay_enabled() {
            return Ok(());
        }

        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;
        let base_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Final,
                    debug_layout_lines: false,
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let debug_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Final,
                    debug_layout_lines: true,
                    ..EvaluationOptions::default()
                },
            )
            .await?;

        let base_plot_area_labels = collect_text_x_positions(&base_eval.scene_graph, "plot-area");
        let debug_plot_area_labels = collect_text_x_positions(&debug_eval.scene_graph, "plot-area");

        assert_eq!(
            base_plot_area_labels.len(),
            0,
            "baseline evaluation unexpectedly has layout debug labels"
        );
        assert!(
            !debug_plot_area_labels.is_empty(),
            "debug option should enable layout overlay labels"
        );

        Ok(())
    }

    #[tokio::test]
    async fn canvas_mode_measurement_matches_final_layout_bounds() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;
        let bounds = measurement.layout.plot_area_bounds();
        let delta_w = (measurement.plot_area_width - bounds.width).abs();
        let delta_h = (measurement.plot_area_height - bounds.height).abs();
        assert!(
            delta_w <= CompiledPlot::LAYOUT_REFINEMENT_EPSILON,
            "plot area width mismatch after refinement: measurement={} layout={} delta={}",
            measurement.plot_area_width,
            bounds.width,
            delta_w
        );
        assert!(
            delta_h <= CompiledPlot::LAYOUT_REFINEMENT_EPSILON,
            "plot area height mismatch after refinement: measurement={} layout={} delta={}",
            measurement.plot_area_height,
            bounds.height,
            delta_h
        );
        Ok(())
    }

    #[tokio::test]
    async fn level1_left_legend_slab_stays_on_child_facet() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_two_level_col_legend_sharing_plot(&ctx, LegendPosition::Left).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let outer_facet = facet_band_ref(&measurement)
            .expect("top-level legend sharing test should measure as FacetBandCoordMeasurement");
        let outer_left_slab =
            legend_slab_for_position(&outer_facet.coordinated_overflow, LegendPosition::Left);

        let first_non_empty = outer_facet
            .cells
            .iter()
            .find(|cell| !cell.plan.is_empty)
            .expect("expected non-empty outer facet cell");
        let child_facet = facet_band_ref(&first_non_empty.measurement)
            .expect("expected nested child facet measurement");
        let child_left_slab =
            legend_slab_for_position(&child_facet.coordinated_overflow, LegendPosition::Left);

        assert!(
            outer_left_slab <= 0.5,
            "outer facet should not reserve descendant left legend slab (found {})",
            outer_left_slab
        );
        assert!(
            child_left_slab > 1.0,
            "child facet should retain its own left legend slab (found {})",
            child_left_slab
        );
        Ok(())
    }

    #[tokio::test]
    async fn level1_bottom_legend_slab_stays_on_child_facet() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_two_level_col_legend_sharing_plot(&ctx, LegendPosition::Bottom).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let outer_facet = facet_band_ref(&measurement)
            .expect("top-level legend sharing test should measure as FacetBandCoordMeasurement");
        let outer_bottom_slab =
            legend_slab_for_position(&outer_facet.coordinated_overflow, LegendPosition::Bottom);

        let first_non_empty = outer_facet
            .cells
            .iter()
            .find(|cell| !cell.plan.is_empty)
            .expect("expected non-empty outer facet cell");
        let child_facet = facet_band_ref(&first_non_empty.measurement)
            .expect("expected nested child facet measurement");
        let child_bottom_slab =
            legend_slab_for_position(&child_facet.coordinated_overflow, LegendPosition::Bottom);

        assert!(
            outer_bottom_slab <= 0.5,
            "outer facet should not reserve descendant bottom legend slab (found {})",
            outer_bottom_slab
        );
        assert!(
            child_bottom_slab > 1.0,
            "child facet should retain its own bottom legend slab (found {})",
            child_bottom_slab
        );
        Ok(())
    }

    #[tokio::test]
    async fn level1_top_legend_slab_stays_on_child_facet() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_two_level_col_legend_sharing_plot(&ctx, LegendPosition::Top).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let outer_facet = facet_band_ref(&measurement)
            .expect("top-level legend sharing test should measure as FacetBandCoordMeasurement");
        let outer_top_slab =
            legend_slab_for_position(&outer_facet.coordinated_overflow, LegendPosition::Top);

        let first_non_empty = outer_facet
            .cells
            .iter()
            .find(|cell| !cell.plan.is_empty)
            .expect("expected non-empty outer facet cell");
        let child_facet = facet_band_ref(&first_non_empty.measurement)
            .expect("expected nested child facet measurement");
        let child_top_slab =
            legend_slab_for_position(&child_facet.coordinated_overflow, LegendPosition::Top);
        let first_child_subplot_height = child_facet
            .cells
            .iter()
            .find(|cell| !cell.plan.is_empty)
            .expect("expected non-empty child facet cell")
            .measurement
            .plot_area_height;
        let expected_child_subplot_height =
            (first_non_empty.measurement.plot_area_height - child_top_slab).max(1.0);

        assert!(
            outer_top_slab <= 0.5,
            "outer facet should not reserve descendant top legend slab (found {})",
            outer_top_slab
        );
        assert!(
            child_top_slab > 1.0,
            "child facet should retain its own top legend slab (found {})",
            child_top_slab
        );
        assert!(
            (first_child_subplot_height - expected_child_subplot_height).abs() <= 0.5,
            "child subplot height should preserve room for the child top legend slab: expected {expected_child_subplot_height}, found {first_child_subplot_height}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn row_facet_group_origins_include_main_axis_legend_start_slab()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_single_level_row_legend_plot(&ctx, LegendPosition::Left).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;

        let row_facet = facet_band_ref(&measurement)
            .expect("row facet origin invariant expects FacetBandCoordMeasurement");
        let legend_start =
            legend_slab_for_position(&row_facet.coordinated_overflow, LegendPosition::Left);
        let base_x = measurement.layout.plot_area_bounds().x;
        let base_y = measurement.layout.plot_area_bounds().y;
        let row_scale = measurement
            .scales
            .get("row")
            .expect("expected row scale for facet row origin invariant");
        let expected_y_starts: Vec<f32> = BandPositionIterator::from_scale(row_scale)?
            .map(|band| base_y + band.start())
            .collect();
        let row_origins = absolute_origins_for_named_groups(&evaluated.scene_graph, "facet_row_");
        assert!(
            !row_origins.is_empty(),
            "expected non-empty row facet groups in evaluated scene"
        );
        assert_eq!(
            row_origins.len(),
            expected_y_starts.len(),
            "row facet groups should match row band count"
        );

        for ((name, origin), expected_y) in row_origins.iter().zip(expected_y_starts.iter()) {
            assert!(
                (origin[0] - (base_x + legend_start)).abs() <= 1.0,
                "row facet group {} should include left legend start slab at x (origin_x={}, expected={})",
                name,
                origin[0],
                base_x + legend_start
            );
            assert!(
                (origin[1] - *expected_y).abs() <= 1.0,
                "row facet group {} should align to row band start y (origin_y={}, expected={})",
                name,
                origin[1],
                expected_y
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn col_facet_group_origins_include_main_axis_legend_start_slab()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_single_level_col_legend_plot(&ctx, LegendPosition::Top).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;

        let col_facet = facet_band_ref(&measurement)
            .expect("col facet origin invariant expects FacetBandCoordMeasurement");
        let legend_start =
            legend_slab_for_position(&col_facet.coordinated_overflow, LegendPosition::Top);
        let base_x = measurement.layout.plot_area_bounds().x;
        let base_y = measurement.layout.plot_area_bounds().y;
        let col_scale = measurement
            .scales
            .get("column")
            .expect("expected column scale for facet col origin invariant");
        let expected_x_starts: Vec<f32> = BandPositionIterator::from_scale(col_scale)?
            .map(|band| base_x + band.start())
            .collect();
        let col_origins = absolute_origins_for_named_groups(&evaluated.scene_graph, "facet_col_");
        assert!(
            !col_origins.is_empty(),
            "expected non-empty col facet groups in evaluated scene"
        );
        assert_eq!(
            col_origins.len(),
            expected_x_starts.len(),
            "col facet groups should match column band count"
        );

        for ((name, origin), expected_x) in col_origins.iter().zip(expected_x_starts.iter()) {
            assert!(
                (origin[1] - (base_y + legend_start)).abs() <= 1.0,
                "col facet group {} should include top legend start slab at y (origin_y={}, expected={})",
                name,
                origin[1],
                base_y + legend_start
            );
            assert!(
                (origin[0] - *expected_x).abs() <= 1.0,
                "col facet group {} should align to column band start x (origin_x={}, expected={})",
                name,
                origin[0],
                expected_x
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn level0_right_outer_labels_align_with_child_department_titles()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_two_level_col_legend_sharing_plot(&ctx, LegendPosition::Right).await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;

        let mut division_label_centers = Vec::new();
        for label in ["DivA", "DivB"] {
            let xs = collect_text_x_positions(&evaluated.scene_graph, label);
            assert_eq!(xs.len(), 1, "expected exactly one {:?} label", label);
            division_label_centers.push(xs[0]);
        }
        division_label_centers.sort_by(f32::total_cmp);

        let mut department_title_centers =
            collect_text_x_positions(&evaluated.scene_graph, "department");
        assert_eq!(
            department_title_centers.len(),
            2,
            "expected one inner 'department' title per outer division"
        );
        department_title_centers.sort_by(f32::total_cmp);

        for (division_x, department_x) in division_label_centers
            .iter()
            .zip(department_title_centers.iter())
        {
            assert!(
                (division_x - department_x).abs() <= 2.0,
                "outer division label should align to child department title center (division_x={}, department_x={})",
                division_x,
                department_x
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn level2_right_outer_labels_align_with_child_department_titles()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_three_level_col_legend_sharing_plot(&ctx, LegendPosition::Right).await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;

        let mut division_label_centers = Vec::new();
        for label in ["DivA", "DivB"] {
            let xs = collect_text_x_positions(&evaluated.scene_graph, label);
            assert_eq!(xs.len(), 1, "expected exactly one {:?} label", label);
            division_label_centers.push(xs[0]);
        }
        division_label_centers.sort_by(f32::total_cmp);

        let mut department_title_centers =
            collect_text_x_positions(&evaluated.scene_graph, "department");
        assert_eq!(
            department_title_centers.len(),
            2,
            "expected one inner 'department' title per outer division"
        );
        department_title_centers.sort_by(f32::total_cmp);

        for (division_x, department_x) in division_label_centers
            .iter()
            .zip(department_title_centers.iter())
        {
            assert!(
                (division_x - department_x).abs() <= 2.0,
                "outer division label should align to child department title center (division_x={}, department_x={})",
                division_x,
                department_x
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn shared_row_basic_right_owner_renders_each_row_label_once_globally()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_nested_shared_row_basic_plot(&ctx).await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;

        for label in ["R1", "R2", "R3"] {
            let xs = collect_text_x_positions(&evaluated.scene_graph, label);
            assert_eq!(
                xs.len(),
                1,
                "expected shared-row label {:?} to render exactly once on owning column",
                label
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn empty_cell_policy_controls_whether_empty_slots_render_subplots()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let hole_plot = compile_nested_shared_row_shared_both_plot_with_empty_policy(
            &ctx,
            FacetEmptyCellPolicy::Hole,
        )
        .await?;
        let hole_scene = hole_plot.evaluate(&ctx, None).await?;

        let ctx = SessionContext::new();
        let empty_subplot_plot = compile_nested_shared_row_shared_both_plot_with_empty_policy(
            &ctx,
            FacetEmptyCellPolicy::EmptySubplot,
        )
        .await?;
        let empty_subplot_scene = empty_subplot_plot.evaluate(&ctx, None).await?;

        let ctx = SessionContext::new();
        let auto_plot = compile_nested_shared_row_shared_both_plot_with_empty_policy(
            &ctx,
            FacetEmptyCellPolicy::Auto,
        )
        .await?;
        let auto_scene = auto_plot.evaluate(&ctx, None).await?;

        let hole_empty_groups =
            count_groups_with_name(&hole_scene.scene_graph, "facet_row_", "_empty");
        let empty_subplot_empty_groups =
            count_groups_with_name(&empty_subplot_scene.scene_graph, "facet_row_", "_empty");
        let auto_empty_groups =
            count_groups_with_name(&auto_scene.scene_graph, "facet_row_", "_empty");

        assert!(
            hole_empty_groups > 0,
            "hole policy should render explicit empty groups for empty slots"
        );
        assert_eq!(
            empty_subplot_empty_groups, 0,
            "empty subplot policy should render full subplot groups instead of *_empty placeholders"
        );
        assert_eq!(
            auto_empty_groups, hole_empty_groups,
            "auto policy should resolve to hole in this release"
        );

        Ok(())
    }

    #[tokio::test]
    async fn empty_subplot_policy_renders_structural_subplot_groups_for_empty_slots()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let hole_plot = compile_nested_shared_row_shared_both_plot_with_empty_policy(
            &ctx,
            FacetEmptyCellPolicy::Hole,
        )
        .await?;
        let hole_scene = hole_plot.evaluate(&ctx, None).await?;

        let ctx = SessionContext::new();
        let empty_subplot_plot = compile_nested_shared_row_shared_both_plot_with_empty_policy(
            &ctx,
            FacetEmptyCellPolicy::EmptySubplot,
        )
        .await?;
        let empty_subplot_scene = empty_subplot_plot.evaluate(&ctx, None).await?;

        let hole_empty_groups =
            count_groups_with_name(&hole_scene.scene_graph, "facet_row_", "_empty");
        let hole_subplot_groups = count_groups_with_prefix_excluding_suffix(
            &hole_scene.scene_graph,
            "facet_row_",
            "_empty",
        );
        let empty_subplot_groups = count_groups_with_prefix_excluding_suffix(
            &empty_subplot_scene.scene_graph,
            "facet_row_",
            "_empty",
        );

        assert!(
            hole_empty_groups > 0,
            "fixture should include hole placeholders so policy replacement can be validated"
        );
        assert!(
            empty_subplot_groups > hole_subplot_groups,
            "empty subplot policy should add subplot groups in slots that are holes under hole policy"
        );
        assert_eq!(
            empty_subplot_groups,
            hole_subplot_groups + hole_empty_groups,
            "empty subplot policy should replace each hole placeholder with a structural subplot group"
        );

        Ok(())
    }

    #[tokio::test]
    async fn data_empty_shared_cells_receive_coordinated_domain_extents()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_nested_shared_row_shared_both_plot(&ctx).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let outer_facet = facet_band_ref(&measurement)
            .expect("expected top-level FacetBandCoordMeasurement for shared-both probe");

        let mut covered_cell_count = 0usize;
        for outer_cell in &outer_facet.cells {
            let inner_row = facet_band_ref(&outer_cell.measurement)
                .expect("expected nested row FacetBandCoordMeasurement");

            for cell in &inner_row.cells {
                if !cell.plan.is_empty {
                    covered_cell_count += 1;
                    assert!(
                        cell.local_domain_extents.contains_key("x")
                            || cell.coordinated_domain_extents.contains_key("x"),
                        "shared cell {:?} missing local or coordinated x extent",
                        cell.plan.full_path
                    );
                    assert!(
                        cell.local_domain_extents.contains_key("y")
                            || cell.coordinated_domain_extents.contains_key("y"),
                        "shared cell {:?} missing local or coordinated y extent",
                        cell.plan.full_path
                    );
                }
            }
        }

        assert!(
            covered_cell_count > 0,
            "expected at least one non-hole shared cell with domain extent coverage"
        );

        Ok(())
    }

    #[test]
    fn jagged_group_local_shared_row_keeps_one_owner_column_per_group() {
        run_with_large_stack(|| async {
            let ctx = SessionContext::new();
            let compiled = compile_jagged_group_local_shared_row_plot(&ctx).await?;
            let evaluated = compiled.evaluate(&ctx, None).await?;

            let mut a_owner_xs = Vec::new();
            for label in ["A1", "A2", "A3"] {
                let xs = collect_text_x_positions(&evaluated.scene_graph, label);
                assert_eq!(
                    xs.len(),
                    1,
                    "expected group-A label {:?} to render once on group-local owner column",
                    label
                );
                a_owner_xs.push(xs[0]);
            }

            let mut b_owner_xs = Vec::new();
            for label in ["B1", "B2"] {
                let xs = collect_text_x_positions(&evaluated.scene_graph, label);
                assert_eq!(
                    xs.len(),
                    1,
                    "expected group-B label {:?} to render once on group-local owner column",
                    label
                );
                b_owner_xs.push(xs[0]);
            }

            let a_owner_x = a_owner_xs[0];
            for x in &a_owner_xs {
                assert!(
                    (*x - a_owner_x).abs() <= 1.0,
                    "expected group-A row labels to share one owner column x (got {:?})",
                    a_owner_xs
                );
            }

            let b_owner_x = b_owner_xs[0];
            for x in &b_owner_xs {
                assert!(
                    (*x - b_owner_x).abs() <= 1.0,
                    "expected group-B row labels to share one owner column x (got {:?})",
                    b_owner_xs
                );
            }

            Ok(())
        });
    }

    #[tokio::test]
    async fn nested_sparse_col_title_centers_over_coordinated_slot_span()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_nested_sparse_row_plot(&ctx).await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;
        let title_positions = collect_text_x_positions(&evaluated.scene_graph, "petal_width_bin");
        assert_eq!(
            title_positions.len(),
            1,
            "expected exactly one visible petal_width_bin title"
        );
        let title_x = title_positions[0];

        let narrow_label_positions = collect_text_x_positions(&evaluated.scene_graph, "narrow");
        assert_eq!(
            narrow_label_positions.len(),
            1,
            "expected exactly one visible narrow label in top sparse row"
        );
        let narrow_x = narrow_label_positions[0];
        let medium_positions = collect_text_x_positions(&evaluated.scene_graph, "medium");
        let wide_positions = collect_text_x_positions(&evaluated.scene_graph, "wide");
        assert!(
            !medium_positions.is_empty() && !wide_positions.is_empty(),
            "expected visible medium/wide labels to infer coordinated slot pitch"
        );
        let slot_step = (wide_positions[0] - medium_positions[0]).abs();
        assert!(
            slot_step > 1.0,
            "expected positive coordinated slot pitch, got {}",
            slot_step
        );
        let expected_title_x = narrow_x + 0.5 * slot_step;

        assert!(
            (title_x - expected_title_x).abs() <= 2.0,
            "expected petal_width_bin title x={} to align with coordinated slot midpoint {}",
            title_x,
            expected_title_x
        );

        Ok(())
    }

    #[tokio::test]
    async fn nested_plot_area_measurements_use_coord_aware_overflow()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;
        let (eval_ctx, _, measurement) =
            prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let outer_facet = facet_band_ref(&measurement)
            .expect("top-level deeply nested test should measure as FacetBandCoordMeasurement");

        let first_non_empty = outer_facet
            .cells
            .iter()
            .find(|cell| !cell.plan.is_empty)
            .expect("expected at least one non-empty outer facet cell");

        let child_measurement = &first_non_empty.measurement;
        let child_layout_spec = EvaluatedLayoutSpec {
            canvas: EvaluatedSizeMode::Auto,
            plot_area: EvaluatedSizeMode::Fixed {
                width: child_measurement.plot_area_width,
                height: child_measurement.plot_area_height,
            },
            margins: EvaluatedMargins {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            },
        };

        let child_plot = outer_facet.compiled_subplot.as_ref();
        let (_, coord_aware_layout, _) = child_plot
            .rebuild_layout_with_coord_overflow(
                &child_layout_spec,
                &child_measurement.scales,
                child_measurement.plot_area_width,
                child_measurement.plot_area_height,
                &child_measurement.params,
                Some(&first_non_empty.data_override),
                &eval_ctx.session_context,
                eval_ctx.facet_tree.as_ref(),
                &first_non_empty.plan.full_path,
                Some(child_measurement.coord_measurement.as_ref()),
            )
            .await?;
        let (_, no_coord_layout, _) = child_plot
            .rebuild_layout_with_coord_overflow(
                &child_layout_spec,
                &child_measurement.scales,
                child_measurement.plot_area_width,
                child_measurement.plot_area_height,
                &child_measurement.params,
                Some(&first_non_empty.data_override),
                &eval_ctx.session_context,
                eval_ctx.facet_tree.as_ref(),
                &first_non_empty.plan.full_path,
                None,
            )
            .await?;

        let guide_delta_coord = max_overflow_abs_delta(
            &child_measurement.layout.overflow,
            &coord_aware_layout.overflow,
        );
        let total_delta_coord = max_overflow_abs_delta(
            &child_measurement.layout.total_overflow,
            &coord_aware_layout.total_overflow,
        );
        let guide_delta_no_coord = max_overflow_abs_delta(
            &child_measurement.layout.overflow,
            &no_coord_layout.overflow,
        );
        let total_delta_no_coord = max_overflow_abs_delta(
            &child_measurement.layout.total_overflow,
            &no_coord_layout.total_overflow,
        );
        assert!(
            guide_delta_coord <= 1.0,
            "nested child measurement guide overflow should come from coord-aware pass (delta={})",
            guide_delta_coord
        );
        assert!(
            total_delta_coord <= 1.0,
            "nested child measurement total overflow should come from coord-aware pass (delta={})",
            total_delta_coord
        );
        assert!(
            guide_delta_no_coord >= guide_delta_coord + 0.5
                || total_delta_no_coord >= total_delta_coord + 0.5,
            "coord-aware overflow should be materially closer than coord-agnostic overflow (guide coord={} no_coord={}, total coord={} no_coord={})",
            guide_delta_coord,
            guide_delta_no_coord,
            total_delta_coord,
            total_delta_no_coord
        );
        Ok(())
    }

    #[tokio::test]
    async fn deeply_nested_outer_padding_inner_not_pathological() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let outer_facet = facet_band_ref(&measurement)
            .expect("top-level deeply nested test should measure as FacetBandCoordMeasurement");
        let active_layout = outer_facet
            .coordinated_layout
            .as_ref()
            .unwrap_or(&outer_facet.local_layout);

        assert!(
            active_layout.padding_inner_px < 180.0,
            "outer facet padding_inner_px regressed into pathological range: {}",
            active_layout.padding_inner_px
        );
        Ok(())
    }
}
