#[cfg(test)]
mod tests {
    use avenger_chart::coords::Cartesian;
    use avenger_chart::marks::ChannelExpr;
    use avenger_chart::marks::symbol::Symbol;
    use avenger_chart::plot::Plot;
    use avenger_chart::render::CanvasExt;
    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};
    use datafusion::arrow::array::Float64Array;
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::record_batch::RecordBatch;
    use datafusion::logical_expr::col;
    use datafusion::prelude::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_symbol_sizes() {
        let ctx = SessionContext::new();

        // Test 1: Column with multiple values
        let x1 = Float64Array::from(vec![1.0, 2.0, 3.0]);
        let y1 = Float64Array::from(vec![1.0, 1.0, 1.0]);
        let size1 = Float64Array::from(vec![50.0, 100.0, 150.0]);

        let schema1 = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("size", DataType::Float64, false),
        ]));

        let batch1 =
            RecordBatch::try_new(schema1, vec![Arc::new(x1), Arc::new(y1), Arc::new(size1)])
                .unwrap();

        let df1 = ctx.read_batch(batch1).unwrap();

        // Test 2: All same values
        let x2 = Float64Array::from(vec![1.0, 2.0, 3.0]);
        let y2 = Float64Array::from(vec![2.0, 2.0, 2.0]);
        let size2 = Float64Array::from(vec![100.0, 100.0, 100.0]);

        let schema2 = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("size", DataType::Float64, false),
        ]));

        let batch2 =
            RecordBatch::try_new(schema2, vec![Arc::new(x2), Arc::new(y2), Arc::new(size2)])
                .unwrap();

        let df2 = ctx.read_batch(batch2).unwrap();

        // Create plot
        let plot = Plot::new(Cartesian)
            .scale_x(|s| s.domain((0.0, 4.0)))
            .scale_y(|s| s.domain((0.0, 3.0)))
            // Row 1: Different sizes
            .mark(
                Symbol::new()
                    .data(df1)
                    .x(col("x"))
                    .y(col("y"))
                    .size(col("size").identity())
                    .fill("#ff0000"),
            )
            // Row 2: Same sizes
            .mark(
                Symbol::new()
                    .data(df2)
                    .x(col("x"))
                    .y(col("y"))
                    .size(col("size").identity())
                    .fill("#0000ff"),
            )
            // Row 3: Literal size
            .mark(
                Symbol::new()
                    .data(
                        ctx.read_batch(RecordBatch::new_empty(Arc::new(Schema::empty())))
                            .unwrap(),
                    )
                    .x(2.0)
                    .y(2.5)
                    .size(100.0)
                    .fill("#00ff00"),
            );

        // Render
        let dimensions = CanvasDimensions {
            size: [400.0, 300.0],
            scale: 2.0,
        };
        let config = CanvasConfig::default();

        let mut canvas = PngCanvas::new(dimensions, config)
            .await
            .expect("Failed to create canvas");

        canvas
            .render_plot(&plot)
            .await
            .expect("Failed to render plot");

        let img = canvas.render().await.expect("Failed to render image");
        img.save("symbol_size_test.png")
            .expect("Failed to save image");

        println!("Saved test image to symbol_size_test.png");
        println!("Row 1 (red): Different sizes [50, 100, 150] with .identity()");
        println!("Row 2 (blue): Same sizes [100, 100, 100] with .identity()");
        println!("Row 3 (green): Single literal size 100");
    }
}
