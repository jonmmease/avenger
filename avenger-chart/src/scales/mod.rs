use avenger_scales::scales::ConfiguredScale;

// Public submodules
pub mod builder;
pub mod codec;
pub mod domain_extent;
pub mod extensions;
pub mod udf;

// Internal modules
mod defaults;
mod domain;
mod domain_inference;
mod range;
mod scale;
pub mod spec;

// Re-export the main types
pub use builder::{ChannelScaleData, ScaleBuilder};
pub use codec::AvengerChartExtensionCodec;
pub use defaults::default_range_for_channel;
pub use domain::{DomainExpr, ResolvedDomain, ScaleDefaultDomain, ScaleDomain};
pub use domain_extent::{
    DomainBounds, DomainExtent, RadiusPadding, SerializableDataExtents, SerializableDomainValue,
};
pub use extensions::{ConfiguredScaleDataFusionExt, ConfiguredScaleLegendExt, DomainValues};
pub use range::ScaleRange;
pub use scale::Scale;
pub use spec::{
    Auto, Band, Linear, Log, Ordinal, Point, Pow, Quantile, Quantize, ScaleSpec, Sqrt, Symlog,
    Threshold, Time,
};
pub use udf::create_scale_udf;

/// Plot-area dimension used by a coordinate-owned scale range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlotAreaDimension {
    Width,
    Height,
    MinWidthHeight,
}

impl PlotAreaDimension {
    fn resolve(self, plot_area_width: f64, plot_area_height: f64) -> f64 {
        match self {
            PlotAreaDimension::Width => plot_area_width,
            PlotAreaDimension::Height => plot_area_height,
            PlotAreaDimension::MinWidthHeight => plot_area_width.min(plot_area_height),
        }
    }
}

/// One endpoint of a coordinate-owned scale range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlotAreaRangeEndpoint {
    Constant(f64),
    Dimension {
        dimension: PlotAreaDimension,
        factor: f64,
    },
}

impl PlotAreaRangeEndpoint {
    pub const ZERO: Self = Self::Constant(0.0);
    pub const WIDTH: Self = Self::Dimension {
        dimension: PlotAreaDimension::Width,
        factor: 1.0,
    };
    pub const HEIGHT: Self = Self::Dimension {
        dimension: PlotAreaDimension::Height,
        factor: 1.0,
    };
    pub const HALF_MIN_DIMENSION: Self = Self::Dimension {
        dimension: PlotAreaDimension::MinWidthHeight,
        factor: 0.5,
    };

    fn resolve(self, plot_area_width: f64, plot_area_height: f64) -> f64 {
        match self {
            PlotAreaRangeEndpoint::Constant(value) => value,
            PlotAreaRangeEndpoint::Dimension { dimension, factor } => {
                dimension.resolve(plot_area_width, plot_area_height) * factor
            }
        }
    }
}

/// Coordinate-owned expression for a scale range as a function of plot dimensions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlotAreaRangeExpr {
    pub start: PlotAreaRangeEndpoint,
    pub end: PlotAreaRangeEndpoint,
}

impl PlotAreaRangeExpr {
    pub const fn new(start: PlotAreaRangeEndpoint, end: PlotAreaRangeEndpoint) -> Self {
        Self { start, end }
    }

    pub fn resolve(self, plot_area_width: f64, plot_area_height: f64) -> (f64, f64) {
        (
            self.start.resolve(plot_area_width, plot_area_height),
            self.end.resolve(plot_area_width, plot_area_height),
        )
    }
}

/// Describes how a configured scale range should respond to plot-area resizing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScaleRangeBinding {
    /// The coordinate system owns this range, and it should be recomputed after
    /// plot-area dimensions change.
    PlotArea(PlotAreaRangeExpr),
    /// The coordinate system supplied this fixed range, but it is not dimension-dependent.
    FixedInterval(f64, f64),
    /// The range comes from a mark, theme, user configuration, or another non-layout source.
    Independent,
}

impl ScaleRangeBinding {
    pub const fn plot_area(start: PlotAreaRangeEndpoint, end: PlotAreaRangeEndpoint) -> Self {
        Self::PlotArea(PlotAreaRangeExpr::new(start, end))
    }

    pub const fn fixed_interval(start: f64, end: f64) -> Self {
        Self::FixedInterval(start, end)
    }

    pub fn resolve(self, plot_area_width: f64, plot_area_height: f64) -> Option<(f64, f64)> {
        match self {
            ScaleRangeBinding::PlotArea(expr) => {
                Some(expr.resolve(plot_area_width, plot_area_height))
            }
            ScaleRangeBinding::FixedInterval(start, end) => Some((start, end)),
            ScaleRangeBinding::Independent => None,
        }
    }

    pub fn resolve_for_retarget(
        self,
        plot_area_width: f64,
        plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        match self {
            ScaleRangeBinding::PlotArea(expr) => {
                Some(expr.resolve(plot_area_width, plot_area_height))
            }
            ScaleRangeBinding::FixedInterval(_, _) | ScaleRangeBinding::Independent => None,
        }
    }
}

/// Wrapper that holds both the original Scale<Auto> specification and its ConfiguredScale
///
/// This allows us to maintain full extensibility - when creating DataFusion expressions,
/// we need the original Scale<Auto> to recreate the ScaleUDF, but for most operations
/// we just need the ConfiguredScale.
#[derive(Debug, Clone)]
pub struct ConfiguredScaleWithSpec {
    /// The original scale specification
    scale: Scale<Auto>,
    /// The configured scale with resolved domain/range
    configured: ConfiguredScale,
    /// Semantic owner of the configured range.
    range_binding: ScaleRangeBinding,
}

impl ConfiguredScaleWithSpec {
    /// Create a new ConfiguredScaleWithSpec
    pub fn new(scale: Scale<Auto>, configured: ConfiguredScale) -> Self {
        Self::with_range_binding(scale, configured, ScaleRangeBinding::Independent)
    }

    /// Create a new ConfiguredScaleWithSpec with explicit range binding metadata.
    pub fn with_range_binding(
        scale: Scale<Auto>,
        configured: ConfiguredScale,
        range_binding: ScaleRangeBinding,
    ) -> Self {
        Self {
            scale,
            configured,
            range_binding,
        }
    }

    /// Access the scale specification
    pub fn spec(&self) -> &Scale<Auto> {
        &self.scale
    }

    /// Access the configured scale
    pub fn configured(&self) -> &ConfiguredScale {
        &self.configured
    }

    /// Access semantic range binding metadata.
    pub fn range_binding(&self) -> ScaleRangeBinding {
        self.range_binding
    }

    /// Replace the configured scale while preserving its specification and range binding.
    pub fn set_configured(&mut self, configured: ConfiguredScale) {
        self.configured = configured;
    }

    /// Retarget a plot-area-bound scale range after dimensions change.
    pub fn retarget_plot_area_range(
        &mut self,
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> Option<((f32, f32), (f32, f32))> {
        let Ok(old_range) = self.configured.numeric_interval_range() else {
            return None;
        };
        let (new_start, new_end) = self
            .range_binding
            .resolve_for_retarget(plot_area_width as f64, plot_area_height as f64)?;
        let new_range = (new_start as f32, new_end as f32);
        if (old_range.0 - new_range.0).abs() <= 0.01 && (old_range.1 - new_range.1).abs() <= 0.01 {
            return None;
        }

        self.configured = self.configured.clone().with_range_interval(new_range);
        Some((old_range, new_range))
    }
}
