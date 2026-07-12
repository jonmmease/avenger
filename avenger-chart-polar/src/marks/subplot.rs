use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelValue, CompileContext, CompiledMark, CompiledMarkState,
    CompiledPositionedSubplot, Mark, PositionConfig, PositionedSubplotChannel,
    PositionedSubplotSpec, SubplotContainerCoordinateSystem, SubplotMarkCore,
    compile_positioned_subplot_mark, compile_positioned_subplot_mark_with_context,
};
use avenger_chart_marks::Subplot;
use datafusion::prelude::SessionContext;

use crate::{Polar, PolarPositionConfig};

#[doc(hidden)]
pub const POLAR_SUBPLOT_PARTITION_CHANNEL: &str = "partition";

pub type CompiledPolarSubplot = CompiledPositionedSubplot;

/// Position-channel builder methods for `Subplot<Polar>`.
pub trait PolarSubplotPositionChannels: Sized {
    /// Set the parent radial position for coordinate-positioned child plot frames.
    fn r<V: Into<ChannelValue>>(self, value: V) -> Self;

    /// Set the parent angular position for coordinate-positioned child plot frames.
    fn theta<V: Into<ChannelValue>>(self, value: V) -> Self;

    /// Partition parent data into one coordinate-positioned child frame per value.
    fn partition_by<V: Into<ChannelValue>>(self, value: V) -> Self;

    /// Configure the parent radial position channel for child plot frames.
    fn r_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PolarPositionConfig) -> PolarPositionConfig;

    /// Configure the parent angular position channel for child plot frames.
    fn theta_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PolarPositionConfig) -> PolarPositionConfig;

    /// Set the child plot-area width used for each positioned child frame.
    fn plot_width<W: avenger_chart_core::IntoExpr>(self, width: W) -> Self;

    /// Set the child plot-area height used for each positioned child frame.
    fn plot_height<H: avenger_chart_core::IntoExpr>(self, height: H) -> Self;

    /// Set both child plot-area dimensions used for each positioned child frame.
    fn plot_size<W, H>(self, width: W, height: H) -> Self
    where
        W: avenger_chart_core::IntoExpr,
        H: avenger_chart_core::IntoExpr;
}

impl PolarSubplotPositionChannels for Subplot<Polar> {
    fn r<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("r", value.into())
    }

    fn theta<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("theta", value.into())
    }

    fn partition_by<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value(POLAR_SUBPLOT_PARTITION_CHANNEL, value.into().no_scale())
    }

    fn r_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PolarPositionConfig) -> PolarPositionConfig,
    {
        with_position_config(self, "r", value.into(), f)
    }

    fn theta_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PolarPositionConfig) -> PolarPositionConfig,
    {
        with_position_config(self, "theta", value.into(), f)
    }

    fn plot_width<W: avenger_chart_core::IntoExpr>(mut self, width: W) -> Self {
        self.set_plot_width_config(width);
        self
    }

    fn plot_height<H: avenger_chart_core::IntoExpr>(mut self, height: H) -> Self {
        self.set_plot_height_config(height);
        self
    }

    fn plot_size<W, H>(mut self, width: W, height: H) -> Self
    where
        W: avenger_chart_core::IntoExpr,
        H: avenger_chart_core::IntoExpr,
    {
        self.set_plot_width_config(width);
        self.set_plot_height_config(height);
        self
    }
}

fn with_position_config<F>(
    mark: Subplot<Polar>,
    channel: &'static str,
    value: ChannelValue,
    f: F,
) -> Subplot<Polar>
where
    F: FnOnce(PolarPositionConfig) -> PolarPositionConfig,
{
    let configured = f(PolarPositionConfig::new(value));
    let (channel_value, axis_config) = configured.take_axis_config();
    let mut mark = mark.with_channel_value(channel, channel_value);
    if let Some(axis_config) = axis_config {
        mark.state_mut()
            .axis_configs
            .insert(channel.to_string(), Arc::new(axis_config));
    }
    mark
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl SubplotContainerCoordinateSystem for Polar {
    async fn compile_subplot_mark(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        compile_positioned_subplot_mark(
            subplot,
            compiled_state,
            session_context,
            PositionedSubplotSpec::new(
                "Polar",
                "polar_subplot",
                vec![
                    PositionedSubplotChannel::new("r", "r"),
                    PositionedSubplotChannel::new("theta", "theta"),
                ],
            )
            .with_partition_channel(POLAR_SUBPLOT_PARTITION_CHANNEL),
        )
        .await
    }

    async fn compile_subplot_mark_with_context(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
        compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        compile_positioned_subplot_mark_with_context(
            subplot,
            compiled_state,
            session_context,
            PositionedSubplotSpec::new(
                "Polar",
                "polar_subplot",
                vec![
                    PositionedSubplotChannel::new("r", "r"),
                    PositionedSubplotChannel::new("theta", "theta"),
                ],
            )
            .with_partition_channel(POLAR_SUBPLOT_PARTITION_CHANNEL),
            compile_context,
        )
        .await
    }
}
