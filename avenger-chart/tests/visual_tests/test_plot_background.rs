use super::helpers::assert_visual_match_default;
use avenger_chart::cartesian::guide::CartesianGuide;
use avenger_chart::polar::guide::PolarGuide;
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::future::Future;
use std::sync::Arc;

fn run_with_large_stack<F, Fut>(f: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name("plot-background-visual-large-stack".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build tokio runtime for plot background visual test");
            rt.block_on(f());
        })
        .expect("spawn large-stack plot background visual test thread")
        .join()
        .expect("large-stack plot background visual test panicked");
}

/// Create test data for scatter plot
fn create_test_data() -> DataFrame {
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let y_values = Float64Array::from(vec![10.0, 20.0, 15.0, 25.0, 30.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)]).unwrap();

    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

#[test]
fn test_cartesian_plot_background() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = create_test_data();

        // Create a plot with light blue background
        let plot = Plot::<Cartesian>::new()
            .data(df)
            .configure_guide(CartesianGuide::new().plot_background_color("#e6f2ff")) // Light blue
            .mark(
                Line::new()
                    .x_with(col("x"), |c| c.scale(|s| s))
                    .y_with(col("y"), |c| c.scale(|s| s))
                    .stroke("#e74c3c")
                    .stroke_width(3.0),
            );

        let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "plot_background",
            "cartesian_background",
        )
        .await;
    });
}

#[test]
fn test_cartesian_background_with_grid() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = create_test_data();

        // Create a scatter plot with dark background to show grid lines clearly
        let plot = Plot::<Cartesian>::new()
            .data(df)
            .configure_guide(CartesianGuide::new().plot_background_color("#33334d")) // Dark blue-gray
            .mark(
                Symbol::new()
                    .x_with(col("x"), |c| c.scale(|s| s).axis(|a| a.grid(true)))
                    .y_with(col("y"), |c| c.scale(|s| s).axis(|a| a.grid(true)))
                    .size(100.0)
                    .fill("#f39c12")
                    .stroke("#ffffff")
                    .stroke_width(2.0),
            );

        let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "plot_background",
            "cartesian_dark_with_grid",
        )
        .await;
    });
}

#[test]
fn test_polar_plot_background() {
    run_with_large_stack(|| async {
        use datafusion::arrow::array::StringArray;

        // Create polar data
        let radius_values = Float64Array::from(vec![30.0, 45.0, 60.0, 75.0, 90.0]);
        let theta_values = Float64Array::from(vec![0.0, 1.57, 3.14, 4.71, 6.28]);
        let categories = StringArray::from(vec!["A", "B", "C", "D", "E"]);

        let schema = Arc::new(Schema::new(vec![
            Field::new("radius", DataType::Float64, false),
            Field::new("theta", DataType::Float64, false),
            Field::new("category", DataType::Utf8, false),
        ]));

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(radius_values),
                Arc::new(theta_values),
                Arc::new(categories),
            ],
        )
        .unwrap();

        let ctx = SessionContext::new();
        let df = ctx.read_batch(batch).unwrap();

        // Create a polar plot with light green background
        let plot = Plot::<Polar>::new()
            .data(df)
            .configure_guide(PolarGuide::new().plot_background_color("#e6ffe6")) // Light green
            .mark(
                Symbol::<Polar>::new()
                    .r(col("radius"))
                    .theta(col("theta"))
                    .size(150.0)
                    .fill("#e74c3c")
                    .stroke("#2c3e50")
                    .stroke_width(2.0),
            );

        let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
        assert_visual_match_default(&compiled, &ctx, None, "plot_background", "polar_background")
            .await;
    });
}

#[test]
fn test_polar_background_with_grid() {
    run_with_large_stack(|| async {
        // Create more data points for a fuller polar plot
        let mut radius_vals = Vec::new();
        let mut theta_vals = Vec::new();

        for i in 0..12 {
            let angle = (i as f64) * (2.0 * std::f64::consts::PI / 12.0);
            radius_vals.push(40.0 + (i as f64) * 5.0);
            theta_vals.push(angle);
        }

        let radius_values = Float64Array::from(radius_vals);
        let theta_values = Float64Array::from(theta_vals);

        let schema = Arc::new(Schema::new(vec![
            Field::new("r", DataType::Float64, false),
            Field::new("theta", DataType::Float64, false),
        ]));

        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(radius_values), Arc::new(theta_values)],
        )
        .unwrap();

        let ctx = SessionContext::new();
        let df = ctx.read_batch(batch).unwrap();

        // Create a polar plot with dark background to show grid lines
        let plot = Plot::<Polar>::new()
            .data(df)
            .configure_guide(PolarGuide::new().plot_background_color("#262633")) // Dark background
            .mark(
                Symbol::<Polar>::new()
                    .r(col("r"))
                    .theta(col("theta"))
                    .size(80.0)
                    .fill("#00ff00")
                    .stroke("#ffffff")
                    .stroke_width(1.0),
            );

        let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "plot_background",
            "polar_dark_with_grid",
        )
        .await;
    });
}
