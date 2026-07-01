use datafusion::{
    logical_expr::{Expr, lit},
    prelude::get_field,
};
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};

use avenger_chart_core::{
    Axis, ChannelConfig, ChannelValue, ColorChannelConfig, CoordinationScope, IntoExpr, MarkState,
    OpacityChannelConfig, PrimitiveMarkEffects, Scale, ScaleChannelValue, ScaleSpec,
    define_common_mark_channels, impl_mark_base_with_extra_fields,
};

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
    pub fn raster(self, data: impl IntoExpr) -> Self {
        self.configure_raster(data.into_expr(), None, None)
    }

    pub fn null_color(mut self, color: impl Into<ChannelValue>) -> Self {
        self.options.null_color = color.into();
        self
    }

    pub fn non_finite_color(mut self, color: impl Into<ChannelValue>) -> Self {
        self.options.non_finite_color = color.into();
        self
    }

    pub fn transpose(mut self) -> Self {
        self.options.transpose = true;
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
        positions: Option<(ChannelValue, ChannelValue)>,
    ) -> Self {
        let fields = UniformRaster2DFields::new(raster_expr.clone());
        let fill = fill.unwrap_or_else(|| ChannelValue::from(fields.values_data()));
        let (x_channel, y_channel) = positions.unwrap_or_else(default_raster_position_channels);
        self.options.x_channel = x_channel;
        self.options.y_channel = y_channel;
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
    pub x_channel: ChannelValue,
    pub y_channel: ChannelValue,
    pub transpose: bool,
    pub smooth: bool,
}

impl Default for UniformRaster2DOptions {
    fn default() -> Self {
        let (x_channel, y_channel) = default_raster_position_channels();
        Self {
            null_color: ChannelValue::from("#00000000"),
            non_finite_color: ChannelValue::from("#00000000"),
            x_channel,
            y_channel,
            transpose: false,
            smooth: false,
        }
    }
}

fn default_raster_position_channels() -> (ChannelValue, ChannelValue) {
    (
        ChannelValue::from(lit(0.0)).with_scale_name("x"),
        ChannelValue::from(lit(0.0)).with_scale_name("y"),
    )
}

pub struct RasterChannelsConfig<A: Clone + Default + Send + Sync + 'static> {
    fill: ColorChannelConfig,
    x: RasterPositionConfig<A>,
    y: RasterPositionConfig<A>,
}

impl<A: Clone + Default + Send + Sync + 'static> RasterChannelsConfig<A> {
    pub fn new(
        fill: ColorChannelConfig,
        x: RasterPositionConfig<A>,
        y: RasterPositionConfig<A>,
    ) -> Self {
        Self { fill, x, y }
    }

    pub fn fill<F>(mut self, f: F) -> Self
    where
        F: FnOnce(ColorChannelConfig) -> ColorChannelConfig,
    {
        self.fill = f(self.fill);
        self
    }

    pub fn x<F>(mut self, f: F) -> Self
    where
        F: FnOnce(RasterPositionConfig<A>) -> RasterPositionConfig<A>,
    {
        self.x = f(self.x);
        self
    }

    pub fn y<F>(mut self, f: F) -> Self
    where
        F: FnOnce(RasterPositionConfig<A>) -> RasterPositionConfig<A>,
    {
        self.y = f(self.y);
        self
    }

    #[doc(hidden)]
    pub fn into_parts(
        self,
    ) -> (
        ChannelValue,
        RasterPositionConfig<A>,
        RasterPositionConfig<A>,
    ) {
        (self.fill.into_inner(), self.x, self.y)
    }
}

#[derive(Clone)]
pub struct RasterPositionConfig<A: Clone + Default + Send + Sync + 'static> {
    inner: ChannelValue,
    axis_config: Option<A>,
}

impl<A: Clone + Default + Send + Sync + 'static> RasterPositionConfig<A> {
    pub fn new(value: ChannelValue) -> Self {
        Self {
            inner: value,
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
        self.inner = self.inner.with_scale_name(name);
        self
    }

    pub fn scale<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale<avenger_chart_core::Auto>) -> Scale<avenger_chart_core::Auto>
            + Send
            + Sync
            + 'static,
    {
        self.inner = self.inner.scale(f);
        self
    }

    pub fn scale_with<S: ScaleSpec + Default>(
        mut self,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        self.inner = self.inner.scale_with::<S>(f);
        self
    }

    pub fn with_domain_scope(mut self, scope: CoordinationScope) -> Self {
        self.inner = self.inner.with_domain_scope(scope);
        self
    }

    pub fn share_domain(self) -> Self {
        self.with_domain_scope(CoordinationScope::Shared)
    }

    pub fn free_domain(self) -> Self {
        self.with_domain_scope(CoordinationScope::Free)
    }

    #[doc(hidden)]
    pub fn take(self) -> (ChannelValue, Option<A>) {
        (self.inner, self.axis_config)
    }
}

impl<A: Axis + Default + Clone + Send + Sync + 'static> From<ChannelValue>
    for RasterPositionConfig<A>
{
    fn from(value: ChannelValue) -> Self {
        Self::new(value)
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

    pub fn coordinate_space(&self) -> Expr {
        field(self.geometry(), "coordinate_space")
    }

    pub fn columns(&self) -> Expr {
        field(self.geometry(), "columns")
    }

    pub fn rows(&self) -> Expr {
        field(self.geometry(), "rows")
    }

    pub fn columns_coord(&self) -> Expr {
        field(self.columns(), "coord")
    }

    pub fn columns_sampling(&self) -> Expr {
        field(self.columns(), "sampling")
    }

    pub fn columns_start(&self) -> Expr {
        field(self.columns(), "start")
    }

    pub fn columns_stop(&self) -> Expr {
        field(self.columns(), "stop")
    }

    pub fn columns_count(&self) -> Expr {
        field(self.columns(), "count")
    }

    pub fn rows_coord(&self) -> Expr {
        field(self.rows(), "coord")
    }

    pub fn rows_sampling(&self) -> Expr {
        field(self.rows(), "sampling")
    }

    pub fn rows_start(&self) -> Expr {
        field(self.rows(), "start")
    }

    pub fn rows_stop(&self) -> Expr {
        field(self.rows(), "stop")
    }

    pub fn rows_count(&self) -> Expr {
        field(self.rows(), "count")
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
