use datafusion::{logical_expr::Expr, prelude::get_field};
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};

use avenger_chart_core::{
    Axis, ChannelConfig, ChannelValue, ColorChannelConfig, CoordinationScope, MarkState,
    OpacityChannelConfig, PrimitiveMarkEffects, Scale, ScaleChannelValue, ScaleSpec,
    define_common_mark_channels, impl_mark_base_with_extra_fields,
};

pub const UNIFORM_RASTER_2D_RASTER_CHANNEL: &str = "raster";
pub const UNIFORM_RASTER_2D_FILL_CHANNEL: &str = "fill";

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RasterDim {
    name: String,
}

impl RasterDim {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

pub fn dim(name: impl Into<String>) -> RasterDim {
    RasterDim::new(name)
}

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
