use avenger_scales::scales::ConfiguredScale;

pub mod builder;
pub mod channel_config;
pub mod codec;
pub mod defaults;
pub mod domain;
pub mod domain_extent;
mod domain_inference;
pub mod extensions;
pub mod mark_scale_builder;
pub mod range;
pub mod scale;
pub mod serialization;
pub mod spec;
pub mod udf;

pub use avenger_chart_core::{
    Auto, ConfiguredScaleLegendExt, DerivedScalarMap, DomainValues, PlotAreaDimension,
    PlotAreaRangeEndpoint, PlotAreaRangeExpr, ResolvedDomain, Scale, ScaleConfigSpec,
    ScaleOrderingSpec, ScaleRangeBinding, ScaleSpec,
};
pub use builder::{ChannelScaleData, DataExtents, DefaultScaleRangeResolver, ScaleBuilder};
pub use channel_config::{ScaleChannelConfig, ScaleChannelValue};
pub use codec::AvengerChartExtensionCodec;
pub use defaults::default_range_for_channel;
pub use domain::{DomainExpr, ScaleDefaultDomain, ScaleDomain};
pub use domain_extent::{
    DomainBounds, DomainExtent, RadiusPadding, SerializableDataExtents, SerializableDomainValue,
};
pub use extensions::ConfiguredScaleDataFusionExt;
pub use mark_scale_builder::{
    PreparedScaleMark, build_scale_builder_from_marks, build_scale_builder_from_prepared_marks,
};
pub use range::ScaleRange;
pub use scale::{
    BandScaleExt, LinearScaleExt, LogScaleExt, OrdinalScaleExt, PointScaleExt, PowScaleExt,
    ScaleDomainInferenceExt, ScaleRuntimeExt, SqrtScaleExt, SymlogScaleExt, TimeScaleExt,
};
pub use spec::{
    Band, Linear, Log, NestedBand, Ordinal, Point, Pow, Quantile, Quantize, Sqrt, Symlog,
    Threshold, Time, scale_spec_for_preference,
};
pub use udf::create_scale_udf;

/// How a plot-level scale is defined for a channel.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum PlotScaleSpec {
    /// Scale defined locally on this plot with configuration.
    Local(ScaleConfigSpec),
}

/// Wrapper that holds both the original `Scale<Auto>` specification and its
/// configured runtime scale.
///
/// This lets DataFusion expressions recreate the original scale UDF while most
/// runtime callers work directly with the configured scale.
#[derive(Debug, Clone)]
pub struct ConfiguredScaleWithSpec {
    /// The original scale specification.
    scale: Scale<Auto>,
    /// The configured scale with resolved domain/range.
    configured: ConfiguredScale,
    /// Semantic owner of the configured range.
    range_binding: ScaleRangeBinding,
    /// Runtime-derived scalar expressions referenced by channel config.
    derived_scalars: DerivedScalarMap,
}

impl ConfiguredScaleWithSpec {
    /// Create a new `ConfiguredScaleWithSpec`.
    pub fn new(scale: Scale<Auto>, configured: ConfiguredScale) -> Self {
        Self::with_range_binding(scale, configured, ScaleRangeBinding::Independent)
    }

    /// Create a new `ConfiguredScaleWithSpec` with explicit range binding metadata.
    pub fn with_range_binding(
        scale: Scale<Auto>,
        configured: ConfiguredScale,
        range_binding: ScaleRangeBinding,
    ) -> Self {
        Self {
            scale,
            configured,
            range_binding,
            derived_scalars: DerivedScalarMap::new(),
        }
    }

    /// Attach runtime-derived scalar expressions referenced by this channel.
    pub fn with_derived_scalars(mut self, derived_scalars: DerivedScalarMap) -> Self {
        self.derived_scalars = derived_scalars;
        self
    }

    /// Access the scale specification.
    pub fn spec(&self) -> &Scale<Auto> {
        &self.scale
    }

    /// Access the configured scale.
    pub fn configured(&self) -> &ConfiguredScale {
        &self.configured
    }

    /// Access runtime-derived scalar expressions referenced by this channel.
    pub fn derived_scalars(&self) -> &DerivedScalarMap {
        &self.derived_scalars
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
