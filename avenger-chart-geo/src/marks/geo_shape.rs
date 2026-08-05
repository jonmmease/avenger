//! `GeoShape<Geo>`: one mark instance per row, geometry from an ISO WKB
//! column (doc §7). Each feature streams zero-copy through the projection
//! pipeline (rotation → antimeridian cut → adaptive resample → plot clip)
//! into a `lyon` path; fill/stroke/tooltip channels encode ordinary
//! columns, so choropleths are joins, not a chart type.
//!
//! Fit participation: `.geometry(&geo, col)` also binds hidden
//! `x`/`x2`/`y`/`y2` channels to `least`/`greatest` over eight projected
//! samples of the feature's lon/lat bbox (the `bbox_*` sibling columns the
//! ingest writes), so the coordinate's ordinary channel-extent machinery
//! drives fit-to-data with no custom domain plumbing.

use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, ChannelValue, ColorChannelConfig, CompiledDataContext,
    CompiledMark, CompiledMarkCore, CompiledMarkState, CoordinateSystemTransformCore,
    LegendRendererKind, LegendRendererSelection, Mark, MarkRuntimeContext, MarkState,
    OpacityChannelConfig, PrimitiveMarkEffects, RenderedMarkData, ScaleTypePreference,
    StrokeWidthChannelConfig, apply_opacity_to_color, coerce_color_channel_with_renderer,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel_with_renderer,
    default_scale_type_for_data_type, define_common_mark_channels,
    impl_mark_base_with_extra_fields, is_continuous_scale,
};
use avenger_common::{
    types::{PathTransform, StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use avenger_geo::sinks::LyonPathSink;
use avenger_scenegraph::marks::{mark::SceneMark, path::ScenePathMark};
use datafusion::{
    arrow::{
        array::{Array, BinaryArray, BinaryViewArray, LargeBinaryArray, RecordBatch},
        datatypes::DataType as ArrowDataType,
    },
    common::ScalarValue,
    functions::expr_fn::{greatest, least},
    logical_expr::{Expr, col},
};
use serde::{Deserialize, Serialize};

use crate::Geo;
use crate::view::GeoCoordMeasurement;

pub struct GeoShape<C> {
    pub(crate) state: MarkState,
    pub(crate) effects: PrimitiveMarkEffects,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base_with_extra_fields!(GeoShape {
    effects: PrimitiveMarkEffects::default(),
});

define_common_mark_channels! {
    GeoShape {
        fill: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        stroke: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            allow_column: false,
            with_config: StrokeWidthChannelConfig,
        },
        opacity: {
            allow_column: true,
            with_config: OpacityChannelConfig,
        },
    }
}

impl<C> GeoShape<C> {
    #[doc(hidden)]
    pub fn mark_effects(&self) -> &PrimitiveMarkEffects {
        &self.effects
    }
}

/// Default bbox side-column names written by
/// [`avenger_geo::ingest`]-based loaders.
pub const DEFAULT_BBOX_COLUMNS: [&str; 4] = ["bbox_xmin", "bbox_ymin", "bbox_xmax", "bbox_ymax"];

impl GeoShape<Geo> {
    /// Bind the WKB geometry column and register the feature bboxes with
    /// the view-fit machinery (assumes the ingest's `bbox_*` sibling
    /// columns; use [`GeoShape::geometry_with_bbox_columns`] to override).
    pub fn geometry(self, geo: &Geo, value: impl Into<Expr>) -> Self {
        self.geometry_with_bbox_columns(geo, value, DEFAULT_BBOX_COLUMNS)
    }

    /// Bind geometry with explicit bbox column names
    /// `[xmin, ymin, xmax, ymax]` (lon/lat degrees).
    pub fn geometry_with_bbox_columns(
        self,
        geo: &Geo,
        value: impl Into<Expr>,
        bbox: [&str; 4],
    ) -> Self {
        let projection = geo.projection();
        let [xmin, ymin, xmax, ymax] = bbox.map(col);
        let xmid = (xmin.clone() + xmax.clone()) / datafusion::logical_expr::lit(2.0);
        let ymid = (ymin.clone() + ymax.clone()) / datafusion::logical_expr::lit(2.0);
        // Eight boundary samples of the lon/lat bbox: the projected image
        // of a curved-edge bbox is bounded well enough for fit.
        let samples = [
            (xmin.clone(), ymin.clone()),
            (xmin.clone(), ymax.clone()),
            (xmax.clone(), ymin.clone()),
            (xmax.clone(), ymax.clone()),
            (xmid.clone(), ymin.clone()),
            (xmid, ymax.clone()),
            (xmin, ymid.clone()),
            (xmax, ymid),
        ];
        let mut gx = Vec::with_capacity(8);
        let mut gy = Vec::with_capacity(8);
        for (lon, lat) in samples {
            let (x, y) = crate::expr::geo_position_exprs(&projection, lon, lat);
            gx.push(x);
            gy.push(y);
        }
        self.with_channel_value("geometry", ChannelValue::from(value.into()).no_scale())
            .with_channel_value("x", ChannelValue::from(least(gx.clone())))
            .with_channel_value("x2", ChannelValue::from(greatest(gx)).with_scale_name("x"))
            .with_channel_value("y", ChannelValue::from(least(gy.clone())))
            .with_channel_value("y2", ChannelValue::from(greatest(gy)).with_scale_name("y"))
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<Geo> for GeoShape<Geo> {
    avenger_chart_core::impl_mark_trait_common!(GeoShape);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        if !self.mark_effects().is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "GeoShape<Geo> adjustments are not implemented yet".to_string(),
            ));
        }
        Ok(Arc::new(CompiledGeoShape {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledGeoShape {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledGeoShape {
    avenger_chart_core::impl_mark_with_data_context!();

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
        "geoshape"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        let optional = |name: &'static str| ChannelDescriptor {
            name,
            required: false,
            default_value: None,
            allow_column_ref: true,
        };
        vec![
            optional("geometry"),
            optional("x"),
            optional("y"),
            optional("x2"),
            optional("y2"),
            optional("fill"),
            optional("stroke"),
            optional("stroke_width"),
            optional("opacity"),
        ]
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        match channel {
            "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))),
            "stroke" => Some(ScalarValue::Utf8(Some("transparent".to_string()))),
            "stroke_width" => Some(ScalarValue::Float32(Some(0.0))),
            "opacity" => Some(ScalarValue::Float32(Some(1.0))),
            _ => None,
        }
    }

    fn wants_full_data_batch(&self) -> bool {
        true
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &ArrowDataType,
    ) -> Option<ScaleTypePreference> {
        match (channel, data_type) {
            (
                "fill" | "stroke",
                ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View,
            ) => Some(ScaleTypePreference::Ordinal),
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
            "fill" | "stroke" if is_continuous => Some(LegendRendererSelection::BuiltIn(
                LegendRendererKind::Colorbar,
            )),
            "fill" | "stroke" | "opacity" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Rect))
            }
            _ => None,
        }
    }
}

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl CompiledMark for CompiledGeoShape {
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
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        let data = data.ok_or_else(|| {
            AvengerChartError::InternalError(
                "GeoShape requires array data for the geometry column".to_string(),
            )
        })?;
        let mark_context = context.core_view();
        let len = data.num_rows();

        let measurement =
            GeoCoordMeasurement::downcast(context.coord_measurement()).ok_or_else(|| {
                AvengerChartError::CoordinateSystemError(
                    "GeoShape requires the Geo coordinate system".to_string(),
                )
            })?;
        let projector = measurement.view_projector();

        let geometry_column = data.column_by_name("geometry").ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "GeoShape geometry channel is required (use .geometry(&geo, col))".to_string(),
            )
        })?;

        let fill = coerce_color_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "fill",
            &mark_context,
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
        )?;
        let stroke = coerce_color_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "stroke",
            &mark_context,
            [0.0, 0.0, 0.0, 0.0],
        )?;
        let opacity = coerce_opacity_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "opacity",
            &mark_context,
            1.0,
        )?;
        let stroke_width = coerce_numeric_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "stroke_width",
            &mark_context,
            0.0,
        )?;
        let fill_values = fill.as_vec(len, None);
        let stroke_values = stroke.as_vec(len, None);
        let opacity_values = opacity.as_vec(len, None);
        let stroke_width_values = stroke_width.as_vec(len, None);

        let mut paths = Vec::new();
        let mut fills = Vec::new();
        let mut strokes = Vec::new();
        let mut indices = Vec::new();
        for row in 0..len {
            let Some(wkb_bytes) = binary_value(geometry_column.as_ref(), row)? else {
                continue;
            };
            let mut sink = LyonPathSink::fill();
            avenger_geo::ingest::stream_wkb_through(&projector, wkb_bytes, &mut sink).map_err(
                |err| {
                    AvengerChartError::InvalidArgument(format!(
                        "GeoShape row {row}: invalid WKB geometry: {err}"
                    ))
                },
            )?;
            if !sink.has_content() {
                continue;
            }
            paths.push(sink.finish());
            let alpha = opacity_values[row].clamp(0.0, 1.0);
            fills.push(apply_opacity_to_color(&fill_values[row], alpha));
            strokes.push(apply_opacity_to_color(&stroke_values[row], alpha));
            indices.push(row);
        }

        if paths.is_empty() {
            return Ok(RenderedMarkData::new(Vec::new()));
        }

        let stroke_width = stroke_width_values.first().copied().unwrap_or(0.0);
        let mark = ScenePathMark {
            name: "geoshape".to_string(),
            interactive: true,
            clip: true,
            len: paths.len() as u32,
            gradients: Vec::new(),
            stroke_cap: StrokeCap::Round,
            stroke_join: StrokeJoin::Round,
            stroke_width: (stroke_width > 0.0).then_some(stroke_width),
            path: ScalarOrArray::new_array(paths),
            fill: ScalarOrArray::new_array(fills),
            fill_pattern: ScalarOrArray::new_scalar(None),
            stroke: ScalarOrArray::new_array(strokes),
            transform: ScalarOrArray::new_scalar(PathTransform::identity()),
            indices: None,
            zindex: self.state.zindex,
        };

        Ok(RenderedMarkData::with_source_row_indices(
            vec![SceneMark::Path(mark)],
            vec![indices],
        ))
    }
}

fn binary_value(array: &dyn Array, row: usize) -> Result<Option<&[u8]>, AvengerChartError> {
    if array.is_null(row) {
        return Ok(None);
    }
    if let Some(binary) = array.as_any().downcast_ref::<BinaryArray>() {
        Ok(Some(binary.value(row)))
    } else if let Some(binary) = array.as_any().downcast_ref::<LargeBinaryArray>() {
        Ok(Some(binary.value(row)))
    } else if let Some(binary) = array.as_any().downcast_ref::<BinaryViewArray>() {
        Ok(Some(binary.value(row)))
    } else {
        Err(AvengerChartError::InvalidArgument(format!(
            "GeoShape geometry column must be Binary, got {:?}",
            array.data_type()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_value_accepts_binary_view_arrays() {
        let array = BinaryViewArray::from(vec![Some(b"wkb".as_ref()), None]);
        assert_eq!(binary_value(&array, 0).unwrap(), Some(b"wkb".as_ref()));
        assert_eq!(binary_value(&array, 1).unwrap(), None);
    }
}
