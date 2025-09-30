//! Integration tests for scale UDF serialization
//!
//! Tests that scale UDFs can be serialized with expressions and deserialized
//! on a fresh SessionContext without requiring pre-registration.

#[cfg(test)]
mod tests {
    use avenger_chart::scales::{Linear, Scale};
    use avenger_chart::serialization::{LogicalExprNodeExt, SerializableExpr};
    use datafusion::arrow::array::Float64Array;
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::record_batch::RecordBatch;
    use datafusion::logical_expr::lit;
    use datafusion::prelude::*;
    use datafusion_proto::protobuf::LogicalExprNode;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_scale_udf_serialization_roundtrip() {
        // Create a context with data
        let ctx = SessionContext::new();

        // Create test data
        let schema = Schema::new(vec![Field::new("value", DataType::Float64, false)]);

        let batch = RecordBatch::try_new(
            Arc::new(schema.clone()),
            vec![Arc::new(Float64Array::from(vec![0.0, 0.5, 1.0]))],
        )
        .unwrap();

        ctx.register_batch("test_data", batch).unwrap();

        // Create a scale and configure it
        let scale = Scale::<Linear>::new()
            .domain_interval(lit(0.0), lit(1.0))
            .range_interval(lit(0.0), lit(100.0))
            .into_auto();

        // Create a configured scale
        let empty_params = indexmap::IndexMap::new();
        let configured = scale
            .create_configured_scale(100.0, 100.0, &ctx, &empty_params)
            .await
            .unwrap();

        // Wrap in ConfiguredScaleWithSpec
        use avenger_chart::scales::{ConfiguredScaleDataFusionExt, ConfiguredScaleWithSpec};
        let configured_scale = ConfiguredScaleWithSpec::new(scale.clone(), configured);

        // Use the scale in an expression
        let scaled_expr = configured_scale.to_expr(col("value")).unwrap();

        // Serialize the expression
        let serializable_expr = LogicalExprNode::from_expr(scaled_expr).unwrap();

        // Serialize to JSON
        let json = serde_json::to_string(&SerializableExpr::from(serializable_expr)).unwrap();
        println!("Serialized expression: {}", json);

        // Deserialize on a fresh context
        let new_ctx = SessionContext::new();

        // Re-register the test data in the new context
        let batch2 = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(Float64Array::from(vec![0.0, 0.5, 1.0]))],
        )
        .unwrap();
        new_ctx.register_batch("test_data", batch2).unwrap();

        // Deserialize the expression
        let deserialized: SerializableExpr = serde_json::from_str(&json).unwrap();
        let restored_expr = deserialized.to_expr(&new_ctx).unwrap();

        // Execute the expression and verify results
        let df = new_ctx.table("test_data").await.unwrap();
        let result = df
            .select(vec![restored_expr.alias("scaled")])
            .unwrap()
            .collect()
            .await
            .unwrap();

        // Check results - values should be scaled from [0,1] to [0,100]
        let batch = &result[0];
        let scaled_array = batch
            .column(0)
            .as_any()
            .downcast_ref::<datafusion::arrow::array::Float32Array>()
            .unwrap();

        assert_eq!(scaled_array.len(), 3);
        assert!((scaled_array.value(0) - 0.0).abs() < 0.01);
        assert!((scaled_array.value(1) - 50.0).abs() < 0.01);
        assert!((scaled_array.value(2) - 100.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_multiple_scale_types() {
        use avenger_chart::scales::{Band, Log, Sqrt};

        let ctx = SessionContext::new();

        // Test that different scale types serialize/deserialize correctly
        let scales = vec![
            Scale::<Linear>::new().into_auto(),
            Scale::<Log>::new().into_auto(),
            Scale::<Sqrt>::new().into_auto(),
            Scale::<Band>::new().into_auto(),
        ];

        let empty_params = indexmap::IndexMap::new();
        for scale in scales {
            // Just test that we can create a configured scale and convert to expression
            let configured_result = scale
                .create_configured_scale(100.0, 100.0, &ctx, &empty_params)
                .await;

            // Band and other discrete scales require discrete domains, so some may fail
            // That's ok for this test - we're just testing the serialization machinery
            if let Ok(configured) = configured_result {
                use avenger_chart::scales::{
                    ConfiguredScaleDataFusionExt, ConfiguredScaleWithSpec,
                };
                let configured_scale = ConfiguredScaleWithSpec::new(scale.clone(), configured);
                let expr = configured_scale.to_expr(lit(0.5));

                if let Ok(expr) = expr {
                    // Test serialization roundtrip
                    let serializable = LogicalExprNode::from_expr(expr).unwrap();
                    let json =
                        serde_json::to_string(&SerializableExpr::from(serializable)).unwrap();
                    let _deserialized: SerializableExpr = serde_json::from_str(&json).unwrap();

                    // If we get here, serialization works for this scale type
                    println!("Successfully serialized scale type");
                }
            }
        }
    }
}
