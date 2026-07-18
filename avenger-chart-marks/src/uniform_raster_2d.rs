use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::{Hash, Hasher},
    sync::{Arc, Mutex},
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
use avenger_color::{ColorOrGradient, OklabMixer};
use avenger_common::{
    time::Instant,
    types::{ImageAlign, ImageBaseline},
    value::ScalarOrArray,
};
use avenger_image::RgbaImage;
use avenger_scales::scales::{ConfiguredScale, coerce::Coercer};
use avenger_scenegraph::marks::{
    image::{SceneImageMark, SceneImageSource},
    mark::SceneMark,
};

pub use avenger_chart_core::{RasterDim, dim};

pub const UNIFORM_RASTER_2D_RASTER_CHANNEL: &str = "raster";
pub const UNIFORM_RASTER_2D_FILL_CHANNEL: &str = "fill";
pub const UNIFORM_RASTER_2D_OPACITY_BY_TOTAL_CHANNEL: &str = "opacity_by_total";

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
    pub fn with_mark_effects(mut self, effects: PrimitiveMarkEffects) -> Self {
        self.effects = effects;
        self
    }

    #[doc(hidden)]
    pub fn configure_raster(
        self,
        raster_expr: Expr,
        fill: Option<ChannelValue>,
        positions: Option<(Option<RasterPositionSpec>, Option<RasterPositionSpec>)>,
    ) -> Self {
        self.configure_raster_with_overlay(raster_expr, fill, positions, None, None)
    }

    #[doc(hidden)]
    pub fn configure_raster_with_overlay(
        mut self,
        raster_expr: Expr,
        fill: Option<ChannelValue>,
        positions: Option<(Option<RasterPositionSpec>, Option<RasterPositionSpec>)>,
        fill_by: Option<(RasterDim, ChannelValue)>,
        opacity_by_total: Option<ChannelValue>,
    ) -> Self {
        let fields = UniformRaster2DFields::new(raster_expr.clone());
        // The categorical overlay owns the fill channel: its value is the
        // fill_by channel value (Utf8-typed so the fill scale defaults to
        // Ordinal); otherwise fill defaults to the raster cell values.
        let fill = match &fill_by {
            Some((_, fill_by_value)) => fill_by_value.clone(),
            None => fill.unwrap_or_else(|| ChannelValue::from(fields.values_data())),
        };
        if let Some((x_position, y_position)) = positions {
            self.options.x_position = x_position;
            self.options.y_position = y_position;
        }
        if let Some((dim, channel_value)) = fill_by {
            self.options.fill_by = Some(RasterPositionSpec {
                dim,
                channel_value: channel_value.clone(),
            });
        }
        let mut mark = self
            .with_channel_value(
                UNIFORM_RASTER_2D_RASTER_CHANNEL,
                ChannelValue::from(raster_expr).no_scale(),
            )
            .with_channel_value(UNIFORM_RASTER_2D_FILL_CHANNEL, fill);
        if let Some(opacity_by_total) = opacity_by_total {
            mark.options.opacity_by_total = Some(opacity_by_total.clone());
            mark = mark
                .with_channel_value(UNIFORM_RASTER_2D_OPACITY_BY_TOTAL_CHANNEL, opacity_by_total);
        }
        mark
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
    /// Categorical overlay: the plane dimension whose values drive the fill
    /// channel's ordinal scale; K planes render as ONE Oklab-mixed image.
    #[serde(default)]
    pub fill_by: Option<RasterPositionSpec>,
    /// Density channel: per-pixel plane totals run through this channel's
    /// numeric scale to produce alpha. When absent, a linear scale over
    /// (0, max total) with range (0.15, 1.0) is used.
    #[serde(default)]
    pub opacity_by_total: Option<ChannelValue>,
}

impl Default for UniformRaster2DOptions {
    fn default() -> Self {
        Self {
            null_color: ChannelValue::from("#00000000"),
            non_finite_color: ChannelValue::from("#00000000"),
            x_position: None,
            y_position: None,
            smooth: false,
            fill_by: None,
            opacity_by_total: None,
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
    fill_by: Option<(RasterDim, ColorChannelConfig)>,
    opacity_by_total: Option<OpacityChannelConfig>,
}

impl<A: Clone + Default + Send + Sync + 'static> RasterChannelsConfig<A> {
    pub fn new(fill: ColorChannelConfig) -> Self {
        Self {
            fill,
            x: None,
            y: None,
            fill_by: None,
            opacity_by_total: None,
        }
    }

    /// Categorical overlay: bind the fill channel to the VALUES of a
    /// categorical raster dimension (one produced by `Rasterize2D::by`, or
    /// present in an external 3D raster). The dimension's planes render as
    /// one Oklab-mixed image; the ordinal fill scale supplies category
    /// colors (theme default categorical scheme unless configured) and the
    /// standard swatch legend.
    pub fn fill_by<F>(mut self, dim: RasterDim, f: F) -> Self
    where
        F: FnOnce(ColorChannelConfig) -> ColorChannelConfig,
    {
        // A SCALED Utf8-typed channel value (bare &str literals are
        // unscaled and would suppress scale building): the Utf8 type makes
        // the fill scale default to Ordinal with the theme's categorical
        // scheme. NULL so unique-value domain inference never picks the
        // placeholder up as a spurious category.
        let config = ColorChannelConfig::new(ChannelValue::from(datafusion::logical_expr::lit(
            ScalarValue::Utf8(None),
        )));
        self.fill_by = Some((dim, f(config)));
        self
    }

    /// Density -> alpha: run the per-pixel plane totals through a numeric
    /// scale. The range floor is the datashader `min_alpha` analog.
    pub fn opacity_by_total<F>(mut self, f: F) -> Self
    where
        F: FnOnce(OpacityChannelConfig) -> OpacityChannelConfig,
    {
        // A SCALED literal seed (plain f64 literals are unscaled and would
        // suppress building the opacity scale).
        let config =
            OpacityChannelConfig::new(ChannelValue::from(datafusion::logical_expr::lit(1.0_f64)));
        self.opacity_by_total = Some(f(config));
        self
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
    pub fn into_parts(self) -> RasterChannelsParts<A> {
        RasterChannelsParts {
            fill: self.fill.into_inner(),
            x: self.x,
            y: self.y,
            fill_by: self.fill_by.map(|(dim, config)| (dim, config.into_inner())),
            opacity_by_total: self.opacity_by_total.map(ChannelConfig::into_inner),
        }
    }
}

#[doc(hidden)]
pub struct RasterChannelsParts<A: Clone + Default + Send + Sync + 'static> {
    pub fill: ChannelValue,
    pub x: Option<RasterPositionConfig<A>>,
    pub y: Option<RasterPositionConfig<A>>,
    pub fill_by: Option<(RasterDim, ChannelValue)>,
    pub opacity_by_total: Option<ChannelValue>,
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
    mixed_entries: HashMap<MixedRasterImageKey, Arc<RgbaImage>>,
    mixed_order: VecDeque<MixedRasterImageKey>,
    mixed_identity_entries: HashMap<MixedRasterImageIdentityKey, MixedRasterIdentityCachedImage>,
    mixed_identity_order: VecDeque<MixedRasterImageIdentityKey>,
}

/// Identity cache entry for the categorical overlay path; pins the raster
/// array for the same buffer-identity reasons as the scalar-fill entry.
struct MixedRasterIdentityCachedImage {
    raster_values: ArrayRef,
    image: Arc<RgbaImage>,
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

    fn get_mixed(&self, key: &MixedRasterImageKey) -> Option<Arc<RgbaImage>> {
        self.mixed_entries.get(key).cloned()
    }

    fn get_mixed_identity(
        &self,
        key: &MixedRasterImageIdentityKey,
        raster_values: &ArrayRef,
    ) -> Option<Arc<RgbaImage>> {
        let entry = self.mixed_identity_entries.get(key)?;
        if arrays_share_identity(&entry.raster_values, raster_values) {
            Some(entry.image.clone())
        } else {
            None
        }
    }

    fn insert_mixed(&mut self, key: MixedRasterImageKey, image: Arc<RgbaImage>) {
        if self.mixed_entries.contains_key(&key) {
            self.mixed_entries.insert(key, image);
            return;
        }
        self.mixed_entries.insert(key.clone(), image);
        self.mixed_order.push_back(key);
        while self.mixed_entries.len() > UNIFORM_RASTER_IMAGE_CACHE_CAPACITY {
            if let Some(oldest) = self.mixed_order.pop_front() {
                self.mixed_entries.remove(&oldest);
            } else {
                break;
            }
        }
    }

    fn insert_mixed_identity(
        &mut self,
        key: MixedRasterImageIdentityKey,
        raster_values: ArrayRef,
        image: Arc<RgbaImage>,
    ) {
        let entry = MixedRasterIdentityCachedImage {
            raster_values,
            image,
        };
        if self.mixed_identity_entries.contains_key(&key) {
            self.mixed_identity_entries.insert(key, entry);
            return;
        }
        self.mixed_identity_entries.insert(key.clone(), entry);
        self.mixed_identity_order.push_back(key);
        while self.mixed_identity_entries.len() > UNIFORM_RASTER_IMAGE_CACHE_CAPACITY {
            if let Some(oldest) = self.mixed_identity_order.pop_front() {
                self.mixed_identity_entries.remove(&oldest);
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MixedRasterImageIdentityKey {
    raster_values_identity: u64,
    mix_dim: String,
    category_colors_hash: u64,
    opacity_scale_hash: u64,
    x_dim: String,
    y_dim: String,
    x_indices_hash: u64,
    y_indices_hash: u64,
    flip_x: bool,
    flip_y: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MixedRasterImageKey {
    raster_values_hash: u64,
    mix_dim: String,
    category_colors_hash: u64,
    opacity_scale_hash: u64,
    x_dim: String,
    y_dim: String,
    x_indices_hash: u64,
    y_indices_hash: u64,
    flip_x: bool,
    flip_y: bool,
}

fn hash_category_colors(category_colors: &[(String, [f32; 4])]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for (category, color) in category_colors {
        category.hash(&mut hasher);
        color_bits(*color).hash(&mut hasher);
    }
    hasher.finish()
}

/// Content fingerprint of a configured scale, for cache keys: scale type,
/// domain/range array contents, and options.
fn hash_configured_scale(scale: &ConfiguredScale) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    scale.scale_impl.scale_type().hash(&mut hasher);
    hash_array_data(&scale.config.domain.to_data(), &mut hasher);
    hash_array_data(&scale.config.range.to_data(), &mut hasher);
    let mut options = scale
        .config
        .options
        .iter()
        .map(|(name, value)| (name.clone(), format!("{value:?}")))
        .collect::<Vec<_>>();
    options.sort();
    options.hash(&mut hasher);
    hasher.finish()
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
        self.cell_index_with(&[(x_dim, x_index), (y_dim, y_index)])
    }

    /// Flat cell index from (dimension, index) assignments covering every
    /// entry of `values_dims`.
    pub fn cell_index_with(
        &self,
        assignments: &[(&str, usize)],
    ) -> Result<usize, AvengerChartError> {
        let mut index = 0usize;
        for dim_name in &self.values_dims {
            let dim = self.dimension(dim_name)?;
            let dim_index = assignments
                .iter()
                .find(|(name, _)| name == dim_name)
                .map(|(_, value)| *value)
                .ok_or_else(|| {
                    AvengerChartError::InvalidArgument(format!(
                        "UniformRaster2D cannot render unselected value dimension '{dim_name}'"
                    ))
                })?;
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

    /// The categorical dimension's values, for the overlay (mixing) path.
    pub fn categorical_dim_values(&self, name: &str) -> Result<&[String], AvengerChartError> {
        match &self.dimension(name)?.coords {
            RasterCoords::Categorical { values } => Ok(values),
            RasterCoords::Uniform { .. } => Err(AvengerChartError::InvalidArgument(format!(
                "UniformRaster2D dimension '{name}' is uniform; the overlay mix dimension must be categorical"
            ))),
        }
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

/// Categorical overlay: build (or reuse) ONE mixed RGBA image from the K
/// planes of `mix_dim`. Per pixel the plane values act as convex weights
/// over the resolved category colors (Oklab mixing — not configurable), and
/// alpha comes from the opacity scale applied to the plane total. Zero- or
/// all-null-total pixels are fully transparent.
///
/// `category_colors` are matched to planes BY CATEGORY VALUE: every plane
/// must have a color (a plane without one is an error — the fill scale's
/// domain should cover all observed categories); extra colors are ignored.
#[allow(clippy::too_many_arguments)]
/// Resolve everything the categorical overlay needs from the runtime
/// context: category colors (through the fill_by channel's ordinal scale,
/// matched by value) and the opacity scale (configured
/// `opacity_by_total`, or the default linear-over-max-total scale).
/// Returns `None` when the raster row observed no categories.
pub fn resolve_overlay_inputs(
    raster: &GridRasterRow,
    fill_by: &RasterPositionSpec,
    has_opacity_scale: bool,
    context: &dyn MarkRuntimeContext,
) -> Result<Option<(Vec<(String, [f32; 4])>, ConfiguredScale)>, AvengerChartError> {
    let mix_dim = fill_by.dim.name();
    let categories = raster.categorical_dim_values(mix_dim)?.to_vec();
    if categories.is_empty() {
        return Ok(None);
    }
    let fill_scale_name = fill_by
        .channel_value
        .get_scale_name(UNIFORM_RASTER_2D_FILL_CHANNEL)
        .unwrap_or_else(|| UNIFORM_RASTER_2D_FILL_CHANNEL.to_string());
    let fill_scale = context.configured_scale(&fill_scale_name).ok_or_else(|| {
        AvengerChartError::ScaleNotFound(format!(
            "UniformRaster2D fill_by expected configured scale '{fill_scale_name}'"
        ))
    })?;
    let category_colors = resolve_category_colors(fill_scale, &categories)?;
    let opacity_scale = if has_opacity_scale {
        context
            .configured_scale(UNIFORM_RASTER_2D_OPACITY_BY_TOTAL_CHANNEL)
            .cloned()
            .ok_or_else(|| {
                AvengerChartError::ScaleNotFound(
                    "UniformRaster2D opacity_by_total scale was configured but not built"
                        .to_string(),
                )
            })?
    } else {
        default_total_opacity_scale(raster, mix_dim)?
    };
    Ok(Some((category_colors, opacity_scale)))
}

/// Resolve per-category colors through the fill scale, BY VALUE.
pub fn resolve_category_colors(
    scale: &ConfiguredScale,
    categories: &[String],
) -> Result<Vec<(String, [f32; 4])>, AvengerChartError> {
    let values = Arc::new(StringArray::from(
        categories.iter().map(String::as_str).collect::<Vec<_>>(),
    )) as ArrayRef;
    let colors = scale
        .scale_to_color(&values)
        .map_err(AvengerChartError::ScaleError)?
        .as_vec(categories.len(), None);
    categories
        .iter()
        .zip(colors)
        .map(|(category, color)| {
            Ok((
                category.clone(),
                color_to_rgba(color, &format!("fill_by category '{category}'"))?,
            ))
        })
        .collect()
}

/// Maximum per-cell plane total of a categorical raster — the default
/// opacity domain upper bound when no opacity_by_total scale is configured.
pub fn max_plane_total(raster: &GridRasterRow, mix_dim: &str) -> Result<f64, AvengerChartError> {
    let plane_count = raster.categorical_dim_values(mix_dim)?.len();
    let mix_stride = *raster.strides.get(mix_dim).ok_or_else(|| {
        AvengerChartError::InternalError(format!(
            "UniformRaster2D missing computed stride for dimension '{mix_dim}'"
        ))
    })?;
    let values = datafusion::arrow::compute::cast(&raster.values, &DataType::Float64)
        .map_err(AvengerChartError::ArrowError)?;
    let values = values
        .as_any()
        .downcast_ref::<Float64Array>()
        .expect("cast to Float64 produced a Float64Array");
    if plane_count == 0 || values.is_empty() {
        return Ok(0.0);
    }
    // Removing the mix-dim contribution from a flat index yields an
    // injective key for the remaining coordinates (contiguous stride
    // layouts), so totals fold per non-mix cell without reconstructing the
    // full coordinate tuple.
    let mut totals: HashMap<usize, f64> = HashMap::new();
    for index in 0..values.len() {
        if values.is_null(index) {
            continue;
        }
        let value = values.value(index);
        if !value.is_finite() || value <= 0.0 {
            continue;
        }
        let mix_coord = (index / mix_stride) % plane_count;
        let bucket = index - mix_coord * mix_stride;
        *totals.entry(bucket).or_insert(0.0) += value;
    }
    Ok(totals.values().copied().fold(0.0_f64, f64::max))
}

/// Default opacity scale when `opacity_by_total` is not configured:
/// linear over (0, max plane total) to (0.15, 1.0) — the range floor is
/// the datashader min_alpha analog.
pub fn default_total_opacity_scale(
    raster: &GridRasterRow,
    mix_dim: &str,
) -> Result<ConfiguredScale, AvengerChartError> {
    let max_total = max_plane_total(raster, mix_dim)?.max(1.0);
    Ok(avenger_scales::scales::linear::LinearScale::configured(
        (0.0, max_total as f32),
        (0.15, 1.0),
    ))
}

pub fn build_or_reuse_mixed_rgba_image(
    raster: &GridRasterRow,
    mix_dim: &str,
    category_colors: &[(String, [f32; 4])],
    opacity_scale: &ConfiguredScale,
    x_dim: &str,
    y_dim: &str,
    x_indices: &[usize],
    y_indices: &[usize],
    flip_x: bool,
    flip_y: bool,
    image_cache: &UniformRasterImageCacheHandle,
) -> Result<Arc<RgbaImage>, AvengerChartError> {
    let category_colors_hash = hash_category_colors(category_colors);
    let opacity_scale_hash = hash_configured_scale(opacity_scale);
    let identity_key = MixedRasterImageIdentityKey {
        raster_values_identity: hash_array_identity(&raster.values),
        mix_dim: mix_dim.to_string(),
        category_colors_hash,
        opacity_scale_hash,
        x_dim: x_dim.to_string(),
        y_dim: y_dim.to_string(),
        x_indices_hash: hash_indices(x_indices),
        y_indices_hash: hash_indices(y_indices),
        flip_x,
        flip_y,
    };
    if let Some(image) = image_cache
        .lock()
        .expect("uniform raster image cache lock poisoned")
        .get_mixed_identity(&identity_key, &raster.values)
    {
        return Ok(image);
    }

    let key = MixedRasterImageKey {
        raster_values_hash: hash_array_contents(&raster.values),
        mix_dim: mix_dim.to_string(),
        category_colors_hash,
        opacity_scale_hash,
        x_dim: x_dim.to_string(),
        y_dim: y_dim.to_string(),
        x_indices_hash: hash_indices(x_indices),
        y_indices_hash: hash_indices(y_indices),
        flip_x,
        flip_y,
    };
    let content_cached_image = {
        let mut cache = image_cache
            .lock()
            .expect("uniform raster image cache lock poisoned");
        let image = cache.get_mixed(&key);
        if let Some(image) = &image {
            cache.insert_mixed_identity(identity_key.clone(), raster.values.clone(), image.clone());
        }
        image
    };
    if let Some(image) = content_cached_image {
        return Ok(image);
    }

    let build_start = Instant::now();
    let image = Arc::new(build_mixed_rgba_image(
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
    )?);
    debug!(
        target: "avenger_chart::raster",
        mix_dim,
        planes = raster.categorical_dim_values(mix_dim).map(|values| values.len()).unwrap_or(0),
        width = x_indices.len(),
        height = y_indices.len(),
        build_ms = build_start.elapsed().as_secs_f64() * 1000.0,
        "mixed categorical raster RGBA image cache miss"
    );

    let mut cache = image_cache
        .lock()
        .expect("uniform raster image cache lock poisoned");
    cache.insert_mixed(key, image.clone());
    cache.insert_mixed_identity(identity_key, raster.values.clone(), image.clone());
    Ok(image)
}

pub fn build_mixed_rgba_image(
    raster: &GridRasterRow,
    mix_dim: &str,
    category_colors: &[(String, [f32; 4])],
    opacity_scale: &ConfiguredScale,
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

    let plane_values = raster.categorical_dim_values(mix_dim)?;
    // Plane colors matched by category VALUE; plane order, scale domain
    // order, and legend order are three independent orders.
    let plane_colors = plane_values
        .iter()
        .map(|category| {
            category_colors
                .iter()
                .find(|(candidate, _)| candidate == category)
                .map(|(_, color)| *color)
                .ok_or_else(|| {
                    AvengerChartError::InvalidArgument(format!(
                        "UniformRaster2D overlay has no color for category '{category}' of dimension '{mix_dim}'"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mixer = OklabMixer::new(&plane_colors);
    let plane_count = plane_values.len();

    // Weights as f64 regardless of the stored cell type (counts are UInt64).
    let weight_values = datafusion::arrow::compute::cast(&raster.values, &DataType::Float64)
        .map_err(AvengerChartError::ArrowError)?;
    let weight_values = weight_values
        .as_any()
        .downcast_ref::<Float64Array>()
        .expect("cast to Float64 produced a Float64Array");

    // Pass 1: per-pixel totals in pixel order, then batch-scale to alpha.
    let pixel_count = width as usize * height as usize;
    let mut totals = Vec::with_capacity(pixel_count);
    let mut weights = vec![0.0_f32; plane_count];
    let mut cell_of_pixel = Vec::with_capacity(pixel_count);
    for py in 0..height as usize {
        for px in 0..width as usize {
            let x_offset = if flip_x { width as usize - 1 - px } else { px };
            let y_offset = if flip_y { height as usize - 1 - py } else { py };
            let x_index = *x_indices.get(x_offset).ok_or_else(|| {
                AvengerChartError::InternalError(
                    "UniformRaster2D x pixel index out of bounds".to_string(),
                )
            })?;
            let y_index = *y_indices.get(y_offset).ok_or_else(|| {
                AvengerChartError::InternalError(
                    "UniformRaster2D y pixel index out of bounds".to_string(),
                )
            })?;
            let mut total = 0.0_f64;
            for plane in 0..plane_count {
                let cell = raster.cell_index_with(&[
                    (x_dim, x_index),
                    (y_dim, y_index),
                    (mix_dim, plane),
                ])?;
                if plane == 0 {
                    cell_of_pixel.push(cell);
                }
                if weight_values.is_null(cell) {
                    continue;
                }
                let value = weight_values.value(cell);
                if value.is_finite() && value > 0.0 {
                    total += value;
                }
            }
            totals.push(total);
        }
    }
    let totals_array = Arc::new(Float64Array::from(totals.clone())) as ArrayRef;
    let alphas = opacity_scale
        .scale_to_numeric(&totals_array)
        .map_err(AvengerChartError::ScaleError)?
        .as_vec(pixel_count, None);

    // Pass 2: mix per pixel. Plane cell indices differ from the plane-0
    // index by the mix dimension's stride.
    let mix_stride = *raster.strides.get(mix_dim).ok_or_else(|| {
        AvengerChartError::InternalError(format!(
            "UniformRaster2D missing computed stride for dimension '{mix_dim}'"
        ))
    })?;
    let mut data = Vec::with_capacity(pixel_count * 4);
    for pixel in 0..pixel_count {
        let total = totals[pixel];
        if total <= 0.0 {
            data.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        let base_cell = cell_of_pixel[pixel];
        for (plane, weight) in weights.iter_mut().enumerate() {
            let cell = base_cell + plane * mix_stride;
            *weight = if weight_values.is_null(cell) {
                0.0
            } else {
                let value = weight_values.value(cell);
                if value.is_finite() && value > 0.0 {
                    value as f32
                } else {
                    0.0
                }
            };
        }
        let Some(mixed) = mixer.mix(&weights) else {
            data.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        };
        let alpha = alphas[pixel];
        let alpha = if alpha.is_finite() {
            alpha.clamp(0.0, 1.0)
        } else {
            0.0
        };
        push_rgba8(&mut data, [mixed[0], mixed[1], mixed[2], 1.0], alpha);
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

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::UInt64Array;

    /// 2x2 grid with two categorical planes ("a", "b"), plane-major
    /// [cat, y, x] layout:
    ///   plane a: cell(0,0)=4, cell(x0,y1)=1
    ///   plane b: cell(x1,y0)=2, cell(x0,y1)=1
    fn two_plane_row() -> GridRasterRow {
        let mut strides = HashMap::new();
        strides.insert("x".to_string(), 1usize);
        strides.insert("y".to_string(), 2usize);
        strides.insert("cat".to_string(), 4usize);
        GridRasterRow {
            dimensions: vec![
                RasterDimension {
                    name: "x".to_string(),
                    coords: RasterCoords::Uniform {
                        _sampling: Some("linear".to_string()),
                        start: 0.0,
                        stop: 2.0,
                        count: 2,
                    },
                },
                RasterDimension {
                    name: "y".to_string(),
                    coords: RasterCoords::Uniform {
                        _sampling: Some("linear".to_string()),
                        start: 0.0,
                        stop: 2.0,
                        count: 2,
                    },
                },
                RasterDimension {
                    name: "cat".to_string(),
                    coords: RasterCoords::Categorical {
                        values: vec!["a".to_string(), "b".to_string()],
                    },
                },
            ],
            values_dims: vec!["cat".to_string(), "y".to_string(), "x".to_string()],
            strides,
            values: Arc::new(UInt64Array::from(vec![
                4, 0, 1, 0, // plane a
                0, 2, 1, 0, // plane b
            ])) as ArrayRef,
        }
    }

    fn category_colors() -> Vec<(String, [f32; 4])> {
        vec![
            ("a".to_string(), [1.0, 0.0, 0.0, 1.0]),
            ("b".to_string(), [0.0, 0.0, 1.0, 1.0]),
        ]
    }

    fn opacity_scale() -> ConfiguredScale {
        avenger_scales::scales::linear::LinearScale::configured((0.0, 4.0), (0.0, 1.0))
    }

    fn pixel(image: &RgbaImage, px: usize, py: usize) -> [u8; 4] {
        let offset = (py * image.width as usize + px) * 4;
        [
            image.data[offset],
            image.data[offset + 1],
            image.data[offset + 2],
            image.data[offset + 3],
        ]
    }

    #[test]
    fn mixed_rgba_pure_mixed_and_transparent_pixels() {
        let raster = two_plane_row();
        let image = build_mixed_rgba_image(
            &raster,
            "cat",
            &category_colors(),
            &opacity_scale(),
            "x",
            "y",
            &[0, 1],
            &[0, 1],
            false,
            false,
        )
        .unwrap();

        // Pure a at (0,0): exact red, total=4 -> alpha 255.
        assert_eq!(pixel(&image, 0, 0), [255, 0, 0, 255]);
        // Pure b at (1,0): exact blue, total=2 -> alpha ~128.
        let blue = pixel(&image, 1, 0);
        assert_eq!([blue[0], blue[1], blue[2]], [0, 0, 255]);
        assert!(
            (blue[3] as i32 - 128).unsigned_abs() <= 1,
            "alpha {}",
            blue[3]
        );
        // 50/50 mix at (0,1) equals the OklabMixer output.
        let mixer = OklabMixer::new(&[[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]]);
        let expected = mixer.mix(&[1.0, 1.0]).unwrap();
        let mixed = pixel(&image, 0, 1);
        for channel in 0..3 {
            assert!(
                (mixed[channel] as f32 / 255.0 - expected[channel]).abs() <= 1.5 / 255.0,
                "channel {channel}: {mixed:?} vs {expected:?}"
            );
        }
        // Zero-total at (1,1): fully transparent.
        assert_eq!(pixel(&image, 1, 1), [0, 0, 0, 0]);
    }

    #[test]
    fn mixed_rgba_flip_y_reverses_rows() {
        let raster = two_plane_row();
        let unflipped = build_mixed_rgba_image(
            &raster,
            "cat",
            &category_colors(),
            &opacity_scale(),
            "x",
            "y",
            &[0, 1],
            &[0, 1],
            false,
            false,
        )
        .unwrap();
        let flipped = build_mixed_rgba_image(
            &raster,
            "cat",
            &category_colors(),
            &opacity_scale(),
            "x",
            "y",
            &[0, 1],
            &[0, 1],
            false,
            true,
        )
        .unwrap();
        assert_eq!(pixel(&flipped, 0, 1), pixel(&unflipped, 0, 0));
        assert_eq!(pixel(&flipped, 0, 0), pixel(&unflipped, 0, 1));
    }

    #[test]
    fn mixed_rgba_missing_plane_color_errors() {
        let raster = two_plane_row();
        let colors = vec![("a".to_string(), [1.0, 0.0, 0.0, 1.0])];
        let err = build_mixed_rgba_image(
            &raster,
            "cat",
            &colors,
            &opacity_scale(),
            "x",
            "y",
            &[0, 1],
            &[0, 1],
            false,
            false,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("no color for category 'b'"),
            "{err}"
        );
    }

    #[test]
    fn mixed_rgba_cache_hits_and_color_change_misses() {
        let raster = two_plane_row();
        let cache = default_uniform_raster_image_cache();
        let build = |colors: &[(String, [f32; 4])]| {
            build_or_reuse_mixed_rgba_image(
                &raster,
                "cat",
                colors,
                &opacity_scale(),
                "x",
                "y",
                &[0, 1],
                &[0, 1],
                false,
                false,
                &cache,
            )
            .unwrap()
        };
        let first = build(&category_colors());
        let second = build(&category_colors());
        assert!(
            Arc::ptr_eq(&first, &second),
            "same inputs must hit the cache"
        );
        let mut recolored = category_colors();
        recolored[1].1 = [0.0, 1.0, 0.0, 1.0];
        let third = build(&recolored);
        assert!(
            !Arc::ptr_eq(&first, &third),
            "changed category colors must rebuild"
        );
    }
}
