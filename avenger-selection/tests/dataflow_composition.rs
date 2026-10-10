mod common;
#[path = "../examples/support/composition.rs"]
mod composition;
use avenger_datafusion_dataflow::{
    CachePolicy, DataflowBuilder, Runtime, RuntimeConfig, TableSnapshot,
};
use avenger_selection::*;
use common::*;
use composition::{ExampleResult, OptimizedQuery};
use datafusion::{
    arrow::{array::Float32Array, datatypes::DataType},
    functions_aggregate::expr_fn::count,
    logical_expr::{col, lit, LogicalPlan, LogicalPlanBuilder},
};
use std::sync::Arc;

fn target(rows: LogicalPlan) -> datafusion::common::Result<LogicalPlan> {
    LogicalPlanBuilder::from(rows)
        .aggregate(vec![col("carrier")], vec![count(lit(1_i64)).alias("n")])?
        .build()
}

#[tokio::test]
async fn drag_reuses_states_fixed_changes_replace_preparation_and_source_changes_invalidate(
) -> ExampleResult<()> {
    let focus = interval("focus", "delay");
    let other = point("other", "carrier");
    let inactive = state(Resolution::Intersect);
    for cache in [CachePolicy::default(), CachePolicy::Disabled] {
        let cached = !matches!(cache, CachePolicy::Disabled);
        let mut builder = DataflowBuilder::new();
        let table = TableSnapshot::from_batches(flights().schema(), vec![flights()])?;
        let input = builder.table_input("flights", table.schema().clone())?;
        let source = builder.add_plan("source", input.plan_ref())?;
        let source_output = builder.table_output("source", &source)?;
        let full = builder.expr_input("full", DataType::Boolean)?;
        let direct = builder.add_plan(
            "direct",
            target(
                LogicalPlanBuilder::from(source.plan_ref())
                    .filter(full.expr_ref())?
                    .build()?,
            )?,
        )?;
        let direct = builder.table_output("direct", &direct)?;
        let base = Runtime::new(RuntimeConfig {
            cache,
            ..Default::default()
        })?
        .prepare(&builder.finish()?)
        .await?;
        let initial = membership().predicates(&inactive, &focus)?;
        let mut additional = DataflowBuilder::with_base(&base.interface());
        let imported = additional.import_table("flights", &source_output)?;
        let fallback = additional.import_table("direct", &direct)?;
        let fallback = additional.table_output("direct", &fallback)?;
        let optimized = OptimizedQuery::install(
            &mut additional,
            "airlines",
            composition::prepare(imported.plan_ref(), initial.split().unwrap(), target)?,
            initial.split().unwrap(),
        )?
        .unwrap();
        let extension = base.prepare_extension(&additional.finish()?).await?;
        let base_inputs = |state: &SelectionSet, table: TableSnapshot| {
            base.inputs()
                .table(&input, table)?
                .expr(&full, membership().predicate(state).unwrap())?
                .finish()
        };
        let warm_binding = optimized.bind(initial.split().unwrap())?.unwrap();
        let warm_inputs = warm_binding.apply(extension.inputs())?.finish()?;
        let warm = extension
            .query(
                &[warm_binding.materialization_output().unwrap()],
                &[],
                &base_inputs(&inactive, table.clone())?,
                &warm_inputs,
            )
            .await?;
        assert!(warm
            .report()
            .executed_nodes
            .iter()
            .any(|n| n.ends_with("airlines_states")));
        let mut observed = Vec::new();
        for (lo, hi) in [(10, 30), (20, 40), (20, 40)] {
            let state = inactive.set(&focus, between("delay", lo, hi))?;
            let p = membership().predicates(&state, &focus)?;
            let binding = optimized.bind(p.split().unwrap())?.unwrap();
            let inputs = binding.apply(extension.inputs())?.finish()?;
            let result = extension
                .query(
                    &[binding.output()],
                    &[],
                    &base_inputs(&state, table.clone())?,
                    &inputs,
                )
                .await?;
            if cached {
                assert!(!result
                    .report()
                    .executed_nodes
                    .iter()
                    .any(|n| n.ends_with("airlines_states")));
                assert!(result.report().cache_hits > 0);
                if observed.len() == 2 {
                    assert!(result.report().executed_nodes.is_empty());
                }
            }
            observed.push((state, result.table(&binding.output())?.clone()));
        }
        // Reference requests follow the measured sequence so they cannot warm it.
        for (state, expected) in &observed {
            let reference = base
                .query(&[direct], &[], &base_inputs(state, table.clone())?)
                .await?;
            assert_results(expected.batches(), reference.table(&direct)?.batches(), &[]);
        }
        let fixed_changed = observed[1]
            .0
            .set(&other, values("carrier", ["AA".into()]))?;
        let changed = membership().predicates(&fixed_changed, &focus)?;
        assert!(optimized.bind(changed.split().unwrap())?.is_none());
        let neutral = optimized
            .query
            .bind(lit(true))?
            .apply(extension.inputs())?
            .finish()?;
        let direct_result = extension
            .query(
                &[fallback],
                &[],
                &base_inputs(&fixed_changed, table.clone())?,
                &neutral,
            )
            .await?;

        let mut replacement = DataflowBuilder::with_base(&base.interface());
        let imported = replacement.import_table("flights", &source_output)?;
        let next = OptimizedQuery::install(
            &mut replacement,
            "airlines",
            composition::prepare(imported.plan_ref(), changed.split().unwrap(), target)?,
            changed.split().unwrap(),
        )?
        .unwrap();
        let replacement = base.prepare_extension(&replacement.finish()?).await?;
        let binding = next.bind(changed.split().unwrap())?.unwrap();
        let inputs = binding.apply(replacement.inputs())?.finish()?;
        let result = replacement
            .query(
                &[binding.output()],
                &[],
                &base_inputs(&fixed_changed, table.clone())?,
                &inputs,
            )
            .await?;
        assert_results(
            result.table(&binding.output())?.batches(),
            direct_result.table(&fallback)?.batches(),
            &[],
        );
        assert!(result
            .report()
            .executed_nodes
            .iter()
            .any(|n| n.ends_with("airlines_states")));
        if cached {
            assert!(!result
                .report()
                .executed_nodes
                .iter()
                .any(|n| n == "base::source"));
        }
        let refreshed =
            TableSnapshot::from_batches(flights().schema(), vec![flights().slice(0, 2)])?;
        let result = replacement
            .query(
                &[binding.output()],
                &[],
                &base_inputs(&fixed_changed, refreshed.clone())?,
                &inputs,
            )
            .await?;
        let expected = base
            .query(&[direct], &[], &base_inputs(&fixed_changed, refreshed)?)
            .await?;
        assert_results(
            result.table(&binding.output())?.batches(),
            expected.table(&direct)?.batches(),
            &[],
        );
        assert!(result
            .report()
            .executed_nodes
            .iter()
            .any(|n| n.ends_with("airlines_states")));
    }
    Ok(())
}

#[tokio::test]
async fn changed_grid_or_union_uses_the_complete_current_direct_predicate() -> ExampleResult<()> {
    let focus = interval("focus", "delay");
    let grid = |size| {
        PixelGrid::new(
            avenger_scales_datafusion::BuiltinScale::Linear,
            Arc::new(Float32Array::from(vec![0., 40.])),
            Arc::new(Float32Array::from(vec![0., 40.])),
            Default::default(),
            0.,
            size,
        )
    };
    let focus = focus.with_pixel_grids([(ProjectionId::new("delay")?, grid(10.)?)])?;
    let other = point("other", "carrier");
    let initial = state(Resolution::Intersect).set(&other, values("carrier", ["AA".into()]))?;
    let context = datafusion::prelude::SessionContext::new();
    let source = context.read_batch(flights())?.into_unoptimized_plan();
    let mut b = DataflowBuilder::new();
    let full = b.expr_input("full", DataType::Boolean)?;
    let direct = b.add_plan(
        "direct",
        target(
            LogicalPlanBuilder::from(source.clone())
                .filter(full.expr_ref())?
                .build()?,
        )?,
    )?;
    let direct = b.table_output("direct", &direct)?;
    let p = membership().predicates(&initial, &focus)?;
    let optimized = OptimizedQuery::install(
        &mut b,
        "airlines",
        composition::prepare(source.clone(), p.split().unwrap(), target)?,
        p.split().unwrap(),
    )?
    .unwrap();
    let flow = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let changed = focus.with_pixel_grids([(ProjectionId::new("delay")?, grid(5.)?)])?;
    let grid_state = initial.set(&changed, between("delay", 10, 30))?;
    let union = state(Resolution::Union).apply_all([
        SelectionUpdate::set(&focus, between("delay", 10, 30)),
        SelectionUpdate::set(&other, values("carrier", ["AA".into()])),
    ])?;
    for state in [&grid_state, &union] {
        let p = membership().predicates(state, &focus)?;
        assert!(p.split().is_err());
        let inputs = optimized
            .query
            .bind(lit(true))?
            .apply(flow.inputs().expr(&full, p.full().clone())?)?
            .finish()?;
        let result = flow.query(&[direct], &[], &inputs).await?;
        assert!(!result
            .report()
            .executed_nodes
            .iter()
            .any(|n| n.ends_with("states")));
        let expected = context
            .execute_logical_plan(target(
                LogicalPlanBuilder::from(source.clone())
                    .filter(p.full().clone())?
                    .build()?,
            )?)
            .await?
            .collect()
            .await?;
        assert_results(result.table(&direct)?.batches(), &expected, &[]);
    }
    Ok(())
}
