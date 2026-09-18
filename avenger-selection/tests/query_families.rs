mod common;

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use avenger_selection::*;
use common::*;
use datafusion::{
    arrow::{datatypes::DataType, record_batch::RecordBatch},
    functions_aggregate::expr_fn::count,
    logical_expr::{
        col, create_udf, lit, scalar_subquery, LogicalPlan, LogicalPlanBuilder, Volatility,
    },
    prelude::SessionContext,
};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
fn direct(bound: BoundQuery) -> LogicalPlan {
    match bound {
        BoundQuery::Direct { plan, .. } => plan,
        _ => panic!("expected direct execution"),
    }
}
async fn execute(plan: LogicalPlan) -> Vec<RecordBatch> {
    SessionContext::new()
        .execute_logical_plan(plan)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap()
}
async fn result_rows(plan: LogicalPlan) -> Vec<String> {
    let mut result = Vec::new();
    for batch in execute(plan).await {
        for i in 0..batch.num_rows() {
            result.push(
                (0..batch.num_columns())
                    .map(|c| {
                        datafusion::common::ScalarValue::try_from_array(batch.column(c), i)
                            .unwrap()
                            .to_string()
                    })
                    .collect::<Vec<_>>()
                    .join("|"),
            );
        }
    }
    result.sort();
    result
}

#[tokio::test]
async fn construction_runs_once_and_planning_never_evaluates_functions() -> TestResult {
    let source = SessionContext::new()
        .read_batch(flights())?
        .into_unoptimized_plan();
    let calls = Arc::new(AtomicUsize::new(0));
    let invoked = calls.clone();
    let udf = create_udf(
        "observed",
        vec![DataType::Int64],
        DataType::Int64,
        Volatility::Immutable,
        Arc::new(move |args| {
            invoked.fetch_add(1, Ordering::Relaxed);
            Ok(args[0].clone())
        }),
    );
    let mut constructions = 0;
    let query = membership().query(source, |rows| {
        constructions += 1;
        LogicalPlanBuilder::from(rows)
            .project(vec![udf.call(vec![col("id")]).alias("id")])?
            .build()
    })?;
    let p = point("points", "id");
    let inactive = state(Resolution::Intersect);
    let family = query.plan(&inactive).focus(&p).build()?;
    assert_eq!(family.focus(), Some(&p));
    assert_eq!(family.policy(), QueryPolicy::Auto);
    assert_eq!(
        family.explain().direct_reason,
        Some(DirectReason::UnsupportedQueryShape)
    );
    let selected = inactive.apply(
        &id(),
        SelectionUpdate::set(&p, values("id", [1_i64.into(), 3_i64.into()])),
    )?;
    let automatic = family.bind(&selected)?;
    let forced = family.bind_with_policy(&selected, QueryPolicy::ForceDirect)?;
    assert_eq!(forced.explain().direct_reason, Some(DirectReason::Forced));
    assert_eq!(constructions, 1);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert_eq!(ids(&execute(direct(automatic)).await), vec![1, 3]);
    assert_eq!(ids(&execute(direct(forced)).await), vec![1, 3]);
    assert!(calls.load(Ordering::Relaxed) > 0);
    assert_eq!(
        ids(&execute(query.logical_plan(&inactive)?).await),
        (0..7).collect::<Vec<_>>()
    );
    Ok(())
}

#[tokio::test]
async fn owned_sites_handle_repeated_relations_subqueries_and_unrelated_filters() -> TestResult {
    let source = SessionContext::new()
        .read_batch(flights())?
        .into_unoptimized_plan();
    let selected = state(Resolution::Union).apply(
        &id(),
        SelectionUpdate::set(&point("points", "id"), values("id", [2_i64.into()])),
    )?;
    let repeated = membership().query(source.clone(), |rows| {
        LogicalPlanBuilder::from(rows.clone()).union(rows)?.build()
    })?;
    assert_eq!(
        ids(&execute(repeated.logical_plan(&selected)?).await),
        vec![2, 2]
    );
    let subquery = membership().query(source.clone(), |rows| {
        let count = LogicalPlanBuilder::from(rows)
            .aggregate(
                Vec::<datafusion::logical_expr::Expr>::new(),
                vec![count(lit(1_i64))],
            )?
            .build()?;
        LogicalPlanBuilder::empty(true)
            .project(vec![scalar_subquery(Arc::new(count)).alias("n")])?
            .build()
    })?;
    assert_eq!(
        result_rows(subquery.logical_plan(&selected)?).await,
        vec!["1"]
    );
    let unrelated = membership().query(source.clone(), |rows| {
        LogicalPlanBuilder::from(rows).filter(lit(false))?.build()
    })?;
    assert!(execute(unrelated.logical_plan(&selected)?)
        .await
        .iter()
        .all(|b| b.num_rows() == 0));
    let ignored = membership()
        .query(source.clone(), |_| Ok(source.clone()))
        .unwrap_err();
    assert!(matches!(ignored, Error::InvalidQuery(_)));
    // A relation captured by one callback cannot become another query's owned site.
    let mut captured = None;
    membership().query(source.clone(), |rows| {
        captured = Some(rows.clone());
        Ok(rows)
    })?;
    assert!(membership()
        .query(source, |rows| LogicalPlanBuilder::from(rows)
            .union(captured.unwrap())?
            .build())
        .is_err());
    Ok(())
}

#[tokio::test]
async fn direct_families_preserve_count_schemas_nulls_and_empty_results() -> TestResult {
    let source = SessionContext::new()
        .read_batch(flights())?
        .into_unoptimized_plan();
    let p = point("points", "id");
    let inactive = state(Resolution::Intersect);
    let snapshots = [
        inactive.clone(),
        inactive.apply(
            &id(),
            SelectionUpdate::set(&p, values("id", [6_i64.into()])),
        )?,
        inactive.apply(&id(), SelectionUpdate::set(&p, values("id", [])))?,
    ];
    for groups in [vec![], vec![col("carrier")]] {
        let query = membership().query(source.clone(), |rows| {
            LogicalPlanBuilder::from(rows)
                .aggregate(
                    groups.clone(),
                    vec![
                        count(lit(1_i64)).alias("rows"),
                        count(col("carrier")).alias("non_null"),
                    ],
                )?
                .build()
        })?;
        let family = query
            .plan(&inactive)
            .focus(&p)
            .policy(QueryPolicy::ForceDirect)
            .build()?;
        for s in &snapshots {
            let original = LogicalPlanBuilder::from(source.clone())
                .filter(membership().predicate(s)?)?
                .aggregate(
                    groups.clone(),
                    vec![
                        count(lit(1_i64)).alias("rows"),
                        count(col("carrier")).alias("non_null"),
                    ],
                )?
                .build()?;
            let bound = family.bind(s)?;
            assert_eq!(bound.explain().direct_reason, Some(DirectReason::Forced));
            let actual = direct(bound);
            assert_eq!(actual.schema(), original.schema());
            assert_eq!(result_rows(actual).await, result_rows(original).await);
        }
        if groups.is_empty() {
            assert_eq!(
                result_rows(direct(family.bind(&snapshots[2])?)).await,
                vec!["0|0"]
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn automatic_and_forced_bindings_keep_pixel_semantics_across_regridding() -> TestResult {
    use avenger_scales_datafusion::BuiltinScale;
    use datafusion::arrow::array::Float32Array;
    let original = interval("delay", "delay");
    let grid = |size| {
        PixelGrid::new(
            BuiltinScale::Linear,
            Arc::new(Float32Array::from(vec![0.0, 100.0])),
            Arc::new(Float32Array::from(vec![0.0, 100.0])),
            Default::default(),
            0.0,
            size,
        )
    };
    let p = original.with_pixel_grids([(ProjectionId::new("delay")?, grid(20.0)?)])?;
    let source = SessionContext::new()
        .read_batch(flights())?
        .into_unoptimized_plan();
    let filter = cross(view("other"));
    let query = filter.query(source, Ok)?;
    let inactive = state(Resolution::Intersect);
    let family = query.plan(&inactive).focus(&p).build()?;
    let selected = inactive.apply(&id(), SelectionUpdate::set(&p, between("delay", 11, 21)))?;
    let resized = p.with_pixel_grids([(ProjectionId::new("delay")?, grid(1.0)?)])?;
    let changed = selected.apply(
        &id(),
        SelectionUpdate::set(&resized, between("delay", 11, 21)),
    )?;
    for (s, expected) in [
        (&selected, vec![0, 1, 6]),
        (&changed, vec![2, 4, 5, 6]),
        (&inactive, (0..7).collect()),
    ] {
        for policy in [QueryPolicy::Auto, QueryPolicy::ForceDirect] {
            assert_eq!(
                ids(&execute(direct(family.bind_with_policy(s, policy)?)).await),
                expected
            );
        }
    }
    Ok(())
}

#[test]
fn forced_execution_keeps_validation_and_family_values_are_not_frozen() -> TestResult {
    let source = SessionContext::new()
        .read_batch(flights())?
        .into_unoptimized_plan();
    let query = membership().query(source, Ok)?;
    let s = state(Resolution::Intersect);
    let family = query.plan(&s).policy(QueryPolicy::ForceDirect).build()?;
    let missing = SelectionSet::new([])?;
    assert!(matches!(
        family.bind(&missing),
        Err(Error::MissingSelection(_))
    ));
    assert!(matches!(
        query.plan(&missing).build(),
        Err(Error::MissingSelection(_))
    ));
    let foreign_focus = ProducerDefinition::new(
        ProducerAddress {
            selection: SelectionId::new("missing")?,
            ..point("p", "id").address().clone()
        },
        SelectionKind::Point,
        vec![Projection::new(ProjectionId::new("id")?, col("id"))?],
    )?;
    assert!(matches!(
        query.plan(&s).focus(&foreign_focus).build(),
        Err(Error::MissingSelection(_))
    ));
    Ok(())
}
