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
    // Phase 7: Global aggregate + distribution (overflow/layout/domain extents).
    run_phase7_global_aggregate_and_distribution(measurement);
    debug!("coordinate_facet_measurement_tree phase 7 complete");

    // Phase 8: Coordinated apply + selective remeasure.
    run_phase8_apply_coordinated_overflow_and_remeasure(measurement, eval_ctx).await?;
    debug!("coordinate_facet_measurement_tree phase 8 complete");

    // Phase 9: Post-remeasure reconciliation.
    run_phase9_post_remeasure_reconciliation(measurement);
    debug!("coordinate_facet_measurement_tree phase 9 complete");

    // Phase 10: Scale retarget + adjustment propagation.
    run_phase10_scale_retarget_and_adjustments(measurement);
    trace!("coordinate_facet_measurement_tree phase 10 complete");

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

fn run_phase7_global_aggregate_and_distribution(measurement: &mut ComponentsMeasurement) {
    let initial_snapshot = collect_coordination_snapshot(measurement, true);
    let initial_aggregates = aggregate_snapshot(initial_snapshot, true);

    debug!(
        overflow_groups = initial_aggregates.merged_overflow_by_key.len(),
        layout_groups = initial_aggregates.merged_layout_by_key.len(),
        domain_groups = initial_aggregates.unified_domain_extents.len(),
        "coordinate_facet_measurement_tree phase 7 global aggregate + distribution"
    );

    apply_aggregates(measurement, &initial_aggregates, true);
}

fn run_phase9_post_remeasure_reconciliation(measurement: &mut ComponentsMeasurement) {
    let post_snapshot = collect_coordination_snapshot(measurement, false);
    let post_aggregates = aggregate_snapshot(post_snapshot, false);

    debug!(
        overflow_groups = post_aggregates.merged_overflow_by_key.len(),
        layout_groups = post_aggregates.merged_layout_by_key.len(),
        "coordinate_facet_measurement_tree phase 9 post-remeasure reconciliation"
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

fn run_phase8_apply_coordinated_overflow_and_remeasure<'a>(
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
                run_phase8_apply_coordinated_overflow_and_remeasure(child, eval_ctx).await?;
            }
        }

        Ok(())
    })
}

fn run_phase10_scale_retarget_and_adjustments(measurement: &mut ComponentsMeasurement) {
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
            run_phase10_scale_retarget_and_adjustments(child);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        coords::{CoordinatedLayout, CoordinatedOverflow, FacetAxis},
        facet::evaluated_facet_tree::EvaluatedFacetTree,
        layout::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode},
        plot::compiled::{
            ComponentsMeasurement, scale_provider::DynamicScaleProvider,
            scales::build_scale_builder_from_marks,
        },
        prelude::*,
        render::EvaluationContext,
        theme::Theme,
    };
    use datafusion::{dataframe::DataFrame, prelude::SessionContext};
    use indexmap::IndexMap;
    use std::sync::Arc;

    #[derive(Clone)]
    struct Depth1State {
        key: CoordinationGroupKey,
        local_layout: CoordinatedLayout,
        coordinated_layout: Option<CoordinatedLayout>,
        coordinated_overflow: CoordinatedOverflow,
    }

    fn approx_eq_within(lhs: f32, rhs: f32, tolerance: f32) -> bool {
        (lhs - rhs).abs() <= tolerance
    }

    fn assert_layout_close(actual: &CoordinatedLayout, expected: &CoordinatedLayout) {
        assert!(approx_eq_within(
            actual.padding_inner_px,
            expected.padding_inner_px,
            0.01
        ));
        assert!(approx_eq_within(
            actual.outer_start,
            expected.outer_start,
            0.01
        ));
        assert!(approx_eq_within(actual.outer_end, expected.outer_end, 0.01));
        assert_eq!(actual.n, expected.n);
    }

    fn assert_overflow_close(actual: &CoordinatedOverflow, expected: &CoordinatedOverflow) {
        assert!(approx_eq_within(actual.guide.top, expected.guide.top, 0.01));
        assert!(approx_eq_within(
            actual.guide.right,
            expected.guide.right,
            0.01
        ));
        assert!(approx_eq_within(
            actual.guide.bottom,
            expected.guide.bottom,
            0.01
        ));
        assert!(approx_eq_within(
            actual.guide.left,
            expected.guide.left,
            0.01
        ));
        assert!(approx_eq_within(actual.total.top, expected.total.top, 0.01));
        assert!(approx_eq_within(
            actual.total.right,
            expected.total.right,
            0.01
        ));
        assert!(approx_eq_within(
            actual.total.bottom,
            expected.total.bottom,
            0.01
        ));
        assert!(approx_eq_within(
            actual.total.left,
            expected.total.left,
            0.01
        ));
    }

    fn depth1_states(measurement: &ComponentsMeasurement) -> Vec<Depth1State> {
        let mut states = Vec::new();
        visit_facet_bands(measurement, 0, &mut |depth, facet_band| {
            if depth == 1 {
                states.push(Depth1State {
                    key: facet_band.coordination_group_key_for_depth(depth),
                    local_layout: facet_band.local_layout.clone(),
                    coordinated_layout: facet_band.coordinated_layout.clone(),
                    coordinated_overflow: facet_band.coordinated_overflow.clone(),
                });
            }
        });
        states
    }

    fn build_nested_plot(df: DataFrame) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(640, 420)
            .mark(
                Facet::new()
                    .col_with(col("outer_group"), |c| c.facet(|f| f.title("Outer Group")))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("inner_group"), |c| {
                                    c.facet(|f| f.title("Inner Group"))
                                })
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x(col("x"))
                                            .y(col("y"))
                                            .fill("#4682b4")
                                            .size(42.0),
                                    ),
                                ),
                        ),
                    ),
            )
    }

    async fn nested_fixture()
    -> Result<(ComponentsMeasurement, EvaluationContext), AvengerChartError> {
        let session = SessionContext::new();
        let data_df = session
            .sql(
                "SELECT * FROM (VALUES \
                 ('A', 'X', 1.0, 10.0), \
                 ('A', 'Y', 2.0, 12.0), \
                 ('B', 'X', 3.0, 14.0), \
                 ('B', 'Y', 4.0, 16.0), \
                 ('B', 'Y', 5.0, 18.0) \
                 ) AS t(outer_group, inner_group, x, y)",
            )
            .await
            .map_err(|e| AvengerChartError::InternalError(e.to_string()))?;

        let plot = build_nested_plot(data_df);
        let compiled_plot = plot.compile(&session).await?;

        let facet_tree =
            Arc::new(EvaluatedFacetTree::from_compiled_plot(&compiled_plot, &session).await?);
        let eval_ctx = EvaluationContext::new(
            Arc::new(Theme::light()),
            Arc::new(session.clone()),
            IndexMap::new(),
            facet_tree,
        );

        let theme = compiled_plot.get_theme();
        let scale_builder = build_scale_builder_from_marks(
            &compiled_plot.marks,
            &compiled_plot.scale_specs,
            &compiled_plot.coord_transform,
            &compiled_plot.data,
            None,
            &session,
            &eval_ctx.params,
            theme.as_ref(),
        )
        .await?;

        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: &compiled_plot,
        };

        let evaluated_layout_spec = EvaluatedLayoutSpec {
            canvas: EvaluatedSizeMode::Fixed {
                width: 640.0,
                height: 420.0,
            },
            plot_area: EvaluatedSizeMode::Auto,
            margins: EvaluatedMargins {
                top: 10.0,
                right: 10.0,
                bottom: 10.0,
                left: 10.0,
            },
        };

        let measurement = compiled_plot
            .measure_plot_components(&eval_ctx, &evaluated_layout_spec, &provider, None, &[])
            .await?;

        Ok((measurement, eval_ctx))
    }

    #[tokio::test]
    async fn phase7_applies_initial_aggregates_to_all_matching_groups()
    -> Result<(), AvengerChartError> {
        let (mut measurement, _) = nested_fixture().await?;

        run_phase7_global_aggregate_and_distribution(&mut measurement);

        let states = depth1_states(&measurement);
        assert!(
            states.len() >= 2,
            "fixture should yield at least two depth-1 facet bands"
        );

        let first = states.first().expect("at least one depth-1 state");
        let expected_layout = first
            .coordinated_layout
            .as_ref()
            .expect("phase 7 should set coordinated layout");
        for state in states.iter().skip(1) {
            assert_eq!(state.key, first.key);
            assert_overflow_close(&state.coordinated_overflow, &first.coordinated_overflow);
            let actual_layout = state
                .coordinated_layout
                .as_ref()
                .expect("phase 7 should set coordinated layout");
            assert_layout_close(actual_layout, expected_layout);
        }

        Ok(())
    }

    #[tokio::test]
    async fn phase8_can_mutate_measurements_via_apply_coordinated_overflow()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;

        let before_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;
        {
            let root = facet_band_mut(&mut measurement)
                .expect("fixture should produce root facet-band measurement");
            let mut forced_layout = root.local_layout.clone();
            forced_layout.n = forced_layout.n.saturating_add(8);
            forced_layout.padding_inner_px += 12.0;
            root.set_coordinated_layout_value(forced_layout);
        }

        run_phase8_apply_coordinated_overflow_and_remeasure(&mut measurement, &eval_ctx).await?;

        let after_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;
        assert!(
            (after_cross_size - before_cross_size).abs() > 0.01,
            "phase 8 should mutate subplot cross size when coordinated layout differs"
        );

        Ok(())
    }

    #[tokio::test]
    async fn phase9_reconciles_layout_after_phase8_remeasure() -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;

        run_phase7_global_aggregate_and_distribution(&mut measurement);
        run_phase8_apply_coordinated_overflow_and_remeasure(&mut measurement, &eval_ctx).await?;

        let mut bumped_one = false;
        visit_facet_bands_mut(&mut measurement, 0, &mut |depth, facet_band| {
            if depth == 1 {
                facet_band.coordinated_layout = None;
                if !bumped_one {
                    facet_band.local_layout.padding_inner_px += 11.0;
                    facet_band.local_layout.n = facet_band.local_layout.n.saturating_add(2);
                    bumped_one = true;
                }
            }
        });
        assert!(bumped_one, "fixture should have a depth-1 band to perturb");

        let expected_states = depth1_states(&measurement);
        let expected_padding = expected_states
            .iter()
            .map(|state| state.local_layout.padding_inner_px)
            .fold(0.0f32, f32::max);
        let expected_n = expected_states
            .iter()
            .map(|state| state.local_layout.n)
            .max()
            .unwrap_or(0);

        run_phase9_post_remeasure_reconciliation(&mut measurement);

        let states = depth1_states(&measurement);
        assert!(
            states.len() >= 2,
            "fixture should yield at least two depth-1 facet bands"
        );
        for state in states {
            let coordinated = state
                .coordinated_layout
                .as_ref()
                .expect("phase 9 should set coordinated layout");
            assert!(approx_eq_within(
                coordinated.padding_inner_px,
                expected_padding,
                0.01
            ));
            assert_eq!(coordinated.n, expected_n);
        }

        Ok(())
    }

    #[tokio::test]
    async fn phase10_retargets_child_scales_when_parent_cross_size_changes()
    -> Result<(), AvengerChartError> {
        let (mut measurement, _) = nested_fixture().await?;

        let (target_cross_size, old_child_width) = {
            let root = facet_band_mut(&mut measurement)
                .expect("fixture should produce root facet-band measurement");
            assert_eq!(root.axis, FacetAxis::Column);
            root.subplot_cross_size += 40.0;
            let old_child_width = root
                .child_measurements_iter()
                .next()
                .expect("fixture should have child measurements")
                .plot_area_width;
            (root.subplot_cross_size, old_child_width)
        };

        run_phase10_scale_retarget_and_adjustments(&mut measurement);

        let root = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement");
        let first_child = root
            .child_measurements_iter()
            .next()
            .expect("fixture should have child measurements");

        assert!(approx_eq_within(
            first_child.plot_area_width,
            target_cross_size,
            0.01
        ));
        assert!(
            (first_child.plot_area_width - old_child_width).abs() > 0.01,
            "phase 10 should retarget child plot width when parent cross size changes"
        );

        if let Some(scale_with_spec) = first_child.scales.get(root.axis.scale_name()) {
            if let Ok((start, end)) = scale_with_spec.configured().numeric_interval_range() {
                let span = (end - start).abs();
                assert!(approx_eq_within(span, target_cross_size, 2.0));
            }
        }

        Ok(())
    }
}
