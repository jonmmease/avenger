#[cfg(test)]
mod tests {
    use avenger_chart::prelude::*;
    use datafusion::arrow::array::Float64Array;
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::record_batch::RecordBatch;
    use datafusion::prelude::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_symbol_rendering() {
        let ctx = SessionContext::new();

        // Test case 1: Single value array
        let x = Float64Array::from(vec![100.0]);
        let y = Float64Array::from(vec![100.0]);
        let size = Float64Array::from(vec![100.0]);

        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("size", DataType::Float64, false),
        ]));

        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(x), Arc::new(y), Arc::new(size)]).unwrap();

        let df = ctx.read_batch(batch).unwrap();

        // Create plots with different size configurations
        let configs = vec![
            (
                "literal_100",
                Symbol::new()
                    .data(df.clone())
                    .x(col("x"))
                    .y(col("y"))
                    .size(100.0)
                    .fill("#ff0000"),
            ),
            (
                "column_identity",
                Symbol::new()
                    .data(df.clone())
                    .x(col("x"))
                    .y(col("y"))
                    .size(col("size"))
                    .fill("#00ff00"),
            ),
            (
                "column_scaled",
                Symbol::new()
                    .data(df.clone())
                    .x(col("x"))
                    .y(col("y"))
                    .size(col("size"))
                    .fill("#0000ff"),
            ),
        ];

        use avenger_chart::render::CanvasExt;
        use avenger_common::canvas::CanvasDimensions;
        use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};

        let dimensions = CanvasDimensions {
            size: [300.0, 300.0],
            scale: 2.0,
        };

        for (name, mut symbol) in configs {
            // Configure the scales on the symbol's channels
            symbol = symbol
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 200.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 200.0))));

            if name == "column_scaled" {
                symbol = symbol.size_with(col("size"), |c| {
                    c.scale(|s| s.range_interval(lit(16.0), lit(64.0)))
                });
            }

            let plot = Plot::<Cartesian>::new().mark(symbol);
            let compiled = plot.compile(&ctx).await.unwrap();

            let mut canvas = PngCanvas::new(dimensions, CanvasConfig::default())
                .await
                .expect("Failed to create canvas");

            canvas
                .render_plot(&compiled, &ctx, None)
                .await
                .expect("Failed to render plot");

            // Verify rendering succeeded without saving files
            let _img = canvas.render().await.expect("Failed to render image");

            // Test passed if we get here without errors
            println!("Successfully rendered symbol with {} configuration", name);
        }
    }
}
