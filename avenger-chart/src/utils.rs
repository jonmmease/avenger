pub use avenger_chart_core::color::{
    parse_color_string, parse_color_string_strict, parse_color_to_array,
    parse_color_to_array_strict,
};

pub use avenger_chart_core::datafusion_utils::{
    ArrayRefHelpers, DataFrameChartHelpers, ExprHelpers, ScalarValueHelpers, array_value_to_f64,
    contains_aggregate, eval_to_scalars, params_to_datafusion, partition_expressions,
    scalar_to_scalar_value, simplify_to_scalar_sync,
};

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::{
        arrow::{
            array::{Float32Array, StringArray},
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        prelude::SessionContext,
        scalar::ScalarValue,
    };

    use super::*;

    #[test]
    fn test_parse_white_color() {
        let white = parse_color_to_array("#FFFFFF");
        eprintln!("Parsed white: {:?}", white);
        assert!(
            white[0] > 0.99 && white[0] <= 1.0,
            "Red should be ~1.0, got {}",
            white[0]
        );
        assert!(
            white[1] > 0.99 && white[1] <= 1.0,
            "Green should be ~1.0, got {}",
            white[1]
        );
        assert!(
            white[2] > 0.99 && white[2] <= 1.0,
            "Blue should be ~1.0, got {}",
            white[2]
        );
        assert_eq!(white[3], 1.0, "Alpha should be 1.0");
    }

    #[test]
    fn test_parse_black_color() {
        let black = parse_color_to_array("#000000");
        eprintln!("Parsed black: {:?}", black);
        assert!(black[0] < 0.01, "Red should be ~0.0, got {}", black[0]);
        assert!(black[1] < 0.01, "Green should be ~0.0, got {}", black[1]);
        assert!(black[2] < 0.01, "Blue should be ~0.0, got {}", black[2]);
        assert_eq!(black[3], 1.0, "Alpha should be 1.0");
    }

    #[tokio::test]
    async fn test_span_numeric_columns() {
        // Create test data with numeric columns
        let ctx = SessionContext::new();

        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Float32, false),
            Field::new("b", DataType::Float32, false),
            Field::new("c", DataType::Float32, false),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float32Array::from(vec![4.0, 5.0, 6.0])),
                Arc::new(Float32Array::from(vec![7.0, 8.0, 9.0])),
            ],
        )
        .unwrap();

        let df = ctx.read_batch(batch).unwrap();

        // Create span expression
        let span_expr = df.span().unwrap();

        // The span expression should be a subquery that computes min/max across all numeric columns
        // When evaluated, it should return [1.0, 9.0] since 1.0 is the min and 9.0 is the max
        let result = span_expr.eval_to_scalar(Some(&ctx), None).await.unwrap();
        let span = result.as_f32x2().unwrap();

        assert_eq!(span[0], 1.0);
        assert_eq!(span[1], 9.0);
    }

    #[tokio::test]
    async fn test_unique_values_string_columns() {
        // Create test data with string columns
        let ctx = SessionContext::new();

        let schema = Arc::new(Schema::new(vec![
            Field::new("col1", DataType::Utf8, false),
            Field::new("col2", DataType::Utf8, false),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec!["A", "B", "A"])),
                Arc::new(StringArray::from(vec!["B", "C", "D"])),
            ],
        )
        .unwrap();

        let df = ctx.read_batch(batch).unwrap();

        // Create unique values expression
        let unique_expr = df.unique_values().unwrap();

        // The unique values expression should return ["A", "B", "C", "D"]
        let result = unique_expr.eval_to_scalar(Some(&ctx), None).await.unwrap();

        if let ScalarValue::List(array) = result {
            let values_vec = array.value(0).to_scalar_vec().unwrap();
            let mut values: Vec<String> = values_vec
                .iter()
                .map(|v| v.as_scalar_string().unwrap())
                .collect();
            values.sort();
            assert_eq!(values, vec!["A", "B", "C", "D"]);
        } else {
            panic!("Expected List result");
        }
    }

    #[tokio::test]
    async fn test_span_mixed_numeric_types() {
        // Test with different numeric types (int32, float64, etc)
        let ctx = SessionContext::new();

        let schema = Arc::new(Schema::new(vec![
            Field::new("int_col", DataType::Int32, false),
            Field::new("float_col", DataType::Float64, false),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(arrow::array::Int32Array::from(vec![10, 20, 30])),
                Arc::new(arrow::array::Float64Array::from(vec![5.5, 15.5, 25.5])),
            ],
        )
        .unwrap();

        let df = ctx.read_batch(batch).unwrap();

        // Create span expression
        let span_expr = df.span().unwrap();

        // Should find min=5.5 and max=30.0 across both columns
        let result = span_expr.eval_to_scalar(Some(&ctx), None).await.unwrap();
        let span = result.as_f32x2().unwrap();

        assert_eq!(span[0], 5.5);
        assert_eq!(span[1], 30.0);
    }

    #[tokio::test]
    async fn test_all_values() {
        // Test all_values which should return all values (including duplicates)
        let ctx = SessionContext::new();

        let schema = Arc::new(Schema::new(vec![
            Field::new("col1", DataType::Utf8, false),
            Field::new("col2", DataType::Utf8, false),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec!["A", "B", "A"])),
                Arc::new(StringArray::from(vec!["B", "C", "A"])),
            ],
        )
        .unwrap();

        let df = ctx.read_batch(batch).unwrap();

        // Create all values expression
        let all_expr = df.all_values().unwrap();

        // Should return all 6 values: ["A", "B", "A", "B", "C", "A"]
        let result = all_expr.eval_to_scalar(Some(&ctx), None).await.unwrap();

        if let ScalarValue::List(array) = result {
            let values_vec = array.value(0).to_scalar_vec().unwrap();
            assert_eq!(values_vec.len(), 6);

            // Count occurrences
            let values: Vec<String> = values_vec
                .iter()
                .map(|v| v.as_scalar_string().unwrap())
                .collect();

            assert_eq!(values.iter().filter(|v| v == &"A").count(), 3);
            assert_eq!(values.iter().filter(|v| v == &"B").count(), 2);
            assert_eq!(values.iter().filter(|v| v == &"C").count(), 1);
        } else {
            panic!("Expected List result");
        }
    }
}
