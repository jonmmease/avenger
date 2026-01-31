//! Channel descriptor types for defining mark channels

use datafusion::common::ScalarValue;

/// Default value for a channel
#[derive(Debug, Clone)]
pub enum ChannelDefault {
    Scalar(ScalarValue),
    // Could extend with other types later, like theme based defaults,
    // expressions based on other channels, etc.
}

/// Descriptor for a channel that a mark supports
#[derive(Debug, Clone)]
pub struct ChannelDescriptor {
    pub name: &'static str,
    pub required: bool,
    pub default_value: Option<ChannelDefault>,
    pub allow_column_ref: bool,
}
