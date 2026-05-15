//! Sizing-mode policies for the shared facet coordination driver.
//!
//! The coordination pipeline is the same across facet sizing modes: collect
//! requirements, retarget geometry, reconcile requirements, then propagate final
//! plot-area and scale-range updates. Strategies keep the small sizing-mode
//! differences explicit without forking that pipeline.

use crate::{
    coords::FacetAxis,
    error::AvengerChartError,
    facet::{
        coord::{
            FacetBandCoordMeasurement, FacetBandCoordMeasurementPlotAreaSized,
            facet_band_canvas_mut as facet_band_canvas_mut_from_coord,
            facet_band_canvas_ref as facet_band_canvas_ref_from_coord,
            facet_band_plot_area_sized_mut as facet_band_plot_area_sized_mut_from_coord,
            facet_band_plot_area_sized_ref as facet_band_plot_area_sized_ref_from_coord,
            retarget_measurement_plot_area_no_remeasure, retarget_scale_ranges_for_plot_area,
        },
        coordination_plans::{
            BandRetargetAction, CellRetargetAction, FinalPropagationChildPlan,
            FinalPropagationPlan, FinalPropagationTrace, RetargetNodeActions,
            RetargetNodeRequirements, RetargetPlan, RetargetTrace,
        },
        layout_plan::FacetCellPlan,
    },
    plot::compiled::{CompiledPlot, ComponentsMeasurement},
    render::{EvaluationContext, context::FacetRuntimeSizingMode},
};
use tracing::trace;

#[derive(Clone, Copy)]
pub(crate) enum FacetBandRef<'a> {
    CanvasFit(&'a FacetBandCoordMeasurement),
    PlotAreaSized(&'a FacetBandCoordMeasurementPlotAreaSized),
}

impl<'a> FacetBandRef<'a> {
    pub(crate) fn base(self) -> &'a FacetBandCoordMeasurement {
        match self {
            Self::CanvasFit(facet_band) => facet_band,
            Self::PlotAreaSized(facet_band) => &facet_band.base,
        }
    }
}

pub(crate) enum FacetBandMut<'a> {
    CanvasFit(&'a mut FacetBandCoordMeasurement),
    PlotAreaSized(&'a mut FacetBandCoordMeasurementPlotAreaSized),
}

impl<'a> FacetBandMut<'a> {
    pub(crate) fn base(&self) -> &FacetBandCoordMeasurement {
        match self {
            Self::CanvasFit(facet_band) => facet_band,
            Self::PlotAreaSized(facet_band) => &facet_band.base,
        }
    }

    pub(crate) fn base_mut(&mut self) -> &mut FacetBandCoordMeasurement {
        match self {
            Self::CanvasFit(facet_band) => facet_band,
            Self::PlotAreaSized(facet_band) => &mut facet_band.base,
        }
    }

    pub(crate) fn recompute_explicit_placement_if_plot_area_sized(&mut self) {
        if let Self::PlotAreaSized(facet_band) = self {
            facet_band.recompute_explicit_placement();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FinalChildResizePolicy {
    pub(crate) allow_plot_area_resize: bool,
    pub(crate) allow_scale_range_retarget: bool,
}

pub(crate) trait FacetSizingCoordinationStrategy {
    const LABEL: &'static str;

    fn facet_band_ref(measurement: &ComponentsMeasurement) -> Option<FacetBandRef<'_>>;

    fn facet_band_mut(measurement: &mut ComponentsMeasurement) -> Option<FacetBandMut<'_>>;

    fn before_run(
        _measurement: &ComponentsMeasurement,
        _eval_ctx: &EvaluationContext,
    ) -> Result<(), AvengerChartError> {
        Ok(())
    }

    fn after_retarget_trace(
        _measurement: &ComponentsMeasurement,
        _eval_ctx: &EvaluationContext,
        _plan: &RetargetPlan,
        _trace: &RetargetTrace,
    ) -> Result<(), AvengerChartError> {
        Ok(())
    }

    fn after_final_propagation_trace(
        _measurement: &ComponentsMeasurement,
        _eval_ctx: &EvaluationContext,
        _plan: &FinalPropagationPlan,
        _trace: &FinalPropagationTrace,
    ) -> Result<(), AvengerChartError> {
        Ok(())
    }

    fn refresh_placement_after_requirement_patch(
        _facet_band: &mut FacetBandMut<'_>,
        _patch_applied: bool,
    ) {
    }

    fn refresh_placement_after_retarget_node(_facet_band: &mut FacetBandMut<'_>) {}

    fn refresh_placement_after_final_propagation_node(_facet_band: &mut FacetBandMut<'_>) {}

    fn build_retarget_actions(
        facet_band: FacetBandRef<'_>,
        requirements: &RetargetNodeRequirements,
    ) -> RetargetNodeActions;

    fn build_retarget_actions_for_eval(
        facet_band: FacetBandRef<'_>,
        requirements: &RetargetNodeRequirements,
        _eval_ctx: &EvaluationContext,
    ) -> RetargetNodeActions {
        Self::build_retarget_actions(facet_band, requirements)
    }

    fn set_child_parent_bandwidth_if_same_axis(
        child: &mut ComponentsMeasurement,
        parent_axis: FacetAxis,
        cross_size: f32,
    ) -> bool;

    fn final_child_resize_policy(
        axis: FacetAxis,
        parent_cross_size_target: Option<f32>,
        child: &ComponentsMeasurement,
    ) -> FinalChildResizePolicy;

    fn apply_final_propagation_child_update(
        axis: FacetAxis,
        child: &mut ComponentsMeasurement,
        cell_plan: Option<&FacetCellPlan>,
        compiled_subplot: &CompiledPlot,
        eval_ctx: &EvaluationContext,
        child_plan: &FinalPropagationChildPlan,
    ) -> Result<(bool, usize), AvengerChartError>;
}

pub(crate) struct CanvasFitCoordinationStrategy;

impl FacetSizingCoordinationStrategy for CanvasFitCoordinationStrategy {
    const LABEL: &'static str = "canvas-fit";

    fn facet_band_ref(measurement: &ComponentsMeasurement) -> Option<FacetBandRef<'_>> {
        facet_band_canvas_ref_from_coord(measurement.coord_measurement.as_ref())
            .map(FacetBandRef::CanvasFit)
    }

    fn facet_band_mut(measurement: &mut ComponentsMeasurement) -> Option<FacetBandMut<'_>> {
        facet_band_canvas_mut_from_coord(measurement.coord_measurement.as_mut())
            .map(FacetBandMut::CanvasFit)
    }

    fn build_retarget_actions(
        _facet_band: FacetBandRef<'_>,
        requirements: &RetargetNodeRequirements,
    ) -> RetargetNodeActions {
        let band_action = if requirements.layout_changed {
            BandRetargetAction::ApplyCoordinatedLayout
        } else {
            BandRetargetAction::Preserve
        };
        let retarget_children =
            requirements.has_legend_overflow || requirements.has_coordinated_extents;
        let legend_slab_total = requirements.legend_main_axis_slab.total();
        let child_actions = requirements
            .child_plot_areas
            .iter()
            .enumerate()
            .map(|(idx, plot_area)| {
                if !retarget_children {
                    return CellRetargetAction::Preserve;
                }

                let original_main_size = match requirements.axis {
                    FacetAxis::Column => plot_area.height,
                    FacetAxis::Row => plot_area.width,
                };
                let main_axis_size = (original_main_size - legend_slab_total).max(1.0);
                let child_has_domains = requirements
                    .child_has_coordinated_extents
                    .get(idx)
                    .copied()
                    .unwrap_or(false);
                if child_has_domains {
                    CellRetargetAction::RetargetPlotAreaAndDomains { main_axis_size }
                } else {
                    CellRetargetAction::RetargetPlotArea { main_axis_size }
                }
            })
            .collect();

        RetargetNodeActions {
            node_id: requirements.node_id.clone(),
            axis: requirements.axis,
            band_action,
            child_actions,
        }
    }

    fn set_child_parent_bandwidth_if_same_axis(
        child: &mut ComponentsMeasurement,
        parent_axis: FacetAxis,
        cross_size: f32,
    ) -> bool {
        let Some(child_facet_band) = child
            .coord_measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
        else {
            return false;
        };
        if child_facet_band.axis != parent_axis {
            return false;
        }
        child_facet_band.set_parent_bandwidth_value(cross_size);
        true
    }

    fn final_child_resize_policy(
        _axis: FacetAxis,
        _parent_cross_size_target: Option<f32>,
        _child: &ComponentsMeasurement,
    ) -> FinalChildResizePolicy {
        FinalChildResizePolicy {
            allow_plot_area_resize: true,
            allow_scale_range_retarget: true,
        }
    }

    fn apply_final_propagation_child_update(
        axis: FacetAxis,
        child: &mut ComponentsMeasurement,
        cell_plan: Option<&FacetCellPlan>,
        compiled_subplot: &CompiledPlot,
        eval_ctx: &EvaluationContext,
        child_plan: &FinalPropagationChildPlan,
    ) -> Result<(bool, usize), AvengerChartError> {
        let target_plot_area_width = child_plan
            .target_plot_area_width
            .unwrap_or(child.plot_area_width)
            .max(1.0);
        let target_plot_area_height = child_plan
            .target_plot_area_height
            .unwrap_or(child.plot_area_height)
            .max(1.0);
        let plot_area_adjusted = (child.plot_area_width - target_plot_area_width).abs() > 0.01
            || (child.plot_area_height - target_plot_area_height).abs() > 0.01;

        update_band_scale_range(axis, child, child_plan);

        if !plot_area_adjusted {
            return Ok((false, 0));
        }

        let cell_plan = cell_plan.ok_or_else(|| {
            AvengerChartError::InternalError(
                "canvas-fit final propagation requires a facet cell plan".to_string(),
            )
        })?;
        trace!(
            old_width = child.plot_area_width,
            old_height = child.plot_area_height,
            new_width = target_plot_area_width,
            new_height = target_plot_area_height,
            "coordinate_facet_measurement_tree retargeting child plot area through layout metadata"
        );
        retarget_measurement_plot_area_no_remeasure(
            child,
            compiled_subplot,
            eval_ctx,
            &cell_plan.full_path,
            target_plot_area_width,
            target_plot_area_height,
        )?;

        Ok((true, 1))
    }
}

pub(crate) struct PlotAreaSizedCoordinationStrategy;

impl FacetSizingCoordinationStrategy for PlotAreaSizedCoordinationStrategy {
    const LABEL: &'static str = "plot-area-sized";

    fn facet_band_ref(measurement: &ComponentsMeasurement) -> Option<FacetBandRef<'_>> {
        facet_band_plot_area_sized_ref_from_coord(measurement.coord_measurement.as_ref())
            .map(FacetBandRef::PlotAreaSized)
    }

    fn facet_band_mut(measurement: &mut ComponentsMeasurement) -> Option<FacetBandMut<'_>> {
        facet_band_plot_area_sized_mut_from_coord(measurement.coord_measurement.as_mut())
            .map(FacetBandMut::PlotAreaSized)
    }

    fn before_run(
        measurement: &ComponentsMeasurement,
        _eval_ctx: &EvaluationContext,
    ) -> Result<(), AvengerChartError> {
        assert_no_canvas_fit_measurements_in_plot_area_sized_tree(measurement);
        Ok(())
    }

    fn after_retarget_trace(
        measurement: &ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        _plan: &RetargetPlan,
        _trace: &RetargetTrace,
    ) -> Result<(), AvengerChartError> {
        assert_fixed_leaf_plot_sizes_for_eval_ctx(measurement, eval_ctx, "retarget");
        Ok(())
    }

    fn after_final_propagation_trace(
        measurement: &ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        _plan: &FinalPropagationPlan,
        _trace: &FinalPropagationTrace,
    ) -> Result<(), AvengerChartError> {
        assert_fixed_leaf_plot_sizes_for_eval_ctx(measurement, eval_ctx, "final propagation");
        Ok(())
    }

    fn refresh_placement_after_requirement_patch(
        facet_band: &mut FacetBandMut<'_>,
        patch_applied: bool,
    ) {
        if patch_applied {
            facet_band.recompute_explicit_placement_if_plot_area_sized();
        }
    }

    fn refresh_placement_after_retarget_node(facet_band: &mut FacetBandMut<'_>) {
        facet_band.recompute_explicit_placement_if_plot_area_sized();
    }

    fn refresh_placement_after_final_propagation_node(facet_band: &mut FacetBandMut<'_>) {
        facet_band.recompute_explicit_placement_if_plot_area_sized();
    }

    fn build_retarget_actions(
        _facet_band: FacetBandRef<'_>,
        requirements: &RetargetNodeRequirements,
    ) -> RetargetNodeActions {
        let child_actions = requirements
            .child_has_coordinated_extents
            .iter()
            .map(|has_domains| {
                if *has_domains {
                    CellRetargetAction::RebuildDomains
                } else {
                    CellRetargetAction::Preserve
                }
            })
            .collect();

        RetargetNodeActions {
            node_id: requirements.node_id.clone(),
            axis: requirements.axis,
            band_action: BandRetargetAction::Preserve,
            child_actions,
        }
    }

    fn set_child_parent_bandwidth_if_same_axis(
        child: &mut ComponentsMeasurement,
        parent_axis: FacetAxis,
        cross_size: f32,
    ) -> bool {
        let Some(child_facet_band) = child
            .coord_measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurementPlotAreaSized>()
        else {
            return false;
        };
        if child_facet_band.axis != parent_axis {
            return false;
        }
        child_facet_band.set_parent_bandwidth_value(cross_size);
        true
    }

    fn final_child_resize_policy(
        _axis: FacetAxis,
        _parent_cross_size_target: Option<f32>,
        child: &ComponentsMeasurement,
    ) -> FinalChildResizePolicy {
        FinalChildResizePolicy {
            allow_plot_area_resize: Self::facet_band_ref(child).is_some(),
            allow_scale_range_retarget: true,
        }
    }

    fn apply_final_propagation_child_update(
        axis: FacetAxis,
        child: &mut ComponentsMeasurement,
        _cell_plan: Option<&FacetCellPlan>,
        _compiled_subplot: &CompiledPlot,
        _eval_ctx: &EvaluationContext,
        child_plan: &FinalPropagationChildPlan,
    ) -> Result<(bool, usize), AvengerChartError> {
        let mut plot_area_adjusted = false;
        if child_plan.adjust_plot_area {
            if let Some(new_width) = child_plan.target_plot_area_width
                && (child.plot_area_width - new_width).abs() > 0.01
            {
                child.plot_area_width = new_width;
                plot_area_adjusted = true;
            }
            if let Some(new_height) = child_plan.target_plot_area_height
                && (child.plot_area_height - new_height).abs() > 0.01
            {
                child.plot_area_height = new_height;
                plot_area_adjusted = true;
            }
        }

        update_band_scale_range(axis, child, child_plan);

        if (child.plot_area_width - child_plan.old_plot_area_width).abs() <= 0.01
            && (child.plot_area_height - child_plan.old_plot_area_height).abs() <= 0.01
        {
            return Ok((plot_area_adjusted, 0));
        }

        let retarget_count = retarget_scale_ranges_for_plot_area(
            &mut child.scales,
            child.plot_area_width,
            child.plot_area_height,
        );
        Ok((plot_area_adjusted, retarget_count))
    }
}

pub(crate) struct DimensionPolicyCoordinationStrategy;

impl FacetSizingCoordinationStrategy for DimensionPolicyCoordinationStrategy {
    const LABEL: &'static str = "dimension-policy";

    fn facet_band_ref(measurement: &ComponentsMeasurement) -> Option<FacetBandRef<'_>> {
        facet_band_plot_area_sized_ref_from_coord(measurement.coord_measurement.as_ref())
            .map(FacetBandRef::PlotAreaSized)
            .or_else(|| {
                facet_band_canvas_ref_from_coord(measurement.coord_measurement.as_ref())
                    .map(FacetBandRef::CanvasFit)
            })
    }

    fn facet_band_mut(measurement: &mut ComponentsMeasurement) -> Option<FacetBandMut<'_>> {
        if facet_band_plot_area_sized_ref_from_coord(measurement.coord_measurement.as_ref())
            .is_some()
        {
            return facet_band_plot_area_sized_mut_from_coord(
                measurement.coord_measurement.as_mut(),
            )
            .map(FacetBandMut::PlotAreaSized);
        }
        facet_band_canvas_mut_from_coord(measurement.coord_measurement.as_mut())
            .map(FacetBandMut::CanvasFit)
    }

    fn refresh_placement_after_requirement_patch(
        facet_band: &mut FacetBandMut<'_>,
        patch_applied: bool,
    ) {
        if patch_applied {
            facet_band.recompute_explicit_placement_if_plot_area_sized();
        }
    }

    fn refresh_placement_after_retarget_node(facet_band: &mut FacetBandMut<'_>) {
        facet_band.recompute_explicit_placement_if_plot_area_sized();
    }

    fn refresh_placement_after_final_propagation_node(facet_band: &mut FacetBandMut<'_>) {
        facet_band.recompute_explicit_placement_if_plot_area_sized();
    }

    fn build_retarget_actions(
        facet_band: FacetBandRef<'_>,
        requirements: &RetargetNodeRequirements,
    ) -> RetargetNodeActions {
        CanvasFitCoordinationStrategy::build_retarget_actions(facet_band, requirements)
    }

    fn build_retarget_actions_for_eval(
        _facet_band: FacetBandRef<'_>,
        requirements: &RetargetNodeRequirements,
        eval_ctx: &EvaluationContext,
    ) -> RetargetNodeActions {
        let policy = eval_ctx.facet_runtime_sizing_mode().policy();
        let band_action = if policy
            .facet_band_dimension(requirements.axis)
            .is_canvas_constrained()
            && requirements.layout_changed
        {
            BandRetargetAction::ApplyCoordinatedLayout
        } else {
            BandRetargetAction::Preserve
        };

        let shrink_for_legend = requirements.has_legend_overflow
            && policy
                .facet_orthogonal_dimension(requirements.axis)
                .is_canvas_constrained();
        let legend_slab_total = requirements.legend_main_axis_slab.total();
        let child_actions = requirements
            .child_plot_areas
            .iter()
            .enumerate()
            .map(|(idx, plot_area)| {
                let child_has_domains = requirements
                    .child_has_coordinated_extents
                    .get(idx)
                    .copied()
                    .unwrap_or(false);
                match (shrink_for_legend, child_has_domains) {
                    (false, false) => CellRetargetAction::Preserve,
                    (false, true) => CellRetargetAction::RebuildDomains,
                    (true, false) | (true, true) => {
                        let original_main_size = match requirements.axis {
                            FacetAxis::Column => plot_area.height,
                            FacetAxis::Row => plot_area.width,
                        };
                        let main_axis_size = (original_main_size - legend_slab_total).max(1.0);
                        if child_has_domains {
                            CellRetargetAction::RetargetPlotAreaAndDomains { main_axis_size }
                        } else {
                            CellRetargetAction::RetargetPlotArea { main_axis_size }
                        }
                    }
                }
            })
            .collect();

        RetargetNodeActions {
            node_id: requirements.node_id.clone(),
            axis: requirements.axis,
            band_action,
            child_actions,
        }
    }

    fn set_child_parent_bandwidth_if_same_axis(
        child: &mut ComponentsMeasurement,
        parent_axis: FacetAxis,
        cross_size: f32,
    ) -> bool {
        if let Some(child_facet_band) = child
            .coord_measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurementPlotAreaSized>()
            && child_facet_band.axis == parent_axis
        {
            child_facet_band.set_parent_bandwidth_value(cross_size);
            return true;
        }
        let Some(child_facet_band) = child
            .coord_measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
        else {
            return false;
        };
        if child_facet_band.axis != parent_axis {
            return false;
        }
        child_facet_band.set_parent_bandwidth_value(cross_size);
        true
    }

    fn final_child_resize_policy(
        _axis: FacetAxis,
        _parent_cross_size_target: Option<f32>,
        _child: &ComponentsMeasurement,
    ) -> FinalChildResizePolicy {
        FinalChildResizePolicy {
            allow_plot_area_resize: true,
            allow_scale_range_retarget: true,
        }
    }

    fn apply_final_propagation_child_update(
        axis: FacetAxis,
        child: &mut ComponentsMeasurement,
        cell_plan: Option<&FacetCellPlan>,
        compiled_subplot: &CompiledPlot,
        eval_ctx: &EvaluationContext,
        child_plan: &FinalPropagationChildPlan,
    ) -> Result<(bool, usize), AvengerChartError> {
        let policy = eval_ctx.facet_runtime_sizing_mode().policy();
        if policy.facet_band_dimension(axis).is_canvas_constrained() {
            CanvasFitCoordinationStrategy::apply_final_propagation_child_update(
                axis,
                child,
                cell_plan,
                compiled_subplot,
                eval_ctx,
                child_plan,
            )
        } else {
            let _ = (child, cell_plan, compiled_subplot, child_plan);
            Ok((false, 0))
        }
    }
}

fn update_band_scale_range(
    axis: FacetAxis,
    child: &mut ComponentsMeasurement,
    child_plan: &FinalPropagationChildPlan,
) {
    if child_plan.update_band_range
        && let (Some(band_scale), Some(range_end)) = (
            child.scales.get_mut(axis.scale_name()),
            child_plan.target_band_range_end,
        )
    {
        let updated_config = band_scale
            .configured()
            .clone()
            .with_range_interval((0.0, range_end));
        band_scale.set_configured(updated_config);
    }
}

fn assert_no_canvas_fit_measurements_in_plot_area_sized_tree(measurement: &ComponentsMeasurement) {
    debug_assert!(
        facet_band_canvas_ref_from_coord(measurement.coord_measurement.as_ref()).is_none(),
        "plot-area-sized coordinator must not traverse canvas-fit facet measurements"
    );

    if let Some(plot_area_sized_facet) =
        facet_band_plot_area_sized_ref_from_coord(measurement.coord_measurement.as_ref())
    {
        for child in plot_area_sized_facet.child_measurements_iter() {
            assert_no_canvas_fit_measurements_in_plot_area_sized_tree(child);
        }
    }
}

fn assert_fixed_leaf_plot_sizes_for_eval_ctx(
    measurement: &ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    stage: &str,
) {
    if let FacetRuntimeSizingMode::PlotAreaSized {
        leaf_plot_width,
        leaf_plot_height,
    } = eval_ctx.facet_runtime_sizing_mode()
    {
        assert_fixed_leaf_plot_sizes(measurement, leaf_plot_width, leaf_plot_height, stage);
    }
}

fn assert_fixed_leaf_plot_sizes(
    measurement: &ComponentsMeasurement,
    expected_leaf_plot_width: f32,
    expected_leaf_plot_height: f32,
    stage: &str,
) {
    if let Some(plot_area_sized_facet) =
        facet_band_plot_area_sized_ref_from_coord(measurement.coord_measurement.as_ref())
    {
        for child in plot_area_sized_facet.child_measurements_iter() {
            assert_fixed_leaf_plot_sizes(
                child,
                expected_leaf_plot_width,
                expected_leaf_plot_height,
                stage,
            );
        }
    } else {
        debug_assert!(
            (measurement.plot_area_width - expected_leaf_plot_width).abs() <= 0.01,
            "plot-area-sized leaf width drifted at stage {stage}: width={}, expected={}",
            measurement.plot_area_width,
            expected_leaf_plot_width
        );
        debug_assert!(
            (measurement.plot_area_height - expected_leaf_plot_height).abs() <= 0.01,
            "plot-area-sized leaf height drifted at stage {stage}: height={}, expected={}",
            measurement.plot_area_height,
            expected_leaf_plot_height
        );
    }
}
