//! Facet coordination pipeline for aligning layout requirements.
//!
//! Coordination runs after local facet-band layout and applies four ordered steps:
//! 1. collect and distribute initial requirements,
//! 2. retarget affected measurements,
//! 3. collect and distribute retargeted requirements,
//! 4. propagate final plot-area and scale-range updates.
//!
//! Immutable coordination plans are built in `coordination_attributes`, and
//! bounded side effects are applied through executors in `coordination_apply`.

use std::collections::HashSet;

use crate::{
    error::AvengerChartError,
    facet::{
        coordination_apply::visit_facet_bands_with_node_id,
        coordination_attributes::{
            CoordNodeKey, FinalPropagationPlan, FinalPropagationTrace,
            InitialRequirementNodeSnapshot, InitialRequirementPass, InitialRequirementSnapshot,
            RetargetPlan, RetargetTrace, RetargetedRequirementNodeSnapshot,
            RetargetedRequirementPass, RetargetedRequirementSnapshot,
        },
    },
    plot::compiled::ComponentsMeasurement,
    render::EvaluationContext,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FacetCoordinationStage {
    InitialRequirements,
    Retarget,
    RetargetedRequirements,
    FinalPropagation,
}

#[cfg(test)]
use crate::facet::coord::{
    FacetBandCoordMeasurement, facet_band_canvas_mut as facet_band_canvas_mut_from_coord,
    facet_band_canvas_ref as facet_band_canvas_ref_from_coord,
};
#[cfg(test)]
use crate::facet::coordination_apply::{
    apply_initial_requirement_pass, apply_retargeted_requirement_pass,
    build_final_propagation_plan, build_retarget_plan, run_final_propagation_with_trace,
    run_retarget_with_trace, visit_facet_bands_with_node_id_mut,
};
#[cfg(test)]
use crate::facet::coordination_attributes::{
    build_initial_requirement_pass, build_retargeted_requirement_pass,
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

pub(crate) fn collect_initial_requirement_snapshot(
    measurement: &ComponentsMeasurement,
) -> InitialRequirementSnapshot {
    let mut nodes = Vec::new();
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, depth, facet_band| {
            let mut domain_infos = Vec::new();
            facet_band.collect_cell_domain_infos(&mut domain_infos);
            nodes.push(InitialRequirementNodeSnapshot {
                node_id: node_id.clone(),
                key: facet_band.coordination_group_key_for_depth(depth),
                local_overflow: facet_band.local_overflow_value(),
                local_layout: facet_band.local_layout_value(),
                domain_infos,
            });
        },
    );
    InitialRequirementSnapshot { nodes }
}

pub(crate) fn collect_retargeted_requirement_snapshot(
    measurement: &ComponentsMeasurement,
) -> RetargetedRequirementSnapshot {
    let mut nodes = Vec::new();
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, depth, facet_band| {
            nodes.push(RetargetedRequirementNodeSnapshot {
                node_id: node_id.clone(),
                key: facet_band.coordination_group_key_for_depth(depth),
                local_overflow: facet_band.local_overflow_value(),
                local_layout: facet_band.local_layout_value(),
            });
        },
    );
    RetargetedRequirementSnapshot { nodes }
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

pub(crate) fn debug_assert_stage_transition(
    current: Option<FacetCoordinationStage>,
    next: FacetCoordinationStage,
) {
    let allowed = matches!(
        (current, next),
        (None, FacetCoordinationStage::InitialRequirements)
            | (
                Some(FacetCoordinationStage::InitialRequirements),
                FacetCoordinationStage::Retarget
            )
            | (
                Some(FacetCoordinationStage::Retarget),
                FacetCoordinationStage::RetargetedRequirements
            )
            | (
                Some(FacetCoordinationStage::RetargetedRequirements),
                FacetCoordinationStage::FinalPropagation
            )
    );
    debug_assert!(
        allowed,
        "invalid coordination stage transition: current={current:?}, next={next:?}"
    );
}

pub(crate) fn debug_assert_initial_requirement_coverage(
    initial_requirement_pass: &InitialRequirementPass,
) {
    let snapshot_nodes: HashSet<CoordNodeKey> = initial_requirement_pass
        .snapshot
        .nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect();
    let layout_patch_nodes: HashSet<CoordNodeKey> = initial_requirement_pass
        .distribution
        .layout_patches_by_node
        .keys()
        .cloned()
        .collect();

    debug_assert_eq!(
        layout_patch_nodes, snapshot_nodes,
        "initial requirements layout patch coverage must match snapshot nodes"
    );

    if !initial_requirement_pass
        .aggregates
        .unified_domain_extents
        .is_empty()
    {
        debug_assert_eq!(
            initial_requirement_pass.distribution.domain_target_nodes, snapshot_nodes,
            "initial requirements domain target coverage must match snapshot nodes when domain aggregates exist"
        );
    }
}

pub(crate) fn debug_assert_retargeted_requirement_coverage(
    retargeted_requirement_pass: &RetargetedRequirementPass,
) {
    let snapshot_nodes: HashSet<CoordNodeKey> = retargeted_requirement_pass
        .snapshot
        .nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect();
    let layout_patch_nodes: HashSet<CoordNodeKey> = retargeted_requirement_pass
        .distribution
        .layout_patches_by_node
        .keys()
        .cloned()
        .collect();

    debug_assert_eq!(
        layout_patch_nodes, snapshot_nodes,
        "retargeted requirements layout patch coverage must match snapshot nodes"
    );
}

pub(crate) fn debug_assert_retarget_plan_coverage(
    measurement: &ComponentsMeasurement,
    plan: &RetargetPlan,
) {
    let expected_ids: HashSet<CoordNodeKey> =
        measurement_node_ids(measurement).into_iter().collect();
    let actual_ids: HashSet<CoordNodeKey> = plan
        .node_plans
        .iter()
        .map(|node| node.node_id.clone())
        .collect();

    debug_assert_eq!(
        actual_ids, expected_ids,
        "retarget plan nodes must match current measurement tree node coverage"
    );
}

pub(crate) fn debug_assert_final_propagation_plan_coverage(
    measurement: &ComponentsMeasurement,
    plan: &FinalPropagationPlan,
) {
    let expected_ids: HashSet<CoordNodeKey> =
        measurement_node_ids(measurement).into_iter().collect();
    let actual_ids: HashSet<CoordNodeKey> = plan
        .node_plans
        .iter()
        .map(|node| node.node_id.clone())
        .collect();

    debug_assert_eq!(
        actual_ids, expected_ids,
        "final propagation plan nodes must match current measurement tree node coverage"
    );
}

pub(crate) fn debug_assert_retarget_trace_alignment(plan: &RetargetPlan, trace: &RetargetTrace) {
    let derived_ids: Vec<CoordNodeKey> = plan
        .node_plans
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
        "retarget execution trace nodes must match plan nodes in deterministic order"
    );

    for (planned, trace_result) in plan.node_plans.iter().zip(trace.node_results.iter()) {
        debug_assert_eq!(trace_result.node_id, planned.node_id);
        debug_assert_eq!(trace_result.axis, planned.axis);
        debug_assert_eq!(
            trace_result.planned_has_legend_overflow,
            planned.has_legend_overflow
        );
        debug_assert_eq!(
            trace_result.planned_has_coordinated_extents,
            planned.has_coordinated_extents
        );
        debug_assert_eq!(
            trace_result.planned_remeasure_required,
            planned.remeasure_triggered
        );
        debug_assert_eq!(
            trace_result.planned_axis_owner_ignore_empty_cells,
            planned.apply_plan.axis_owner_ignore_empty_cells
        );
        debug_assert_eq!(
            trace_result.planned_adjusted_main_size,
            planned.apply_plan.adjusted_main_size
        );
        debug_assert_eq!(
            trace_result.remeasure_triggered,
            planned.remeasure_triggered
        );
        debug_assert_eq!(trace_result.planned_child_count, planned.child_count);
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

pub(crate) fn debug_assert_final_propagation_trace_alignment(
    plan: &FinalPropagationPlan,
    trace: &FinalPropagationTrace,
) {
    let derived_ids: Vec<CoordNodeKey> = plan
        .node_plans
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
        "final propagation execution trace nodes must match plan nodes in deterministic order"
    );

    for (planned, trace_result) in plan.node_plans.iter().zip(trace.node_results.iter()) {
        debug_assert_eq!(trace_result.node_id, planned.node_id);
        debug_assert_eq!(trace_result.axis, planned.axis);
        debug_assert_eq!(
            trace_result.planned_parent_cross_size_target,
            planned.parent_cross_size_target
        );
        debug_assert_eq!(trace_result.planned_child_count, planned.child_count);
        debug_assert_eq!(
            planned.child_plans.len(),
            planned.child_count,
            "final propagation child plan count must match planned child count"
        );
        for (idx, child_plan) in planned.child_plans.iter().enumerate() {
            debug_assert_eq!(
                child_plan.child_index, idx,
                "final propagation child plans must preserve deterministic child index order"
            );
        }
        debug_assert_eq!(
            trace_result.planned_child_plan_count,
            planned.child_plans.len()
        );
        debug_assert_eq!(
            trace_result.planned_plot_area_adjustments_count,
            planned.expected_plot_area_adjustments_count
        );
        debug_assert!(
            trace_result.child_plot_area_adjustments_count
                <= planned.expected_plot_area_adjustments_count,
            "final propagation may execute fewer plot-area adjustments than planned when an ancestor retarget already propagated the same size"
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
    async fn initial_requirement_pass_applies_initial_aggregates_to_all_matching_groups()
    -> Result<(), AvengerChartError> {
        let (mut measurement, _) = nested_fixture().await?;

        let initial_requirement_pass =
            build_initial_requirement_pass(collect_initial_requirement_snapshot(&measurement));
        apply_initial_requirement_pass(&mut measurement, &initial_requirement_pass);

        let states = depth1_states(&measurement);
        assert!(
            states.len() >= 2,
            "fixture should yield at least two depth-1 facet bands"
        );

        let first = states.first().expect("at least one depth-1 state");
        let expected_layout = first
            .coordinated_layout
            .as_ref()
            .expect("initial requirements should set coordinated layout");
        for state in states.iter().skip(1) {
            assert_eq!(state.key, first.key);
            assert_overflow_close(&state.coordinated_overflow, &first.coordinated_overflow);
            let actual_layout = state
                .coordinated_layout
                .as_ref()
                .expect("initial requirements should set coordinated layout");
            assert_layout_close(actual_layout, expected_layout);
        }

        Ok(())
    }

    #[tokio::test]
    async fn retarget_trace_can_mutate_measurements_via_apply_coordinated_overflow()
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

        let retarget_plan = build_retarget_plan(&measurement);
        let _retarget_trace =
            run_retarget_with_trace(&mut measurement, &eval_ctx, &retarget_plan).await?;

        let after_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;
        assert!(
            (after_cross_size - before_cross_size).abs() > 0.01,
            "retarget should mutate subplot cross size when coordinated layout differs"
        );

        Ok(())
    }

    #[tokio::test]
    async fn retargeted_requirement_pass_reconciles_layout_after_retarget_trace_remeasure()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;

        let initial_requirement_pass =
            build_initial_requirement_pass(collect_initial_requirement_snapshot(&measurement));
        apply_initial_requirement_pass(&mut measurement, &initial_requirement_pass);
        let retarget_plan = build_retarget_plan(&measurement);
        let _retarget_trace =
            run_retarget_with_trace(&mut measurement, &eval_ctx, &retarget_plan).await?;

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

        let retargeted_requirement_pass = build_retargeted_requirement_pass(
            collect_retargeted_requirement_snapshot(&measurement),
        );
        apply_retargeted_requirement_pass(&mut measurement, &retargeted_requirement_pass);

        let states = depth1_states(&measurement);
        assert!(
            states.len() >= 2,
            "fixture should yield at least two depth-1 facet bands"
        );
        for state in states {
            let coordinated = state
                .coordinated_layout
                .as_ref()
                .expect("retargeted requirements should set coordinated layout");
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
    async fn final_propagation_trace_retargets_child_scales_when_parent_cross_size_changes()
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

        let final_propagation_plan = build_final_propagation_plan(&measurement);
        let _final_propagation_trace =
            run_final_propagation_with_trace(&mut measurement, &eval_ctx, &final_propagation_plan)?;

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
            "final propagation should retarget child plot width when parent cross size changes"
        );

        if let Some(scale_with_spec) = first_child.scales.get(root.axis.scale_name())
            && let Ok((start, end)) = scale_with_spec.configured().numeric_interval_range()
        {
            let span = (end - start).abs();
            assert!(approx_eq_within(span, target_cross_size, 2.0));
        }

        Ok(())
    }

    #[tokio::test]
    async fn initial_requirement_pass_contains_expected_group_counts_and_node_targets()
    -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let initial_requirement_pass =
            build_initial_requirement_pass(collect_initial_requirement_snapshot(&measurement));
        assert!(!initial_requirement_pass.snapshot.nodes.is_empty());
        assert!(
            !initial_requirement_pass
                .aggregates
                .merged_layout_by_key
                .is_empty()
        );
        assert_eq!(
            initial_requirement_pass
                .distribution
                .layout_patches_by_node
                .len(),
            initial_requirement_pass.snapshot.nodes.len()
        );
        assert!(
            initial_requirement_pass
                .distribution
                .overflow_patches_by_node
                .len()
                <= initial_requirement_pass.snapshot.nodes.len()
        );
        if !initial_requirement_pass
            .aggregates
            .unified_domain_extents
            .is_empty()
        {
            assert_eq!(
                initial_requirement_pass
                    .distribution
                    .domain_target_nodes
                    .len(),
                initial_requirement_pass.snapshot.nodes.len()
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn retarget_trace_records_remeasure_and_parent_cross_propagation()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = col_col_fixture().await?;
        let initial_requirement_pass =
            build_initial_requirement_pass(collect_initial_requirement_snapshot(&measurement));
        apply_initial_requirement_pass(&mut measurement, &initial_requirement_pass);

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

        let retarget_plan = build_retarget_plan(&measurement);
        let retarget_trace =
            run_retarget_with_trace(&mut measurement, &eval_ctx, &retarget_plan).await?;
        assert!(!retarget_trace.node_results.is_empty());
        let root_result = retarget_trace
            .node_results
            .iter()
            .find(|result| result.node_id.path.is_empty())
            .expect("retarget should include a root node trace");
        assert!(root_result.parent_cross_size_propagated);
        assert!(root_result.planned_has_coordinated_extents);
        assert!(!root_result.planned_remeasure_required);
        assert!(!root_result.remeasure_triggered);
        assert_eq!(root_result.remeasured_cell_count, 0);
        assert_eq!(root_result.remeasure_skipped_cell_count, 0);
        assert_eq!(root_result.remeasured_non_empty_cell_count, 0);
        assert_eq!(root_result.remeasured_with_coordinated_extents_count, 0);
        Ok(())
    }

    #[tokio::test]
    async fn retargeted_requirement_pass_distribution_stabilizes_layout_values()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        let initial_requirement_pass =
            build_initial_requirement_pass(collect_initial_requirement_snapshot(&measurement));
        apply_initial_requirement_pass(&mut measurement, &initial_requirement_pass);
        let retarget_plan = build_retarget_plan(&measurement);
        let _retarget_trace =
            run_retarget_with_trace(&mut measurement, &eval_ctx, &retarget_plan).await?;

        let retargeted_requirement_pass = build_retargeted_requirement_pass(
            collect_retargeted_requirement_snapshot(&measurement),
        );
        assert!(
            !retargeted_requirement_pass
                .distribution
                .layout_patches_by_node
                .is_empty()
        );
        apply_retargeted_requirement_pass(&mut measurement, &retargeted_requirement_pass);

        let states = depth1_states(&measurement);
        assert!(states.len() >= 2);
        let first_layout = states
            .first()
            .and_then(|state| state.coordinated_layout.as_ref())
            .expect("retargeted requirements should set coordinated layout on depth-1 states")
            .clone();
        for state in states.iter().skip(1) {
            let coordinated = state
                .coordinated_layout
                .as_ref()
                .expect("retargeted requirements should set coordinated layout");
            assert_layout_close(coordinated, &first_layout);
        }
        Ok(())
    }

    #[tokio::test]
    async fn final_propagation_trace_records_plot_resize_and_scale_retarget_events()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;

        {
            let root = facet_band_mut(&mut measurement)
                .expect("fixture should produce root facet-band measurement");
            root.subplot_cross_size += 40.0;
        }

        let final_propagation_plan = build_final_propagation_plan(&measurement);
        let final_propagation_trace =
            run_final_propagation_with_trace(&mut measurement, &eval_ctx, &final_propagation_plan)?;
        assert!(!final_propagation_trace.node_results.is_empty());
        let root_result = final_propagation_trace
            .node_results
            .iter()
            .find(|result| result.node_id.path.is_empty())
            .expect("final propagation should include a root node trace");
        let root_plan = final_propagation_plan
            .node_plans
            .iter()
            .find(|node| node.node_id.path.is_empty())
            .expect("final propagation plan should include a root node");
        assert!(root_result.child_plot_area_adjustments_count > 0);
        assert!(root_result.scale_range_retarget_count > 0);
        assert_eq!(
            root_result.planned_child_plan_count,
            root_plan.child_plans.len()
        );
        assert_eq!(
            root_result.planned_plot_area_adjustments_count,
            root_plan.expected_plot_area_adjustments_count
        );
        assert!(
            root_result.child_plot_area_adjustments_count
                <= root_plan.expected_plot_area_adjustments_count
        );
        assert_eq!(root_result.planned_child_count, root_plan.child_count);
        Ok(())
    }

    #[tokio::test]
    async fn final_propagation_plan_contains_ordered_child_plans() -> Result<(), AvengerChartError>
    {
        let (measurement, _) = nested_fixture().await?;
        let plan = build_final_propagation_plan(&measurement);
        assert!(!plan.node_plans.is_empty());
        for node in &plan.node_plans {
            assert_eq!(node.child_plans.len(), node.child_count);
            let expected_adjustments = node
                .child_plans
                .iter()
                .filter(|child_plan| child_plan.adjust_plot_area)
                .count();
            assert_eq!(
                node.expected_plot_area_adjustments_count,
                expected_adjustments
            );
            for (idx, child_plan) in node.child_plans.iter().enumerate() {
                assert_eq!(child_plan.child_index, idx);
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn final_propagation_trace_matches_planned_child_adjustment_counts()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        {
            let root = facet_band_mut(&mut measurement)
                .expect("fixture should produce root facet-band measurement");
            root.subplot_cross_size += 40.0;
        }

        let plan = build_final_propagation_plan(&measurement);
        let final_propagation_trace =
            run_final_propagation_with_trace(&mut measurement, &eval_ctx, &plan)?;

        for (planned, trace_result) in plan
            .node_plans
            .iter()
            .zip(final_propagation_trace.node_results.iter())
        {
            assert_eq!(trace_result.node_id, planned.node_id);
            assert_eq!(
                trace_result.planned_child_plan_count,
                planned.child_plans.len()
            );
            assert_eq!(
                trace_result.planned_plot_area_adjustments_count,
                planned.expected_plot_area_adjustments_count
            );
            assert!(
                trace_result.child_plot_area_adjustments_count
                    <= planned.expected_plot_area_adjustments_count
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn final_propagation_trace_child_plans_encode_axis_target_cross_sizes()
    -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let plan = build_final_propagation_plan(&measurement);
        for node in &plan.node_plans {
            match (node.axis, node.parent_cross_size_target) {
                (FacetAxis::Column, Some(target_cross_size)) => {
                    for child_plan in &node.child_plans {
                        assert!(
                            child_plan.target_plot_area_height.is_none(),
                            "column plan should not target child plot height"
                        );
                        if child_plan.adjust_plot_area {
                            assert_eq!(child_plan.target_plot_area_width, Some(target_cross_size));
                        } else {
                            assert!(child_plan.target_plot_area_width.is_none());
                        }
                        if child_plan.update_band_range {
                            assert_eq!(child_plan.target_band_range_end, Some(target_cross_size));
                        } else {
                            assert!(child_plan.target_band_range_end.is_none());
                        }
                    }
                }
                (FacetAxis::Row, Some(target_cross_size)) => {
                    for child_plan in &node.child_plans {
                        assert!(
                            child_plan.target_plot_area_width.is_none(),
                            "row plan should not target child plot width"
                        );
                        if child_plan.adjust_plot_area {
                            assert_eq!(child_plan.target_plot_area_height, Some(target_cross_size));
                        } else {
                            assert!(child_plan.target_plot_area_height.is_none());
                        }
                        if child_plan.update_band_range {
                            assert_eq!(child_plan.target_band_range_end, Some(target_cross_size));
                        } else {
                            assert!(child_plan.target_band_range_end.is_none());
                        }
                    }
                }
                (_, None) => {
                    for child_plan in &node.child_plans {
                        assert!(child_plan.target_plot_area_width.is_none());
                        assert!(child_plan.target_plot_area_height.is_none());
                        assert!(child_plan.target_band_range_end.is_none());
                        assert!(!child_plan.adjust_plot_area);
                        assert!(!child_plan.update_band_range);
                    }
                }
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn retarget_plan_is_read_only_and_node_complete() -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let before_node_ids = measurement_node_ids(&measurement);
        let before_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;

        let plan = build_retarget_plan(&measurement);

        let after_node_ids = measurement_node_ids(&measurement);
        let after_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;

        assert_eq!(before_node_ids, after_node_ids);
        assert_eq!(before_cross_size, after_cross_size);
        assert_eq!(
            plan.node_plans
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
    async fn retarget_plan_contains_apply_plan_for_each_node() -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let plan = build_retarget_plan(&measurement);
        assert!(!plan.node_plans.is_empty());
        for node in &plan.node_plans {
            assert_eq!(node.axis, node.apply_plan.axis);
            assert!(!node.remeasure_triggered);
            if node.has_legend_overflow {
                assert!(node.apply_plan.legend_slab_applied > 0.0);
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn retarget_executor_applies_plan_with_behavior_parity() -> Result<(), AvengerChartError>
    {
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

        let plan = build_retarget_plan(&measurement);
        let retarget_trace = run_retarget_with_trace(&mut measurement, &eval_ctx, &plan).await?;

        debug_assert_retarget_trace_alignment(&plan, &retarget_trace);
        let root_result = retarget_trace
            .node_results
            .iter()
            .find(|result| result.node_id.path.is_empty())
            .expect("retarget should include a root node trace");
        assert_eq!(root_result.axis, FacetAxis::Column);
        assert!(root_result.subplot_cross_size_after > 0.0);
        assert!(root_result.planned_has_coordinated_extents);
        assert!(!root_result.planned_remeasure_required);
        assert!(!root_result.remeasure_triggered);
        assert_eq!(root_result.remeasured_cell_count, 0);
        assert_eq!(root_result.remeasure_skipped_cell_count, 0);
        assert_eq!(root_result.remeasured_non_empty_cell_count, 0);
        assert_eq!(root_result.remeasured_with_coordinated_extents_count, 0);
        Ok(())
    }

    #[tokio::test]
    async fn retarget_trace_node_identity_stable_between_plan_and_execution()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        let initial_requirement_pass =
            build_initial_requirement_pass(collect_initial_requirement_snapshot(&measurement));
        apply_initial_requirement_pass(&mut measurement, &initial_requirement_pass);

        let retarget_plan = build_retarget_plan(&measurement);
        let retarget_trace =
            run_retarget_with_trace(&mut measurement, &eval_ctx, &retarget_plan).await?;

        assert_eq!(
            retarget_trace
                .node_results
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>(),
            retarget_plan
                .node_plans
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>()
        );
        Ok(())
    }

    #[tokio::test]
    async fn final_propagation_plan_is_read_only_and_node_complete() -> Result<(), AvengerChartError>
    {
        let (measurement, _) = nested_fixture().await?;
        let before_node_ids = measurement_node_ids(&measurement);
        let before_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;

        let plan = build_final_propagation_plan(&measurement);

        let after_node_ids = measurement_node_ids(&measurement);
        let after_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;
        assert_eq!(before_node_ids, after_node_ids);
        assert_eq!(before_cross_size, after_cross_size);
        assert_eq!(
            plan.node_plans
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
    async fn final_propagation_trace_executor_applies_retarget_with_behavior_parity()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        {
            let root = facet_band_mut(&mut measurement)
                .expect("fixture should produce root facet-band measurement");
            root.subplot_cross_size += 40.0;
        }

        let plan = build_final_propagation_plan(&measurement);
        let final_propagation_trace =
            run_final_propagation_with_trace(&mut measurement, &eval_ctx, &plan)?;
        debug_assert_final_propagation_trace_alignment(&plan, &final_propagation_trace);

        let root_result = final_propagation_trace
            .node_results
            .iter()
            .find(|result| result.node_id.path.is_empty())
            .expect("final propagation should include a root node trace");
        assert!(root_result.child_plot_area_adjustments_count > 0);
        assert!(root_result.planned_child_plan_count > 0);
        assert!(
            root_result.child_plot_area_adjustments_count
                <= root_result.planned_plot_area_adjustments_count
        );
        Ok(())
    }

    #[tokio::test]
    async fn initial_requirement_pass_distribution_patch_coverage_matches_snapshot()
    -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let initial_requirement_pass =
            build_initial_requirement_pass(collect_initial_requirement_snapshot(&measurement));
        debug_assert_initial_requirement_coverage(&initial_requirement_pass);
        let snapshot_nodes: std::collections::HashSet<CoordNodeKey> = initial_requirement_pass
            .snapshot
            .nodes
            .iter()
            .map(|node| node.node_id.clone())
            .collect();
        let layout_patch_nodes: std::collections::HashSet<CoordNodeKey> = initial_requirement_pass
            .distribution
            .layout_patches_by_node
            .keys()
            .cloned()
            .collect();
        assert_eq!(layout_patch_nodes, snapshot_nodes);
        Ok(())
    }

    #[tokio::test]
    async fn retargeted_requirement_pass_distribution_patch_coverage_matches_snapshot()
    -> Result<(), AvengerChartError> {
        let (measurement, _) = nested_fixture().await?;
        let retargeted_requirement_pass = build_retargeted_requirement_pass(
            collect_retargeted_requirement_snapshot(&measurement),
        );
        debug_assert_retargeted_requirement_coverage(&retargeted_requirement_pass);
        let snapshot_nodes: std::collections::HashSet<CoordNodeKey> = retargeted_requirement_pass
            .snapshot
            .nodes
            .iter()
            .map(|node| node.node_id.clone())
            .collect();
        let layout_patch_nodes: std::collections::HashSet<CoordNodeKey> =
            retargeted_requirement_pass
                .distribution
                .layout_patches_by_node
                .keys()
                .cloned()
                .collect();
        assert_eq!(layout_patch_nodes, snapshot_nodes);
        Ok(())
    }

    #[tokio::test]
    async fn retarget_and_final_propagation_node_identity_stable_across_plan_and_execution()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        let initial_requirement_pass =
            build_initial_requirement_pass(collect_initial_requirement_snapshot(&measurement));
        apply_initial_requirement_pass(&mut measurement, &initial_requirement_pass);

        let retarget_plan = build_retarget_plan(&measurement);
        let retarget_trace =
            run_retarget_with_trace(&mut measurement, &eval_ctx, &retarget_plan).await?;
        assert_eq!(
            retarget_trace
                .node_results
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>(),
            retarget_plan
                .node_plans
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>()
        );

        let retargeted_requirement_pass = build_retargeted_requirement_pass(
            collect_retargeted_requirement_snapshot(&measurement),
        );
        apply_retargeted_requirement_pass(&mut measurement, &retargeted_requirement_pass);

        let final_propagation_plan = build_final_propagation_plan(&measurement);
        let final_propagation_trace =
            run_final_propagation_with_trace(&mut measurement, &eval_ctx, &final_propagation_plan)?;
        assert_eq!(
            final_propagation_trace
                .node_results
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>(),
            final_propagation_plan
                .node_plans
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>()
        );
        for (planned, trace_result) in final_propagation_trace
            .node_results
            .iter()
            .zip(final_propagation_plan.node_plans.iter())
        {
            assert_eq!(planned.node_id, trace_result.node_id);
            assert_eq!(
                planned.planned_child_plan_count,
                trace_result.child_plans.len()
            );
        }
        Ok(())
    }
}
