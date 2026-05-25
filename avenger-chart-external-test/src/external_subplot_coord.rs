//! Dogfood for external coordinate systems that support positioned `Subplot`.

use std::{any::Any, collections::HashMap, sync::Arc};

use async_trait::async_trait;
use avenger_chart_core::{
    compile_positioned_subplot_mark, AvengerChartError, CompiledGuide, CompiledMark,
    CompiledMarkCore, CompiledMarkState, CoordMeasurement, CoordinateGuide, CoordinateSystem,
    CoordinateSystemCore, CoordinateSystemTransform, CoordinateSystemTransformCore,
    GuideSharingContext, GuideUpdate, LayoutBounds, OverflowSpaceRequirement,
    PlotAreaRangeEndpoint, PlotGeometry, PointGeometry, PositionedSubplotChannel,
    PositionedSubplotSpec, ScaleRangeBinding, SubplotContainerCoordinateSystem, SubplotMarkCore,
    Theme,
};
use avenger_chart_marks::Subplot;
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark};
use datafusion::{common::ScalarValue, dataframe::DataFrame};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub const EXTERNAL_SUBPLOT_U_CHANNEL: &str = "subplot_u";
pub const EXTERNAL_SUBPLOT_V_CHANNEL: &str = "subplot_v";
pub const EXTERNAL_SUBPLOT_PARTITION_CHANNEL: &str = "partition";

/// Minimal external coordinate system that opts into generic positioned subplot runtime.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExternalSubplotCoord;

pub trait ExternalSubplotPositionChannels: Sized {
    fn subplot_u<V: Into<avenger_chart_core::ChannelValue>>(self, value: V) -> Self;
    fn subplot_v<V: Into<avenger_chart_core::ChannelValue>>(self, value: V) -> Self;
    fn partition_by<V: Into<avenger_chart_core::ChannelValue>>(self, value: V) -> Self;
    fn plot_size(self, width: f32, height: f32) -> Self;
}

impl ExternalSubplotPositionChannels for Subplot<ExternalSubplotCoord> {
    fn subplot_u<V: Into<avenger_chart_core::ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value(EXTERNAL_SUBPLOT_U_CHANNEL, value.into())
    }

    fn subplot_v<V: Into<avenger_chart_core::ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value(EXTERNAL_SUBPLOT_V_CHANNEL, value.into())
    }

    fn partition_by<V: Into<avenger_chart_core::ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value(EXTERNAL_SUBPLOT_PARTITION_CHANNEL, value.into().no_scale())
    }

    fn plot_size(mut self, width: f32, height: f32) -> Self {
        self.set_plot_width_config(Some(width));
        self.set_plot_height_config(Some(height));
        self
    }
}

impl CoordinateSystemCore for ExternalSubplotCoord {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }
}

impl CoordinateSystem for ExternalSubplotCoord {
    type Guide = ExternalSubplotCoordGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(ExternalSubplotCoordTransform)
    }
}

#[async_trait]
impl SubplotContainerCoordinateSystem for ExternalSubplotCoord {
    async fn compile_subplot_mark(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        compile_positioned_subplot_mark(
            subplot,
            compiled_state,
            session_context,
            PositionedSubplotSpec::new(
                "ExternalSubplotCoord",
                "external_subplot",
                vec![
                    PositionedSubplotChannel::new(
                        EXTERNAL_SUBPLOT_U_CHANNEL,
                        EXTERNAL_SUBPLOT_U_CHANNEL,
                    ),
                    PositionedSubplotChannel::new(
                        EXTERNAL_SUBPLOT_V_CHANNEL,
                        EXTERNAL_SUBPLOT_V_CHANNEL,
                    ),
                ],
            )
            .with_partition_channel(EXTERNAL_SUBPLOT_PARTITION_CHANNEL),
        )
        .await
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExternalSubplotCoordTransform;

impl CoordinateSystemTransformCore for ExternalSubplotCoordTransform {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        _position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        let x = position_channels
            .get(EXTERNAL_SUBPLOT_U_CHANNEL)
            .ok_or_else(|| {
                AvengerChartError::InternalError("Missing subplot_u position channel".to_string())
            })?
            .clone();
        let y = position_channels
            .get(EXTERNAL_SUBPLOT_V_CHANNEL)
            .ok_or_else(|| {
                AvengerChartError::InternalError("Missing subplot_v position channel".to_string())
            })?
            .clone();
        Ok(Box::new(PointGeometry { x, y }))
    }

    fn default_range_binding(&self, channel: &str) -> Option<ScaleRangeBinding> {
        match channel {
            EXTERNAL_SUBPLOT_U_CHANNEL => Some(ScaleRangeBinding::plot_area(
                PlotAreaRangeEndpoint::ZERO,
                PlotAreaRangeEndpoint::WIDTH,
            )),
            EXTERNAL_SUBPLOT_V_CHANNEL => Some(ScaleRangeBinding::plot_area(
                PlotAreaRangeEndpoint::HEIGHT,
                PlotAreaRangeEndpoint::ZERO,
            )),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn avenger_scales::scales::ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        HashMap::new()
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for ExternalSubplotCoordTransform {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExternalSubplotCoordGuide;

impl GuideUpdate for ExternalSubplotCoordGuide {
    fn update(self, _other: Self) -> Self {
        self
    }
}

impl CoordinateGuide for ExternalSubplotCoordGuide {
    type Axis = ();

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {}

    fn set_compiled_marks<M>(
        &mut self,
        _compiled_marks: &[Arc<M>],
        _session_context: &datafusion::prelude::SessionContext,
    ) where
        M: CompiledMarkCore + ?Sized,
    {
    }

    fn update(&mut self, _other: Self) {}

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[async_trait]
#[typetag::serde]
impl CompiledGuide for ExternalSubplotCoordGuide {
    async fn measure_overflow(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _data_override: Option<&DataFrame>,
        _ctx: &datafusion::prelude::SessionContext,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        Ok(Default::default())
    }

    async fn evaluate(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _plot_bounds: &LayoutBounds,
        _guide_overflow: &OverflowSpaceRequirement,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _ctx: &datafusion::prelude::SessionContext,
        _data_override: Option<&DataFrame>,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Ok(Vec::new())
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Clip {
        Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
