//! Mark implementations for ZeroDCoord
//!
//! Only Symbol marks are supported in zero-dimensional coordinate systems.
//! Line and Rect marks don't make sense without spatial extent.

use crate::error::AvengerChartError;
use crate::impl_mark_trait_common;
use crate::marks::Mark;
use crate::render_context::RenderContext;
use crate::zerod::ZeroDCoord;
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion_common::ScalarValue;

// Only Symbol marks are supported in ZeroDCoord
pub use crate::marks::symbol::Symbol;

// Implement position channel methods for ZeroDCoord marks
// Since ZeroDCoord has no position channels (0D space), these implementations are minimal

impl Symbol<ZeroDCoord> {
    pub fn position_channel_descriptors() -> Vec<crate::marks::ChannelDescriptor> {
        vec![] // No position channels in 0D space
    }

    pub fn all_channel_descriptors() -> Vec<crate::marks::ChannelDescriptor> {
        let mut descriptors = Self::common_channel_descriptors();
        descriptors.extend(Self::position_channel_descriptors());
        descriptors
    }
}

// Implement Mark trait for ZeroDCoord Symbol
impl Mark<ZeroDCoord> for Symbol<ZeroDCoord> {
    impl_mark_trait_common!(Symbol, ZeroDCoord, "symbol");

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        Symbol::<ZeroDCoord>::common_mark_specific_default(channel)
    }

    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &RenderContext,
        coord: &ZeroDCoord,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // In 0D space, symbols render at the center point
        // Use the common rendering logic which handles all the channel processing
        self.render_from_data_common(data, scalars, context, coord)
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<std::sync::Arc<dyn crate::legend::LegendRenderer>> {
        // ZeroD has no position channels
        Symbol::<ZeroDCoord>::common_preferred_legend_renderer(channel, scale, &[])
    }
}
