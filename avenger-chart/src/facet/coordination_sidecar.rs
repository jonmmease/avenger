use std::{collections::HashMap, future::Future, pin::Pin};

use tracing::trace;

use crate::{
    coords::FacetAxis,
    error::AvengerChartError,
    facet::{
        coord::FacetBandCoordMeasurement,
        coordination_ir::{
            CoordNodeId, CoordPhase7Ir, CoordPhase8Derivation, CoordPhase8Ir,
            CoordPhase8NodeDerivation, CoordPhase8NodeResult, CoordPhase9Ir,
            CoordPhase10Derivation, CoordPhase10Ir, CoordPhase10NodeDerivation,
            CoordPhase10NodeResult,
        },
        layout_slabs::LayoutSlabs,
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
    F: FnMut(&CoordNodeId, usize, &FacetBandCoordMeasurement),
{
    if let Some(facet_band) = facet_band_ref(measurement) {
        let node_id = CoordNodeId::new(node_path.clone());
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
    F: FnMut(&CoordNodeId, usize, &mut FacetBandCoordMeasurement),
{
    if let Some(facet_band) = facet_band_mut(measurement) {
        let node_id = CoordNodeId::new(node_path.clone());
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
    phase7: &CoordPhase7Ir,
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
    phase9: &CoordPhase9Ir,
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

pub(crate) fn derive_phase8(measurement: &ComponentsMeasurement) -> CoordPhase8Derivation {
    let mut node_derivations = Vec::new();
    let mut node_path = Vec::new();
    derive_phase8_recursive(measurement, &mut node_path, &mut node_derivations);
    CoordPhase8Derivation { node_derivations }
}

fn derive_phase8_recursive(
    measurement: &ComponentsMeasurement,
    node_path: &mut Vec<usize>,
    node_derivations: &mut Vec<CoordPhase8NodeDerivation>,
) {
    if let Some(facet_band) = facet_band_ref(measurement) {
        for (idx, child) in facet_band.child_measurements_iter().enumerate() {
            node_path.push(idx);
            derive_phase8_recursive(child, node_path, node_derivations);
            node_path.pop();
        }

        let node_id = CoordNodeId::new(node_path.clone());
        let axis = facet_band.axis;
        let legend_slabs = LayoutSlabs::from_coordinated(&facet_band.coordinated_overflow);
        let (legend_start, legend_end) = match axis {
            FacetAxis::Column => legend_slabs.legend_vertical(),
            FacetAxis::Row => legend_slabs.legend_horizontal(),
        };
        let has_legend_overflow = legend_start > 0.0 || legend_end > 0.0;
        let has_coordinated_extents = facet_band
            .cells
            .iter()
            .any(|cell| !cell.coordinated_domain_extents.is_empty());
        let remeasure_triggered = has_legend_overflow || has_coordinated_extents;
        let child_count = facet_band.child_measurements_iter().count();

        node_derivations.push(CoordPhase8NodeDerivation {
            node_id,
            axis,
            has_legend_overflow,
            has_coordinated_extents,
            remeasure_triggered,
            child_count,
        });
    }
}

pub(crate) fn run_phase8_apply_coordinated_overflow_and_remeasure_with_trace<'a>(
    measurement: &'a mut ComponentsMeasurement,
    eval_ctx: &'a EvaluationContext,
    derivation: &'a CoordPhase8Derivation,
) -> Pin<Box<dyn Future<Output = Result<CoordPhase8Ir, AvengerChartError>> + Send + 'a>> {
    Box::pin(async move {
        let derivation_by_node: HashMap<CoordNodeId, CoordPhase8NodeDerivation> = derivation
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
        Ok(CoordPhase8Ir { node_results })
    })
}

fn run_phase8_apply_recursive<'a>(
    measurement: &'a mut ComponentsMeasurement,
    eval_ctx: &'a EvaluationContext,
    derivation_by_node: &'a HashMap<CoordNodeId, CoordPhase8NodeDerivation>,
    node_path: &'a mut Vec<usize>,
    node_results: &'a mut Vec<CoordPhase8NodeResult>,
) -> Pin<Box<dyn Future<Output = Result<(), AvengerChartError>> + Send + 'a>> {
    Box::pin(async move {
        if let Some(facet_band) = facet_band_mut(measurement) {
            let node_id = CoordNodeId::new(node_path.clone());
            let axis = facet_band.axis;
            let subplot_cross_size_before = facet_band.subplot_cross_size;

            let derived = derivation_by_node.get(&node_id);
            let derived_has_legend_overflow = derived.is_some_and(|node| node.has_legend_overflow);
            let derived_has_coordinated_extents =
                derived.is_some_and(|node| node.has_coordinated_extents);
            let derived_remeasure_triggered = derived.is_some_and(|node| node.remeasure_triggered);
            let derived_child_count = derived.map_or(0, |node| node.child_count);

            facet_band.apply_coordinated_overflow(eval_ctx).await?;
            let subplot_cross_size_after = facet_band.subplot_cross_size;
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

            node_results.push(CoordPhase8NodeResult {
                node_id,
                axis,
                derived_has_legend_overflow,
                derived_has_coordinated_extents,
                derived_child_count,
                parent_cross_size_propagated,
                subplot_cross_size_before,
                subplot_cross_size_after,
                remeasure_triggered: derived_remeasure_triggered,
            });
        }
        Ok(())
    })
}

pub(crate) fn derive_phase10(measurement: &ComponentsMeasurement) -> CoordPhase10Derivation {
    let mut node_derivations = Vec::new();
    let mut node_path = Vec::new();
    derive_phase10_recursive(measurement, &mut node_path, &mut node_derivations);
    CoordPhase10Derivation { node_derivations }
}

fn derive_phase10_recursive(
    measurement: &ComponentsMeasurement,
    node_path: &mut Vec<usize>,
    node_derivations: &mut Vec<CoordPhase10NodeDerivation>,
) {
    if let Some(facet_band) = facet_band_ref(measurement) {
        for (idx, child) in facet_band.child_measurements_iter().enumerate() {
            node_path.push(idx);
            derive_phase10_recursive(child, node_path, node_derivations);
            node_path.pop();
        }

        node_derivations.push(CoordPhase10NodeDerivation {
            node_id: CoordNodeId::new(node_path.clone()),
            axis: facet_band.axis,
            parent_cross_size_target: facet_band.coordinated_subplot_cross_size(),
            child_count: facet_band.child_measurements_iter().count(),
        });
    }
}

pub(crate) fn run_phase10_scale_retarget_and_adjustments_with_trace(
    measurement: &mut ComponentsMeasurement,
    derivation: &CoordPhase10Derivation,
) -> CoordPhase10Ir {
    let derivation_by_node: HashMap<CoordNodeId, CoordPhase10NodeDerivation> = derivation
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
    CoordPhase10Ir { node_results }
}

fn run_phase10_scale_retarget_recursive(
    measurement: &mut ComponentsMeasurement,
    derivation_by_node: &HashMap<CoordNodeId, CoordPhase10NodeDerivation>,
    node_path: &mut Vec<usize>,
    node_results: &mut Vec<CoordPhase10NodeResult>,
) {
    measurement
        .coord_measurement
        .apply_scale_adjustments(&mut measurement.scales);

    if let Some(facet_band) = facet_band_mut(measurement) {
        let node_id = CoordNodeId::new(node_path.clone());
        let parent_width = facet_band.coordinated_subplot_cross_size();
        let axis = facet_band.axis;
        let mut child_plot_area_adjustments_count = 0usize;
        let mut scale_range_retarget_count = 0usize;

        for (idx, child) in facet_band.child_measurements_iter_mut().enumerate() {
            let old_plot_area_width = child.plot_area_width;
            let old_plot_area_height = child.plot_area_height;
            let mut plot_area_adjusted = false;

            if let Some(width) = parent_width {
                match axis {
                    FacetAxis::Column if (child.plot_area_width - width).abs() > 0.01 => {
                        trace!(
                            old_width = child.plot_area_width,
                            new_width = width,
                            axis = "column",
                            "coordinate_facet_measurement_tree updating child plot area cross-size"
                        );
                        child.plot_area_width = width;
                        plot_area_adjusted = true;
                    }
                    FacetAxis::Row if (child.plot_area_height - width).abs() > 0.01 => {
                        trace!(
                            old_height = child.plot_area_height,
                            new_height = width,
                            axis = "row",
                            "coordinate_facet_measurement_tree updating child plot area cross-size"
                        );
                        child.plot_area_height = width;
                        plot_area_adjusted = true;
                    }
                    _ => {}
                }

                if let Some(band_scale) = child.scales.get_mut(axis.scale_name()) {
                    let updated_config = band_scale
                        .configured()
                        .clone()
                        .with_range_interval((0.0, width));
                    *band_scale =
                        ConfiguredScaleWithSpec::new(band_scale.spec().clone(), updated_config);
                }
            }

            if plot_area_adjusted {
                child_plot_area_adjustments_count += 1;
            }

            if (child.plot_area_width - old_plot_area_width).abs() > 0.01
                || (child.plot_area_height - old_plot_area_height).abs() > 0.01
            {
                scale_range_retarget_count += retarget_child_scales_for_resized_plot_area(
                    child,
                    old_plot_area_width,
                    old_plot_area_height,
                );
            }
            node_path.push(idx);
            run_phase10_scale_retarget_recursive(
                child,
                derivation_by_node,
                node_path,
                node_results,
            );
            node_path.pop();
        }

        let derived = derivation_by_node.get(&node_id);
        node_results.push(CoordPhase10NodeResult {
            node_id,
            axis,
            derived_parent_cross_size_target: derived
                .and_then(|node| node.parent_cross_size_target),
            derived_child_count: derived.map_or(0, |node| node.child_count),
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
