use crate::legend::Legend;
use crate::marks::channel::{ChannelValue, ConditionalBuilder};
use crate::scales::{Auto, Scale, ScaleSpec as ScaleTypeSpec};
use datafusion::logical_expr::Expr;

/// Typed wrapper for position channels (x, y, x2, y2, etc.)
/// These channels support scales but not legends
#[derive(Clone)]
pub struct PositionChannel(pub ChannelValue);

impl PositionChannel {
    /// Disable scaling for this channel
    pub fn no_scale(self) -> Self {
        Self(self.0.no_scale())
    }

    /// Configure the scale for this channel
    pub fn scale<F>(self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        PositionChannel(self.0.scale(f))
    }

    /// Configure the scale with explicit type
    pub fn scale_with<S: ScaleTypeSpec, F>(self, f: F) -> Self
    where
        F: Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    {
        PositionChannel(self.0.scale_with(f))
    }

    /// Set the band parameter
    pub fn band(self, band: f64) -> Self {
        PositionChannel(self.0.band(band))
    }

    /// Set a custom scale name
    pub fn with_scale_name(self, name: impl Into<String>) -> Self {
        PositionChannel(self.0.with_scale_name(name))
    }

    /// Get the inner ChannelValue
    pub fn into_inner(self) -> ChannelValue {
        self.0
    }
}

/// Typed wrapper for color channels (fill, stroke)
/// These channels support both scales and legends
#[derive(Clone)]
pub struct ColorChannel(pub ChannelValue);

impl ColorChannel {
    /// Configure the scale for this channel
    pub fn scale<F>(self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        ColorChannel(self.0.scale(f))
    }

    /// Configure the scale with explicit type
    pub fn scale_with<S: ScaleTypeSpec>(
        self,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        ColorChannel(self.0.scale_with(f))
    }

    /// Configure the legend for this channel
    pub fn legend<F>(self, f: F) -> Self
    where
        F: Fn(Legend) -> Legend + Send + Sync + 'static,
    {
        ColorChannel(self.0.legend(f))
    }

    /// Disable legend for this channel
    pub fn no_legend(self) -> Self {
        ColorChannel(self.0.no_legend())
    }

    /// Disable scaling for this channel
    pub fn no_scale(self) -> Self {
        Self(self.0.no_scale())
    }

    /// Set a custom scale name
    pub fn with_scale_name(self, name: impl Into<String>) -> Self {
        ColorChannel(self.0.with_scale_name(name))
    }

    /// Get the inner ChannelValue
    pub fn into_inner(self) -> ChannelValue {
        self.0
    }
}

/// Typed wrapper for size channels (size, stroke_width)
/// These channels support both scales and legends
#[derive(Clone)]
pub struct SizeChannel(pub ChannelValue);

impl SizeChannel {
    /// Disable scaling for this channel
    pub fn no_scale(self) -> Self {
        Self(self.0.no_scale())
    }

    /// Configure the scale for this channel
    pub fn scale<F>(self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        SizeChannel(self.0.scale(f))
    }

    /// Configure the scale with explicit type
    pub fn scale_with<S: ScaleTypeSpec, F>(self, f: F) -> Self
    where
        F: Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    {
        SizeChannel(self.0.scale_with(f))
    }

    /// Configure the legend for this channel
    pub fn legend<F>(self, f: F) -> Self
    where
        F: Fn(Legend) -> Legend + Send + Sync + 'static,
    {
        SizeChannel(self.0.legend(f))
    }

    /// Disable legend for this channel
    pub fn no_legend(self) -> Self {
        SizeChannel(self.0.no_legend())
    }

    /// Set a custom scale name
    pub fn with_scale_name(self, name: impl Into<String>) -> Self {
        SizeChannel(self.0.with_scale_name(name))
    }

    /// Get the inner ChannelValue
    pub fn into_inner(self) -> ChannelValue {
        self.0
    }
}

/// Typed wrapper for shape channels
/// These channels support both scales and legends
#[derive(Clone)]
pub struct ShapeChannel(pub ChannelValue);

impl ShapeChannel {
    /// Disable scaling for this channel
    pub fn no_scale(self) -> Self {
        Self(self.0.no_scale())
    }

    /// Configure the scale for this channel
    pub fn scale<F>(self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        ShapeChannel(self.0.scale(f))
    }

    /// Configure the scale with explicit type
    pub fn scale_with<S: ScaleTypeSpec, F>(self, f: F) -> Self
    where
        F: Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    {
        ShapeChannel(self.0.scale_with(f))
    }

    /// Configure the legend for this channel
    pub fn legend<F>(self, f: F) -> Self
    where
        F: Fn(Legend) -> Legend + Send + Sync + 'static,
    {
        ShapeChannel(self.0.legend(f))
    }

    /// Disable legend for this channel
    pub fn no_legend(self) -> Self {
        ShapeChannel(self.0.no_legend())
    }

    /// Set a custom scale name
    pub fn with_scale_name(self, name: impl Into<String>) -> Self {
        ShapeChannel(self.0.with_scale_name(name))
    }

    /// Get the inner ChannelValue
    pub fn into_inner(self) -> ChannelValue {
        self.0
    }
}

/// Typed wrapper for angle channels
/// These channels support both scales and legends
#[derive(Clone)]
pub struct AngleChannel(pub ChannelValue);

impl AngleChannel {
    /// Disable scaling for this channel
    pub fn no_scale(self) -> Self {
        Self(self.0.no_scale())
    }

    /// Configure the scale for this channel
    pub fn scale<F>(self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        AngleChannel(self.0.scale(f))
    }

    /// Configure the scale with explicit type
    pub fn scale_with<S: ScaleTypeSpec, F>(self, f: F) -> Self
    where
        F: Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    {
        AngleChannel(self.0.scale_with(f))
    }

    /// Configure the legend for this channel
    pub fn legend<F>(self, f: F) -> Self
    where
        F: Fn(Legend) -> Legend + Send + Sync + 'static,
    {
        AngleChannel(self.0.legend(f))
    }

    /// Disable legend for this channel
    pub fn no_legend(self) -> Self {
        AngleChannel(self.0.no_legend())
    }

    /// Set a custom scale name
    pub fn with_scale_name(self, name: impl Into<String>) -> Self {
        AngleChannel(self.0.with_scale_name(name))
    }

    /// Get the inner ChannelValue
    pub fn into_inner(self) -> ChannelValue {
        self.0
    }
}

/// Typed wrapper for stroke dash channels
/// These channels support both scales and legends
#[derive(Clone)]
pub struct StrokeDashChannel(pub ChannelValue);

impl StrokeDashChannel {
    /// Disable scaling for this channel
    pub fn no_scale(self) -> Self {
        Self(self.0.no_scale())
    }

    /// Configure the scale for this channel
    pub fn scale<F>(self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        StrokeDashChannel(self.0.scale(f))
    }

    /// Configure the scale with explicit type
    pub fn scale_with<S: ScaleTypeSpec, F>(self, f: F) -> Self
    where
        F: Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    {
        StrokeDashChannel(self.0.scale_with(f))
    }

    /// Configure the legend for this channel
    pub fn legend<F>(self, f: F) -> Self
    where
        F: Fn(Legend) -> Legend + Send + Sync + 'static,
    {
        StrokeDashChannel(self.0.legend(f))
    }

    /// Disable legend for this channel
    pub fn no_legend(self) -> Self {
        StrokeDashChannel(self.0.no_legend())
    }

    /// Set a custom scale name
    pub fn with_scale_name(self, name: impl Into<String>) -> Self {
        StrokeDashChannel(self.0.with_scale_name(name))
    }

    /// Get the inner ChannelValue
    pub fn into_inner(self) -> ChannelValue {
        self.0
    }
}

// Conversion implementations for easy usage
impl From<Expr> for PositionChannel {
    fn from(expr: Expr) -> Self {
        PositionChannel(expr.into())
    }
}

impl From<PositionChannel> for ChannelValue {
    fn from(channel: PositionChannel) -> Self {
        channel.0
    }
}

impl From<Expr> for ColorChannel {
    fn from(expr: Expr) -> Self {
        ColorChannel(expr.into())
    }
}

impl From<ColorChannel> for ChannelValue {
    fn from(channel: ColorChannel) -> Self {
        channel.0
    }
}

impl From<Expr> for SizeChannel {
    fn from(expr: Expr) -> Self {
        SizeChannel(expr.into())
    }
}

impl From<SizeChannel> for ChannelValue {
    fn from(channel: SizeChannel) -> Self {
        channel.0
    }
}

impl From<Expr> for ShapeChannel {
    fn from(expr: Expr) -> Self {
        ShapeChannel(expr.into())
    }
}

impl From<ShapeChannel> for ChannelValue {
    fn from(channel: ShapeChannel) -> Self {
        channel.0
    }
}

impl From<Expr> for AngleChannel {
    fn from(expr: Expr) -> Self {
        AngleChannel(expr.into())
    }
}

impl From<AngleChannel> for ChannelValue {
    fn from(channel: AngleChannel) -> Self {
        channel.0
    }
}

impl From<Expr> for StrokeDashChannel {
    fn from(expr: Expr) -> Self {
        StrokeDashChannel(expr.into())
    }
}

impl From<StrokeDashChannel> for ChannelValue {
    fn from(channel: StrokeDashChannel) -> Self {
        channel.0
    }
}

// From implementations for literals
impl From<&str> for ColorChannel {
    fn from(s: &str) -> Self {
        ColorChannel(s.into())
    }
}

impl From<f64> for PositionChannel {
    fn from(v: f64) -> Self {
        PositionChannel(v.into())
    }
}

impl From<f64> for SizeChannel {
    fn from(v: f64) -> Self {
        SizeChannel(v.into())
    }
}

impl From<i32> for PositionChannel {
    fn from(v: i32) -> Self {
        PositionChannel(v.into())
    }
}

impl From<i32> for SizeChannel {
    fn from(v: i32) -> Self {
        SizeChannel(v.into())
    }
}

// From ConditionalBuilder for all channel types
impl From<ConditionalBuilder> for PositionChannel {
    fn from(builder: ConditionalBuilder) -> Self {
        PositionChannel(builder.into())
    }
}

impl From<ConditionalBuilder> for ColorChannel {
    fn from(builder: ConditionalBuilder) -> Self {
        ColorChannel(builder.into())
    }
}

impl From<ConditionalBuilder> for SizeChannel {
    fn from(builder: ConditionalBuilder) -> Self {
        SizeChannel(builder.into())
    }
}

impl From<ConditionalBuilder> for ShapeChannel {
    fn from(builder: ConditionalBuilder) -> Self {
        ShapeChannel(builder.into())
    }
}

impl From<ConditionalBuilder> for AngleChannel {
    fn from(builder: ConditionalBuilder) -> Self {
        AngleChannel(builder.into())
    }
}

impl From<ConditionalBuilder> for StrokeDashChannel {
    fn from(builder: ConditionalBuilder) -> Self {
        StrokeDashChannel(builder.into())
    }
}
