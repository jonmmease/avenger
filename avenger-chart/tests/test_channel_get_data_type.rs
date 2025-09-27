use avenger_chart::channel::ChannelValue;
use avenger_chart::serialization::SerializableExpr;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::common::DFSchema;
use datafusion::prelude::*;
use std::sync::Arc;

#[test]
fn test_channel_value_get_data_type() {
    // Create a simple channel value
    let expr = col("x");
    let serializable = SerializableExpr::from_expr(expr).unwrap();
    let channel_value = ChannelValue::Scaled {
        expr: serializable,
        scale_name: None,
        band: None,
        scale_config: None,
        legend_config: None,
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