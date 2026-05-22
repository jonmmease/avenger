//! Compile-time dogfood for external subplot container coordinate systems.
//!
//! This module intentionally stops at the compile boundary. A full external
//! child-frame container still needs public child-frame measurement and
//! placement services before it can render children with the same machinery as
//! built-in concat/facet.

use std::{any::Any, collections::HashMap, sync::Arc};

use async_trait::async_trait;
use avenger_chart::{
    channel::ChannelDescriptor,
    container::{
        compile_subplot_payload, CompiledSubplotPayload, SubplotContainerCoordinateSystem,
    },
    coords::{
        CoordMeasurement, CoordinateSystem, CoordinateSystemTransform, PlotGeometry, PointGeometry,
    },
    error::AvengerChartError,
    guide::{CompiledGuide, CoordinateGuide, GuideSharingContext, GuideUpdate},
    marks::{CompiledDataContext, CompiledMark, CompiledMarkState, Subplot},
    render::RenderContext,
};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark};
use datafusion::{arrow::record_batch::RecordBatch, common::ScalarValue, dataframe::DataFrame};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Minimal external container coordinate system.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExternalStack;

impl CoordinateSystem for ExternalStack {
    type Guide = ExternalStackGuide;

    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(ExternalStackTransform)
    }
}

#[async_trait]
impl SubplotContainerCoordinateSystem for ExternalStack {
    async fn compile_subplot_mark(
        subplot: &Subplot<Self>,
        compiled_state: CompiledMarkState,
        session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledExternalStackSubplot {
            payload: compile_subplot_payload(subplot, compiled_state, session_context).await?,
        }))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExternalStackTransform;

#[async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for ExternalStackTransform {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn avenger_scales::scales::ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        HashMap::new()
    }

    fn transform(
        &self,
        _position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        _position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        Ok(Box::new(PointGeometry {
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
        }))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExternalStackGuide;

impl GuideUpdate for ExternalStackGuide {
    fn update(self, _other: Self) -> Self {
        self
    }
}

impl CoordinateGuide for ExternalStackGuide {
    type Axis = ();

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {}

    fn set_compiled_marks(
        &mut self,
        _compiled_marks: Vec<Arc<dyn CompiledMark>>,
        _session_context: &datafusion::prelude::SessionContext,
    ) {
    }

    fn update(&mut self, _other: Self) {}

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[async_trait]
#[typetag::serde]
impl CompiledGuide for ExternalStackGuide {
    async fn measure_overflow(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &avenger_chart::theme::Theme,
        _params: &IndexMap<String, ScalarValue>,
        _data_override: Option<&DataFrame>,
        _ctx: &datafusion::prelude::SessionContext,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<avenger_chart::guide::OverflowSpaceRequirement, AvengerChartError> {
        Ok(Default::default())
    }

    async fn evaluate(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _plot_bounds: &avenger_chart::layout::LayoutBounds,
        _guide_overflow: &avenger_chart::guide::OverflowSpaceRequirement,
        _theme: &avenger_chart::theme::Theme,
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

/// Compiled external subplot mark that proves `Subplot<ExternalStack>` can use
/// Avenger's blanket subplot-container compile path from another crate.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledExternalStackSubplot {
    payload: CompiledSubplotPayload,
}

impl CompiledExternalStackSubplot {
    pub fn payload(&self) -> &CompiledSubplotPayload {
        &self.payload
    }
}

#[typetag::serde]
#[async_trait]
impl CompiledMark for CompiledExternalStackSubplot {
    fn state(&self) -> &CompiledMarkState {
        self.payload.compiled_state()
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        self.payload.compiled_state_mut()
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.payload.compiled_state().data
    }

    fn mark_type(&self) -> &str {
        "external_stack_subplot"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        Vec::new()
    }

    fn wants_full_data_batch(&self) -> bool {
        true
    }

    async fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &RenderContext,
        _coord: Box<dyn CoordinateSystemTransform>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Ok(Vec::new())
    }
}
