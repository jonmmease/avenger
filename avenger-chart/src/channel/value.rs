//! Compatibility re-export for core channel value types.

pub use avenger_chart_core::channel::strip_trailing_numbers;
pub use avenger_chart_core::channel_value::expr_to_string;
pub use avenger_chart_core::{BaseChannelName, ChannelValue, ConditionalValue};

#[cfg(test)]
mod tests {
    use super::ChannelValue;
    use crate::scales::ScaleChannelValue;
    use datafusion::prelude::{col, lit};

    #[test]
    fn test_channel_value_scale_config_serialization_roundtrip() {
        let cv: ChannelValue = col("x").into();
        let cv = cv.scale(|s| {
            s.domain_interval(lit(0.0), lit(10.0))
                .range_interval(lit(0.0), lit(100.0))
        });

        let json = serde_json::to_string(&cv).expect("serialize channel value");
        let decoded: ChannelValue = serde_json::from_str(&json).expect("deserialize channel value");
        let scale_config = decoded.get_scale_config().expect("scale config");

        assert!(scale_config.domain.is_set());
        assert!(scale_config.range.is_set());
    }
}
