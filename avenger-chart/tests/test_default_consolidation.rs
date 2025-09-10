//! Test that verifies mark default values are consistently used

use avenger_chart::prelude::*;
use avenger_chart::render_context::RenderContext;
use avenger_chart::theme::Theme;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;
use std::sync::Arc;

#[tokio::test]
async fn test_symbol_defaults_used_in_rendering() {
    // Create a simple symbol mark without explicitly setting values
    let ctx = SessionContext::new();
    let _df = Arc::new(ctx.read_empty().unwrap());

    let symbol = Symbol::<Cartesian>::new().x(col("x")).y(col("y"));

    // Create a RenderContext with default theme
    let theme = Theme::default();
    let context = RenderContext::new(theme, 500.0, 400.0);

    // Get the mark's default values
    let size_default = symbol.default_channel_value("size", &context).unwrap();
    let fill_default = symbol.default_channel_value("fill", &context).unwrap();
    let stroke_default = symbol.default_channel_value("stroke", &context).unwrap();
    let stroke_width_default = symbol
        .default_channel_value("stroke_width", &context)
        .unwrap();
    let shape_default = symbol.default_channel_value("shape", &context).unwrap();

    // Expected values from our consolidation
    assert_eq!(size_default, ScalarValue::Float32(Some(64.0)));
    assert_eq!(fill_default, ScalarValue::Utf8(Some("#4682b4".to_string())));
    assert_eq!(
        stroke_default,
        ScalarValue::Utf8(Some("#000000".to_string()))
    );
    assert_eq!(stroke_width_default, ScalarValue::Float32(Some(1.0)));
    assert_eq!(shape_default, ScalarValue::Utf8(Some("circle".to_string())));

    // The symbol mark instance can directly render

    // Create a simple data batch with one point
    use datafusion::arrow::array::Float64Array;
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::record_batch::RecordBatch;

    let x_values = Float64Array::from(vec![10.0]);
    let y_values = Float64Array::from(vec![20.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)]).unwrap();

    // Create an empty scalar batch for this test
    let empty_schema = Arc::new(Schema::empty());
    let scalar_batch = RecordBatch::new_empty(empty_schema);

    // Create a render context with default theme and dummy dimensions
    let theme = avenger_chart::theme::Theme::default();
    let context = avenger_chart::render_context::RenderContext::new(theme, 500.0, 500.0);

    // Create a Cartesian coordinate system
    let coord = avenger_chart::cartesian::Cartesian::default();

    // Render the mark
    let rendered = symbol
        .render_from_data(Some(&batch), &scalar_batch, &context, &coord)
        .unwrap();

    // Check that the rendered mark uses our defaults
    if let avenger_scenegraph::marks::mark::SceneMark::Symbol(scene_symbol) = &rendered[0] {
        // Size should be 64.0
        if let avenger_common::value::ScalarOrArrayValue::Scalar(size) = scene_symbol.size.value() {
            assert_eq!(*size, 64.0);
        } else {
            panic!("Expected scalar size");
        }

        // Stroke width should be 1.0
        assert_eq!(scene_symbol.stroke_width, Some(1.0));

        // Fill should be our default blue
        if let avenger_common::value::ScalarOrArrayValue::Scalar(fill) = scene_symbol.fill.value() {
            if let avenger_common::types::ColorOrGradient::Color(color) = fill {
                // Check if it's approximately our blue color (#4682b4)
                assert!((color[0] - 70.0 / 255.0).abs() < 0.01);
                assert!((color[1] - 130.0 / 255.0).abs() < 0.01);
                assert!((color[2] - 180.0 / 255.0).abs() < 0.01);
                assert_eq!(color[3], 1.0);
            } else {
                panic!("Expected solid color");
            }
        } else {
            panic!("Expected scalar fill");
        }

        // Shape should be circle
        assert_eq!(scene_symbol.shapes.len(), 1);
        assert!(matches!(
            scene_symbol.shapes[0],
            avenger_common::types::SymbolShape::Circle
        ));
    } else {
        panic!("Expected symbol mark");
    }
}
