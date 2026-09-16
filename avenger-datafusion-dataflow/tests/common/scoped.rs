#![allow(dead_code)]
use avenger_datafusion_dataflow::{
    arrow::{
        array::{Int32Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema, SchemaRef},
        record_batch::RecordBatch,
    },
    datafusion::{
        functions_aggregate::expr_fn::max,
        logical_expr::{col, scalar_subquery, Expr, JoinType, LogicalPlanBuilder},
    },
    DataflowBuilder, Result, ScalarInput, ScalarOutput, ScopeHandle, TableInput, TableOutput,
    TableSnapshot,
};
use std::sync::Arc;

pub fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, true),
        Field::new("year", DataType::Int32, false),
        Field::new("product", DataType::Utf8, false),
        Field::new("amount", DataType::Int64, false),
    ]))
}
pub fn data(rows: &[(Option<&str>, i32, &str, i64)]) -> TableSnapshot {
    let batch = RecordBatch::try_new(
        schema(),
        vec![
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.0).collect::<Vec<_>>(),
            )),
            Arc::new(Int32Array::from(
                rows.iter().map(|row| row.1).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.2).collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from(
                rows.iter().map(|row| row.3).collect::<Vec<_>>(),
            )),
        ],
    )
    .unwrap();
    TableSnapshot::from_batches(schema(), vec![batch]).unwrap()
}
pub fn sales() -> TableSnapshot {
    data(&[
        (Some("East"), 2024, "A", 10),
        (Some("West"), 2025, "A", 50),
        (Some("East"), 2025, "A", 40),
        (Some("East"), 2024, "B", 30),
        (Some("West"), 2025, "B", 90),
        (Some("East"), 2025, "B", 80),
    ])
}
pub fn products(items: &[&str]) -> TableSnapshot {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "product",
        DataType::Utf8,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(StringArray::from(items.to_vec()))],
    )
    .unwrap();
    TableSnapshot::from_batches(schema, vec![batch]).unwrap()
}
pub fn amounts(table: &TableSnapshot) -> Vec<i64> {
    let mut values = table
        .batches()
        .iter()
        .flat_map(|batch| {
            batch
                .column_by_name("amount")
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values()
                .to_vec()
        })
        .collect::<Vec<_>>();
    values.sort();
    values
}
pub struct Year {
    pub fraction: ScalarInput,
    pub selected: TableInput,
    pub rows: TableOutput,
    pub threshold: ScalarOutput,
    pub maximum: ScalarOutput,
}
pub struct Region {
    pub limit: ScalarInput,
    pub years: ScopeHandle,
    pub year: Year,
    pub rows: TableOutput,
}
pub struct Fixture {
    pub sales: TableInput,
    pub multiplier: ScalarInput,
    pub regions: ScopeHandle,
    pub region: Region,
}
pub fn build(graph: &mut DataflowBuilder) -> Result<Fixture> {
    let sales = graph.table_input("sales", schema())?;
    let multiplier = graph.scalar_input("multiplier", DataType::Int64)?;
    let (regions, region) =
        graph.partition_by("regions", sales.plan_ref(), vec![col("region")], |scope| {
            let rows = scope.rows();
            let output = scope.table_output("rows", &rows)?;
            let limit = scope.scalar_input("limit", DataType::Int64)?;
            let regional = scope.add_expr("regional", multiplier.expr_ref() * limit.expr_ref())?;
            let (years, year) =
                scope.partition_by("years", rows.plan_ref(), vec![col("year")], |scope| {
                    let fraction = scope.scalar_input("fraction", DataType::Int64)?;
                    let selected = scope.table_input("selected", products(&[]).schema().clone())?;
                    scope.scalar_input("unused", DataType::Utf8)?;
                    let threshold =
                        scope.add_expr("threshold", regional.expr_ref() * fraction.expr_ref())?;
                    let filtered = scope.add_plan(
                        "filtered",
                        LogicalPlanBuilder::from(scope.rows().plan_ref())
                            .join(
                                selected.plan_ref(),
                                JoinType::LeftSemi,
                                (vec!["product"], vec!["product"]),
                                None,
                            )?
                            .filter(col("amount").gt(threshold.expr_ref()))?
                            .build()?,
                    )?;
                    let maximum_table = scope.add_plan(
                        "maximum_table",
                        LogicalPlanBuilder::from(filtered.plan_ref())
                            .aggregate(
                                Vec::<Expr>::new(),
                                vec![max(col("amount")).alias("maximum")],
                            )?
                            .build()?,
                    )?;
                    let maximum = scope.add_expr(
                        "maximum",
                        scalar_subquery(Arc::new(maximum_table.plan_ref())),
                    )?;
                    Ok(Year {
                        fraction,
                        selected,
                        rows: scope.table_output("rows", &filtered)?,
                        threshold: scope.scalar_output("threshold", &threshold)?,
                        maximum: scope.scalar_output("maximum", &maximum)?,
                    })
                })?;
            Ok(Region {
                limit,
                years,
                year,
                rows: output,
            })
        })?;
    Ok(Fixture {
        sales,
        multiplier,
        regions,
        region,
    })
}
