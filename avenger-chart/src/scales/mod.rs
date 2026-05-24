pub mod builder {
    pub use avenger_chart_scales::builder::*;
}

pub use avenger_chart_scales::{
    Auto, AvengerChartExtensionCodec, Band, ChannelScaleData, ConfiguredScaleDataFusionExt,
    ConfiguredScaleLegendExt, ConfiguredScaleWithSpec, DataExtents, DefaultScaleRangeResolver,
    DomainBounds, DomainExpr, DomainExtent, DomainValues, Linear, Log, Ordinal, PlotAreaDimension,
    PlotAreaRangeEndpoint, PlotAreaRangeExpr, Point, Pow, Quantile, Quantize, RadiusPadding,
    ResolvedDomain, Scale, ScaleBuilder, ScaleChannelConfig, ScaleChannelValue, ScaleConfigSpec,
    ScaleDefaultDomain, ScaleDomain, ScaleRange, ScaleRangeBinding, ScaleSpec,
    SerializableDataExtents, SerializableDomainValue, Sqrt, Symlog, Threshold, Time,
    create_scale_udf, default_range_for_channel, scale_spec_for_preference,
};

pub use avenger_chart_scales::{
    channel_config, codec, defaults, domain, domain_extent, extensions, range, scale, spec, udf,
};

pub(crate) fn default_range_for_compiled_marks<'a>(
    compiled_marks: &'a [std::sync::Arc<dyn crate::marks::CompiledMark>],
) -> impl Fn(
    &str,
    &dyn avenger_scales::scales::ScaleImpl,
    &crate::chart_core::ResolvedDomain,
    &datafusion::arrow::datatypes::DataType,
    &crate::theme::Theme,
    &indexmap::IndexMap<String, datafusion_common::ScalarValue>,
) -> Option<crate::chart_core::ScaleRange>
+ 'a {
    move |channel_name, scale_impl, resolved_domain, data_type, theme, params| {
        for mark in compiled_marks {
            if let Some(mark_range) = mark.default_channel_range(
                channel_name,
                scale_impl,
                resolved_domain,
                data_type,
                theme,
                params,
            ) {
                return Some(mark_range);
            }
        }

        None
    }
}
