//! `Line<Geo>`: polylines with screen or great-circle geometry.
//!
//! Adapted from `avenger-chart-polar/src/marks/line.rs`. The
//! [`GeometrySpace`] option selects how segments between vertices are
//! built (scratch/geo decision 4; doc §6):
//!
//! - `Coordinate` (default): segments follow great circles, adaptively
//!   resampled through the projection pipeline and cut at the
//!   antimeridian. Requires positions authored via
//!   [`crate::marks::GeoPositionChannels::lon_lat`] so the raw spherical
//!   coordinates are available on the `lon`/`lat` channels; falls back to
//!   `Display` when they are not.
//! - `Display`: straight pixel-space segments between projected vertices.

use std::{collections::HashMap, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, DetailColumns, GeometrySpace,
    LegendRendererKind, LegendRendererSelection, Mark, MarkRuntimeContext, PointGeometry,
    RadiusExpression, RenderedMarkData, ScaleTypePreference, apply_opacity_to_color,
    coerce_bool_channel_with_renderer, coerce_color_channel_with_renderer,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel_with_renderer,
    coerce_stroke_cap_channel_values_with_renderer, coerce_stroke_dash_channel,
    coerce_stroke_join_channel_values_with_renderer, default_scale_type_for_data_type,
    impl_mark_trait_common, is_continuous_scale, stroke_rendering,
};
use avenger_chart_marks::{Line, line_channel_defaults};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use avenger_geo::sinks::PolylineSink;
use avenger_geo::stream::GeoStream;
use avenger_scenegraph::marks::{line::SceneLineMark, mark::SceneMark};
use datafusion::{
    arrow::{array::RecordBatch, datatypes::DataType as ArrowDataType},
    common::ScalarValue,
    logical_expr::Expr,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::Geo;
use crate::view::GeoCoordMeasurement;

#[async_trait::async_trait]
impl Mark<Geo> for Line<Geo> {
    impl_mark_trait_common!(Line);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        if !self.mark_effects().is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "Line<Geo> adjustments are not implemented yet".to_string(),
            ));
        }
        Ok(Arc::new(CompiledGeoLine {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledGeoLine {
    pub(crate) state: CompiledMarkState,
}

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
struct LineRenderPartitionKey {
    details: Vec<ScalarValue>,
    stroke: String,
    stroke_width_bits: u32,
    stroke_dash: String,
    opacity_bits: u32,
    stroke_cap: String,
    stroke_join: String,
}

impl CompiledMarkCore for CompiledGeoLine {
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
        "line"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        let optional = |name: &'static str| ChannelDescriptor {
            name,
            required: false,
            default_value: None,
            allow_column_ref: true,
        };
        vec![
            optional("x"),
            optional("y"),
            optional("lon"),
            optional("lat"),
            optional("stroke"),
            optional("stroke_width"),
            optional("stroke_dash"),
            optional("opacity"),
            optional("stroke_cap"),
            optional("stroke_join"),
            optional("defined"),
            optional("order"),
        ]
    }

    fn supports_order(&self) -> bool {
        true
    }

    fn details_partition_continuous_geometry(&self) -> bool {
        true
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        line_channel_defaults(channel)
    }

    fn radius_expression(
        &self,
        _dimension: &str,
        _resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        None
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &ArrowDataType,
    ) -> Option<ScaleTypePreference> {
        match (channel, data_type) {
            (
                "x" | "y",
                ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View,
            ) => Some(ScaleTypePreference::Point),
            _ => default_scale_type_for_data_type(data_type),
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        let is_continuous = is_continuous_scale(scale.scale_impl.as_ref());
        match channel {
            "stroke" if is_continuous => Some(LegendRendererSelection::BuiltIn(
                LegendRendererKind::Colorbar,
            )),
            "stroke" | "stroke_width" | "stroke_dash" | "opacity" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line))
            }
            "x" | "y" | "lon" | "lat" | "defined" | "order" | "stroke_cap" | "stroke_join" => None,
            _ => Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line)),
        }
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledGeoLine {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        self.render_mark_data(data, scalars, context, coord)
            .await
            .map(|rendered| rendered.marks)
    }

    async fn render_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        let data = data.ok_or_else(|| {
            AvengerChartError::InternalError(
                "Line mark requires array data for positions".to_string(),
            )
        })?;
        let mark_context = context.core_view();
        let len = data.num_rows();

        let visual = self.coerce_line_visual_channels(Some(data), scalars, &mark_context, len)?;
        let detail_columns = DetailColumns::from_mark_data(self, data)?;
        let stroke_values = visual.stroke.as_vec(len, None);
        let stroke_strings = stroke_rendering::color_channel_strings(&visual.stroke, len);
        let stroke_width_values = visual.stroke_width.as_vec(len, None);
        let opacity_values = visual.opacity.as_vec(len, None);
        let defined_values = visual.defined.as_vec(len, None);

        let mut partition_groups: IndexMap<LineRenderPartitionKey, Vec<usize>> = IndexMap::new();
        for i in 0..len {
            let key = LineRenderPartitionKey {
                details: detail_columns.key_for_row(i)?,
                stroke: stroke_strings[i].clone(),
                stroke_width_bits: stroke_width_values[i].to_bits(),
                stroke_dash: visual.stroke_dash_strings[i].clone(),
                opacity_bits: opacity_values[i].clamp(0.0, 1.0).to_bits(),
                stroke_cap: visual.stroke_cap_strings[i].clone(),
                stroke_join: visual.stroke_join_strings[i].clone(),
            };
            partition_groups.entry(key).or_default().push(i);
        }

        // Coordinate-space (great-circle) geometry needs raw spherical
        // coordinates and the realized view; fall back to display geometry
        // when either is unavailable.
        let has_spherical =
            data.column_by_name("lon").is_some() && data.column_by_name("lat").is_some();
        let measurement = GeoCoordMeasurement::downcast(context.coord_measurement());
        let requested_space = self.state.geometry_space_or(GeometrySpace::Coordinate);
        let blending = measurement.is_some_and(|m| m.blend_t() > 0.0);
        let geometry_space = match requested_space {
            GeometrySpace::Coordinate if has_spherical && measurement.is_some() => {
                GeometrySpace::Coordinate
            }
            // Display geometry under an active blend must still re-project
            // vertices (authored-unit positions are only valid at t = 0);
            // treat it as Coordinate-space with straight-segment semantics
            // approximated by the resampler-free projector point path.
            GeometrySpace::Display if blending && has_spherical && measurement.is_some() => {
                GeometrySpace::Display
            }
            _ => GeometrySpace::Display,
        };
        let blend_projector = (blending && has_spherical)
            .then(|| measurement.expect("measurement checked").view_projector());

        let mut scene_marks = Vec::new();
        let mut source_row_indices = Vec::new();

        match geometry_space {
            GeometrySpace::Coordinate => {
                let measurement = measurement.expect("checked above");
                let projector = measurement.view_projector();
                let lon = coerce_numeric_channel_with_renderer(
                    self,
                    Some(data),
                    scalars,
                    "lon",
                    &mark_context,
                    0.0,
                )?;
                let lat = coerce_numeric_channel_with_renderer(
                    self,
                    Some(data),
                    scalars,
                    "lat",
                    &mark_context,
                    0.0,
                )?;
                let lon_values = lon.as_vec(len, None);
                let lat_values = lat.as_vec(len, None);

                for (partition_key, indices) in partition_groups {
                    if indices.is_empty() {
                        continue;
                    }
                    let geometry = sample_great_circle_line(
                        &lon_values,
                        &lat_values,
                        &defined_values,
                        &indices,
                        &projector,
                    );
                    if geometry.len == 0 {
                        continue;
                    }
                    scene_marks.push(SceneMark::Line(self.build_line_mark(
                        &partition_key,
                        &stroke_values,
                        indices[0],
                        geometry,
                    )?));
                    source_row_indices.push(indices);
                }
            }
            GeometrySpace::Display => {
                let x = coerce_numeric_channel_with_renderer(
                    self,
                    Some(data),
                    scalars,
                    "x",
                    &mark_context,
                    0.0,
                )?;
                let y = coerce_numeric_channel_with_renderer(
                    self,
                    Some(data),
                    scalars,
                    "y",
                    &mark_context,
                    0.0,
                )?;
                let (x_values, y_values) = if let Some(projector) = &blend_projector {
                    let lon = coerce_numeric_channel_with_renderer(
                        self,
                        Some(data),
                        scalars,
                        "lon",
                        &mark_context,
                        0.0,
                    )?;
                    let lat = coerce_numeric_channel_with_renderer(
                        self,
                        Some(data),
                        scalars,
                        "lat",
                        &mark_context,
                        0.0,
                    )?;
                    let lon_values = lon.as_vec(len, None);
                    let lat_values = lat.as_vec(len, None);
                    let mut xs = Vec::with_capacity(len);
                    let mut ys = Vec::with_capacity(len);
                    for i in 0..len {
                        match projector.project(f64::from(lon_values[i]), f64::from(lat_values[i]))
                        {
                            Some((px, py)) => {
                                xs.push(px as f32);
                                ys.push(py as f32);
                            }
                            None => {
                                xs.push(f32::NAN);
                                ys.push(f32::NAN);
                            }
                        }
                    }
                    (xs, ys)
                } else {
                    let geometry = project_display_arrays(
                        &x.as_vec(len, None),
                        &y.as_vec(len, None),
                        coord,
                        context.plot_width(),
                        context.plot_height(),
                    )?;
                    (geometry.x.as_vec(len, None), geometry.y.as_vec(len, None))
                };

                for (partition_key, indices) in partition_groups {
                    if indices.is_empty() {
                        continue;
                    }
                    let geometry = SampledLineGeometry {
                        len: indices.len(),
                        x: ScalarOrArray::new_array(indices.iter().map(|i| x_values[*i]).collect()),
                        y: ScalarOrArray::new_array(indices.iter().map(|i| y_values[*i]).collect()),
                        defined: ScalarOrArray::new_array(
                            indices.iter().map(|i| defined_values[*i]).collect(),
                        ),
                    };
                    scene_marks.push(SceneMark::Line(self.build_line_mark(
                        &partition_key,
                        &stroke_values,
                        indices[0],
                        geometry,
                    )?));
                    source_row_indices.push(indices);
                }
            }
        }

        Ok(RenderedMarkData::with_source_row_indices(
            scene_marks,
            source_row_indices,
        ))
    }
}

impl CompiledGeoLine {
    fn build_line_mark(
        &self,
        partition_key: &LineRenderPartitionKey,
        stroke_values: &[ColorOrGradient],
        first_index: usize,
        geometry: SampledLineGeometry,
    ) -> Result<SceneLineMark, AvengerChartError> {
        let stroke_color = apply_opacity_to_color(
            &stroke_values[first_index],
            f32::from_bits(partition_key.opacity_bits),
        );
        let stroke_dash = stroke_rendering::stroke_dash_from_name(&partition_key.stroke_dash)?;
        let stroke_cap = stroke_rendering::coerce_stroke_cap_strings(
            std::slice::from_ref(&partition_key.stroke_cap),
            "stroke_cap",
        )?
        .first()
        .copied()
        .unwrap_or(StrokeCap::Round);
        let stroke_join = stroke_rendering::coerce_stroke_join_strings(
            std::slice::from_ref(&partition_key.stroke_join),
            "stroke_join",
        )?
        .first()
        .copied()
        .unwrap_or(StrokeJoin::Miter);

        Ok(SceneLineMark {
            name: "line".to_string(),
            clip: true,
            len: geometry.len as u32,
            x: geometry.x,
            y: geometry.y,
            gradients: vec![],
            stroke: stroke_color,
            stroke_width: f32::from_bits(partition_key.stroke_width_bits),
            stroke_dash,
            stroke_cap,
            stroke_join,
            defined: geometry.defined,
            zindex: self.state.zindex,
            interactive: true,
        })
    }

    fn coerce_line_visual_channels(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &avenger_chart_core::MarkRenderContext<'_>,
        len: usize,
    ) -> Result<LineVisualChannels, AvengerChartError> {
        let stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke",
            context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let stroke_width = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke_width",
            context,
            2.0,
        )?;
        let opacity =
            coerce_opacity_channel_with_renderer(self, data, scalars, "opacity", context, 1.0)?;
        let defined =
            coerce_bool_channel_with_renderer(self, data, scalars, "defined", context, true)?;
        let stroke_dash = stroke_rendering::optional_stroke_dash(coerce_stroke_dash_channel(
            data,
            scalars,
            "stroke_dash",
        )?);
        let stroke_cap = coerce_stroke_cap_channel_values_with_renderer(
            self,
            data,
            scalars,
            "stroke_cap",
            context,
            StrokeCap::Round,
        )?;
        let stroke_join = coerce_stroke_join_channel_values_with_renderer(
            self,
            data,
            scalars,
            "stroke_join",
            context,
            StrokeJoin::Miter,
        )?;

        Ok(LineVisualChannels {
            stroke,
            stroke_width,
            stroke_dash_strings: stroke_rendering::stroke_dash_strings(&stroke_dash, len),
            stroke_cap_strings: stroke_rendering::stroke_cap_strings(&stroke_cap, len),
            stroke_join_strings: stroke_rendering::stroke_join_strings(&stroke_join, len),
            opacity,
            defined,
        })
    }
}

#[derive(Clone)]
struct LineVisualChannels {
    stroke: ScalarOrArray<ColorOrGradient>,
    stroke_width: ScalarOrArray<f32>,
    stroke_dash_strings: Vec<String>,
    stroke_cap_strings: Vec<String>,
    stroke_join_strings: Vec<String>,
    opacity: ScalarOrArray<f32>,
    defined: ScalarOrArray<bool>,
}

struct SampledLineGeometry {
    len: usize,
    x: ScalarOrArray<f32>,
    y: ScalarOrArray<f32>,
    defined: ScalarOrArray<bool>,
}

/// Stream each defined run of vertices through the projection pipeline as a
/// spherical line: segments become great-circle arcs, adaptively resampled
/// and cut at the antimeridian (breaks arrive as `defined = false` samples
/// from the [`PolylineSink`]).
fn sample_great_circle_line(
    lon_values: &[f32],
    lat_values: &[f32],
    defined_values: &[bool],
    indices: &[usize],
    projector: &avenger_geo::projector::Projector,
) -> SampledLineGeometry {
    let mut sink = PolylineSink::default();
    let mut run: Vec<(f64, f64)> = Vec::new();

    // Feed each run through the full pipeline: rotation, antimeridian cut,
    // adaptive resample, plot-rect clip. The PolylineSink inserts
    // `defined = false` breaks between the emitted sub-lines.
    struct RunLine<'a>(&'a [(f64, f64)]);
    impl avenger_geo::streamable::Streamable for RunLine<'_> {
        fn stream(&self, sink: &mut dyn GeoStream) {
            sink.line_start();
            for (lon, lat) in self.0 {
                sink.point(*lon, *lat, None);
            }
            sink.line_end();
        }
    }
    let flush = |run: &mut Vec<(f64, f64)>, sink: &mut PolylineSink| {
        if run.len() >= 2 {
            projector.stream(&RunLine(run.as_slice()), sink);
        }
        run.clear();
    };

    for index in indices.iter().copied() {
        let defined = defined_values.get(index).copied().unwrap_or(false);
        if defined {
            run.push((f64::from(lon_values[index]), f64::from(lat_values[index])));
        } else {
            flush(&mut run, &mut sink);
        }
    }
    flush(&mut run, &mut sink);

    SampledLineGeometry {
        len: sink.x.len(),
        x: ScalarOrArray::new_array(sink.x),
        y: ScalarOrArray::new_array(sink.y),
        defined: ScalarOrArray::new_array(sink.defined),
    }
}

fn project_display_arrays(
    x_values: &[f32],
    y_values: &[f32],
    coord: &dyn CoordinateSystemTransformCore,
    plot_width: f32,
    plot_height: f32,
) -> Result<PointGeometry, AvengerChartError> {
    let mut position_channels = HashMap::new();
    position_channels.insert("x", ScalarOrArray::new_array(x_values.to_vec()));
    position_channels.insert("y", ScalarOrArray::new_array(y_values.to_vec()));

    let geometry = coord.transform(&position_channels, None, plot_width, plot_height)?;
    geometry
        .as_any()
        .downcast_ref::<PointGeometry>()
        .cloned()
        .ok_or_else(|| {
            AvengerChartError::CoordinateSystemError(
                "Failed to downcast transformed geo line points to PointGeometry".to_string(),
            )
        })
}
