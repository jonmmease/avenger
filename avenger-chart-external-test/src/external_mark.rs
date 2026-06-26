//! Integration test to verify external marks can be defined and used

use std::{any::Any, marker::PhantomData, sync::Arc};

use avenger_chart_cartesian::{Cartesian, CartesianPositionConfig};
use avenger_chart_core::{
    define_common_mark_channels, define_position_channels, impl_mark_base, impl_mark_trait_common,
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemCore, CoordinateSystemTransformCore, Legend, LegendChannel,
    LegendRenderOutput, LegendRenderer, LegendRendererSelection, Mark, MarkRuntimeContext,
    MarkState, Size2D, Theme,
};
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::{arrow::record_batch::RecordBatch, prelude::SessionContext, scalar::ScalarValue};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// A custom hexbin mark defined in an external crate
pub struct HexBin<C> {
    pub(crate) state: MarkState,
    pub(crate) _phantom: PhantomData<C>,
}

// Use the exported macro for base implementation
impl_mark_base!(HexBin);

// Define common channels
define_common_mark_channels! {
    HexBin {
        fill: {},
        stroke: {},
        opacity: {},
        size: {},
    }
}

// Define position channels for Cartesian HexBin using the macro
define_position_channels! {
    HexBin<Cartesian> {
        x: {
            required: true,
            with_config: CartesianPositionConfig,
        },
        y: {
            required: true,
            with_config: CartesianPositionConfig,
        }
    }
}

#[async_trait::async_trait]
impl<C> Mark<C> for HexBin<C>
where
    C: CoordinateSystemCore,
{
    impl_mark_trait_common!(HexBin);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledHexBin {
            state: compiled_state,
        }))
    }
}

/// Compiled version of HexBin mark for rendering
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledHexBin {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledHexBin {
    fn state(&self) -> &CompiledMarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        &mut self.state
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.state.data
    }

    fn mark_type(&self) -> &str {
        "hexbin"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "x",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "y",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "size",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "fill",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "opacity",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        match channel {
            "size" => Some(ScalarValue::Float32(Some(20.0))),
            "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))),
            "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
            "opacity" => Some(ScalarValue::Float32(Some(1.0))),
            _ => None,
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        _scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        match channel {
            "fill" | "stroke" => Some(LegendRendererSelection::Custom(Arc::new(
                HexBinLegendRenderer,
            ))),
            _ => None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct HexBinLegendRenderer;

#[typetag::serde]
#[async_trait::async_trait]
impl LegendRenderer for HexBinLegendRenderer {
    fn name(&self) -> &'static str {
        "HexBinLegendRenderer"
    }

    fn can_evaluate(&self, channels: &[LegendChannel]) -> bool {
        channels
            .iter()
            .any(|channel| matches!(channel.channel_type.as_str(), "fill" | "stroke"))
    }

    async fn evaluate(
        &self,
        _channels: &[LegendChannel],
        _config: &Legend,
        _x: f32,
        _y: f32,
        _width: f32,
        _height: f32,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _ctx: &SessionContext,
    ) -> Result<Option<LegendRenderOutput>, AvengerChartError> {
        Ok(None)
    }

    async fn measure(
        &self,
        _channels: &[LegendChannel],
        _config: &Legend,
        _available_space: Size2D,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _ctx: &SessionContext,
    ) -> Result<Size2D, AvengerChartError> {
        Ok(Size2D {
            width: 24.0,
            height: 18.0,
        })
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledHexBin {
    async fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Custom hexbin rendering logic would go here
        // For this test, we just return an empty vector
        Ok(vec![])
    }
}
