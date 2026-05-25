#![allow(
    clippy::borrowed_box,
    clippy::items_after_test_module,
    clippy::large_enum_variant,
    clippy::module_inception,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

pub mod axis;
pub mod cartesian;
pub mod channel;
pub mod color;
pub mod concat;
#[doc(hidden)]
pub mod container;
pub mod coords;
pub mod doc;
pub mod error;
pub mod facet;
pub mod guide;
pub mod layout;
pub mod legend;
pub mod marks;
pub mod param;
pub(crate) mod partition;
pub mod plot;
pub mod polar;
pub(crate) mod positioned_subplot;
pub mod prelude;
pub mod render;
// render_context moved to render/context
pub mod maybe;
pub mod scales;
pub mod serialization;
pub mod theme;
pub mod utils;
pub mod zerod;

pub use avenger_chart_core::{
    define_common_mark_channels, define_position_channels, impl_mark_base, impl_mark_trait_common,
    impl_supported_channels,
};

#[cfg(test)]
mod serialization_tests {
    use datafusion::prelude::col;
    use datafusion_proto::protobuf::LogicalExprNode;

    use crate::{channel::value::ChannelValue, serialization::LogicalExprNodeExt};

    #[test]
    fn test_channel_value_serialization() {
        // Test that we can create a ChannelValue with LogicalExprNode
        let expr = col("test");
        let expr_node = LogicalExprNode::from_expr(expr).expect("Failed to serialize expr");

        let channel = ChannelValue::Scaled {
            expr: expr_node.clone(),
            scale_name: Some("x".to_string()),
            band: None,
            scale_config: None,
            legend_config: None,
            share_mode: None,
        };

        // Serialize to JSON
        let json = serde_json::to_string_pretty(&channel).expect("Failed to serialize to JSON");
        println!("Serialized ChannelValue:\n{}", json);

        // Deserialize back
        let _deserialized: ChannelValue =
            serde_json::from_str(&json).expect("Failed to deserialize from JSON");
        println!("✓ Successfully deserialized ChannelValue");
    }
}
