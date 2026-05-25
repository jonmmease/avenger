use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelValue, CompiledMark, CompiledMarkState, CompiledPositionedSubplot,
    Mark, PositionConfig, PositionedSubplotChannel, PositionedSubplotSpec,
    SubplotContainerCoordinateSystem, SubplotMarkCore, compile_positioned_subplot_mark,
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
    fn plot_width(self, width: f32) -> Self;

    /// Set the child plot-area height used for each positioned child frame.
    fn plot_height(self, height: f32) -> Self;

    /// Set both child plot-area dimensions used for each positioned child frame.
    fn plot_size(self, width: f32, height: f32) -> Self;
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

    fn plot_width(mut self, width: f32) -> Self {
        self.set_plot_width_config(Some(width));
        self
    }

    fn plot_height(mut self, height: f32) -> Self {
        self.set_plot_height_config(Some(height));
        self
    }

    fn plot_size(mut self, width: f32, height: f32) -> Self {
        self.set_plot_width_config(Some(width));
        self.set_plot_height_config(Some(height));
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

#[async_trait::async_trait]
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
}
