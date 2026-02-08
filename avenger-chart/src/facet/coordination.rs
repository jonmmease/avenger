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
    pub facet_axis: CoordinationAxis,
    pub facet_field_identity: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoordinationAxis {
    Column,
    Row,
    Generic,
}

impl CoordinationGroupKey {
    pub fn new(
        depth: usize,
        facet_axis: CoordinationAxis,
        facet_field_identity: impl Into<String>,
    ) -> Self {
        Self {
            depth,
            facet_axis,
            facet_field_identity: facet_field_identity.into(),
        }
    }
}

fn facet_col_ref(measurement: &ComponentsMeasurement) -> Option<&FacetColCoordMeasurement> {
    measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetColCoordMeasurement>()
}

fn facet_col_mut(measurement: &mut ComponentsMeasurement) -> Option<&mut FacetColCoordMeasurement> {
    measurement
        .coord_measurement
        .as_any_mut()
        .downcast_mut::<FacetColCoordMeasurement>()
}

fn visit_facet_cols<F>(measurement: &ComponentsMeasurement, depth: usize, visit: &mut F)
where
    F: FnMut(usize, &FacetColCoordMeasurement),
{
    if let Some(facet_col) = facet_col_ref(measurement) {
        visit(depth, facet_col);
        for child in facet_col.child_measurements_iter() {
            visit_facet_cols(child, depth + 1, visit);
        }
    }
}

fn visit_facet_cols_mut<F>(measurement: &mut ComponentsMeasurement, depth: usize, visit: &mut F)
where
    F: FnMut(usize, &mut FacetColCoordMeasurement),
{
    if let Some(facet_col) = facet_col_mut(measurement) {
        visit(depth, facet_col);
        for child in facet_col.child_measurements_iter_mut() {
            visit_facet_cols_mut(child, depth + 1, visit);
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
    visit_facet_cols(measurement, 0, &mut |depth, facet_col| {
        if let Some(local) = facet_col.local_overflow_value() {
            let key = facet_col.coordination_group_key_for_depth(depth);
            overflow_by_key.entry(key).or_default().push(local);
        }
    });

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

    visit_facet_cols_mut(measurement, 0, &mut |depth, facet_col| {
        let key = facet_col.coordination_group_key_for_depth(depth);
        if let Some(max_overflow) = max_by_key.get(&key).cloned() {
            facet_col.set_coordinated_overflow_value(max_overflow);
        }
    });
}

fn distribute_coordinated_layout(measurement: &mut ComponentsMeasurement) {
    let mut layout_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedLayout>> = HashMap::new();
    visit_facet_cols(measurement, 0, &mut |depth, facet_col| {
        let key = facet_col.coordination_group_key_for_depth(depth);
        layout_by_key
            .entry(key)
            .or_default()
            .push(facet_col.local_layout_value());
    });

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

    visit_facet_cols_mut(measurement, 0, &mut |depth, facet_col| {
        let key = facet_col.coordination_group_key_for_depth(depth);
        if let Some(max_layout) = max_by_key.get(&key).cloned() {
            facet_col.set_coordinated_layout_value(max_layout);
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

fn coordinate_cell_domain_extents(measurement: &mut ComponentsMeasurement) {
    let mut all_extents = Vec::new();
    visit_facet_cols(measurement, 0, &mut |_, facet_col| {
        facet_col.collect_cell_domain_infos(&mut all_extents);
    });

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

    visit_facet_cols_mut(measurement, 0, &mut |_, facet_col| {
        facet_col.distribute_coordinated_domain_extents(&unified);
    });
}

fn apply_coordinated_overflow_recursive<'a>(
    measurement: &'a mut ComponentsMeasurement,
    eval_ctx: &'a EvaluationContext,
) -> Pin<Box<dyn Future<Output = Result<(), AvengerChartError>> + Send + 'a>> {
    Box::pin(async move {
        if let Some(facet_col) = facet_col_mut(measurement) {
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

    if let Some(facet_col) = facet_col_mut(measurement) {
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
