//! Facet coordination pipeline for AG-style collection and inherited propagation.
//!
//! Coordination runs after local synthesis and applies four ordered steps:
//! 1. Collection round A (aggregate + distribute),
//! 2. inherited apply with selective resynthesis,
//! 3. recollection round stabilization,
//! 4. inherited propagation (retarget + adjustment).
//!
//! Immutable artifacts are built in `coordination_attributes`, and bounded side effects are
//! applied through executors in `coordination_sidecar`.
//!
//! Reference terminology:
//! - JastAdd concept overview: https://jastadd.cs.lth.se/web/documentation/concept-overview.php
//! - JastAdd reference manual: https://jastadd.cs.lth.se/web/documentation/reference-manual.php
//! - Knuth attribute grammars: https://doi.org/10.1007/BF01692511

use std::collections::HashSet;

use crate::{
    error::AvengerChartError,
    facet::{
        coordination_attributes::{
            CollectionRoundA, CollectionRoundNodeSnapshot, CollectionRoundSnapshot, CoordNodeKey,
            InheritedApplyIntent, InheritedApplyTrace, InheritedPropagationIntent,
            InheritedPropagationTrace, RecollectionRound, RecollectionRoundNodeSnapshot,
            RecollectionRoundSnapshot,
        },
        coordination_sidecar::visit_facet_bands_with_node_id,
    },
    plot::compiled::ComponentsMeasurement,
    render::EvaluationContext,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FacetCircularEpoch {
    CollectionA,
    InheritedApply,
    Recollection,
    InheritedPropagation,
}

#[cfg(test)]
use crate::facet::coord::{
    FacetBandCoordMeasurement, facet_band_canvas_mut as facet_band_canvas_mut_from_coord,
    facet_band_canvas_ref as facet_band_canvas_ref_from_coord,
};
#[cfg(test)]
use crate::facet::coordination_attributes::{build_collection_round_a, build_recollection_round};
#[cfg(test)]
use crate::facet::coordination_sidecar::{
    apply_collection_round_a, apply_recollection_round, derive_inherited_apply_intent,
    derive_inherited_propagation_intent, run_inherited_apply_with_trace,
    run_inherited_propagation_with_trace, visit_facet_bands_with_node_id_mut,
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

#[cfg(test)]
fn facet_band_ref(measurement: &ComponentsMeasurement) -> Option<&FacetBandCoordMeasurement> {
    facet_band_canvas_ref_from_coord(measurement.coord_measurement.as_ref())
}

#[cfg(test)]
fn facet_band_mut(
    measurement: &mut ComponentsMeasurement,
) -> Option<&mut FacetBandCoordMeasurement> {
    facet_band_canvas_mut_from_coord(measurement.coord_measurement.as_mut())
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
    crate::facet::coordination_canvas_fit::coordinate_facet_measurement_tree_canvas_fit(
        measurement,
        eval_ctx,
    )
    .await
}

pub(crate) fn collect_collection_round_snapshot(
    measurement: &ComponentsMeasurement,
) -> CollectionRoundSnapshot {
    let mut nodes = Vec::new();
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, depth, facet_band| {
            let mut domain_infos = Vec::new();
            facet_band.collect_cell_domain_infos(&mut domain_infos);
            nodes.push(CollectionRoundNodeSnapshot {
                node_id: node_id.clone(),
                key: facet_band.coordination_group_key_for_depth(depth),
                local_overflow: facet_band.local_overflow_value(),
                local_layout: facet_band.local_layout_value(),
                domain_infos,
            });
        },
    );
    CollectionRoundSnapshot { nodes }
}

pub(crate) fn collect_recollection_round_snapshot(
    measurement: &ComponentsMeasurement,
) -> RecollectionRoundSnapshot {
    let mut nodes = Vec::new();
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, depth, facet_band| {
            nodes.push(RecollectionRoundNodeSnapshot {
                node_id: node_id.clone(),
                key: facet_band.coordination_group_key_for_depth(depth),
                local_overflow: facet_band.local_overflow_value(),
                local_layout: facet_band.local_layout_value(),
            });
        },
    );
    RecollectionRoundSnapshot { nodes }
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

pub(crate) fn debug_assert_epoch_transition(
    current: Option<FacetCircularEpoch>,
    next: FacetCircularEpoch,
) {
    let allowed = matches!(
        (current, next),
        (None, FacetCircularEpoch::CollectionA)
            | (
                Some(FacetCircularEpoch::CollectionA),
                FacetCircularEpoch::InheritedApply
            )
            | (
                Some(FacetCircularEpoch::InheritedApply),
                FacetCircularEpoch::Recollection
            )
            | (
                Some(FacetCircularEpoch::Recollection),
                FacetCircularEpoch::InheritedPropagation
            )
    );
    debug_assert!(
        allowed,
        "invalid coordination circular schedule transition: current={current:?}, next={next:?}"
    );
}

pub(crate) fn debug_assert_collection_round_coverage(collection_round_a: &CollectionRoundA) {
    let snapshot_nodes: HashSet<CoordNodeKey> = collection_round_a
        .snapshot
        .nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect();
    let layout_patch_nodes: HashSet<CoordNodeKey> = collection_round_a
        .distribution
        .layout_patches_by_node
        .keys()
        .cloned()
        .collect();

    debug_assert_eq!(
        layout_patch_nodes, snapshot_nodes,
        "collection round A layout patch coverage must match snapshot nodes"
    );

    if !collection_round_a
        .aggregates
        .unified_domain_extents
        .is_empty()
    {
        debug_assert_eq!(
            collection_round_a.distribution.domain_target_nodes, snapshot_nodes,
            "collection round A domain target coverage must match snapshot nodes when domain aggregates exist"
        );
    }
}

pub(crate) fn debug_assert_recollection_round_coverage(recollection_round: &RecollectionRound) {
    let snapshot_nodes: HashSet<CoordNodeKey> = recollection_round
        .snapshot
        .nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect();
    let layout_patch_nodes: HashSet<CoordNodeKey> = recollection_round
        .distribution
        .layout_patches_by_node
        .keys()
        .cloned()
        .collect();

    debug_assert_eq!(
        layout_patch_nodes, snapshot_nodes,
        "recollection round layout patch coverage must match snapshot nodes"
    );
}

pub(crate) fn debug_assert_inherited_apply_derivation_coverage(
    measurement: &ComponentsMeasurement,
    derivation: &InheritedApplyIntent,
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
        "inherited apply derivation nodes must match current measurement tree node coverage"
    );
}

pub(crate) fn debug_assert_inherited_propagation_derivation_coverage(
    measurement: &ComponentsMeasurement,
    derivation: &InheritedPropagationIntent,
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
        "inherited propagation derivation nodes must match current measurement tree node coverage"
    );
}

pub(crate) fn debug_assert_inherited_apply_trace_alignment(
    derivation: &InheritedApplyIntent,
    trace: &InheritedApplyTrace,
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
        "inherited apply execution trace nodes must match derivation nodes in deterministic order"
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
        debug_assert_eq!(trace_result.derived_child_count, derived.child_count);
        debug_assert!(!trace_result.remeasure_triggered);
        debug_assert_eq!(trace_result.remeasured_cell_count, 0);
        debug_assert_eq!(trace_result.remeasure_skipped_cell_count, 0);
        debug_assert_eq!(trace_result.remeasured_non_empty_cell_count, 0);
        debug_assert_eq!(trace_result.remeasured_with_coordinated_extents_count, 0);
        if !trace_result.remeasure_triggered {
            debug_assert_eq!(trace_result.remeasured_cell_count, 0);
            debug_assert_eq!(trace_result.remeasure_skipped_cell_count, 0);
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

pub(crate) fn debug_assert_inherited_propagation_trace_alignment(
    derivation: &InheritedPropagationIntent,
    trace: &InheritedPropagationTrace,
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
        "inherited propagation execution trace nodes must match derivation nodes in deterministic order"
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
            "inherited propagation child intent count must match derived child count"
        );
        for (idx, child_intent) in derived.child_intents.iter().enumerate() {
            debug_assert_eq!(
                child_intent.child_index, idx,
                "inherited propagation child intents must preserve deterministic child index order"
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
        debug_assert!(
            trace_result.child_plot_area_adjustments_count
                <= derived.expected_plot_area_adjustments_count,
            "inherited propagation may execute fewer plot-area adjustments than derived when an ancestor retarget already propagated the same size"
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
    async fn collection_round_a_applies_initial_aggregates_to_all_matching_groups()
    -> Result<(), AvengerChartError> {
        let (mut measurement, _) = nested_fixture().await?;

        let collection_round_a =
            build_collection_round_a(collect_collection_round_snapshot(&measurement));
        apply_collection_round_a(&mut measurement, &collection_round_a);

        let states = depth1_states(&measurement);
        assert!(
            states.len() >= 2,
            "fixture should yield at least two depth-1 facet bands"
        );

        let first = states.first().expect("at least one depth-1 state");
        let expected_layout = first
            .coordinated_layout
            .as_ref()
            .expect("collection round A should set coordinated layout");
        for state in states.iter().skip(1) {
            assert_eq!(state.key, first.key);
            assert_overflow_close(&state.coordinated_overflow, &first.coordinated_overflow);
            let actual_layout = state
                .coordinated_layout
                .as_ref()
                .expect("collection round A should set coordinated layout");
            assert_layout_close(actual_layout, expected_layout);
        }

        Ok(())
    }

    #[tokio::test]
    async fn inherited_apply_can_mutate_measurements_via_apply_coordinated_overflow()
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

        let inherited_apply_derivation = derive_inherited_apply_intent(&measurement);
        let _inherited_apply = run_inherited_apply_with_trace(
            &mut measurement,
            &eval_ctx,
            &inherited_apply_derivation,
        )
        .await?;

        let after_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;
        assert!(
            (after_cross_size - before_cross_size).abs() > 0.01,
            "inherited apply should mutate subplot cross size when coordinated layout differs"
        );

        Ok(())
    }

    #[tokio::test]
    async fn recollection_round_reconciles_layout_after_inherited_apply_remeasure()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;

        let collection_round_a =
            build_collection_round_a(collect_collection_round_snapshot(&measurement));
        apply_collection_round_a(&mut measurement, &collection_round_a);
        let inherited_apply_derivation = derive_inherited_apply_intent(&measurement);
        let _inherited_apply = run_inherited_apply_with_trace(
            &mut measurement,
            &eval_ctx,
            &inherited_apply_derivation,
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

        let recollection_round =
            build_recollection_round(collect_recollection_round_snapshot(&measurement));
        apply_recollection_round(&mut measurement, &recollection_round);

        let states = depth1_states(&measurement);
        assert!(
            states.len() >= 2,
            "fixture should yield at least two depth-1 facet bands"
        );
        for state in states {
            let coordinated = state
                .coordinated_layout
                .as_ref()
                .expect("recollection round should set coordinated layout");
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
    async fn inherited_propagation_retargets_child_scales_when_parent_cross_size_changes()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;

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

        let inherited_propagation_derivation = derive_inherited_propagation_intent(&measurement);
        let _inherited_propagation = run_inherited_propagation_with_trace(
            &mut measurement,
            &eval_ctx,
            &inherited_propagation_derivation,
        )?;

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
            "inherited propagation should retarget child plot width when parent cross size changes"
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
    async fn collection_round_a_ir_contains_expected_group_counts_and_node_targets()
    -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let collection_round_a =
            build_collection_round_a(collect_collection_round_snapshot(&measurement));
        assert!(!collection_round_a.snapshot.nodes.is_empty());
        assert!(
            !collection_round_a
                .aggregates
                .merged_layout_by_key
                .is_empty()
        );
        assert_eq!(
            collection_round_a.distribution.layout_patches_by_node.len(),
            collection_round_a.snapshot.nodes.len()
        );
        assert!(
            collection_round_a
                .distribution
                .overflow_patches_by_node
                .len()
                <= collection_round_a.snapshot.nodes.len()
        );
        if !collection_round_a
            .aggregates
            .unified_domain_extents
            .is_empty()
        {
            assert_eq!(
                collection_round_a.distribution.domain_target_nodes.len(),
                collection_round_a.snapshot.nodes.len()
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn inherited_apply_ir_trace_records_remeasure_and_parent_cross_propagation()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = col_col_fixture().await?;
        let collection_round_a =
            build_collection_round_a(collect_collection_round_snapshot(&measurement));
        apply_collection_round_a(&mut measurement, &collection_round_a);

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

        let inherited_apply_derivation = derive_inherited_apply_intent(&measurement);
        let inherited_apply = run_inherited_apply_with_trace(
            &mut measurement,
            &eval_ctx,
            &inherited_apply_derivation,
        )
        .await?;
        assert!(!inherited_apply.node_results.is_empty());
        let root_result = inherited_apply
            .node_results
            .iter()
            .find(|result| result.node_id.path.is_empty())
            .expect("inherited apply should include a root node trace");
        assert!(root_result.parent_cross_size_propagated);
        assert!(root_result.derived_has_coordinated_extents);
        assert!(!root_result.derived_remeasure_required);
        assert!(!root_result.remeasure_triggered);
        assert_eq!(root_result.remeasured_cell_count, 0);
        assert_eq!(root_result.remeasure_skipped_cell_count, 0);
        assert_eq!(root_result.remeasured_non_empty_cell_count, 0);
        assert_eq!(root_result.remeasured_with_coordinated_extents_count, 0);
        Ok(())
    }

    #[tokio::test]
    async fn recollection_round_ir_distribution_stabilizes_layout_values()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        let collection_round_a =
            build_collection_round_a(collect_collection_round_snapshot(&measurement));
        apply_collection_round_a(&mut measurement, &collection_round_a);
        let inherited_apply_derivation = derive_inherited_apply_intent(&measurement);
        let _inherited_apply = run_inherited_apply_with_trace(
            &mut measurement,
            &eval_ctx,
            &inherited_apply_derivation,
        )
        .await?;

        let recollection_round =
            build_recollection_round(collect_recollection_round_snapshot(&measurement));
        assert!(
            !recollection_round
                .distribution
                .layout_patches_by_node
                .is_empty()
        );
        apply_recollection_round(&mut measurement, &recollection_round);

        let states = depth1_states(&measurement);
        assert!(states.len() >= 2);
        let first_layout = states
            .first()
            .and_then(|state| state.coordinated_layout.as_ref())
            .expect("recollection round should set coordinated layout on depth-1 states")
            .clone();
        for state in states.iter().skip(1) {
            let coordinated = state
                .coordinated_layout
                .as_ref()
                .expect("recollection round should set coordinated layout");
            assert_layout_close(coordinated, &first_layout);
        }
        Ok(())
    }

    #[tokio::test]
    async fn inherited_propagation_ir_trace_records_plot_resize_and_scale_retarget_events()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;

        {
            let root = facet_band_mut(&mut measurement)
                .expect("fixture should produce root facet-band measurement");
            root.subplot_cross_size += 40.0;
        }

        let inherited_propagation_derivation = derive_inherited_propagation_intent(&measurement);
        let inherited_propagation = run_inherited_propagation_with_trace(
            &mut measurement,
            &eval_ctx,
            &inherited_propagation_derivation,
        )?;
        assert!(!inherited_propagation.node_results.is_empty());
        let root_result = inherited_propagation
            .node_results
            .iter()
            .find(|result| result.node_id.path.is_empty())
            .expect("inherited propagation should include a root node trace");
        let root_derivation = inherited_propagation_derivation
            .node_derivations
            .iter()
            .find(|node| node.node_id.path.is_empty())
            .expect("inherited propagation derivation should include a root node");
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
        assert!(
            root_result.child_plot_area_adjustments_count
                <= root_derivation.expected_plot_area_adjustments_count
        );
        assert_eq!(root_result.derived_child_count, root_derivation.child_count);
        Ok(())
    }

    #[tokio::test]
    async fn inherited_propagation_derivation_contains_ordered_child_intents()
    -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let derivation = derive_inherited_propagation_intent(&measurement);
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
    async fn inherited_propagation_trace_matches_derived_child_intent_adjustment_counts()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        {
            let root = facet_band_mut(&mut measurement)
                .expect("fixture should produce root facet-band measurement");
            root.subplot_cross_size += 40.0;
        }

        let derivation = derive_inherited_propagation_intent(&measurement);
        let inherited_propagation =
            run_inherited_propagation_with_trace(&mut measurement, &eval_ctx, &derivation)?;

        for (derived, trace_result) in derivation
            .node_derivations
            .iter()
            .zip(inherited_propagation.node_results.iter())
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
            assert!(
                trace_result.child_plot_area_adjustments_count
                    <= derived.expected_plot_area_adjustments_count
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn inherited_propagation_child_intents_encode_axis_target_cross_sizes()
    -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let derivation = derive_inherited_propagation_intent(&measurement);
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
    async fn inherited_apply_derivation_is_read_only_and_node_complete()
    -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let before_node_ids = measurement_node_ids(&measurement);
        let before_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;

        let derivation = derive_inherited_apply_intent(&measurement);

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
    async fn inherited_apply_derivation_contains_apply_plan_for_each_node()
    -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let derivation = derive_inherited_apply_intent(&measurement);
        assert!(!derivation.node_derivations.is_empty());
        for node in &derivation.node_derivations {
            assert_eq!(node.axis, node.apply_plan.axis);
            assert!(!node.remeasure_triggered);
            if node.has_legend_overflow {
                assert!(node.apply_plan.legend_slab_applied > 0.0);
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn inherited_apply_executor_applies_derivation_with_behavior_parity()
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

        let derivation = derive_inherited_apply_intent(&measurement);
        let inherited_apply =
            run_inherited_apply_with_trace(&mut measurement, &eval_ctx, &derivation).await?;

        debug_assert_inherited_apply_trace_alignment(&derivation, &inherited_apply);
        let root_result = inherited_apply
            .node_results
            .iter()
            .find(|result| result.node_id.path.is_empty())
            .expect("inherited apply should include a root node trace");
        assert_eq!(root_result.axis, FacetAxis::Column);
        assert!(root_result.subplot_cross_size_after > 0.0);
        assert!(root_result.derived_has_coordinated_extents);
        assert!(!root_result.derived_remeasure_required);
        assert!(!root_result.remeasure_triggered);
        assert_eq!(root_result.remeasured_cell_count, 0);
        assert_eq!(root_result.remeasure_skipped_cell_count, 0);
        assert_eq!(root_result.remeasured_non_empty_cell_count, 0);
        assert_eq!(root_result.remeasured_with_coordinated_extents_count, 0);
        Ok(())
    }

    #[tokio::test]
    async fn inherited_apply_node_identity_stable_between_derivation_and_execution()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        let collection_round_a =
            build_collection_round_a(collect_collection_round_snapshot(&measurement));
        apply_collection_round_a(&mut measurement, &collection_round_a);

        let inherited_apply_derivation = derive_inherited_apply_intent(&measurement);
        let inherited_apply = run_inherited_apply_with_trace(
            &mut measurement,
            &eval_ctx,
            &inherited_apply_derivation,
        )
        .await?;

        assert_eq!(
            inherited_apply
                .node_results
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>(),
            inherited_apply_derivation
                .node_derivations
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>()
        );
        Ok(())
    }

    #[tokio::test]
    async fn inherited_propagation_derivation_is_read_only_and_node_complete()
    -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let before_node_ids = measurement_node_ids(&measurement);
        let before_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;

        let derivation = derive_inherited_propagation_intent(&measurement);

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
    async fn inherited_propagation_executor_applies_retarget_with_behavior_parity()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        {
            let root = facet_band_mut(&mut measurement)
                .expect("fixture should produce root facet-band measurement");
            root.subplot_cross_size += 40.0;
        }

        let derivation = derive_inherited_propagation_intent(&measurement);
        let inherited_propagation =
            run_inherited_propagation_with_trace(&mut measurement, &eval_ctx, &derivation)?;
        debug_assert_inherited_propagation_trace_alignment(&derivation, &inherited_propagation);

        let root_result = inherited_propagation
            .node_results
            .iter()
            .find(|result| result.node_id.path.is_empty())
            .expect("inherited propagation should include a root node trace");
        assert!(root_result.child_plot_area_adjustments_count > 0);
        assert!(root_result.derived_child_intent_count > 0);
        assert!(
            root_result.child_plot_area_adjustments_count
                <= root_result.derived_expected_plot_area_adjustments_count
        );
        Ok(())
    }

    #[tokio::test]
    async fn collection_round_a_distribution_patch_coverage_matches_snapshot()
    -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let collection_round_a =
            build_collection_round_a(collect_collection_round_snapshot(&measurement));
        debug_assert_collection_round_coverage(&collection_round_a);
        let snapshot_nodes: std::collections::HashSet<CoordNodeKey> = collection_round_a
            .snapshot
            .nodes
            .iter()
            .map(|node| node.node_id.clone())
            .collect();
        let layout_patch_nodes: std::collections::HashSet<CoordNodeKey> = collection_round_a
            .distribution
            .layout_patches_by_node
            .keys()
            .cloned()
            .collect();
        assert_eq!(layout_patch_nodes, snapshot_nodes);
        Ok(())
    }

    #[tokio::test]
    async fn recollection_round_distribution_patch_coverage_matches_snapshot()
    -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let recollection_round =
            build_recollection_round(collect_recollection_round_snapshot(&measurement));
        debug_assert_recollection_round_coverage(&recollection_round);
        let snapshot_nodes: std::collections::HashSet<CoordNodeKey> = recollection_round
            .snapshot
            .nodes
            .iter()
            .map(|node| node.node_id.clone())
            .collect();
        let layout_patch_nodes: std::collections::HashSet<CoordNodeKey> = recollection_round
            .distribution
            .layout_patches_by_node
            .keys()
            .cloned()
            .collect();
        assert_eq!(layout_patch_nodes, snapshot_nodes);
        Ok(())
    }

    #[tokio::test]
    async fn inherited_apply_and_inherited_propagation_node_identity_stable_across_derivation_and_execution()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        let collection_round_a =
            build_collection_round_a(collect_collection_round_snapshot(&measurement));
        apply_collection_round_a(&mut measurement, &collection_round_a);

        let inherited_apply_derivation = derive_inherited_apply_intent(&measurement);
        let inherited_apply = run_inherited_apply_with_trace(
            &mut measurement,
            &eval_ctx,
            &inherited_apply_derivation,
        )
        .await?;
        assert_eq!(
            inherited_apply
                .node_results
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>(),
            inherited_apply_derivation
                .node_derivations
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>()
        );

        let recollection_round =
            build_recollection_round(collect_recollection_round_snapshot(&measurement));
        apply_recollection_round(&mut measurement, &recollection_round);

        let inherited_propagation_derivation = derive_inherited_propagation_intent(&measurement);
        let inherited_propagation = run_inherited_propagation_with_trace(
            &mut measurement,
            &eval_ctx,
            &inherited_propagation_derivation,
        )?;
        assert_eq!(
            inherited_propagation
                .node_results
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>(),
            inherited_propagation_derivation
                .node_derivations
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>()
        );
        for (derived, trace_result) in inherited_propagation
            .node_results
            .iter()
            .zip(inherited_propagation_derivation.node_derivations.iter())
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
