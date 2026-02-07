//! Facet-specific coordination keys and grouping helpers.
//!
//! This module provides stable keys for grouping "equivalent" facet nodes across
//! branches during coordination passes (overflow/layout/domain distribution).

use std::{collections::HashMap, future::Future, pin::Pin};

use datafusion::common::ScalarValue;

use crate::{
    coords::{CellDomainInfo, CoordMeasurement, CoordinatedLayout, CoordinatedOverflow},
    error::AvengerChartError,
    facet::{coord::union_domain_extents, sharing_policy},
    plot::compiled::ComponentsMeasurement,
    render::EvaluationContext,
    scales::{ConfiguredScaleWithSpec, domain_extent::DomainExtent},
};

/// Stable key identifying a coordination group in the measurement tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CoordinationGroupKey {
    pub depth: usize,
    pub facet_axis: FacetAxis,
    pub facet_field_identity: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FacetAxis {
    Column,
    Row,
    Generic,
}

impl CoordinationGroupKey {
    pub fn new(
        depth: usize,
        facet_axis: FacetAxis,
        facet_field_identity: impl Into<String>,
    ) -> Self {
        Self {
            depth,
            facet_axis,
            facet_field_identity: facet_field_identity.into(),
        }
    }

    /// Fallback key used by non-facet coords that do not provide explicit identity.
    pub fn fallback(depth: usize) -> Self {
        Self::new(depth, FacetAxis::Generic, "generic")
    }
}

/// Resolve the coordination key for a node, falling back to depth-only grouping.
pub fn key_for_measurement(
    measurement: &dyn CoordMeasurement,
    depth: usize,
) -> CoordinationGroupKey {
    measurement
        .coordination_group_key(depth)
        .unwrap_or_else(|| CoordinationGroupKey::fallback(depth))
}

/// Coordinate overflow, layout, and domains across the measurement tree.
///
/// This is the facet-specific coordination entrypoint invoked from `coords.rs`.
pub async fn coordinate_facet_measurement_tree(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<(), AvengerChartError> {
    // 1) distribute shared overflow by coordination key
    distribute_coordinated_overflow(measurement);
    // 2) distribute shared layout params by coordination key
    distribute_coordinated_layout(measurement);
    // 3) unify/distribute domain extents by sharing policy
    coordinate_cell_domain_extents(measurement);
    // 4) apply coordinated overflow/layout/domain to children
    apply_coordinated_overflow_recursive(measurement, eval_ctx).await?;
    // 5) re-distribute because step 4 may replace child measurements
    distribute_coordinated_overflow(measurement);
    distribute_coordinated_layout(measurement);
    // 6) update stored scales and propagated widths after coordination
    reapply_scale_adjustments_recursive(measurement);

    Ok(())
}

fn distribute_coordinated_overflow(measurement: &mut ComponentsMeasurement) {
    let mut overflow_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedOverflow>> =
        HashMap::new();
    collect_overflow_by_key(measurement, 0, &mut overflow_by_key);

    let max_by_key: HashMap<CoordinationGroupKey, CoordinatedOverflow> = overflow_by_key
        .into_iter()
        .map(|(key, values)| {
            let mut merged = CoordinatedOverflow::default();
            for v in &values {
                merged.merge(v);
            }
            (key, merged)
        })
        .collect();

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!("coordinate_facet_measurement_tree overflow: {max_by_key:?}");
    }

    distribute_overflow_by_key(measurement, 0, &max_by_key);
}

fn collect_overflow_by_key(
    measurement: &ComponentsMeasurement,
    depth: usize,
    registry: &mut HashMap<CoordinationGroupKey, Vec<CoordinatedOverflow>>,
) {
    if let Some(local) = measurement.coord_measurement.local_overflow() {
        let key = key_for_measurement(measurement.coord_measurement.as_ref(), depth);
        registry.entry(key).or_default().push(local);
    }

    for child in measurement.coord_measurement.child_measurements() {
        collect_overflow_by_key(child, depth + 1, registry);
    }
}

fn distribute_overflow_by_key(
    measurement: &mut ComponentsMeasurement,
    depth: usize,
    max_by_key: &HashMap<CoordinationGroupKey, CoordinatedOverflow>,
) {
    let key = key_for_measurement(measurement.coord_measurement.as_ref(), depth);
    if let Some(max_overflow) = max_by_key.get(&key) {
        measurement
            .coord_measurement
            .set_coordinated_overflow(max_overflow.clone());
    }

    for child in measurement.coord_measurement.child_measurements_mut() {
        distribute_overflow_by_key(child, depth + 1, max_by_key);
    }
}

fn distribute_coordinated_layout(measurement: &mut ComponentsMeasurement) {
    let mut layout_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedLayout>> = HashMap::new();
    collect_layout_by_key(measurement, 0, &mut layout_by_key);

    if layout_by_key.is_empty() {
        return;
    }

    let max_by_key: HashMap<CoordinationGroupKey, CoordinatedLayout> = layout_by_key
        .into_iter()
        .map(|(key, values)| {
            let mut merged = CoordinatedLayout::default();
            for v in &values {
                merged.merge(v);
            }
            (key, merged)
        })
        .collect();

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!("coordinate_facet_measurement_tree layout: {max_by_key:?}");
    }

    distribute_layout_by_key(measurement, 0, &max_by_key);
}

fn collect_layout_by_key(
    measurement: &ComponentsMeasurement,
    depth: usize,
    registry: &mut HashMap<CoordinationGroupKey, Vec<CoordinatedLayout>>,
) {
    if let Some(local) = measurement.coord_measurement.local_layout() {
        let key = key_for_measurement(measurement.coord_measurement.as_ref(), depth);
        registry.entry(key).or_default().push(local);
    }

    for child in measurement.coord_measurement.child_measurements() {
        collect_layout_by_key(child, depth + 1, registry);
    }
}

fn distribute_layout_by_key(
    measurement: &mut ComponentsMeasurement,
    depth: usize,
    max_by_key: &HashMap<CoordinationGroupKey, CoordinatedLayout>,
) {
    let key = key_for_measurement(measurement.coord_measurement.as_ref(), depth);
    if let Some(max_layout) = max_by_key.get(&key) {
        measurement
            .coord_measurement
            .set_coordinated_layout(max_layout.clone());
    }

    for child in measurement.coord_measurement.child_measurements_mut() {
        distribute_layout_by_key(child, depth + 1, max_by_key);
    }
}

fn collect_cell_domain_extents_recursive(
    measurement: &ComponentsMeasurement,
    collector: &mut Vec<CellDomainInfo>,
) {
    measurement
        .coord_measurement
        .collect_cell_domain_extents(collector);

    for child in measurement.coord_measurement.child_measurements() {
        collect_cell_domain_extents_recursive(child, collector);
    }
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

fn distribute_cell_domain_extents_recursive(
    measurement: &mut ComponentsMeasurement,
    unified: &HashMap<(String, Vec<ScalarValue>), DomainExtent>,
) {
    measurement
        .coord_measurement
        .distribute_cell_domain_extents(unified);

    for child in measurement.coord_measurement.child_measurements_mut() {
        distribute_cell_domain_extents_recursive(child, unified);
    }
}

fn coordinate_cell_domain_extents(measurement: &mut ComponentsMeasurement) {
    let mut all_extents = Vec::new();
    collect_cell_domain_extents_recursive(measurement, &mut all_extents);

    if all_extents.is_empty() {
        return;
    }

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "coordinate_facet_measurement_tree domains: collected {} extents",
            all_extents.len()
        );
    }

    let unified = aggregate_domain_extents(&all_extents);

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "coordinate_facet_measurement_tree domains: unified into {} groups",
            unified.len()
        );
    }

    distribute_cell_domain_extents_recursive(measurement, &unified);
}

fn apply_coordinated_overflow_recursive<'a>(
    measurement: &'a mut ComponentsMeasurement,
    eval_ctx: &'a EvaluationContext,
) -> Pin<Box<dyn Future<Output = Result<(), AvengerChartError>> + Send + 'a>> {
    Box::pin(async move {
        measurement
            .coord_measurement
            .apply_coordinated_overflow(eval_ctx)
            .await?;

        let parent_width = measurement.coord_measurement.coordinated_subplot_width();

        for child in measurement.coord_measurement.child_measurements_mut() {
            if let Some(width) = parent_width {
                child.coord_measurement.set_parent_bandwidth(width);
            }
            apply_coordinated_overflow_recursive(child, eval_ctx).await?;
        }

        Ok(())
    })
}

fn reapply_scale_adjustments_recursive(measurement: &mut ComponentsMeasurement) {
    measurement
        .coord_measurement
        .apply_scale_adjustments(&mut measurement.scales);

    let parent_width = measurement.coord_measurement.coordinated_subplot_width();

    for child in measurement.coord_measurement.child_measurements_mut() {
        if let Some(width) = parent_width {
            if (child.plot_area_width - width).abs() > 0.01 {
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "coordinate_facet_measurement_tree: child plot_area_width {:.1} -> {:.1}",
                        child.plot_area_width, width
                    );
                }
                child.plot_area_width = width;

                if let Some(column_scale) = child.scales.get_mut("column") {
                    let updated_config = column_scale
                        .configured()
                        .clone()
                        .with_range_interval((0.0, width));
                    *column_scale =
                        ConfiguredScaleWithSpec::new(column_scale.spec().clone(), updated_config);
                }
            }
        }
        reapply_scale_adjustments_recursive(child);
    }
}
