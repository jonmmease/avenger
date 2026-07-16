//! `UniformRaster2D<Geo>`: CRS-tagged dense rasters on the Geo coordinate
//! system.
//!
//! Rendering dispatches like the tile guide. On the identity fast path —
//! an unrotated Mercator projection with no active adaptive blend, and a
//! raster whose rows are uniform in mercator y (untagged raw units or
//! EPSG:3857) — the raster is an axis-aligned image positioned through the
//! coordinate-owned linear raw-unit scales (the same math as tile
//! placement). Everywhere else (rotations, Albers and other projections,
//! active blends, and EPSG:4326 grids, whose latitude-uniform rows are
//! never axis-affine on a Mercator screen) the raster renders as a
//! `SceneWarpedImageMark` through the measurement's view projector via
//! [`warped_raster_mesh`]. Both paths build the identical RGBA image
//! through the shared two-tier cache; only positioning differs.

use std::{collections::HashMap, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, ChannelValue, ColorChannelConfig, CompiledDataContext,
    CompiledMark, CompiledMarkCore, CompiledMarkState, CoordinateSystemTransformCore, IntoExpr,
    LegendRendererKind, LegendRendererSelection, Mark, MarkRuntimeContext, PointGeometry,
    RenderedMarkData, ResolvedDomain, ScaleRange, ScaleTypePreference, Theme,
    coerce_opacity_channel_with_renderer, default_scale_type_for_data_type, impl_mark_trait_common,
    is_continuous_scale,
};
use avenger_chart_marks::{
    RasterChannelsConfig, UNIFORM_RASTER_2D_FILL_CHANNEL, UNIFORM_RASTER_2D_RASTER_CHANNEL,
    UniformRaster2D, UniformRaster2DOptions,
    uniform_raster_2d::{
        GridRasterRow, RasterCoords, UNIFORM_RASTER_2D_OPACITY_BY_TOTAL_CHANNEL,
        UniformRasterImageCacheHandle, as_struct_array, build_or_reuse_mixed_rgba_image,
        build_or_reuse_rgba_image, channel_array, default_uniform_raster_image_cache,
        extract_grid_raster_row, list_row_values, option_color, raster_crs, required_position,
        resolve_overlay_inputs, row_for_channel, scale_numeric_extent, scene_image_mark,
        struct_child_struct,
    },
    uniform_raster_2d_channel_defaults,
};
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::marks::{
    image::SceneImageSource, mark::SceneMark, warped_image::SceneWarpedImageMark,
};
use datafusion::{
    arrow::{datatypes::DataType, record_batch::RecordBatch},
    common::ScalarValue,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::{
    Geo, crs,
    tiles::{RasterVAxis, is_identity_fast_path, warped_raster_mesh},
    view::GeoCoordMeasurement,
};

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<Geo> for UniformRaster2D<Geo> {
    impl_mark_trait_common!(UniformRaster2D);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledGeoUniformRaster2D {
            state: compiled_state,
            options: self.raster_options().clone(),
            image_cache: default_uniform_raster_image_cache(),
        }))
    }
}

/// Raster channel builders for `UniformRaster2D<Geo>`. Geo has no axes, so
/// the per-position axis configuration slot is `()`.
pub trait GeoUniformRaster2DChannels: Sized {
    fn raster_with<V, F>(self, data: V, f: F) -> Self
    where
        V: IntoExpr,
        F: FnOnce(RasterChannelsConfig<()>) -> RasterChannelsConfig<()>;
}

impl GeoUniformRaster2DChannels for UniformRaster2D<Geo> {
    fn raster_with<V, F>(self, data: V, f: F) -> Self
    where
        V: IntoExpr,
        F: FnOnce(RasterChannelsConfig<()>) -> RasterChannelsConfig<()>,
    {
        let raster_expr = data.into_expr();
        let fields = avenger_chart_marks::UniformRaster2DFields::new(raster_expr.clone());
        let config = RasterChannelsConfig::new(ColorChannelConfig::new(ChannelValue::from(
            fields.values_data(),
        )));
        let parts = f(config).into_parts();
        let x_position = parts.x.map(|x| x.take().0);
        let y_position = parts.y.map(|y| y.take().0);
        self.configure_raster_with_overlay(
            raster_expr,
            Some(parts.fill),
            Some((x_position, y_position)),
            parts.fill_by,
            parts.opacity_by_total,
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledGeoUniformRaster2D {
    pub(crate) state: CompiledMarkState,
    pub(crate) options: UniformRaster2DOptions,
    #[serde(skip, default = "default_uniform_raster_image_cache")]
    image_cache: UniformRasterImageCacheHandle,
}

impl CompiledMarkCore for CompiledGeoUniformRaster2D {
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
            ChannelDescriptor {
                name: UNIFORM_RASTER_2D_OPACITY_BY_TOTAL_CHANNEL,
                required: false,
                default_value: None,
                allow_column_ref: false,
            },
        ]
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        uniform_raster_2d_channel_defaults(channel)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        match channel {
            UNIFORM_RASTER_2D_RASTER_CHANNEL => None,
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
            // The density->alpha scale is an internal mapping, not a
            // legend-worthy channel.
            UNIFORM_RASTER_2D_OPACITY_BY_TOTAL_CHANNEL => None,
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

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl CompiledMark for CompiledGeoUniformRaster2D {
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

impl CompiledGeoUniformRaster2D {
    fn render_uniform_raster_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        let measurement =
            GeoCoordMeasurement::downcast(context.coord_measurement()).ok_or_else(|| {
                AvengerChartError::CoordinateSystemError(
                    "UniformRaster2D<Geo> expected a Geo coordinate measurement".to_string(),
                )
            })?;

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

        let x_position = required_position(&self.options.x_position, "x")?;
        let y_position = required_position(&self.options.y_position, "y")?;
        if x_position.dim.name() == y_position.dim.name() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "UniformRaster2D x and y dimensions must be distinct, got '{}'",
                x_position.dim.name()
            )));
        }

        debug!(
            mark_type = self.mark_type(),
            rows = len,
            smooth = self.options.smooth,
            identity = is_identity_fast_path(measurement),
            "rendering uniform raster rows on Geo"
        );

        let mut marks = Vec::new();
        let mut source_row_indices = Vec::new();
        for (row, opacity_value) in opacity_values.iter().copied().enumerate() {
            let raster_row =
                row_for_channel(raster_array, row, len, UNIFORM_RASTER_2D_RASTER_CHANNEL)?;
            let raster = extract_grid_raster_row(raster_array, raster_row)?;
            let geometry =
                struct_child_struct(as_struct_array(raster_array, "raster")?, "geometry")?;
            let frame = parse_raster_frame(raster_crs(geometry, raster_row).as_deref())?;

            // Resolve the image source: categorical overlay (fill_by) or the
            // scalar fill path. Overlay inputs borrow per row.
            let fill_values_storage;
            let overlay_storage;
            let image_spec = if let Some(fill_by) = &self.options.fill_by {
                let mix_dim = fill_by.dim.name();
                if mix_dim == x_position.dim.name() || mix_dim == y_position.dim.name() {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "UniformRaster2D fill_by dimension '{mix_dim}' cannot also be a position dimension"
                    )));
                }
                match resolve_overlay_inputs(
                    &raster,
                    fill_by,
                    self.options.opacity_by_total.is_some(),
                    context,
                )? {
                    Some(inputs) => {
                        overlay_storage = inputs;
                        RasterImageSpec::Mixed {
                            mix_dim,
                            category_colors: &overlay_storage.0,
                            opacity_scale: &overlay_storage.1,
                        }
                    }
                    // No observed categories: nothing to draw for this row.
                    None => continue,
                }
            } else {
                let fill_row =
                    row_for_channel(fill_array, row, len, UNIFORM_RASTER_2D_FILL_CHANNEL)?;
                raster.validate_scalar_render_dims(x_position.dim.name(), y_position.dim.name())?;
                fill_values_storage =
                    list_row_values(fill_array, fill_row, UNIFORM_RASTER_2D_FILL_CHANNEL)?;
                if fill_values_storage.len() != raster.values.len() {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "UniformRaster2D fill row {row} has {} cells, but raster values require {}",
                        fill_values_storage.len(),
                        raster.values.len()
                    )));
                }
                RasterImageSpec::Scalar {
                    fill_values: &fill_values_storage,
                    null_color,
                    non_finite_color,
                    opacity: opacity_value,
                }
            };

            // 4326 rows are latitude-uniform: never axis-affine on the
            // Mercator screen, so they always take the warped path.
            let identity = is_identity_fast_path(measurement) && frame != RasterFrame::Degrees4326;
            if !identity {
                if let Some(mark) = build_warped_raster_mark(
                    &raster,
                    &image_spec,
                    x_position,
                    y_position,
                    frame,
                    measurement,
                    self.options.smooth,
                    self.state.zindex,
                    &self.image_cache,
                    context,
                )? {
                    marks.push(mark);
                    source_row_indices.push(vec![row]);
                }
                continue;
            }

            let to_raw_units = match frame {
                RasterFrame::RawUnits => 1.0,
                RasterFrame::Meters3857 => 1.0 / crs::WEB_MERCATOR_RADIUS_M,
                // Unreachable: 4326 dispatched to the warped path above.
                RasterFrame::Degrees4326 => unreachable!("4326 rasters always warp"),
            };
            let mark = build_identity_raster_mark(
                &raster,
                &image_spec,
                x_position,
                y_position,
                to_raw_units,
                self.options.smooth,
                self.state.zindex,
                &self.image_cache,
                context,
                coord,
            )?;
            marks.push(mark);
            source_row_indices.push(vec![row]);
        }

        Ok(RenderedMarkData::with_source_row_indices(
            marks,
            source_row_indices,
        ))
    }
}

/// The raster's declared coordinate frame, from `geometry.crs`. Untagged
/// rasters carry authored-plane raw units.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RasterFrame {
    RawUnits,
    Meters3857,
    Degrees4326,
}

fn parse_raster_frame(crs_tag: Option<&str>) -> Result<RasterFrame, AvengerChartError> {
    match crs_tag {
        None => Ok(RasterFrame::RawUnits),
        Some(crs::EPSG_3857) => Ok(RasterFrame::Meters3857),
        Some(crs::EPSG_4326) => Ok(RasterFrame::Degrees4326),
        Some(other) => Err(AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D<Geo> raster declares unknown CRS '{other}'; supported: \
             untagged (raw units), '{}', or '{}'",
            crs::EPSG_3857,
            crs::EPSG_4326
        ))),
    }
}

/// Warped path: build the RGBA image exactly as the fast path does (image
/// row 0 = the raster's north/stop edge, matching mesh UV v = 0), then
/// position it through the measurement's view projector with
/// How a raster row's RGBA image is produced: the scalar fill path, or the
/// categorical Oklab overlay. Geometry (identity extent vs warped mesh) is
/// orthogonal to the image source.
enum RasterImageSpec<'a> {
    Scalar {
        fill_values: &'a datafusion::arrow::array::ArrayRef,
        null_color: [f32; 4],
        non_finite_color: [f32; 4],
        opacity: f32,
    },
    Mixed {
        mix_dim: &'a str,
        category_colors: &'a [(String, [f32; 4])],
        opacity_scale: &'a avenger_scales::scales::ConfiguredScale,
    },
}

impl RasterImageSpec<'_> {
    #[allow(clippy::too_many_arguments)]
    fn build(
        &self,
        raster: &GridRasterRow,
        x_dim: &str,
        y_dim: &str,
        x_indices: &[usize],
        y_indices: &[usize],
        flip_x: bool,
        flip_y: bool,
        image_cache: &UniformRasterImageCacheHandle,
    ) -> Result<std::sync::Arc<avenger_image::RgbaImage>, AvengerChartError> {
        match self {
            Self::Scalar {
                fill_values,
                null_color,
                non_finite_color,
                opacity,
            } => build_or_reuse_rgba_image(
                raster,
                fill_values,
                *null_color,
                *non_finite_color,
                *opacity,
                x_dim,
                y_dim,
                x_indices,
                y_indices,
                flip_x,
                flip_y,
                image_cache,
            ),
            Self::Mixed {
                mix_dim,
                category_colors,
                opacity_scale,
            } => build_or_reuse_mixed_rgba_image(
                raster,
                mix_dim,
                category_colors,
                opacity_scale,
                x_dim,
                y_dim,
                x_indices,
                y_indices,
                flip_x,
                flip_y,
                image_cache,
            ),
        }
    }
}

/// [`warped_raster_mesh`]. Returns `Ok(None)` when the mesh is fully
/// offscreen or degenerate.
#[allow(clippy::too_many_arguments)]
fn build_warped_raster_mark(
    raster: &GridRasterRow,
    image_spec: &RasterImageSpec<'_>,
    x_position: &avenger_chart_marks::RasterPositionSpec,
    y_position: &avenger_chart_marks::RasterPositionSpec,
    frame: RasterFrame,
    measurement: &GeoCoordMeasurement,
    smooth: bool,
    zindex: Option<i32>,
    image_cache: &UniformRasterImageCacheHandle,
    context: &dyn MarkRuntimeContext,
) -> Result<Option<SceneMark>, AvengerChartError> {
    let x_dim = raster.dimension(x_position.dim.name())?;
    let y_dim = raster.dimension(y_position.dim.name())?;

    let (
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
    ) = (&x_dim.coords, &y_dim.coords)
    else {
        return Err(AvengerChartError::InvalidArgument(
            "categorical raster dimensions are not supported on Geo yet".to_string(),
        ));
    };

    // Extents are ascending in every frame (validate_extent), so the stop
    // edge is east/north.
    let (lon_west, lon_east, v_axis) = match frame {
        RasterFrame::RawUnits => (
            x_start.to_degrees(),
            x_stop.to_degrees(),
            RasterVAxis::MercatorY {
                top: *y_stop,
                bottom: *y_start,
            },
        ),
        RasterFrame::Meters3857 => {
            let to_raw = 1.0 / crs::WEB_MERCATOR_RADIUS_M;
            (
                (x_start * to_raw).to_degrees(),
                (x_stop * to_raw).to_degrees(),
                RasterVAxis::MercatorY {
                    top: y_stop * to_raw,
                    bottom: y_start * to_raw,
                },
            )
        }
        RasterFrame::Degrees4326 => (
            *x_start,
            *x_stop,
            RasterVAxis::Latitude {
                north: *y_stop,
                south: *y_start,
            },
        ),
    };

    // Image row 0 must be the north edge (mesh v = 0 is the v_axis top),
    // and the raster's y dimension is ascending south -> north: flip rows.
    let x_indices = (0..*x_count as usize).collect::<Vec<_>>();
    let y_indices = (0..*y_count as usize).collect::<Vec<_>>();
    let image = image_spec.build(
        raster,
        x_dim.name.as_str(),
        y_dim.name.as_str(),
        &x_indices,
        &y_indices,
        false,
        true,
        image_cache,
    )?;

    let projector = measurement.view_projector();
    let Some(mesh) = warped_raster_mesh(
        lon_west,
        lon_east,
        v_axis,
        &projector,
        measurement.projection.precision,
        context.plot_width(),
        context.plot_height(),
    ) else {
        return Ok(None);
    };

    Ok(Some(
        SceneWarpedImageMark {
            name: "uniform_raster_2d".to_string(),
            interactive: false,
            clip: true,
            smooth,
            image: SceneImageSource::shared_inline(image),
            positions: mesh.positions,
            uvs: mesh.uvs,
            indices: mesh.indices,
            unavailable_policy: Default::default(),
            zindex,
            tile_texture_size: None,
        }
        .into(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn build_identity_raster_mark(
    raster: &GridRasterRow,
    image_spec: &RasterImageSpec<'_>,
    x_position: &avenger_chart_marks::RasterPositionSpec,
    y_position: &avenger_chart_marks::RasterPositionSpec,
    to_raw_units: f64,
    smooth: bool,
    zindex: Option<i32>,
    image_cache: &UniformRasterImageCacheHandle,
    context: &dyn MarkRuntimeContext,
    coord: &dyn CoordinateSystemTransformCore,
) -> Result<SceneMark, AvengerChartError> {
    let x_dim = raster.dimension(x_position.dim.name())?;
    let y_dim = raster.dimension(y_position.dim.name())?;

    let (
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
    ) = (&x_dim.coords, &y_dim.coords)
    else {
        return Err(AvengerChartError::InvalidArgument(
            "categorical raster dimensions are not supported on Geo yet".to_string(),
        ));
    };

    let (scaled_x0, scaled_x1) = scale_numeric_extent(
        context,
        "x",
        &x_position.channel_value,
        x_start * to_raw_units,
        x_stop * to_raw_units,
    )?;
    let (scaled_y0, scaled_y1) = scale_numeric_extent(
        context,
        "y",
        &y_position.channel_value,
        y_start * to_raw_units,
        y_stop * to_raw_units,
    )?;
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
    let image = image_spec.build(
        raster,
        x_dim.name.as_str(),
        y_dim.name.as_str(),
        &x_indices,
        &y_indices,
        flip_x,
        flip_y,
        image_cache,
    )?;

    Ok(scene_image_mark(image, x, y, width, height, smooth, zindex))
}

fn transform_extent(
    coord: &dyn CoordinateSystemTransformCore,
    context: &dyn MarkRuntimeContext,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
) -> Result<(f32, f32, f32, f32), AvengerChartError> {
    let mut position_channels = HashMap::new();
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
                "UniformRaster2D<Geo> expected point geometry from the coordinate transform"
                    .to_string(),
            )
        })?;
    let x = point.x.as_vec(2, None);
    let y = point.y.as_vec(2, None);
    Ok((x[0], y[0], x[1], y[1]))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use avenger_chart_core::{
        CoordMeasurement, CoordinateMeasureRequest, CoordinateMeasurementProvider,
        EvaluationContext, FacetDataScope, MarkDataMode, MarkRenderContext,
    };
    use avenger_chart_marks::{RasterPositionSpec, dim};
    use avenger_scales::scales::linear::LinearScale;
    use avenger_scenegraph::marks::image::SceneImageSource;
    use datafusion::{
        arrow::{
            array::{
                ArrayRef, Float64Array, ListArray, ListBuilder, StringArray, StringBuilder,
                StructArray, UInt32Array,
            },
            buffer::OffsetBuffer,
            datatypes::{Field, Schema},
        },
        prelude::SessionContext,
    };
    use indexmap::IndexMap;

    use crate::coord::BlendConfig;

    use super::*;

    struct TestRuntimeContext {
        eval: EvaluationContext,
        measurement: Box<dyn CoordMeasurement>,
        scales: HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
    }

    impl MarkRuntimeContext for TestRuntimeContext {
        fn core_view(&self) -> MarkRenderContext<'_> {
            MarkRenderContext::new(&self.eval, self.plot_width, self.plot_height)
        }

        fn coord_measurement(&self) -> &dyn CoordMeasurement {
            self.measurement.as_ref()
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

    async fn measurement_for(
        geo: &Geo,
        params: &IndexMap<String, ScalarValue>,
    ) -> Box<dyn CoordMeasurement> {
        let ctx = SessionContext::new();
        let request = CoordinateMeasureRequest {
            plot_width: 100.0,
            plot_height: 100.0,
            params,
            session_context: &ctx,
            data: None,
            compiled_marks: &[],
            facet_path: &[],
            scales: HashMap::new(),
        };
        geo.measure_coordinate(request)
            .await
            .expect("measure")
            .expect("measurement")
    }

    fn pinned_view_params(geo: &Geo) -> IndexMap<String, ScalarValue> {
        // 0.01 raw units/px over a 100x100 plot centered at (1.5, 0.5):
        // x domain [1.0, 2.0], y domain [0.0, 1.0].
        let mut params = IndexMap::new();
        params.insert(geo.center_x_param(), ScalarValue::Float64(Some(1.5)));
        params.insert(geo.center_y_param(), ScalarValue::Float64(Some(0.5)));
        params.insert(
            geo.units_per_pixel_param(),
            ScalarValue::Float64(Some(0.01)),
        );
        params
    }

    /// Context matching the pinned view: coordinate-owned linear raw-unit
    /// scales over the same domains the measurement realizes.
    async fn pinned_context(geo: &Geo) -> TestRuntimeContext {
        let params = pinned_view_params(geo);
        let measurement = measurement_for(geo, &params).await;
        let mut scales = HashMap::new();
        scales.insert(
            "x".to_string(),
            LinearScale::configured((1.0, 2.0), (0.0, 100.0)),
        );
        scales.insert(
            "y".to_string(),
            LinearScale::configured((0.0, 1.0), (100.0, 0.0)),
        );
        TestRuntimeContext {
            eval: EvaluationContext::new(
                Arc::new(Theme::light()),
                Arc::new(SessionContext::new()),
                Default::default(),
            ),
            measurement,
            scales,
            plot_width: 100.0,
            plot_height: 100.0,
        }
    }

    fn compiled_state() -> CompiledMarkState {
        CompiledMarkState {
            id: None,
            identity: Default::default(),
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

    fn position(dim_name: &str, channel: &str) -> RasterPositionSpec {
        RasterPositionSpec {
            dim: dim(dim_name),
            channel_value: ChannelValue::from(0.0).with_scale_name(channel),
        }
    }

    fn raster_options() -> UniformRaster2DOptions {
        UniformRaster2DOptions {
            x_position: Some(position("x", "x")),
            y_position: Some(position("y", "y")),
            ..Default::default()
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

    enum TestDim<'a> {
        Uniform(&'a str, f64, f64, u32),
        Categorical(&'a str, &'a [&'a str]),
    }

    fn dimensions_list(dims: &[TestDim<'_>]) -> ArrayRef {
        let names: Vec<&str> = dims
            .iter()
            .map(|dim| match dim {
                TestDim::Uniform(name, ..) | TestDim::Categorical(name, _) => *name,
            })
            .collect();
        let kinds: Vec<&str> = dims
            .iter()
            .map(|dim| match dim {
                TestDim::Uniform(..) => "uniform",
                TestDim::Categorical(..) => "categorical",
            })
            .collect();
        let starts: Vec<Option<f64>> = dims
            .iter()
            .map(|dim| match dim {
                TestDim::Uniform(_, start, ..) => Some(*start),
                TestDim::Categorical(..) => None,
            })
            .collect();
        let stops: Vec<Option<f64>> = dims
            .iter()
            .map(|dim| match dim {
                TestDim::Uniform(_, _, stop, _) => Some(*stop),
                TestDim::Categorical(..) => None,
            })
            .collect();
        let counts: Vec<Option<u32>> = dims
            .iter()
            .map(|dim| match dim {
                TestDim::Uniform(_, _, _, count) => Some(*count),
                TestDim::Categorical(..) => None,
            })
            .collect();
        let mut coord_values_builder = ListBuilder::new(StringBuilder::new());
        for dim in dims {
            match dim {
                TestDim::Uniform(..) => coord_values_builder.append(false),
                TestDim::Categorical(_, values) => {
                    for value in *values {
                        coord_values_builder.values().append_value(*value);
                    }
                    coord_values_builder.append(true);
                }
            }
        }
        let coord_values = Arc::new(coord_values_builder.finish()) as ArrayRef;
        let samplings: Vec<Option<&str>> = vec![None; dims.len()];
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

    fn raster_batch(
        dims: &[TestDim<'_>],
        values_dims: &[&str],
        cells: &[&str],
        crs: Option<&str>,
    ) -> RecordBatch {
        let values_data = string_list(cells);
        let dimensions = dimensions_list(dims);
        let mut geometry_fields = vec![(
            Arc::new(Field::new("kind", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["grid"])) as ArrayRef,
        )];
        if crs.is_some() {
            geometry_fields.push((
                Arc::new(Field::new("crs", DataType::Utf8, true)),
                Arc::new(StringArray::from(vec![crs])) as ArrayRef,
            ));
        }
        geometry_fields.push((
            Arc::new(Field::new(
                "dimensions",
                dimensions.data_type().clone(),
                false,
            )),
            dimensions,
        ));
        let geometry = Arc::new(StructArray::from(geometry_fields)) as ArrayRef;
        let dims_list = string_list(values_dims);
        let values = Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new("dims", dims_list.data_type().clone(), false)),
                dims_list,
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
        RecordBatch::try_from_iter(vec![
            (UNIFORM_RASTER_2D_RASTER_CHANNEL, raster),
            (UNIFORM_RASTER_2D_FILL_CHANNEL, values_data),
        ])
        .expect("record batch")
    }

    fn four_by_four_cells() -> Vec<&'static str> {
        vec!["#ff0000"; 16]
    }

    fn render(
        data: RecordBatch,
        context: &TestRuntimeContext,
        geo: &Geo,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mark = CompiledGeoUniformRaster2D {
            state: compiled_state(),
            options: raster_options(),
            image_cache: default_uniform_raster_image_cache(),
        };
        let scalars = RecordBatch::new_empty(Arc::new(Schema::empty()));
        mark.render_uniform_raster_mark_data(Some(&data), &scalars, context, geo)
            .map(|rendered| rendered.marks)
    }

    fn image_rect(mark: &SceneMark) -> (f32, f32, f32, f32, u32, u32) {
        let SceneMark::Image(image) = mark else {
            panic!("expected SceneImageMark, got {mark:?}");
        };
        let source = image.image.as_vec(1, None).remove(0);
        let SceneImageSource::SharedInline(rgba) = source else {
            panic!("expected shared inline image");
        };
        (
            image.x.as_vec(1, None)[0],
            image.y.as_vec(1, None)[0],
            image.width.as_vec(1, None)[0],
            image.height.as_vec(1, None)[0],
            rgba.width,
            rgba.height,
        )
    }

    fn assert_px(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "expected {expected}px, got {actual}px"
        );
    }

    /// A 3857-framed raster on the pinned Mercator view lands exactly where
    /// the tile placement math puts the same raw-unit rect:
    /// px = (left − x0)/(x1 − x0)·w, py = (y1 − top)/(y1 − y0)·h.
    #[tokio::test]
    async fn epsg_3857_raster_matches_tile_pixel_math() {
        let geo = Geo::mercator();
        let context = pinned_context(&geo).await;
        let r = crs::WEB_MERCATOR_RADIUS_M;
        // Raw-unit extents [1.2, 1.6] x [0.2, 0.8], declared in 3857 meters.
        let data = raster_batch(
            &[
                TestDim::Uniform("x", 1.2 * r, 1.6 * r, 4),
                TestDim::Uniform("y", 0.2 * r, 0.8 * r, 4),
            ],
            &["y", "x"],
            &four_by_four_cells(),
            Some(crs::EPSG_3857),
        );
        let marks = render(data, &context, &geo).expect("render");
        assert_eq!(marks.len(), 1);
        let (x, y, width, height, image_width, image_height) = image_rect(&marks[0]);

        // Tile-math expectations on x domain [1.0, 2.0], y domain [0.0, 1.0],
        // 100x100 px.
        let expected_x = (1.2 - 1.0) / (2.0 - 1.0) * 100.0;
        let expected_y = (1.0 - 0.8) / (1.0 - 0.0) * 100.0;
        let expected_width = (1.6 - 1.2) / (2.0 - 1.0) * 100.0;
        let expected_height = (0.8 - 0.2) / (1.0 - 0.0) * 100.0;
        assert_px(x, expected_x as f32);
        assert_px(y, expected_y as f32);
        assert_px(width, expected_width as f32);
        assert_px(height, expected_height as f32);
        assert_eq!((image_width, image_height), (4, 4));
    }

    /// Untagged rasters are authored raw units already — same placement with
    /// no conversion.
    #[tokio::test]
    async fn untagged_raster_renders_in_raw_units() {
        let geo = Geo::mercator();
        let context = pinned_context(&geo).await;
        let data = raster_batch(
            &[
                TestDim::Uniform("x", 1.2, 1.6, 4),
                TestDim::Uniform("y", 0.2, 0.8, 4),
            ],
            &["y", "x"],
            &four_by_four_cells(),
            None,
        );
        let marks = render(data, &context, &geo).expect("render");
        let (x, y, width, height, _, _) = image_rect(&marks[0]);
        assert_px(x, 20.0);
        assert_px(y, 20.0);
        assert_px(width, 40.0);
        assert_px(height, 60.0);
    }

    fn warped_mark(mark: &SceneMark) -> &SceneWarpedImageMark {
        let SceneMark::WarpedImage(warped) = mark else {
            panic!("expected SceneWarpedImageMark, got {mark:?}");
        };
        warped
    }

    /// 4326 rows are latitude-uniform, so even an identity-Mercator view
    /// takes the warped path.
    #[tokio::test]
    async fn epsg_4326_raster_takes_warped_path_on_identity_mercator() {
        let geo = Geo::mercator();
        let context = pinned_context(&geo).await;
        let data = raster_batch(
            &[
                TestDim::Uniform("x", 60.0, 100.0, 4),
                TestDim::Uniform("y", 10.0, 40.0, 4),
            ],
            &["y", "x"],
            &four_by_four_cells(),
            Some(crs::EPSG_4326),
        );
        let marks = render(data, &context, &geo).expect("render 4326 raster");
        assert_eq!(marks.len(), 1);
        let warped = warped_mark(&marks[0]);
        assert!(!warped.indices.is_empty());
        assert!(warped.tile_texture_size.is_none());
    }

    #[tokio::test]
    async fn unknown_crs_raster_errors() {
        let geo = Geo::mercator();
        let context = pinned_context(&geo).await;
        let data = raster_batch(
            &[
                TestDim::Uniform("x", 0.0, 1.0, 4),
                TestDim::Uniform("y", 0.0, 1.0, 4),
            ],
            &["y", "x"],
            &four_by_four_cells(),
            Some("epsg:32618"),
        );
        let err = render(data, &context, &geo).expect_err("unknown CRS must error");
        assert!(
            err.to_string().contains("unknown CRS 'epsg:32618'"),
            "{err}"
        );
    }

    /// Corner orientation on the warped path: with an asymmetric 2×2
    /// raster, mesh UV (0,0) must project to the raster's WEST/NORTH
    /// corner and the image's top-left pixel must hold the west/north
    /// cell's color (image rows flipped so row 0 = the north edge).
    #[tokio::test]
    async fn albers_warped_corners_land_on_projected_positions() {
        let geo = Geo::albers_usa_conus();
        let params = IndexMap::new();
        let measurement = measurement_for(&geo, &params).await;
        let mut context = pinned_context(&Geo::mercator()).await;
        context.measurement = measurement;
        // Raw-unit extents over the central US.
        let (x_start, x_stop) = (-2.0_f64, -1.5_f64);
        let (y_start, y_stop) = (0.6_f64, 0.85_f64);
        // dims [y, x] row-major: [SW, SE, NW, NE].
        let data = raster_batch(
            &[
                TestDim::Uniform("x", x_start, x_stop, 2),
                TestDim::Uniform("y", y_start, y_stop, 2),
            ],
            &["y", "x"],
            &["#ff0000", "#00ff00", "#0000ff", "#ffffff"],
            None,
        );
        let marks = render(data, &context, &geo).expect("render warped raster");
        assert_eq!(marks.len(), 1);
        let warped = warped_mark(&marks[0]);

        // Image rows: top = north.
        let SceneImageSource::SharedInline(rgba) = &warped.image else {
            panic!("expected shared inline image");
        };
        assert_eq!((rgba.width, rgba.height), (2, 2));
        let pixel = |px: u32, py: u32| {
            let offset = ((py * rgba.width + px) * 4) as usize;
            [
                rgba.data[offset],
                rgba.data[offset + 1],
                rgba.data[offset + 2],
                rgba.data[offset + 3],
            ]
        };
        assert_eq!(pixel(0, 0), [0, 0, 255, 255], "top-left = NW = blue");
        assert_eq!(pixel(1, 0), [255, 255, 255, 255], "top-right = NE = white");
        assert_eq!(pixel(0, 1), [255, 0, 0, 255], "bottom-left = SW = red");
        assert_eq!(pixel(1, 1), [0, 255, 0, 255], "bottom-right = SE = green");

        // Mesh corners: UV (0,0) is the west/north corner, UV (1,1) the
        // east/south corner, both exactly where the view projector puts
        // them.
        let projector = GeoCoordMeasurement::downcast(context.measurement.as_ref())
            .expect("geo measurement")
            .view_projector();
        let north_lat = y_stop.sinh().atan().to_degrees();
        let south_lat = y_start.sinh().atan().to_degrees();
        let corner = |target_uv: [f32; 2]| {
            let index = warped
                .uvs
                .iter()
                .position(|uv| *uv == target_uv)
                .unwrap_or_else(|| panic!("no vertex at uv {target_uv:?}"));
            warped.positions[index]
        };
        let expected_nw = projector
            .project(x_start.to_degrees(), north_lat)
            .expect("project NW corner");
        let actual_nw = corner([0.0, 0.0]);
        assert!(
            (f64::from(actual_nw[0]) - expected_nw.0).abs() < 1e-2
                && (f64::from(actual_nw[1]) - expected_nw.1).abs() < 1e-2,
            "NW corner {actual_nw:?} vs projected {expected_nw:?}"
        );
        let expected_se = projector
            .project(x_stop.to_degrees(), south_lat)
            .expect("project SE corner");
        let actual_se = corner([1.0, 1.0]);
        assert!(
            (f64::from(actual_se[0]) - expected_se.0).abs() < 1e-2
                && (f64::from(actual_se[1]) - expected_se.1).abs() < 1e-2,
            "SE corner {actual_se:?} vs projected {expected_se:?}"
        );
    }

    /// An active blend leaves the authored plane: the raster must follow
    /// the blended view projector (warped path), not the scales.
    #[tokio::test]
    async fn active_blend_takes_warped_path() {
        let geo = Geo::mercator().adaptive_blend(BlendConfig {
            z0: 0.0,
            z1: 1.0,
            force_t: Some(1.0),
        });
        let params = pinned_view_params(&geo);
        let measurement = measurement_for(&geo, &params).await;
        let mut context = pinned_context(&Geo::mercator()).await;
        context.measurement = measurement;
        let data = raster_batch(
            &[
                TestDim::Uniform("x", 1.2, 1.6, 4),
                TestDim::Uniform("y", 0.2, 0.8, 4),
            ],
            &["y", "x"],
            &four_by_four_cells(),
            None,
        );
        let marks = render(data, &context, &geo).expect("render under active blend");
        assert_eq!(marks.len(), 1);
        assert!(!warped_mark(&marks[0]).indices.is_empty());
    }

    #[tokio::test]
    async fn categorical_dimensions_error() {
        let geo = Geo::mercator();
        let context = pinned_context(&geo).await;
        let data = raster_batch(
            &[
                TestDim::Uniform("x", 1.2, 1.6, 2),
                TestDim::Categorical("y", &["A", "B"]),
            ],
            &["y", "x"],
            &["#ff0000", "#00ff00", "#0000ff", "#ffffff"],
            None,
        );
        let err = render(data, &context, &geo).expect_err("categorical dims must error");
        assert!(
            err.to_string()
                .contains("categorical raster dimensions are not supported on Geo yet"),
            "{err}"
        );
    }
}
