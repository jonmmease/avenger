//! Census failure policy: an unparser-infidelity failure is fixed by (in order
//! of preference) adjusting expansion to unparse a semantically identical but
//! unparser-friendly plan, filing/fixing upstream in DataFusion, or adding the
//! stage to `NATIVE_ONLY_STAGES` with a comment and link. Never weaken the
//! batch-equality assertion.

use avenger_chart_core::{
    CoordinationScope, DataTransform, DataTransformCompileContext, DataTransformStage,
    ExecutionShape, Param,
};
use avenger_chart_transforms::expand::{expand_stage, test_support::assert_stage_roundtrip};
use avenger_chart_transforms::{
    Aggregate, Calculate, Filter, Fold, Impute, JoinAggregate, Kde, Lump, ScalarAggregate, Select,
    Stack, StackOffset, TimeLevels, Window,
};
use datafusion::arrow::{
    array::{Float64Array, Int64Array, StringArray, TimestampMillisecondArray},
    datatypes::{DataType, Field, Schema, TimeUnit},
    record_batch::RecordBatch,
};
use datafusion::common::ScalarValue;
use datafusion::functions::expr_fn::lower;
use datafusion::functions_aggregate::expr_fn::sum;
use datafusion::functions_window::expr_fn::row_number;
use datafusion::prelude::{SessionContext, col, lit};
use std::sync::Arc;

fn sample_batch() -> RecordBatch {
    let mut categories = Vec::new();
    let mut series = Vec::new();
    let mut values = Vec::new();
    let mut values2 = Vec::new();
    let mut months = Vec::new();
    let mut timestamps = Vec::new();

    for i in 0..30 {
        categories.push(Some(match i % 3 {
            0 => "A",
            1 => "B",
            _ => "C",
        }));
        series.push(Some(if i % 2 == 0 { "s1" } else { "s2" }));
        values.push(if i % 11 == 0 {
            None
        } else {
            Some((i as f64 % 7.0) - 2.0)
        });
        values2.push(Some((i as f64) * 0.5 + 1.0));
        months.push(Some((i % 5) as i64));
        timestamps.push(Some(1_609_459_200_000 + (i as i64) * 86_400_000));
    }

    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, true),
            Field::new("series", DataType::Utf8, true),
            Field::new("value", DataType::Float64, true),
            Field::new("value2", DataType::Float64, true),
            Field::new("month", DataType::Int64, true),
            Field::new(
                "timestamp",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                true,
            ),
        ])),
        vec![
            Arc::new(StringArray::from(categories)) as _,
            Arc::new(StringArray::from(series)) as _,
            Arc::new(Float64Array::from(values)) as _,
            Arc::new(Float64Array::from(values2)) as _,
            Arc::new(Int64Array::from(months)) as _,
            Arc::new(TimestampMillisecondArray::from(timestamps)) as _,
        ],
    )
    .expect("sample batch")
}

fn stage<T>(transform: T) -> DataTransformStage
where
    T: DataTransform,
{
    let (compiled, _) = transform
        .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
        .expect("compile transform");
    DataTransformStage::new(CoordinationScope::Free, compiled)
}

fn census_cases() -> Vec<(&'static str, DataTransformStage)> {
    vec![
        (
            "aggregate_sum",
            stage(
                Aggregate::new()
                    .group_by([col("category")])
                    .sum("total", col("value")),
            ),
        ),
        (
            "aggregate_mean_two_keys",
            stage(
                Aggregate::new()
                    .group_by([col("category"), col("series")])
                    .mean("avg_value", col("value")),
            ),
        ),
        (
            "calculate_arithmetic_string",
            stage(
                Calculate::new()
                    .expr("shifted", col("value") + lit(2.0))
                    .expr("category_lower", lower(col("category"))),
            ),
        ),
        ("filter", stage(Filter::new(col("value").gt(lit(1.0))))),
        (
            "fold",
            stage(
                Fold::new()
                    .field("value", col("value"))
                    .field("value2", col("value2"))
                    .index("fold_index"),
            ),
        ),
        (
            "impute_value",
            stage(
                Impute::new(col("value"))
                    .key(col("month"))
                    .group_by([col("series")])
                    .value(lit(0.0))
                    .flag("was_imputed"),
            ),
        ),
        (
            "select",
            stage(
                Select::new()
                    .expr(col("category"))
                    .expr(col("value").alias("renamed_value")),
            ),
        ),
        (
            "stack_zero",
            stage(
                Stack::new(col("value"))
                    .group_by([col("category")])
                    .sort_by_exprs([col("series")])
                    .name("value_stack"),
            ),
        ),
        (
            "stack_normalize",
            stage(
                Stack::new(col("value"))
                    .group_by([col("category")])
                    .sort_by_exprs([col("series")])
                    .offset(StackOffset::Normalize)
                    .name("normalized_stack"),
            ),
        ),
        (
            "time_levels",
            stage(TimeLevels::new(col("timestamp")).year().month()),
        ),
        (
            "window_row_number",
            stage(
                Window::new()
                    .partition_by([col("category")])
                    .order_by([col("value").sort(true, false)])
                    .expr("row_number", row_number()),
            ),
        ),
    ]
}

#[tokio::test]
async fn class_one_transforms_roundtrip_through_sql_expansion() {
    let ctx = SessionContext::new();
    let sample = sample_batch();
    for (name, stage) in census_cases() {
        eprintln!("roundtrip {name}");
        assert_stage_roundtrip(&stage, sample.clone(), &ctx).await;
    }
}

#[tokio::test]
async fn break_transforms_do_not_expand() {
    let ctx = SessionContext::new();
    let schema = sample_batch().schema();

    let kde = stage(Kde::new(col("value")).steps(lit(8.0)));
    assert_eq!(kde.transform.execution_shape(), ExecutionShape::PlanBreak);
    assert!(
        expand_stage(&kde, Arc::clone(&schema), &ctx)
            .await
            .unwrap()
            .is_none()
    );

    let scalar = stage(ScalarAggregate::new().count("n"));
    assert_eq!(
        scalar.transform.execution_shape(),
        ExecutionShape::PlanBreak
    );
    assert!(
        expand_stage(&scalar, Arc::clone(&schema), &ctx)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn native_only_stages_do_not_expand() {
    let ctx = SessionContext::new();
    let schema = sample_batch().schema();
    let join_aggregate = stage(
        JoinAggregate::new()
            .group_by([col("category")])
            .sum("category_total", col("value")),
    );
    let lump = stage(
        Lump::top_n(col("category"), 2)
            .order_by(sum(col("value")))
            .name("category_lump"),
    );

    for stage in [join_aggregate, lump] {
        assert_eq!(
            stage.transform.execution_shape(),
            ExecutionShape::PlanRewrite
        );
        assert!(
            expand_stage(&stage, Arc::clone(&schema), &ctx)
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn placeholder_filter_roundtrips() {
    let ctx = SessionContext::new();
    let min = Param::new("min", ScalarValue::Float64(Some(1.0)));
    let stage = stage(Filter::new(col("value").gt(min.expr())));
    avenger_chart_transforms::expand::test_support::assert_stage_roundtrip_with_params(
        &stage,
        sample_batch(),
        &ctx,
        &indexmap::indexmap! { min.name => ScalarValue::Float64(Some(1.0)) },
    )
    .await;
}

#[ignore]
#[tokio::test]
async fn print_generated_sql() {
    let ctx = SessionContext::new();
    let schema = sample_batch().schema();
    for (name, stage) in census_cases() {
        let Some(expanded) = expand_stage(&stage, Arc::clone(&schema), &ctx)
            .await
            .expect("expand stage")
        else {
            println!("{name}: <native>");
            continue;
        };
        println!("{name}:\n{}\n", expanded.sql);
    }
}
