use std::{collections::HashMap, future::Future, pin::Pin};

use tracing::trace;

use crate::{
    coords::FacetAxis,
    error::AvengerChartError,
    facet::{
        coord::{
            FacetBandCoordMeasurement, FacetCellRuntime,
            facet_band_canvas_mut as facet_band_canvas_mut_from_coord,
            facet_band_canvas_ref as facet_band_canvas_ref_from_coord,
            retarget_measurement_plot_area_no_remeasure,
        },
        coordination_attributes::{
            CollectionRoundA, CoordNodeKey, InheritedApplyIntent, InheritedApplyNodeIntent,
            InheritedApplyNodeOutcome, InheritedApplyTrace, InheritedPropagationChildIntent,
            InheritedPropagationIntent, InheritedPropagationNodeIntent,
            InheritedPropagationNodeOutcome, InheritedPropagationTrace, RecollectionRound,
        },
    },
    plot::compiled::ComponentsMeasurement,
    render::EvaluationContext,
};

fn facet_band_ref(measurement: &ComponentsMeasurement) -> Option<&FacetBandCoordMeasurement> {
    facet_band_canvas_ref_from_coord(measurement.coord_measurement.as_ref())
}

fn facet_band_mut(
    measurement: &mut ComponentsMeasurement,
) -> Option<&mut FacetBandCoordMeasurement> {
    facet_band_canvas_mut_from_coord(measurement.coord_measurement.as_mut())
}

pub(crate) fn visit_facet_bands_with_node_id<F>(
    measurement: &ComponentsMeasurement,
    depth: usize,
    node_path: &mut Vec<usize>,
    visit: &mut F,
) where
    F: FnMut(&CoordNodeKey, usize, &FacetBandCoordMeasurement),
{
    if let Some(facet_band) = facet_band_ref(measurement) {
        let node_id = CoordNodeKey::new(node_path.clone());
        visit(&node_id, depth, facet_band);
        for (idx, child) in facet_band.child_measurements_iter().enumerate() {
            node_path.push(idx);
            visit_facet_bands_with_node_id(child, depth + 1, node_path, visit);
            node_path.pop();
        }
    }
}

pub(crate) fn visit_facet_bands_with_node_id_mut<F>(
    measurement: &mut ComponentsMeasurement,
    depth: usize,
    node_path: &mut Vec<usize>,
    visit: &mut F,
) where
    F: FnMut(&CoordNodeKey, usize, &mut FacetBandCoordMeasurement),
{
    if let Some(facet_band) = facet_band_mut(measurement) {
        let node_id = CoordNodeKey::new(node_path.clone());
        visit(&node_id, depth, facet_band);
        for (idx, child) in facet_band.child_measurements_iter_mut().enumerate() {
            node_path.push(idx);
            visit_facet_bands_with_node_id_mut(child, depth + 1, node_path, visit);
            node_path.pop();
        }
    }
}

pub(crate) fn apply_collection_round_a(
    measurement: &mut ComponentsMeasurement,
    collection_round_a: &CollectionRoundA,
) {
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id_mut(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, _depth, facet_band| {
            if let Some(overflow) = collection_round_a
                .distribution
                .overflow_patches_by_node
                .get(node_id)
                .cloned()
            {
                facet_band.set_coordinated_overflow_value(overflow);
            }
            if let Some(layout) = collection_round_a
                .distribution
                .layout_patches_by_node
                .get(node_id)
                .cloned()
            {
                facet_band.set_coordinated_layout_value(layout);
            }
            if collection_round_a
                .distribution
                .domain_target_nodes
                .contains(node_id)
                && !collection_round_a
                    .distribution
                    .unified_domain_extents
                    .is_empty()
            {
                facet_band.distribute_coordinated_domain_extents(
                    &collection_round_a.distribution.unified_domain_extents,
                );
            }
        },
    );
}

pub(crate) fn apply_recollection_round(
    measurement: &mut ComponentsMeasurement,
    recollection_round: &RecollectionRound,
) {
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id_mut(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, _depth, facet_band| {
            if let Some(overflow) = recollection_round
                .distribution
                .overflow_patches_by_node
                .get(node_id)
                .cloned()
            {
                facet_band.set_coordinated_overflow_value(overflow);
            }
            if let Some(layout) = recollection_round
                .distribution
                .layout_patches_by_node
                .get(node_id)
                .cloned()
            {
                facet_band.set_coordinated_layout_value(layout);
            }
        },
    );
}

pub(crate) fn derive_inherited_apply_intent(
    measurement: &ComponentsMeasurement,
) -> InheritedApplyIntent {
    let mut node_derivations = Vec::new();
    let mut node_path = Vec::new();
    derive_inherited_apply_intent_recursive(measurement, &mut node_path, &mut node_derivations);
    InheritedApplyIntent { node_derivations }
}

fn derive_inherited_apply_intent_recursive(
    measurement: &ComponentsMeasurement,
    node_path: &mut Vec<usize>,
    node_derivations: &mut Vec<InheritedApplyNodeIntent>,
) {
    if let Some(facet_band) = facet_band_ref(measurement) {
        for (idx, child) in facet_band.child_measurements_iter().enumerate() {
            node_path.push(idx);
            derive_inherited_apply_intent_recursive(child, node_path, node_derivations);
            node_path.pop();
        }

        let node_id = CoordNodeKey::new(node_path.clone());
        let apply_plan = facet_band.derive_coordinated_apply_plan();
        let child_count = facet_band.child_measurements_iter().count();
        node_derivations.push(InheritedApplyNodeIntent {
            node_id,
            axis: apply_plan.axis,
            has_legend_overflow: apply_plan.has_legend_overflow,
            has_coordinated_extents: apply_plan.has_coordinated_extents,
            remeasure_triggered: false,
            apply_plan,
            child_count,
        });
    }
}

pub(crate) fn run_inherited_apply_with_trace<'a>(
    measurement: &'a mut ComponentsMeasurement,
    eval_ctx: &'a EvaluationContext,
    derivation: &'a InheritedApplyIntent,
) -> Pin<Box<dyn Future<Output = Result<InheritedApplyTrace, AvengerChartError>> + Send + 'a>> {
    Box::pin(async move {
        let derivation_by_node: HashMap<CoordNodeKey, InheritedApplyNodeIntent> = derivation
            .node_derivations
            .iter()
            .cloned()
            .map(|node| (node.node_id.clone(), node))
            .collect();

        let mut node_results = Vec::new();
        let mut node_path = Vec::new();
        run_inherited_apply_recursive(
            measurement,
            eval_ctx,
            &derivation_by_node,
            &mut node_path,
            &mut node_results,
        )
        .await?;
        Ok(InheritedApplyTrace { node_results })
    })
}

fn run_inherited_apply_recursive<'a>(
    measurement: &'a mut ComponentsMeasurement,
    eval_ctx: &'a EvaluationContext,
    derivation_by_node: &'a HashMap<CoordNodeKey, InheritedApplyNodeIntent>,
    node_path: &'a mut Vec<usize>,
    node_results: &'a mut Vec<InheritedApplyNodeOutcome>,
) -> Pin<Box<dyn Future<Output = Result<(), AvengerChartError>> + Send + 'a>> {
    Box::pin(async move {
        if let Some(facet_band) = facet_band_mut(measurement) {
            let node_id = CoordNodeKey::new(node_path.clone());
            let derived = derivation_by_node.get(&node_id).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing inherited-apply derivation for node path {:?}",
                    node_id.path
                ))
            })?;
            let outcome = facet_band
                .apply_coordinated_overflow_with_plan(eval_ctx, &derived.apply_plan)
                .await?;
            let parent_cross_size = facet_band.coordinated_subplot_cross_size();
            let parent_axis = facet_band.axis;
            let mut parent_cross_size_propagated = false;

            for (idx, child) in facet_band.child_measurements_iter_mut().enumerate() {
                if let Some(cross_size) = parent_cross_size
                    && let Some(child_facet_band) = child
                        .coord_measurement
                        .as_any_mut()
                        .downcast_mut::<FacetBandCoordMeasurement>()
                    && child_facet_band.axis == parent_axis
                {
                    child_facet_band.set_parent_bandwidth_value(cross_size);
                    parent_cross_size_propagated = true;
                }
                node_path.push(idx);
                run_inherited_apply_recursive(
                    child,
                    eval_ctx,
                    derivation_by_node,
                    node_path,
                    node_results,
                )
                .await?;
                node_path.pop();
            }

            node_results.push(InheritedApplyNodeOutcome {
                node_id,
                axis: derived.axis,
                derived_has_legend_overflow: derived.has_legend_overflow,
                derived_has_coordinated_extents: derived.has_coordinated_extents,
                derived_remeasure_required: derived.remeasure_triggered,
                derived_axis_owner_ignore_empty_cells: derived
                    .apply_plan
                    .axis_owner_ignore_empty_cells,
                derived_adjusted_main_size: derived.apply_plan.adjusted_main_size,
                derived_child_count: derived.child_count,
                parent_cross_size_propagated,
                subplot_cross_size_before: outcome.subplot_cross_size_before,
                subplot_cross_size_after: outcome.subplot_cross_size_after,
                remeasure_triggered: outcome.remeasure_triggered,
                remeasured_cell_count: outcome.remeasured_cell_count,
                remeasure_skipped_cell_count: outcome.remeasure_skipped_cell_count,
                remeasured_non_empty_cell_count: outcome.remeasured_non_empty_cell_count,
                remeasured_with_coordinated_extents_count: outcome
                    .remeasured_with_coordinated_extents_count,
            });
        }
        Ok(())
    })
}

pub(crate) fn derive_inherited_propagation_intent(
    measurement: &ComponentsMeasurement,
) -> InheritedPropagationIntent {
    let mut node_derivations = Vec::new();
    let mut node_path = Vec::new();
    derive_inherited_propagation_intent_recursive(
        measurement,
        &mut node_path,
        &mut node_derivations,
    );
    InheritedPropagationIntent { node_derivations }
}

fn derive_inherited_propagation_intent_recursive(
    measurement: &ComponentsMeasurement,
    node_path: &mut Vec<usize>,
    node_derivations: &mut Vec<InheritedPropagationNodeIntent>,
) {
    if let Some(facet_band) = facet_band_ref(measurement) {
        for (idx, child) in facet_band.child_measurements_iter().enumerate() {
            node_path.push(idx);
            derive_inherited_propagation_intent_recursive(child, node_path, node_derivations);
            node_path.pop();
        }

        let parent_cross_size_target = facet_band.coordinated_subplot_cross_size();
        let child_intents = build_inherited_propagation_child_intents(
            facet_band.axis,
            parent_cross_size_target,
            facet_band.child_measurements_iter(),
        );
        let expected_plot_area_adjustments_count = child_intents
            .iter()
            .filter(|intent| intent.adjust_plot_area)
            .count();

        node_derivations.push(InheritedPropagationNodeIntent {
            node_id: CoordNodeKey::new(node_path.clone()),
            axis: facet_band.axis,
            parent_cross_size_target,
            child_count: child_intents.len(),
            child_intents,
            expected_plot_area_adjustments_count,
        });
    }
}

fn build_inherited_propagation_child_intents<'a, I>(
    axis: FacetAxis,
    parent_cross_size_target: Option<f32>,
    child_measurements: I,
) -> Vec<InheritedPropagationChildIntent>
where
    I: Iterator<Item = &'a ComponentsMeasurement>,
{
    child_measurements
        .enumerate()
        .map(|(idx, child)| {
            let has_band_scale = child.scales.contains_key(axis.scale_name());
            build_inherited_propagation_child_intent(
                idx,
                axis,
                parent_cross_size_target,
                child.plot_area_width,
                child.plot_area_height,
                has_band_scale,
            )
        })
        .collect()
}

fn build_inherited_propagation_child_intent(
    child_index: usize,
    axis: FacetAxis,
    parent_cross_size_target: Option<f32>,
    old_plot_area_width: f32,
    old_plot_area_height: f32,
    has_band_scale: bool,
) -> InheritedPropagationChildIntent {
    let (target_plot_area_width, target_plot_area_height, adjust_plot_area) =
        match (axis, parent_cross_size_target) {
            (FacetAxis::Column, Some(target_width))
                if (old_plot_area_width - target_width).abs() > 0.01 =>
            {
                (Some(target_width), None, true)
            }
            (FacetAxis::Row, Some(target_height))
                if (old_plot_area_height - target_height).abs() > 0.01 =>
            {
                (None, Some(target_height), true)
            }
            _ => (None, None, false),
        };

    let target_band_range_end = if has_band_scale {
        parent_cross_size_target
    } else {
        None
    };
    let update_band_range = target_band_range_end.is_some();

    InheritedPropagationChildIntent {
        child_index,
        old_plot_area_width,
        old_plot_area_height,
        target_plot_area_width,
        target_plot_area_height,
        target_band_range_end,
        adjust_plot_area,
        update_band_range,
    }
}

fn apply_inherited_propagation_cell_update(
    axis: FacetAxis,
    cell: &mut FacetCellRuntime,
    compiled_subplot: &crate::plot::compiled::CompiledPlot,
    eval_ctx: &EvaluationContext,
    intent: &InheritedPropagationChildIntent,
) -> Result<(bool, usize), AvengerChartError> {
    let child = &mut cell.measurement;
    let target_plot_area_width = intent
        .target_plot_area_width
        .unwrap_or(child.plot_area_width)
        .max(1.0);
    let target_plot_area_height = intent
        .target_plot_area_height
        .unwrap_or(child.plot_area_height)
        .max(1.0);
    let plot_area_adjusted = (child.plot_area_width - target_plot_area_width).abs() > 0.01
        || (child.plot_area_height - target_plot_area_height).abs() > 0.01;

    if intent.update_band_range
        && let (Some(band_scale), Some(range_end)) = (
            child.scales.get_mut(axis.scale_name()),
            intent.target_band_range_end,
        )
    {
        let updated_config = band_scale
            .configured()
            .clone()
            .with_range_interval((0.0, range_end));
        band_scale.set_configured(updated_config);
    }

    if !plot_area_adjusted {
        return Ok((false, 0));
    }

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
        &cell.plan.full_path,
        target_plot_area_width,
        target_plot_area_height,
    )?;

    Ok((true, 1))
}

pub(crate) fn run_inherited_propagation_with_trace(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    derivation: &InheritedPropagationIntent,
) -> Result<InheritedPropagationTrace, AvengerChartError> {
    let derivation_by_node: HashMap<CoordNodeKey, InheritedPropagationNodeIntent> = derivation
        .node_derivations
        .iter()
        .cloned()
        .map(|node| (node.node_id.clone(), node))
        .collect();

    let mut node_results = Vec::new();
    let mut node_path = Vec::new();
    run_inherited_propagation_recursive(
        measurement,
        eval_ctx,
        &derivation_by_node,
        &mut node_path,
        &mut node_results,
    )?;
    Ok(InheritedPropagationTrace { node_results })
}

fn run_inherited_propagation_recursive(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    derivation_by_node: &HashMap<CoordNodeKey, InheritedPropagationNodeIntent>,
    node_path: &mut Vec<usize>,
    node_results: &mut Vec<InheritedPropagationNodeOutcome>,
) -> Result<(), AvengerChartError> {
    measurement
        .coord_measurement
        .apply_scale_adjustments(&mut measurement.scales);

    if let Some(facet_band) = facet_band_mut(measurement) {
        let node_id = CoordNodeKey::new(node_path.clone());
        let axis = facet_band.axis;
        let derived = derivation_by_node.get(&node_id);
        debug_assert!(
            derived.is_some(),
            "Missing inherited-propagation derivation for node path {:?}",
            node_id.path
        );
        let fallback_child_intents = build_inherited_propagation_child_intents(
            axis,
            facet_band.coordinated_subplot_cross_size(),
            facet_band.child_measurements_iter(),
        );
        let fallback_expected_plot_area_adjustments_count = fallback_child_intents
            .iter()
            .filter(|intent| intent.adjust_plot_area)
            .count();
        let (
            child_intents,
            derived_parent_cross_size_target,
            derived_child_count,
            derived_expected_plot_area_adjustments_count,
        ) = if let Some(derived) = derived {
            (
                derived.child_intents.clone(),
                derived.parent_cross_size_target,
                derived.child_count,
                derived.expected_plot_area_adjustments_count,
            )
        } else {
            (
                fallback_child_intents,
                facet_band.coordinated_subplot_cross_size(),
                facet_band.child_measurements_iter().count(),
                fallback_expected_plot_area_adjustments_count,
            )
        };
        let mut child_plot_area_adjustments_count = 0usize;
        let mut scale_range_retarget_count = 0usize;
        let compiled_subplot = facet_band.compiled_subplot.clone();

        for (idx, cell) in facet_band.cells.iter_mut().enumerate() {
            let child = &mut cell.measurement;
            let intent = child_intents.get(idx).cloned().unwrap_or_else(|| {
                let has_band_scale = child.scales.contains_key(axis.scale_name());
                build_inherited_propagation_child_intent(
                    idx,
                    axis,
                    derived_parent_cross_size_target,
                    child.plot_area_width,
                    child.plot_area_height,
                    has_band_scale,
                )
            });

            let (plot_area_adjusted, retarget_count) = apply_inherited_propagation_cell_update(
                axis,
                cell,
                compiled_subplot.as_ref(),
                eval_ctx,
                &intent,
            )?;
            if plot_area_adjusted {
                child_plot_area_adjustments_count += 1;
            }
            scale_range_retarget_count += retarget_count;
            node_path.push(idx);
            run_inherited_propagation_recursive(
                &mut cell.measurement,
                eval_ctx,
                derivation_by_node,
                node_path,
                node_results,
            )?;
            node_path.pop();
        }

        facet_band.apply_cross_axis_coordinated_side_slabs_to_cells();

        node_results.push(InheritedPropagationNodeOutcome {
            node_id,
            axis,
            derived_parent_cross_size_target,
            derived_child_count,
            derived_child_intent_count: child_intents.len(),
            derived_expected_plot_area_adjustments_count,
            child_plot_area_adjustments_count,
            scale_range_retarget_count,
        });
    }

    Ok(())
}
