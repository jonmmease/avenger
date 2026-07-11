//! Facet coordination pipeline for aligning layout requirements.
//!
//! Coordination runs after local facet-band layout as fold → solve →
//! install → adopt: collect a requirement snapshot, solve the real facet
//! tree once for channels, install the solution artifact on every band,
//! adopt the implied geometry, then install the current settled geometry
//! for render/readback. Plot areas and scale ranges move, measured chrome
//! stays frozen within the round (the staleness law).
//!
//! Immutable coordination plans are built in `coordination_plans`; bounded
//! side effects (channel install, geometry adoption) are applied through
//! `coordination_apply`.

use avenger_chart_core::AvengerChartError;
use tracing::{debug, trace};

use crate::{
    facet::{
        coord::renderable_for_empty_policy,
        coordination_apply::{
            apply_requirement_pass, refresh_current_facet_geometry, run_adopt,
            visit_facet_bands_with_node_id,
        },
        coordination_plans::{
            CoordinationRunArtifacts, RequirementNodeSnapshot, RequirementSnapshot,
            build_requirement_pass_with_round,
        },
        layout_plan::effective_edge_indices,
    },
    plot::compiled::ComponentsMeasurement,
    render::{CoordinationCheckpoint, EvaluationContext},
};

#[cfg(test)]
use crate::facet::coord::FacetBandCoordMeasurement;
#[cfg(test)]
use crate::facet::coordination_plans::{CoordinationNodeKey, build_requirement_pass};
#[cfg(test)]
use crate::render::context::{FacetRuntimeSizingMode, FacetRuntimeSizingPolicy};

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
            visit(d, facet_band);
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
        requirement_pass_nodes = artifacts.requirement_pass.snapshot.nodes.len(),
        adopted_bands = artifacts.adopt_trace.bands,
        adopted_cells = artifacts.adopt_trace.cells_adopted,
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
/// facet tree for channels, install the channel solution on every band,
/// ADOPT the solve's allotments, then install the current settled geometry.
/// Plot areas become slots, scale ranges follow, and chrome stays frozen
/// within the round. `stop_at` maps the public checkpoints onto these
/// stages and returns `None` when the run stops early.
async fn run_facet_coordination(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    stop_at: Option<CoordinationCheckpoint>,
) -> Result<Option<CoordinationRunArtifacts>, AvengerChartError> {
    let snapshot = collect_requirement_snapshot(measurement);
    // ONE solve: real topology, pre-folded chart scalars, per-node
    // Region.coordinated channel reads.
    let sizing = eval_ctx.facet_runtime_sizing_mode();
    let solved = crate::facet::tree_solve::tree_solved_round(measurement, sizing)?;
    let requirement_pass = build_requirement_pass_with_round(snapshot, solved)?;
    debug!(
        overflow_groups = requirement_pass.diagnostics.overflow_groups,
        boundary_overflow_groups = requirement_pass.diagnostics.boundary_overflow_groups,
        layout_groups = requirement_pass.diagnostics.layout_groups,
        layout_round_delta = requirement_pass.layout_round_deltas.content
            + requirement_pass.layout_round_deltas.edge,
        overflow_round_delta = requirement_pass.overflow_round_deltas.content
            + requirement_pass.overflow_round_deltas.edge,
        "coordinate_facet_measurement_tree solve + distribution"
    );
    apply_requirement_pass(measurement, &requirement_pass)?;

    if stop_at == Some(CoordinationCheckpoint::ChannelsInstalled) {
        return Ok(None);
    }

    // Adopt coordinated channel values into scales/plot areas, then solve the
    // settled tree used by render/readback.
    let adopt_trace = run_adopt(measurement, eval_ctx)?;
    refresh_current_facet_geometry(measurement, sizing)?;
    debug!(
        bands = adopt_trace.bands,
        cells_adopted = adopt_trace.cells_adopted,
        cells_unchanged = adopt_trace.cells_unchanged,
        "coordinate_facet_measurement_tree adopt complete"
    );
    if stop_at == Some(CoordinationCheckpoint::Adopted) {
        return Ok(None);
    }

    // Env-gated shadow diagnostics: re-solve the tree from the settled
    // state and report slot-vs-live geometry deltas plus the idempotence,
    // adoption, and content-driven geometry probes; behavior-neutral.
    if crate::facet::tree_solve::shadow_enabled() {
        crate::facet::tree_solve::run_shadow_census(
            measurement,
            eval_ctx.facet_runtime_sizing_mode(),
        );
    }

    Ok(Some(CoordinationRunArtifacts {
        requirement_pass,
        adopt_trace,
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
mod tests {
    use super::*;
    use crate::{
        coords::{CoordinatedLayout, CoordinatedOverflow},
        facet::{
            coord::{
                retarget_measurement_plot_area_no_remeasure,
                retarget_measurement_plot_area_policy_no_remeasure,
            },
            evaluated_facet_tree::EvaluatedFacetTree,
        },
        layout::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode},
        plot::CompiledPlot,
        plot::compiled::{
            ComponentsMeasurement, CoordinationScopeKey, scale_provider::DynamicScaleProvider,
            scales::build_scale_builder_from_compiled_plot,
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
                    key: facet_band.coordination_scope_key_for_depth(depth),
                    coordinated_layout: facet_band.coordinated_layout_value(),
                    coordinated_overflow: facet_band.active_overflow().clone(),
                });
            }
        });
        states
    }

    fn build_nested_plot(df: DataFrame) -> crate::plot::Chart<FacetColumn> {
        crate::plot::Chart::<FacetColumn>::new()
            .data(df)
            .canvas_size(640, 420)
            .mark(
                Subplot::new(
                    crate::plot::Plot::<FacetRow>::new().mark(
                        Subplot::new(
                            crate::plot::Plot::<Cartesian>::new().mark(
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

    async fn nested_compiled_fixture()
    -> Result<(Arc<CompiledPlot>, ComponentsMeasurement, EvaluationContext), AvengerChartError>
    {
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
        let compiled_plot = Arc::new(plot.compile(&session).await?);

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
        let scale_builder =
            build_scale_builder_from_compiled_plot(&compiled_plot, None, &eval_ctx, theme.as_ref())
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

        Ok((compiled_plot, measurement, eval_ctx))
    }

    async fn nested_fixture()
    -> Result<(ComponentsMeasurement, EvaluationContext), AvengerChartError> {
        let (_, measurement, eval_ctx) = nested_compiled_fixture().await?;
        Ok((measurement, eval_ctx))
    }

    fn root_current_geometry_size(
        measurement: &ComponentsMeasurement,
    ) -> Option<avenger_layout::Size> {
        let mut size = None;
        visit_facet_bands(measurement, 0, &mut |depth, facet_band| {
            if depth == 0
                && size.is_none()
                && let Some((geometry, _)) = facet_band.current_geometry_handle()
            {
                size = Some(geometry.solution.size);
            }
        });
        size
    }

    fn assert_current_geometry_installed_for_real_cells(measurement: &ComponentsMeasurement) {
        let mut band_count = 0usize;
        let mut nested_band_count = 0usize;
        visit_facet_bands(measurement, 0, &mut |depth, facet_band| {
            band_count += 1;
            if depth > 0 {
                nested_band_count += 1;
            }
            assert!(
                facet_band.current_geometry_handle().is_some(),
                "facet band at depth {depth} should have current geometry"
            );
            let (content_size, regions) = facet_band
                .current_child_frame_regions()
                .expect("current geometry should provide child-frame regions");
            assert!(content_size.width >= 0.0);
            assert!(content_size.height >= 0.0);
            assert_eq!(
                regions.len(),
                facet_band.cells.len(),
                "current geometry should expose real child cells only"
            );
            let mut child_indices = regions
                .iter()
                .map(|region| region.child_index)
                .collect::<Vec<_>>();
            child_indices.sort_unstable();
            assert_eq!(
                child_indices,
                (0..facet_band.cells.len()).collect::<Vec<_>>(),
                "current geometry child regions should be one-to-one with real cells"
            );
        });
        assert!(band_count > 0, "fixture should contain facet bands");
        assert!(
            nested_band_count > 0,
            "fixture should contain nested facet bands"
        );
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
    async fn current_geometry_installs_on_nested_bands_and_excludes_ghost_slots()
    -> Result<(), AvengerChartError> {
        let (mut measurement, eval_ctx) = nested_fixture().await?;
        coordinate_facet_measurement_tree(&mut measurement, &eval_ctx).await?;

        assert_current_geometry_installed_for_real_cells(&measurement);
        Ok(())
    }

    #[tokio::test]
    async fn standard_retarget_refreshes_current_facet_geometry() -> Result<(), AvengerChartError> {
        let (compiled_plot, mut measurement, eval_ctx) = nested_compiled_fixture().await?;
        coordinate_facet_measurement_tree(&mut measurement, &eval_ctx).await?;
        let before = root_current_geometry_size(&measurement)
            .expect("coordination should install root current geometry");
        let new_width = (measurement.plot_area_width * 0.72).max(1.0);
        let new_height = (measurement.plot_area_height * 0.81).max(1.0);

        retarget_measurement_plot_area_no_remeasure(
            &mut measurement,
            compiled_plot.as_ref(),
            &eval_ctx,
            &[],
            new_width,
            new_height,
        )?;

        let after = root_current_geometry_size(&measurement)
            .expect("retarget should refresh root current geometry");
        assert!(
            (before.width - after.width).abs() > 0.01
                || (before.height - after.height).abs() > 0.01,
            "retarget should update installed current geometry"
        );
        assert_current_geometry_installed_for_real_cells(&measurement);
        Ok(())
    }

    #[tokio::test]
    async fn policy_retarget_refreshes_current_facet_geometry() -> Result<(), AvengerChartError> {
        let (compiled_plot, mut measurement, eval_ctx) = nested_compiled_fixture().await?;
        coordinate_facet_measurement_tree(&mut measurement, &eval_ctx).await?;
        let before = root_current_geometry_size(&measurement)
            .expect("coordination should install root current geometry");
        let new_width = (measurement.plot_area_width * 0.68).max(1.0);
        let new_height = (measurement.plot_area_height * 0.77).max(1.0);

        retarget_measurement_plot_area_policy_no_remeasure(
            &mut measurement,
            compiled_plot.as_ref(),
            &eval_ctx,
            &[],
            new_width,
            new_height,
        )?;

        let after = root_current_geometry_size(&measurement)
            .expect("policy retarget should refresh root current geometry");
        assert!(
            (before.width - after.width).abs() > 0.01
                || (before.height - after.height).abs() > 0.01,
            "policy retarget should update installed current geometry"
        );
        assert_current_geometry_installed_for_real_cells(&measurement);
        Ok(())
    }
}
