use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelValue, CompiledMark, CompiledMarkState, CompiledPositionedSubplot,
    Mark, PositionConfig, PositionedSubplotChannel, PositionedSubplotSpec,
    SubplotContainerCoordinateSystem, SubplotMarkCore, compile_positioned_subplot_mark,
};
use avenger_chart_marks::Subplot;
use datafusion::prelude::SessionContext;

use crate::{Cartesian, CartesianPositionConfig};

#[doc(hidden)]
pub const CARTESIAN_SUBPLOT_PARTITION_CHANNEL: &str = "partition";
#[doc(hidden)]
pub const CARTESIAN_SUBPLOT_X_CHANNEL: &str = "subplot_x";
#[doc(hidden)]
pub const CARTESIAN_SUBPLOT_Y_CHANNEL: &str = "subplot_y";

pub type CompiledCartesianSubplot = CompiledPositionedSubplot;

/// Position-channel builder methods for `Subplot<Cartesian>`.
pub trait CartesianSubplotPositionChannels: Sized {
    /// Set the parent x-position for coordinate-positioned child plot frames.
    fn subplot_x<V: Into<ChannelValue>>(self, value: V) -> Self;

    /// Set the parent y-position for coordinate-positioned child plot frames.
    fn subplot_y<V: Into<ChannelValue>>(self, value: V) -> Self;

    /// Partition parent data into one coordinate-positioned child frame per value.
    fn partition_by<V: Into<ChannelValue>>(self, value: V) -> Self;

    /// Configure the parent x-position channel for coordinate-positioned child plot frames.
    fn subplot_x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig;

    /// Configure the parent y-position channel for coordinate-positioned child plot frames.
    fn subplot_y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig;

    /// Set the child plot-area width used for each positioned child frame.
    fn plot_width(self, width: f32) -> Self;

    /// Set the child plot-area height used for each positioned child frame.
    fn plot_height(self, height: f32) -> Self;

    /// Set both child plot-area dimensions used for each positioned child frame.
    fn plot_size(self, width: f32, height: f32) -> Self;
}

impl CartesianSubplotPositionChannels for Subplot<Cartesian> {
    fn subplot_x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value(CARTESIAN_SUBPLOT_X_CHANNEL, value.into())
    }

    fn subplot_y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value(CARTESIAN_SUBPLOT_Y_CHANNEL, value.into())
    }

    fn partition_by<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value(CARTESIAN_SUBPLOT_PARTITION_CHANNEL, value.into().no_scale())
    }

    fn subplot_x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        with_position_config(self, CARTESIAN_SUBPLOT_X_CHANNEL, value.into(), f)
    }

    fn subplot_y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        with_position_config(self, CARTESIAN_SUBPLOT_Y_CHANNEL, value.into(), f)
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
    mark: Subplot<Cartesian>,
    channel: &'static str,
    value: ChannelValue,
    f: F,
) -> Subplot<Cartesian>
where
    F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
{
    let configured = f(CartesianPositionConfig::new(value));
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
impl SubplotContainerCoordinateSystem for Cartesian {
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
                "Cartesian",
                "cartesian_subplot",
                vec![
                    PositionedSubplotChannel::new(CARTESIAN_SUBPLOT_X_CHANNEL, "x"),
                    PositionedSubplotChannel::new(CARTESIAN_SUBPLOT_Y_CHANNEL, "y"),
                ],
            )
            .with_partition_channel(CARTESIAN_SUBPLOT_PARTITION_CHANNEL),
        )
        .await
    }
}
