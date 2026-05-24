//! Integration test to verify external marks can be defined and used

use std::{marker::PhantomData, sync::Arc};

use avenger_chart::{
    cartesian::Cartesian,
    coords::CoordinateSystemTransform,
    define_position_channels,
    error::AvengerChartError,
    impl_mark_trait_common,
    marks::{CompiledMark, Mark},
    render::RenderContext,
};
use avenger_chart_core::{
    define_common_mark_channels, impl_mark_base, ChannelDescriptor, CompiledDataContext,
    CompiledMarkState, MarkState,
};
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::{arrow::record_batch::RecordBatch, scalar::ScalarValue};
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
            with_config: avenger_chart::cartesian::channels::CartesianPositionConfig,
        },
        y: {
            required: true,
            with_config: avenger_chart::cartesian::channels::CartesianPositionConfig,
        }
    }
}

// Implement the Mark trait for Cartesian
#[async_trait::async_trait]
impl Mark<Cartesian> for HexBin<Cartesian> {
    impl_mark_trait_common!(HexBin);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianHexBin {
            state: compiled_state,
        }))
    }
}

/// Compiled version of HexBin mark for rendering
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianHexBin {
    pub(crate) state: CompiledMarkState,
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianHexBin {
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

    async fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &RenderContext,
        _coord: Box<dyn CoordinateSystemTransform>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Custom hexbin rendering logic would go here
        // For this test, we just return an empty vector
        Ok(vec![])
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
}
