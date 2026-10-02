mod common;
// Each executable uses the relevant subset of the shared chart-side helpers.
#[allow(dead_code)]
#[path = "../examples/support/composition.rs"]
mod composition;
use avenger_datafusion_preaggregate::{BoundQuery, DirectReason, FilterQuery};
use avenger_selection::*;
use common::*;
use composition::ExampleResult;
use datafusion::{
    arrow::{
        array::{Float32Array, Float64Array},
        datatypes::DataType,
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    datasource::{provider_as_source, MemTable},
    functions_aggregate::expr_fn::count,
    logical_expr::{col, create_udf, lit, Expr, LogicalPlan, LogicalPlanBuilder, Volatility},
    prelude::SessionContext,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn source(batch: RecordBatch) -> LogicalPlan {
    SessionContext::new()
        .read_batch(batch)
        .unwrap()
        .into_unoptimized_plan()
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
async fn compare(
    batch: RecordBatch,
    filter: &ConsumerFilter,
    focus: &ProducerDefinition,
    states: &[SelectionSet],
    groups: Vec<Expr>,
    measures: Vec<Expr>,
) -> ExampleResult<()> {
    let rows = source(batch);
    let target = |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(groups.clone(), measures.clone())?
            .build()
    };
    let initial = filter.predicates(&states[0], focus)?;
    let prepared = composition::prepare(rows.clone(), initial.split().unwrap(), target)?;
    assert_eq!(prepared.explain().direct_reason, None);
    let materialization = prepared.materialization_plan().unwrap();
    let stored = MemTable::try_new(
        Arc::new(materialization.schema().as_arrow().clone()),
        vec![execute(materialization.clone()).await],
    )?;
    let relation =
        LogicalPlanBuilder::scan("stored", provider_as_source(Arc::new(stored)), None)?.build()?;
    let direct = FilterQuery::new(rows, target)?;
    for state in states {
        let predicates = filter.predicates(state, focus)?;
        let split = predicates.split().unwrap();
        assert_eq!(split.fixed(), initial.split().unwrap().fixed());
        let BoundQuery::Preaggregated { rollup, .. } = prepared.bind(split.changing().clone())?
        else {
            panic!("eligible binding")
        };
        assert_results(
            &execute(rollup.with_materialization(relation.clone())?).await,
            &execute(direct.direct(predicates.full().clone())?).await,
            FLOAT_MEASURES,
        );
    }
    Ok(())
}

#[tokio::test]
async fn histogram_category_and_mixed_measures_use_the_same_predicate_api() -> ExampleResult<()> {
    let focus = interval("focus", "delay");
    let fixed = point("airline", "carrier");
    let inactive =
        state(Resolution::Intersect).set(&fixed, values("carrier", ["AA".into(), "DL".into()]))?;
    let mut states = vec![inactive.clone()];
    for (lo, hi) in [(0, 15), (10, 30), (20, 40), (100, 200)] {
        states.push(inactive.set(&focus, between("delay", lo, hi))?);
    }
    for group in [
        col("carrier"),
        (col("distance") / lit(500_i64)).alias("bin"),
    ] {
        compare(
            flights(),
            &membership(),
            &focus,
            &states,
            vec![group],
            measures("distance"),
        )
        .await?;
    }
    Ok(())
}

#[tokio::test]
async fn mapped_correlated_pixel_dimensions_and_inactive_match_none_can_warm() -> ExampleResult<()>
{
    let raw = producer("brush", view("focus"), &["x", "category"]);
    let focus = raw.with_pixel_grids([(
        ProjectionId::new("x")?,
        PixelGrid::new(
            avenger_scales_datafusion::BuiltinScale::Linear,
            Arc::new(Float32Array::from(vec![0., 40.])),
            Arc::new(Float32Array::from(vec![40., 0.])),
            Default::default(),
            0.,
            10.,
        )?,
    )])?;
    let filter = ConsumerFilter::new(
        view("target"),
        SelectionFilter::membership(&id(), EmptySelection::MatchNone),
    )
    .with_projection(&raw, &ProjectionId::new("x")?, col("delay"))?
    .with_projection(&raw, &ProjectionId::new("category")?, col("carrier"))?;

    let inactive = state(Resolution::Intersect);
    let mut states = vec![inactive.clone()];
    for lower in [0, 10, 20] {
        states.push(inactive.set(
            &focus,
            SelectionValue::tuples(vec![
                vec![
                    term(
                        "x",
                        ValueTest::Range {
                            lower: std::ops::Bound::Included(lower.into()),
                            upper: std::ops::Bound::Excluded((lower + 10).into()),
                        },
                    ),
                    term("category", ValueTest::Equal("AA".into())),
                ],
                vec![
                    term(
                        "x",
                        ValueTest::Range {
                            lower: std::ops::Bound::Included(20_i64.into()),
                            upper: std::ops::Bound::Included(30_i64.into()),
                        },
                    ),
                    term("category", ValueTest::Equal("DL".into())),
                ],
            ]),
        )?);
    }
    compare(
        flights(),
        &filter,
        &focus,
        &states,
        vec![col("region")],
        vec![count(lit(1_i64)).alias("n")],
    )
    .await
}

#[tokio::test]
async fn exact_nonfinite_categories_and_invalid_pixel_cells_survive_materialization(
) -> ExampleResult<()> {
    let data = batch(vec![(
        "x",
        Arc::new(Float64Array::from(vec![
            Some(-0.),
            Some(0.),
            Some(f64::NAN),
            Some(f64::from_bits(f64::NAN.to_bits() + 1)),
            Some(f64::INFINITY),
            Some(f64::NEG_INFINITY),
            Some(1.),
            None,
        ])),
    )]);
    let focus = point("points", "x");
    let inactive = state(Resolution::Intersect);
    let mut states = vec![inactive.clone()];
    for v in [None, Some(-0.), Some(f64::NAN), Some(f64::INFINITY)] {
        states.push(inactive.set(&focus, values("x", [ScalarValue::Float64(v)]))?);
    }
    compare(
        data.clone(),
        &membership(),
        &focus,
        &states,
        vec![],
        vec![count(lit(1_i64)).alias("n")],
    )
    .await?;
    let pixel = interval("brush", "x").with_pixel_grids([(
        ProjectionId::new("x")?,
        PixelGrid::new(
            avenger_scales_datafusion::BuiltinScale::Linear,
            Arc::new(Float32Array::from(vec![0., 2.])),
            Arc::new(Float32Array::from(vec![0., 20.])),
            Default::default(),
            0.,
            2.,
        )?,
    )])?;
    let brushed = inactive.set(
        &pixel,
        range(
            "x",
            std::ops::Bound::Included(0_f64.into()),
            std::ops::Bound::Excluded(2_f64.into()),
        ),
    )?;
    let pixel_data = batch(vec![(
        "x",
        Arc::new(Float32Array::from(vec![
            Some(-0.),
            Some(0.),
            Some(f32::NAN),
            Some(f32::NAN),
            Some(f32::INFINITY),
            Some(f32::NEG_INFINITY),
            Some(1.),
            None,
        ])),
    )]);
    compare(
        pixel_data,
        &membership(),
        &pixel,
        &[inactive.clone(), brushed, inactive],
        vec![],
        vec![count(lit(1_i64)).alias("n")],
    )
    .await
}

#[tokio::test]
async fn unsupported_shapes_fall_back_and_immutable_fixed_udfs_preaggregate() -> ExampleResult<()> {
    let focus = interval("brush", "delay");
    let selected = state(Resolution::Intersect).set(&focus, between("delay", 10, 31))?;
    let predicates = membership().predicates(&selected, &focus)?;
    let scatter = composition::prepare(source(flights()), predicates.split().unwrap(), Ok)?;
    assert_eq!(
        scatter.explain().direct_reason,
        Some(DirectReason::UnsupportedQueryShape)
    );
    let scatter_direct =
        FilterQuery::new(source(flights()), Ok)?.direct(predicates.full().clone())?;
    assert_eq!(ids(&execute(scatter_direct).await), vec![1, 2, 3, 4, 5, 6]);
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
    let other = ProducerDefinition::new(
        id(),
        ProducerId::new("other").unwrap(),
        view("other"),
        vec![Projection::new(
            ProjectionId::new("distance")?,
            udf.call(vec![col("distance")]),
        )?],
    )?;
    let changed = selected.set(&other, values("distance", [600_i64.into()]))?;
    let p = membership().predicates(&changed, &focus)?;
    let prepared = composition::prepare(source(flights()), p.split().unwrap(), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(vec![col("carrier")], vec![count(lit(1_i64))])?
            .build()
    })?;
    assert_eq!(prepared.explain().direct_reason, None);
    assert_eq!(calls.load(Ordering::Relaxed), 0);

    compare(
        flights(),
        &membership(),
        &focus,
        &[changed],
        vec![col("carrier")],
        vec![count(lit(1_i64))],
    )
    .await?;
    assert!(calls.load(Ordering::Relaxed) > 0);
    Ok(())
}
