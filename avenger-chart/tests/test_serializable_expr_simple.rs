use avenger_chart::serialization::LogicalExprNodeExt;
use datafusion::prelude::*;
use datafusion_proto::protobuf::LogicalExprNode;

#[test]
fn test_serializable_expr_simple_col() {
    // Create a simple column expression
    let expr = col("x");

    // Serialize it
    let serializable = LogicalExprNode::from_expr(expr.clone()).unwrap();

    // Deserialize it
    let ctx = SessionContext::new();
    let deserialized = serializable.to_expr(&ctx).unwrap();

    // Check they're equivalent
    assert_eq!(format!("{:?}", expr), format!("{:?}", deserialized));
}
