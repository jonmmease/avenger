use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::{Hash, Hasher},
    sync::{Arc, Mutex},
    time::Instant,
};

use datafusion::{
    arrow::{
        array::{Array, ArrayData, ArrayRef, AsArray, Float64Array, StringArray, StructArray},
        datatypes::{DataType, Float32Type, Float64Type},
        record_batch::RecordBatch,
    },
    logical_expr::Expr,
    prelude::get_field,
};
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};
use tracing::debug;

use avenger_chart_core::{
    AvengerChartError, Axis, ChannelConfig, ChannelValue, ColorChannelConfig, CoordinationScope,
    MarkRenderContext, MarkRuntimeContext, MarkState, OpacityChannelConfig, PrimitiveMarkEffects,
    Scale, ScaleChannelValue, ScaleSpec, define_common_mark_channels,
    impl_mark_base_with_extra_fields,
};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{ImageAlign, ImageBaseline},
    value::ScalarOrArray,
};
use avenger_image::RgbaImage;
use avenger_scales::scales::coerce::Coercer;
use avenger_scenegraph::marks::{
    image::{SceneImageMark, SceneImageSource},
    mark::SceneMark,
};

pub use avenger_chart_core::{RasterDim, dim};

pub const UNIFORM_RASTER_2D_RASTER_CHANNEL: &str = "raster";
pub const UNIFORM_RASTER_2D_FILL_CHANNEL: &str = "fill";

pub struct UniformRaster2D<C> {
    pub(crate) state: MarkState,
    pub(crate) effects: PrimitiveMarkEffects,
    pub(crate) options: UniformRaster2DOptions,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base_with_extra_fields!(UniformRaster2D {
    effects: PrimitiveMarkEffects::default(),
    options: UniformRaster2DOptions::default(),
});

impl<C> UniformRaster2D<C> {
    pub fn null_color(mut self, color: impl Into<ChannelValue>) -> Self {
        self.options.null_color = color.into();
        self
    }

    pub fn non_finite_color(mut self, color: impl Into<ChannelValue>) -> Self {
        self.options.non_finite_color = color.into();
        self
    }

    pub fn smooth(mut self, smooth: bool) -> Self {
        self.options.smooth = smooth;
        self
    }

    #[doc(hidden)]
    pub fn raster_options(&self) -> &UniformRaster2DOptions {
        &self.options
    }

    #[doc(hidden)]
    pub fn mark_effects(&self) -> &PrimitiveMarkEffects {
        &self.effects
    }

    #[doc(hidden)]
    pub fn configure_raster(
        mut self,
        raster_expr: Expr,
        fill: Option<ChannelValue>,
        positions: Option<(Option<RasterPositionSpec>, Option<RasterPositionSpec>)>,
    ) -> Self {
        let fields = UniformRaster2DFields::new(raster_expr.clone());
        let fill = fill.unwrap_or_else(|| ChannelValue::from(fields.values_data()));
        if let Some((x_position, y_position)) = positions {
            self.options.x_position = x_position;
            self.options.y_position = y_position;
        }
        self.with_channel_value(
            UNIFORM_RASTER_2D_RASTER_CHANNEL,
            ChannelValue::from(raster_expr).no_scale(),
        )
        .with_channel_value(UNIFORM_RASTER_2D_FILL_CHANNEL, fill)
    }
}

define_common_mark_channels! {
    UniformRaster2D {
        opacity: {
            allow_column: true,
            with_config: OpacityChannelConfig,
        },
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct UniformRaster2DOptions {
    pub null_color: ChannelValue,
    pub non_finite_color: ChannelValue,
    pub x_position: Option<RasterPositionSpec>,
    pub y_position: Option<RasterPositionSpec>,
    pub smooth: bool,
}

impl Default for UniformRaster2DOptions {
    fn default() -> Self {
        Self {
            null_color: ChannelValue::from("#00000000"),
            non_finite_color: ChannelValue::from("#00000000"),
            x_position: None,
            y_position: None,
            smooth: false,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RasterPositionSpec {
    pub dim: RasterDim,
    pub channel_value: ChannelValue,
}

pub struct RasterChannelsConfig<A: Clone + Default + Send + Sync + 'static> {
    fill: ColorChannelConfig,
    x: Option<RasterPositionConfig<A>>,
    y: Option<RasterPositionConfig<A>>,
}

impl<A: Clone + Default + Send + Sync + 'static> RasterChannelsConfig<A> {
    pub fn new(fill: ColorChannelConfig) -> Self {
        Self {
            fill,
            x: None,
            y: None,
        }
    }

    pub fn fill<F>(mut self, f: F) -> Self
    where
        F: FnOnce(ColorChannelConfig) -> ColorChannelConfig,
    {
        self.fill = f(self.fill);
        self
    }

    pub fn x(mut self, dim: RasterDim) -> Self
    where
        A: Axis + Default,
    {
        self.x = Some(RasterPositionConfig::new(dim, "x"));
        self
    }

    pub fn x_with<F>(mut self, dim: RasterDim, f: F) -> Self
    where
        A: Axis + Default,
        F: FnOnce(RasterPositionConfig<A>) -> RasterPositionConfig<A>,
    {
        self.x = Some(f(RasterPositionConfig::new(dim, "x")));
        self
    }

    pub fn y(mut self, dim: RasterDim) -> Self
    where
        A: Axis + Default,
    {
        self.y = Some(RasterPositionConfig::new(dim, "y"));
        self
    }

    pub fn y_with<F>(mut self, dim: RasterDim, f: F) -> Self
    where
        A: Axis + Default,
        F: FnOnce(RasterPositionConfig<A>) -> RasterPositionConfig<A>,
    {
        self.y = Some(f(RasterPositionConfig::new(dim, "y")));
        self
    }

    #[doc(hidden)]
    pub fn into_parts(
        self,
    ) -> (
        ChannelValue,
        Option<RasterPositionConfig<A>>,
        Option<RasterPositionConfig<A>>,
    ) {
        (self.fill.into_inner(), self.x, self.y)
    }
}

#[derive(Clone)]
pub struct RasterPositionConfig<A: Clone + Default + Send + Sync + 'static> {
    dim: RasterDim,
    channel_value: ChannelValue,
    axis_config: Option<A>,
}

impl<A: Clone + Default + Send + Sync + 'static> RasterPositionConfig<A> {
    pub fn new(dim: RasterDim, channel: &str) -> Self {
        Self {
            dim,
            channel_value: ChannelValue::from(0.0).with_scale_name(channel),
            axis_config: None,
        }
    }

    pub fn axis<F>(mut self, f: F) -> Self
    where
        F: FnOnce(A) -> A,
    {
        let axis = self.axis_config.take().unwrap_or_default();
        self.axis_config = Some(f(axis));
        self
    }

    pub fn with_scale_name(mut self, name: impl Into<String>) -> Self {
        self.channel_value = self.channel_value.with_scale_name(name);
        self
    }

    pub fn scale<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale<avenger_chart_core::Auto>) -> Scale<avenger_chart_core::Auto>
            + Send
            + Sync
            + 'static,
    {
        self.channel_value = self.channel_value.scale(f);
        self
    }

    pub fn scale_with<S: ScaleSpec + Default>(
        mut self,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        self.channel_value = self.channel_value.scale_with::<S>(f);
        self
    }

    pub fn with_domain_scope(mut self, scope: CoordinationScope) -> Self {
        self.channel_value = self.channel_value.with_domain_scope(scope);
        self
    }

    pub fn share_domain(self) -> Self {
        self.with_domain_scope(CoordinationScope::Shared)
    }

    pub fn free_domain(self) -> Self {
        self.with_domain_scope(CoordinationScope::Free)
    }

    #[doc(hidden)]
    pub fn take(self) -> (RasterPositionSpec, Option<A>) {
        (
            RasterPositionSpec {
                dim: self.dim,
                channel_value: self.channel_value,
            },
            self.axis_config,
        )
    }
}

#[derive(Clone)]
pub struct UniformRaster2DFields {
    expr: Expr,
}

impl UniformRaster2DFields {
    pub fn new(expr: Expr) -> Self {
        Self { expr }
    }

    pub fn expr_ref(&self) -> &Expr {
        &self.expr
    }

    pub fn into_expr(self) -> Expr {
        self.expr
    }

    pub fn geometry(&self) -> Expr {
        field(self.expr.clone(), "geometry")
    }

    pub fn values(&self) -> Expr {
        field(self.expr.clone(), "values")
    }

    pub fn geometry_kind(&self) -> Expr {
        field(self.geometry(), "kind")
    }

    pub fn dimensions(&self) -> Expr {
        field(self.geometry(), "dimensions")
    }

    pub fn values_dims(&self) -> Expr {
        field(self.values(), "dims")
    }

    pub fn values_data(&self) -> Expr {
        field(self.values(), "data")
    }
}

fn field(expr: Expr, name: &str) -> Expr {
    get_field(expr, name.to_string())
}

pub fn uniform_raster_2d_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Coordinate-agnostic rendering core shared by the per-coordinate-system
// `Mark` implementations: raster struct readers, the RGBA image builder and
// its two-tier cache, and the `SceneImageMark` constructor. Everything here
// works in scaled pixel space; scaling and coordinate transforms stay with
// the coordinate-system crates.
// ---------------------------------------------------------------------------

pub type UniformRasterImageCacheHandle = Arc<Mutex<UniformRasterImageCache>>;

pub fn default_uniform_raster_image_cache() -> UniformRasterImageCacheHandle {
    Arc::new(Mutex::new(UniformRasterImageCache::default()))
}

const UNIFORM_RASTER_IMAGE_CACHE_CAPACITY: usize = 16;

/// Identity cache entry: pins the source arrays so their buffer allocations
/// cannot be freed and reused while the entry lives. Buffer addresses are
/// only meaningful as identity while the referenced allocation is alive —
/// without pinning, a different raster allocated at a recycled address
/// produces a false identity hit and the wrong image is served (the
/// mechanism behind the flaky faceted raster visual tests).
struct UniformRasterIdentityCachedImage {
    raster_values: ArrayRef,
    fill_values: ArrayRef,
    image: Arc<RgbaImage>,
}

#[derive(Default)]
pub struct UniformRasterImageCache {
    entries: HashMap<UniformRasterImageKey, Arc<RgbaImage>>,
    order: VecDeque<UniformRasterImageKey>,
    identity_entries: HashMap<UniformRasterImageIdentityKey, UniformRasterIdentityCachedImage>,
    identity_order: VecDeque<UniformRasterImageIdentityKey>,
}

impl UniformRasterImageCache {
    fn get(&self, key: &UniformRasterImageKey) -> Option<Arc<RgbaImage>> {
        self.entries.get(key).cloned()
    }

    fn get_identity(
        &self,
        key: &UniformRasterImageIdentityKey,
        raster_values: &ArrayRef,
        fill_values: &ArrayRef,
    ) -> Option<Arc<RgbaImage>> {
        let entry = self.identity_entries.get(key)?;
        // Verify true pointer identity against the pinned arrays; a hash
        // collision or any drift must fall through to the content path.
        if arrays_share_identity(&entry.raster_values, raster_values)
            && arrays_share_identity(&entry.fill_values, fill_values)
        {
            Some(entry.image.clone())
        } else {
            None
        }
    }

    fn insert_identity(
        &mut self,
        key: UniformRasterImageIdentityKey,
        raster_values: ArrayRef,
        fill_values: ArrayRef,
        image: Arc<RgbaImage>,
    ) {
        let entry = UniformRasterIdentityCachedImage {
            raster_values,
            fill_values,
            image,
        };
        if self.identity_entries.contains_key(&key) {
            self.identity_entries.insert(key, entry);
            return;
        }

        self.identity_entries.insert(key.clone(), entry);
        self.identity_order.push_back(key);

        while self.identity_entries.len() > UNIFORM_RASTER_IMAGE_CACHE_CAPACITY {
            if let Some(oldest) = self.identity_order.pop_front() {
                self.identity_entries.remove(&oldest);
            } else {
                break;
            }
        }
    }

    fn insert(&mut self, key: UniformRasterImageKey, image: Arc<RgbaImage>) {
        if self.entries.contains_key(&key) {
            self.entries.insert(key, image);
            return;
        }

        self.entries.insert(key.clone(), image);
        self.order.push_back(key);

        while self.entries.len() > UNIFORM_RASTER_IMAGE_CACHE_CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            } else {
                break;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct UniformRasterImageIdentityKey {
    raster_values_identity: u64,
    fill_values_identity: u64,
    x_dim: String,
    y_dim: String,
    x_indices_hash: u64,
    y_indices_hash: u64,
    flip_x: bool,
    flip_y: bool,
    opacity: u32,
    null_color: [u32; 4],
    non_finite_color: [u32; 4],
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct UniformRasterImageKey {
    raster_values_hash: u64,
    fill_values_hash: u64,
    x_dim: String,
    y_dim: String,
    x_indices_hash: u64,
    y_indices_hash: u64,
    flip_x: bool,
    flip_y: bool,
    opacity: u32,
    null_color: [u32; 4],
    non_finite_color: [u32; 4],
}

pub fn scene_image_mark(
    image: Arc<RgbaImage>,
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
        image: ScalarOrArray::new_scalar(SceneImageSource::shared_inline(image)),
        x: ScalarOrArray::new_scalar(x),
        y: ScalarOrArray::new_scalar(y),
        width: ScalarOrArray::new_scalar(width),
        height: ScalarOrArray::new_scalar(height),
        align: ScalarOrArray::new_scalar(ImageAlign::Left),
        baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
        unavailable_policy: Default::default(),
        indices: None,
        zindex,
        tile_texture_size: None,
    }
    .into()
}

#[derive(Debug)]
pub struct GridRasterRow {
    pub dimensions: Vec<RasterDimension>,
    pub values_dims: Vec<String>,
    pub strides: HashMap<String, usize>,
    pub values: ArrayRef,
}

#[derive(Debug)]
pub struct RasterDimension {
    pub name: String,
    pub coords: RasterCoords,
}

#[derive(Debug)]
pub enum RasterCoords {
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
    pub fn len(&self) -> usize {
        match self {
            Self::Uniform { count, .. } => *count as usize,
            Self::Categorical { values } => values.len(),
        }
    }
}

impl GridRasterRow {
    pub fn dimension(&self, name: &str) -> Result<&RasterDimension, AvengerChartError> {
        self.dimensions
            .iter()
            .find(|dimension| dimension.name == name)
            .ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "UniformRaster2D raster is missing dimension '{name}'"
                ))
            })
    }

    pub fn validate_scalar_render_dims(
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

    pub fn cell_index(
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

pub fn channel_array<'a>(
    data: Option<&'a RecordBatch>,
    scalars: &'a RecordBatch,
    channel: &str,
) -> Result<&'a ArrayRef, AvengerChartError> {
    data.and_then(|batch| batch.column_by_name(channel))
        .or_else(|| scalars.column_by_name(channel))
        .ok_or_else(|| AvengerChartError::MissingChannelError(channel.to_string()))
}

pub fn row_for_channel(
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

pub fn extract_grid_raster_row(
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

/// Read the raster's declared CRS tag from the `geometry` struct.
///
/// Returns `None` when the `crs` field is absent (rasters serialized before the
/// field existed keep working) or null (untagged raster — chart data units).
pub fn raster_crs(geometry: &StructArray, row: usize) -> Option<String> {
    let crs = geometry.column_by_name("crs")?;
    let crs = crs.as_any().downcast_ref::<StringArray>()?;
    if crs.is_null(row) {
        None
    } else {
        Some(crs.value(row).to_string())
    }
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

pub fn as_struct_array<'a>(
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

pub fn struct_child<'a>(
    array: &'a StructArray,
    name: &str,
) -> Result<&'a ArrayRef, AvengerChartError> {
    array.column_by_name(name).ok_or_else(|| {
        AvengerChartError::InvalidArgument(format!(
            "UniformRaster2D raster struct is missing required field '{name}'"
        ))
    })
}

pub fn struct_child_struct<'a>(
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

pub fn list_row_values(
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

pub fn required_position<'a>(
    position: &'a Option<RasterPositionSpec>,
    channel: &str,
) -> Result<&'a RasterPositionSpec, AvengerChartError> {
    position.as_ref().ok_or_else(|| {
        AvengerChartError::MissingChannelError(format!(
            "UniformRaster2D raster_with(...).{channel}(dim(...))"
        ))
    })
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

pub fn option_color(
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

pub fn scale_numeric_extent(
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

pub fn build_or_reuse_rgba_image(
    raster: &GridRasterRow,
    fill_values: &ArrayRef,
    null_color: [f32; 4],
    non_finite_color: [f32; 4],
    opacity: f32,
    x_dim: &str,
    y_dim: &str,
    x_indices: &[usize],
    y_indices: &[usize],
    flip_x: bool,
    flip_y: bool,
    image_cache: &UniformRasterImageCacheHandle,
) -> Result<Arc<RgbaImage>, AvengerChartError> {
    let identity_key_start = Instant::now();
    let identity_key = UniformRasterImageIdentityKey {
        raster_values_identity: hash_array_identity(&raster.values),
        fill_values_identity: hash_array_identity(fill_values),
        x_dim: x_dim.to_string(),
        y_dim: y_dim.to_string(),
        x_indices_hash: hash_indices(x_indices),
        y_indices_hash: hash_indices(y_indices),
        flip_x,
        flip_y,
        opacity: opacity.to_bits(),
        null_color: color_bits(null_color),
        non_finite_color: color_bits(non_finite_color),
    };
    let identity_key_elapsed = identity_key_start.elapsed();

    if let Some(image) = image_cache
        .lock()
        .expect("uniform raster image cache lock poisoned")
        .get_identity(&identity_key, &raster.values, fill_values)
    {
        debug!(
            target: "avenger_chart::raster",
            x_dim,
            y_dim,
            width = x_indices.len(),
            height = y_indices.len(),
            identity_key_ms = identity_key_elapsed.as_secs_f64() * 1000.0,
            "uniform raster RGBA image identity cache hit"
        );
        return Ok(image);
    }

    let key_start = Instant::now();
    let key = UniformRasterImageKey {
        raster_values_hash: hash_array_contents(&raster.values),
        fill_values_hash: hash_array_contents(fill_values),
        x_dim: x_dim.to_string(),
        y_dim: y_dim.to_string(),
        x_indices_hash: hash_indices(x_indices),
        y_indices_hash: hash_indices(y_indices),
        flip_x,
        flip_y,
        opacity: opacity.to_bits(),
        null_color: color_bits(null_color),
        non_finite_color: color_bits(non_finite_color),
    };
    let key_elapsed = key_start.elapsed();

    let content_cached_image = {
        let mut cache = image_cache
            .lock()
            .expect("uniform raster image cache lock poisoned");
        let image = cache.get(&key);
        if let Some(image) = &image {
            cache.insert_identity(
                identity_key.clone(),
                raster.values.clone(),
                fill_values.clone(),
                image.clone(),
            );
        }
        image
    };
    if let Some(image) = content_cached_image {
        debug!(
            target: "avenger_chart::raster",
            x_dim,
            y_dim,
            width = x_indices.len(),
            height = y_indices.len(),
            identity_key_ms = identity_key_elapsed.as_secs_f64() * 1000.0,
            content_key_ms = key_elapsed.as_secs_f64() * 1000.0,
            "uniform raster RGBA image cache hit"
        );
        return Ok(image);
    }

    let coerce_start = Instant::now();
    let colors = coerce_cell_colors(fill_values)?;
    let coerce_elapsed = coerce_start.elapsed();
    let build_start = Instant::now();
    let image = Arc::new(build_rgba_image(
        raster,
        &colors,
        null_color,
        non_finite_color,
        opacity,
        x_dim,
        y_dim,
        x_indices,
        y_indices,
        flip_x,
        flip_y,
    )?);
    let build_elapsed = build_start.elapsed();

    {
        let mut cache = image_cache
            .lock()
            .expect("uniform raster image cache lock poisoned");
        cache.insert(key, image.clone());
        cache.insert_identity(
            identity_key,
            raster.values.clone(),
            fill_values.clone(),
            image.clone(),
        );
    }
    debug!(
        target: "avenger_chart::raster",
        x_dim,
        y_dim,
        width = x_indices.len(),
        height = y_indices.len(),
        identity_key_ms = identity_key_elapsed.as_secs_f64() * 1000.0,
        content_key_ms = key_elapsed.as_secs_f64() * 1000.0,
        coerce_ms = coerce_elapsed.as_secs_f64() * 1000.0,
        build_ms = build_elapsed.as_secs_f64() * 1000.0,
        "uniform raster RGBA image cache miss"
    );
    Ok(image)
}

pub fn build_rgba_image(
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

fn color_bits(color: [f32; 4]) -> [u32; 4] {
    [
        color[0].to_bits(),
        color[1].to_bits(),
        color[2].to_bits(),
        color[3].to_bits(),
    ]
}

fn hash_indices(indices: &[usize]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    indices.hash(&mut hasher);
    hasher.finish()
}

fn hash_array_contents(array: &ArrayRef) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hash_array_data(&array.to_data(), &mut hasher);
    hasher.finish()
}

fn hash_array_identity(array: &ArrayRef) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hash_array_data_identity(&array.to_data(), &mut hasher);
    hasher.finish()
}

fn hash_array_data(data: &ArrayData, hasher: &mut impl Hasher) {
    format!("{:?}", data.data_type()).hash(hasher);
    data.len().hash(hasher);
    data.offset().hash(hasher);
    data.null_count().hash(hasher);

    if let Some(nulls) = data.nulls() {
        nulls.buffer().as_slice().hash(hasher);
    }
    for buffer in data.buffers() {
        buffer.as_slice().hash(hasher);
    }
    for child in data.child_data() {
        hash_array_data(child, hasher);
    }
}

fn hash_array_data_identity(data: &ArrayData, hasher: &mut impl Hasher) {
    format!("{:?}", data.data_type()).hash(hasher);
    data.len().hash(hasher);
    data.offset().hash(hasher);
    data.null_count().hash(hasher);

    if let Some(nulls) = data.nulls() {
        hash_buffer_identity(nulls.buffer().as_slice(), hasher);
    }
    for buffer in data.buffers() {
        hash_buffer_identity(buffer.as_slice(), hasher);
    }
    for child in data.child_data() {
        hash_array_data_identity(child, hasher);
    }
}

fn hash_buffer_identity<T>(slice: &[T], hasher: &mut impl Hasher) {
    (slice.as_ptr() as usize).hash(hasher);
    slice.len().hash(hasher);
}

/// True pointer identity between two arrays: same buffers at the same
/// addresses with the same layout. Only meaningful when one side is pinned
/// alive (see `UniformRasterIdentityCachedImage`).
fn arrays_share_identity(left: &ArrayRef, right: &ArrayRef) -> bool {
    array_data_shares_identity(&left.to_data(), &right.to_data())
}

fn array_data_shares_identity(left: &ArrayData, right: &ArrayData) -> bool {
    if left.data_type() != right.data_type()
        || left.len() != right.len()
        || left.offset() != right.offset()
        || left.null_count() != right.null_count()
    {
        return false;
    }
    let nulls_match = match (left.nulls(), right.nulls()) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            buffer_shares_identity(left.buffer().as_slice(), right.buffer().as_slice())
        }
        _ => false,
    };
    if !nulls_match {
        return false;
    }
    if left.buffers().len() != right.buffers().len()
        || left.child_data().len() != right.child_data().len()
    {
        return false;
    }
    for (left_buffer, right_buffer) in left.buffers().iter().zip(right.buffers()) {
        if !buffer_shares_identity(left_buffer.as_slice(), right_buffer.as_slice()) {
            return false;
        }
    }
    for (left_child, right_child) in left.child_data().iter().zip(right.child_data()) {
        if !array_data_shares_identity(left_child, right_child) {
            return false;
        }
    }
    true
}

fn buffer_shares_identity<T>(left: &[T], right: &[T]) -> bool {
    std::ptr::eq(left.as_ptr(), right.as_ptr()) && left.len() == right.len()
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
