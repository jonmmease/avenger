//! Facet-specific coordination keys and grouping helpers.
//!
//! This module provides stable keys for grouping "equivalent" facet nodes across
//! branches during coordination passes (overflow/layout/domain distribution).

use std::{collections::HashMap, future::Future, pin::Pin};

use datafusion::common::ScalarValue;
use tracing::{debug, trace};

use crate::{
    coords::{CellDomainInfo, CoordinatedLayout, CoordinatedOverflow, FacetAxis},
    error::AvengerChartError,
    facet::{
        coord::{FacetBandCoordMeasurement, union_domain_extents},
        sharing_policy,
    },
    plot::compiled::ComponentsMeasurement,
    render::EvaluationContext,
    scales::{ConfiguredScaleWithSpec, domain_extent::DomainExtent},
};

/// Stable key identifying a coordination group in the measurement tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CoordinationGroupKey {
    pub depth: usize,
    pub facet_group_identity: String,
}

impl CoordinationGroupKey {
    pub fn new(depth: usize, facet_group_identity: impl Into<String>) -> Self {
        Self {
            depth,
            facet_group_identity: facet_group_identity.into(),
        }
    }
}

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

fn visit_facet_bands<F>(measurement: &ComponentsMeasurement, depth: usize, visit: &mut F)
where
    F: FnMut(usize, &FacetBandCoordMeasurement),
{
    if let Some(facet_band) = facet_band_ref(measurement) {
        visit(depth, facet_band);
        for child in facet_band.child_measurements_iter() {
            visit_facet_bands(child, depth + 1, visit);
        }
    }
}

fn visit_facet_bands_mut<F>(measurement: &mut ComponentsMeasurement, depth: usize, visit: &mut F)
where
    F: FnMut(usize, &mut FacetBandCoordMeasurement),
{
    if let Some(facet_band) = facet_band_mut(measurement) {
        visit(depth, facet_band);
        for child in facet_band.child_measurements_iter_mut() {
            visit_facet_bands_mut(child, depth + 1, visit);
        }
    }
}

/// Coordinate overflow, layout, and domains across the measurement tree.
///
/// This is the facet-specific coordination entrypoint invoked from `coords.rs`.
pub async fn coordinate_facet_measurement_tree(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<(), AvengerChartError> {
    // Invariants for facet coordination:
    // 1. Initial phase coordinates overflow/layout/domain extents across equivalent nodes.
    // 2. Remeasure can replace child measurements, so overflow/layout must be recollected.
    // 3. Final scale adjustment pass propagates coordinated widths to descendants.
    run_initial_coordination_phase(measurement);
    debug!("coordinate_facet_measurement_tree initial phase complete");

    apply_coordinated_overflow_recursive(measurement, eval_ctx).await?;
    debug!("coordinate_facet_measurement_tree remeasure phase complete");

    run_post_remeasure_coordination_phase(measurement);
    debug!("coordinate_facet_measurement_tree post-remeasure phase complete");

    reapply_scale_adjustments_recursive(measurement);
    trace!("facet coordination complete");

    Ok(())
}

#[derive(Default)]
struct CoordinationSnapshot {
    overflow_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedOverflow>>,
    layout_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedLayout>>,
    domain_infos: Vec<CellDomainInfo>,
}

struct CoordinationAggregates {
    merged_overflow_by_key: HashMap<CoordinationGroupKey, CoordinatedOverflow>,
    merged_layout_by_key: HashMap<CoordinationGroupKey, CoordinatedLayout>,
    unified_domain_extents: HashMap<(String, Vec<ScalarValue>), DomainExtent>,
}

fn collect_coordination_snapshot(
    measurement: &ComponentsMeasurement,
    include_domains: bool,
) -> CoordinationSnapshot {
    let mut snapshot = CoordinationSnapshot::default();
    visit_facet_bands(measurement, 0, &mut |depth, facet_band| {
        let key = facet_band.coordination_group_key_for_depth(depth);
        if let Some(local_overflow) = facet_band.local_overflow_value() {
            snapshot
                .overflow_by_key
                .entry(key.clone())
                .or_default()
                .push(local_overflow);
        }
        snapshot
            .layout_by_key
            .entry(key)
            .or_default()
            .push(facet_band.local_layout_value());

        if include_domains {
            facet_band.collect_cell_domain_infos(&mut snapshot.domain_infos);
        }
    });
    snapshot
}

fn merge_overflow_groups(
    overflow_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedOverflow>>,
) -> HashMap<CoordinationGroupKey, CoordinatedOverflow> {
    overflow_by_key
        .into_iter()
        .map(|(key, values)| {
            let mut merged = CoordinatedOverflow::default();
            for v in &values {
                merged.merge(v);
            }
            (key, merged)
        })
        .collect()
}

fn merge_layout_groups(
    layout_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedLayout>>,
) -> HashMap<CoordinationGroupKey, CoordinatedLayout> {
    layout_by_key
        .into_iter()
        .map(|(key, values)| {
            let mut merged = CoordinatedLayout::default();
            for v in &values {
                merged.merge(v);
            }
            (key, merged)
        })
        .collect()
}

fn aggregate_snapshot(
    snapshot: CoordinationSnapshot,
    include_domains: bool,
) -> CoordinationAggregates {
    let unified_domain_extents = if include_domains && !snapshot.domain_infos.is_empty() {
        aggregate_domain_extents(&snapshot.domain_infos)
    } else {
        HashMap::new()
    };

    CoordinationAggregates {
        merged_overflow_by_key: merge_overflow_groups(snapshot.overflow_by_key),
        merged_layout_by_key: merge_layout_groups(snapshot.layout_by_key),
        unified_domain_extents,
    }
}

fn run_initial_coordination_phase(measurement: &mut ComponentsMeasurement) {
    let initial_snapshot = collect_coordination_snapshot(measurement, true);
    let initial_aggregates = aggregate_snapshot(initial_snapshot, true);

    debug!(
        overflow_groups = initial_aggregates.merged_overflow_by_key.len(),
        layout_groups = initial_aggregates.merged_layout_by_key.len(),
        domain_groups = initial_aggregates.unified_domain_extents.len(),
        "coordinate_facet_measurement_tree initial coordination phase"
    );

    apply_aggregates(measurement, &initial_aggregates, true);
}

fn run_post_remeasure_coordination_phase(measurement: &mut ComponentsMeasurement) {
    let post_snapshot = collect_coordination_snapshot(measurement, false);
    let post_aggregates = aggregate_snapshot(post_snapshot, false);

    debug!(
        overflow_groups = post_aggregates.merged_overflow_by_key.len(),
        layout_groups = post_aggregates.merged_layout_by_key.len(),
        "coordinate_facet_measurement_tree post-remeasure coordination phase"
    );

    apply_aggregates(measurement, &post_aggregates, false);
}

fn apply_aggregates(
    measurement: &mut ComponentsMeasurement,
    aggregates: &CoordinationAggregates,
    apply_domains: bool,
) {
    visit_facet_bands_mut(measurement, 0, &mut |depth, facet_band| {
        let key = facet_band.coordination_group_key_for_depth(depth);
        if let Some(overflow) = aggregates.merged_overflow_by_key.get(&key).cloned() {
            facet_band.set_coordinated_overflow_value(overflow);
        }
        if let Some(layout) = aggregates.merged_layout_by_key.get(&key).cloned() {
            facet_band.set_coordinated_layout_value(layout);
        }
        if apply_domains && !aggregates.unified_domain_extents.is_empty() {
            facet_band.distribute_coordinated_domain_extents(&aggregates.unified_domain_extents);
        }
    });
}

fn aggregate_domain_extents(
    infos: &[CellDomainInfo],
) -> HashMap<(String, Vec<ScalarValue>), DomainExtent> {
    let mut groups: HashMap<(String, Vec<ScalarValue>), Vec<&DomainExtent>> = HashMap::new();

    for info in infos {
        let ancestor_key = sharing_policy::domain_group_key(
            &info.full_cell_path,
            info.sharing_level,
            info.facet_depth,
        );
        groups
            .entry((info.channel.clone(), ancestor_key))
            .or_default()
            .push(&info.extent);
    }

    groups
        .into_iter()
        .map(|(key, extents)| {
            let unified = extents
                .into_iter()
                .fold(None, |acc: Option<DomainExtent>, extent| match acc {
                    None => Some(extent.clone()),
                    Some(acc) => Some(union_domain_extents(&acc, extent)),
                })
                .unwrap();
            (key, unified)
        })
        .collect()
}

fn apply_coordinated_overflow_recursive<'a>(
    measurement: &'a mut ComponentsMeasurement,
    eval_ctx: &'a EvaluationContext,
) -> Pin<Box<dyn Future<Output = Result<(), AvengerChartError>> + Send + 'a>> {
    Box::pin(async move {
        if let Some(facet_band) = facet_band_mut(measurement) {
            facet_band.apply_coordinated_overflow(eval_ctx).await?;
            let parent_cross_size = facet_band.coordinated_subplot_cross_size();
            let parent_axis = facet_band.axis;

            for child in facet_band.child_measurements_iter_mut() {
                if let Some(cross_size) = parent_cross_size {
                    if let Some(child_facet_band) = child
                        .coord_measurement
                        .as_any_mut()
                        .downcast_mut::<FacetBandCoordMeasurement>()
                    {
                        if child_facet_band.axis == parent_axis {
                            child_facet_band.set_parent_bandwidth_value(cross_size);
                        }
                    }
                }
                apply_coordinated_overflow_recursive(child, eval_ctx).await?;
            }
        }

        Ok(())
    })
}

fn reapply_scale_adjustments_recursive(measurement: &mut ComponentsMeasurement) {
    measurement
        .coord_measurement
        .apply_scale_adjustments(&mut measurement.scales);

    if let Some(facet_band) = facet_band_mut(measurement) {
        let parent_width = facet_band.coordinated_subplot_cross_size();
        let axis = facet_band.axis;

        for child in facet_band.child_measurements_iter_mut() {
            let old_plot_area_width = child.plot_area_width;
            let old_plot_area_height = child.plot_area_height;

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
                    }
                    FacetAxis::Row if (child.plot_area_height - width).abs() > 0.01 => {
                        trace!(
                            old_height = child.plot_area_height,
                            new_height = width,
                            axis = "row",
                            "coordinate_facet_measurement_tree updating child plot area cross-size"
                        );
                        child.plot_area_height = width;
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

            if (child.plot_area_width - old_plot_area_width).abs() > 0.01
                || (child.plot_area_height - old_plot_area_height).abs() > 0.01
            {
                retarget_child_scales_for_resized_plot_area(
                    child,
                    old_plot_area_width,
                    old_plot_area_height,
                );
            }
            reapply_scale_adjustments_recursive(child);
        }
    }
}

fn retarget_child_scales_for_resized_plot_area(
    child: &mut ComponentsMeasurement,
    old_plot_area_width: f32,
    old_plot_area_height: f32,
) {
    let new_plot_area_width = child.plot_area_width;
    let new_plot_area_height = child.plot_area_height;

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
