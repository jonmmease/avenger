use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, ChannelValue, CompiledDataContext, CompiledMark,
    CompiledMarkCore, CompiledMarkState, CoordinateSystemTransformCore, LegendRendererKind,
    LegendRendererSelection, Mark, MarkRenderContext, MarkRuntimeContext, MarkScaleDomainSource,
    PointGeometry, RenderedMarkData, ResolvedDomain, ScaleRange, ScaleTypePreference, Theme,
    coerce_opacity_channel_with_renderer, default_scale_type_for_data_type, impl_mark_trait_common,
    is_continuous_scale,
};
use avenger_chart_marks::{
    UNIFORM_RASTER_2D_FILL_CHANNEL, UNIFORM_RASTER_2D_RASTER_CHANNEL, UniformRaster2D,
    UniformRaster2DFields, UniformRaster2DOptions, uniform_raster_2d_channel_defaults,
};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{ImageAlign, ImageBaseline},
    value::ScalarOrArray,
};
use avenger_image::RgbaImage;
use avenger_scales::scales::{ConfiguredScale, ScaleImpl, coerce::Coercer};
use avenger_scenegraph::marks::{
    image::{SceneImageMark, SceneImageSource},
    mark::SceneMark,
};
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, Float64Array, StructArray},
        datatypes::{DataType, Float32Type, Float64Type},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    prelude::SessionContext,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tracing::{debug, trace};

use crate::Cartesian;

#[async_trait::async_trait]
impl Mark<Cartesian> for UniformRaster2D<Cartesian> {
    impl_mark_trait_common!(UniformRaster2D);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianUniformRaster2D {
            state: compiled_state,
            options: self.raster_options().clone(),
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianUniformRaster2D {
    pub(crate) state: CompiledMarkState,
    pub(crate) options: UniformRaster2DOptions,
}

impl CompiledMarkCore for CompiledCartesianUniformRaster2D {
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
        "uniform_raster_2d"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: UNIFORM_RASTER_2D_RASTER_CHANNEL,
                required: true,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: UNIFORM_RASTER_2D_FILL_CHANNEL,
                required: true,
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
        uniform_raster_2d_channel_defaults(channel)
    }

    fn scale_domain_sources(
        &self,
        domain_dataframe: Option<&DataFrame>,
        ctx: &SessionContext,
    ) -> Result<Vec<MarkScaleDomainSource>, AvengerChartError> {
        if self.state.exclude_from_scale_domains {
            return Ok(Vec::new());
        }

        let Some(dataframe) = domain_dataframe.cloned() else {
            return Ok(Vec::new());
        };
        let Some(raster_channel) = self
            .data_context()
            .channel(UNIFORM_RASTER_2D_RASTER_CHANNEL)
        else {
            return Ok(Vec::new());
        };
        let Some(raster_expr) = raster_channel.expr(ctx) else {
            return Ok(Vec::new());
        };

        let fields = UniformRaster2DFields::new(raster_expr);
        let (x_start, x_stop, y_start, y_stop) = if self.options.transpose {
            (
                fields.rows_start(),
                fields.rows_stop(),
                fields.columns_start(),
                fields.columns_stop(),
            )
        } else {
            (
                fields.columns_start(),
                fields.columns_stop(),
                fields.rows_start(),
                fields.rows_stop(),
            )
        };

        Ok(vec![
            MarkScaleDomainSource {
                channel: "x".to_string(),
                channel_value: self.options.x_channel.clone(),
                dataframe: dataframe.clone(),
                exprs: vec![x_start, x_stop],
            },
            MarkScaleDomainSource {
                channel: "y".to_string(),
                channel_value: self.options.y_channel.clone(),
                dataframe,
                exprs: vec![y_start, y_stop],
            },
        ])
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        match channel {
            UNIFORM_RASTER_2D_RASTER_CHANNEL => None,
            _ => default_scale_type_for_data_type(data_type),
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        let is_continuous = is_continuous_scale(scale.scale_impl.as_ref());
        match channel {
            UNIFORM_RASTER_2D_FILL_CHANNEL if is_continuous => Some(
                LegendRendererSelection::BuiltIn(LegendRendererKind::Colorbar),
            ),
            UNIFORM_RASTER_2D_FILL_CHANNEL | "opacity" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Rect))
            }
            _ => None,
        }
    }

    fn default_channel_range(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        domain: &ResolvedDomain,
        _data_type: &DataType,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
    ) -> Option<ScaleRange> {
        let range_kind = scale_impl.range_kind();
        let cardinality = match domain {
            ResolvedDomain::Discrete(count) => Some(*count),
            ResolvedDomain::Interval => None,
        };

        theme
            .get_range_for_channel(self.mark_type(), channel, range_kind, cardinality, params)
            .or_else(|| {
                theme.get_range_for_channel("rect", channel, range_kind, cardinality, params)
            })
    }
}

impl CompiledCartesianUniformRaster2D {
    fn render_uniform_raster_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        let mark_context = context.core_view();
        let len = data.map_or(1, RecordBatch::num_rows);
        let raster_array = channel_array(data, scalars, UNIFORM_RASTER_2D_RASTER_CHANNEL)?;
        let fill_array = channel_array(data, scalars, UNIFORM_RASTER_2D_FILL_CHANNEL)?;
        let opacity = coerce_opacity_channel_with_renderer(
            self,
            data,
            scalars,
            "opacity",
            &mark_context,
            1.0,
        )?;
        let opacity_values = opacity.as_vec(len, None);
        let null_color = option_color(
            &self.options.null_color,
            &mark_context,
            "null_color",
            [0.0, 0.0, 0.0, 0.0],
        )?;
        let non_finite_color = option_color(
            &self.options.non_finite_color,
            &mark_context,
            "non_finite_color",
            [0.0, 0.0, 0.0, 0.0],
        )?;

        debug!(
            mark_type = self.mark_type(),
            rows = len,
            transpose = self.options.transpose,
            smooth = self.options.smooth,
            "rendering uniform raster rows"
        );

        let mut marks = Vec::with_capacity(len);
        let mut source_row_indices = Vec::with_capacity(len);
        for row in 0..len {
            let raster_row =
                row_for_channel(raster_array, row, len, UNIFORM_RASTER_2D_RASTER_CHANNEL)?;
            let fill_row = row_for_channel(fill_array, row, len, UNIFORM_RASTER_2D_FILL_CHANNEL)?;
            let raster = extract_uniform_raster_row(raster_array, raster_row)?;
            let fill_values =
                list_row_values(fill_array, fill_row, UNIFORM_RASTER_2D_FILL_CHANNEL)?;
            let cell_count = raster.cell_count()?;
            if fill_values.len() != cell_count {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "UniformRaster2D fill row {row} has {} cells, but raster geometry requires {cell_count}",
                    fill_values.len()
                )));
            }

            let colors = coerce_cell_colors(&fill_values)?;
            let (x_start, x_stop, _x_count, y_start, y_stop, _y_count) = if self.options.transpose {
                (
                    raster.rows_start,
                    raster.rows_stop,
                    raster.rows_count,
                    raster.columns_start,
                    raster.columns_stop,
                    raster.columns_count,
                )
            } else {
                (
                    raster.columns_start,
                    raster.columns_stop,
                    raster.columns_count,
                    raster.rows_start,
                    raster.rows_stop,
                    raster.rows_count,
                )
            };

            let (scaled_x0, scaled_x1) =
                scale_extent(context, "x", &self.options.x_channel, x_start, x_stop)?;
            let (scaled_y0, scaled_y1) =
                scale_extent(context, "y", &self.options.y_channel, y_start, y_stop)?;
            let (scaled_x0, scaled_y0, scaled_x1, scaled_y1) =
                transform_extent(coord, context, scaled_x0, scaled_y0, scaled_x1, scaled_y1)?;

            let flip_x = scaled_x1 < scaled_x0;
            let flip_y = scaled_y1 < scaled_y0;
            let x = scaled_x0.min(scaled_x1);
            let y = scaled_y0.min(scaled_y1);
            let width = (scaled_x1 - scaled_x0).abs();
            let height = (scaled_y1 - scaled_y0).abs();
            let image = build_rgba_image(
                &raster,
                &colors,
                null_color,
                non_finite_color,
                opacity_values[row],
                self.options.transpose,
                flip_x,
                flip_y,
            )?;

            trace!(
                row,
                columns_count = raster.columns_count,
                rows_count = raster.rows_count,
                image_width = image.width,
                image_height = image.height,
                x,
                y,
                width,
                height,
                opacity = opacity_values[row],
                flip_x,
                flip_y,
                "uniform raster row converted to SceneImageMark"
            );

            marks.push(
                SceneImageMark {
                    name: "uniform_raster_2d".to_string(),
                    interactive: true,
                    clip: true,
                    len: 1,
                    aspect: false,
                    smooth: self.options.smooth,
                    image: ScalarOrArray::new_scalar(SceneImageSource::Inline(image)),
                    x: ScalarOrArray::new_scalar(x),
                    y: ScalarOrArray::new_scalar(y),
                    width: ScalarOrArray::new_scalar(width),
                    height: ScalarOrArray::new_scalar(height),
                    align: ScalarOrArray::new_scalar(ImageAlign::Left),
                    baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
                    unavailable_policy: Default::default(),
                    indices: None,
                    zindex: self.state.zindex,
                }
                .into(),
            );
            source_row_indices.push(vec![row]);
        }

        Ok(RenderedMarkData::with_source_row_indices(
            marks,
            source_row_indices,
        ))
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianUniformRaster2D {
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
        self.render_uniform_raster_mark_data(data, scalars, context, coord)
    }
}

#[derive(Debug)]
struct UniformRasterRow {
    columns_start: f64,
    columns_stop: f64,
    columns_count: u32,
    rows_start: f64,
    rows_stop: f64,
    rows_count: u32,
    values: ArrayRef,
}

impl UniformRasterRow {
    fn cell_count(&self) -> Result<usize, AvengerChartError> {
        let count = self
            .columns_count
            .checked_mul(self.rows_count)
            .ok_or_else(|| {
                AvengerChartError::InvalidArgument(
                    "UniformRaster2D rows.count * columns.count overflowed".to_string(),
                )
            })?;
        Ok(count as usize)
    }
}

fn channel_array<'a>(
    data: Option<&'a RecordBatch>,
    scalars: &'a RecordBatch,
    channel: &str,
) -> Result<&'a ArrayRef, AvengerChartError> {
    data.and_then(|batch| batch.column_by_name(channel))
        .or_else(|| scalars.column_by_name(channel))
        .ok_or_else(|| AvengerChartError::MissingChannelError(channel.to_string()))
}

fn row_for_channel(
    array: &ArrayRef,
    row: usize,
    mark_len: usize,
    channel: &str,
) -> Result<usize, AvengerChartError> {
    if array.len() == mark_len {
        Ok(row)
    } else if array.len() == 1 {
        Ok(0)
    } else {
        Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D channel '{channel}' has length {}, expected 1 or {mark_len}",
            array.len()
        )))
    }
}

fn extract_uniform_raster_row(
    array: &ArrayRef,
    row: usize,
) -> Result<UniformRasterRow, AvengerChartError> {
    let raster = as_struct_array(array, "raster")?;
    if raster.is_null(row) {
        return Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D raster row {row} is null"
        )));
    }

    let geometry = struct_child_struct(raster, "geometry")?;
    let values = struct_child_struct(raster, "values")?;
    let kind = required_string(struct_child(geometry, "kind")?, row, "geometry.kind")?;
    if kind != "uniform" {
        return Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D requires geometry.kind = 'uniform', got '{kind}'"
        )));
    }

    if let Some(coordinate_space) = geometry.column_by_name("coordinate_space")
        && !coordinate_space.is_null(row)
    {
        return Err(AvengerChartError::InvalidArgument(
            "CRS-backed rasters require WebMercator or a later geo raster implementation"
                .to_string(),
        ));
    }

    let columns = struct_child_struct(geometry, "columns")?;
    let rows = struct_child_struct(geometry, "rows")?;
    reject_legacy_units(columns, "columns")?;
    reject_legacy_units(rows, "rows")?;
    validate_sampling(columns, row, "columns")?;
    validate_sampling(rows, row, "rows")?;

    let columns_count = required_u32(struct_child(columns, "count")?, row, "columns.count")?;
    let rows_count = required_u32(struct_child(rows, "count")?, row, "rows.count")?;
    if columns_count == 0 || rows_count == 0 {
        return Err(AvengerChartError::InvalidArgument(
            "UniformRaster2D rows.count and columns.count must be greater than zero".to_string(),
        ));
    }

    let values = list_row_values(struct_child(values, "data")?, row, "values.data")?;
    let raster = UniformRasterRow {
        columns_start: required_f64(struct_child(columns, "start")?, row, "columns.start")?,
        columns_stop: required_f64(struct_child(columns, "stop")?, row, "columns.stop")?,
        columns_count,
        rows_start: required_f64(struct_child(rows, "start")?, row, "rows.start")?,
        rows_stop: required_f64(struct_child(rows, "stop")?, row, "rows.stop")?,
        rows_count,
        values,
    };
    let cell_count = raster.cell_count()?;
    if raster.values.len() != cell_count {
        return Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D values.data row {row} has {} cells, but rows.count * columns.count requires {cell_count}",
            raster.values.len()
        )));
    }
    Ok(raster)
}

fn as_struct_array<'a>(
    array: &'a ArrayRef,
    label: &str,
) -> Result<&'a StructArray, AvengerChartError> {
    array.as_any().downcast_ref::<StructArray>().ok_or_else(|| {
        AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D expected {label} to be a StructArray, got {:?}",
            array.data_type()
        ))
    })
}

fn struct_child<'a>(array: &'a StructArray, name: &str) -> Result<&'a ArrayRef, AvengerChartError> {
    array.column_by_name(name).ok_or_else(|| {
        AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D raster struct is missing required field '{name}'"
        ))
    })
}

fn struct_child_struct<'a>(
    array: &'a StructArray,
    name: &str,
) -> Result<&'a StructArray, AvengerChartError> {
    as_struct_array(struct_child(array, name)?, name)
}

fn reject_legacy_units(axis: &StructArray, label: &str) -> Result<(), AvengerChartError> {
    if axis.column_by_name("unit").is_some() || axis.column_by_name("units").is_some() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D geometry.{label} unit fields are not supported in the v1 raster schema"
        )));
    }
    Ok(())
}

fn validate_sampling(axis: &StructArray, row: usize, label: &str) -> Result<(), AvengerChartError> {
    let Some(sampling_array) = axis.column_by_name("sampling") else {
        return Ok(());
    };
    let Some(sampling) = optional_string(sampling_array, row, &format!("{label}.sampling"))? else {
        return Ok(());
    };
    if sampling == "linear" {
        Ok(())
    } else {
        Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D Phase 1 supports only linear sampling, got geometry.{label}.sampling = '{sampling}'"
        )))
    }
}

fn list_row_values(
    array: &ArrayRef,
    row: usize,
    label: &str,
) -> Result<ArrayRef, AvengerChartError> {
    if array.is_null(row) {
        return Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D {label} row {row} is null"
        )));
    }
    match array.data_type() {
        DataType::List(_) => Ok(array.as_list::<i32>().value(row)),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D expected {label} to be ListArray, got {other:?}"
        ))),
    }
}

fn required_f64(array: &ArrayRef, row: usize, label: &str) -> Result<f64, AvengerChartError> {
    if array.is_null(row) {
        return Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D required field {label} is null"
        )));
    }
    let casted = datafusion::arrow::compute::cast(array, &DataType::Float64)?;
    Ok(casted.as_primitive::<Float64Type>().value(row))
}

fn required_u32(array: &ArrayRef, row: usize, label: &str) -> Result<u32, AvengerChartError> {
    if array.is_null(row) {
        return Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D required field {label} is null"
        )));
    }
    let value = ScalarValue::try_from_array(array.as_ref(), row)?;
    match value {
        ScalarValue::UInt32(Some(value)) => Ok(value),
        ScalarValue::UInt64(Some(value)) => u32::try_from(value).map_err(|_| {
            AvengerChartError::InvalidArgument(format!(
                "UniformRaster2D field {label} value {value} does not fit in UInt32"
            ))
        }),
        ScalarValue::Int32(Some(value)) if value >= 0 => Ok(value as u32),
        ScalarValue::Int64(Some(value)) if value >= 0 => u32::try_from(value).map_err(|_| {
            AvengerChartError::InvalidArgument(format!(
                "UniformRaster2D field {label} value {value} does not fit in UInt32"
            ))
        }),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D field {label} must be UInt32-compatible, got {other:?}"
        ))),
    }
}

fn required_string(array: &ArrayRef, row: usize, label: &str) -> Result<String, AvengerChartError> {
    optional_string(array, row, label)?.ok_or_else(|| {
        AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D required field {label} is null"
        ))
    })
}

fn optional_string(
    array: &ArrayRef,
    row: usize,
    label: &str,
) -> Result<Option<String>, AvengerChartError> {
    if array.is_null(row) {
        return Ok(None);
    }
    match array.data_type() {
        DataType::Utf8 => Ok(Some(array.as_string::<i32>().value(row).to_string())),
        DataType::LargeUtf8 => Ok(Some(array.as_string::<i64>().value(row).to_string())),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D field {label} must be Utf8, got {other:?}"
        ))),
    }
}

fn option_color(
    value: &ChannelValue,
    context: &MarkRenderContext<'_>,
    label: &str,
    fallback: [f32; 4],
) -> Result<[f32; 4], AvengerChartError> {
    let Some(expr) = value.expr(context.session_context()) else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D {label} must be a scalar color expression"
        )));
    };
    if !expr.column_refs().is_empty() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D {label} cannot reference dataframe columns in v1"
        )));
    }
    let scalar = match expr {
        datafusion::logical_expr::Expr::Literal(scalar, _) => scalar,
        other => {
            return Err(AvengerChartError::InvalidArgument(format!(
                "UniformRaster2D {label} must currently be a literal color, got {other:?}"
            )));
        }
    };
    let array = scalar.to_array()?;
    let color = Coercer::default()
        .to_color(&array, Some(ColorOrGradient::Color(fallback)))
        .map_err(AvengerChartError::ScaleError)?
        .first()
        .cloned()
        .unwrap_or(ColorOrGradient::Color(fallback));
    color_to_rgba(color, label)
}

fn color_to_rgba(color: ColorOrGradient, label: &str) -> Result<[f32; 4], AvengerChartError> {
    match color {
        ColorOrGradient::Color(color) => Ok(color),
        ColorOrGradient::GradientIndex(_) => Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D {label} cannot be a gradient in v1"
        ))),
    }
}

fn coerce_cell_colors(values: &ArrayRef) -> Result<Vec<[f32; 4]>, AvengerChartError> {
    let colors = Coercer::default()
        .to_color(values, Some(ColorOrGradient::transparent()))
        .map_err(AvengerChartError::ScaleError)?;
    colors
        .as_vec(values.len(), None)
        .into_iter()
        .enumerate()
        .map(|(index, color)| color_to_rgba(color, &format!("fill cell {index}")))
        .collect()
}

fn scale_extent(
    context: &dyn MarkRuntimeContext,
    channel: &str,
    channel_value: &ChannelValue,
    start: f64,
    stop: f64,
) -> Result<(f32, f32), AvengerChartError> {
    let scale_name = channel_value
        .get_scale_name(channel)
        .unwrap_or_else(|| channel.to_string());
    let scale = context.configured_scale(&scale_name).ok_or_else(|| {
        AvengerChartError::ScaleNotFound(format!(
            "UniformRaster2D expected configured scale '{scale_name}' for channel '{channel}'"
        ))
    })?;
    let values = Arc::new(Float64Array::from(vec![start, stop])) as ArrayRef;
    let scaled = scale.scale_to_numeric(&values)?;
    let scaled = scaled.as_vec(2, None);
    let start = scaled[0];
    let stop = scaled[1];
    if !start.is_finite() || !stop.is_finite() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D scaled {channel} extent is non-finite: {start:?} -> {stop:?}"
        )));
    }
    Ok((start, stop))
}

fn transform_extent(
    coord: &dyn CoordinateSystemTransformCore,
    context: &dyn MarkRuntimeContext,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
) -> Result<(f32, f32, f32, f32), AvengerChartError> {
    let mut position_channels = std::collections::HashMap::new();
    position_channels.insert("x", ScalarOrArray::new_array(vec![x0, x1]));
    position_channels.insert("y", ScalarOrArray::new_array(vec![y0, y1]));
    let geometry = coord.transform(
        &position_channels,
        None,
        context.plot_width(),
        context.plot_height(),
    )?;
    let point = geometry
        .as_any()
        .downcast_ref::<PointGeometry>()
        .ok_or_else(|| {
            AvengerChartError::CoordinateSystemError(
                "UniformRaster2D expected Cartesian point geometry".to_string(),
            )
        })?;
    let x = point.x.as_vec(2, None);
    let y = point.y.as_vec(2, None);
    Ok((x[0], y[0], x[1], y[1]))
}

fn build_rgba_image(
    raster: &UniformRasterRow,
    colors: &[[f32; 4]],
    null_color: [f32; 4],
    non_finite_color: [f32; 4],
    opacity: f32,
    transpose: bool,
    flip_x: bool,
    flip_y: bool,
) -> Result<RgbaImage, AvengerChartError> {
    let width = if transpose {
        raster.rows_count
    } else {
        raster.columns_count
    };
    let height = if transpose {
        raster.columns_count
    } else {
        raster.rows_count
    };
    let mut data = Vec::with_capacity(width as usize * height as usize * 4);
    for py in 0..height as usize {
        for px in 0..width as usize {
            let axis_x = if flip_x { width as usize - 1 - px } else { px };
            let axis_y = if flip_y { height as usize - 1 - py } else { py };
            let (storage_row, storage_col) = if transpose {
                (axis_x, axis_y)
            } else {
                (axis_y, axis_x)
            };
            let cell_index = storage_row * raster.columns_count as usize + storage_col;
            let color = if raster.values.is_null(cell_index) {
                null_color
            } else if cell_is_non_finite(&raster.values, cell_index)? {
                non_finite_color
            } else {
                colors[cell_index]
            };
            push_rgba8(&mut data, color, opacity);
        }
    }
    Ok(RgbaImage {
        width,
        height,
        data,
    })
}

fn cell_is_non_finite(array: &ArrayRef, index: usize) -> Result<bool, AvengerChartError> {
    match array.data_type() {
        DataType::Float32 => {
            let values = array.as_primitive::<Float32Type>();
            Ok(!values.value(index).is_finite())
        }
        DataType::Float64 => {
            let values = array.as_primitive::<Float64Type>();
            Ok(!values.value(index).is_finite())
        }
        _ => Ok(false),
    }
}

fn push_rgba8(data: &mut Vec<u8>, mut rgba: [f32; 4], opacity: f32) {
    rgba[3] *= opacity;
    for component in rgba {
        data.push((component.clamp(0.0, 1.0) * 255.0).round() as u8);
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use avenger_chart_core::{
        ChannelConfig, ChannelValue, CompiledDataContext, CompiledMarkCore, CompiledMarkState,
        CoordMeasurement, EmptyCoordMeasurement, EvaluationContext, FacetDataScope, MarkDataMode,
        MarkRenderContext, MarkRuntimeContext, Theme,
    };
    use avenger_chart_marks::{
        UNIFORM_RASTER_2D_FILL_CHANNEL, UNIFORM_RASTER_2D_RASTER_CHANNEL, UniformRaster2D,
    };
    use avenger_scales::scales::{ConfiguredScale, linear::LinearScale};
    use avenger_scenegraph::marks::{image::SceneImageSource, mark::SceneMark};
    use datafusion::{
        arrow::{
            array::{
                ArrayRef, Float32Builder, Float64Array, Float64Builder, ListBuilder, StringArray,
                StringBuilder, StructArray, UInt32Array,
            },
            datatypes::{DataType, Field},
            record_batch::RecordBatch,
        },
        common::ScalarValue,
        prelude::{SessionContext, col},
    };

    use crate::marks::CartesianUniformRaster2DChannels;

    use super::*;

    struct TestRuntimeContext {
        eval: EvaluationContext,
        measurement: EmptyCoordMeasurement,
        scales: HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
    }

    impl MarkRuntimeContext for TestRuntimeContext {
        fn core_view(&self) -> MarkRenderContext<'_> {
            MarkRenderContext::new(&self.eval, self.plot_width, self.plot_height)
        }

        fn coord_measurement(&self) -> &dyn CoordMeasurement {
            &self.measurement
        }

        fn facet_path(&self) -> &[ScalarValue] {
            &[]
        }

        fn plot_width(&self) -> f32 {
            self.plot_width
        }

        fn plot_height(&self) -> f32 {
            self.plot_height
        }

        fn configured_scale(&self, scale_key: &str) -> Option<&ConfiguredScale> {
            self.scales.get(scale_key)
        }
    }

    fn compiled_state() -> CompiledMarkState {
        CompiledMarkState {
            id: None,
            public_target_path: None,
            data: CompiledDataContext::default(),
            data_mode: MarkDataMode::Inherit,
            mark_index: 0,
            facet_data_scope: FacetDataScope::FILTERED,
            exclude_from_scale_domains: false,
            visible: None,
            details: None,
            zindex: None,
            geometry_space: None,
            axis_configs: HashMap::new(),
        }
    }

    fn string_list(values: &[&str]) -> ArrayRef {
        let mut builder = ListBuilder::new(StringBuilder::new());
        for value in values {
            builder.values().append_value(*value);
        }
        builder.append(true);
        Arc::new(builder.finish()) as ArrayRef
    }

    fn float64_list(values: &[Option<f64>]) -> ArrayRef {
        let mut builder = ListBuilder::new(Float64Builder::new());
        for value in values {
            if let Some(value) = value {
                builder.values().append_value(*value);
            } else {
                builder.values().append_null();
            }
        }
        builder.append(true);
        Arc::new(builder.finish()) as ArrayRef
    }

    fn rgba_list(colors: &[[f32; 4]]) -> ArrayRef {
        let mut builder = ListBuilder::new(ListBuilder::new(Float32Builder::new()));
        for color in colors {
            for component in color {
                builder.values().values().append_value(*component);
            }
            builder.values().append(true);
        }
        builder.append(true);
        Arc::new(builder.finish()) as ArrayRef
    }

    fn one_row_axis_with_sampling(
        start: f64,
        stop: f64,
        count: u32,
        sampling: Option<&str>,
    ) -> StructArray {
        StructArray::from(vec![
            (
                Arc::new(Field::new("coord", DataType::Utf8, false)),
                Arc::new(StringArray::from(vec!["axis"])) as ArrayRef,
            ),
            (
                Arc::new(Field::new("sampling", DataType::Utf8, true)),
                Arc::new(StringArray::from(vec![sampling])) as ArrayRef,
            ),
            (
                Arc::new(Field::new("start", DataType::Float64, false)),
                Arc::new(Float64Array::from(vec![start])) as ArrayRef,
            ),
            (
                Arc::new(Field::new("stop", DataType::Float64, false)),
                Arc::new(Float64Array::from(vec![stop])) as ArrayRef,
            ),
            (
                Arc::new(Field::new("count", DataType::UInt32, false)),
                Arc::new(UInt32Array::from(vec![count])) as ArrayRef,
            ),
        ])
    }

    fn raster_batch() -> RecordBatch {
        raster_batch_with_sampling(None, None)
    }

    fn raster_batch_with_sampling(
        columns_sampling: Option<&str>,
        rows_sampling: Option<&str>,
    ) -> RecordBatch {
        let values_data = string_list(&["#ff0000", "#00ff00", "#0000ff", "#ffffff"]);
        raster_batch_from_parts(
            one_row_axis_with_sampling(0.0, 2.0, 2, columns_sampling),
            one_row_axis_with_sampling(0.0, 2.0, 2, rows_sampling),
            values_data.clone(),
            values_data,
            None,
        )
    }

    fn raster_batch_from_parts(
        columns_axis: StructArray,
        rows_axis: StructArray,
        values_data: ArrayRef,
        fill_data: ArrayRef,
        opacity: Option<f64>,
    ) -> RecordBatch {
        let columns = Arc::new(columns_axis) as ArrayRef;
        let rows = Arc::new(rows_axis) as ArrayRef;
        let geometry = Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new("kind", DataType::Utf8, false)),
                Arc::new(StringArray::from(vec!["uniform"])) as ArrayRef,
            ),
            (
                Arc::new(Field::new("columns", columns.data_type().clone(), false)),
                columns,
            ),
            (
                Arc::new(Field::new("rows", rows.data_type().clone(), false)),
                rows,
            ),
        ])) as ArrayRef;
        let values = Arc::new(StructArray::from(vec![(
            Arc::new(Field::new("data", values_data.data_type().clone(), false)),
            values_data.clone(),
        )])) as ArrayRef;
        let raster = Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new("geometry", geometry.data_type().clone(), false)),
                geometry,
            ),
            (
                Arc::new(Field::new("values", values.data_type().clone(), false)),
                values,
            ),
        ])) as ArrayRef;

        let mut columns = vec![
            (UNIFORM_RASTER_2D_RASTER_CHANNEL, raster),
            (UNIFORM_RASTER_2D_FILL_CHANNEL, fill_data),
        ];
        if let Some(opacity) = opacity {
            columns.push((
                "opacity",
                Arc::new(Float64Array::from(vec![opacity])) as ArrayRef,
            ));
        }

        RecordBatch::try_from_iter(columns).expect("record batch")
    }

    fn test_context() -> TestRuntimeContext {
        test_context_with_domains((0.0, 2.0), (0.0, 2.0), 200.0, 200.0)
    }

    fn test_context_with_domains(
        x_domain: (f32, f32),
        y_domain: (f32, f32),
        plot_width: f32,
        plot_height: f32,
    ) -> TestRuntimeContext {
        let mut scales = HashMap::new();
        scales.insert(
            "x".to_string(),
            LinearScale::configured(x_domain, (0.0, plot_width)),
        );
        scales.insert(
            "y".to_string(),
            LinearScale::configured(y_domain, (plot_height, 0.0)),
        );
        TestRuntimeContext {
            eval: EvaluationContext::new(
                Arc::new(Theme::light()),
                Arc::new(SessionContext::new()),
                Default::default(),
            ),
            measurement: EmptyCoordMeasurement,
            scales,
            plot_width,
            plot_height,
        }
    }

    #[test]
    fn raster_with_keeps_xy_out_of_render_channels_and_domain_sources() {
        let ctx = SessionContext::new();
        let mark = UniformRaster2D::<Cartesian>::new().raster_with(col("raster"), |r| {
            r.fill(|fill| fill.no_scale())
                .x(|x| x.with_scale_name("raster_x").axis(|axis| axis))
                .y(|y| y.with_scale_name("raster_y"))
        });
        let channels = mark.get_data_context().channels();
        assert!(channels.contains_key(UNIFORM_RASTER_2D_RASTER_CHANNEL));
        assert!(channels.contains_key(UNIFORM_RASTER_2D_FILL_CHANNEL));
        assert!(!channels.contains_key("x"));
        assert!(!channels.contains_key("y"));
        assert!(matches!(
            channels.get(UNIFORM_RASTER_2D_RASTER_CHANNEL),
            Some(ChannelValue::Value { .. })
        ));
        assert!(matches!(
            channels.get(UNIFORM_RASTER_2D_FILL_CHANNEL),
            Some(ChannelValue::Value { .. })
        ));
        assert!(mark.state().axis_configs.contains_key("x"));

        let data = RecordBatch::try_from_iter(vec![(
            "dummy",
            Arc::new(Float64Array::from(vec![1.0])) as ArrayRef,
        )])
        .expect("dummy batch");
        let df = ctx.read_batch(data).expect("dataframe");
        let compiled = CompiledCartesianUniformRaster2D {
            state: CompiledMarkState::from_mark_state(mark.state(), Some(df.clone())),
            options: mark.raster_options().clone(),
        };
        let sources = compiled
            .scale_domain_sources(Some(&df), &ctx)
            .expect("domain sources");
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].channel, "x");
        assert_eq!(
            sources[0].channel_value.get_scale_name("x").as_deref(),
            Some("raster_x")
        );
        assert_eq!(sources[0].exprs.len(), 2);
        assert_eq!(sources[1].channel, "y");
        assert_eq!(
            sources[1].channel_value.get_scale_name("y").as_deref(),
            Some("raster_y")
        );
        assert_eq!(sources[1].exprs.len(), 2);
    }

    #[test]
    fn renders_direct_color_uniform_raster_to_inline_scene_image() {
        let mark = CompiledCartesianUniformRaster2D {
            state: compiled_state(),
            options: UniformRaster2DOptions::default(),
        };
        let data = raster_batch();
        let scalars =
            RecordBatch::new_empty(Arc::new(datafusion::arrow::datatypes::Schema::empty()));
        let context = test_context();
        let rendered = mark
            .render_uniform_raster_mark_data(Some(&data), &scalars, &context, &Cartesian::new())
            .expect("render raster");

        assert_eq!(rendered.marks.len(), 1);
        assert_eq!(rendered.source_row_indices, Some(vec![vec![0]]));

        let SceneMark::Image(image_mark) = &rendered.marks[0] else {
            panic!("expected image mark");
        };
        assert_eq!(*image_mark.x.first().expect("x"), 0.0);
        assert_eq!(*image_mark.y.first().expect("y"), 0.0);
        assert_eq!(*image_mark.width.first().expect("width"), 200.0);
        assert_eq!(*image_mark.height.first().expect("height"), 200.0);

        let SceneImageSource::Inline(image) = image_mark.image.first().expect("image") else {
            panic!("expected inline image");
        };
        assert_eq!(image.width, 2);
        assert_eq!(image.height, 2);
        assert_eq!(
            image.data,
            vec![
                0, 0, 255, 255, 255, 255, 255, 255, 255, 0, 0, 255, 0, 255, 0, 255,
            ]
        );
    }

    #[test]
    fn applies_null_non_finite_colors_and_row_opacity_after_fill_colorization() {
        let mut options = UniformRaster2DOptions::default();
        options.null_color = ChannelValue::from("#00000080");
        options.non_finite_color = ChannelValue::from("#ff00ff");
        let mark = CompiledCartesianUniformRaster2D {
            state: compiled_state(),
            options,
        };
        let values_data = float64_list(&[Some(0.0), None, Some(f64::NAN), Some(f64::INFINITY)]);
        let fill_data = rgba_list(&[
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
        ]);
        let data = raster_batch_from_parts(
            one_row_axis_with_sampling(0.0, 2.0, 2, None),
            one_row_axis_with_sampling(0.0, 2.0, 2, None),
            values_data,
            fill_data,
            Some(0.5),
        );
        let scalars =
            RecordBatch::new_empty(Arc::new(datafusion::arrow::datatypes::Schema::empty()));
        let context = test_context();
        let rendered = mark
            .render_uniform_raster_mark_data(Some(&data), &scalars, &context, &Cartesian::new())
            .expect("render raster");

        let SceneMark::Image(image_mark) = &rendered.marks[0] else {
            panic!("expected image mark");
        };
        let SceneImageSource::Inline(image) = image_mark.image.first().expect("image") else {
            panic!("expected inline image");
        };
        assert_eq!(
            image.data,
            vec![
                255, 0, 255, 128, 255, 0, 255, 128, 255, 0, 0, 128, 0, 0, 0, 64,
            ]
        );
    }

    #[test]
    fn transpose_swaps_dimensions_and_pixel_mapping() {
        let mut options = UniformRaster2DOptions::default();
        options.transpose = true;
        let mark = CompiledCartesianUniformRaster2D {
            state: compiled_state(),
            options,
        };
        let values_data = string_list(&[
            "#ff0000", "#00ff00", "#0000ff", "#ffffff", "#000000", "#ffff00",
        ]);
        let data = raster_batch_from_parts(
            one_row_axis_with_sampling(0.0, 2.0, 2, None),
            one_row_axis_with_sampling(0.0, 3.0, 3, None),
            values_data.clone(),
            values_data,
            None,
        );
        let scalars =
            RecordBatch::new_empty(Arc::new(datafusion::arrow::datatypes::Schema::empty()));
        let context = test_context_with_domains((0.0, 3.0), (0.0, 2.0), 300.0, 200.0);
        let rendered = mark
            .render_uniform_raster_mark_data(Some(&data), &scalars, &context, &Cartesian::new())
            .expect("render raster");

        let SceneMark::Image(image_mark) = &rendered.marks[0] else {
            panic!("expected image mark");
        };
        let SceneImageSource::Inline(image) = image_mark.image.first().expect("image") else {
            panic!("expected inline image");
        };
        assert_eq!(image.width, 3);
        assert_eq!(image.height, 2);
        assert_eq!(
            image.data,
            vec![
                0, 255, 0, 255, 255, 255, 255, 255, 255, 255, 0, 255, 255, 0, 0, 255, 0, 0, 255,
                255, 0, 0, 0, 255,
            ]
        );
    }

    #[test]
    fn rejects_nonlinear_sampling_in_phase_1() {
        let mark = CompiledCartesianUniformRaster2D {
            state: compiled_state(),
            options: UniformRaster2DOptions::default(),
        };
        let data = raster_batch_with_sampling(Some("log10"), None);
        let scalars =
            RecordBatch::new_empty(Arc::new(datafusion::arrow::datatypes::Schema::empty()));
        let context = test_context();
        let err = match mark.render_uniform_raster_mark_data(
            Some(&data),
            &scalars,
            &context,
            &Cartesian::new(),
        ) {
            Ok(_) => panic!("nonlinear sampling should be rejected"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("supports only linear sampling"),
            "{err}"
        );
    }
}
