use std::sync::Arc;

use avenger_chart_cartesian::{
    Cartesian, CartesianAreaPositionChannels, CartesianAxis, CartesianPositionConfig,
};
use avenger_chart_core::IntoExpr;
use avenger_chart_core::{
    AvengerChartError, ChannelConfig, ChannelExpr, ChannelValue, ColorChannelConfig,
    CoordinationScope, DataContext, DataTransform, DataTransformCompileContext,
    DefaultLogicalExprNodeExt, FacetDataScope, Mark, MarkDataMode, MarkGroup, OpacityChannelConfig,
    PlotMark, PositionConfig, StoreData, StrokeDashChannelConfig, StrokeWidthChannelConfig,
};
use avenger_chart_marks::Area;
use avenger_chart_transforms::{Calculate, JoinAggregate, Kde, KdeResolve};
use datafusion::prelude::{Expr, col, lit};
use datafusion_proto::protobuf::LogicalExprNode;

use crate::marks::compound::{
    CompoundGrouping, band_scale_hint_for_grouping, compound_grouping_from_position,
    validate_preserved_style_channel,
};

const VIOLIN_SAMPLE_FIELD: &str = "__avenger_violin_sample";
const VIOLIN_DENSITY_FIELD: &str = "__avenger_violin_density";
const VIOLIN_MAX_DENSITY_FIELD: &str = "__avenger_violin_max_density";
const VIOLIN_HALF_WIDTH_FIELD: &str = "__avenger_violin_half_width";
const VIOLIN_BAND_START_FIELD: &str = "__avenger_violin_band_start";
const VIOLIN_BAND_END_FIELD: &str = "__avenger_violin_band_end";

/// Compound violin mark built from ordinary chart primitives.
///
/// `Violin` expands into a root [`MarkGroup`] with a KDE transform, width
/// normalization transforms, and one generated `Area` body. The grouping axis
/// must be categorical: either a source column or a nested categorical position
/// such as `nested(["division", "team"])`. The value axis supplies the
/// continuous samples for KDE.
///
/// Body paths are split by the grouping source columns through `details`, so
/// a coarser style channel such as `fill(col("division"))` can color multiple
/// independent nested violins without merging their geometry.
///
/// If the mark has an id, the generated body is targetable by rooted event or
/// scene-query paths such as `my_violin.body`.
#[derive(Clone)]
pub struct Violin {
    id: Option<String>,
    data: DataContext,
    data_mode: MarkDataMode,
    facet_data_scope: FacetDataScope,
    x: Option<ViolinPositionChannel>,
    y: Option<ViolinPositionChannel>,
    orientation: Option<ViolinOrientation>,
    bandwidth: Expr,
    steps: Expr,
    density_extent: Option<(Expr, Expr)>,
    counts: bool,
    density_extent_resolve: KdeResolve,
    density_data_scope: CoordinationScope,
    width: f64,
    width_normalization: ViolinWidthNormalization,
    body_style: ViolinBodyStyle,
}

/// Orientation of a [`Violin`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViolinOrientation {
    /// Quantitative values are encoded on `x`; groups are encoded on `y`.
    Horizontal,
    /// Quantitative values are encoded on `y`; groups are encoded on `x`.
    Vertical,
}

/// Width normalization strategy for generated violin bodies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViolinWidthNormalization {
    /// Normalize all violins in the density execution scope by one maximum.
    #[default]
    Shared,
    /// Normalize each violin independently by its own maximum.
    PerViolin,
}

#[derive(Clone)]
struct ViolinPositionChannel {
    expr: Expr,
    value: ChannelValue,
    axis: Option<CartesianAxis>,
}

/// Style overrides for the generated violin body.
#[derive(Clone, Default)]
pub struct ViolinBodyStyle {
    fill: Option<ChannelValue>,
    stroke: Option<ChannelValue>,
    stroke_width: Option<ChannelValue>,
    stroke_dash: Option<ChannelValue>,
    opacity: Option<ChannelValue>,
}

impl Default for Violin {
    fn default() -> Self {
        Self {
            id: None,
            data: DataContext::default(),
            data_mode: MarkDataMode::Inherit,
            facet_data_scope: FacetDataScope::FILTERED,
            x: None,
            y: None,
            orientation: None,
            bandwidth: lit(0.0),
            steps: lit(200.0),
            density_extent: None,
            counts: false,
            density_extent_resolve: KdeResolve::Independent,
            density_data_scope: CoordinationScope::Free,
            width: 0.84,
            width_normalization: ViolinWidthNormalization::Shared,
            body_style: ViolinBodyStyle::default(),
        }
    }
}

impl ViolinBodyStyle {
    pub fn fill(mut self, value: impl Into<ChannelValue>) -> Self {
        self.fill = Some(value.into());
        self
    }

    pub fn fill_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(ColorChannelConfig) -> ColorChannelConfig,
    {
        self.fill = Some(f(ColorChannelConfig::new(value.into())).into_inner());
        self
    }

    pub fn stroke(mut self, value: impl Into<ChannelValue>) -> Self {
        self.stroke = Some(value.into());
        self
    }

    pub fn stroke_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(ColorChannelConfig) -> ColorChannelConfig,
    {
        self.stroke = Some(f(ColorChannelConfig::new(value.into())).into_inner());
        self
    }

    pub fn stroke_width(mut self, value: impl Into<ChannelValue>) -> Self {
        self.stroke_width = Some(value.into());
        self
    }

    pub fn stroke_width_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(StrokeWidthChannelConfig) -> StrokeWidthChannelConfig,
    {
        self.stroke_width = Some(f(StrokeWidthChannelConfig::new(value.into())).into_inner());
        self
    }

    pub fn stroke_dash(mut self, value: impl Into<ChannelValue>) -> Self {
        self.stroke_dash = Some(value.into());
        self
    }

    pub fn stroke_dash_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(StrokeDashChannelConfig) -> StrokeDashChannelConfig,
    {
        self.stroke_dash = Some(f(StrokeDashChannelConfig::new(value.into())).into_inner());
        self
    }

    pub fn opacity(mut self, value: impl Into<ChannelValue>) -> Self {
        self.opacity = Some(value.into());
        self
    }

    pub fn opacity_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(OpacityChannelConfig) -> OpacityChannelConfig,
    {
        self.opacity = Some(f(OpacityChannelConfig::new(value.into())).into_inner());
        self
    }
}

impl Violin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn data(mut self, dataframe: datafusion::dataframe::DataFrame) -> Self {
        self.data = DataContext::new(dataframe);
        self.data_mode = MarkDataMode::Inherit;
        self
    }

    pub fn data_store(mut self, data: StoreData) -> Self {
        self.data = DataContext::store_data(data);
        self.data_mode = MarkDataMode::Inherit;
        self
    }

    pub fn x<V>(self, value: V) -> Self
    where
        V: Into<ChannelExpr>,
    {
        self.x_with(value, |channel| channel)
    }

    pub fn x_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelExpr>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        self.x = Some(configure_position_channel(value, f));
        self
    }

    pub fn y<V>(self, value: V) -> Self
    where
        V: Into<ChannelExpr>,
    {
        self.y_with(value, |channel| channel)
    }

    pub fn y_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelExpr>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        self.y = Some(configure_position_channel(value, f));
        self
    }

    pub fn orientation(mut self, orientation: ViolinOrientation) -> Self {
        self.orientation = Some(orientation);
        self
    }

    pub fn horizontal(self) -> Self {
        self.orientation(ViolinOrientation::Horizontal)
    }

    pub fn vertical(self) -> Self {
        self.orientation(ViolinOrientation::Vertical)
    }

    pub fn bandwidth(mut self, bandwidth: impl IntoExpr) -> Self {
        self.bandwidth = bandwidth.into_expr();
        self
    }

    pub fn steps(mut self, steps: impl IntoExpr) -> Self {
        self.steps = steps.into_expr();
        self
    }

    pub fn density_extent(mut self, start: impl IntoExpr, stop: impl IntoExpr) -> Self {
        self.density_extent = Some((start.into_expr(), stop.into_expr()));
        self
    }

    pub fn counts(mut self, counts: bool) -> Self {
        self.counts = counts;
        self
    }

    pub fn density_extent_resolve(mut self, resolve: KdeResolve) -> Self {
        self.density_extent_resolve = resolve;
        self
    }

    pub fn density_data_scope(mut self, scope: CoordinationScope) -> Self {
        self.density_data_scope = scope;
        self
    }

    pub fn width(mut self, width: f64) -> Self {
        self.width = width;
        self
    }

    pub fn width_normalization(mut self, normalization: ViolinWidthNormalization) -> Self {
        self.width_normalization = normalization;
        self
    }

    pub fn body<F>(mut self, f: F) -> Self
    where
        F: FnOnce(ViolinBodyStyle) -> ViolinBodyStyle,
    {
        self.body_style = f(self.body_style);
        self
    }

    pub fn fill(mut self, value: impl Into<ChannelValue>) -> Self {
        self.body_style.fill = Some(value.into());
        self
    }

    pub fn fill_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(ColorChannelConfig) -> ColorChannelConfig,
    {
        self.body_style.fill = Some(f(ColorChannelConfig::new(value.into())).into_inner());
        self
    }

    pub fn stroke(mut self, value: impl Into<ChannelValue>) -> Self {
        self.body_style.stroke = Some(value.into());
        self
    }

    pub fn stroke_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(ColorChannelConfig) -> ColorChannelConfig,
    {
        self.body_style.stroke = Some(f(ColorChannelConfig::new(value.into())).into_inner());
        self
    }

    pub fn stroke_width(mut self, value: impl Into<ChannelValue>) -> Self {
        self.body_style.stroke_width = Some(value.into());
        self
    }

    pub fn stroke_width_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(StrokeWidthChannelConfig) -> StrokeWidthChannelConfig,
    {
        self.body_style.stroke_width =
            Some(f(StrokeWidthChannelConfig::new(value.into())).into_inner());
        self
    }

    pub fn stroke_dash(mut self, value: impl Into<ChannelValue>) -> Self {
        self.body_style.stroke_dash = Some(value.into());
        self
    }

    pub fn stroke_dash_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(StrokeDashChannelConfig) -> StrokeDashChannelConfig,
    {
        self.body_style.stroke_dash =
            Some(f(StrokeDashChannelConfig::new(value.into())).into_inner());
        self
    }

    pub fn opacity(mut self, value: impl Into<ChannelValue>) -> Self {
        self.body_style.opacity = Some(value.into());
        self
    }

    pub fn opacity_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(OpacityChannelConfig) -> OpacityChannelConfig,
    {
        self.body_style.opacity = Some(f(OpacityChannelConfig::new(value.into())).into_inner());
        self
    }

    pub fn transform<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_free(transform, f)
    }

    pub fn transform_free<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Free, transform, f)
    }

    pub fn transform_level<T, F>(self, level: u8, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Level(level), transform, f)
    }

    pub fn transform_shared<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Shared, transform, f)
    }

    pub fn transform_with_scope<T, F>(
        mut self,
        scope: CoordinationScope,
        transform: T,
        f: F,
    ) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        let scope = scope.to_normalized();
        let (compiled_transform, output) = transform
            .into_compiled_and_output(DataTransformCompileContext::new(scope))
            .expect("Failed to build data transform");
        self.data = self.data.with_transform_stage(scope, compiled_transform);
        f(self, output)
    }

    pub fn transform_no_output<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform<Output = ()>,
        F: FnOnce(Self) -> Self,
    {
        self.transform_free_no_output(transform, f)
    }

    pub fn transform_free_no_output<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform<Output = ()>,
        F: FnOnce(Self) -> Self,
    {
        self.transform_with_scope_no_output(CoordinationScope::Free, transform, f)
    }

    pub fn transform_level_no_output<T, F>(self, level: u8, transform: T, f: F) -> Self
    where
        T: DataTransform<Output = ()>,
        F: FnOnce(Self) -> Self,
    {
        self.transform_with_scope_no_output(CoordinationScope::Level(level), transform, f)
    }

    pub fn transform_shared_no_output<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform<Output = ()>,
        F: FnOnce(Self) -> Self,
    {
        self.transform_with_scope_no_output(CoordinationScope::Shared, transform, f)
    }

    pub fn transform_with_scope_no_output<T, F>(
        self,
        scope: CoordinationScope,
        transform: T,
        f: F,
    ) -> Self
    where
        T: DataTransform<Output = ()>,
        F: FnOnce(Self) -> Self,
    {
        self.transform_with_scope(scope, transform, |mark, ()| f(mark))
    }

    pub fn facet_data_scope(mut self, scope: FacetDataScope) -> Self {
        self.facet_data_scope = scope;
        self
    }

    pub fn facet_data_level(mut self, level: u8) -> Self {
        self.facet_data_scope = FacetDataScope::level(level);
        self
    }

    pub fn broadcast_to_facets(mut self) -> Self {
        self.facet_data_scope = FacetDataScope::BROADCAST;
        self
    }

    fn into_mark_group(self) -> Result<MarkGroup<Cartesian>, AvengerChartError> {
        let x = self.x.ok_or_else(|| {
            AvengerChartError::InvalidArgument("Violin requires an x channel".to_string())
        })?;
        let y = self.y.ok_or_else(|| {
            AvengerChartError::InvalidArgument("Violin requires a y channel".to_string())
        })?;
        if !self.width.is_finite() || self.width <= 0.0 || self.width > 1.0 {
            return Err(AvengerChartError::InvalidArgument(
                "Violin width must be finite and in the interval (0, 1]".to_string(),
            ));
        }

        let orientation = resolve_orientation(&x, &y, self.orientation);
        let value_channel = orientation.value_channel(&x, &y);
        let group_channel = orientation.group_channel(&x, &y);
        validate_violin_channels(orientation, value_channel)?;
        let grouping = violin_grouping(orientation, group_channel)?;
        validate_body_style_channels(&self.body_style, &grouping)?;
        let mut root = MarkGroup::new()
            .with_data_context(self.data, self.data_mode)
            .facet_data_scope(self.facet_data_scope);
        if let Some(hint) = band_scale_hint_for_grouping(
            &grouping,
            &group_channel.value,
            orientation.group_axis_name(),
        ) {
            root = root.with_scale_inference_hint(hint);
        }
        if let Some(id) = self.id {
            root = root.id(id);
        }
        let body = violin_body_mark(
            &x,
            &y,
            orientation,
            &grouping,
            self.facet_data_scope,
            ViolinBodyConfig {
                bandwidth: self.bandwidth,
                steps: self.steps,
                density_extent: self.density_extent,
                counts: self.counts,
                density_extent_resolve: self.density_extent_resolve,
                density_data_scope: self.density_data_scope,
                width: self.width,
                width_normalization: self.width_normalization,
                style: self.body_style,
            },
        );
        Ok(root.mark(body))
    }
}

struct ViolinBodyConfig {
    bandwidth: Expr,
    steps: Expr,
    density_extent: Option<(Expr, Expr)>,
    counts: bool,
    density_extent_resolve: KdeResolve,
    density_data_scope: CoordinationScope,
    width: f64,
    width_normalization: ViolinWidthNormalization,
    style: ViolinBodyStyle,
}

fn resolve_orientation(
    x: &ViolinPositionChannel,
    y: &ViolinPositionChannel,
    explicit: Option<ViolinOrientation>,
) -> ViolinOrientation {
    explicit.unwrap_or_else(|| {
        if x.value.get_nested_band_config().is_some() && y.value.get_nested_band_config().is_none()
        {
            ViolinOrientation::Vertical
        } else if y.value.get_nested_band_config().is_some()
            && x.value.get_nested_band_config().is_none()
        {
            ViolinOrientation::Horizontal
        } else {
            ViolinOrientation::Vertical
        }
    })
}

impl ViolinOrientation {
    fn value_axis_name(self) -> &'static str {
        match self {
            Self::Horizontal => "x",
            Self::Vertical => "y",
        }
    }

    fn group_axis_name(self) -> &'static str {
        match self {
            Self::Horizontal => "y",
            Self::Vertical => "x",
        }
    }

    fn area_orientation(self) -> &'static str {
        match self {
            Self::Horizontal => "vertical",
            Self::Vertical => "horizontal",
        }
    }

    fn value_channel<'a>(
        self,
        x: &'a ViolinPositionChannel,
        y: &'a ViolinPositionChannel,
    ) -> &'a ViolinPositionChannel {
        match self {
            Self::Horizontal => x,
            Self::Vertical => y,
        }
    }

    fn group_channel<'a>(
        self,
        x: &'a ViolinPositionChannel,
        y: &'a ViolinPositionChannel,
    ) -> &'a ViolinPositionChannel {
        match self {
            Self::Horizontal => y,
            Self::Vertical => x,
        }
    }
}

fn validate_violin_channels(
    orientation: ViolinOrientation,
    value_channel: &ViolinPositionChannel,
) -> Result<(), AvengerChartError> {
    if value_channel.value.get_nested_band_config().is_some() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Violin {} value channel cannot use nested band positioning; use {} for grouping or choose the opposite orientation",
            orientation.value_axis_name(),
            orientation.group_axis_name()
        )));
    }
    Ok(())
}

impl avenger_chart_core::IntoPlotMark<Cartesian> for Violin {
    fn into_plot_marks(self) -> Vec<PlotMark<Cartesian>> {
        match self.into_mark_group() {
            Ok(group) => vec![PlotMark::from_group(group)],
            Err(AvengerChartError::InvalidArgument(message)) => {
                vec![PlotMark::from_invalid_argument(message)]
            }
            Err(err) => vec![PlotMark::from_invalid_argument(err.to_string())],
        }
    }
}

fn configure_position_channel(
    value: impl Into<ChannelExpr>,
    f: impl FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
) -> ViolinPositionChannel {
    let value = value.into();
    let expr = value.data_expr().clone();
    let channel_value = value.into_channel_value();
    let config = CartesianPositionConfig::new(channel_value);
    let configured = f(config);
    let (value, axis) = configured.take_axis_config();
    ViolinPositionChannel { expr, value, axis }
}

fn violin_body_mark(
    x: &ViolinPositionChannel,
    y: &ViolinPositionChannel,
    orientation: ViolinOrientation,
    grouping: &CompoundGrouping,
    facet_data_scope: FacetDataScope,
    config: ViolinBodyConfig,
) -> Area<Cartesian> {
    let value_channel = orientation.value_channel(x, y);
    let group_channel = orientation.group_channel(x, y);
    let mut kde = Kde::new(value_channel.expr.clone())
        .group_by(grouping.key_exprs.clone())
        .bandwidth(config.bandwidth)
        .counts(config.counts)
        .resolve(config.density_extent_resolve)
        .steps(config.steps)
        .as_fields(VIOLIN_SAMPLE_FIELD, VIOLIN_DENSITY_FIELD);
    if let Some((start, stop)) = config.density_extent {
        kde = kde.extent(start, stop);
    }

    let density_scope = config.density_data_scope;
    let width_normalization = config.width_normalization;
    let width = config.width;
    let style = config.style;
    let grouping_for_max = grouping.clone();
    let grouping_for_area = grouping.clone();

    Area::new()
        .facet_data_scope(facet_data_scope)
        .transform_with_scope(density_scope, kde, move |area, kde| {
            let max_density = match width_normalization {
                ViolinWidthNormalization::Shared => {
                    JoinAggregate::new().max(VIOLIN_MAX_DENSITY_FIELD, kde.density())
                }
                ViolinWidthNormalization::PerViolin => JoinAggregate::new()
                    .group_by(grouping_for_max.key_exprs.clone())
                    .max(VIOLIN_MAX_DENSITY_FIELD, kde.density()),
            };
            area.transform_with_scope_no_output(density_scope, max_density, move |area| {
                area.transform_with_scope_no_output(
                    density_scope,
                    Calculate::new()
                        .expr(
                            VIOLIN_HALF_WIDTH_FIELD,
                            lit(width / 2.0) * col(VIOLIN_DENSITY_FIELD)
                                / col(VIOLIN_MAX_DENSITY_FIELD),
                        )
                        .expr(
                            VIOLIN_BAND_START_FIELD,
                            lit(0.5) - col(VIOLIN_HALF_WIDTH_FIELD),
                        )
                        .expr(
                            VIOLIN_BAND_END_FIELD,
                            lit(0.5) + col(VIOLIN_HALF_WIDTH_FIELD),
                        ),
                    move |area| {
                        violin_body_area(
                            area,
                            x,
                            y,
                            orientation,
                            group_channel,
                            value_channel,
                            kde.value(),
                            &grouping_for_area,
                            &style,
                        )
                    },
                )
            })
        })
}

fn violin_body_area(
    base: Area<Cartesian>,
    x: &ViolinPositionChannel,
    y: &ViolinPositionChannel,
    orientation: ViolinOrientation,
    group_channel: &ViolinPositionChannel,
    value_channel: &ViolinPositionChannel,
    kde_value: Expr,
    grouping: &CompoundGrouping,
    style: &ViolinBodyStyle,
) -> Area<Cartesian> {
    let mut area = match orientation {
        ViolinOrientation::Horizontal => base
            .id("body")
            .orientation(orientation.area_orientation())
            .x(value_with_expr(value_channel, kde_value.clone()))
            .x2(value_with_expr(value_channel, kde_value))
            .y(position_band_expr(
                group_channel,
                col(VIOLIN_BAND_START_FIELD),
            ))
            .y2(position_band_expr(
                group_channel,
                col(VIOLIN_BAND_END_FIELD),
            )),
        ViolinOrientation::Vertical => base
            .id("body")
            .orientation(orientation.area_orientation())
            .x(position_band_expr(
                group_channel,
                col(VIOLIN_BAND_START_FIELD),
            ))
            .x2(position_band_expr(
                group_channel,
                col(VIOLIN_BAND_END_FIELD),
            ))
            .y(value_with_expr(value_channel, kde_value.clone()))
            .y2(value_with_expr(value_channel, kde_value)),
    };
    area = area
        .details(grouping.key_names.clone())
        .fill("#93c5fd")
        .stroke("#1f2937")
        .stroke_width(1.0)
        .opacity(0.72)
        .order(ChannelValue::from(col(VIOLIN_SAMPLE_FIELD)).no_scale());
    area = apply_body_style(area, style);
    let area = with_position_axis(area, "x", &x.axis);
    with_position_axis(area, "y", &y.axis)
}

fn value_with_expr(channel: &ViolinPositionChannel, expr: Expr) -> ChannelValue {
    channel.value.clone().with_expr(
        LogicalExprNode::from_expr(expr).expect("serialize generated Violin channel expression"),
    )
}

fn position_band_expr(channel: &ViolinPositionChannel, band: Expr) -> ChannelValue {
    channel.value.clone().band(band)
}

fn apply_body_style(mut mark: Area<Cartesian>, style: &ViolinBodyStyle) -> Area<Cartesian> {
    if let Some(fill) = &style.fill {
        mark = mark.fill(fill.clone());
    }
    if let Some(stroke) = &style.stroke {
        mark = mark.stroke(stroke.clone());
    }
    if let Some(stroke_width) = &style.stroke_width {
        mark = mark.stroke_width(stroke_width.clone());
    }
    if let Some(stroke_dash) = &style.stroke_dash {
        mark = mark.stroke_dash(stroke_dash.clone());
    }
    if let Some(opacity) = &style.opacity {
        mark = mark.opacity(opacity.clone());
    }
    mark
}

fn with_position_axis<M>(mut mark: M, channel: &str, axis: &Option<CartesianAxis>) -> M
where
    M: Mark<Cartesian>,
{
    if let Some(axis) = axis {
        mark.state_mut()
            .axis_configs
            .insert(channel.to_string(), Arc::new(axis.clone()));
    }
    mark
}

fn violin_grouping(
    orientation: ViolinOrientation,
    channel: &ViolinPositionChannel,
) -> Result<CompoundGrouping, AvengerChartError> {
    compound_grouping_from_position(
        &format!("Violin {} grouping channel", orientation.group_axis_name()),
        &channel.expr,
        &channel.value,
    )
}

fn validate_body_style_channels(
    style: &ViolinBodyStyle,
    grouping: &CompoundGrouping,
) -> Result<(), AvengerChartError> {
    validate_preserved_style_channel("Violin body fill", style.fill.as_ref(), grouping)?;
    validate_preserved_style_channel("Violin body stroke", style.stroke.as_ref(), grouping)?;
    validate_preserved_style_channel(
        "Violin body stroke_width",
        style.stroke_width.as_ref(),
        grouping,
    )?;
    validate_preserved_style_channel(
        "Violin body stroke_dash",
        style.stroke_dash.as_ref(),
        grouping,
    )?;
    validate_preserved_style_channel("Violin body opacity", style.opacity.as_ref(), grouping)
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_chart_core::{ScaleInferenceHint, ScaleTypePreference, nested};
    use datafusion::{
        arrow::{
            array::{ArrayRef, Float64Array, StringArray},
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        prelude::SessionContext,
    };
    use std::sync::Arc;

    fn string_group_df(
        ctx: &SessionContext,
    ) -> datafusion::error::Result<datafusion::dataframe::DataFrame> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("group", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["A", "A", "B", "B"])) as ArrayRef,
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0])) as ArrayRef,
            ],
        )?;
        ctx.read_batch(batch)
    }

    fn nested_group_df(
        ctx: &SessionContext,
    ) -> datafusion::error::Result<datafusion::dataframe::DataFrame> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("segment", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["A", "A", "B", "B"])) as ArrayRef,
                Arc::new(StringArray::from(vec!["one", "two", "one", "two"])) as ArrayRef,
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0])) as ArrayRef,
            ],
        )?;
        ctx.read_batch(batch)
    }

    async fn scale_type_for_violin(
        ctx: &SessionContext,
        data: datafusion::dataframe::DataFrame,
        mark: Violin,
        channel: &str,
    ) -> String {
        let compiled = crate::plot::Plot::<Cartesian>::new()
            .data(data.clone())
            .mark(mark)
            .compile(ctx)
            .await
            .expect("compile violin");
        let scales = compiled
            .build_scales_for_dataframe(&data, 300.0, 200.0, ctx, compiled.get_default_params())
            .await
            .expect("build scales");
        scales
            .get(channel)
            .unwrap_or_else(|| panic!("{channel} scale"))
            .configured()
            .scale_impl
            .scale_type()
            .to_string()
    }

    #[test]
    fn orientation_defaults_and_explicit_overrides() {
        let x_nested =
            configure_position_channel(nested(["category", "segment"]), |channel| channel);
        let y_value = configure_position_channel(col("value"), |channel| channel);
        assert_eq!(
            resolve_orientation(&x_nested, &y_value, None),
            ViolinOrientation::Vertical
        );
        assert_eq!(
            resolve_orientation(&y_value, &x_nested, None),
            ViolinOrientation::Horizontal
        );
        assert_eq!(
            resolve_orientation(&y_value, &x_nested, Some(ViolinOrientation::Vertical)),
            ViolinOrientation::Vertical
        );
        assert_eq!(
            resolve_orientation(&y_value, &y_value, None),
            ViolinOrientation::Vertical
        );
    }

    fn violin_error(mark: Violin) -> String {
        match mark.into_mark_group() {
            Ok(_) => panic!("violin should fail validation"),
            Err(err) => err.to_string(),
        }
    }

    #[test]
    fn nested_band_value_axis_is_invalid_even_with_explicit_orientation() {
        let err = violin_error(
            Violin::new()
                .horizontal()
                .x(nested(["category", "segment"]))
                .y(col("value")),
        );
        assert!(
            err.contains("x value channel cannot use nested band"),
            "{err}"
        );

        let err = violin_error(
            Violin::new()
                .vertical()
                .x(col("group"))
                .y(nested(["category", "segment"])),
        );
        assert!(
            err.contains("y value channel cannot use nested band"),
            "{err}"
        );
    }

    #[test]
    fn arbitrary_group_expression_is_invalid_without_source_columns() {
        let err = violin_error(Violin::new().x(lit("all")).y(col("value")));
        assert!(
            err.contains("x grouping channel must be a source column or nested"),
            "{err}"
        );

        let err = violin_error(
            Violin::new()
                .horizontal()
                .x(col("value"))
                .y(col("a") + col("b")),
        );
        assert!(
            err.contains("y grouping channel must be a source column or nested"),
            "{err}"
        );
    }

    #[test]
    fn width_must_be_inside_unit_interval() {
        let err = violin_error(Violin::new().x(col("group")).y(col("value")).width(0.0));
        assert!(err.contains("width must be finite"), "{err}");

        let err = violin_error(Violin::new().x(col("group")).y(col("value")).width(1.5));
        assert!(err.contains("width must be finite"), "{err}");
    }

    #[test]
    fn violin_adds_band_hint_for_non_nested_grouping() {
        let group = Violin::new()
            .x(col("group"))
            .y(col("value"))
            .into_mark_group()
            .expect("violin group");
        assert_eq!(
            group.scale_inference_hints(),
            &[ScaleInferenceHint::new("x", ScaleTypePreference::Band)]
        );
    }

    #[test]
    fn violin_does_not_add_band_hint_for_nested_grouping() {
        let group = Violin::new()
            .x(nested(["category", "segment"]))
            .y(col("value"))
            .into_mark_group()
            .expect("violin group");
        assert!(group.scale_inference_hints().is_empty());
    }

    #[test]
    fn style_expression_must_be_preserved_grouping_column() {
        let err = violin_error(
            Violin::new()
                .x(col("group"))
                .y(col("value"))
                .fill(col("unrelated")),
        );
        assert!(err.contains("references column 'unrelated'"), "{err}");
    }

    #[tokio::test]
    async fn violin_string_grouping_infers_band_scale() {
        let ctx = SessionContext::new();
        let data = string_group_df(&ctx).expect("dataframe");
        let scale_type = scale_type_for_violin(
            &ctx,
            data,
            Violin::new().x(col("group")).y(col("value")),
            "x",
        )
        .await;
        assert_eq!(scale_type, "band");
    }

    #[tokio::test]
    async fn violin_nested_grouping_infers_nested_band_scale() {
        let ctx = SessionContext::new();
        let data = nested_group_df(&ctx).expect("dataframe");
        let scale_type = scale_type_for_violin(
            &ctx,
            data,
            Violin::new()
                .x(nested(["category", "segment"]))
                .y(col("value")),
            "x",
        )
        .await;
        assert_eq!(scale_type, "nested_band");
    }
}
