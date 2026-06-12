//! Facet coordination pipeline for aligning layout requirements.
//!
//! Coordination runs after local facet-band layout and applies four ordered steps:
//! 1. collect and distribute initial requirements,
//! 2. retarget affected measurements,
//! 3. collect and distribute retargeted requirements,
//! 4. propagate final plot-area and scale-range updates.
//!
//! Immutable coordination plans are built in `coordination_plans`, bounded side
//! effects are applied through `coordination_apply`, and sizing-mode differences
//! are supplied by `coordination_policy`.

use avenger_chart_core::AvengerChartError;
use tracing::{debug, trace};

use crate::{
    facet::{
        coord::renderable_for_empty_policy,
        coordination_apply::{
            apply_requirement_pass, run_final_propagation, run_retarget,
            visit_facet_bands_with_node_id,
        },
        coordination_plans::{
            CoordinationRunArtifacts, RequirementNodeSnapshot, RequirementSnapshot,
            build_requirement_pass_with_round,
        },
        coordination_policy::FacetCoordinationPolicy,
        layout_plan::effective_edge_indices,
    },
    layout::ConvergenceTrace,
    plot::compiled::ComponentsMeasurement,
    render::{CoordinationCheckpoint, EvaluationContext},
};

#[cfg(test)]
use crate::facet::coord::{
    FacetBandCoordMeasurement, facet_band_mut as facet_band_mut_from_coord,
    facet_band_ref as facet_band_ref_from_coord,
};
#[cfg(test)]
use crate::facet::coordination_apply::{
    apply_final_propagation_decisions, apply_retarget_decisions, build_final_propagation_plan,
    derive_retarget_decisions, visit_facet_bands_with_node_id_mut,
};
#[cfg(test)]
use crate::facet::coordination_plans::{CoordinationNodeKey, build_requirement_pass};
#[cfg(test)]
use crate::render::context::{FacetRuntimeSizingMode, FacetRuntimeSizingPolicy};

#[cfg(test)]
fn facet_band_ref(measurement: &ComponentsMeasurement) -> Option<&FacetBandCoordMeasurement> {
    facet_band_ref_from_coord(measurement.coord_measurement.as_ref())
}

#[cfg(test)]
fn facet_band_mut(
    measurement: &mut ComponentsMeasurement,
) -> Option<&mut FacetBandCoordMeasurement> {
    facet_band_mut_from_coord(measurement.coord_measurement.as_mut())
}

#[cfg(test)]
fn canvas_sizing_mode(width: f32, height: f32) -> FacetRuntimeSizingMode {
    FacetRuntimeSizingMode::Policy(FacetRuntimeSizingPolicy::fully_canvas_constrained(
        width, height,
    ))
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
            visit(d, facet_band.base());
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
            visit(d, facet_band.base_mut());
        },
    );
}

/// Coordinate overflow, layout, plot-area retargeting, and scale ranges across the measurement tree.
///
/// This is the facet-specific coordination entrypoint invoked from `coords.rs`.
pub async fn coordinate_facet_measurement_tree(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<(), AvengerChartError> {
    let artifacts = run_facet_coordination_pipeline(measurement, eval_ctx).await?;
    trace!(
        initial_requirement_pass_nodes = artifacts.initial_requirement_pass.snapshot.nodes.len(),
        retarget_trace_nodes = artifacts.retarget_trace.node_results.len(),
        retargeted_requirement_pass_nodes =
            artifacts.retargeted_requirement_pass.snapshot.nodes.len(),
        final_propagation_trace_nodes = artifacts.final_propagation_trace.node_results.len(),
        "coordinate_facet_measurement_tree complete"
    );
    Ok(())
}

pub(crate) async fn coordinate_facet_measurement_tree_until(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    checkpoint: CoordinationCheckpoint,
) -> Result<(), AvengerChartError> {
    run_facet_coordination_until(measurement, eval_ctx, checkpoint).await
}

pub(crate) async fn run_facet_coordination_pipeline(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<CoordinationRunArtifacts, AvengerChartError> {
    let artifacts = run_facet_coordination(measurement, eval_ctx, None).await?;
    Ok(artifacts.expect("a full coordination run always produces artifacts"))
}

async fn run_facet_coordination_until(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    checkpoint: CoordinationCheckpoint,
) -> Result<(), AvengerChartError> {
    run_facet_coordination(measurement, eval_ctx, Some(checkpoint))
        .await
        .map(|_| ())
}

/// The coordination pass: collect a requirement snapshot, solve the real
/// facet tree for the channel values, install the solution on every band,
/// retarget frames at the written-back targets, and propagate final plot
/// areas and scale ranges to descendants. `stop_at` maps the public
/// checkpoints onto these stages and returns `None` when the run stops
/// early.
async fn run_facet_coordination(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    stop_at: Option<CoordinationCheckpoint>,
) -> Result<Option<CoordinationRunArtifacts>, AvengerChartError> {
    let mut layout_convergence = ConvergenceTrace::default();
    let mut overflow_convergence = ConvergenceTrace::default();
    let mut shadow_snapshot_hashes: Vec<u64> = Vec::new();

    // One round. Requirement snapshots read only epoch-frozen and
    // construction-time values, so consecutive collect-and-install passes
    // are byte-identical no-ops; the driver is linear: pass -> install ->
    // retarget -> final propagation.
    let snapshot = collect_requirement_snapshot(measurement);
    if crate::facet::tree_solve::shadow_enabled() {
        shadow_snapshot_hashes.push(crate::facet::tree_solve::snapshot_hash(&snapshot));
    }
    // The requirement channels come from the real-tree solve: one tree,
    // real topology, per-node Region.coordinated reads.
    let sizing = eval_ctx.facet_runtime_sizing_mode();
    let solved = crate::facet::tree_solve::tree_solved_round(measurement, sizing)?;
    let requirement_pass = build_requirement_pass_with_round(snapshot, solved)?;
    debug!(
        policy = FacetCoordinationPolicy::LABEL,
        overflow_groups = requirement_pass.diagnostics.overflow_groups,
        boundary_overflow_groups = requirement_pass.diagnostics.boundary_overflow_groups,
        layout_groups = requirement_pass.diagnostics.layout_groups,
        "coordinate_facet_measurement_tree round aggregate + distribution"
    );
    apply_requirement_pass(measurement, &requirement_pass)?;
    layout_convergence.record_round(
        requirement_pass.layout_round_deltas.content,
        requirement_pass.layout_round_deltas.edge,
    );
    overflow_convergence.record_round(
        requirement_pass.overflow_round_deltas.content,
        requirement_pass.overflow_round_deltas.edge,
    );

    if stop_at == Some(CoordinationCheckpoint::InitialRequirementsApplied) {
        return Ok(None);
    }

    // Retarget: frames re-solve at the written-back coordinated targets.
    // The walk derives all decisions from pre-retarget state, then applies
    // parent-first; coverage and trace alignment are by construction.
    let trace = run_retarget(measurement, eval_ctx).await?;
    debug!(
        policy = FacetCoordinationPolicy::LABEL,
        parent_cross_propagations = trace
            .node_results
            .iter()
            .filter(|result| result.parent_cross_size_propagated)
            .count(),
        cross_size_changes = trace
            .node_results
            .iter()
            .filter(|result| {
                (result.subplot_cross_size_after - result.subplot_cross_size_before).abs() > 0.01
            })
            .count(),
        band_layout_nodes = trace
            .node_results
            .iter()
            .filter(|result| result.band_layout_applied)
            .count(),
        plot_area_retarget_nodes = trace
            .node_results
            .iter()
            .filter(|result| result.plot_area_retarget_count > 0)
            .count(),
        width_retarget_count = trace
            .node_results
            .iter()
            .map(|result| result.width_retarget_count)
            .sum::<usize>(),
        height_retarget_count = trace
            .node_results
            .iter()
            .map(|result| result.height_retarget_count)
            .sum::<usize>(),
        "coordinate_facet_measurement_tree retarget complete"
    );
    let retarget_trace = trace;
    if stop_at == Some(CoordinationCheckpoint::RetargetComplete)
        || stop_at == Some(CoordinationCheckpoint::RetargetedRequirementsApplied)
    {
        return Ok(None);
    }

    debug!(
        policy = FacetCoordinationPolicy::LABEL,
        layout_rounds = layout_convergence.rounds().len(),
        layout_converged = layout_convergence.is_converged(0.01),
        layout_non_converging = layout_convergence.is_non_converging(0.01),
        overflow_converged = overflow_convergence.is_converged(0.01),
        overflow_non_converging = overflow_convergence.is_non_converging(0.01),
        "coordinate_facet_measurement_tree rounds complete"
    );

    let final_propagation_trace = run_final_propagation(measurement, eval_ctx)?;
    debug!(
        policy = FacetCoordinationPolicy::LABEL,
        scale_range_retargets = final_propagation_trace
            .node_results
            .iter()
            .map(|result| result.scale_range_retarget_count)
            .sum::<usize>(),
        plot_area_adjustments = final_propagation_trace
            .node_results
            .iter()
            .map(|result| result.child_plot_area_adjustments_count)
            .sum::<usize>(),
        "coordinate_facet_measurement_tree final propagation complete"
    );

    // Env-gated shadow diagnostics: re-solve the tree from the settled
    // state and report slot-vs-live geometry deltas plus the idempotence
    // and snapshot-identity probes; behavior-neutral.
    if crate::facet::tree_solve::shadow_enabled() {
        crate::facet::tree_solve::run_shadow_census(
            measurement,
            eval_ctx.facet_runtime_sizing_mode(),
            &shadow_snapshot_hashes,
        );
    }

    // The retargeted artifact IS the initial pass: snapshots are
    // input-identical across the run (they read only epoch-frozen and
    // construction-time values).
    let retargeted_requirement_pass = requirement_pass.clone();
    Ok(Some(CoordinationRunArtifacts {
        initial_requirement_pass: requirement_pass,
        retarget_trace,
        retargeted_requirement_pass,
        final_propagation_trace,
    }))
}

pub(crate) fn collect_requirement_snapshot(
    measurement: &ComponentsMeasurement,
) -> RequirementSnapshot {
    let mut nodes = Vec::new();
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, depth, facet_band| {
            let facet_band = facet_band.base();
            let (first_edge_index, last_edge_index) = facet_band_edge_indices(facet_band);
            nodes.push(RequirementNodeSnapshot {
                node_id: node_id.clone(),
                key: facet_band.coordination_scope_key_for_depth(depth),
                axis: facet_band.axis,
                slot_sharing: facet_band.slot_sharing,
                min_slot_count: facet_band.min_slot_count,
                overflow_cells: facet_band.overflow_cell_envelopes(),
                local_layout: facet_band.local_layout_value(),
                guide_padding_inner_px: facet_band.guide_padding_inner_px_value(),
                first_edge_index,
                last_edge_index,
            });
        },
    );
    RequirementSnapshot { nodes }
}

fn facet_band_edge_indices(
    facet_band: &crate::facet::coord::FacetBandCoordMeasurement,
) -> (usize, usize) {
    let renderable_cells = facet_band
        .cells
        .iter()
        .map(|cell| {
            renderable_for_empty_policy(facet_band.empty_cell_policy, !cell.plan.has_data_rows)
        })
        .collect::<Vec<_>>();
    effective_edge_indices(&renderable_cells, facet_band.cells.len()).unwrap_or((0, 0))
}

#[cfg(test)]
fn measurement_node_ids(measurement: &ComponentsMeasurement) -> Vec<CoordinationNodeKey> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        coords::{CoordinatedLayout, CoordinatedOverflow, FacetAxis},
        facet::evaluated_facet_tree::EvaluatedFacetTree,
        layout::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode},
        plot::compiled::{
            ComponentsMeasurement, CoordinationScopeKey, scale_provider::DynamicScaleProvider,
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
        key: CoordinationScopeKey,
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

    fn assert_internal_error_contains(error: AvengerChartError, expected: &str) {
        match error {
            AvengerChartError::InternalError(message) => assert!(
                message.contains(expected),
                "expected internal error containing '{expected}', got '{message}'"
            ),
            other => panic!("expected internal error containing '{expected}', got {other}"),
        }
    }

    fn depth1_states(measurement: &ComponentsMeasurement) -> Vec<Depth1State> {
        let mut states = Vec::new();
        visit_facet_bands(measurement, 0, &mut |depth, facet_band| {
            if depth == 1 {
                states.push(Depth1State {
                    key: facet_band.coordination_scope_key_for_depth(depth),
                    local_layout: facet_band.local_layout.clone(),
                    coordinated_layout: facet_band.coordinated_layout_value(),
                    coordinated_overflow: facet_band.active_overflow().clone(),
                });
            }
        });
        states
    }

    fn force_root_top_legend_slab(measurement: &mut ComponentsMeasurement, slab: f32) {
        let root = facet_band_mut(measurement)
            .expect("fixture should produce root facet-band measurement");
        root.force_coordinated_overflow_for_tests(|overflow| {
            overflow.total.top = overflow.guide.top + slab;
        });
    }

    fn build_nested_plot(df: DataFrame) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(640, 420)
            .mark(
                Subplot::new(
                    Plot::<FacetRow>::new().mark(
                        Subplot::new(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x(col("x"))
                                    .y(col("y"))
                                    .fill("#4682b4")
                                    .size(42.0),
                            ),
                        )
                        .row_with(col("inner_group"), |c| c.guide(|g| g.title("Inner Group"))),
                    ),
                )
                .col_with(col("outer_group"), |c| c.guide(|g| g.title("Outer Group"))),
            )
    }

    fn build_col_col_nested_plot(df: DataFrame) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(640, 420)
            .mark(
                Subplot::new(
                    Plot::<FacetColumn>::new().mark(
                        Subplot::new(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x(col("x"))
                                    .y(col("y"))
                                    .fill("#4682b4")
                                    .size(42.0),
                            ),
                        )
                        .col_with(col("inner_group"), |c| c.guide(|g| g.title("Inner Group"))),
                    ),
                )
                .col_with(col("outer_group"), |c| c.guide(|g| g.title("Outer Group"))),
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
        )
        .with_facet_runtime_sizing_mode(canvas_sizing_mode(640.0, 420.0));

        let theme = compiled_plot.get_theme();
        let scale_builder = build_scale_builder_from_marks(
            &compiled_plot.marks,
            &compiled_plot.scale_specs,
            &compiled_plot.coord_transform,
            &compiled_plot.data,
            None,
            &eval_ctx,
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
        )
        .with_facet_runtime_sizing_mode(canvas_sizing_mode(640.0, 420.0));

        let theme = compiled_plot.get_theme();
        let scale_builder = build_scale_builder_from_marks(
            &compiled_plot.marks,
            &compiled_plot.scale_specs,
            &compiled_plot.coord_transform,
            &compiled_plot.data,
            None,
            &eval_ctx,
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
            build_requirement_pass(collect_requirement_snapshot(&measurement))?;
        apply_requirement_pass(&mut measurement, &initial_requirement_pass)?;

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
    async fn retarget_trace_can_mutate_measurements_via_retarget_actions()
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

        let _retarget_trace = run_retarget(&mut measurement, &eval_ctx).await?;

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
    async fn retargeted_requirement_pass_reconciles_layout_after_retarget_trace()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;

        let initial_requirement_pass =
            build_requirement_pass(collect_requirement_snapshot(&measurement))?;
        apply_requirement_pass(&mut measurement, &initial_requirement_pass)?;
        let _retarget_trace = run_retarget(&mut measurement, &eval_ctx).await?;

        let mut bumped_one = false;
        visit_facet_bands_mut(&mut measurement, 0, &mut |depth, facet_band| {
            if depth == 1 {
                facet_band.clear_coordinated_layout_for_tests();
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
        let local_slot_counts = expected_states
            .iter()
            .map(|state| state.local_layout.n)
            .collect::<Vec<_>>();
        assert_ne!(
            local_slot_counts.iter().max(),
            local_slot_counts.iter().min(),
            "perturbation should make depth-1 local slot counts diverge"
        );

        let retargeted_requirement_pass =
            build_requirement_pass(collect_requirement_snapshot(&measurement))?;
        apply_requirement_pass(&mut measurement, &retargeted_requirement_pass)?;

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
            // Chrome and spacing still coordinate to the group maximum.
            assert!(approx_eq_within(
                coordinated.padding_inner_px,
                expected_padding,
                0.01
            ));
            // The fixture's bands use the default FREE slot sharing, which
            // keeps slot counts local instead of adopting the group maximum:
            // free nested facets must not reserve hidden slots.
            assert_eq!(coordinated.n, state.local_layout.n);
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
        let _final_propagation_trace = apply_final_propagation_decisions(
            &mut measurement,
            &eval_ctx,
            &final_propagation_plan,
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
            build_requirement_pass(collect_requirement_snapshot(&measurement))?;
        assert!(!initial_requirement_pass.snapshot.nodes.is_empty());
        assert!(initial_requirement_pass.diagnostics.layout_groups > 0);
        assert_eq!(
            initial_requirement_pass.solution.layout_by_node.len(),
            initial_requirement_pass.snapshot.nodes.len()
        );
        assert!(
            initial_requirement_pass.solution.overflow_by_node.len()
                <= initial_requirement_pass.snapshot.nodes.len()
        );
        Ok(())
    }

    #[tokio::test]
    async fn retarget_trace_records_cell_retarget_and_parent_cross_propagation()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = col_col_fixture().await?;
        let initial_requirement_pass =
            build_requirement_pass(collect_requirement_snapshot(&measurement))?;
        apply_requirement_pass(&mut measurement, &initial_requirement_pass)?;
        force_root_top_legend_slab(&mut measurement, 18.0);

        let retarget_trace = run_retarget(&mut measurement, &eval_ctx).await?;
        assert!(!retarget_trace.node_results.is_empty());
        let root_result = retarget_trace
            .node_results
            .iter()
            .find(|result| result.node_id.path.is_empty())
            .expect("retarget should include a root node trace");
        assert!(root_result.parent_cross_size_propagated);
        assert!(
            root_result
                .planned_child_action_counts
                .plot_area_retarget_count()
                > 0
        );
        assert!(root_result.plot_area_retarget_count > 0);
        Ok(())
    }

    #[tokio::test]
    async fn retargeted_requirement_pass_distribution_stabilizes_layout_values()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        let initial_requirement_pass =
            build_requirement_pass(collect_requirement_snapshot(&measurement))?;
        apply_requirement_pass(&mut measurement, &initial_requirement_pass)?;
        let _retarget_trace = run_retarget(&mut measurement, &eval_ctx).await?;

        let retargeted_requirement_pass =
            build_requirement_pass(collect_requirement_snapshot(&measurement))?;
        assert!(
            !retargeted_requirement_pass
                .solution
                .layout_by_node
                .is_empty()
        );
        apply_requirement_pass(&mut measurement, &retargeted_requirement_pass)?;

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
        let final_propagation_trace = apply_final_propagation_decisions(
            &mut measurement,
            &eval_ctx,
            &final_propagation_plan,
        )?;
        assert!(!final_propagation_trace.node_results.is_empty());
        let root_result = final_propagation_trace
            .node_results
            .iter()
            .find(|result| result.node_id.path.is_empty())
            .expect("final propagation should include a root node trace");
        let root_plan = final_propagation_plan
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
        let (measurement, _eval_ctx) = nested_fixture().await?;
        let plan = build_final_propagation_plan(&measurement);
        assert!(!plan.is_empty());
        for node in &plan {
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
    async fn final_propagation_errors_when_node_plan_missing() -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        let mut plan = build_final_propagation_plan(&measurement);
        let root_idx = plan
            .iter()
            .position(|node| node.node_id.path.is_empty())
            .expect("final propagation plan should include a root node");
        plan.remove(root_idx);

        let error = apply_final_propagation_decisions(&mut measurement, &eval_ctx, &plan)
            .expect_err("missing final propagation node plan should error");
        assert_internal_error_contains(error, "Missing final propagation plan");
        Ok(())
    }

    #[tokio::test]
    async fn final_propagation_errors_when_child_plan_count_mismatches()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        let mut plan = build_final_propagation_plan(&measurement);
        let root_plan = plan
            .iter_mut()
            .find(|node| node.node_id.path.is_empty())
            .expect("final propagation plan should include a root node");
        assert!(
            root_plan.child_plans.pop().is_some(),
            "fixture root should have at least one child plan"
        );

        let error = apply_final_propagation_decisions(&mut measurement, &eval_ctx, &plan)
            .expect_err("final propagation child plan mismatch should error");
        assert_internal_error_contains(error, "Final propagation plan child count mismatch");
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
            apply_final_propagation_decisions(&mut measurement, &eval_ctx, &plan)?;

        for (planned, trace_result) in plan.iter().zip(final_propagation_trace.node_results.iter())
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
        let (measurement, _eval_ctx) = nested_fixture().await?;
        let plan = build_final_propagation_plan(&measurement);
        for node in &plan {
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
        let (measurement, eval_ctx) = nested_fixture().await?;
        let before_node_ids = measurement_node_ids(&measurement);
        let before_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;

        let decisions = derive_retarget_decisions(&measurement, &eval_ctx)?;

        let after_node_ids = measurement_node_ids(&measurement);
        let after_cross_size = facet_band_ref(&measurement)
            .expect("fixture should produce root facet-band measurement")
            .subplot_cross_size;

        assert_eq!(before_node_ids, after_node_ids);
        assert_eq!(before_cross_size, after_cross_size);
        assert_eq!(
            decisions
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
    async fn retarget_plan_contains_requirements_and_actions_for_each_node()
    -> Result<(), AvengerChartError> {
        let (measurement, eval_ctx) = nested_fixture().await?;
        let decisions = derive_retarget_decisions(&measurement, &eval_ctx)?;
        assert!(!decisions.is_empty());
        for node in &decisions {
            assert_eq!(node.requirements.node_id, node.node_id);
            assert_eq!(node.actions.node_id, node.node_id);
            assert_eq!(node.actions.axis, node.requirements.axis);
            assert_eq!(
                node.actions.child_actions.len(),
                node.requirements.child_count
            );
            if node.requirements.has_legend_overflow {
                assert!(node.requirements.legend_main_axis_slab.has_slab());
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn retarget_errors_when_node_plan_missing() -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        let mut decisions = derive_retarget_decisions(&measurement, &eval_ctx)?;
        let root_idx = decisions
            .iter()
            .position(|node| node.node_id.path.is_empty())
            .expect("retarget decisions should include a root node");
        decisions.remove(root_idx);

        let error = apply_retarget_decisions(&mut measurement, &eval_ctx, &decisions)
            .await
            .expect_err("missing retarget node plan should error");
        assert_internal_error_contains(error, "Missing retarget plan");
        Ok(())
    }

    #[tokio::test]
    async fn retarget_errors_when_child_action_count_mismatches() -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        let mut decisions = derive_retarget_decisions(&measurement, &eval_ctx)?;
        let root_plan = decisions
            .iter_mut()
            .find(|node| node.node_id.path.is_empty())
            .expect("retarget decisions should include a root node");
        assert!(
            root_plan.actions.child_actions.pop().is_some(),
            "fixture root should have at least one child action"
        );

        let error = apply_retarget_decisions(&mut measurement, &eval_ctx, &decisions)
            .await
            .expect_err("retarget child action mismatch should error");
        assert_internal_error_contains(error, "Retarget action child count mismatch");
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
        }
        force_root_top_legend_slab(&mut measurement, 18.0);

        let retarget_trace = run_retarget(&mut measurement, &eval_ctx).await?;
        let root_result = retarget_trace
            .node_results
            .iter()
            .find(|result| result.node_id.path.is_empty())
            .expect("retarget should include a root node trace");
        assert_eq!(root_result.axis, FacetAxis::Column);
        assert!(root_result.subplot_cross_size_after > 0.0);
        assert!(
            root_result
                .planned_child_action_counts
                .plot_area_retarget_count()
                > 0
        );
        assert!(root_result.plot_area_retarget_count > 0);
        Ok(())
    }

    #[tokio::test]
    async fn retarget_trace_node_identity_stable_between_plan_and_execution()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        let initial_requirement_pass =
            build_requirement_pass(collect_requirement_snapshot(&measurement))?;
        apply_requirement_pass(&mut measurement, &initial_requirement_pass)?;

        let decisions = derive_retarget_decisions(&measurement, &eval_ctx)?;
        let retarget_trace = run_retarget(&mut measurement, &eval_ctx).await?;

        assert_eq!(
            retarget_trace
                .node_results
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>(),
            decisions
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
            plan.iter()
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
            apply_final_propagation_decisions(&mut measurement, &eval_ctx, &plan)?;

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
        // Coverage is validated at construction; the `?` exercises it.
        let initial_requirement_pass =
            build_requirement_pass(collect_requirement_snapshot(&measurement))?;
        let snapshot_nodes: std::collections::HashSet<CoordinationNodeKey> =
            initial_requirement_pass
                .snapshot
                .nodes
                .iter()
                .map(|node| node.node_id.clone())
                .collect();
        let layout_patch_nodes: std::collections::HashSet<CoordinationNodeKey> =
            initial_requirement_pass
                .solution
                .layout_by_node
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
        // Coverage is validated at construction; the `?` exercises it.
        let retargeted_requirement_pass =
            build_requirement_pass(collect_requirement_snapshot(&measurement))?;
        let snapshot_nodes: std::collections::HashSet<CoordinationNodeKey> =
            retargeted_requirement_pass
                .snapshot
                .nodes
                .iter()
                .map(|node| node.node_id.clone())
                .collect();
        let layout_patch_nodes: std::collections::HashSet<CoordinationNodeKey> =
            retargeted_requirement_pass
                .solution
                .layout_by_node
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
            build_requirement_pass(collect_requirement_snapshot(&measurement))?;
        apply_requirement_pass(&mut measurement, &initial_requirement_pass)?;

        let decisions = derive_retarget_decisions(&measurement, &eval_ctx)?;
        let retarget_trace = run_retarget(&mut measurement, &eval_ctx).await?;
        assert_eq!(
            retarget_trace
                .node_results
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>(),
            decisions
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>()
        );

        let retargeted_requirement_pass =
            build_requirement_pass(collect_requirement_snapshot(&measurement))?;
        apply_requirement_pass(&mut measurement, &retargeted_requirement_pass)?;

        let final_propagation_plan = build_final_propagation_plan(&measurement);
        let final_propagation_trace = apply_final_propagation_decisions(
            &mut measurement,
            &eval_ctx,
            &final_propagation_plan,
        )?;
        assert_eq!(
            final_propagation_trace
                .node_results
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>(),
            final_propagation_plan
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>()
        );
        for (planned, trace_result) in final_propagation_trace
            .node_results
            .iter()
            .zip(final_propagation_plan.iter())
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
