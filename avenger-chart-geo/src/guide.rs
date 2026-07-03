//! The `Geo` coordinate guide: sphere outline and graticule, streamed
//! through the projection pipeline so they bend, cut at the antimeridian,
//! and clip to the plot rectangle correctly.
//!
//! Structure mirrors `avenger-chart-webmercator/src/guide.rs` (a
//! measurement-driven guide with no axis machinery).

use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, CompiledGuide, CompiledMarkCore, CoordMeasurement, CoordinateGuide,
    GuideRenderContext, GuideSharingContext, GuideUpdate, LayoutBounds, OverflowSpaceRequirement,
    Theme,
};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{PathTransform, StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use avenger_geo::graticule::Graticule;
use avenger_geo::sinks::LyonPathSink;
use avenger_geo::streamable::Sphere;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{
    group::{Clip, SceneGroup},
    mark::SceneMark,
    path::ScenePathMark,
};
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::view::GeoCoordMeasurement;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GeoGuide;

impl GuideUpdate for GeoGuide {
    fn update(self, _other: Self) -> Self {
        self
    }
}

impl CoordinateGuide for GeoGuide {
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
impl CompiledGuide for GeoGuide {
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
        _render_context: GuideRenderContext<'_>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let Some(measurement) = GeoCoordMeasurement::downcast(coord_measurement) else {
            return Ok(Vec::new());
        };
        if measurement.sphere.is_none() && measurement.graticule.is_none() {
            return Ok(Vec::new());
        }

        let projector = measurement.view_projector();
        let mut plot_area_marks: Vec<SceneMark> = Vec::new();

        if let Some(style) = &measurement.sphere {
            let mut sink = LyonPathSink::fill();
            projector.stream(&Sphere, &mut sink);
            if sink.has_content() {
                plot_area_marks.push(
                    path_mark(
                        "geo-sphere",
                        sink.finish(),
                        ColorOrGradient::Color(style.fill),
                        ColorOrGradient::Color(style.stroke),
                        Some(style.stroke_width),
                        0,
                    )
                    .into(),
                );
            }
        }

        if let Some(style) = &measurement.graticule {
            let graticule = Graticule {
                step_minor: [style.step[0], style.step[1]],
                ..Graticule::default()
            };
            let mut sink = LyonPathSink::stroke();
            projector.stream(&graticule.lines(), &mut sink);
            if sink.has_content() {
                plot_area_marks.push(
                    path_mark(
                        "geo-graticule",
                        sink.finish(),
                        ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
                        ColorOrGradient::Color(style.stroke),
                        Some(style.stroke_width),
                        1,
                    )
                    .into(),
                );
            }
        }

        if plot_area_marks.is_empty() {
            return Ok(Vec::new());
        }

        Ok(vec![
            SceneGroup {
                name: "geo-guide".to_string(),
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

fn path_mark(
    name: &str,
    path: lyon_path::Path,
    fill: ColorOrGradient,
    stroke: ColorOrGradient,
    stroke_width: Option<f32>,
    zindex: i32,
) -> ScenePathMark {
    ScenePathMark {
        name: name.to_string(),
        interactive: false,
        clip: true,
        len: 1,
        gradients: Vec::new(),
        stroke_cap: StrokeCap::Round,
        stroke_join: StrokeJoin::Round,
        stroke_width,
        path: ScalarOrArray::new_scalar(path),
        fill: ScalarOrArray::new_scalar(fill),
        fill_pattern: ScalarOrArray::new_scalar(None),
        stroke: ScalarOrArray::new_scalar(stroke),
        transform: ScalarOrArray::new_scalar(PathTransform::identity()),
        indices: None,
        zindex: Some(zindex),
    }
}
