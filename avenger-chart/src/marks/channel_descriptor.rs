//! Channel descriptor types for defining mark channels

use datafusion::scalar::ScalarValue;

/// Types of channels that marks can support
#[derive(Debug, Clone)]
pub enum ChannelType {
    Position, // x, y, x2, y2
    Color,    // fill, stroke
    Size,     // stroke_width, size
    Text,     // text labels
    Numeric,  // opacity, angle
    Boolean,  // defined, visible
    Enum {
        // discrete choices with allowed values
        values: &'static [&'static str],
    },
}

/// Default value for a channel
#[derive(Debug, Clone)]
pub enum ChannelDefault {
    Scalar(ScalarValue),
    // Could extend with other types later
}

/// Descriptor for a channel that a mark supports
#[derive(Debug, Clone)]
pub struct ChannelDescriptor {
    pub name: &'static str,
    pub required: bool,
    pub channel_type: ChannelType,
    pub default_value: Option<ChannelDefault>,
    pub allow_column_ref: bool, // Can this channel vary per mark instance?
}