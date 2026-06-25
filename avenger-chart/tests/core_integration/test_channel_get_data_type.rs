use avenger_chart::channel::ChannelValue;
use avenger_chart::serialization::LogicalExprNodeExt;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::common::DFSchema;
use datafusion::prelude::*;
use datafusion_proto::protobuf::LogicalExprNode;

#[test]
fn test_channel_value_get_data_type() {
    // Create a simple channel value
    let expr = col("x");
    let serializable = LogicalExprNode::from_expr(expr).unwrap();
    let channel_value = ChannelValue::Scaled {
        expr: serializable,
        scale_name: None,
        position_boundary: None,
        scale_config: None,
        nested_band_config: None,
        legend_config: None,
        axis_config: None,
        domain_coordination: None,
        transform_scope: None,
    };

    // Create a schema with the column
    let arrow_schema = Schema::new(vec![Field::new("x", DataType::Float32, false)]);
    let df_schema = DFSchema::try_from(arrow_schema).unwrap();

    // Get the data type
    let ctx = SessionContext::new();
    let data_type = channel_value.get_data_type(&df_schema, &ctx);

    assert!(data_type.is_ok(), "Should be able to get data type");
    assert_eq!(data_type.unwrap(), DataType::Float32);
}

#[test]
fn test_nested_struct_channel_value_get_data_type_preserves_field_order() {
    let expr = named_struct(vec![lit("outer"), col("group"), lit("leaf"), col("series")]);
    let channel_value = ChannelValue::from(expr);

    let arrow_schema = Schema::new(vec![
        Field::new("group", DataType::Utf8, false),
        Field::new("series", DataType::Int32, false),
    ]);
    let df_schema = DFSchema::try_from(arrow_schema).unwrap();

    let ctx = SessionContext::new();
    let data_type = channel_value
        .get_data_type(&df_schema, &ctx)
        .expect("nested struct data type");

    let DataType::Struct(fields) = data_type else {
        panic!("expected struct data type");
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name(), "outer");
    assert_eq!(fields[0].data_type(), &DataType::Utf8);
    assert_eq!(fields[1].name(), "leaf");
    assert_eq!(fields[1].data_type(), &DataType::Int32);
}
