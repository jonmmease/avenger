//! Mark implementations for ZeroDCoord
//!
//! These implementations provide access to coordinate-agnostic default values
//! for marks in a zero-dimensional coordinate system. Since there are no position
//! channels in 0D space, only visual channels (color, size, shape, etc.) are relevant.

use crate::error::AvengerChartError;
use crate::marks::Mark;
use crate::zerod::ZeroDCoord;
use crate::impl_mark_trait_common;
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion_common::ScalarValue;

// Re-export base mark types for ZeroDCoord
pub use crate::marks::line::Line;
pub use crate::marks::rect::Rect;
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

impl Line<ZeroDCoord> {
    pub fn position_channel_descriptors() -> Vec<crate::marks::ChannelDescriptor> {
        vec![] // No position channels in 0D space
    }

    pub fn all_channel_descriptors() -> Vec<crate::marks::ChannelDescriptor> {
        let mut descriptors = Self::common_channel_descriptors();
        descriptors.extend(Self::position_channel_descriptors());
        descriptors
    }
}

impl Rect<ZeroDCoord> {
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

    fn default_channel_value(&self, channel: &str) -> Option<ScalarValue> {
        // Visual channel defaults (coordinate-agnostic)
        match channel {
            "size" => Some(ScalarValue::Float32(Some(64.0))),
            "shape" => Some(ScalarValue::Utf8(Some("circle".to_string()))),
            "angle" => Some(ScalarValue::Float32(Some(0.0))),
            "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))),
            "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
            "stroke_width" => Some(ScalarValue::Float32(Some(1.0))),
            "opacity" => Some(ScalarValue::Float32(Some(1.0))),
            _ => None,
        }
    }

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        unreachable!("ZeroDCoord marks should not be rendered through standard pipeline")
    }
}

// Implement Mark trait for ZeroDCoord Line
impl Mark<ZeroDCoord> for Line<ZeroDCoord> {
    impl_mark_trait_common!(Line, ZeroDCoord, "line");

    fn default_channel_value(&self, channel: &str) -> Option<ScalarValue> {
        // Visual channel defaults (coordinate-agnostic)
        match channel {
            "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
            "stroke_width" => Some(ScalarValue::Float32(Some(2.0))),
            "stroke_dash" => Some(ScalarValue::Utf8(Some("solid".to_string()))),
            "stroke_cap" => Some(ScalarValue::Utf8(Some("round".to_string()))),
            "stroke_join" => Some(ScalarValue::Utf8(Some("round".to_string()))),
            "opacity" => Some(ScalarValue::Float32(Some(1.0))),
            "defined" => Some(ScalarValue::Boolean(Some(true))),
            _ => None,
        }
    }

    fn supports_order(&self) -> bool {
        true
    }

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        unreachable!("ZeroDCoord marks should not be rendered through standard pipeline")
    }
}

// Implement Mark trait for ZeroDCoord Rect
impl Mark<ZeroDCoord> for Rect<ZeroDCoord> {
    impl_mark_trait_common!(Rect, ZeroDCoord, "rect");

    fn default_channel_value(&self, channel: &str) -> Option<ScalarValue> {
        // Visual channel defaults (coordinate-agnostic)
        match channel {
            "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))),
            "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
            "stroke_width" => Some(ScalarValue::Float32(Some(1.0))),
            "corner_radius" => Some(ScalarValue::Float32(Some(0.0))),
            _ => None,
        }
    }

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        unreachable!("ZeroDCoord marks should not be rendered through standard pipeline")
    }
}
