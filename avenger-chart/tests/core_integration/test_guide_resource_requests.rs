use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_chart::prelude::Plot;
use avenger_chart_core::{
    AvengerChartError, CompiledGuide, CompiledMarkCore, CoordMeasurement, CoordinateGuide,
    CoordinateSystem, CoordinateSystemCore, CoordinateSystemTransform,
    CoordinateSystemTransformCore, GuideRenderContext, GuideSharingContext, LayoutBounds,
    OverflowSpaceRequirement, PlotGeometry, PointGeometry, ScaleRangeBinding, Theme,
};
use avenger_common::value::ScalarOrArray;
use avenger_resource::{
    ResourceCachePolicy, ResourceKey, ResourceKind, ResourceRequest, ResourceRequestPurpose,
    ResourceSource,
};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark};
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[tokio::test]
async fn coordinate_guide_can_request_image_resources() {
    let ctx = SessionContext::new();
    let compiled = Plot::with_coord(ResourceGuideCoord)
        .compile(&ctx)
        .await
        .expect("compile plot");

    let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate plot");

    assert_eq!(evaluated.resource_requests, vec![guide_resource_request()]);
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ResourceGuideCoord;

impl CoordinateSystemCore for ResourceGuideCoord {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }
}

impl CoordinateSystem for ResourceGuideCoord {
    type Guide = ResourceGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystemTransformCore for ResourceGuideCoord {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
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

    fn default_range_binding(&self, _channel: &str) -> Option<ScaleRangeBinding> {
        None
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        HashMap::new()
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for ResourceGuideCoord {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ResourceGuide;

impl CoordinateGuide for ResourceGuide {
    type Axis = ();

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {}

    fn set_compiled_marks<M>(
        &mut self,
        _compiled_marks: &[Arc<M>],
        _session_context: &SessionContext,
    ) where
        M: CompiledMarkCore + ?Sized,
    {
    }

    fn update(&mut self, _other: Self) {}

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for ResourceGuide {
    async fn measure_overflow(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _data_override: Option<&DataFrame>,
        _ctx: &SessionContext,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: Option<&dyn CoordMeasurement>,
        _text_measurer: &dyn avenger_text::measurement::TextMeasurer,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        Ok(OverflowSpaceRequirement::default())
    }

    async fn evaluate(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _plot_bounds: &LayoutBounds,
        _guide_overflow: &OverflowSpaceRequirement,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _ctx: &SessionContext,
        _data_override: Option<&DataFrame>,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: &dyn CoordMeasurement,
        render_context: GuideRenderContext<'_>,
        _text_measurer: &dyn avenger_text::measurement::TextMeasurer,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        render_context.request_resource(guide_resource_request());
        Ok(Vec::new())
    }

    fn get_clip(
        &self,
        _plot_width: f32,
        _plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip {
        Clip::None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn guide_resource_request() -> ResourceRequest {
    ResourceRequest {
        key: ResourceKey::new("guide-tile"),
        kind: ResourceKind::new("image"),
        source: ResourceSource::DataUri {
            data_uri: "data:image/png;base64,unused".to_string(),
        },
        priority: 0.0,
        cache_policy: ResourceCachePolicy::default(),
        purpose: ResourceRequestPurpose::Required,
    }
}
