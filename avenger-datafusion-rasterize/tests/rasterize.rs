//! Raster queries bin rows into pixel cells through their scales and aggregate each cell's rows.

use std::sync::Arc;

use arrow::array::{Array, ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use avenger_datafusion_rasterize::{Raster, Rasterize};
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::log::LogScale;
use datafusion::common::ScalarValue;
use datafusion::datasource::MemTable;
use datafusion::functions_aggregate::count::count_all;
use datafusion::functions_aggregate::expr_fn::{
    approx_percentile_cont, avg, count, first_value, sum,
};
use datafusion::logical_expr::{col, lit, ExprFunctionExt};
use datafusion::physical_plan::{collect, displayable};
use datafusion::prelude::{DataFrame, SessionConfig, SessionContext};

/// The partitions that each table's rows split into, so each query aggregates partial grids and
/// merges them.
const PARTITIONS: usize = 3;

fn frame(columns: Vec<(&str, ArrayRef)>) -> DataFrame {
    let batch = RecordBatch::try_from_iter(columns).unwrap();
    let rows = batch.num_rows();
    let chunk = rows.div_ceil(PARTITIONS);
    let partitions = (0..PARTITIONS)
        .map(|partition| {
            let start = (partition * chunk).min(rows);
            vec![batch.slice(start, chunk.min(rows - start))]
        })
        .collect();
    let table = MemTable::try_new(batch.schema(), partitions).unwrap();
    let config = SessionConfig::new().with_target_partitions(PARTITIONS);
    SessionContext::new_with_config(config)
        .read_table(Arc::new(table))
        .unwrap()
}

fn floats(values: &[Option<f64>]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

async fn rasters(query: &Rasterize, df: DataFrame) -> Vec<Raster> {
    let plan = query.plan(df).unwrap();
    let context = Arc::new(plan.task_ctx());
    let physical = plan.create_physical_plan().await.unwrap();
    let shown = displayable(physical.as_ref()).indent(false).to_string();
    assert!(shown.contains("mode=Partial"), "{shown}");
    query
        .pack(&collect(physical, context).await.unwrap())
        .unwrap()
}

/// A plane's cells, with `None` where no row reached.
fn cells(plane: &Float64Array) -> Vec<Option<f64>> {
    plane.iter().collect()
}

/// A count raster matches a direct count of the rows in each cell, 0 where no row lies. Rows
/// outside the domains, and rows without values, count nowhere, and a row at a domain's end
/// counts in the edge cell. An aggregate under aliases counts the same.
#[tokio::test]
async fn a_count_raster_counts_the_rows_in_each_cell() {
    // Cell centers and edges in a 4 × 3 grid over x in [0, 4] and y in [0, 3], and rows outside.
    let xs = [
        Some(0.5),
        Some(0.0),
        Some(4.0),
        Some(3.5),
        Some(3.5),
        Some(1.5),
        Some(-0.1),
        Some(4.1),
        Some(2.5),
        Some(f64::NAN),
        Some(2.5),
        None,
        Some(1.5),
    ];
    let ys = [
        Some(0.5),
        Some(3.0),
        Some(0.0),
        Some(2.5),
        Some(2.5),
        Some(1.5),
        Some(1.0),
        Some(1.0),
        Some(3.1),
        Some(1.0),
        Some(-0.1),
        Some(1.0),
        None,
    ];
    let df = frame(vec![("x", floats(&xs)), ("y", floats(&ys))]);
    let query = Rasterize::new(
        col("x"),
        &LinearScale::configured((0.0, 4.0), (0.0, 1.0)),
        4,
    )
    .with_y(
        col("y"),
        &LinearScale::configured((0.0, 3.0), (0.0, 1.0)),
        3,
    )
    .aggregate(count(lit(1)))
    .aggregate(count_all().alias("count"));
    let rasters = rasters(&query, df).await;
    assert_eq!(rasters.len(), 1);
    assert_eq!(rasters[0].planes[1], rasters[0].planes[0]);

    // Row 0 is the top, at y's upper end.
    let mut expected = vec![0.0; 12];
    for (x, y) in xs.iter().zip(ys) {
        if let (Some(x), Some(y)) = (*x, y) {
            if (0.0..=4.0).contains(&x) && (0.0..=3.0).contains(&y) {
                let column = (x.floor() as usize).min(3);
                let row = ((3.0 - y).floor() as usize).min(2);
                expected[row * 4 + column] += 1.0;
            }
        }
    }
    assert_eq!(rasters[0].planes[0].values().to_vec(), expected);
    assert_eq!(rasters[0].planes[0].null_count(), 0);
    assert_eq!(expected.iter().sum::<f64>(), 6.0);
    assert_eq!((rasters[0].width, rasters[0].height), (4, 3));
}

/// Each aggregate fills its own plane, with its literal arguments, its ordering when it takes
/// one, and its filter, and each group gets its own raster. Cells without rows have no value,
/// even in a group after one whose rows reach every cell.
#[tokio::test]
async fn aggregates_fill_planes_and_groups_get_rasters() {
    let df = frame(vec![
        ("x", floats(&[0.5, 0.5, 0.5, 1.5, 2.5, 0.5].map(Some))),
        ("v", floats(&[1.0, 2.0, 6.0, 4.0, 8.0, 10.0].map(Some))),
        ("t", Arc::new(Int64Array::from(vec![3, 1, 2, 1, 5, 0]))),
        (
            "kind",
            Arc::new(StringArray::from(vec!["a", "a", "a", "a", "a", "b"])),
        ),
    ]);
    let by_t = || vec![col("t").sort(true, false)];
    let query = Rasterize::new(
        col("x"),
        &LinearScale::configured((0.0, 3.0), (0.0, 1.0)),
        3,
    )
    .aggregate(avg(col("v")))
    .aggregate(first_value(col("v"), by_t()))
    // `sum` ignores its rows' order.
    .aggregate(sum(col("v")).order_by(by_t()).build().unwrap())
    .aggregate(approx_percentile_cont(
        col("v").sort(true, false),
        lit(0.5),
        None,
    ))
    .aggregate(count(lit(1)).filter(col("v").gt(lit(2.0))).build().unwrap())
    .group_by(col("kind"));
    let rasters = rasters(&query, df).await;
    let groups: Vec<_> = rasters.iter().map(|raster| raster.group.clone()).collect();
    let kind = |kind: &str| vec![ScalarValue::Utf8(Some(kind.to_string()))];
    assert_eq!(groups, [kind("a"), kind("b")]);
    let planes = |raster: &Raster| raster.planes.iter().map(cells).collect::<Vec<_>>();
    assert_eq!(
        planes(&rasters[0]),
        [
            [Some(3.0), Some(4.0), Some(8.0)],
            [Some(2.0), Some(4.0), Some(8.0)],
            [Some(9.0), Some(4.0), Some(8.0)],
            [Some(2.0), Some(4.0), Some(8.0)],
            [Some(1.0), Some(1.0), Some(1.0)],
        ]
    );
    assert_eq!(
        planes(&rasters[1]),
        [
            [Some(10.0), None, None],
            [Some(10.0), None, None],
            [Some(10.0), None, None],
            [Some(10.0), None, None],
            [Some(1.0), Some(0.0), Some(0.0)],
        ]
    );
}

/// A log scale bins through the scale, so each decade of its domain gets an equal share of the
/// columns, and a query without y bins into one row.
#[tokio::test]
async fn a_log_scale_bins_equal_ratios_into_equal_widths() {
    let xs = [5.0, 50.0, 500.0, 5000.0, 10000.0, 0.5].map(Some);
    let df = frame(vec![("x", floats(&xs))]);
    let query = Rasterize::new(
        col("x"),
        &LogScale::configured((1.0, 10000.0), (0.0, 1.0)),
        4,
    )
    .aggregate(count(lit(1)));
    let rasters = rasters(&query, df).await;
    assert_eq!(
        cells(&rasters[0].planes[0]),
        [Some(1.0), Some(1.0), Some(1.0), Some(2.0)]
    );
    assert_eq!(rasters[0].height, 1);
}
