//! Sizing policy helpers for the facet coordination driver.
//!
//! The coordination pipeline is the same across facet sizing modes: collect
//! requirements, retarget geometry, reconcile requirements, then propagate final
//! plot-area and scale-range updates. This module keeps the small sizing-policy
//! decisions explicit without forking that pipeline.

use crate::{
    coords::FacetAxis,
    error::AvengerChartError,
    facet::{
        coord::{
            FacetBandCoordMeasurement, facet_band_mut as facet_band_mut_from_coord,
            facet_band_ref as facet_band_ref_from_coord,
            retarget_measurement_plot_area_policy_no_remeasure,
        },
        coordination_plans::{
            BandRetargetAction, CellRetargetAction, FinalPropagationChildPlan, PlotAreaSize,
            PlotAreaTarget, RetargetNodeActions, RetargetNodeRequirements,
        },
        layout_plan::FacetCellPlan,
    },
    plot::compiled::{CompiledPlot, ComponentsMeasurement},
    render::EvaluationContext,
};

#[derive(Clone, Copy)]
pub(crate) struct FacetBandRef<'a> {
    base: &'a FacetBandCoordMeasurement,
}

impl<'a> FacetBandRef<'a> {
    pub(crate) fn new(base: &'a FacetBandCoordMeasurement) -> Self {
        Self { base }
    }

    pub(crate) fn base(self) -> &'a FacetBandCoordMeasurement {
        self.base
    }
}

pub(crate) struct FacetBandMut<'a> {
    base: &'a mut FacetBandCoordMeasurement,
}

impl<'a> FacetBandMut<'a> {
    pub(crate) fn new(base: &'a mut FacetBandCoordMeasurement) -> Self {
        Self { base }
    }

    pub(crate) fn base(&self) -> &FacetBandCoordMeasurement {
        self.base
    }

    pub(crate) fn base_mut(&mut self) -> &mut FacetBandCoordMeasurement {
        self.base
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FinalChildResizePolicy {
    pub(crate) allow_width_resize: bool,
    pub(crate) allow_height_resize: bool,
    pub(crate) allow_x_range_retarget: bool,
    pub(crate) allow_y_range_retarget: bool,
}

impl FinalChildResizePolicy {
    pub(crate) fn allow_band_range_retarget(self, axis: FacetAxis) -> bool {
        match axis {
            FacetAxis::Column => self.allow_x_range_retarget,
            FacetAxis::Row => self.allow_y_range_retarget,
        }
    }
}

fn facet_child_plot_area_target(
    axis: FacetAxis,
    plot_area: PlotAreaSize,
    target_subplot_cross_size: f32,
    target_band_dimension: bool,
    target_orthogonal_dimension: bool,
    legend_main_axis_slab: f32,
) -> Option<PlotAreaTarget> {
    let mut target = PlotAreaTarget::default();
    if target_band_dimension {
        match axis {
            FacetAxis::Column => target.width = Some(target_subplot_cross_size.max(1.0)),
            FacetAxis::Row => target.height = Some(target_subplot_cross_size.max(1.0)),
        }
    }
    if target_orthogonal_dimension {
        match axis {
            FacetAxis::Column => {
                target.height = Some((plot_area.height - legend_main_axis_slab).max(1.0));
            }
            FacetAxis::Row => {
                target.width = Some((plot_area.width - legend_main_axis_slab).max(1.0));
            }
        }
    }
    target.has_any_target().then_some(target)
}

pub(crate) struct FacetCoordinationPolicy;

impl FacetCoordinationPolicy {
    pub(crate) const LABEL: &'static str = "facet-coordination-policy";

    pub(crate) fn facet_band_ref(measurement: &ComponentsMeasurement) -> Option<FacetBandRef<'_>> {
        facet_band_ref_from_coord(measurement.coord_measurement.as_ref()).map(FacetBandRef::new)
    }

    pub(crate) fn facet_band_mut(
        measurement: &mut ComponentsMeasurement,
    ) -> Option<FacetBandMut<'_>> {
        facet_band_mut_from_coord(measurement.coord_measurement.as_mut()).map(FacetBandMut::new)
    }

    pub(crate) fn build_retarget_actions(
        facet_band: FacetBandRef<'_>,
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
        let band_dimension_canvas = policy
            .facet_band_dimension(requirements.axis)
            .is_canvas_constrained();
        let legend_slab_total = requirements.legend_main_axis_slab.total();
        // Decision-time slot-target probe (site b): the bandwidth-derived
        // cross-size target vs each real cell's band-axis solved track
        // size, tagged with whether the band action applies it.
        // Diagnostics only.
        if crate::facet::tree_solve::slot_target_probe_enabled()
            && requirements.layout_changed
            && let Some(slots) = facet_band.base().solution_cell_slots()
        {
            for (cell_index, slot) in slots.iter().enumerate() {
                let slot_main = match requirements.axis {
                    FacetAxis::Column => slot.width,
                    FacetAxis::Row => slot.height,
                };
                let is_nested = facet_band.base().cells.get(cell_index).is_some_and(|cell| {
                    crate::facet::coord::facet_band_ref(cell.measurement.coord_measurement.as_ref())
                        .is_some()
                });
                tracing::info!(
                    target: "avenger_chart::facet::slot_probe",
                    site = "cross_size",
                    node = ?requirements.node_id.path,
                    axis = ?requirements.axis,
                    explicit = facet_band.base().uses_explicit_placement(),
                    nested = is_nested,
                    applied = matches!(band_action, BandRetargetAction::ApplyCoordinatedLayout),
                    cell = cell_index,
                    policy = requirements.target_subplot_cross_size,
                    slot = slot_main,
                    delta = (requirements.target_subplot_cross_size - slot_main).abs(),
                    "slot-target probe"
                );
            }
        }
        let child_actions = requirements
            .child_plot_areas
            .iter()
            .enumerate()
            .map(|(cell_index, plot_area)| {
                let plot_area_target = facet_child_plot_area_target(
                    requirements.axis,
                    *plot_area,
                    requirements.target_subplot_cross_size,
                    band_dimension_canvas && shrink_for_legend,
                    shrink_for_legend,
                    legend_slab_total,
                );
                // Decision-time slot-target probe (site a): the emitted
                // policy target vs the cell's solved slot extent.
                // Diagnostics only.
                if crate::facet::tree_solve::slot_target_probe_enabled()
                    && let Some(target) = &plot_area_target
                    && let Some(slot) = facet_band
                        .base()
                        .solution_cell_slots()
                        .and_then(|slots| slots.get(cell_index))
                {
                    let is_nested = facet_band.base().cells.get(cell_index).is_some_and(|cell| {
                        crate::facet::coord::facet_band_ref(
                            cell.measurement.coord_measurement.as_ref(),
                        )
                        .is_some()
                    });
                    for (dimension, policy_value, slot_value) in [
                        ("width", target.width, slot.width),
                        ("height", target.height, slot.height),
                    ] {
                        if let Some(policy_value) = policy_value {
                            tracing::info!(
                                target: "avenger_chart::facet::slot_probe",
                                site = "retarget_cell",
                                node = ?requirements.node_id.path,
                                axis = ?requirements.axis,
                                explicit = facet_band.base().uses_explicit_placement(),
                                nested = is_nested,
                                band_dimension_canvas,
                                shrink_for_legend,
                                cell = cell_index,
                                dimension,
                                policy = policy_value,
                                slot = slot_value,
                                delta = (policy_value - slot_value).abs(),
                                "slot-target probe"
                            );
                        }
                    }
                }
                match plot_area_target {
                    None => CellRetargetAction::preserve(),
                    Some(target) => CellRetargetAction::retarget_plot_area(target),
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

    pub(crate) fn set_child_parent_bandwidth_if_same_axis(
        child: &mut ComponentsMeasurement,
        parent_axis: FacetAxis,
        cross_size: f32,
    ) -> bool {
        let Some(child_facet_band) = facet_band_mut_from_coord(child.coord_measurement.as_mut())
        else {
            return false;
        };
        if child_facet_band.axis != parent_axis {
            return false;
        }
        child_facet_band.set_parent_bandwidth_value(cross_size);
        true
    }

    pub(crate) fn final_child_resize_policy(
        _axis: FacetAxis,
        _parent_cross_size_target: Option<f32>,
        _child: &ComponentsMeasurement,
    ) -> FinalChildResizePolicy {
        FinalChildResizePolicy {
            allow_width_resize: true,
            allow_height_resize: true,
            allow_x_range_retarget: true,
            allow_y_range_retarget: true,
        }
    }

    pub(crate) fn final_child_resize_policy_for_eval(
        _axis: FacetAxis,
        _parent_cross_size_target: Option<f32>,
        child: &ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
    ) -> FinalChildResizePolicy {
        let policy = eval_ctx.facet_runtime_sizing_mode().policy();
        let child_is_explicit_facet = facet_band_ref_from_coord(child.coord_measurement.as_ref())
            .is_some_and(|facet_band| facet_band.uses_explicit_placement());
        let allow_leaf_subtree_resize =
            policy.is_fully_leaf_plot_area_sized() && child_is_explicit_facet;
        FinalChildResizePolicy {
            allow_width_resize: policy.width.is_canvas_constrained() || allow_leaf_subtree_resize,
            allow_height_resize: policy.height.is_canvas_constrained() || allow_leaf_subtree_resize,
            allow_x_range_retarget: policy.width.is_canvas_constrained(),
            allow_y_range_retarget: policy.height.is_canvas_constrained(),
        }
    }

    pub(crate) fn apply_final_propagation_child_update(
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
                "policy final propagation requires a facet cell plan".to_string(),
            )
        })?;

        retarget_measurement_plot_area_policy_no_remeasure(
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
