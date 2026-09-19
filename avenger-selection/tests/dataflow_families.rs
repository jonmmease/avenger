#![cfg(feature = "dataflow")]
mod common;

use avenger_datafusion_dataflow::{
    CachePolicy, DataflowBuilder, Error as FlowError, Runtime, RuntimeConfig, TableSnapshot,
};
use avenger_selection::*;
use common::*;
use datafusion::{
    arrow::{datatypes::DataType, record_batch::RecordBatch},
    common::ScalarValue,
    functions_aggregate::expr_fn::count,
    logical_expr::{col, lit, LogicalPlanBuilder},
    prelude::SessionContext,
};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
fn rows(batches: &[RecordBatch]) -> Vec<Vec<ScalarValue>> {
    let mut values = Vec::new();
    for batch in batches {
        for row in 0..batch.num_rows() {
            values.push(
                batch
                    .columns()
                    .iter()
                    .map(|a| ScalarValue::try_from_array(a, row).unwrap())
                    .collect(),
            );
        }
    }
    values
}

#[tokio::test]
async fn installed_families_rebind_in_one_extension_with_transparent_policy_and_caching(
) -> TestResult {
    let delay = interval("delay_hist", "delay");
    let distance = interval("distance_hist", "distance");
    let airline = point("airlines", "carrier");
    let inactive = state(Resolution::Intersect);
    let selected = inactive.apply_all([
        (id(), SelectionUpdate::set(&delay, between("delay", 10, 30))),
        (
            id(),
            SelectionUpdate::set(&distance, between("distance", 500, 1500)),
        ),
        (
            id(),
            SelectionUpdate::set(&airline, values("carrier", ["AA".into(), "DL".into()])),
        ),
    ])?;
    let moved = selected.apply(
        &id(),
        SelectionUpdate::set(&delay, between("delay", 20, 40)),
    )?;
    let filters = [&delay, &distance, &airline].map(|p| cross(p.address().origin.clone()));
    for cache in [CachePolicy::default(), CachePolicy::Disabled] {
        let cached = !matches!(cache, CachePolicy::Disabled);
        let runtime = Runtime::new(RuntimeConfig {
            cache,
            ..Default::default()
        })?;
        let snapshot = TableSnapshot::from_batches(flights().schema(), vec![flights()])?;
        let mut builder = DataflowBuilder::new();
        let source = builder.table_input("flights", snapshot.schema().clone())?;
        let node = builder.add_plan("source", source.plan_ref())?;
        let source_output = builder.table_output("source", &node)?;
        let base = runtime.prepare(&builder.finish()?).await?;
        let base_inputs = base.inputs().table(&source, snapshot)?.finish()?;
        let mut additional = DataflowBuilder::with_base(&base.interface());
        let imported = additional.import_table("flights", &source_output)?;
        let unrelated = additional.scalar_input("unrelated", DataType::Int64)?;
        let mut panels = Vec::new();
        let mut recipes = Vec::new();
        let source_native = SessionContext::new()
            .read_batch(flights())?
            .into_unoptimized_plan();
        for (filter, group) in filters.iter().zip(["delay", "distance", "carrier"]) {
            let recipe = |source| {
                filter.query(source, |rows| {
                    LogicalPlanBuilder::from(rows)
                        .aggregate(vec![col(group)], vec![count(lit(1_i64)).alias("count")])?
                        .sort(vec![col(group).sort(true, true)])?
                        .build()
                })
            };
            let family = recipe(imported.plan_ref())?
                .plan(&inactive)
                .focus(&delay)
                .build()?;
            panels.push(family.install(&mut additional, group)?);
            recipes.push(recipe(source_native.clone())?);
        }
        let definition = additional.finish()?;
        assert_eq!(definition.num_inputs(), 8); // Two optimized targets, one exempt target, and caller input.
        assert_eq!(definition.num_outputs(), 7);
        let extension = base.prepare_extension(&definition).await?;
        let mut saved = None;
        for (step, s) in [&inactive, &selected, &moved, &inactive]
            .into_iter()
            .enumerate()
        {
            let mut prior_tables = None;
            for policy in [QueryPolicy::Auto, QueryPolicy::ForceDirect] {
                let bindings = panels
                    .iter()
                    .map(|p| p.bind_with_policy(s, policy))
                    .collect::<Result<Vec<_>>>()?;
                let outputs: Vec<_> = bindings.iter().map(|b| b.output()).collect();
                let mut inputs = extension.inputs();
                for (index, binding) in bindings.iter().enumerate() {
                    let optimized = policy == QueryPolicy::Auto && index != 0;
                    assert_eq!(
                        binding.explain().strategy,
                        if optimized {
                            QueryStrategy::Preaggregated
                        } else {
                            QueryStrategy::Direct
                        }
                    );
                    assert_eq!(
                        binding.explain().direct_reason,
                        if optimized {
                            None
                        } else {
                            Some(if policy == QueryPolicy::Auto {
                                DirectReason::FocusNotUsed
                            } else {
                                DirectReason::Forced
                            })
                        }
                    );
                    assert_eq!(binding.preaggregate_output().is_some(), optimized);
                    inputs = binding.apply(inputs)?;
                }
                // Installation owns its predicates, not every input in the extension.
                assert!(matches!(
                    bindings[0].apply(extension.inputs())?.finish(),
                    Err(FlowError::MissingInput(_))
                ));
                let inputs = inputs.scalar(&unrelated, 42_i64.into())?.finish()?;
                let result = extension
                    .query(&outputs, &[], &base_inputs, &inputs)
                    .await?;
                let mut tables = Vec::new();
                for (recipe, output) in recipes.iter().zip(&outputs) {
                    let direct = SessionContext::new()
                        .execute_logical_plan(recipe.logical_plan(s)?)
                        .await?
                        .collect()
                        .await?;
                    let actual = result.table(output)?;
                    if let Some(batch) = direct.first() {
                        assert_eq!(actual.schema().as_ref(), batch.schema().as_ref());
                    }
                    assert_eq!(rows(actual.batches()), rows(&direct));
                    tables.push(rows(actual.batches()));
                }
                if let Some(prior) = &prior_tables {
                    assert_eq!(&tables, prior);
                }
                prior_tables = Some(tables);
                if cached && step == 2 && policy == QueryPolicy::Auto {
                    let mut executed = result.report().executed_nodes.clone();
                    executed.sort();
                    assert_eq!(
                        executed,
                        vec![
                            "additional::carrier__aggregate",
                            "additional::distance__aggregate"
                        ],
                        "moving only focus reuses both materializations and the exempt output"
                    );
                }
                if step == 1 && policy == QueryPolicy::Auto {
                    saved = Some((bindings, prior_tables.clone().unwrap()));
                }
            }
        }
        let (bindings, expected) = saved.unwrap();
        let mut inputs = extension.inputs().scalar(&unrelated, 42_i64.into())?;
        for binding in &bindings {
            inputs = binding.apply(inputs)?;
        }
        let outputs: Vec<_> = bindings.iter().map(|b| b.output()).collect();
        let result = extension
            .query(&outputs, &[], &base_inputs, &inputs.finish()?)
            .await?;
        for (out, expected) in outputs.iter().zip(expected) {
            assert_eq!(rows(result.table(out)?.batches()), expected);
        }
    }
    Ok(())
}

#[tokio::test]
async fn root_installation_checks_usage_contexts_handles_and_name_collisions() -> TestResult {
    let mut graph = DataflowBuilder::new();
    let source = graph.table_snapshot(
        "flights",
        TableSnapshot::from_batches(flights().schema(), vec![flights()])?,
    )?;
    let s = state(Resolution::Union);
    let query = membership().query(source.plan_ref(), Ok)?;
    let family = query.plan(&s).policy(QueryPolicy::ForceDirect).build()?;
    let installed = family.install(&mut graph, "rows")?;
    let prepared = Runtime::new(Default::default())?
        .prepare(&graph.finish()?)
        .await?;
    let selected = s.apply(
        &id(),
        SelectionUpdate::set(&point("p", "id"), values("id", [1_i64.into()])),
    )?;
    let binding = installed.bind(&selected)?;
    assert_eq!(binding.explain().direct_reason, Some(DirectReason::Forced));
    let inputs = binding.apply(prepared.inputs())?.finish()?;
    let result = prepared.query(&[binding.output()], &[], &inputs).await?;
    assert_eq!(ids(result.table(&binding.output())?.batches()), vec![1]);
    let cleared = installed.bind(&s)?;
    let edited = cleared.apply(inputs.edit())?.finish()?;
    assert_eq!(
        ids(prepared
            .query(&[cleared.output()], &[], &edited)
            .await?
            .table(&cleared.output())?
            .batches()),
        (0..7).collect::<Vec<_>>()
    );

    let missing_column = s.apply(
        &id(),
        SelectionUpdate::set(&point("bad", "missing"), values("missing", [1_i64.into()])),
    )?;
    let bad = installed.bind_with_policy(&missing_column, QueryPolicy::ForceDirect)?;
    let err = bad.apply(prepared.inputs()).unwrap_err();
    assert!(matches!(
        err,
        Error::Dataflow(FlowError::InvalidExprInput { .. })
    ));
    let text = err.to_string();
    assert!(
        text.contains("rows__direct") && text.contains("missing"),
        "{text}"
    );
    let missing = SelectionSet::new([])?;
    assert!(matches!(
        installed.bind(&missing),
        Err(Error::MissingSelection(_))
    ));
    let foreign = Runtime::new(Default::default())?
        .prepare(&DataflowBuilder::new().finish()?)
        .await?;
    assert!(matches!(
        binding.apply(foreign.inputs()),
        Err(Error::Dataflow(FlowError::ForeignHandle))
    ));

    for namespace in ["input", "computation", "output"] {
        let mut other = DataflowBuilder::new();
        let rows = other.table_snapshot(
            "flights",
            TableSnapshot::from_batches(flights().schema(), vec![flights()])?,
        )?;
        match namespace {
            "input" => {
                other.expr_input("target__selection_full", DataType::Boolean)?;
            }
            "computation" => {
                other.add_plan("target__direct", rows.plan_ref())?;
            }
            _ => {
                other.table_output("target", &rows)?;
            }
        }
        let family = membership().query(rows.plan_ref(), Ok)?.plan(&s).build()?;
        assert!(
            matches!(family.install(&mut other, "target"), Err(Error::Dataflow(FlowError::DuplicateName { namespace: n, .. })) if n == namespace)
        );
    }
    Ok(())
}

#[tokio::test]
async fn warmup_reuses_typed_states_and_tracks_fixed_and_source_dependencies() -> TestResult {
    let focus = interval("delay", "delay");
    let fixed = point("airlines", "carrier");
    let inactive = state(Resolution::Intersect);
    for cache in [
        CachePolicy::default(),
        CachePolicy::Disabled,
        CachePolicy::Lru(avenger_datafusion_dataflow::CacheConfig {
            max_bytes: 1,
            max_entries: 1,
        }),
    ] {
        let retained = matches!(&cache, CachePolicy::Lru(c) if c.max_bytes > 1);
        let runtime = Runtime::new(RuntimeConfig {
            cache,
            ..Default::default()
        })?;
        let mut graph = DataflowBuilder::new();
        let source = graph.table_input("flights", flights().schema())?;
        let query = membership().query(source.plan_ref(), |rows| {
            LogicalPlanBuilder::from(rows)
                .aggregate(vec![col("carrier")], measures("distance"))?
                .sort(vec![col("carrier").sort(true, true)])?
                .build()
        })?;
        // The default policy can be overridden in either direction, without preparation.
        let family = query
            .plan(&inactive)
            .focus(&focus)
            .policy(QueryPolicy::ForceDirect)
            .build()?;
        assert_eq!(family.explain().direct_reason, Some(DirectReason::Forced));
        let installed = family.install(&mut graph, "summary")?;
        let scatter = membership()
            .query(source.plan_ref(), Ok)?
            .plan(&inactive)
            .focus(&focus)
            .build()?;
        assert_eq!(
            scatter.explain().direct_reason,
            Some(DirectReason::UnsupportedQueryShape)
        );
        let scatter = scatter.install(&mut graph, "scatter")?;
        let prepared = runtime.prepare(&graph.finish()?).await?;
        let warm = installed.bind_with_policy(&inactive, QueryPolicy::Auto)?;
        let scatter_binding = scatter.bind(&inactive)?;
        assert!(installed.bind(&inactive)?.preaggregate_output().is_none());
        assert!(scatter_binding.preaggregate_output().is_none());
        let snapshot = TableSnapshot::from_batches(flights().schema(), vec![flights()])?;
        let inputs = warm
            .apply(scatter_binding.apply(prepared.inputs())?)?
            .table(&source, snapshot.clone())?
            .finish()?;
        let materialization = warm.preaggregate_output().unwrap();
        let warmed = prepared.query(&[materialization], &[], &inputs).await?;
        assert_eq!(
            warmed.report().executed_nodes,
            vec!["summary__materialization"]
        );
        assert!(inactive.get(&id())?.contributions().next().is_none());

        let brushed = inactive.apply(
            &id(),
            SelectionUpdate::set(&focus, between("delay", 10, 30)),
        )?;
        let fixed_changed = brushed.apply(
            &id(),
            SelectionUpdate::set(&fixed, values("carrier", ["AA".into()])),
        )?;
        let moved = fixed_changed.apply(
            &id(),
            SelectionUpdate::set(&focus, between("delay", 15, 31)),
        )?;
        let replacement = flights().slice(0, 2);
        let changed_source =
            TableSnapshot::from_batches(replacement.schema(), vec![replacement.clone()])?;
        for (index, (state, snapshot, native)) in [
            (&brushed, snapshot.clone(), flights()),
            (&fixed_changed, snapshot.clone(), flights()),
            (&moved, snapshot.clone(), flights()),
            (&moved, snapshot.clone(), flights()),
            (&moved, changed_source.clone(), replacement.clone()),
            (&inactive, changed_source.clone(), replacement.clone()),
        ]
        .into_iter()
        .enumerate()
        {
            let binding = installed.bind_with_policy(state, QueryPolicy::Auto)?;
            let scatter_binding = scatter.bind(state)?;
            let inputs = binding
                .apply(scatter_binding.apply(prepared.inputs())?)?
                .table(&source, snapshot)?
                .finish()?;
            let result = prepared
                .query(&[binding.output(), scatter_binding.output()], &[], &inputs)
                .await?;
            let mat_executed = result
                .report()
                .executed_nodes
                .iter()
                .any(|n| n == "summary__materialization");
            assert_eq!(mat_executed, !retained || matches!(index, 1 | 4 | 5));
            if retained && index == 3 {
                assert!(
                    result.report().executed_nodes.is_empty(),
                    "an identical request reuses final outputs"
                );
            }
            let forced = installed.bind_with_policy(state, QueryPolicy::ForceDirect)?;
            let forced_inputs = forced.apply(inputs.edit())?.finish()?;
            let forced_result = prepared
                .query(&[forced.output()], &[], &forced_inputs)
                .await?;
            assert_results(
                result.table(&binding.output())?.batches(),
                forced_result.table(&forced.output())?.batches(),
                FLOAT_MEASURES,
            );
            let recipe = membership().query(
                SessionContext::new()
                    .read_batch(native)?
                    .into_unoptimized_plan(),
                |rows| {
                    LogicalPlanBuilder::from(rows)
                        .aggregate(vec![col("carrier")], measures("distance"))?
                        .sort(vec![col("carrier").sort(true, true)])?
                        .build()
                },
            )?;
            let expected = SessionContext::new()
                .execute_logical_plan(recipe.logical_plan(state)?)
                .await?
                .collect()
                .await?;
            assert_results(
                result.table(&binding.output())?.batches(),
                &expected,
                FLOAT_MEASURES,
            );
        }
        prepared.clear_results();
        let result = prepared.query(&[materialization], &[], &inputs).await?;
        assert_eq!(
            result.report().executed_nodes,
            vec!["summary__materialization"]
        );
    }
    Ok(())
}

#[tokio::test]
async fn installed_regridding_uses_direct_output_without_warming_an_incompatible_grid() -> TestResult
{
    use avenger_scales_datafusion::BuiltinScale;
    use datafusion::arrow::array::Float32Array;
    use std::sync::Arc;
    let grid = |size| {
        PixelGrid::new(
            BuiltinScale::Linear,
            Arc::new(Float32Array::from(vec![0.0, 40.0])),
            Arc::new(Float32Array::from(vec![0.0, 40.0])),
            Default::default(),
            0.0,
            size,
        )
    };
    let focus = interval("brush", "delay")
        .with_pixel_grids([(ProjectionId::new("delay")?, grid(10.0)?)])?;
    let inactive = state(Resolution::Intersect);
    let selected = inactive.apply(
        &id(),
        SelectionUpdate::set(&focus, between("delay", 11, 30)),
    )?;
    let resized = focus.with_pixel_grids([(ProjectionId::new("delay")?, grid(1.0)?)])?;
    let changed = selected.apply(
        &id(),
        SelectionUpdate::set(&resized, between("delay", 11, 30)),
    )?;
    let mut graph = DataflowBuilder::new();
    let source = graph.table_snapshot(
        "flights",
        TableSnapshot::from_batches(flights().schema(), vec![flights()])?,
    )?;
    let query = membership().query(source.plan_ref(), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(
                Vec::<datafusion::logical_expr::Expr>::new(),
                vec![count(lit(1_i64)).alias("n")],
            )?
            .build()
    })?;
    let installed = query
        .plan(&inactive)
        .focus(&focus)
        .build()?
        .install(&mut graph, "counts")?;
    let prepared = Runtime::new(Default::default())?
        .prepare(&graph.finish()?)
        .await?;
    for (state, incompatible) in [(&selected, false), (&changed, true), (&selected, false)] {
        let automatic = installed.bind(state)?;
        assert_eq!(automatic.preaggregate_output().is_none(), incompatible);
        if incompatible {
            assert_eq!(
                automatic.explain().direct_reason,
                Some(DirectReason::IncompatibleFocus)
            );
        }
        let direct = installed.bind_with_policy(state, QueryPolicy::ForceDirect)?;
        let actual = prepared
            .query(
                &[automatic.output()],
                &[],
                &automatic.apply(prepared.inputs())?.finish()?,
            )
            .await?;
        let expected = prepared
            .query(
                &[direct.output()],
                &[],
                &direct.apply(prepared.inputs())?.finish()?,
            )
            .await?;
        assert_eq!(
            rows(actual.table(&automatic.output())?.batches()),
            rows(expected.table(&direct.output())?.batches())
        );
        if incompatible {
            assert_eq!(actual.report().executed_nodes, vec!["counts__direct"]);
        }
    }
    Ok(())
}

#[tokio::test]
async fn filtered_computed_measures_window_and_limit_reuse_materialization() -> TestResult {
    use datafusion::{
        functions_aggregate::expr_fn::avg,
        functions_window::expr_fn::row_number,
        logical_expr::{expr_fn::cast, Expr, ExprFunctionExt, Limit, LogicalPlan},
    };
    use std::sync::Arc;
    let focus = interval("brush", "delay");
    let inactive = state(Resolution::Intersect);
    let mut graph = DataflowBuilder::new();
    let source = graph.table_snapshot(
        "flights",
        TableSnapshot::from_batches(flights().schema(), vec![flights()])?,
    )?;
    let measure_limit = graph.scalar_input("measure_limit", DataType::Int64)?;
    let final_limit = graph.scalar_input("final_limit", DataType::Int64)?;
    // Resolve the ordinary scalar upstream. The measure filter receives a typed
    // column, without an unchecked arbitrary-expression parameter in the rewrite.
    let columns = source
        .schema()
        .columns()
        .into_iter()
        .map(Expr::Column)
        .chain(std::iter::once(
            measure_limit.expr_ref().alias("measure_limit"),
        ))
        .collect::<Vec<_>>();
    let source = graph.add_plan(
        "measure_rows",
        LogicalPlanBuilder::from(source.plan_ref())
            .project(columns)?
            .build()?,
    )?;
    let query = membership().query(source.plan_ref(), |rows| {
        let input = LogicalPlanBuilder::from(rows)
            .aggregate(
                vec![col("carrier")],
                vec![
                    count(lit(1_i64)).alias("n"),
                    avg(cast(col("distance"), DataType::Float64) * lit(2.0))
                        .filter(col("delay").lt(col("measure_limit")))
                        .build()?
                        .alias("mean"),
                ],
            )?
            .window(vec![row_number()
                .order_by(vec![
                    col("n").sort(false, true),
                    col("carrier").sort(true, true),
                ])
                .build()?
                .alias("position")])?
            .sort(vec![col("position").sort(true, true)])?
            .build()?;
        Ok(LogicalPlan::Limit(Limit {
            skip: None,
            fetch: Some(Box::new(final_limit.expr_ref())),
            input: Arc::new(input),
        }))
    })?;
    let installed = query
        .plan(&inactive)
        .focus(&focus)
        .build()?
        .install(&mut graph, "summary")?;
    let flow = Runtime::new(Default::default())?
        .prepare(&graph.finish()?)
        .await?;
    let warm = installed.bind(&inactive)?;
    let inputs = warm
        .apply(flow.inputs())?
        .scalar(&measure_limit, 100_i64.into())?
        .scalar(&final_limit, 3_i64.into())?
        .finish()?;
    let report = flow
        .query(
            &[warm.preaggregate_output().expect("eligible warmup")],
            &[],
            &inputs,
        )
        .await?;
    assert!(report
        .report()
        .executed_nodes
        .contains(&"summary__materialization".into()));
    for (lower, upper, measure, limit, rebuild) in [
        (0, 25, 100_i64, 3_i64, false),
        (10, 31, 100, 3, false),
        (10, 31, 100, 1, false),
        (10, 31, 20, 1, true),
    ] {
        let selected = inactive.apply(
            &id(),
            SelectionUpdate::set(&focus, between("delay", lower, upper)),
        )?;
        let binding = installed.bind(&selected)?;
        let inputs = binding
            .apply(flow.inputs())?
            .scalar(&measure_limit, measure.into())?
            .scalar(&final_limit, limit.into())?
            .finish()?;
        let result = flow.query(&[binding.output()], &[], &inputs).await?;
        assert_eq!(
            result
                .report()
                .executed_nodes
                .contains(&"summary__materialization".into()),
            rebuild,
            "{:?}",
            result.report()
        );
        let direct = installed.bind_with_policy(&selected, QueryPolicy::ForceDirect)?;
        let direct_inputs = direct.apply(inputs.edit())?.finish()?;
        let expected = flow.query(&[direct.output()], &[], &direct_inputs).await?;
        assert_results(
            result.table(&binding.output())?.batches(),
            expected.table(&direct.output())?.batches(),
            &["mean"],
        );
    }
    Ok(())
}

#[tokio::test]
async fn untrusted_deferred_inputs_and_new_fixed_expressions_cannot_warm_state() -> TestResult {
    use datafusion::logical_expr::{create_udf, ExprFunctionExt, Volatility};
    use std::sync::Arc;
    let focus = interval("brush", "delay");
    let inactive = state(Resolution::Intersect);
    let mut graph = DataflowBuilder::new();
    let source = graph.table_snapshot(
        "flights",
        TableSnapshot::from_batches(flights().schema(), vec![flights()])?,
    )?;
    let arbitrary = graph.expr_input("arbitrary", DataType::Boolean)?;
    let query = membership().query(source.plan_ref(), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(
                vec![col("carrier")],
                vec![count(lit(1_i64))
                    .filter(arbitrary.expr_ref())
                    .build()?
                    .alias("n")],
            )?
            .build()
    })?;
    let family = query.plan(&inactive).focus(&focus).build()?;
    assert_eq!(
        family.explain().direct_reason,
        Some(DirectReason::UnsafeMovedExpression)
    );
    let direct = family.install(&mut graph, "direct")?;
    let query = membership().query(source.plan_ref(), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(vec![col("carrier")], vec![count(lit(1_i64)).alias("n")])?
            .build()
    })?;
    let family = query.plan(&inactive).focus(&focus).build()?;
    let installed = family.install(&mut graph, "summary")?;
    let udf = create_udf(
        "untrusted",
        vec![DataType::Int64],
        DataType::Int64,
        Volatility::Immutable,
        Arc::new(|args| Ok(args[0].clone())),
    );
    let other = ProducerDefinition::new(
        address("other", view("other")),
        SelectionKind::Point,
        vec![Projection::new(
            ProjectionId::new("value")?,
            udf.call(vec![col("delay")]),
        )?],
    )?;
    let selected = inactive.apply(
        &id(),
        SelectionUpdate::set(&other, values("value", [10_i64.into()])),
    )?;
    let binding = installed.bind(&selected)?;
    assert_eq!(
        binding.explain().direct_reason,
        Some(DirectReason::UnsafeMovedExpression)
    );
    assert!(binding.preaggregate_output().is_none());
    let flow = Runtime::new(Default::default())?
        .prepare(&graph.finish()?)
        .await?;
    let inputs = binding
        .apply(direct.bind(&selected)?.apply(flow.inputs())?)?
        .expr(&arbitrary, col("delay").gt(lit(0_i64)))?
        .finish()?;
    let result = flow.query(&[binding.output()], &[], &inputs).await?;
    assert!(!result
        .report()
        .executed_nodes
        .iter()
        .any(|n| n.ends_with("__materialization")));
    Ok(())
}
