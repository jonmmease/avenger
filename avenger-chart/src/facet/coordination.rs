//! Facet-specific coordination keys and grouping helpers.
//!
//! This module provides stable keys for grouping "equivalent" facet nodes across
//! branches during coordination passes (overflow/layout/domain distribution).

use std::{collections::HashMap, future::Future, pin::Pin};

use datafusion::common::ScalarValue;

use crate::{
    coords::{CellDomainInfo, CoordinatedLayout, CoordinatedOverflow},
    error::AvengerChartError,
    facet::{
        coord::{FacetColCoordMeasurement, union_domain_extents},
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
    measurement: &ComponentsMeasurement,
    depth: usize,
) -> CoordinationGroupKey {
    measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetColCoordMeasurement>()
        .map(|facet_col| facet_col.coordination_group_key_for_depth(depth))
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
    if let Some(local) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetColCoordMeasurement>()
        .and_then(FacetColCoordMeasurement::local_overflow_value)
    {
        let key = key_for_measurement(measurement, depth);
        registry.entry(key).or_default().push(local);
    }

    if let Some(facet_col) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetColCoordMeasurement>()
    {
        for child in facet_col.child_measurements_iter() {
            collect_overflow_by_key(child, depth + 1, registry);
        }
    }
}

fn distribute_overflow_by_key(
    measurement: &mut ComponentsMeasurement,
    depth: usize,
    max_by_key: &HashMap<CoordinationGroupKey, CoordinatedOverflow>,
) {
    let key = key_for_measurement(measurement, depth);
    if let Some(max_overflow) = max_by_key.get(&key).cloned() {
        if let Some(facet_col) = measurement
            .coord_measurement
            .as_any_mut()
            .downcast_mut::<FacetColCoordMeasurement>()
        {
            facet_col.set_coordinated_overflow_value(max_overflow);
        }
    }

    if let Some(facet_col) = measurement
        .coord_measurement
        .as_any_mut()
        .downcast_mut::<FacetColCoordMeasurement>()
    {
        for child in facet_col.child_measurements_iter_mut() {
            distribute_overflow_by_key(child, depth + 1, max_by_key);
        }
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
    if let Some(facet_col) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetColCoordMeasurement>()
    {
        let key = key_for_measurement(measurement, depth);
        registry
            .entry(key)
            .or_default()
            .push(facet_col.local_layout_value());

        for child in facet_col.child_measurements_iter() {
            collect_layout_by_key(child, depth + 1, registry);
        }
    }
}

fn distribute_layout_by_key(
    measurement: &mut ComponentsMeasurement,
    depth: usize,
    max_by_key: &HashMap<CoordinationGroupKey, CoordinatedLayout>,
) {
    let key = key_for_measurement(measurement, depth);
    if let Some(max_layout) = max_by_key.get(&key).cloned() {
        if let Some(facet_col) = measurement
            .coord_measurement
            .as_any_mut()
            .downcast_mut::<FacetColCoordMeasurement>()
        {
            facet_col.set_coordinated_layout_value(max_layout);
        }
    }

    if let Some(facet_col) = measurement
        .coord_measurement
        .as_any_mut()
        .downcast_mut::<FacetColCoordMeasurement>()
    {
        for child in facet_col.child_measurements_iter_mut() {
            distribute_layout_by_key(child, depth + 1, max_by_key);
        }
    }
}

fn collect_cell_domain_extents_recursive(
    measurement: &ComponentsMeasurement,
    collector: &mut Vec<CellDomainInfo>,
) {
    if let Some(facet_col) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetColCoordMeasurement>()
    {
        facet_col.collect_cell_domain_infos(collector);

        for child in facet_col.child_measurements_iter() {
            collect_cell_domain_extents_recursive(child, collector);
        }
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
    if let Some(facet_col) = measurement
        .coord_measurement
        .as_any_mut()
        .downcast_mut::<FacetColCoordMeasurement>()
    {
        facet_col.distribute_coordinated_domain_extents(unified);

        for child in facet_col.child_measurements_iter_mut() {
            distribute_cell_domain_extents_recursive(child, unified);
        }
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
        if let Some(facet_col) = measurement
            .coord_measurement
            .as_any_mut()
            .downcast_mut::<FacetColCoordMeasurement>()
        {
            facet_col.apply_coordinated_overflow(eval_ctx).await?;
            let parent_width = facet_col.coordinated_subplot_width_value();

            for child in facet_col.child_measurements_iter_mut() {
                if let Some(width) = parent_width {
                    if let Some(child_facet_col) = child
                        .coord_measurement
                        .as_any_mut()
                        .downcast_mut::<FacetColCoordMeasurement>()
                    {
                        child_facet_col.set_parent_bandwidth_value(width);
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

    if let Some(facet_col) = measurement
        .coord_measurement
        .as_any_mut()
        .downcast_mut::<FacetColCoordMeasurement>()
    {
        let parent_width = facet_col.coordinated_subplot_width_value();

        for child in facet_col.child_measurements_iter_mut() {
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
                        *column_scale = ConfiguredScaleWithSpec::new(
                            column_scale.spec().clone(),
                            updated_config,
                        );
                    }
                }
            }
            reapply_scale_adjustments_recursive(child);
        }
    }
}
