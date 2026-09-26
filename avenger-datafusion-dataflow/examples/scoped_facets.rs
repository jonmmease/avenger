use std::sync::Arc;

use avenger_datafusion_dataflow::{
    arrow::{
        array::{Float64Array, Int32Array, StringArray},
        datatypes::{DataType, Field, Schema, SchemaRef},
        record_batch::RecordBatch,
        util::pretty::pretty_format_batches,
    },
    datafusion::{
        common::ScalarValue,
        functions_aggregate::expr_fn::max,
        logical_expr::{col, scalar_subquery, Expr, JoinType, LogicalPlanBuilder},
    },
    DataflowBuilder, DataflowResult, Result, Runtime, RuntimeConfig, ScalarInput, ScalarOutput,
    ScopeHandle, TableInput, TableOutput, TableSnapshot, TableStore,
};

struct YearHandles {
    fraction: ScalarInput,
    selected_products: TableInput,
    rows: TableOutput,
    threshold: ScalarOutput,
    maximum: ScalarOutput,
}
struct RegionHandles {
    limit: ScalarInput,
    years: ScopeHandle,
    year: YearHandles,
}

#[tokio::main]
async fn main() -> Result<()> {
    let sales_schema = Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, false),
        Field::new("year", DataType::Int32, false),
        Field::new("product", DataType::Utf8, false),
        Field::new("amount", DataType::Float64, false),
    ]));
    let sales = TableSnapshot::from_batches(
        sales_schema.clone(),
        vec![RecordBatch::try_new(
            sales_schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![
                    "East", "West", "East", "East", "West", "East",
                ])),
                Arc::new(Int32Array::from(vec![2024, 2025, 2025, 2024, 2025, 2025])),
                Arc::new(StringArray::from(vec!["A", "A", "A", "B", "B", "B"])),
                Arc::new(Float64Array::from(vec![
                    40.0, 80.0, 60.0, 70.0, 120.0, 90.0,
                ])),
            ],
        )?],
    )?;
    let sales_store = TableStore::new(sales);
    let product_schema = Arc::new(Schema::new(vec![Field::new(
        "product",
        DataType::Utf8,
        false,
    )]));
    let all_products = products(product_schema.clone(), &["A", "B"])?;
    let east_products = products(product_schema.clone(), &["B"])?;

    let mut graph = DataflowBuilder::new();
    let sales = graph.table_input("sales", sales_schema)?;
    let multiplier = graph.scalar_input("multiplier", DataType::Float64)?;
    let (regions, region) =
        graph.partition_by("regions", sales.plan_ref(), vec![col("region")], |scope| {
            let local_rows = scope.rows();
            let limit = scope.scalar_input("limit", DataType::Float64)?;
            let regional_limit =
                scope.add_scalar("regional_limit", multiplier.expr_ref() * limit.expr_ref())?;
            let (years, year) =
                scope.partition_by("years", local_rows.plan_ref(), vec![col("year")], |scope| {
                    let fraction = scope.scalar_input("fraction", DataType::Float64)?;
                    let selected_products =
                        scope.table_input("selected_products", product_schema)?;
                    let threshold = scope
                        .add_scalar("threshold", regional_limit.expr_ref() * fraction.expr_ref())?;
                    let filtered = scope.add_plan(
                        "filtered",
                        LogicalPlanBuilder::from(scope.rows().plan_ref())
                            .join(
                                selected_products.plan_ref(),
                                JoinType::LeftSemi,
                                (vec!["product"], vec!["product"]),
                                None,
                            )?
                            .filter(col("amount").gt(threshold.expr_ref()))?
                            .sort(vec![col("amount").sort(true, false)])?
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
                    let maximum = scope.add_scalar(
                        "maximum",
                        scalar_subquery(Arc::new(maximum_table.plan_ref())),
                    )?;
                    Ok(YearHandles {
                        fraction,
                        selected_products,
                        rows: scope.table_output("rows", &filtered)?,
                        threshold: scope.scalar_output("threshold", &threshold)?,
                        maximum: scope.scalar_output("maximum", &maximum)?,
                    })
                })?;
            Ok(RegionHandles { limit, years, year })
        })?;
    let (flat_panels, flat_rows) = graph.partition_by(
        "flat_panels",
        sales.plan_ref(),
        vec![col("region"), col("year")],
        |scope| scope.table_output("rows", &scope.rows()),
    )?;

    let prepared = Runtime::new(RuntimeConfig::default())?
        .prepare(&graph.finish()?)
        .await?;
    let east_2025 = regions
        .instance([ScalarValue::from("East")])?
        .child(&region.years, [ScalarValue::from(2025_i32)])?;
    let inputs = prepared
        .inputs()
        .table(&sales, sales_store.snapshot())?
        .scalar(&multiplier, ScalarValue::from(1.0))?
        .scope_defaults(&regions, |b| {
            b.scalar(&region.limit, ScalarValue::from(100.0))
        })?
        .scope_defaults(&region.years, |b| {
            b.scalar(&region.year.fraction, ScalarValue::from(0.5))?
                .table(&region.year.selected_products, all_products)
        })?
        .at(&east_2025, |b| {
            b.scalar(&region.year.fraction, ScalarValue::from(0.8))?
                .table(&region.year.selected_products, east_products)
        })?
        .finish()?;
    let result = prepared
        .query(
            &[region.year.rows],
            &[region.year.threshold, region.year.maximum],
            &inputs,
        )
        .await?;
    println!("Initial nested results:");
    print_panels(&result, &regions, &region)?;

    let east_key = regions.key([ScalarValue::from("East")])?;
    let year_key = region.years.key([ScalarValue::from(2025_i32)])?;
    let panel = result
        .scope(&regions)?
        .get(&east_key)
        .expect("East discovered")
        .scope(&region.years)?
        .get(&year_key)
        .expect("2025 discovered");
    assert_eq!(panel.table(&region.year.rows)?.num_rows(), 1);
    let next_inputs = inputs
        .edit()
        .at(panel.instance(), |b| {
            b.scalar(&region.year.fraction, ScalarValue::from(0.95))
        })?
        .finish()?;
    let next = prepared
        .query(
            &[region.year.rows],
            &[region.year.threshold, region.year.maximum],
            &next_inputs,
        )
        .await?;
    println!("After updating East / 2025:");
    print_panels(&next, &regions, &region)?;
    let empty_panel = next
        .scope(&regions)?
        .get(&east_key)
        .unwrap()
        .scope(&region.years)?
        .get(&year_key)
        .unwrap();
    assert_eq!(empty_panel.table(&region.year.rows)?.num_rows(), 0);
    assert_eq!(
        empty_panel.scalar(&region.year.maximum)?,
        &ScalarValue::Float64(None)
    );
    assert_eq!(panel.table(&region.year.rows)?.num_rows(), 1);

    let flat = prepared.query(&[flat_rows], &[], &inputs).await?;
    let composite = flat_panels.key([ScalarValue::from("East"), ScalarValue::from(2025_i32)])?;
    let flat_panel = flat.scope(&flat_panels)?.get(&composite).unwrap();
    println!("Composite-key source partition {}:", flat_panel.instance());
    println!(
        "{}",
        pretty_format_batches(flat_panel.table(&flat_rows)?.batches())?
    );
    assert_eq!(flat.scope(&flat_panels)?.len(), 3);
    assert_eq!(flat_panel.table(&flat_rows)?.num_rows(), 2);
    Ok(())
}

fn products(schema: SchemaRef, values: &[&str]) -> Result<TableSnapshot> {
    TableSnapshot::from_batches(
        schema.clone(),
        vec![RecordBatch::try_new(
            schema,
            vec![Arc::new(StringArray::from(values.to_vec()))],
        )?],
    )
}

fn print_panels(
    result: &DataflowResult,
    regions: &ScopeHandle,
    region: &RegionHandles,
) -> Result<()> {
    let mut panels = vec![];
    for (_, parent) in result.scope(regions)?.iter() {
        panels.extend(parent.scope(&region.years)?.iter().map(|(_, panel)| panel));
    }
    panels.sort_by_key(|panel| panel.instance().to_string());
    for panel in panels {
        println!(
            "{}: threshold={}, maximum={}, rows={}",
            panel.instance(),
            panel.scalar(&region.year.threshold)?,
            panel.scalar(&region.year.maximum)?,
            panel.table(&region.year.rows)?.num_rows()
        );
        println!(
            "{}",
            pretty_format_batches(panel.table(&region.year.rows)?.batches())?
        );
    }
    println!(
        "Physical plans: {}, cache hits: {}. Partition indexing and gathering run each query.\n",
        result.report().physical_plans,
        result.report().cache_hits
    );
    Ok(())
}
