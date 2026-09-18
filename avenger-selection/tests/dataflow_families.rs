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
        assert_eq!(definition.num_inputs(), 4); // One full predicate per target and the caller's scalar.
        assert_eq!(definition.num_outputs(), 3);
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
                for binding in &bindings {
                    assert_eq!(binding.explain().strategy, QueryStrategy::Direct);
                    assert_eq!(
                        binding.explain().direct_reason,
                        Some(if policy == QueryPolicy::Auto {
                            DirectReason::PreaggregationNotImplemented
                        } else {
                            DirectReason::Forced
                        })
                    );
                    assert!(binding.preaggregate_output().is_none());
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
                if cached && policy == QueryPolicy::ForceDirect {
                    assert_eq!(
                        result.report().physical_plans,
                        0,
                        "policy changes do not change a direct predicate's cache key"
                    );
                }
                if cached && step == 2 && policy == QueryPolicy::Auto {
                    assert_eq!(
                        result.report().physical_plans,
                        2,
                        "the focused view still has its own cached result"
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
