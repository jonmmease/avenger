use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;

use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_discrete_color_legend() {
    // Create test data with categories
    let categories = StringArray::from(vec!["A", "B", "C", "A", "B", "C", "A", "B", "C"]);
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 10.0, 9.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(x_values), Arc::new(y_values)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("category"), |c| {
                c.legend(|legend| legend.title("Category"))
            })
            .size_with(lit(100.0), |c| c.no_scale()),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "legend", "discrete_color_legend").await;
}

#[tokio::test]
async fn test_legend_visibility() {
    // Create the same plot but with legend disabled
    let categories = StringArray::from(vec!["A", "B", "C", "A", "B", "C", "A", "B", "C"]);
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 10.0, 9.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(x_values), Arc::new(y_values)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|scale| scale.domain((0.0, 10.0))))
            .y_with(col("y"), |c| c.scale(|scale| scale.domain((0.0, 12.0))))
            .fill_with(col("category"), |c| {
                c.scale(|scale| scale.domain_discrete(vec![lit("A"), lit("B"), lit("C")]))
                    .no_legend()
            })
            .size(100.0),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "legend",
        "legend_visibility_disabled",
    )
    .await;
}

#[tokio::test]
async fn test_continuous_color_legend() {
    // Create test data with continuous values
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 10.0, 9.0]);
    let color_values =
        Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("temperature", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(color_values),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|scale| scale.domain((0.0, 10.0))))
            .y_with(col("y"), |c| c.scale(|scale| scale.domain((0.0, 12.0))))
            .fill_with(col("temperature"), |c| {
                c.scale(|scale| scale.domain((0.0, 100.0)))
                    .legend(|legend| legend.title("Temperature"))
            })
            .size(100.0),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "legend", "continuous_color_legend").await;
}

// #[tokio::test]
// async fn test_size_legend() {
//     // Create test data with size encoding
//     let categories = StringArray::from(vec!["Small", "Medium", "Large", "Small", "Medium", "Large"]);
//     let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
//     let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0]);
//     let size_values = StringArray::from(vec!["F", "H", "T", "F", "H", "T"]);
//
//     let schema = Arc::new(Schema::new(vec![
//         Field::new("category", DataType::Utf8, false),
//         Field::new("x", DataType::Float64, false),
//         Field::new("y", DataType::Float64, false),
//         Field::new("size_value", DataType::Utf8, false),
//     ]));
//
//     let batch = RecordBatch::try_new(
//         schema,
//         vec![Arc::new(categories), Arc::new(x_values), Arc::new(y_values), Arc::new(size_values)]
//     ).unwrap();
//
//     let ctx = SessionContext::new();
//     let df = ctx.read_batch(batch).unwrap();
//
//     let plot = Plot::<Cartesian>::new()
//         .data(df)
//         .mark(
//             Symbol::new()
//                 .x_with(col("x"), |c| c.scale(|scale| scale.domain((0.0, 7.0))))
//                 .y_with(col("y"), |c| c.scale(|scale| scale.domain((0.0, 8.0))))
//                 .size(col("size_value")
//                     .scale(|scale| scale.range_discrete(vec![50.0, 100.0, 200.0]))
//                     .legend(|legend| legend.title("Size")))
//                 .fill("#4682b4")
//         );
//
//     let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
//     assert_visual_match_default(&compiled, &ctx, None, "legend", "size_legend").await;
// }

#[tokio::test]
async fn test_shape_legend() {
    // Create test data with shape encoding
    let shapes = StringArray::from(vec![
        "Type A", "Type B", "Type C", "Type A", "Type B", "Type C",
    ]);
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("shape_type", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(shapes), Arc::new(x_values), Arc::new(y_values)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|scale| scale.domain((0.0, 7.0))))
            .y_with(col("y"), |c| c.scale(|scale| scale.domain((0.0, 8.0))))
            .shape_with(col("shape_type"), |c| {
                c.scale(|scale| {
                    scale.domain_discrete(vec![lit("Type A"), lit("Type B"), lit("Type C")])
                })
                .legend(|legend| legend.title("Shape Type"))
            })
            .size(100.0)
            .fill("#4682b4"),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "legend", "shape_legend").await;
}

#[tokio::test]
async fn test_combined_fill_and_shape_legend() {
    // Create test data where both fill and shape are encoded to the same column
    let categories = StringArray::from(vec!["A", "B", "C", "A", "B", "C", "A", "B", "C"]);
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 10.0, 9.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(x_values), Arc::new(y_values)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("category"), |c| {
                c.legend(|legend| legend.title("Category"))
            }) // Legend config on channel
            .shape(col("category")) // This should result in legend with both color AND shape varying
            .size_with(lit(100.0), |c| c.no_scale()),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "legend",
        "combined_fill_and_shape_legend",
    )
    .await;
}

// TODO: Re-enable when size scales support discrete domains with numeric ranges
// #[tokio::test]
// async fn test_multiple_legends() {
//     // Create test data with multiple encodings
//     let categories = StringArray::from(vec!["A", "B", "C", "A", "B", "C"]);
//     let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
//     let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0]);
//     let size_values = StringArray::from(vec!["small", "medium", "large", "small", "medium", "large"]);
//
//     let schema = Arc::new(Schema::new(vec![
//         Field::new("category", DataType::Utf8, false),
//         Field::new("x", DataType::Float64, false),
//         Field::new("y", DataType::Float64, false),
//         Field::new("size_value", DataType::Utf8, false),
//     ]));
//
//     let batch = RecordBatch::try_new(
//         schema,
//         vec![Arc::new(categories), Arc::new(x_values), Arc::new(y_values), Arc::new(size_values)]
//     ).unwrap();
//
//     let ctx = SessionContext::new();
//     let df = ctx.read_batch(batch).unwrap();
//
//     let plot = Plot::<Cartesian>::new()
//         .data(df)
//         .mark(
//             Symbol::new()
//                 .x_with(col("x"), |c| c.scale(|scale| scale.domain((0.0, 7.0))))
//                 .y_with(col("y"), |c| c.scale(|scale| scale.domain((0.0, 8.0))))
//                 .fill(col("category")
//                     .scale(|scale| scale.domain_discrete(vec![lit("A"), lit("B"), lit("C")]))
//                     .legend(|legend| legend.title("Category")))
//                 .size(col("size_value")
//                     .scale(|scale| scale.domain_discrete(vec![lit("small"), lit("medium"), lit("large")])
//                         .range_discrete(vec![50.0, 100.0, 200.0]))
//                     .legend(|legend| legend.title("Size")))
//         );
//
//     let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
//     assert_visual_match_default(&compiled, &ctx, None, "legend", "multiple_legends").await;
// }
