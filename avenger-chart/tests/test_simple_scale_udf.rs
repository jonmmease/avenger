//! Simple test for scale UDF serialization

#[cfg(test)]
mod tests {
    use avenger_chart::scales::{Linear, Scale, ScaleRuntimeExt};
    use avenger_chart::serialization::LogicalExprNodeExt;
    use avenger_chart::serialization::SerializableExpr;
    use datafusion::logical_expr::lit;
    use datafusion::prelude::*;
    use datafusion_proto::protobuf::LogicalExprNode;

    #[test]
    fn test_scale_serialization_basic() {
        // Create a simple linear scale
        let scale = Scale::<Linear>::new()
            .domain_interval(lit(0.0), lit(1.0))
            .range_interval(lit(0.0), lit(100.0))
            .into_auto();

        // Verify we can convert to ScaleImpl
        let scale_impl = scale.to_scale_impl();
        assert!(scale_impl.is_ok());
    }

    #[tokio::test]
    async fn test_scale_expr_serialization() {
        let ctx = SessionContext::new();

        // Create a linear scale
        let scale = Scale::<Linear>::new()
            .domain_interval(lit(0.0), lit(1.0))
            .range_interval(lit(0.0), lit(100.0))
            .into_auto();

        // Try to create configured scale
        println!("Creating configured scale...");
        let empty_params = indexmap::IndexMap::new();
        let configured = scale
            .create_configured_scale(100.0, 100.0, &ctx, &empty_params)
            .await
            .unwrap();

        // Wrap in ConfiguredScaleWithSpec
        use avenger_chart::scales::ConfiguredScaleWithSpec;
        let configured_scale = ConfiguredScaleWithSpec::new(scale.clone(), configured);

        println!("Creating expression...");
        // Create an expression using the scale
        use avenger_chart::scales::ConfiguredScaleDataFusionExt;
        let expr = configured_scale.to_expr(lit(0.5)).unwrap();

        println!("Serializing expression...");
        // Serialize the expression
        let serializable = LogicalExprNode::from_expr(expr).unwrap();
        let json = serde_json::to_string(&SerializableExpr::from(serializable.clone())).unwrap();
        println!("Serialized: {}", &json[..100.min(json.len())]);

        println!("Deserializing expression...");
        // Deserialize
        let new_ctx = SessionContext::new();
        let deserialized: SerializableExpr = serde_json::from_str(&json).unwrap();
        let expr_node: datafusion_proto::protobuf::LogicalExprNode = deserialized.into();
        let restored = expr_node.to_expr(&new_ctx).unwrap();

        println!("Successfully restored expression: {:?}", restored);
    }
}
