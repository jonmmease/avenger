use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, CompiledGuide, CompiledMarkCore, CoordMeasurement, CoordinateGuide,
    GuideRenderContext, GuideSharingContext, GuideUpdate, LayoutBounds, OverflowSpaceRequirement,
    Theme,
};
use avenger_common::{
    types::{ImageAlign, ImageBaseline},
    value::ScalarOrArray,
};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{
    group::{Clip, SceneGroup},
    image::{SceneImageMark, SceneImageResource, SceneImageSource, SceneImageUnavailablePolicy},
    mark::SceneMark,
    text::SceneTextMark,
};
use avenger_text::measurement::TextMeasurer;
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    tiles::{PlannedTileUnavailablePolicy, VisibleRasterTile},
    viewport::WebMercatorCoordMeasurement,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WebMercatorGuide;

impl GuideUpdate for WebMercatorGuide {
    fn update(self, _other: Self) -> Self {
        self
    }
}

impl CoordinateGuide for WebMercatorGuide {
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
impl CompiledGuide for WebMercatorGuide {
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
        _text_measurer: &dyn TextMeasurer,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        Ok(OverflowSpaceRequirement::default())
    }

    async fn evaluate(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        _guide_overflow: &OverflowSpaceRequirement,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _ctx: &SessionContext,
        _data_override: Option<&DataFrame>,
        _sharing_context: GuideSharingContext<'_>,
        coord_measurement: &dyn CoordMeasurement,
        render_context: GuideRenderContext<'_>,
        _text_measurer: &dyn TextMeasurer,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let Some(measurement) = coord_measurement
            .as_any()
            .downcast_ref::<WebMercatorCoordMeasurement>()
        else {
            return Ok(Vec::new());
        };

        let mut plot_area_marks = Vec::new();
        let mut attribution_index = 0usize;
        for layer in &measurement.tile_layers {
            let plan = layer.tile_plan(&measurement.view)?;
            for request in plan.prefetch_requests {
                render_context.request_resource(request);
            }
            for planned in plan.rendered_tiles {
                render_context.request_resource(layer.resource_request(&planned.tile));
                plot_area_marks.push(tile_image_mark(layer.zindex_value(), &planned).into());
            }
            if let Some(attribution) = layer.attribution_text() {
                plot_area_marks.push(attribution_mark(
                    layer.layer_id(),
                    attribution,
                    render_context.plot_height(),
                    attribution_index,
                ));
                attribution_index += 1;
            }
        }
        if plot_area_marks.is_empty() {
            return Ok(Vec::new());
        }

        Ok(vec![
            SceneGroup {
                name: "webmercator-guide".to_string(),
                interactive: false,
                origin: [plot_bounds.x, plot_bounds.y],
                clip: Clip::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: plot_width,
                    height: plot_height,
                },
                marks: plot_area_marks,
                ..Default::default()
            }
            .into(),
        ])
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
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

fn tile_image_mark(zindex: i32, planned: &crate::tiles::PlannedRasterTile) -> SceneImageMark {
    let tile: &VisibleRasterTile = &planned.tile;
    let unavailable_policy = match planned.unavailable_policy {
        PlannedTileUnavailablePolicy::RendererDefault => {
            SceneImageUnavailablePolicy::RendererDefault
        }
        PlannedTileUnavailablePolicy::Skip => SceneImageUnavailablePolicy::Skip,
    };
    SceneImageMark {
        name: format!(
            "webmercator-tile-{}-{}-{}-{}-{}",
            tile.layer_id, tile.z, tile.unwrapped_x, tile.x, tile.y
        ),
        interactive: false,
        clip: true,
        len: 1,
        aspect: false,
        smooth: true,
        image: ScalarOrArray::new_scalar(SceneImageSource::Resource(SceneImageResource {
            key: tile.resource_key.clone(),
            intrinsic_width: tile.intrinsic_size,
            intrinsic_height: tile.intrinsic_size,
            fallback_key: None,
        })),
        x: ScalarOrArray::new_scalar(tile.pixel_x),
        y: ScalarOrArray::new_scalar(tile.pixel_y),
        width: ScalarOrArray::new_scalar(tile.pixel_width),
        height: ScalarOrArray::new_scalar(tile.pixel_height),
        align: ScalarOrArray::new_scalar(ImageAlign::Left),
        baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
        unavailable_policy,
        zindex: Some(zindex),
        ..Default::default()
    }
}

fn attribution_mark(
    layer_id: &str,
    attribution: &str,
    plot_height: f32,
    attribution_index: usize,
) -> SceneMark {
    SceneTextMark {
        name: format!("webmercator-attribution-{layer_id}"),
        interactive: false,
        clip: false,
        len: 1,
        text: ScalarOrArray::new_scalar(attribution.to_string()),
        x: ScalarOrArray::new_scalar(4.0),
        y: ScalarOrArray::new_scalar(plot_height - 4.0 - 12.0 * attribution_index as f32),
        font_size: ScalarOrArray::new_scalar(10.0),
        zindex: Some(100),
        ..Default::default()
    }
    .into()
}
