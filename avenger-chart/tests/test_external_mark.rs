//! Integration test to verify external marks can be defined and used

use avenger_chart::{
    cartesian::Cartesian,
    coords::CoordinateSystem,
    define_common_mark_channels, define_position_mark_channels, impl_mark_base,
    impl_mark_trait_common,
    marks::{ChannelType, Mark, MarkState},
    plot::Plot,
};
use avenger_chart::error::AvengerChartError;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::scalar::ScalarValue;
use std::marker::PhantomData;

/// A custom hexbin mark defined in an external crate
pub struct HexBin<C: CoordinateSystem> {
    state: MarkState<C>,
    _phantom: PhantomData<C>,
}

// Use the exported macro for base implementation
impl_mark_base!(HexBin);

// Define common channels
define_common_mark_channels! {
    HexBin {
        fill: { type: ChannelType::Color },
        stroke: { type: ChannelType::Color },
        opacity: { type: ChannelType::Numeric },
        size: { type: ChannelType::Numeric },
    }
}

// Define position channels for Cartesian
define_position_mark_channels! {
    HexBin<Cartesian> {
        x: { type: ChannelType::Numeric, required: true },
        y: { type: ChannelType::Numeric, required: true },
    }
}

// Implement the Mark trait for Cartesian
impl Mark<Cartesian> for HexBin<Cartesian> {
    impl_mark_trait_common!(HexBin, Cartesian, "hexbin");

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Custom hexbin rendering logic would go here
        // For this test, we just return an empty vector
        Ok(vec![])
    }

    fn default_channel_value(&self, channel: &str) -> Option<ScalarValue> {
        match channel {
            "size" => Some(ScalarValue::Float32(Some(20.0))),
            "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))),
            _ => None,
        }
    }
}

#[test]
fn test_external_mark_can_be_created() {
    // Create a custom mark
    let hexbin = HexBin::<Cartesian>::new()
        .x("temperature")
        .y("humidity")
        .fill("category")
        .size(15.0);

    // Verify state access works
    assert_eq!(hexbin.mark_type(), "hexbin");
    assert_eq!(hexbin.state().zindex, None);
    
    // Verify channels are properly defined
    let channels = hexbin.supported_channels();
    let channel_names: Vec<_> = channels.iter().map(|c| c.name).collect();
    assert!(channel_names.contains(&"x"));
    assert!(channel_names.contains(&"y"));
    assert!(channel_names.contains(&"fill"));
    assert!(channel_names.contains(&"size"));
}

#[test]
fn test_external_mark_can_be_added_to_plot() {
    // Create a plot
    let plot = Plot::new(Cartesian);
    
    // Create a custom mark
    let hexbin = HexBin::<Cartesian>::new()
        .x("temperature")
        .y("humidity")
        .fill("category");

    // Add the mark to the plot - this tests that the types work correctly
    let plot_with_mark = plot.mark(hexbin);
    
    // The plot should accept our custom mark without issue
    assert!(plot_with_mark.marks().len() > 0);
}

#[test]
fn test_external_mark_state_mutation() {
    // Create a custom mark
    let mut hexbin = HexBin::<Cartesian>::new();
    
    // Verify we can mutate the state
    hexbin.state_mut().zindex = Some(10);
    assert_eq!(hexbin.state().zindex, Some(10));
    
    // Verify builder methods work
    let hexbin2 = HexBin::<Cartesian>::new().zindex(5);
    assert_eq!(hexbin2.state().zindex, Some(5));
}