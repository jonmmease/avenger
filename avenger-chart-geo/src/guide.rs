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
use avenger_common::types::{ImageAlign, ImageBaseline};
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
    image::{SceneImageMark, SceneImageResource, SceneImageSource, SceneImageUnavailablePolicy},
    mark::SceneMark,
    path::ScenePathMark,
    text::SceneTextMark,
    warped_image::SceneWarpedImageMark,
};
use avenger_text::types::FontStyle;
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::tiles::{
    GeoZoomPrefetchPlanner, PlannedGeoTile, PlannedTileUnavailablePolicy, TileLoadingPolicy,
    tile_mesh,
};
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
        render_context: GuideRenderContext<'_>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let Some(measurement) = GeoCoordMeasurement::downcast(coord_measurement) else {
            return Ok(Vec::new());
        };
        if measurement.sphere.is_none()
            && measurement.graticule.is_none()
            && measurement.tile_layers.is_empty()
        {
            return Ok(Vec::new());
        }

        let projector = measurement.view_projector();
        let mut plot_area_marks: Vec<SceneMark> = Vec::new();

        let identity_tiles = crate::tiles::is_identity_fast_path(measurement);
        let mut attribution_index = 0usize;
        for layer in &measurement.tile_layers {
            let plan = layer.tile_plan(measurement, [plot_bounds.x, plot_bounds.y])?;
            for request in plan.prefetch_requests {
                render_context.request_resource(request);
            }
            // Publish a hover-retarget planner so the fetch scheduler can
            // re-anchor this layer's zoom-prefetch set as the cursor moves
            // between evaluations.
            if matches!(
                layer.loading_policy_value(),
                TileLoadingPolicy::SmoothZoom { .. }
            ) {
                let targets = plan
                    .rendered_tiles
                    .iter()
                    .filter(|planned| planned.is_target)
                    .map(|planned| planned.tile.clone())
                    .collect::<Vec<_>>();
                render_context.publish_prefetch_planner(Arc::new(
                    GeoZoomPrefetchPlanner::from_snapshot(
                        layer.clone(),
                        measurement,
                        targets,
                        [plot_bounds.x, plot_bounds.y],
                    ),
                ));
            }
            for planned in plan.rendered_tiles {
                // Only target tiles are fetched as Required; fallback-zoom
                // tiles render opportunistically from cache (their pixels
                // come from earlier target/prefetch fetches), keeping
                // request volume at viewport + modest look-ahead.
                if planned.is_target {
                    render_context.request_resource(layer.resource_request_with_purpose(
                        &planned.tile,
                        avenger_resource::ResourceRequestPurpose::Required,
                        planned.fetch_priority,
                    ));
                }
                let mark = if identity_tiles {
                    tile_image_mark(layer.zindex_value(), &planned, &measurement.view)
                } else {
                    tile_warped_mark(
                        layer.zindex_value(),
                        &planned,
                        &projector,
                        measurement.projection.precision,
                        plot_width,
                        plot_height,
                    )
                };
                plot_area_marks.extend(mark);
            }
            if let Some(attribution) = layer.attribution_text() {
                plot_area_marks.push(attribution_mark(
                    layer.layer_id(),
                    attribution,
                    plot_height,
                    attribution_index,
                ));
                attribution_index += 1;
            }
        }

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
                        // Between the canvas background (-100) and data
                        // marks (0), like axis grid lines (-1); tile
                        // layers default to -4, above this world fill.
                        -5,
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
                        -2,
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

fn tile_unavailable_policy(planned: &PlannedGeoTile) -> SceneImageUnavailablePolicy {
    match planned.unavailable_policy {
        PlannedTileUnavailablePolicy::RendererDefault => {
            SceneImageUnavailablePolicy::RendererDefault
        }
        PlannedTileUnavailablePolicy::Skip => SceneImageUnavailablePolicy::Skip,
    }
}

fn tile_source(planned: &PlannedGeoTile) -> SceneImageSource {
    SceneImageSource::Resource(SceneImageResource {
        key: planned.tile.resource_key.clone(),
        intrinsic_width: planned.tile.intrinsic_size,
        intrinsic_height: planned.tile.intrinsic_size,
        fallback_key: None,
    })
}

/// Identity fast path: an axis-aligned image mark, exactly like the
/// WebMercator tile guide.
fn tile_image_mark(
    zindex: i32,
    planned: &PlannedGeoTile,
    view: &crate::view::GeoView,
) -> Option<SceneMark> {
    let [x, y, width, height] = crate::tiles::tile_pixel_rect(&planned.tile, view);
    let tile = &planned.tile;
    Some(
        SceneImageMark {
            name: format!(
                "geo-tile-{}-{}-{}-{}-{}",
                tile.layer_id, tile.z, tile.unwrapped_x, tile.x, tile.y
            ),
            interactive: false,
            clip: true,
            len: 1,
            aspect: false,
            smooth: true,
            image: ScalarOrArray::new_scalar(tile_source(planned)),
            x: ScalarOrArray::new_scalar(x),
            y: ScalarOrArray::new_scalar(y),
            width: ScalarOrArray::new_scalar(width),
            height: ScalarOrArray::new_scalar(height),
            align: ScalarOrArray::new_scalar(ImageAlign::Left),
            baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
            unavailable_policy: tile_unavailable_policy(planned),
            zindex: Some(zindex),
            tile_texture_size: Some(tile.intrinsic_size),
            ..Default::default()
        }
        .into(),
    )
}

/// General path: the tile as a textured mesh warped through the view
/// projector.
fn tile_warped_mark(
    zindex: i32,
    planned: &PlannedGeoTile,
    projector: &avenger_geo::projector::Projector,
    precision_px: f64,
    plot_width: f32,
    plot_height: f32,
) -> Option<SceneMark> {
    let mesh = tile_mesh(
        &planned.tile,
        projector,
        precision_px,
        plot_width,
        plot_height,
    )?;
    let tile = &planned.tile;
    Some(
        SceneWarpedImageMark {
            name: format!(
                "geo-tile-{}-{}-{}-{}-{}",
                tile.layer_id, tile.z, tile.unwrapped_x, tile.x, tile.y
            ),
            interactive: false,
            clip: true,
            smooth: true,
            image: tile_source(planned),
            positions: mesh.positions,
            uvs: mesh.uvs,
            indices: mesh.indices,
            unavailable_policy: tile_unavailable_policy(planned),
            zindex: Some(zindex),
            tile_texture_size: Some(tile.intrinsic_size),
        }
        .into(),
    )
}

fn attribution_mark(
    layer_id: &str,
    attribution: &str,
    plot_height: f32,
    attribution_index: usize,
) -> SceneMark {
    SceneTextMark {
        name: format!("geo-attribution-{layer_id}"),
        interactive: false,
        clip: false,
        len: 1,
        text: ScalarOrArray::new_scalar(attribution.to_string()),
        x: ScalarOrArray::new_scalar(4.0),
        y: ScalarOrArray::new_scalar(plot_height - 4.0 - 12.0 * attribution_index as f32),
        font_size: ScalarOrArray::new_scalar(10.0),
        font_style: ScalarOrArray::new_scalar(FontStyle::Italic),
        zindex: Some(100),
        ..Default::default()
    }
    .into()
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
