#[cfg(test)]
mod tests {
    use avenger_chart::coords::Cartesian;
    use avenger_chart::marks::symbol::Symbol;
    use avenger_chart::plot::Plot;
    use datafusion::arrow::array::{Float64Array, StringArray};
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::record_batch::RecordBatch;
    use datafusion::logical_expr::{col, lit};
    use datafusion::prelude::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_shape_ordinal_scale_automatic() {
        // Create data with shape categories
        let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let y_values = Float64Array::from(vec![2.0, 3.0, 2.5, 3.5, 2.8, 3.2]);
        let shape_values = StringArray::from(vec![
            "circle", "square", "diamond", "circle", "square", "diamond",
        ]);

        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("shape", DataType::Utf8, false),
        ]));

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(x_values),
                Arc::new(y_values),
                Arc::new(shape_values),
            ],
        )
        .expect("Failed to create RecordBatch");

        let ctx = SessionContext::new();
        let df = ctx
            .read_batch(batch)
            .expect("Failed to read batch into DataFrame");

        // Create plot using col("shape") without .identity()
        // This should automatically create an ordinal scale with shape strings as range
        let plot = Plot::new(Cartesian).data(df).mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .shape(col("shape"))
                .size(lit(150.0))
                .fill("#4682b4")
                .stroke("#000000")
                .stroke_width(lit(2.0)),
        );

        // Test rendering
        let renderer = avenger_chart::render::PlotRenderer::new(&plot);
        let result = renderer.render().await;
        match &result {
            Ok(_) => println!("Render succeeded with automatic ordinal scale"),
            Err(e) => println!("Render failed with error: {:?}", e),
        }
        assert!(
            result.is_ok(),
            "Render should succeed with automatic ordinal scale"
        );

        let render_result = result.unwrap();
        let marks = &render_result.scene_graph.marks;

        // Find symbol mark within groups
        fn find_symbol_mark(
            mark: &avenger_scenegraph::marks::mark::SceneMark,
        ) -> Option<&avenger_scenegraph::marks::symbol::SceneSymbolMark> {
            match mark {
                avenger_scenegraph::marks::mark::SceneMark::Symbol(s) => Some(s),
                avenger_scenegraph::marks::mark::SceneMark::Group(g) => {
                    for m in &g.marks {
                        if let Some(s) = find_symbol_mark(m) {
                            return Some(s);
                        }
                    }
                    None
                }
                _ => None,
            }
        }

        let symbol = find_symbol_mark(&marks[0]).expect("Should find symbol mark");
        assert_eq!(symbol.len, 6);
        // Should have 3 unique shapes (circle, square, diamond)
        assert_eq!(symbol.shapes.len(), 3);

        // Verify we have different shape types
        let has_circle = symbol
            .shapes
            .iter()
            .any(|s| matches!(s, avenger_common::types::SymbolShape::Circle));
        let has_paths = symbol
            .shapes
            .iter()
            .any(|s| matches!(s, avenger_common::types::SymbolShape::Path(_)));

        assert!(has_circle, "Should have circle shape");
        assert!(has_paths, "Should have path-based shapes (square, diamond)");
    }

    #[tokio::test]
    async fn test_custom_enumeration_ordinal_scale() {
        // Create data with custom enumeration values
        let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
        let y_values = Float64Array::from(vec![10.0, 25.0, 15.0, 30.0, 20.0, 35.0, 18.0, 28.0]);
        let category_values = StringArray::from(vec![
            "low", "high", "medium", "high", "medium", "high", "low", "medium",
        ]);

        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("category", DataType::Utf8, false),
        ]));

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(x_values),
                Arc::new(y_values),
                Arc::new(category_values),
            ],
        )
        .expect("Failed to create RecordBatch");

        let ctx = SessionContext::new();
        let df = ctx
            .read_batch(batch)
            .expect("Failed to read batch into DataFrame");

        // Create plot using col("category") for shape mapping
        // This demonstrates automatic ordinal scale creation for any string enumeration
        let plot = Plot::new(Cartesian).data(df).mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .shape(col("category")) // Maps "low", "medium", "high" to shapes
                .fill(col("category")) // Also use for color
                .size(lit(200.0)),
        );

        // Test rendering
        let renderer = avenger_chart::render::PlotRenderer::new(&plot);
        let result = renderer.render().await;
        assert!(
            result.is_ok(),
            "Render should succeed with custom enumeration"
        );

        let render_result = result.unwrap();
        let marks = &render_result.scene_graph.marks;

        // Find symbol mark
        fn find_symbol_mark(
            mark: &avenger_scenegraph::marks::mark::SceneMark,
        ) -> Option<&avenger_scenegraph::marks::symbol::SceneSymbolMark> {
            match mark {
                avenger_scenegraph::marks::mark::SceneMark::Symbol(s) => Some(s),
                avenger_scenegraph::marks::mark::SceneMark::Group(g) => {
                    for m in &g.marks {
                        if let Some(s) = find_symbol_mark(m) {
                            return Some(s);
                        }
                    }
                    None
                }
                _ => None,
            }
        }

        let symbol = find_symbol_mark(&marks[0]).expect("Should find symbol mark");
        assert_eq!(symbol.len, 8);
        // Should have 3 unique shapes for 3 categories
        assert_eq!(symbol.shapes.len(), 3);
    }
}
