use std::{collections::HashMap, future::Future, pin::Pin};

use tracing::trace;

use crate::{
    coords::FacetAxis,
    error::AvengerChartError,
    facet::{
        coord::{
            FacetBandCoordMeasurementFixed, compute_fixed_main_axis_positions,
            facet_band_fixed_mut as facet_band_fixed_mut_from_coord,
            facet_band_fixed_ref as facet_band_fixed_ref_from_coord,
        },
        coordination_attributes::{
            CollectionRoundA, CoordNodeKey, InheritedApplyIntent, InheritedApplyNodeIntent,
            InheritedApplyNodeOutcome, InheritedApplyTrace, InheritedPropagationChildIntent,
            InheritedPropagationIntent, InheritedPropagationNodeIntent,
            InheritedPropagationNodeOutcome, InheritedPropagationTrace, RecollectionRound,
        },
        coordination_remeasure::derive_facet_coord_remeasure_plan,
        layout_slabs::LayoutSlabs,
    },
    plot::compiled::ComponentsMeasurement,
    render::EvaluationContext,
    scales::ConfiguredScaleWithSpec,
};

fn facet_band_ref(measurement: &ComponentsMeasurement) -> Option<&FacetBandCoordMeasurementFixed> {
    facet_band_fixed_ref_from_coord(measurement.coord_measurement.as_ref())
}

fn facet_band_mut(
    measurement: &mut ComponentsMeasurement,
) -> Option<&mut FacetBandCoordMeasurementFixed> {
    facet_band_fixed_mut_from_coord(measurement.coord_measurement.as_mut())
}

pub(crate) fn visit_fixed_facet_bands_with_node_id<F>(
    measurement: &ComponentsMeasurement,
    depth: usize,
    node_path: &mut Vec<usize>,
    visit: &mut F,
) where
    F: FnMut(&CoordNodeKey, usize, &FacetBandCoordMeasurementFixed),
{
    if let Some(facet_band) = facet_band_ref(measurement) {
        let node_id = CoordNodeKey::new(node_path.clone());
        visit(&node_id, depth, facet_band);
        for (idx, child) in facet_band.child_measurements_iter().enumerate() {
            node_path.push(idx);
            visit_fixed_facet_bands_with_node_id(child, depth + 1, node_path, visit);
            node_path.pop();
        }
    }
}

pub(crate) fn visit_fixed_facet_bands_with_node_id_mut<F>(
    measurement: &mut ComponentsMeasurement,
    depth: usize,
    node_path: &mut Vec<usize>,
    visit: &mut F,
) where
    F: FnMut(&CoordNodeKey, usize, &mut FacetBandCoordMeasurementFixed),
{
    if let Some(facet_band) = facet_band_mut(measurement) {
        let node_id = CoordNodeKey::new(node_path.clone());
        visit(&node_id, depth, facet_band);
        for (idx, child) in facet_band.child_measurements_iter_mut().enumerate() {
            node_path.push(idx);
            visit_fixed_facet_bands_with_node_id_mut(child, depth + 1, node_path, visit);
            node_path.pop();
        }
    }
}

pub(crate) fn apply_collection_round_a_fixed(
    measurement: &mut ComponentsMeasurement,
    collection_round_a: &CollectionRoundA,
) {
    let mut node_path = Vec::new();
    visit_fixed_facet_bands_with_node_id_mut(
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
                let active_layout = facet_band
                    .coordinated_layout
                    .as_ref()
                    .unwrap_or(&facet_band.local_layout);
                facet_band.fixed_main_axis_positions = compute_fixed_main_axis_positions(
                    facet_band.axis,
                    &facet_band.cells,
                    active_layout,
                );
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

pub(crate) fn apply_recollection_round_fixed(
    measurement: &mut ComponentsMeasurement,
    recollection_round: &RecollectionRound,
) {
    let mut node_path = Vec::new();
    visit_fixed_facet_bands_with_node_id_mut(
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
                let active_layout = facet_band
                    .coordinated_layout
                    .as_ref()
                    .unwrap_or(&facet_band.local_layout);
                facet_band.fixed_main_axis_positions = compute_fixed_main_axis_positions(
                    facet_band.axis,
                    &facet_band.cells,
                    active_layout,
                );
            }
        },
    );
}

pub(crate) fn derive_inherited_apply_intent_fixed(
    measurement: &ComponentsMeasurement,
) -> InheritedApplyIntent {
    let mut node_derivations = Vec::new();
    let mut node_path = Vec::new();
    derive_inherited_apply_intent_fixed_recursive(
        measurement,
        &mut node_path,
        &mut node_derivations,
    );
    InheritedApplyIntent { node_derivations }
}

fn derive_inherited_apply_intent_fixed_recursive(
    measurement: &ComponentsMeasurement,
    node_path: &mut Vec<usize>,
    node_derivations: &mut Vec<InheritedApplyNodeIntent>,
) {
    if let Some(facet_band) = facet_band_ref(measurement) {
        for (idx, child) in facet_band.child_measurements_iter().enumerate() {
            node_path.push(idx);
            derive_inherited_apply_intent_fixed_recursive(child, node_path, node_derivations);
            node_path.pop();
        }

        let node_id = CoordNodeKey::new(node_path.clone());
        let mut apply_plan = facet_band.derive_coordinated_apply_plan();
        let slabs = LayoutSlabs::from_coordinated(&facet_band.coordinated_overflow);
        let (legend_main_start, legend_main_end) = match facet_band.axis {
            FacetAxis::Column => slabs.legend_vertical(),
            FacetAxis::Row => slabs.legend_horizontal(),
        };
        let has_main_axis_legend_slab = legend_main_start > 0.0 || legend_main_end > 0.0;
        debug_assert_eq!(
            apply_plan.has_legend_overflow, has_main_axis_legend_slab,
            "fixed inherited-apply invariant: apply-plan legend-overflow flag must match coordinated main-axis legend slabs"
        );
        // Fixed-subplot mode keeps per-cell plot area dimensions locked.
        // Keep coordinated layout/domain/remeasure semantics intact, but disable
        // legend-driven main-size shrink for fixed leaf plot areas.
        apply_plan.adjusted_main_size = apply_plan.original_main_size;
        apply_plan.legend_main_axis_shrink = 0.0;
        let child_count = facet_band.child_measurements_iter().count();
        let remeasure_plan = apply_plan.remeasure_required.then(|| {
            derive_facet_coord_remeasure_plan(
                &facet_band.cells,
                &apply_plan,
                facet_band.subplot_cross_size,
            )
        });

        node_derivations.push(InheritedApplyNodeIntent {
            node_id,
            axis: apply_plan.axis,
            remeasure_plan,
            has_legend_overflow: apply_plan.has_legend_overflow,
            has_coordinated_extents: apply_plan.has_coordinated_extents,
            remeasure_triggered: apply_plan.remeasure_required,
            apply_plan,
            child_count,
        });
    }
}

pub(crate) fn run_inherited_apply_with_trace_fixed<'a>(
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
            // Fixed-subplot mode keeps leaf subplot sizes locked. Preserve derived
            // coordination semantics for diagnostics/tracing, but skip coordinated
            // cross-size rewrites during execution.
            let mut execution_plan = derived.apply_plan.clone();
            execution_plan.has_coordinated_layout = false;
            let outcome = facet_band
                .apply_coordinated_overflow_with_plan_and_remeasure_plan(
                    eval_ctx,
                    &execution_plan,
                    derived.remeasure_plan.as_ref(),
                )
                .await?;
            let active_layout = facet_band
                .coordinated_layout
                .as_ref()
                .unwrap_or(&facet_band.local_layout);
            facet_band.fixed_main_axis_positions = compute_fixed_main_axis_positions(
                facet_band.axis,
                &facet_band.cells,
                active_layout,
            );
            let parent_cross_size = facet_band.coordinated_subplot_cross_size();
            let parent_axis = facet_band.axis;
            let mut parent_cross_size_propagated = false;

            for (idx, child) in facet_band.child_measurements_iter_mut().enumerate() {
                if let Some(cross_size) = parent_cross_size {
                    if let Some(child_facet_band) = child
                        .coord_measurement
                        .as_any_mut()
                        .downcast_mut::<FacetBandCoordMeasurementFixed>(
                    ) {
                        if child_facet_band.axis == parent_axis {
                            child_facet_band.set_parent_bandwidth_value(cross_size);
                            parent_cross_size_propagated = true;
                        }
                    }
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

pub(crate) fn derive_inherited_propagation_intent_fixed(
    measurement: &ComponentsMeasurement,
) -> InheritedPropagationIntent {
    let mut node_derivations = Vec::new();
    let mut node_path = Vec::new();
    derive_inherited_propagation_intent_fixed_recursive(
        measurement,
        &mut node_path,
        &mut node_derivations,
    );
    InheritedPropagationIntent { node_derivations }
}

fn derive_inherited_propagation_intent_fixed_recursive(
    measurement: &ComponentsMeasurement,
    node_path: &mut Vec<usize>,
    node_derivations: &mut Vec<InheritedPropagationNodeIntent>,
) {
    if let Some(facet_band) = facet_band_ref(measurement) {
        for (idx, child) in facet_band.child_measurements_iter().enumerate() {
            node_path.push(idx);
            derive_inherited_propagation_intent_fixed_recursive(child, node_path, node_derivations);
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
            let child_is_facet_band = child
                .coord_measurement
                .as_any()
                .downcast_ref::<FacetBandCoordMeasurementFixed>()
                .is_some();
            build_inherited_propagation_child_intent(
                idx,
                axis,
                parent_cross_size_target,
                child.plot_area_width,
                child.plot_area_height,
                has_band_scale,
                child_is_facet_band,
            )
        })
        .collect()
}

fn build_inherited_propagation_child_intent(
    child_index: usize,
    _axis: FacetAxis,
    parent_cross_size_target: Option<f32>,
    old_plot_area_width: f32,
    old_plot_area_height: f32,
    has_band_scale: bool,
    child_is_facet_band: bool,
) -> InheritedPropagationChildIntent {
    // Fixed mode keeps leaf subplot dimensions locked, but facet child containers may still
    // need cross-size propagation for coordinated parent sizing.
    let (target_plot_area_width, target_plot_area_height, adjust_plot_area) = if child_is_facet_band
    {
        match (_axis, parent_cross_size_target) {
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
        }
    } else {
        (None, None, false)
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

fn apply_inherited_propagation_child_resize(
    axis: FacetAxis,
    child: &mut ComponentsMeasurement,
    intent: &InheritedPropagationChildIntent,
) -> bool {
    let mut plot_area_adjusted = false;
    if intent.adjust_plot_area {
        if let Some(new_width) = intent.target_plot_area_width {
            if (child.plot_area_width - new_width).abs() > 0.01 {
                child.plot_area_width = new_width;
                plot_area_adjusted = true;
            }
        }
        if let Some(new_height) = intent.target_plot_area_height {
            if (child.plot_area_height - new_height).abs() > 0.01 {
                child.plot_area_height = new_height;
                plot_area_adjusted = true;
            }
        }
    }

    if intent.update_band_range {
        if let (Some(band_scale), Some(range_end)) = (
            child.scales.get_mut(axis.scale_name()),
            intent.target_band_range_end,
        ) {
            let updated_config = band_scale
                .configured()
                .clone()
                .with_range_interval((0.0, range_end));
            *band_scale = ConfiguredScaleWithSpec::new(band_scale.spec().clone(), updated_config);
        }
    }

    plot_area_adjusted
}

fn apply_inherited_propagation_child_retarget(
    child: &mut ComponentsMeasurement,
    intent: &InheritedPropagationChildIntent,
) -> usize {
    if (child.plot_area_width - intent.old_plot_area_width).abs() <= 0.01
        && (child.plot_area_height - intent.old_plot_area_height).abs() <= 0.01
    {
        return 0;
    }

    retarget_child_scales_for_resized_plot_area(
        child,
        intent.old_plot_area_width,
        intent.old_plot_area_height,
    )
}

pub(crate) fn run_inherited_propagation_with_trace_fixed(
    measurement: &mut ComponentsMeasurement,
    derivation: &InheritedPropagationIntent,
) -> InheritedPropagationTrace {
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
        &derivation_by_node,
        &mut node_path,
        &mut node_results,
    );
    InheritedPropagationTrace { node_results }
}

fn run_inherited_propagation_recursive(
    measurement: &mut ComponentsMeasurement,
    derivation_by_node: &HashMap<CoordNodeKey, InheritedPropagationNodeIntent>,
    node_path: &mut Vec<usize>,
    node_results: &mut Vec<InheritedPropagationNodeOutcome>,
) {
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

        for (idx, child) in facet_band.child_measurements_iter_mut().enumerate() {
            let intent = child_intents.get(idx).cloned().unwrap_or_else(|| {
                let has_band_scale = child.scales.contains_key(axis.scale_name());
                build_inherited_propagation_child_intent(
                    idx,
                    axis,
                    derived_parent_cross_size_target,
                    child.plot_area_width,
                    child.plot_area_height,
                    has_band_scale,
                    child
                        .coord_measurement
                        .as_any()
                        .downcast_ref::<FacetBandCoordMeasurementFixed>()
                        .is_some(),
                )
            });

            let plot_area_adjusted = apply_inherited_propagation_child_resize(axis, child, &intent);
            if plot_area_adjusted {
                child_plot_area_adjustments_count += 1;
            }
            scale_range_retarget_count +=
                apply_inherited_propagation_child_retarget(child, &intent);
            node_path.push(idx);
            run_inherited_propagation_recursive(child, derivation_by_node, node_path, node_results);
            node_path.pop();
        }

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
}

fn retarget_child_scales_for_resized_plot_area(
    child: &mut ComponentsMeasurement,
    old_plot_area_width: f32,
    old_plot_area_height: f32,
) -> usize {
    let new_plot_area_width = child.plot_area_width;
    let new_plot_area_height = child.plot_area_height;
    let mut retarget_count = 0usize;

    for (scale_name, scale_with_spec) in child.scales.iter_mut() {
        let Ok((range_start, range_end)) = scale_with_spec.configured().numeric_interval_range()
        else {
            continue;
        };

        let span = (range_end - range_start).abs();
        if span <= f32::EPSILON {
            continue;
        }

        let width_match = approx_span(span, old_plot_area_width);
        let height_match = approx_span(span, old_plot_area_height);

        let target_span = match (width_match, height_match) {
            (true, false) => new_plot_area_width,
            (false, true) => new_plot_area_height,
            (true, true) => {
                let width_changed = (new_plot_area_width - old_plot_area_width).abs() > 0.01;
                let height_changed = (new_plot_area_height - old_plot_area_height).abs() > 0.01;
                match (width_changed, height_changed) {
                    (true, false) => new_plot_area_width,
                    (false, true) => new_plot_area_height,
                    _ => continue,
                }
            }
            (false, false) => continue,
        };

        let Some((new_range_start, new_range_end)) =
            retarget_interval_preserving_anchor((range_start, range_end), target_span)
        else {
            continue;
        };

        let updated_config = scale_with_spec
            .configured()
            .clone()
            .with_range_interval((new_range_start, new_range_end));
        *scale_with_spec =
            ConfiguredScaleWithSpec::new(scale_with_spec.spec().clone(), updated_config);
        retarget_count += 1;

        trace!(
            scale = %scale_name,
            old_range_start = range_start,
            old_range_end = range_end,
            new_range_start,
            new_range_end,
            old_plot_area_width,
            old_plot_area_height,
            new_plot_area_width,
            new_plot_area_height,
            "coordinate_facet_measurement_tree retargeted child scale range after plot resize"
        );
    }
    retarget_count
}

fn approx_span(actual: f32, expected: f32) -> bool {
    if expected <= 0.0 {
        return false;
    }
    let tolerance = (expected.abs() * 0.02).max(1.0);
    (actual - expected).abs() <= tolerance
}

fn retarget_interval_preserving_anchor(range: (f32, f32), target_span: f32) -> Option<(f32, f32)> {
    if target_span <= 0.0 {
        return None;
    }

    let (start, end) = range;
    let eps = 0.01;

    if start.abs() <= eps {
        let sign = if end >= start { 1.0 } else { -1.0 };
        return Some((0.0, sign * target_span));
    }

    if end.abs() <= eps {
        let sign = if end >= start { 1.0 } else { -1.0 };
        return Some((-sign * target_span, 0.0));
    }

    None
}
