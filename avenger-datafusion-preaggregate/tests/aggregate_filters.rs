mod common;
use avenger_datafusion_preaggregate::*;
use common::*;
use datafusion::{
    common::Result,
    logical_expr::{col, lit},
};

#[tokio::test]
async fn all_families_preserve_per_measure_filters_and_group_existence() -> Result<()> {
    for partitions in [1, 4] {
        let ctx = context(partitions)?;
        for grouped in [false, true] {
            for filter in ["keep", "FALSE", "CAST(NULL AS BOOLEAN)", "x > 0"] {
                let groups = if grouped { "g," } else { "" };
                let suffix = if grouped { "GROUP BY g ORDER BY g" } else { "" };
                let measures = [
                    "COUNT",
                    "SUM",
                    "MIN",
                    "MAX",
                    "AVG",
                    "VAR_SAMP",
                    "VAR_POP",
                    "STDDEV_SAMP",
                    "STDDEV_POP",
                ]
                .iter()
                .map(|f| format!("{f}(x) FILTER (WHERE {filter}) AS {f}_x"))
                .collect::<Vec<_>>()
                .join(", ");
                let sql = format!("SELECT {groups} {measures}, COUNT(*) FILTER (WHERE NOT keep) AS others FROM rows WHERE cell >= 0 {suffix}");
                let q = query(&ctx, &sql).await?;
                let p = PreaggregatePlanner::default().prepare(q, vec![col("cell")])?;
                assert!(
                    p.materialization_plan().is_some(),
                    "{sql}: {:?}",
                    p.explain()
                );
                let state = format!("{}", p.materialization_plan().unwrap().display_indent());
                assert!(state.contains("FILTER"), "{state}");
                assert!(!p
                    .explain()
                    .materialization_schema
                    .as_ref()
                    .unwrap()
                    .fields()
                    .iter()
                    .any(|f| f.name() == "keep"));
                for predicate in [
                    lit(true),
                    lit(false),
                    col("cell").eq(lit(0_i32)),
                    col("cell").eq(lit(1_i32)),
                    col("cell").gt(lit(20_i32)),
                ] {
                    compare(&ctx, &p, predicate).await?;
                }
            }
        }
        let q = query(&ctx,"SELECT g, COUNT(*) FILTER (WHERE keep) AS n FROM rows GROUP BY g HAVING COUNT(*) FILTER (WHERE keep) > 0 ORDER BY g").await?;
        let p = PreaggregatePlanner::default().prepare(q, vec![col("cell")])?;
        compare(&ctx, &p, col("cell").eq(lit(1_i32))).await?;
        let q = query(&ctx,"SELECT g, SUM(CAST(cell AS DECIMAL(10,2))) FILTER (WHERE keep) AS s FROM rows GROUP BY g").await?;
        // Integer-to-decimal casts are deliberately outside the initial total-cast allowlist.
        assert!(PreaggregatePlanner::default()
            .prepare(q, vec![col("cell")])?
            .materialization_plan()
            .is_none());
    }
    Ok(())
}

#[tokio::test]
async fn decimal_filters_and_empty_sources_preserve_field_metadata() -> Result<()> {
    use datafusion::{
        arrow::{
            array::{ArrayRef, BooleanArray, Decimal128Array, Float32Array, Int32Array},
            datatypes::{Field, Schema},
            record_batch::RecordBatch,
        },
        functions_aggregate::expr_fn::{avg, count, sum, var_sample},
        logical_expr::{ExprFunctionExt, LogicalPlanBuilder},
    };
    use std::{collections::HashMap, sync::Arc};
    let measures: Vec<ArrayRef> = vec![
        Arc::new(
            Decimal128Array::from(vec![Some(200), None, Some(400)])
                .with_precision_and_scale(12, 2)?,
        ),
        Arc::new(Float32Array::from(vec![Some(2.0), None, Some(4.0)])),
    ];
    for values in measures {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", values.data_type().clone(), true)
                .with_metadata(HashMap::from([("unit".into(), "meters".into())])),
            Field::new(
                "keep",
                datafusion::arrow::datatypes::DataType::Boolean,
                true,
            ),
            Field::new("cell", datafusion::arrow::datatypes::DataType::Int32, false),
        ]));
        let data = RecordBatch::try_new(
            schema,
            vec![
                values,
                Arc::new(BooleanArray::from(vec![Some(true), Some(false), None])),
                Arc::new(Int32Array::from(vec![0, 1, 1])),
            ],
        )?;
        for data in [data.clone(), data.slice(0, 0)] {
            for groups in [vec![], vec![col("cell")]] {
                let ctx = datafusion::prelude::SessionContext::new();
                let source = ctx.read_batch(data.clone())?.into_unoptimized_plan();
                let q = FilterQuery::new(source, |rows| {
                    LogicalPlanBuilder::from(rows)
                        .aggregate(
                            groups,
                            vec![
                                sum(col("x")).filter(col("keep")).build()?.alias("s"),
                                avg(col("x")).filter(col("keep")).build()?.alias("a"),
                                var_sample(col("x")).filter(col("keep")).build()?.alias("v"),
                                count(col("x")).filter(col("keep")).build()?.alias("n"),
                            ],
                        )?
                        .build()
                })?;
                let p = PreaggregatePlanner::default().prepare(q, vec![col("cell")])?;
                assert!(p.materialization_plan().is_some(), "{:?}", p.explain());
                for predicate in [lit(true), lit(false), col("cell").eq(lit(1_i32))] {
                    compare(&ctx, &p, predicate).await?;
                }
            }
        }
    }
    Ok(())
}
