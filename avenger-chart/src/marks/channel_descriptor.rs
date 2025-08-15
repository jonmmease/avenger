//! Channel descriptor types for defining mark channels

use datafusion::scalar::ScalarValue;

/// Types of channels that marks can support
/// Each type corresponds to a coercion capability in the Coercer
#[derive(Debug, Clone)]
pub enum ChannelType {
    /// Numeric values (uses to_numeric) - for positions, sizes, angles, opacity, etc.
    Numeric,

    /// Unsigned integer values (uses to_usize) - for indices, counts
    Usize,

    /// Color values (uses to_color) - for fill, stroke
    Color,

    /// Boolean values (uses to_boolean) - for defined, visible
    Boolean,

    /// String values (uses to_string) - for text labels
    String,

    /// Stroke dash patterns (uses to_stroke_dash) - for line dash arrays
    StrokeDash,

    /// Stroke cap style (uses to_stroke_cap) - for line endings
    StrokeCap,

    /// Stroke join style (uses to_stroke_join) - for line corners
    StrokeJoin,

    /// Symbol shapes (uses to_symbol_shape) - for mark shapes
    SymbolShape,

    /// Multiple symbol shapes (uses to_symbol_shapes) - for shape arrays
    SymbolShapes,

    /// Path data (uses to_path) - for custom path marks
    Path,

    /// Path transformations (uses to_path_transform) - for path scaling/rotation
    PathTransform,

    /// Image data (uses to_image) - for image marks
    Image,

    /// Image alignment (uses enum coercer) - for horizontal image alignment
    ImageAlign,

    /// Image baseline (uses enum coercer) - for vertical image alignment
    ImageBaseline,

    /// Area orientation (uses enum coercer) - for area mark stacking
    AreaOrientation,

    /// Text alignment (uses enum coercer) - for horizontal text alignment
    TextAlign,

    /// Text baseline (uses enum coercer) - for vertical text alignment
    TextBaseline,

    /// Font weight (uses enum coercer) - for text boldness
    FontWeight,

    /// Font style (uses enum coercer) - for italic/normal text
    FontStyle,
}

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
    pub channel_type: ChannelType,
    pub default_value: Option<ChannelDefault>,
    pub allow_column_ref: bool,
}
