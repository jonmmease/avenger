//! Facet-specific coordination keys and grouping helpers.
//!
//! This module provides stable keys for grouping "equivalent" facet nodes across
//! branches during coordination passes (overflow/layout/domain distribution).
//!
//! Phases 7-10 follow an IR + sidecar model:
//! - immutable IR artifacts are built in `coordination_ir`,
//! - runtime side effects are applied through bounded executors in `coordination_sidecar`.

use std::collections::HashSet;

use tracing::{debug, trace};

use crate::{
    error::AvengerChartError,
    facet::{
        coordination_ir::{
            CoordApplyIntent, CoordApplyTrace, CoordGroupDistribution, CoordGroupNodeSnapshot,
            CoordGroupSnapshot, CoordNodeKey, CoordReconcileDistribution,
            CoordReconcileNodeSnapshot, CoordReconcileSnapshot, CoordRetargetIntent,
            CoordRetargetTrace, CoordinationRunArtifacts, build_phase7_ir, build_phase9_ir,
        },
        coordination_sidecar::{
            apply_phase7_distribution, apply_phase9_distribution, derive_phase8, derive_phase10,
            run_phase8_apply_coordinated_overflow_and_remeasure_with_trace,
            run_phase10_scale_retarget_and_adjustments_with_trace, visit_facet_bands_with_node_id,
        },
    },
    plot::compiled::ComponentsMeasurement,
    render::EvaluationContext,
};

#[cfg(test)]
use crate::facet::coord::FacetBandCoordMeasurement;
#[cfg(test)]
use crate::facet::coordination_sidecar::visit_facet_bands_with_node_id_mut;

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

#[cfg(test)]
fn facet_band_ref(measurement: &ComponentsMeasurement) -> Option<&FacetBandCoordMeasurement> {
    measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
}

#[cfg(test)]
fn facet_band_mut(
    measurement: &mut ComponentsMeasurement,
) -> Option<&mut FacetBandCoordMeasurement> {
    measurement
        .coord_measurement
        .as_any_mut()
        .downcast_mut::<FacetBandCoordMeasurement>()
}

#[cfg(test)]
fn visit_facet_bands<F>(measurement: &ComponentsMeasurement, depth: usize, visit: &mut F)
where
    F: FnMut(usize, &FacetBandCoordMeasurement),
{
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id(
        measurement,
        depth,
        &mut node_path,
        &mut |_, d, facet_band| {
            visit(d, facet_band);
        },
    );
}

#[cfg(test)]
fn visit_facet_bands_mut<F>(measurement: &mut ComponentsMeasurement, depth: usize, visit: &mut F)
where
    F: FnMut(usize, &mut FacetBandCoordMeasurement),
{
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id_mut(
        measurement,
        depth,
        &mut node_path,
        &mut |_, d, facet_band| {
            visit(d, facet_band);
        },
    );
}

/// Coordinate overflow, layout, and domains across the measurement tree.
///
/// This is the facet-specific coordination entrypoint invoked from `coords.rs`.
pub async fn coordinate_facet_measurement_tree(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<(), AvengerChartError> {
    let artifacts = coordinate_facet_measurement_tree_with_artifacts(measurement, eval_ctx).await?;
    trace!(
        phase7_nodes = artifacts.phase7.snapshot.nodes.len(),
        phase8_nodes = artifacts.phase8.node_results.len(),
        phase9_nodes = artifacts.phase9.snapshot.nodes.len(),
        phase10_nodes = artifacts.phase10.node_results.len(),
        "coordinate_facet_measurement_tree complete"
    );
    Ok(())
}

pub(crate) async fn coordinate_facet_measurement_tree_with_artifacts(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<CoordinationRunArtifacts, AvengerChartError> {
    // Phase 7: build immutable aggregate/distribution IR, then apply sidecar patches.
    let phase7 = build_phase7_ir(collect_phase7_snapshot(measurement));
    debug_assert_phase7_distribution_coverage(&phase7);
    debug!(
        overflow_groups = phase7.aggregates.merged_overflow_by_key.len(),
        layout_groups = phase7.aggregates.merged_layout_by_key.len(),
        domain_groups = phase7.aggregates.unified_domain_extents.len(),
        "coordinate_facet_measurement_tree phase 7 global aggregate + distribution"
    );
    apply_phase7_distribution(measurement, &phase7);
    debug!("coordinate_facet_measurement_tree phase 7 complete");

    // Phase 8: coordinated apply + selective remeasure (sidecar mutations + immutable trace).
    let phase8_derivation = derive_phase8(measurement);
    debug_assert_phase8_derivation_node_coverage(measurement, &phase8_derivation);
    let phase8 = run_phase8_apply_coordinated_overflow_and_remeasure_with_trace(
        measurement,
        eval_ctx,
        &phase8_derivation,
    )
    .await?;
    debug_assert_phase8_trace_alignment(&phase8_derivation, &phase8);
    let parent_cross_propagations = phase8
        .node_results
        .iter()
        .filter(|result| result.parent_cross_size_propagated)
        .count();
    let cross_size_changes = phase8
        .node_results
        .iter()
        .filter(|result| (result.subplot_cross_size_after - result.subplot_cross_size_before).abs() > 0.01)
        .count();
    debug!(
        parent_cross_propagations,
        cross_size_changes,
        remeasured_nodes = phase8
            .node_results
            .iter()
            .filter(|result| result.remeasure_triggered)
            .count(),
        "coordinate_facet_measurement_tree phase 8 complete"
    );

    // Phase 9: build immutable post-remeasure reconciliation IR, then apply patches.
    let phase9 = build_phase9_ir(collect_phase9_snapshot(measurement));
    debug_assert_phase9_distribution_coverage(&phase9);
    debug!(
        overflow_groups = phase9.aggregates.merged_overflow_by_key.len(),
        layout_groups = phase9.aggregates.merged_layout_by_key.len(),
        "coordinate_facet_measurement_tree phase 9 post-remeasure reconciliation"
    );
    apply_phase9_distribution(measurement, &phase9);
    debug!("coordinate_facet_measurement_tree phase 9 complete");

    // Phase 10: scale retarget + adjustment propagation (sidecar mutations + immutable trace).
    let phase10_derivation = derive_phase10(measurement);
    debug_assert_phase10_derivation_node_coverage(measurement, &phase10_derivation);
    let phase10 =
        run_phase10_scale_retarget_and_adjustments_with_trace(measurement, &phase10_derivation);
    debug_assert_phase10_trace_alignment(&phase10_derivation, &phase10);
    debug!(
        scale_range_retargets = phase10
            .node_results
            .iter()
            .map(|result| result.scale_range_retarget_count)
            .sum::<usize>(),
        plot_area_adjustments = phase10
            .node_results
            .iter()
            .map(|result| result.child_plot_area_adjustments_count)
            .sum::<usize>(),
        "coordinate_facet_measurement_tree phase 10 complete"
    );

    Ok(CoordinationRunArtifacts {
        phase7,
        phase8,
        phase9,
        phase10,
    })
}

fn collect_phase7_snapshot(measurement: &ComponentsMeasurement) -> CoordGroupSnapshot {
    let mut nodes = Vec::new();
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, depth, facet_band| {
            let mut domain_infos = Vec::new();
            facet_band.collect_cell_domain_infos(&mut domain_infos);
            nodes.push(CoordGroupNodeSnapshot {
                node_id: node_id.clone(),
                key: facet_band.coordination_group_key_for_depth(depth),
                local_overflow: facet_band.local_overflow_value(),
                local_layout: facet_band.local_layout_value(),
                domain_infos,
            });
        },
    );
    CoordGroupSnapshot { nodes }
}

fn collect_phase9_snapshot(measurement: &ComponentsMeasurement) -> CoordReconcileSnapshot {
    let mut nodes = Vec::new();
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, depth, facet_band| {
            nodes.push(CoordReconcileNodeSnapshot {
                node_id: node_id.clone(),
                key: facet_band.coordination_group_key_for_depth(depth),
                local_overflow: facet_band.local_overflow_value(),
                local_layout: facet_band.local_layout_value(),
            });
        },
    );
    CoordReconcileSnapshot { nodes }
}

fn measurement_node_ids(measurement: &ComponentsMeasurement) -> Vec<CoordNodeKey> {
    let mut node_ids = Vec::new();
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, _depth, _facet_band| {
            node_ids.push(node_id.clone());
        },
    );
    node_ids
}

fn debug_assert_phase7_distribution_coverage(phase7: &CoordGroupDistribution) {
    let snapshot_nodes: HashSet<CoordNodeKey> = phase7
        .snapshot
        .nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect();
    let layout_patch_nodes: HashSet<CoordNodeKey> = phase7
        .distribution
        .layout_patches_by_node
        .keys()
        .cloned()
        .collect();

    debug_assert_eq!(
        layout_patch_nodes, snapshot_nodes,
        "phase 7 layout patch coverage must match snapshot nodes"
    );

    if !phase7.aggregates.unified_domain_extents.is_empty() {
        debug_assert_eq!(
            phase7.distribution.domain_target_nodes, snapshot_nodes,
            "phase 7 domain target coverage must match snapshot nodes when domain aggregates exist"
        );
    }
}

fn debug_assert_phase9_distribution_coverage(phase9: &CoordReconcileDistribution) {
    let snapshot_nodes: HashSet<CoordNodeKey> = phase9
        .snapshot
        .nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect();
    let layout_patch_nodes: HashSet<CoordNodeKey> = phase9
        .distribution
        .layout_patches_by_node
        .keys()
        .cloned()
        .collect();

    debug_assert_eq!(
        layout_patch_nodes, snapshot_nodes,
        "phase 9 layout patch coverage must match snapshot nodes"
    );
}

fn debug_assert_phase8_derivation_node_coverage(
    measurement: &ComponentsMeasurement,
    derivation: &CoordApplyIntent,
) {
    let expected_ids: HashSet<CoordNodeKey> =
        measurement_node_ids(measurement).into_iter().collect();
    let actual_ids: HashSet<CoordNodeKey> = derivation
        .node_derivations
        .iter()
        .map(|node| node.node_id.clone())
        .collect();

    debug_assert_eq!(
        actual_ids, expected_ids,
        "phase 8 derivation nodes must match current measurement tree node coverage"
    );
}

fn debug_assert_phase10_derivation_node_coverage(
    measurement: &ComponentsMeasurement,
    derivation: &CoordRetargetIntent,
) {
    let expected_ids: HashSet<CoordNodeKey> =
        measurement_node_ids(measurement).into_iter().collect();
    let actual_ids: HashSet<CoordNodeKey> = derivation
        .node_derivations
        .iter()
        .map(|node| node.node_id.clone())
        .collect();

    debug_assert_eq!(
        actual_ids, expected_ids,
        "phase 10 derivation nodes must match current measurement tree node coverage"
    );
}

fn debug_assert_phase8_trace_alignment(derivation: &CoordApplyIntent, trace: &CoordApplyTrace) {
    let derived_ids: Vec<CoordNodeKey> = derivation
        .node_derivations
        .iter()
        .map(|node| node.node_id.clone())
        .collect();
    let trace_ids: Vec<CoordNodeKey> = trace
        .node_results
        .iter()
        .map(|node| node.node_id.clone())
        .collect();

    debug_assert_eq!(
        trace_ids, derived_ids,
        "phase 8 execution trace nodes must match derivation nodes in deterministic order"
    );

    for (derived, trace_result) in derivation
        .node_derivations
        .iter()
        .zip(trace.node_results.iter())
    {
        debug_assert_eq!(trace_result.node_id, derived.node_id);
        debug_assert_eq!(trace_result.axis, derived.axis);
        debug_assert_eq!(
            trace_result.derived_has_legend_overflow,
            derived.has_legend_overflow
        );
        debug_assert_eq!(
            trace_result.derived_has_coordinated_extents,
            derived.has_coordinated_extents
        );
        debug_assert_eq!(
            trace_result.derived_remeasure_required,
            derived.remeasure_triggered
        );
        debug_assert_eq!(
            trace_result.derived_axis_owner_ignore_empty_cells,
            derived.apply_plan.axis_owner_ignore_empty_cells
        );
        debug_assert_eq!(
            trace_result.derived_adjusted_main_size,
            derived.apply_plan.adjusted_main_size
        );
        debug_assert_eq!(
            trace_result.remeasure_triggered,
            derived.remeasure_triggered
        );
        debug_assert_eq!(
            derived.remeasure_triggered,
            derived.remeasure_plan.is_some(),
            "phase 8 derivation must carry per-cell remeasure plan iff remeasure is required"
        );
        debug_assert_eq!(trace_result.derived_child_count, derived.child_count);
        if let Some(remeasure_plan) = derived.remeasure_plan.as_ref() {
            let derived_cell_count = remeasure_plan.cell_intents.len();
            let derived_non_empty_count = remeasure_plan
                .cell_intents
                .iter()
                .filter(|intent| intent.has_data_rows)
                .count();
            let derived_with_coordinated_extents_count = remeasure_plan
                .cell_intents
                .iter()
                .filter(|intent| intent.use_coordinated_extents)
                .count();
            debug_assert!(trace_result.remeasure_triggered);
            debug_assert_eq!(trace_result.remeasured_cell_count, derived_cell_count);
            debug_assert_eq!(
                trace_result.remeasured_non_empty_cell_count,
                derived_non_empty_count
            );
            debug_assert_eq!(
                trace_result.remeasured_with_coordinated_extents_count,
                derived_with_coordinated_extents_count
            );
        } else {
            debug_assert!(!trace_result.remeasure_triggered);
            debug_assert_eq!(trace_result.remeasured_cell_count, 0);
            debug_assert_eq!(trace_result.remeasured_non_empty_cell_count, 0);
            debug_assert_eq!(trace_result.remeasured_with_coordinated_extents_count, 0);
        }
        debug_assert!(
            trace_result.remeasured_non_empty_cell_count <= trace_result.remeasured_cell_count
        );
        debug_assert!(
            trace_result.remeasured_with_coordinated_extents_count
                <= trace_result.remeasured_cell_count
        );
    }
}

fn debug_assert_phase10_trace_alignment(
    derivation: &CoordRetargetIntent,
    trace: &CoordRetargetTrace,
) {
    let derived_ids: Vec<CoordNodeKey> = derivation
        .node_derivations
        .iter()
        .map(|node| node.node_id.clone())
        .collect();
    let trace_ids: Vec<CoordNodeKey> = trace
        .node_results
        .iter()
        .map(|node| node.node_id.clone())
        .collect();

    debug_assert_eq!(
        trace_ids, derived_ids,
        "phase 10 execution trace nodes must match derivation nodes in deterministic order"
    );

    for (derived, trace_result) in derivation
        .node_derivations
        .iter()
        .zip(trace.node_results.iter())
    {
        debug_assert_eq!(trace_result.node_id, derived.node_id);
        debug_assert_eq!(trace_result.axis, derived.axis);
        debug_assert_eq!(
            trace_result.derived_parent_cross_size_target,
            derived.parent_cross_size_target
        );
        debug_assert_eq!(trace_result.derived_child_count, derived.child_count);
        debug_assert_eq!(
            derived.child_intents.len(),
            derived.child_count,
            "phase 10 child intent count must match derived child count"
        );
        for (idx, child_intent) in derived.child_intents.iter().enumerate() {
            debug_assert_eq!(
                child_intent.child_index, idx,
                "phase 10 child intents must preserve deterministic child index order"
            );
        }
        debug_assert_eq!(
            trace_result.derived_child_intent_count,
            derived.child_intents.len()
        );
        debug_assert_eq!(
            trace_result.derived_expected_plot_area_adjustments_count,
            derived.expected_plot_area_adjustments_count
        );
        debug_assert_eq!(
            trace_result.child_plot_area_adjustments_count,
            derived.expected_plot_area_adjustments_count
        );
    }
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
        scales::domain_extent::DomainExtent,
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

    fn build_col_col_nested_plot(df: DataFrame) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(640, 420)
            .mark(
                Facet::new()
                    .col_with(col("outer_group"), |c| c.facet(|f| f.title("Outer Group")))
                    .subplot(
                        Plot::<FacetColumn>::new().mark(
                            Facet::new()
                                .col_with(col("inner_group"), |c| {
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

    async fn col_col_fixture()
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

        let plot = build_col_col_nested_plot(data_df);
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

        let phase7 = build_phase7_ir(collect_phase7_snapshot(&measurement));
        apply_phase7_distribution(&mut measurement, &phase7);

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

        let phase8_derivation = derive_phase8(&measurement);
        let _phase8 = run_phase8_apply_coordinated_overflow_and_remeasure_with_trace(
            &mut measurement,
            &eval_ctx,
            &phase8_derivation,
        )
        .await?;

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

        let phase7 = build_phase7_ir(collect_phase7_snapshot(&measurement));
        apply_phase7_distribution(&mut measurement, &phase7);
        let phase8_derivation = derive_phase8(&measurement);
        let _phase8 = run_phase8_apply_coordinated_overflow_and_remeasure_with_trace(
            &mut measurement,
            &eval_ctx,
            &phase8_derivation,
        )
        .await?;

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

        let phase9 = build_phase9_ir(collect_phase9_snapshot(&measurement));
        apply_phase9_distribution(&mut measurement, &phase9);

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

        let phase10_derivation = derive_phase10(&measurement);
        let _phase10 = run_phase10_scale_retarget_and_adjustments_with_trace(
            &mut measurement,
            &phase10_derivation,
        );

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

    #[tokio::test]
    async fn phase7_ir_contains_expected_group_counts_and_node_targets()
    -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let phase7 = build_phase7_ir(collect_phase7_snapshot(&measurement));
        assert!(!phase7.snapshot.nodes.is_empty());
        assert!(!phase7.aggregates.merged_layout_by_key.is_empty());
        assert_eq!(
            phase7.distribution.layout_patches_by_node.len(),
            phase7.snapshot.nodes.len()
        );
        assert!(phase7.distribution.overflow_patches_by_node.len() <= phase7.snapshot.nodes.len());
        if !phase7.aggregates.unified_domain_extents.is_empty() {
            assert_eq!(
                phase7.distribution.domain_target_nodes.len(),
                phase7.snapshot.nodes.len()
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn phase8_ir_trace_records_remeasure_and_parent_cross_propagation()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = col_col_fixture().await?;
        let phase7 = build_phase7_ir(collect_phase7_snapshot(&measurement));
        apply_phase7_distribution(&mut measurement, &phase7);

        {
            let root = facet_band_mut(&mut measurement)
                .expect("fixture should produce root facet-band measurement");
            let first_cell = root
                .cells
                .first_mut()
                .expect("fixture should include at least one facet cell");
            first_cell
                .coordinated_domain_extents
                .insert("x".to_string(), DomainExtent::numeric(0.0, 10.0));
        }

        let phase8_derivation = derive_phase8(&measurement);
        let phase8 = run_phase8_apply_coordinated_overflow_and_remeasure_with_trace(
            &mut measurement,
            &eval_ctx,
            &phase8_derivation,
        )
        .await?;
        assert!(!phase8.node_results.is_empty());
        let root_result = phase8
            .node_results
            .iter()
            .find(|result| result.node_id.path.is_empty())
            .expect("phase 8 should include a root node trace");
        let root_derivation = phase8_derivation
            .node_derivations
            .iter()
            .find(|node| node.node_id.path.is_empty())
            .expect("phase 8 derivation should include a root node");
        assert!(root_result.parent_cross_size_propagated);
        assert!(root_result.remeasure_triggered);
        assert!(root_result.derived_has_coordinated_extents);
        assert!(root_result.remeasured_cell_count > 0);
        assert!(root_result.remeasured_non_empty_cell_count <= root_result.remeasured_cell_count);
        assert!(
            root_result.remeasured_with_coordinated_extents_count
                <= root_result.remeasured_cell_count
        );
        let root_remeasure_plan = root_derivation
            .remeasure_plan
            .as_ref()
            .expect("root derivation should include remeasure plan");
        assert_eq!(
            root_result.remeasured_cell_count,
            root_remeasure_plan.cell_intents.len()
        );
        assert_eq!(
            root_result.remeasured_with_coordinated_extents_count,
            root_remeasure_plan
                .cell_intents
                .iter()
                .filter(|intent| intent.use_coordinated_extents)
                .count()
        );
        Ok(())
    }

    #[tokio::test]
    async fn phase9_ir_distribution_stabilizes_layout_values() -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        let phase7 = build_phase7_ir(collect_phase7_snapshot(&measurement));
        apply_phase7_distribution(&mut measurement, &phase7);
        let phase8_derivation = derive_phase8(&measurement);
        let _phase8 = run_phase8_apply_coordinated_overflow_and_remeasure_with_trace(
            &mut measurement,
            &eval_ctx,
            &phase8_derivation,
        )
        .await?;

        let phase9 = build_phase9_ir(collect_phase9_snapshot(&measurement));
        assert!(!phase9.distribution.layout_patches_by_node.is_empty());
        apply_phase9_distribution(&mut measurement, &phase9);

        let states = depth1_states(&measurement);
        assert!(states.len() >= 2);
        let first_layout = states
            .first()
            .and_then(|state| state.coordinated_layout.as_ref())
            .expect("phase 9 should set coordinated layout on depth-1 states")
            .clone();
        for state in states.iter().skip(1) {
            let coordinated = state
                .coordinated_layout
                .as_ref()
                .expect("phase 9 should set coordinated layout");
            assert_layout_close(coordinated, &first_layout);
        }
        Ok(())
    }

    #[tokio::test]
    async fn phase10_ir_trace_records_plot_resize_and_scale_retarget_events()
    -> Result<(), AvengerChartError> {
        let (mut measurement, _) = nested_fixture().await?;

        {
            let root = facet_band_mut(&mut measurement)
                .expect("fixture should produce root facet-band measurement");
            root.subplot_cross_size += 40.0;
        }

        let phase10_derivation = derive_phase10(&measurement);
        let phase10 = run_phase10_scale_retarget_and_adjustments_with_trace(
            &mut measurement,
            &phase10_derivation,
        );
        assert!(!phase10.node_results.is_empty());
        let root_result = phase10
            .node_results
            .iter()
            .find(|result| result.node_id.path.is_empty())
            .expect("phase 10 should include a root node trace");
        let root_derivation = phase10_derivation
            .node_derivations
            .iter()
            .find(|node| node.node_id.path.is_empty())
            .expect("phase 10 derivation should include a root node");
        assert!(root_result.child_plot_area_adjustments_count > 0);
        assert!(root_result.scale_range_retarget_count > 0);
        assert_eq!(
            root_result.derived_child_intent_count,
            root_derivation.child_intents.len()
        );
        assert_eq!(
            root_result.derived_expected_plot_area_adjustments_count,
            root_derivation.expected_plot_area_adjustments_count
        );
        assert_eq!(
            root_result.child_plot_area_adjustments_count,
            root_derivation.expected_plot_area_adjustments_count
        );
        assert_eq!(root_result.derived_child_count, root_derivation.child_count);
        Ok(())
    }

    #[tokio::test]
    async fn phase10_derivation_contains_ordered_child_intents() -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let derivation = derive_phase10(&measurement);
        assert!(!derivation.node_derivations.is_empty());
        for node in &derivation.node_derivations {
            assert_eq!(node.child_intents.len(), node.child_count);
            let expected_adjustments = node
                .child_intents
                .iter()
                .filter(|intent| intent.adjust_plot_area)
                .count();
            assert_eq!(
                node.expected_plot_area_adjustments_count,
                expected_adjustments
            );
            for (idx, intent) in node.child_intents.iter().enumerate() {
                assert_eq!(intent.child_index, idx);
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn phase10_trace_matches_derived_child_intent_adjustment_counts()
    -> Result<(), AvengerChartError> {
        let (mut measurement, _) = nested_fixture().await?;
        {
            let root = facet_band_mut(&mut measurement)
                .expect("fixture should produce root facet-band measurement");
            root.subplot_cross_size += 40.0;
        }

        let derivation = derive_phase10(&measurement);
        let phase10 =
            run_phase10_scale_retarget_and_adjustments_with_trace(&mut measurement, &derivation);

        for (derived, trace_result) in derivation
            .node_derivations
            .iter()
            .zip(phase10.node_results.iter())
        {
            assert_eq!(trace_result.node_id, derived.node_id);
            assert_eq!(
                trace_result.derived_child_intent_count,
                derived.child_intents.len()
            );
            assert_eq!(
                trace_result.derived_expected_plot_area_adjustments_count,
                derived.expected_plot_area_adjustments_count
            );
            assert_eq!(
                trace_result.child_plot_area_adjustments_count,
                derived.expected_plot_area_adjustments_count
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn phase10_child_intents_encode_axis_target_cross_sizes() -> Result<(), AvengerChartError>
    {
        let (measurement, _) = nested_fixture().await?;
        let derivation = derive_phase10(&measurement);
        for node in &derivation.node_derivations {
            match (node.axis, node.parent_cross_size_target) {
                (FacetAxis::Column, Some(target_cross_size)) => {
                    for intent in &node.child_intents {
                        assert!(
                            intent.target_plot_area_height.is_none(),
                            "column intent should not target child plot height"
                        );
                        if intent.adjust_plot_area {
                            assert_eq!(intent.target_plot_area_width, Some(target_cross_size));
                        } else {
                            assert!(intent.target_plot_area_width.is_none());
                        }
                        if intent.update_band_range {
                            assert_eq!(intent.target_band_range_end, Some(target_cross_size));
                        } else {
                            assert!(intent.target_band_range_end.is_none());
                        }
                    }
                }
                (FacetAxis::Row, Some(target_cross_size)) => {
                    for intent in &node.child_intents {
                        assert!(
                            intent.target_plot_area_width.is_none(),
                            "row intent should not target child plot width"
                        );
                        if intent.adjust_plot_area {
                            assert_eq!(intent.target_plot_area_height, Some(target_cross_size));
                        } else {
                            assert!(intent.target_plot_area_height.is_none());
                        }
                        if intent.update_band_range {
                            assert_eq!(intent.target_band_range_end, Some(target_cross_size));
                        } else {
                            assert!(intent.target_band_range_end.is_none());
                        }
                    }
                }
                (_, None) => {
                    for intent in &node.child_intents {
                        assert!(intent.target_plot_area_width.is_none());
                        assert!(intent.target_plot_area_height.is_none());
                        assert!(intent.target_band_range_end.is_none());
                        assert!(!intent.adjust_plot_area);
                        assert!(!intent.update_band_range);
                    }
                }
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn phase8_derivation_is_read_only_and_node_complete() -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let before_node_ids = measurement_node_ids(&measurement);
        let before_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;

        let derivation = derive_phase8(&measurement);

        let after_node_ids = measurement_node_ids(&measurement);
        let after_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;

        assert_eq!(before_node_ids, after_node_ids);
        assert_eq!(before_cross_size, after_cross_size);
        assert_eq!(
            derivation
                .node_derivations
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<std::collections::HashSet<_>>(),
            before_node_ids
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
        );
        Ok(())
    }

    #[tokio::test]
    async fn phase8_derivation_contains_apply_plan_for_each_node() -> Result<(), AvengerChartError>
    {
        let (measurement, _) = nested_fixture().await?;
        let derivation = derive_phase8(&measurement);
        assert!(!derivation.node_derivations.is_empty());
        for node in &derivation.node_derivations {
            assert_eq!(node.axis, node.apply_plan.axis);
            assert_eq!(node.remeasure_triggered, node.apply_plan.remeasure_required);
            assert_eq!(node.remeasure_triggered, node.remeasure_plan.is_some());
            if node.has_legend_overflow {
                assert!(node.apply_plan.legend_slab_applied > 0.0);
            }
            if let Some(remeasure_plan) = node.remeasure_plan.as_ref() {
                assert_eq!(remeasure_plan.request.axis, node.axis);
                assert_eq!(
                    remeasure_plan.request.axis_owner_ignore_empty_cells,
                    node.apply_plan.axis_owner_ignore_empty_cells
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn phase8_executor_applies_derivation_with_behavior_parity()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        {
            let root = facet_band_mut(&mut measurement)
                .expect("fixture should produce root facet-band measurement");
            let mut forced_layout = root.local_layout.clone();
            forced_layout.n = forced_layout.n.saturating_add(8);
            forced_layout.padding_inner_px += 12.0;
            root.set_coordinated_layout_value(forced_layout);
            root.cells[0]
                .coordinated_domain_extents
                .insert("x".to_string(), DomainExtent::numeric(0.0, 15.0));
        }

        let derivation = derive_phase8(&measurement);
        let phase8 = run_phase8_apply_coordinated_overflow_and_remeasure_with_trace(
            &mut measurement,
            &eval_ctx,
            &derivation,
        )
        .await?;

        debug_assert_phase8_trace_alignment(&derivation, &phase8);
        let root_result = phase8
            .node_results
            .iter()
            .find(|result| result.node_id.path.is_empty())
            .expect("phase 8 should include a root node trace");
        let root_derivation = derivation
            .node_derivations
            .iter()
            .find(|node| node.node_id.path.is_empty())
            .expect("phase 8 derivation should include a root node");
        assert_eq!(root_result.axis, FacetAxis::Column);
        assert!(root_result.subplot_cross_size_after > 0.0);
        assert!(root_result.derived_has_coordinated_extents);
        assert!(root_result.derived_remeasure_required);
        assert!(root_result.remeasure_triggered);
        assert!(root_result.remeasured_cell_count > 0);
        assert!(root_result.remeasured_non_empty_cell_count <= root_result.remeasured_cell_count);
        assert!(
            root_result.remeasured_with_coordinated_extents_count
                <= root_result.remeasured_cell_count
        );
        let root_remeasure_plan = root_derivation
            .remeasure_plan
            .as_ref()
            .expect("root derivation should include remeasure plan when remeasure is required");
        assert_eq!(
            root_result.remeasured_cell_count,
            root_remeasure_plan.cell_intents.len()
        );
        assert_eq!(
            root_result.remeasured_non_empty_cell_count,
            root_remeasure_plan
                .cell_intents
                .iter()
                .filter(|intent| intent.has_data_rows)
                .count()
        );
        assert_eq!(
            root_result.remeasured_with_coordinated_extents_count,
            root_remeasure_plan
                .cell_intents
                .iter()
                .filter(|intent| intent.use_coordinated_extents)
                .count()
        );
        Ok(())
    }

    #[tokio::test]
    async fn phase8_node_identity_stable_between_derivation_and_execution()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        let phase7 = build_phase7_ir(collect_phase7_snapshot(&measurement));
        apply_phase7_distribution(&mut measurement, &phase7);

        let phase8_derivation = derive_phase8(&measurement);
        let phase8 = run_phase8_apply_coordinated_overflow_and_remeasure_with_trace(
            &mut measurement,
            &eval_ctx,
            &phase8_derivation,
        )
        .await?;

        assert_eq!(
            phase8
                .node_results
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>(),
            phase8_derivation
                .node_derivations
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>()
        );
        Ok(())
    }

    #[tokio::test]
    async fn phase10_derivation_is_read_only_and_node_complete() -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let before_node_ids = measurement_node_ids(&measurement);
        let before_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;

        let derivation = derive_phase10(&measurement);

        let after_node_ids = measurement_node_ids(&measurement);
        let after_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;
        assert_eq!(before_node_ids, after_node_ids);
        assert_eq!(before_cross_size, after_cross_size);
        assert_eq!(
            derivation
                .node_derivations
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<std::collections::HashSet<_>>(),
            before_node_ids
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
        );
        Ok(())
    }

    #[tokio::test]
    async fn phase10_executor_applies_retarget_with_behavior_parity()
    -> Result<(), AvengerChartError> {
        let (mut measurement, _) = nested_fixture().await?;
        {
            let root = facet_band_mut(&mut measurement)
                .expect("fixture should produce root facet-band measurement");
            root.subplot_cross_size += 40.0;
        }

        let derivation = derive_phase10(&measurement);
        let phase10 =
            run_phase10_scale_retarget_and_adjustments_with_trace(&mut measurement, &derivation);
        debug_assert_phase10_trace_alignment(&derivation, &phase10);

        let root_result = phase10
            .node_results
            .iter()
            .find(|result| result.node_id.path.is_empty())
            .expect("phase 10 should include a root node trace");
        assert!(root_result.child_plot_area_adjustments_count > 0);
        assert!(root_result.derived_child_intent_count > 0);
        assert_eq!(
            root_result.child_plot_area_adjustments_count,
            root_result.derived_expected_plot_area_adjustments_count
        );
        Ok(())
    }

    #[tokio::test]
    async fn phase7_distribution_patch_coverage_matches_snapshot() -> Result<(), AvengerChartError>
    {
        let (measurement, _) = nested_fixture().await?;
        let phase7 = build_phase7_ir(collect_phase7_snapshot(&measurement));
        debug_assert_phase7_distribution_coverage(&phase7);
        let snapshot_nodes: std::collections::HashSet<CoordNodeKey> = phase7
            .snapshot
            .nodes
            .iter()
            .map(|node| node.node_id.clone())
            .collect();
        let layout_patch_nodes: std::collections::HashSet<CoordNodeKey> = phase7
            .distribution
            .layout_patches_by_node
            .keys()
            .cloned()
            .collect();
        assert_eq!(layout_patch_nodes, snapshot_nodes);
        Ok(())
    }

    #[tokio::test]
    async fn phase9_distribution_patch_coverage_matches_snapshot() -> Result<(), AvengerChartError>
    {
        let (measurement, _) = nested_fixture().await?;
        let phase9 = build_phase9_ir(collect_phase9_snapshot(&measurement));
        debug_assert_phase9_distribution_coverage(&phase9);
        let snapshot_nodes: std::collections::HashSet<CoordNodeKey> = phase9
            .snapshot
            .nodes
            .iter()
            .map(|node| node.node_id.clone())
            .collect();
        let layout_patch_nodes: std::collections::HashSet<CoordNodeKey> = phase9
            .distribution
            .layout_patches_by_node
            .keys()
            .cloned()
            .collect();
        assert_eq!(layout_patch_nodes, snapshot_nodes);
        Ok(())
    }

    #[tokio::test]
    async fn phase8_and_phase10_node_identity_stable_across_derivation_and_execution()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        let phase7 = build_phase7_ir(collect_phase7_snapshot(&measurement));
        apply_phase7_distribution(&mut measurement, &phase7);

        let phase8_derivation = derive_phase8(&measurement);
        let phase8 = run_phase8_apply_coordinated_overflow_and_remeasure_with_trace(
            &mut measurement,
            &eval_ctx,
            &phase8_derivation,
        )
        .await?;
        assert_eq!(
            phase8
                .node_results
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>(),
            phase8_derivation
                .node_derivations
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>()
        );

        let phase9 = build_phase9_ir(collect_phase9_snapshot(&measurement));
        apply_phase9_distribution(&mut measurement, &phase9);

        let phase10_derivation = derive_phase10(&measurement);
        let phase10 = run_phase10_scale_retarget_and_adjustments_with_trace(
            &mut measurement,
            &phase10_derivation,
        );
        assert_eq!(
            phase10
                .node_results
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>(),
            phase10_derivation
                .node_derivations
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>()
        );
        for (derived, trace_result) in phase10
            .node_results
            .iter()
            .zip(phase10_derivation.node_derivations.iter())
        {
            assert_eq!(derived.node_id, trace_result.node_id);
            assert_eq!(
                derived.derived_child_intent_count,
                trace_result.child_intents.len()
            );
        }
        Ok(())
    }
}
