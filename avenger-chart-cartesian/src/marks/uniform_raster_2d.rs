use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use avenger_chart_core::{
    Auto, AvengerChartError, ChannelDescriptor, ChannelValue, CompiledDataContext, CompiledMark,
    CompiledMarkCore, CompiledMarkState, CoordinateSystemTransformCore, LegendRendererKind,
    LegendRendererSelection, Mark, MarkRenderContext, MarkRuntimeContext, MarkScaleDomainChannel,
    MarkScaleDomainSource, PointGeometry, RenderedMarkData, ResolvedDomain, Scale, ScaleRange,
    ScaleTypePreference, Theme, coerce_opacity_channel_with_renderer,
    default_scale_type_for_data_type, impl_mark_trait_common, is_continuous_scale,
};
use avenger_chart_marks::{
    RasterPositionSpec, UNIFORM_RASTER_2D_FILL_CHANNEL, UNIFORM_RASTER_2D_RASTER_CHANNEL,
    UniformRaster2D, UniformRaster2DOptions, uniform_raster_2d_channel_defaults,
};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{ImageAlign, ImageBaseline},
    value::ScalarOrArray,
};
use avenger_image::RgbaImage;
use avenger_scales::scales::{ConfiguredScale, DomainKind, ScaleImpl, coerce::Coercer};
use avenger_scenegraph::marks::{
    image::{SceneImageMark, SceneImageSource},
    mark::SceneMark,
};
use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, AsArray, Float64Array, ListBuilder, StringArray, StringBuilder,
            StructArray,
        },
        datatypes::{DataType, Field, Float32Type, Float64Type},
        record_batch::RecordBatch,
    },
    common::{DataFusionError, ScalarValue},
    dataframe::DataFrame,
    logical_expr::{
        ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, TypeSignature,
        Volatility,
    },
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

    fn scale_domain_channels(&self) -> Result<Vec<MarkScaleDomainChannel>, AvengerChartError> {
        if self.state.exclude_from_scale_domains {
            return Ok(Vec::new());
        }

        let x_position = required_position(&self.options.x_position, "x")?;
        let y_position = required_position(&self.options.y_position, "y")?;

        Ok(vec![
            MarkScaleDomainChannel {
                channel: "x".to_string(),
                channel_value: x_position.channel_value.clone(),
            },
            MarkScaleDomainChannel {
                channel: "y".to_string(),
                channel_value: y_position.channel_value.clone(),
            },
        ])
    }

    fn scale_domain_sources(
        &self,
        domain_dataframe: Option<&DataFrame>,
        ctx: &SessionContext,
    ) -> Result<Vec<MarkScaleDomainSource>, AvengerChartError> {
        if self.state.exclude_from_scale_domains {
            return Ok(Vec::new());
        }
        if self.state.view.is_some() {
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

        let x_position = required_position(&self.options.x_position, "x")?;
        let y_position = required_position(&self.options.y_position, "y")?;

        let x_exprs = raster_domain_exprs_for_position(raster_expr.clone(), x_position);
        let y_exprs = raster_domain_exprs_for_position(raster_expr, y_position);

        Ok(vec![
            MarkScaleDomainSource {
                channel: "x".to_string(),
                channel_value: x_position.channel_value.clone(),
                dataframe: dataframe.clone(),
                exprs: x_exprs,
            },
            MarkScaleDomainSource {
                channel: "y".to_string(),
                channel_value: y_position.channel_value.clone(),
                dataframe,
                exprs: y_exprs,
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
            "x" | "y"
                if matches!(
                    data_type,
                    DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
                ) =>
            {
                Some(ScaleTypePreference::Band)
            }
            UNIFORM_RASTER_2D_FILL_CHANNEL
                if matches!(
                    data_type,
                    DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
                ) =>
            {
                Some(ScaleTypePreference::Ordinal)
            }
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
            smooth = self.options.smooth,
            "rendering uniform raster rows"
        );

        let x_position = required_position(&self.options.x_position, "x")?;
        let y_position = required_position(&self.options.y_position, "y")?;
        if x_position.dim.name() == y_position.dim.name() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "UniformRaster2D x and y dimensions must be distinct, got '{}'",
                x_position.dim.name()
            )));
        }

        let mut marks = Vec::new();
        let mut source_row_indices = Vec::new();
        for row in 0..len {
            let raster_row =
                row_for_channel(raster_array, row, len, UNIFORM_RASTER_2D_RASTER_CHANNEL)?;
            let fill_row = row_for_channel(fill_array, row, len, UNIFORM_RASTER_2D_FILL_CHANNEL)?;
            let raster = extract_grid_raster_row(raster_array, raster_row)?;
            raster.validate_scalar_render_dims(x_position.dim.name(), y_position.dim.name())?;
            let fill_values =
                list_row_values(fill_array, fill_row, UNIFORM_RASTER_2D_FILL_CHANNEL)?;
            if fill_values.len() != raster.values.len() {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "UniformRaster2D fill row {row} has {} cells, but raster values require {}",
                    fill_values.len(),
                    raster.values.len()
                )));
            }

            let colors = coerce_cell_colors(&fill_values)?;
            let row_marks = build_scene_marks_for_raster(
                &raster,
                &colors,
                null_color,
                non_finite_color,
                opacity_values[row],
                x_position,
                y_position,
                self.options.smooth,
                self.state.zindex,
                context,
                coord,
            )?;

            for mark in row_marks {
                marks.push(mark);
                source_row_indices.push(vec![row]);
            }
        }

        Ok(RenderedMarkData::with_source_row_indices(
            marks,
            source_row_indices,
        ))
    }
}

fn build_scene_marks_for_raster(
    raster: &GridRasterRow,
    colors: &[[f32; 4]],
    null_color: [f32; 4],
    non_finite_color: [f32; 4],
    opacity: f32,
    x_position: &avenger_chart_marks::RasterPositionSpec,
    y_position: &avenger_chart_marks::RasterPositionSpec,
    smooth: bool,
    zindex: Option<i32>,
    context: &dyn MarkRuntimeContext,
    coord: &dyn CoordinateSystemTransformCore,
) -> Result<Vec<SceneMark>, AvengerChartError> {
    let x_dim = raster.dimension(x_position.dim.name())?;
    let y_dim = raster.dimension(y_position.dim.name())?;

    match (&x_dim.coords, &y_dim.coords) {
        (
            RasterCoords::Uniform {
                start: x_start,
                stop: x_stop,
                count: x_count,
                ..
            },
            RasterCoords::Uniform {
                start: y_start,
                stop: y_stop,
                count: y_count,
                ..
            },
        ) => {
            let (scaled_x0, scaled_x1) =
                scale_numeric_extent(context, "x", &x_position.channel_value, *x_start, *x_stop)?;
            let (scaled_y0, scaled_y1) =
                scale_numeric_extent(context, "y", &y_position.channel_value, *y_start, *y_stop)?;
            let (scaled_x0, scaled_y0, scaled_x1, scaled_y1) =
                transform_extent(coord, context, scaled_x0, scaled_y0, scaled_x1, scaled_y1)?;

            let flip_x = scaled_x1 < scaled_x0;
            let flip_y = scaled_y1 < scaled_y0;
            let x = scaled_x0.min(scaled_x1);
            let y = scaled_y0.min(scaled_y1);
            let width = (scaled_x1 - scaled_x0).abs();
            let height = (scaled_y1 - scaled_y0).abs();
            let x_indices = (0..*x_count as usize).collect::<Vec<_>>();
            let y_indices = (0..*y_count as usize).collect::<Vec<_>>();
            let image = build_rgba_image(
                raster,
                colors,
                null_color,
                non_finite_color,
                opacity,
                x_dim.name.as_str(),
                y_dim.name.as_str(),
                &x_indices,
                &y_indices,
                flip_x,
                flip_y,
            )?;

            trace!(
                x_dim = x_dim.name,
                y_dim = y_dim.name,
                image_width = image.width,
                image_height = image.height,
                x,
                y,
                width,
                height,
                opacity,
                flip_x,
                flip_y,
                "uniform raster row converted to SceneImageMark"
            );

            Ok(vec![scene_image_mark(
                image, x, y, width, height, smooth, zindex,
            )])
        }
        (
            RasterCoords::Uniform {
                start: x_start,
                stop: x_stop,
                count: x_count,
                ..
            },
            RasterCoords::Categorical { values: y_values },
        ) => {
            let (scaled_x0, scaled_x1) =
                scale_numeric_extent(context, "x", &x_position.channel_value, *x_start, *x_stop)?;
            let mut marks = Vec::with_capacity(y_values.len());
            for (y_index, y_value) in y_values.iter().enumerate() {
                let (scaled_y0, scaled_y1) =
                    scale_category_extent(context, "y", &y_position.channel_value, y_value)?;
                let (scaled_x0, scaled_y0, scaled_x1, scaled_y1) =
                    transform_extent(coord, context, scaled_x0, scaled_y0, scaled_x1, scaled_y1)?;
                let flip_x = scaled_x1 < scaled_x0;
                let x = scaled_x0.min(scaled_x1);
                let y = scaled_y0.min(scaled_y1);
                let width = (scaled_x1 - scaled_x0).abs();
                let height = (scaled_y1 - scaled_y0).abs();
                let x_indices = (0..*x_count as usize).collect::<Vec<_>>();
                let y_indices = vec![y_index];
                let image = build_rgba_image(
                    raster,
                    colors,
                    null_color,
                    non_finite_color,
                    opacity,
                    x_dim.name.as_str(),
                    y_dim.name.as_str(),
                    &x_indices,
                    &y_indices,
                    flip_x,
                    false,
                )?;
                marks.push(scene_image_mark(image, x, y, width, height, smooth, zindex));
            }
            Ok(marks)
        }
        (
            RasterCoords::Categorical { values: x_values },
            RasterCoords::Uniform {
                start: y_start,
                stop: y_stop,
                count: y_count,
                ..
            },
        ) => {
            let (scaled_y0, scaled_y1) =
                scale_numeric_extent(context, "y", &y_position.channel_value, *y_start, *y_stop)?;
            let mut marks = Vec::with_capacity(x_values.len());
            for (x_index, x_value) in x_values.iter().enumerate() {
                let (scaled_x0, scaled_x1) =
                    scale_category_extent(context, "x", &x_position.channel_value, x_value)?;
                let (scaled_x0, scaled_y0, scaled_x1, scaled_y1) =
                    transform_extent(coord, context, scaled_x0, scaled_y0, scaled_x1, scaled_y1)?;
                let flip_y = scaled_y1 < scaled_y0;
                let x = scaled_x0.min(scaled_x1);
                let y = scaled_y0.min(scaled_y1);
                let width = (scaled_x1 - scaled_x0).abs();
                let height = (scaled_y1 - scaled_y0).abs();
                let x_indices = vec![x_index];
                let y_indices = (0..*y_count as usize).collect::<Vec<_>>();
                let image = build_rgba_image(
                    raster,
                    colors,
                    null_color,
                    non_finite_color,
                    opacity,
                    x_dim.name.as_str(),
                    y_dim.name.as_str(),
                    &x_indices,
                    &y_indices,
                    false,
                    flip_y,
                )?;
                marks.push(scene_image_mark(image, x, y, width, height, smooth, zindex));
            }
            Ok(marks)
        }
        (RasterCoords::Categorical { .. }, RasterCoords::Categorical { .. }) => {
            Err(AvengerChartError::InvalidArgument(
                "UniformRaster2D supports only one categorical rendered dimension in this phase"
                    .to_string(),
            ))
        }
    }
}

fn scene_image_mark(
    image: RgbaImage,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    smooth: bool,
    zindex: Option<i32>,
) -> SceneMark {
    SceneImageMark {
        name: "uniform_raster_2d".to_string(),
        interactive: true,
        clip: true,
        len: 1,
        aspect: false,
        smooth,
        image: ScalarOrArray::new_scalar(SceneImageSource::Inline(image)),
        x: ScalarOrArray::new_scalar(x),
        y: ScalarOrArray::new_scalar(y),
        width: ScalarOrArray::new_scalar(width),
        height: ScalarOrArray::new_scalar(height),
        align: ScalarOrArray::new_scalar(ImageAlign::Left),
        baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
        unavailable_policy: Default::default(),
        indices: None,
        zindex,
    }
    .into()
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
struct GridRasterRow {
    dimensions: Vec<RasterDimension>,
    values_dims: Vec<String>,
    strides: HashMap<String, usize>,
    values: ArrayRef,
}

#[derive(Debug)]
struct RasterDimension {
    name: String,
    coords: RasterCoords,
}

#[derive(Debug)]
enum RasterCoords {
    Uniform {
        _sampling: Option<String>,
        start: f64,
        stop: f64,
        count: u32,
    },
    Categorical {
        values: Vec<String>,
    },
}

impl RasterCoords {
    fn len(&self) -> usize {
        match self {
            Self::Uniform { count, .. } => *count as usize,
            Self::Categorical { values } => values.len(),
        }
    }
}

impl GridRasterRow {
    fn dimension(&self, name: &str) -> Result<&RasterDimension, AvengerChartError> {
        self.dimensions
            .iter()
            .find(|dimension| dimension.name == name)
            .ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "UniformRaster2D raster is missing dimension '{name}'"
                ))
            })
    }

    fn validate_scalar_render_dims(
        &self,
        x_dim: &str,
        y_dim: &str,
    ) -> Result<(), AvengerChartError> {
        self.dimension(x_dim)?;
        self.dimension(y_dim)?;
        if self.values_dims.len() != 2 {
            return Err(AvengerChartError::InvalidArgument(format!(
                "UniformRaster2D scalar fill requires values.dims to contain exactly x/y dimensions, got {:?}",
                self.values_dims
            )));
        }
        if !self.values_dims.iter().any(|name| name == x_dim)
            || !self.values_dims.iter().any(|name| name == y_dim)
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "UniformRaster2D values.dims {:?} must contain selected x/y dimensions '{x_dim}' and '{y_dim}'",
                self.values_dims
            )));
        }
        Ok(())
    }

    fn cell_index(
        &self,
        x_dim: &str,
        x_index: usize,
        y_dim: &str,
        y_index: usize,
    ) -> Result<usize, AvengerChartError> {
        let mut index = 0usize;
        for dim_name in &self.values_dims {
            let dim = self.dimension(dim_name)?;
            let dim_index = if dim_name == x_dim {
                x_index
            } else if dim_name == y_dim {
                y_index
            } else {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "UniformRaster2D scalar fill cannot render unselected value dimension '{dim_name}'"
                )));
            };
            if dim_index >= dim.coords.len() {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "UniformRaster2D index {dim_index} is out of bounds for dimension '{}' with length {}",
                    dim.name,
                    dim.coords.len()
                )));
            }
            let stride = self.strides.get(dim_name).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "UniformRaster2D missing computed stride for dimension '{dim_name}'"
                ))
            })?;
            index += dim_index * stride;
        }
        Ok(index)
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

fn extract_grid_raster_row(
    array: &ArrayRef,
    row: usize,
) -> Result<GridRasterRow, AvengerChartError> {
    let raster = as_struct_array(array, "raster")?;
    if raster.is_null(row) {
        return Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D raster row {row} is null"
        )));
    }

    let geometry = struct_child_struct(raster, "geometry")?;
    let values = struct_child_struct(raster, "values")?;
    let kind = required_string(struct_child(geometry, "kind")?, row, "geometry.kind")?;
    if kind != "grid" {
        return Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D requires geometry.kind = 'grid', got '{kind}'"
        )));
    }

    let dimensions_array = list_row_values(
        struct_child(geometry, "dimensions")?,
        row,
        "geometry.dimensions",
    )?;
    let dimensions = parse_dimensions(&dimensions_array)?;
    let values_dims =
        string_values_from_list_row(struct_child(values, "dims")?, row, "values.dims")?;
    let values = list_row_values(struct_child(values, "data")?, row, "values.data")?;
    let strides = compute_strides(&dimensions, &values_dims)?;
    let expected_len = values_dims.iter().try_fold(1usize, |acc, dim_name| {
        let dim = dimensions
            .iter()
            .find(|dim| dim.name == *dim_name)
            .ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "UniformRaster2D values.dims references unknown dimension '{dim_name}'"
                ))
            })?;
        acc.checked_mul(dim.coords.len()).ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "UniformRaster2D values.dims dimension lengths overflowed".to_string(),
            )
        })
    })?;
    if values.len() != expected_len {
        return Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D values.data row {row} has {} cells, but values.dims {:?} require {expected_len}",
            values.len(),
            values_dims
        )));
    }
    Ok(GridRasterRow {
        dimensions,
        values_dims,
        strides,
        values,
    })
}

fn parse_dimensions(array: &ArrayRef) -> Result<Vec<RasterDimension>, AvengerChartError> {
    let dimensions = as_struct_array(array, "geometry.dimensions")?;
    let mut seen = HashSet::new();
    let mut parsed = Vec::with_capacity(dimensions.len());
    for index in 0..dimensions.len() {
        if dimensions.is_null(index) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "UniformRaster2D geometry.dimensions[{index}] is null"
            )));
        }
        let name = required_string(struct_child(dimensions, "name")?, index, "dimension.name")?;
        if name.is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "UniformRaster2D dimension names must be non-empty".to_string(),
            ));
        }
        if !seen.insert(name.clone()) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "UniformRaster2D duplicate dimension name '{name}'"
            )));
        }
        let coords = struct_child_struct(dimensions, "coords")?;
        let kind = required_string(struct_child(coords, "kind")?, index, "coords.kind")?;
        let coords = match kind.as_str() {
            "uniform" => {
                let sampling = optional_string(
                    struct_child(coords, "sampling")?,
                    index,
                    &format!("dimension '{name}'.coords.sampling"),
                )?;
                validate_sampling_value(sampling.as_deref(), &format!("dimension '{name}'"))?;
                let count = required_u32(
                    struct_child(coords, "count")?,
                    index,
                    &format!("dimension '{name}'.coords.count"),
                )?;
                if count == 0 {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "UniformRaster2D dimension '{name}' coords.count must be greater than zero"
                    )));
                }
                RasterCoords::Uniform {
                    _sampling: sampling,
                    start: required_f64(
                        struct_child(coords, "start")?,
                        index,
                        &format!("dimension '{name}'.coords.start"),
                    )?,
                    stop: required_f64(
                        struct_child(coords, "stop")?,
                        index,
                        &format!("dimension '{name}'.coords.stop"),
                    )?,
                    count,
                }
            }
            "categorical" => {
                let values = string_values_from_list_row(
                    struct_child(coords, "values")?,
                    index,
                    &format!("dimension '{name}'.coords.values"),
                )?;
                if values.is_empty() {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "UniformRaster2D categorical dimension '{name}' must contain at least one value"
                    )));
                }
                RasterCoords::Categorical { values }
            }
            _ => {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "UniformRaster2D dimension '{name}' has unsupported coords.kind '{kind}'"
                )));
            }
        };
        parsed.push(RasterDimension { name, coords });
    }
    Ok(parsed)
}

fn compute_strides(
    dimensions: &[RasterDimension],
    values_dims: &[String],
) -> Result<HashMap<String, usize>, AvengerChartError> {
    let mut strides = HashMap::new();
    let mut seen = HashSet::new();
    let mut stride = 1usize;
    for dim_name in values_dims.iter().rev() {
        if !seen.insert(dim_name.clone()) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "UniformRaster2D values.dims contains duplicate dimension '{dim_name}'"
            )));
        }
        let dim = dimensions
            .iter()
            .find(|dim| dim.name == *dim_name)
            .ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "UniformRaster2D values.dims references unknown dimension '{dim_name}'"
                ))
            })?;
        strides.insert(dim_name.clone(), stride);
        stride = stride.checked_mul(dim.coords.len()).ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "UniformRaster2D values.dims dimension lengths overflowed".to_string(),
            )
        })?;
    }
    Ok(strides)
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

fn validate_sampling_value(sampling: Option<&str>, label: &str) -> Result<(), AvengerChartError> {
    let Some(sampling) = sampling else {
        return Ok(());
    };
    if sampling == "linear" {
        Ok(())
    } else {
        Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D Phase 1 supports only linear sampling, got {label}.coords.sampling = '{sampling}'"
        )))
    }
}

fn string_values_from_list_row(
    array: &ArrayRef,
    row: usize,
    label: &str,
) -> Result<Vec<String>, AvengerChartError> {
    let values = list_row_values(array, row, label)?;
    match values.data_type() {
        DataType::Utf8 => values
            .as_string::<i32>()
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value.map(str::to_string).ok_or_else(|| {
                    AvengerChartError::InvalidArgument(format!(
                        "UniformRaster2D {label}[{index}] is null"
                    ))
                })
            })
            .collect(),
        DataType::LargeUtf8 => values
            .as_string::<i64>()
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value.map(str::to_string).ok_or_else(|| {
                    AvengerChartError::InvalidArgument(format!(
                        "UniformRaster2D {label}[{index}] is null"
                    ))
                })
            })
            .collect(),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D expected {label} to contain Utf8 values, got {other:?}"
        ))),
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

fn required_position<'a>(
    position: &'a Option<RasterPositionSpec>,
    channel: &str,
) -> Result<&'a RasterPositionSpec, AvengerChartError> {
    position.as_ref().ok_or_else(|| {
        AvengerChartError::MissingChannelError(format!(
            "UniformRaster2D raster_with(...).{channel}(dim(...))"
        ))
    })
}

fn raster_domain_exprs_for_position(
    raster_expr: datafusion::logical_expr::Expr,
    position: &RasterPositionSpec,
) -> Vec<datafusion::logical_expr::Expr> {
    if position_uses_categorical_domain(position) {
        vec![raster_categorical_dim_values_expr(
            raster_expr,
            position.dim.name().to_string(),
        )]
    } else {
        vec![
            raster_uniform_dim_field_expr(
                raster_expr.clone(),
                position.dim.name().to_string(),
                UniformDimField::Start,
            ),
            raster_uniform_dim_field_expr(
                raster_expr,
                position.dim.name().to_string(),
                UniformDimField::Stop,
            ),
        ]
    }
}

fn position_uses_categorical_domain(position: &RasterPositionSpec) -> bool {
    position
        .channel_value
        .get_scale_config()
        .and_then(|config| Scale::<Auto>::from_config(config.clone()).domain_kind())
        .is_some_and(|kind| {
            matches!(
                kind,
                DomainKind::Categorical | DomainKind::NestedCategorical
            )
        })
}

#[derive(Debug, Clone, Copy)]
enum UniformDimField {
    Start,
    Stop,
}

impl UniformDimField {
    fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
        }
    }

    fn value(self, coords: &RasterCoords) -> Result<f64, AvengerChartError> {
        match coords {
            RasterCoords::Uniform { start, stop, .. } => match self {
                Self::Start => Ok(*start),
                Self::Stop => Ok(*stop),
            },
            RasterCoords::Categorical { .. } => Err(AvengerChartError::InvalidArgument(format!(
                "UniformRaster2D cannot infer a numeric domain from categorical dimension"
            ))),
        }
    }
}

fn raster_uniform_dim_field_expr(
    raster_expr: datafusion::logical_expr::Expr,
    dim_name: String,
    field: UniformDimField,
) -> datafusion::logical_expr::Expr {
    ScalarUDF::new_from_impl(RasterUniformDimFieldUdf::new(dim_name, field)).call(vec![raster_expr])
}

fn raster_categorical_dim_values_expr(
    raster_expr: datafusion::logical_expr::Expr,
    dim_name: String,
) -> datafusion::logical_expr::Expr {
    ScalarUDF::new_from_impl(RasterCategoricalDimValuesUdf::new(dim_name)).call(vec![raster_expr])
}

#[derive(Debug)]
struct RasterUniformDimFieldUdf {
    name: String,
    dim_name: String,
    field: UniformDimField,
    signature: Signature,
}

impl RasterUniformDimFieldUdf {
    fn new(dim_name: String, field: UniformDimField) -> Self {
        Self {
            name: format!(
                "avenger_raster_uniform_dim_{}_{}",
                sanitize_udf_name(&dim_name),
                field.name()
            ),
            dim_name,
            field,
            signature: Signature::new(TypeSignature::Any(1), Volatility::Immutable),
        }
    }
}

impl ScalarUDFImpl for RasterUniformDimFieldUdf {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> datafusion::common::Result<DataType> {
        Ok(DataType::Float64)
    }

    fn invoke_with_args(
        &self,
        args: ScalarFunctionArgs,
    ) -> datafusion::common::Result<ColumnarValue> {
        let array = columnar_value_to_array(args.args.first(), args.number_rows)?;
        let mut values = Vec::with_capacity(array.len());
        for row in 0..array.len() {
            let raster = extract_grid_raster_row(&array, row)?;
            let dim = raster.dimension(&self.dim_name)?;
            values.push(self.field.value(&dim.coords)?);
        }
        Ok(ColumnarValue::Array(Arc::new(Float64Array::from(values))))
    }
}

#[derive(Debug)]
struct RasterCategoricalDimValuesUdf {
    name: String,
    dim_name: String,
    signature: Signature,
}

impl RasterCategoricalDimValuesUdf {
    fn new(dim_name: String) -> Self {
        Self {
            name: format!(
                "avenger_raster_categorical_dim_{}_values",
                sanitize_udf_name(&dim_name)
            ),
            dim_name,
            signature: Signature::new(TypeSignature::Any(1), Volatility::Immutable),
        }
    }
}

impl ScalarUDFImpl for RasterCategoricalDimValuesUdf {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> datafusion::common::Result<DataType> {
        Ok(DataType::List(Arc::new(Field::new_list_field(
            DataType::Utf8,
            true,
        ))))
    }

    fn invoke_with_args(
        &self,
        args: ScalarFunctionArgs,
    ) -> datafusion::common::Result<ColumnarValue> {
        let array = columnar_value_to_array(args.args.first(), args.number_rows)?;
        let mut builder = ListBuilder::new(StringBuilder::new());
        for row in 0..array.len() {
            let raster = extract_grid_raster_row(&array, row)?;
            let dim = raster.dimension(&self.dim_name)?;
            let RasterCoords::Categorical { values } = &dim.coords else {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "UniformRaster2D cannot infer a categorical domain from uniform dimension '{}'",
                    self.dim_name
                ))
                .into());
            };
            for value in values {
                builder.values().append_value(value);
            }
            builder.append(true);
        }
        Ok(ColumnarValue::Array(Arc::new(builder.finish())))
    }
}

fn columnar_value_to_array(
    value: Option<&ColumnarValue>,
    len: usize,
) -> datafusion::common::Result<ArrayRef> {
    match value {
        Some(ColumnarValue::Array(array)) => Ok(array.clone()),
        Some(ColumnarValue::Scalar(scalar)) => scalar.to_array_of_size(len),
        None => Err(DataFusionError::Internal(
            "UniformRaster2D raster dimension UDF expected one argument".to_string(),
        )),
    }
}

fn sanitize_udf_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
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

fn scale_numeric_extent(
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

fn scale_category_extent(
    context: &dyn MarkRuntimeContext,
    channel: &str,
    channel_value: &ChannelValue,
    value: &str,
) -> Result<(f32, f32), AvengerChartError> {
    let scale_name = channel_value
        .get_scale_name(channel)
        .unwrap_or_else(|| channel.to_string());
    let scale = context.configured_scale(&scale_name).ok_or_else(|| {
        AvengerChartError::ScaleNotFound(format!(
            "UniformRaster2D expected configured scale '{scale_name}' for channel '{channel}'"
        ))
    })?;
    if !matches!(
        scale.scale_impl.domain_kind(),
        DomainKind::Categorical | DomainKind::NestedCategorical
    ) {
        return Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D categorical dimension on channel '{channel}' requires a categorical scale, got '{}'",
            scale.scale_impl.scale_type()
        )));
    }

    let values = Arc::new(StringArray::from(vec![value.to_string()])) as ArrayRef;
    let start = scale_with_band(scale, &values, 0.0)?
        .first()
        .copied()
        .ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "UniformRaster2D categorical scale '{scale_name}' returned no start value"
            ))
        })?;
    let stop = scale_with_band(scale, &values, 1.0)?
        .first()
        .copied()
        .ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "UniformRaster2D categorical scale '{scale_name}' returned no stop value"
            ))
        })?;
    if !start.is_finite() || !stop.is_finite() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D scaled {channel} category '{value}' extent is non-finite: {start:?} -> {stop:?}"
        )));
    }
    Ok((start, stop))
}

fn scale_with_band(
    scale: &ConfiguredScale,
    values: &ArrayRef,
    band: f32,
) -> Result<Vec<f32>, AvengerChartError> {
    let mut scale = scale.clone();
    scale.config.options.insert("band".to_string(), band.into());
    Ok(scale.scale_to_numeric(values)?.as_vec(values.len(), None))
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
    raster: &GridRasterRow,
    colors: &[[f32; 4]],
    null_color: [f32; 4],
    non_finite_color: [f32; 4],
    opacity: f32,
    x_dim: &str,
    y_dim: &str,
    x_indices: &[usize],
    y_indices: &[usize],
    flip_x: bool,
    flip_y: bool,
) -> Result<RgbaImage, AvengerChartError> {
    let width = u32::try_from(x_indices.len()).map_err(|_| {
        AvengerChartError::InvalidArgument(
            "UniformRaster2D image width does not fit in u32".to_string(),
        )
    })?;
    let height = u32::try_from(y_indices.len()).map_err(|_| {
        AvengerChartError::InvalidArgument(
            "UniformRaster2D image height does not fit in u32".to_string(),
        )
    })?;
    let mut data = Vec::with_capacity(width as usize * height as usize * 4);
    for py in 0..height as usize {
        for px in 0..width as usize {
            let x_offset = if flip_x { width as usize - 1 - px } else { px };
            let y_offset = if flip_y { height as usize - 1 - py } else { py };
            let cell_index = raster.cell_index(
                x_dim,
                *x_indices.get(x_offset).ok_or_else(|| {
                    AvengerChartError::InternalError(
                        "UniformRaster2D x pixel index out of bounds".to_string(),
                    )
                })?,
                y_dim,
                *y_indices.get(y_offset).ok_or_else(|| {
                    AvengerChartError::InternalError(
                        "UniformRaster2D y pixel index out of bounds".to_string(),
                    )
                })?,
            )?;
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
        RasterPositionSpec, UNIFORM_RASTER_2D_FILL_CHANNEL, UNIFORM_RASTER_2D_RASTER_CHANNEL,
        UniformRaster2D, dim,
    };
    use avenger_scales::scales::{
        ConfiguredScale, band::BandScale, linear::LinearScale, ordinal::OrdinalScale,
    };
    use avenger_scenegraph::marks::{image::SceneImageSource, mark::SceneMark};
    use datafusion::{
        arrow::{
            array::{
                ArrayRef, Float32Builder, Float64Array, Float64Builder, ListArray, ListBuilder,
                StringArray, StringBuilder, StructArray, UInt32Array,
            },
            buffer::OffsetBuffer,
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
            view: None,
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

    enum TestDimension<'a> {
        Uniform {
            name: &'a str,
            start: f64,
            stop: f64,
            count: u32,
            sampling: Option<&'a str>,
        },
        Categorical {
            name: &'a str,
            values: Vec<&'a str>,
        },
        Unsupported {
            name: &'a str,
            kind: &'a str,
        },
    }

    impl<'a> TestDimension<'a> {
        fn uniform(name: &'a str, start: f64, stop: f64, count: u32) -> Self {
            Self::Uniform {
                name,
                start,
                stop,
                count,
                sampling: None,
            }
        }

        fn uniform_with_sampling(
            name: &'a str,
            start: f64,
            stop: f64,
            count: u32,
            sampling: Option<&'a str>,
        ) -> Self {
            Self::Uniform {
                name,
                start,
                stop,
                count,
                sampling,
            }
        }

        fn categorical(name: &'a str, values: Vec<&'a str>) -> Self {
            Self::Categorical { name, values }
        }

        fn unsupported(name: &'a str, kind: &'a str) -> Self {
            Self::Unsupported { name, kind }
        }
    }

    fn one_row_list(values: ArrayRef) -> ArrayRef {
        let offsets = OffsetBuffer::from_lengths([values.len()]);
        Arc::new(
            ListArray::try_new(
                Arc::new(Field::new_list_field(values.data_type().clone(), true)),
                offsets,
                values,
                None,
            )
            .expect("list array"),
        ) as ArrayRef
    }

    fn dimensions_list(dimensions: &[TestDimension<'_>]) -> ArrayRef {
        let names = dimensions
            .iter()
            .map(|dimension| match dimension {
                TestDimension::Uniform { name, .. } => *name,
                TestDimension::Categorical { name, .. } => *name,
                TestDimension::Unsupported { name, .. } => *name,
            })
            .collect::<Vec<_>>();
        let kinds = dimensions
            .iter()
            .map(|dimension| match dimension {
                TestDimension::Uniform { .. } => "uniform",
                TestDimension::Categorical { .. } => "categorical",
                TestDimension::Unsupported { kind, .. } => *kind,
            })
            .collect::<Vec<_>>();
        let samplings = dimensions
            .iter()
            .map(|dimension| match dimension {
                TestDimension::Uniform { sampling, .. } => *sampling,
                TestDimension::Categorical { .. } | TestDimension::Unsupported { .. } => None,
            })
            .collect::<Vec<_>>();
        let starts = dimensions
            .iter()
            .map(|dimension| match dimension {
                TestDimension::Uniform { start, .. } => Some(*start),
                TestDimension::Categorical { .. } | TestDimension::Unsupported { .. } => None,
            })
            .collect::<Vec<_>>();
        let stops = dimensions
            .iter()
            .map(|dimension| match dimension {
                TestDimension::Uniform { stop, .. } => Some(*stop),
                TestDimension::Categorical { .. } | TestDimension::Unsupported { .. } => None,
            })
            .collect::<Vec<_>>();
        let counts = dimensions
            .iter()
            .map(|dimension| match dimension {
                TestDimension::Uniform { count, .. } => Some(*count),
                TestDimension::Categorical { .. } | TestDimension::Unsupported { .. } => None,
            })
            .collect::<Vec<_>>();
        let mut coord_values_builder = ListBuilder::new(StringBuilder::new());
        for dimension in dimensions {
            match dimension {
                TestDimension::Uniform { .. } => coord_values_builder.append(false),
                TestDimension::Categorical { values, .. } => {
                    for value in values {
                        coord_values_builder.values().append_value(*value);
                    }
                    coord_values_builder.append(true);
                }
                TestDimension::Unsupported { .. } => coord_values_builder.append(false),
            }
        }
        let coord_values = Arc::new(coord_values_builder.finish()) as ArrayRef;
        let coords = Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new("kind", DataType::Utf8, false)),
                Arc::new(StringArray::from(kinds)) as ArrayRef,
            ),
            (
                Arc::new(Field::new("sampling", DataType::Utf8, true)),
                Arc::new(StringArray::from(samplings)) as ArrayRef,
            ),
            (
                Arc::new(Field::new("start", DataType::Float64, true)),
                Arc::new(Float64Array::from(starts)) as ArrayRef,
            ),
            (
                Arc::new(Field::new("stop", DataType::Float64, true)),
                Arc::new(Float64Array::from(stops)) as ArrayRef,
            ),
            (
                Arc::new(Field::new("count", DataType::UInt32, true)),
                Arc::new(UInt32Array::from(counts)) as ArrayRef,
            ),
            (
                Arc::new(Field::new("values", coord_values.data_type().clone(), true)),
                coord_values,
            ),
        ])) as ArrayRef;
        let dimensions = Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new("name", DataType::Utf8, false)),
                Arc::new(StringArray::from(names)) as ArrayRef,
            ),
            (
                Arc::new(Field::new("coords", coords.data_type().clone(), false)),
                coords,
            ),
        ])) as ArrayRef;
        one_row_list(dimensions)
    }

    fn raster_batch() -> RecordBatch {
        raster_batch_with_sampling(None, None)
    }

    fn raster_batch_with_sampling(
        x_sampling: Option<&str>,
        y_sampling: Option<&str>,
    ) -> RecordBatch {
        let values_data = string_list(&["#ff0000", "#00ff00", "#0000ff", "#ffffff"]);
        raster_batch_from_parts(
            &[
                TestDimension::uniform_with_sampling("x", 0.0, 2.0, 2, x_sampling),
                TestDimension::uniform_with_sampling("y", 0.0, 2.0, 2, y_sampling),
            ],
            &["y", "x"],
            values_data.clone(),
            values_data,
            None,
        )
    }

    fn raster_batch_from_parts(
        dimensions: &[TestDimension<'_>],
        values_dims: &[&str],
        values_data: ArrayRef,
        fill_data: ArrayRef,
        opacity: Option<f64>,
    ) -> RecordBatch {
        let dimensions = dimensions_list(dimensions);
        let geometry = Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new("kind", DataType::Utf8, false)),
                Arc::new(StringArray::from(vec!["grid"])) as ArrayRef,
            ),
            (
                Arc::new(Field::new(
                    "dimensions",
                    dimensions.data_type().clone(),
                    false,
                )),
                dimensions,
            ),
        ])) as ArrayRef;
        let dims = string_list(values_dims);
        let values = Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new("dims", dims.data_type().clone(), false)),
                dims,
            ),
            (
                Arc::new(Field::new("data", values_data.data_type().clone(), false)),
                values_data.clone(),
            ),
        ])) as ArrayRef;
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

    fn position(dim_name: &str, channel: &str) -> RasterPositionSpec {
        RasterPositionSpec {
            dim: dim(dim_name),
            channel_value: ChannelValue::from(0.0).with_scale_name(channel),
        }
    }

    fn raster_options(x_dim: &str, y_dim: &str) -> UniformRaster2DOptions {
        UniformRaster2DOptions {
            x_position: Some(position(x_dim, "x")),
            y_position: Some(position(y_dim, "y")),
            ..Default::default()
        }
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

    fn test_context_with_categorical_y() -> TestRuntimeContext {
        let mut context = test_context_with_domains((0.0, 2.0), (0.0, 1.0), 200.0, 200.0);
        context.scales.insert(
            "y".to_string(),
            BandScale::configured(
                Arc::new(StringArray::from(vec!["A", "B"])) as ArrayRef,
                (200.0, 0.0),
            ),
        );
        context
    }

    fn test_context_with_categorical_x() -> TestRuntimeContext {
        let mut context = test_context_with_domains((0.0, 1.0), (0.0, 2.0), 200.0, 200.0);
        context.scales.insert(
            "x".to_string(),
            BandScale::configured(
                Arc::new(StringArray::from(vec!["A", "B"])) as ArrayRef,
                (0.0, 200.0),
            ),
        );
        context
    }

    fn render_error(
        options: UniformRaster2DOptions,
        data: RecordBatch,
        context: &TestRuntimeContext,
    ) -> AvengerChartError {
        let mark = CompiledCartesianUniformRaster2D {
            state: compiled_state(),
            options,
        };
        let scalars =
            RecordBatch::new_empty(Arc::new(datafusion::arrow::datatypes::Schema::empty()));
        match mark.render_uniform_raster_mark_data(
            Some(&data),
            &scalars,
            context,
            &Cartesian::new(),
        ) {
            Ok(_) => panic!("raster render should fail"),
            Err(err) => err,
        }
    }

    #[test]
    fn raster_with_keeps_xy_out_of_render_channels_and_domain_sources() {
        let ctx = SessionContext::new();
        let mark = UniformRaster2D::<Cartesian>::new().raster_with(col("raster"), |r| {
            r.fill(|fill| fill.no_scale())
                .x_with(dim("x"), |x| {
                    x.with_scale_name("raster_x").axis(|axis| axis)
                })
                .y_with(dim("y"), |y| {
                    y.with_scale_name("raster_y").axis(|axis| axis)
                })
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
        assert!(mark.state().axis_configs.contains_key("y"));

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
            options: raster_options("x", "y"),
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
    fn validates_missing_and_invalid_dimension_metadata() {
        let context = test_context();
        let err = render_error(raster_options("missing", "y"), raster_batch(), &context);
        assert!(
            err.to_string().contains("missing dimension 'missing'"),
            "{err}"
        );

        let err = render_error(raster_options("x", "missing"), raster_batch(), &context);
        assert!(
            err.to_string().contains("missing dimension 'missing'"),
            "{err}"
        );

        let values_data = string_list(&["#ff0000", "#00ff00", "#0000ff", "#ffffff"]);
        let data = raster_batch_from_parts(
            &[
                TestDimension::uniform("x", 0.0, 2.0, 2),
                TestDimension::uniform("x", 0.0, 2.0, 2),
            ],
            &["x", "x"],
            values_data.clone(),
            values_data.clone(),
            None,
        );
        let err = render_error(raster_options("x", "y"), data, &context);
        assert!(
            err.to_string().contains("duplicate dimension name 'x'"),
            "{err}"
        );

        let data = raster_batch_from_parts(
            &[
                TestDimension::uniform("x", 0.0, 2.0, 2),
                TestDimension::uniform("y", 0.0, 2.0, 2),
            ],
            &["y", "z"],
            values_data.clone(),
            values_data.clone(),
            None,
        );
        let err = render_error(raster_options("x", "y"), data, &context);
        assert!(
            err.to_string()
                .contains("values.dims references unknown dimension 'z'"),
            "{err}"
        );

        let data = raster_batch_from_parts(
            &[
                TestDimension::unsupported("x", "rectilinear"),
                TestDimension::uniform("y", 0.0, 2.0, 2),
            ],
            &["y", "x"],
            values_data.clone(),
            values_data,
            None,
        );
        let err = render_error(raster_options("x", "y"), data, &context);
        assert!(
            err.to_string()
                .contains("unsupported coords.kind 'rectilinear'"),
            "{err}"
        );
    }

    #[test]
    fn applies_null_non_finite_colors_and_row_opacity_after_fill_colorization() {
        let mut options = UniformRaster2DOptions::default();
        options.null_color = ChannelValue::from("#00000080");
        options.non_finite_color = ChannelValue::from("#ff00ff");
        let mark = CompiledCartesianUniformRaster2D {
            state: compiled_state(),
            options: UniformRaster2DOptions {
                x_position: Some(position("x", "x")),
                y_position: Some(position("y", "y")),
                ..options
            },
        };
        let values_data = float64_list(&[Some(0.0), None, Some(f64::NAN), Some(f64::INFINITY)]);
        let fill_data = rgba_list(&[
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
        ]);
        let data = raster_batch_from_parts(
            &[
                TestDimension::uniform("x", 0.0, 2.0, 2),
                TestDimension::uniform("y", 0.0, 2.0, 2),
            ],
            &["y", "x"],
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
    fn values_dims_controls_storage_order_and_pixel_mapping() {
        let mark = CompiledCartesianUniformRaster2D {
            state: compiled_state(),
            options: raster_options("x", "y"),
        };
        let values_data = string_list(&[
            "#ff0000", "#00ff00", "#0000ff", "#ffffff", "#000000", "#ffff00",
        ]);
        let data = raster_batch_from_parts(
            &[
                TestDimension::uniform("x", 0.0, 2.0, 2),
                TestDimension::uniform("y", 0.0, 3.0, 3),
            ],
            &["x", "y"],
            values_data.clone(),
            values_data,
            None,
        );
        let scalars =
            RecordBatch::new_empty(Arc::new(datafusion::arrow::datatypes::Schema::empty()));
        let context = test_context_with_domains((0.0, 2.0), (0.0, 3.0), 200.0, 300.0);
        let rendered = mark
            .render_uniform_raster_mark_data(Some(&data), &scalars, &context, &Cartesian::new())
            .expect("render raster");

        let SceneMark::Image(image_mark) = &rendered.marks[0] else {
            panic!("expected image mark");
        };
        let SceneImageSource::Inline(image) = image_mark.image.first().expect("image") else {
            panic!("expected inline image");
        };
        assert_eq!(image.width, 2);
        assert_eq!(image.height, 3);
        assert_eq!(
            image.data,
            vec![
                0, 0, 255, 255, 255, 255, 0, 255, 0, 255, 0, 255, 0, 0, 0, 255, 255, 0, 0, 255,
                255, 255, 255, 255,
            ]
        );
    }

    #[test]
    fn renders_one_categorical_dimension_as_strips() {
        let mark = CompiledCartesianUniformRaster2D {
            state: compiled_state(),
            options: raster_options("x", "group"),
        };
        let values_data = string_list(&["#ff0000", "#00ff00", "#0000ff", "#ffffff"]);
        let data = raster_batch_from_parts(
            &[
                TestDimension::uniform("x", 0.0, 2.0, 2),
                TestDimension::categorical("group", vec!["A", "B"]),
            ],
            &["group", "x"],
            values_data.clone(),
            values_data,
            None,
        );
        let scalars =
            RecordBatch::new_empty(Arc::new(datafusion::arrow::datatypes::Schema::empty()));
        let context = test_context_with_categorical_y();
        let rendered = mark
            .render_uniform_raster_mark_data(Some(&data), &scalars, &context, &Cartesian::new())
            .expect("render raster");

        assert_eq!(rendered.marks.len(), 2);
        assert_eq!(rendered.source_row_indices, Some(vec![vec![0], vec![0]]));

        let SceneMark::Image(first_strip) = &rendered.marks[0] else {
            panic!("expected image mark");
        };
        assert_eq!(*first_strip.x.first().expect("x"), 0.0);
        assert_eq!(*first_strip.y.first().expect("y"), 100.0);
        assert_eq!(*first_strip.width.first().expect("width"), 200.0);
        assert_eq!(*first_strip.height.first().expect("height"), 100.0);
        let SceneImageSource::Inline(image) = first_strip.image.first().expect("image") else {
            panic!("expected inline image");
        };
        assert_eq!(image.width, 2);
        assert_eq!(image.height, 1);
        assert_eq!(image.data, vec![255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn renders_categorical_x_dimension_as_vertical_strips() {
        let mark = CompiledCartesianUniformRaster2D {
            state: compiled_state(),
            options: raster_options("group", "y"),
        };
        let values_data = string_list(&["#ff0000", "#00ff00", "#0000ff", "#ffffff"]);
        let data = raster_batch_from_parts(
            &[
                TestDimension::categorical("group", vec!["A", "B"]),
                TestDimension::uniform("y", 0.0, 2.0, 2),
            ],
            &["y", "group"],
            values_data.clone(),
            values_data,
            None,
        );
        let scalars =
            RecordBatch::new_empty(Arc::new(datafusion::arrow::datatypes::Schema::empty()));
        let context = test_context_with_categorical_x();
        let rendered = mark
            .render_uniform_raster_mark_data(Some(&data), &scalars, &context, &Cartesian::new())
            .expect("render raster");

        assert_eq!(rendered.marks.len(), 2);
        assert_eq!(rendered.source_row_indices, Some(vec![vec![0], vec![0]]));

        let SceneMark::Image(first_strip) = &rendered.marks[0] else {
            panic!("expected image mark");
        };
        assert_eq!(*first_strip.x.first().expect("x"), 0.0);
        assert_eq!(*first_strip.y.first().expect("y"), 0.0);
        assert_eq!(*first_strip.width.first().expect("width"), 100.0);
        assert_eq!(*first_strip.height.first().expect("height"), 200.0);
        let SceneImageSource::Inline(image) = first_strip.image.first().expect("image") else {
            panic!("expected inline image");
        };
        assert_eq!(image.width, 1);
        assert_eq!(image.height, 2);
        assert_eq!(image.data, vec![0, 0, 255, 255, 255, 0, 0, 255]);
    }

    #[test]
    fn fill_legend_renderer_matches_rect_style_selection() {
        let mark = CompiledCartesianUniformRaster2D {
            state: compiled_state(),
            options: raster_options("x", "y"),
        };
        let linear = LinearScale::configured((0.0, 1.0), (0.0, 1.0));
        let ordinal =
            OrdinalScale::configured(Arc::new(StringArray::from(vec!["a", "b"])) as ArrayRef);

        assert!(matches!(
            mark.preferred_legend_renderer(UNIFORM_RASTER_2D_FILL_CHANNEL, &linear),
            Some(LegendRendererSelection::BuiltIn(
                LegendRendererKind::Colorbar
            ))
        ));
        assert!(matches!(
            mark.preferred_legend_renderer(UNIFORM_RASTER_2D_FILL_CHANNEL, &ordinal),
            Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Rect))
        ));
    }

    #[test]
    fn rejects_nonlinear_sampling_in_phase_1() {
        let mark = CompiledCartesianUniformRaster2D {
            state: compiled_state(),
            options: raster_options("x", "y"),
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
