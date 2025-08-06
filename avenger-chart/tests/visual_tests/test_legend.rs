use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::coords::Cartesian;
use avenger_chart::marks::ChannelExpr;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
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

    let plot = Plot::new(Cartesian)
        .preferred_size(600.0, 400.0)
        .data(df)
        .legend_fill(|legend| legend.title("Category"))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("category"))
                .size(lit(100.0).identity()),
        );

    assert_visual_match_default(plot, "legend", "discrete_color_legend").await;
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

    let plot = Plot::new(Cartesian)
        .preferred_size(600.0, 400.0)
        .data(df)
        .scale_x(|scale| scale.domain((0.0, 10.0)))
        .scale_y(|scale| scale.domain((0.0, 12.0)))
        .scale_fill(|scale| scale.domain_discrete(vec![lit("A"), lit("B"), lit("C")]))
        .legend_fill(|legend| legend.visible(false))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("category"))
                .size(100.0),
        );

    assert_visual_match_default(plot, "legend", "legend_visibility_disabled").await;
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

    let plot = Plot::new(Cartesian)
        .preferred_size(600.0, 400.0)
        .data(df)
        .scale_x(|scale| scale.domain((0.0, 10.0)))
        .scale_y(|scale| scale.domain((0.0, 12.0)))
        .scale_fill(|scale| scale.domain((0.0, 100.0)))
        .legend_fill(|legend| legend.title("Temperature"))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("temperature"))
                .size(100.0),
        );

    assert_visual_match_default(plot, "legend", "continuous_color_legend").await;
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
//     let plot = Plot::new(Cartesian)
//         .preferred_size(600.0, 400.0)
//         .data(df)
//         .scale_x(|scale| scale.domain((0.0, 7.0)))
//         .scale_y(|scale| scale.domain((0.0, 8.0)))
//         .scale_size(|scale| scale.range_discrete(vec![lit(50.0), lit(100.0), lit(200.0)]))
//         .legend_size(|legend| legend.title("Size"))
//         .mark(
//             Symbol::new()
//                 .x(col("x"))
//                 .y(col("y"))
//                 .size(col("size_value"))
//                 .fill("#4682b4")
//         );
//
//     assert_visual_match_default(plot, "legend", "size_legend").await;
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

    let plot = Plot::new(Cartesian)
        .preferred_size(600.0, 400.0)
        .data(df)
        .scale_x(|scale| scale.domain((0.0, 7.0)))
        .scale_y(|scale| scale.domain((0.0, 8.0)))
        .scale_shape(|scale| {
            scale.domain_discrete(vec![lit("Type A"), lit("Type B"), lit("Type C")])
        })
        .legend_shape(|legend| legend.title("Shape Type"))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .shape(col("shape_type"))
                .size(100.0)
                .fill("#4682b4"),
        );

    assert_visual_match_default(plot, "legend", "shape_legend").await;
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

    let plot = Plot::new(Cartesian)
        .preferred_size(600.0, 400.0)
        .data(df)
        .legend_fill(|legend| legend.title("Category"))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("category")) // Both fill and shape map to same column
                .shape(col("category")) // This should result in legend with both color AND shape varying
                .size(lit(100.0).identity()),
        );

    assert_visual_match_default(plot, "legend", "combined_fill_and_shape_legend").await;
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
//     let plot = Plot::new(Cartesian)
//         .preferred_size(600.0, 400.0)
//         .data(df)
//         .scale_x(|scale| scale.domain((0.0, 7.0)))
//         .scale_y(|scale| scale.domain((0.0, 8.0)))
//         .scale_fill(|scale| scale.domain_discrete(vec![lit("A"), lit("B"), lit("C")]))
//         .scale_size(|scale| scale.domain_discrete(vec![lit("small"), lit("medium"), lit("large")])
//             .range_discrete(vec![lit(50.0), lit(100.0), lit(200.0)]))
//         .legend_fill(|legend| legend.title("Category"))
//         .legend_size(|legend| legend.title("Size"))
//         .mark(
//             Symbol::new()
//                 .x(col("x"))
//                 .y(col("y"))
//                 .fill(col("category"))
//                 .size(col("size_value"))
//         );
//
//     assert_visual_match_default(plot, "legend", "multiple_legends").await;
// }
