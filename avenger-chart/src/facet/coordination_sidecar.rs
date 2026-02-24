use std::{collections::HashMap, future::Future, pin::Pin};

use tracing::trace;

use crate::{
    coords::FacetAxis,
    error::AvengerChartError,
    facet::{
        coord::FacetBandCoordMeasurement,
        coordination_ir::{
            CoordApplyIntent, CoordApplyNodeIntent, CoordApplyNodeOutcome, CoordApplyTrace,
            CoordGroupDistribution, CoordNodeKey, CoordReconcileDistribution,
            CoordRetargetChildIntent, CoordRetargetIntent, CoordRetargetNodeIntent,
            CoordRetargetNodeOutcome, CoordRetargetTrace,
        },
        coordination_remeasure::derive_facet_coord_remeasure_plan,
    },
    plot::compiled::ComponentsMeasurement,
    render::EvaluationContext,
    scales::ConfiguredScaleWithSpec,
};

fn facet_band_ref(measurement: &ComponentsMeasurement) -> Option<&FacetBandCoordMeasurement> {
    measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
}

fn facet_band_mut(
    measurement: &mut ComponentsMeasurement,
) -> Option<&mut FacetBandCoordMeasurement> {
    measurement
        .coord_measurement
        .as_any_mut()
        .downcast_mut::<FacetBandCoordMeasurement>()
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

pub(crate) fn apply_phase7_distribution(
    measurement: &mut ComponentsMeasurement,
    phase7: &CoordGroupDistribution,
) {
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id_mut(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, _depth, facet_band| {
            if let Some(overflow) = phase7
                .distribution
                .overflow_patches_by_node
                .get(node_id)
                .cloned()
            {
                facet_band.set_coordinated_overflow_value(overflow);
            }
            if let Some(layout) = phase7
                .distribution
                .layout_patches_by_node
                .get(node_id)
                .cloned()
            {
                facet_band.set_coordinated_layout_value(layout);
            }
            if phase7.distribution.domain_target_nodes.contains(node_id)
                && !phase7.distribution.unified_domain_extents.is_empty()
            {
                facet_band.distribute_coordinated_domain_extents(
                    &phase7.distribution.unified_domain_extents,
                );
            }
        },
    );
}

pub(crate) fn apply_phase9_distribution(
    measurement: &mut ComponentsMeasurement,
    phase9: &CoordReconcileDistribution,
) {
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id_mut(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, _depth, facet_band| {
            if let Some(overflow) = phase9
                .distribution
                .overflow_patches_by_node
                .get(node_id)
                .cloned()
            {
                facet_band.set_coordinated_overflow_value(overflow);
            }
            if let Some(layout) = phase9
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

pub(crate) fn derive_phase8(measurement: &ComponentsMeasurement) -> CoordApplyIntent {
    let mut node_derivations = Vec::new();
    let mut node_path = Vec::new();
    derive_phase8_recursive(measurement, &mut node_path, &mut node_derivations);
    CoordApplyIntent { node_derivations }
}

fn derive_phase8_recursive(
    measurement: &ComponentsMeasurement,
    node_path: &mut Vec<usize>,
    node_derivations: &mut Vec<CoordApplyNodeIntent>,
) {
    if let Some(facet_band) = facet_band_ref(measurement) {
        for (idx, child) in facet_band.child_measurements_iter().enumerate() {
            node_path.push(idx);
            derive_phase8_recursive(child, node_path, node_derivations);
            node_path.pop();
        }

        let node_id = CoordNodeKey::new(node_path.clone());
        let apply_plan = facet_band.derive_coordinated_apply_plan();
        let child_count = facet_band.child_measurements_iter().count();
        let remeasure_plan = apply_plan.remeasure_required.then(|| {
            derive_facet_coord_remeasure_plan(
                &facet_band.cells,
                &apply_plan,
                facet_band.subplot_cross_size,
            )
        });

        node_derivations.push(CoordApplyNodeIntent {
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

pub(crate) fn run_phase8_apply_coordinated_overflow_and_remeasure_with_trace<'a>(
    measurement: &'a mut ComponentsMeasurement,
    eval_ctx: &'a EvaluationContext,
    derivation: &'a CoordApplyIntent,
) -> Pin<Box<dyn Future<Output = Result<CoordApplyTrace, AvengerChartError>> + Send + 'a>> {
    Box::pin(async move {
        let derivation_by_node: HashMap<CoordNodeKey, CoordApplyNodeIntent> = derivation
            .node_derivations
            .iter()
            .cloned()
            .map(|node| (node.node_id.clone(), node))
            .collect();

        let mut node_results = Vec::new();
        let mut node_path = Vec::new();
        run_phase8_apply_recursive(
            measurement,
            eval_ctx,
            &derivation_by_node,
            &mut node_path,
            &mut node_results,
        )
        .await?;
        Ok(CoordApplyTrace { node_results })
    })
}

fn run_phase8_apply_recursive<'a>(
    measurement: &'a mut ComponentsMeasurement,
    eval_ctx: &'a EvaluationContext,
    derivation_by_node: &'a HashMap<CoordNodeKey, CoordApplyNodeIntent>,
    node_path: &'a mut Vec<usize>,
    node_results: &'a mut Vec<CoordApplyNodeOutcome>,
) -> Pin<Box<dyn Future<Output = Result<(), AvengerChartError>> + Send + 'a>> {
    Box::pin(async move {
        if let Some(facet_band) = facet_band_mut(measurement) {
            let node_id = CoordNodeKey::new(node_path.clone());
            let derived = derivation_by_node.get(&node_id).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing phase-8 derivation for node path {:?}",
                    node_id.path
                ))
            })?;
            let outcome = facet_band
                .apply_coordinated_overflow_with_plan_and_remeasure_plan(
                    eval_ctx,
                    &derived.apply_plan,
                    derived.remeasure_plan.as_ref(),
                )
                .await?;
            let parent_cross_size = facet_band.coordinated_subplot_cross_size();
            let parent_axis = facet_band.axis;
            let mut parent_cross_size_propagated = false;

            for (idx, child) in facet_band.child_measurements_iter_mut().enumerate() {
                if let Some(cross_size) = parent_cross_size {
                    if let Some(child_facet_band) = child
                        .coord_measurement
                        .as_any_mut()
                        .downcast_mut::<FacetBandCoordMeasurement>()
                    {
                        if child_facet_band.axis == parent_axis {
                            child_facet_band.set_parent_bandwidth_value(cross_size);
                            parent_cross_size_propagated = true;
                        }
                    }
                }
                node_path.push(idx);
                run_phase8_apply_recursive(
                    child,
                    eval_ctx,
                    derivation_by_node,
                    node_path,
                    node_results,
                )
                .await?;
                node_path.pop();
            }

            node_results.push(CoordApplyNodeOutcome {
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
                remeasured_non_empty_cell_count: outcome.remeasured_non_empty_cell_count,
                remeasured_with_coordinated_extents_count: outcome
                    .remeasured_with_coordinated_extents_count,
            });
        }
        Ok(())
    })
}

pub(crate) fn derive_phase10(measurement: &ComponentsMeasurement) -> CoordRetargetIntent {
    let mut node_derivations = Vec::new();
    let mut node_path = Vec::new();
    derive_phase10_recursive(measurement, &mut node_path, &mut node_derivations);
    CoordRetargetIntent { node_derivations }
}

fn derive_phase10_recursive(
    measurement: &ComponentsMeasurement,
    node_path: &mut Vec<usize>,
    node_derivations: &mut Vec<CoordRetargetNodeIntent>,
) {
    if let Some(facet_band) = facet_band_ref(measurement) {
        for (idx, child) in facet_band.child_measurements_iter().enumerate() {
            node_path.push(idx);
            derive_phase10_recursive(child, node_path, node_derivations);
            node_path.pop();
        }

        let parent_cross_size_target = facet_band.coordinated_subplot_cross_size();
        let child_intents = build_phase10_child_intents(
            facet_band.axis,
            parent_cross_size_target,
            facet_band.child_measurements_iter(),
        );
        let expected_plot_area_adjustments_count = child_intents
            .iter()
            .filter(|intent| intent.adjust_plot_area)
            .count();

        node_derivations.push(CoordRetargetNodeIntent {
            node_id: CoordNodeKey::new(node_path.clone()),
            axis: facet_band.axis,
            parent_cross_size_target,
            child_count: child_intents.len(),
            child_intents,
            expected_plot_area_adjustments_count,
        });
    }
}

fn build_phase10_child_intents<'a, I>(
    axis: FacetAxis,
    parent_cross_size_target: Option<f32>,
    child_measurements: I,
) -> Vec<CoordRetargetChildIntent>
where
    I: Iterator<Item = &'a ComponentsMeasurement>,
{
    child_measurements
        .enumerate()
        .map(|(idx, child)| {
            let has_band_scale = child.scales.contains_key(axis.scale_name());
            build_phase10_child_intent(
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

fn build_phase10_child_intent(
    child_index: usize,
    axis: FacetAxis,
    parent_cross_size_target: Option<f32>,
    old_plot_area_width: f32,
    old_plot_area_height: f32,
    has_band_scale: bool,
) -> CoordRetargetChildIntent {
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

    CoordRetargetChildIntent {
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

fn apply_phase10_child_propagation_from_intent(
    axis: FacetAxis,
    child: &mut ComponentsMeasurement,
    intent: &CoordRetargetChildIntent,
) -> bool {
    let mut plot_area_adjusted = false;

    if intent.adjust_plot_area {
        if let Some(new_width) = intent.target_plot_area_width {
            if (child.plot_area_width - new_width).abs() > 0.01 {
                trace!(
                    old_width = child.plot_area_width,
                    new_width,
                    axis = "column",
                    "coordinate_facet_measurement_tree updating child plot area cross-size"
                );
                child.plot_area_width = new_width;
                plot_area_adjusted = true;
            }
        }
        if let Some(new_height) = intent.target_plot_area_height {
            if (child.plot_area_height - new_height).abs() > 0.01 {
                trace!(
                    old_height = child.plot_area_height,
                    new_height,
                    axis = "row",
                    "coordinate_facet_measurement_tree updating child plot area cross-size"
                );
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

fn apply_phase10_child_retarget_from_intent(
    child: &mut ComponentsMeasurement,
    intent: &CoordRetargetChildIntent,
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

pub(crate) fn run_phase10_scale_retarget_and_adjustments_with_trace(
    measurement: &mut ComponentsMeasurement,
    derivation: &CoordRetargetIntent,
) -> CoordRetargetTrace {
    let derivation_by_node: HashMap<CoordNodeKey, CoordRetargetNodeIntent> = derivation
        .node_derivations
        .iter()
        .cloned()
        .map(|node| (node.node_id.clone(), node))
        .collect();

    let mut node_results = Vec::new();
    let mut node_path = Vec::new();
    run_phase10_scale_retarget_recursive(
        measurement,
        &derivation_by_node,
        &mut node_path,
        &mut node_results,
    );
    CoordRetargetTrace { node_results }
}

fn run_phase10_scale_retarget_recursive(
    measurement: &mut ComponentsMeasurement,
    derivation_by_node: &HashMap<CoordNodeKey, CoordRetargetNodeIntent>,
    node_path: &mut Vec<usize>,
    node_results: &mut Vec<CoordRetargetNodeOutcome>,
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
            "Missing phase-10 derivation for node path {:?}",
            node_id.path
        );
        let fallback_child_intents = build_phase10_child_intents(
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
                build_phase10_child_intent(
                    idx,
                    axis,
                    derived_parent_cross_size_target,
                    child.plot_area_width,
                    child.plot_area_height,
                    has_band_scale,
                )
            });

            let plot_area_adjusted =
                apply_phase10_child_propagation_from_intent(axis, child, &intent);
            if plot_area_adjusted {
                child_plot_area_adjustments_count += 1;
            }
            scale_range_retarget_count += apply_phase10_child_retarget_from_intent(child, &intent);
            node_path.push(idx);
            run_phase10_scale_retarget_recursive(
                child,
                derivation_by_node,
                node_path,
                node_results,
            );
            node_path.pop();
        }

        node_results.push(CoordRetargetNodeOutcome {
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
