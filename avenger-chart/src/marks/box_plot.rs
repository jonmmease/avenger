use std::sync::Arc;

use avenger_chart_cartesian::{
    Cartesian, CartesianAxis, CartesianPositionConfig, CartesianRectPositionChannels,
    CartesianRulePositionChannels, CartesianSymbolPositionChannels,
};
use avenger_chart_core::IntoExpr;
use avenger_chart_core::{
    AngleChannelConfig, AvengerChartError, ChannelConfig, ChannelExpr, ChannelValue,
    ColorChannelConfig, CoordinationScope, DataContext, DataTransform, DataTransformCompileContext,
    DefaultLogicalExprNodeExt, FacetDataScope, Mark, MarkDataMode, MarkGroup, OpacityChannelConfig,
    PlotMark, PositionConfig, ShapeChannelConfig, SizeChannelConfig, StoreData,
    StrokeWidthChannelConfig,
};
use avenger_chart_marks::{Rect, Rule, Symbol};
use avenger_chart_transforms::{Aggregate, Filter, JoinAggregate};
use datafusion::prelude::{Expr, col, lit};
use datafusion_proto::protobuf::LogicalExprNode;

pub const BOX_PLOT_Q1_FIELD: &str = "q1";
pub const BOX_PLOT_MEDIAN_FIELD: &str = "median";
pub const BOX_PLOT_Q3_FIELD: &str = "q3";
pub const BOX_PLOT_WHISKER_LOW_FIELD: &str = "whisker_low";
pub const BOX_PLOT_WHISKER_HIGH_FIELD: &str = "whisker_high";

/// Compound box plot mark built from ordinary chart primitives.
///
/// `BoxPlot` expands into a root [`MarkGroup`] with generated `Rect`, `Rule`,
/// and `Symbol` marks plus aggregate/filter transforms. Authors can apply data
/// and transforms to the compound mark as they would for a primitive mark; the
/// generated branches then derive box summaries, whiskers, and raw outliers
/// from that inherited data.
///
/// Grouping is defined by the categorical position channel. For horizontal box
/// plots, `x` is the quantitative value and `y` is the group. For vertical box
/// plots, `y` is the quantitative value and `x` is the group. Nested-band
/// positions such as `nested(["category", "segment"])` group by every nested
/// source column.
///
/// Style channels do not create additional groups. A style column used on the
/// aggregate-backed parts must be one of the grouping source columns; otherwise
/// compilation returns an error. Outlier style channels are evaluated against
/// the raw outlier rows.
///
/// If the mark has an id, generated parts are targetable by rooted event or
/// scene-query paths:
///
/// - `my_box_plot.box`
/// - `my_box_plot.median`
/// - `my_box_plot.whiskers`
/// - `my_box_plot.lower_cap`
/// - `my_box_plot.upper_cap`
/// - `my_box_plot.outliers`
#[derive(Clone)]
pub struct BoxPlot {
    id: Option<String>,
    data: DataContext,
    data_mode: MarkDataMode,
    facet_data_scope: FacetDataScope,
    x: Option<BoxPlotPositionChannel>,
    y: Option<BoxPlotPositionChannel>,
    orientation: Option<BoxPlotOrientation>,
    fill: Option<BoxPlotStyleChannel>,
    box_style: BoxPlotBoxStyle,
    median_style: BoxPlotRuleStyle,
    whisker_style: BoxPlotRuleStyle,
    cap_style: BoxPlotRuleStyle,
    outlier_style: BoxPlotOutlierStyle,
    extent: f64,
}

/// Orientation of a [`BoxPlot`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxPlotOrientation {
    /// Quantitative values are encoded on `x`; groups are encoded on `y`.
    Horizontal,
    /// Quantitative values are encoded on `y`; groups are encoded on `x`.
    Vertical,
}

#[derive(Clone)]
struct BoxPlotPositionChannel {
    expr: Expr,
    value: ChannelValue,
    axis: Option<CartesianAxis>,
}

#[derive(Clone)]
struct BoxPlotStyleChannel {
    value: ChannelValue,
}

/// Style overrides for the generated box body.
#[derive(Clone, Default)]
pub struct BoxPlotBoxStyle {
    fill: Option<ChannelValue>,
    stroke: Option<ChannelValue>,
    stroke_width: Option<ChannelValue>,
    opacity: Option<ChannelValue>,
    band_start: Option<f64>,
    band_end: Option<f64>,
}

/// Style overrides for generated rule parts such as median, whiskers, and caps.
#[derive(Clone, Default)]
pub struct BoxPlotRuleStyle {
    stroke: Option<ChannelValue>,
    stroke_width: Option<ChannelValue>,
    opacity: Option<ChannelValue>,
    band_start: Option<f64>,
    band_end: Option<f64>,
}

/// Style overrides for generated outlier symbols.
///
/// These mirror `Symbol` non-position styling channels. Position, data, and
/// transforms remain owned by [`BoxPlot`] so the outlier branch can stay aligned
/// with the box statistics.
#[derive(Clone, Default)]
pub struct BoxPlotOutlierStyle {
    size: Option<ChannelValue>,
    fill: Option<ChannelValue>,
    stroke: Option<ChannelValue>,
    stroke_width: Option<ChannelValue>,
    shape: Option<ChannelValue>,
    angle: Option<ChannelValue>,
    opacity: Option<ChannelValue>,
}

impl Default for BoxPlot {
    fn default() -> Self {
        Self {
            id: None,
            data: DataContext::default(),
            data_mode: MarkDataMode::Inherit,
            facet_data_scope: FacetDataScope::FILTERED,
            x: None,
            y: None,
            orientation: None,
            fill: None,
            box_style: BoxPlotBoxStyle::default(),
            median_style: BoxPlotRuleStyle::default(),
            whisker_style: BoxPlotRuleStyle::default(),
            cap_style: BoxPlotRuleStyle::default(),
            outlier_style: BoxPlotOutlierStyle::default(),
            extent: 1.5,
        }
    }
}

impl BoxPlotBoxStyle {
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

    pub fn band(mut self, start: f64, end: f64) -> Self {
        self.band_start = Some(start);
        self.band_end = Some(end);
        self
    }
}

impl BoxPlotRuleStyle {
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

    pub fn band(mut self, start: f64, end: f64) -> Self {
        self.band_start = Some(start);
        self.band_end = Some(end);
        self
    }
}

impl BoxPlotOutlierStyle {
    pub fn size(mut self, value: impl Into<ChannelValue>) -> Self {
        self.size = Some(value.into());
        self
    }

    pub fn size_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(SizeChannelConfig) -> SizeChannelConfig,
    {
        self.size = Some(f(SizeChannelConfig::new(value.into())).into_inner());
        self
    }

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

    pub fn shape(mut self, value: impl Into<ChannelValue>) -> Self {
        self.shape = Some(value.into());
        self
    }

    pub fn shape_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(ShapeChannelConfig) -> ShapeChannelConfig,
    {
        self.shape = Some(f(ShapeChannelConfig::new(value.into())).into_inner());
        self
    }

    pub fn angle(mut self, value: impl Into<ChannelValue>) -> Self {
        self.angle = Some(value.into());
        self
    }

    pub fn angle_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(AngleChannelConfig) -> AngleChannelConfig,
    {
        self.angle = Some(f(AngleChannelConfig::new(value.into())).into_inner());
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

impl BoxPlot {
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

    pub fn fill(mut self, value: impl Into<ChannelValue>) -> Self {
        self.fill = Some(BoxPlotStyleChannel {
            value: value.into(),
        });
        self
    }

    pub fn fill_with<V, F>(mut self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(ColorChannelConfig) -> ColorChannelConfig,
    {
        let config = ColorChannelConfig::new(value.into());
        self.fill = Some(BoxPlotStyleChannel {
            value: f(config).into_inner(),
        });
        self
    }

    pub fn orientation(mut self, orientation: BoxPlotOrientation) -> Self {
        self.orientation = Some(orientation);
        self
    }

    pub fn horizontal(self) -> Self {
        self.orientation(BoxPlotOrientation::Horizontal)
    }

    pub fn vertical(self) -> Self {
        self.orientation(BoxPlotOrientation::Vertical)
    }

    pub fn box_body<F>(mut self, f: F) -> Self
    where
        F: FnOnce(BoxPlotBoxStyle) -> BoxPlotBoxStyle,
    {
        self.box_style = f(self.box_style);
        self
    }

    pub fn box_fill(mut self, value: impl Into<ChannelValue>) -> Self {
        self.box_style.fill = Some(value.into());
        self
    }

    pub fn box_stroke(mut self, value: impl Into<ChannelValue>) -> Self {
        self.box_style.stroke = Some(value.into());
        self
    }

    pub fn box_stroke_width(mut self, value: impl Into<ChannelValue>) -> Self {
        self.box_style.stroke_width = Some(value.into());
        self
    }

    pub fn box_opacity(mut self, value: impl Into<ChannelValue>) -> Self {
        self.box_style.opacity = Some(value.into());
        self
    }

    pub fn median<F>(mut self, f: F) -> Self
    where
        F: FnOnce(BoxPlotRuleStyle) -> BoxPlotRuleStyle,
    {
        self.median_style = f(self.median_style);
        self
    }

    pub fn whiskers<F>(mut self, f: F) -> Self
    where
        F: FnOnce(BoxPlotRuleStyle) -> BoxPlotRuleStyle,
    {
        self.whisker_style = f(self.whisker_style);
        self
    }

    pub fn caps<F>(mut self, f: F) -> Self
    where
        F: FnOnce(BoxPlotRuleStyle) -> BoxPlotRuleStyle,
    {
        self.cap_style = f(self.cap_style);
        self
    }

    pub fn outliers<F>(mut self, f: F) -> Self
    where
        F: FnOnce(BoxPlotOutlierStyle) -> BoxPlotOutlierStyle,
    {
        self.outlier_style = f(self.outlier_style);
        self
    }

    pub fn extent(mut self, extent: f64) -> Self {
        self.extent = extent;
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
            AvengerChartError::InvalidArgument("BoxPlot requires an x value channel".to_string())
        })?;
        let y = self.y.ok_or_else(|| {
            AvengerChartError::InvalidArgument("BoxPlot requires a y grouping channel".to_string())
        })?;
        if !self.extent.is_finite() || self.extent < 0.0 {
            return Err(AvengerChartError::InvalidArgument(
                "BoxPlot extent must be finite and non-negative".to_string(),
            ));
        }

        let orientation = resolve_orientation(&x, &y, self.orientation);
        let value_channel = orientation.value_channel(&x, &y);
        let group_channel = orientation.group_channel(&x, &y);
        validate_box_plot_channels(orientation, value_channel, group_channel)?;
        let group_keys = group_key_exprs(group_channel);
        let group_key_names = group_key_names(group_channel);
        validate_summary_style_channels(
            self.fill.as_ref(),
            &self.box_style,
            &self.median_style,
            &self.whisker_style,
            &self.cap_style,
            &group_key_names,
        )?;
        let fence_branch = MarkGroup::new().transform_no_output(
            boxplot_fence_stats(group_keys.clone(), value_channel.expr.clone()),
            |group| {
                group
                    .mark(whisker_branch(
                        &x,
                        &y,
                        orientation,
                        self.extent,
                        &self.whisker_style,
                        &self.cap_style,
                    ))
                    .mark(outlier_branch(
                        &x,
                        &y,
                        orientation,
                        self.extent,
                        &self.outlier_style,
                    ))
            },
        );
        let summary_branch = box_summary_branch(
            &x,
            &y,
            orientation,
            self.fill.as_ref(),
            &self.box_style,
            &self.median_style,
        );
        let mut root = MarkGroup::new()
            .with_data_context(self.data, self.data_mode)
            .facet_data_scope(self.facet_data_scope);
        if let Some(id) = self.id {
            root = root.id(id);
        }
        Ok(root.mark(fence_branch).mark(summary_branch))
    }
}

fn resolve_orientation(
    x: &BoxPlotPositionChannel,
    y: &BoxPlotPositionChannel,
    explicit: Option<BoxPlotOrientation>,
) -> BoxPlotOrientation {
    explicit.unwrap_or_else(|| {
        if x.value.get_nested_band_config().is_some() && y.value.get_nested_band_config().is_none()
        {
            BoxPlotOrientation::Vertical
        } else {
            BoxPlotOrientation::Horizontal
        }
    })
}

impl BoxPlotOrientation {
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

    fn value_channel<'a>(
        self,
        x: &'a BoxPlotPositionChannel,
        y: &'a BoxPlotPositionChannel,
    ) -> &'a BoxPlotPositionChannel {
        match self {
            Self::Horizontal => x,
            Self::Vertical => y,
        }
    }

    fn group_channel<'a>(
        self,
        x: &'a BoxPlotPositionChannel,
        y: &'a BoxPlotPositionChannel,
    ) -> &'a BoxPlotPositionChannel {
        match self {
            Self::Horizontal => y,
            Self::Vertical => x,
        }
    }
}

fn validate_box_plot_channels(
    orientation: BoxPlotOrientation,
    value_channel: &BoxPlotPositionChannel,
    group_channel: &BoxPlotPositionChannel,
) -> Result<(), AvengerChartError> {
    if value_channel.value.get_nested_band_config().is_some() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "BoxPlot {} value channel cannot use nested band positioning; use {} for grouping or choose the opposite orientation",
            orientation.value_axis_name(),
            orientation.group_axis_name()
        )));
    }
    if group_key_names(group_channel).is_empty() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "BoxPlot {} grouping channel must be a source column or nested([...]) expression",
            orientation.group_axis_name()
        )));
    }
    Ok(())
}

impl avenger_chart_core::IntoPlotMark<Cartesian> for BoxPlot {
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
) -> BoxPlotPositionChannel {
    let value = value.into();
    let expr = value.data_expr().clone();
    let channel_value = value.into_channel_value();
    let config = CartesianPositionConfig::new(channel_value);
    let configured = f(config);
    let (value, axis) = configured.take_axis_config();
    BoxPlotPositionChannel { expr, value, axis }
}

fn box_summary_branch(
    x: &BoxPlotPositionChannel,
    y: &BoxPlotPositionChannel,
    orientation: BoxPlotOrientation,
    fill: Option<&BoxPlotStyleChannel>,
    box_style: &BoxPlotBoxStyle,
    median_style: &BoxPlotRuleStyle,
) -> MarkGroup<Cartesian> {
    let value_channel = orientation.value_channel(x, y);
    let group_channel = orientation.group_channel(x, y);
    MarkGroup::new().transform(
        boxplot_summary_stats(group_key_exprs(group_channel), value_channel.expr.clone()),
        |group, stats| {
            let (box_band_start, box_band_end) = style_band_pair(box_style, 0.26, 0.74);
            let mut box_mark = match orientation {
                BoxPlotOrientation::Horizontal => Rect::new()
                    .id("box")
                    .x(value_with_expr(
                        value_channel,
                        stats.output(BOX_PLOT_Q1_FIELD),
                    ))
                    .x2(value_with_expr(
                        value_channel,
                        stats.output(BOX_PLOT_Q3_FIELD),
                    ))
                    .y(position_band(group_channel, box_band_start))
                    .y2(position_band(group_channel, box_band_end))
                    .stroke("#2563eb")
                    .stroke_width(1.5)
                    .zindex(3),
                BoxPlotOrientation::Vertical => Rect::new()
                    .id("box")
                    .x(position_band(group_channel, box_band_start))
                    .x2(position_band(group_channel, box_band_end))
                    .y(value_with_expr(
                        value_channel,
                        stats.output(BOX_PLOT_Q1_FIELD),
                    ))
                    .y2(value_with_expr(
                        value_channel,
                        stats.output(BOX_PLOT_Q3_FIELD),
                    ))
                    .stroke("#2563eb")
                    .stroke_width(1.5)
                    .zindex(3),
            };
            box_mark = if let Some(fill) = box_style
                .fill
                .as_ref()
                .or_else(|| fill.map(|fill| &fill.value))
            {
                box_mark.fill(fill.clone())
            } else {
                box_mark.fill("#bfdbfe")
            };
            box_mark = apply_box_style(box_mark, box_style);
            let box_mark = with_position_axis(box_mark, "x", &x.axis);
            let box_mark = with_position_axis(box_mark, "y", &y.axis);

            let (median_band_start, median_band_end) = style_band_pair(median_style, 0.24, 0.76);
            let median = match orientation {
                BoxPlotOrientation::Horizontal => Rule::new()
                    .id("median")
                    .x(value_with_expr(
                        value_channel,
                        stats.output(BOX_PLOT_MEDIAN_FIELD),
                    ))
                    .x2(value_with_expr(
                        value_channel,
                        stats.output(BOX_PLOT_MEDIAN_FIELD),
                    ))
                    .y(position_band(group_channel, median_band_start))
                    .y2(position_band(group_channel, median_band_end))
                    .stroke("#1e3a8a")
                    .stroke_width(2.2)
                    .zindex(4),
                BoxPlotOrientation::Vertical => Rule::new()
                    .id("median")
                    .x(position_band(group_channel, median_band_start))
                    .x2(position_band(group_channel, median_band_end))
                    .y(value_with_expr(
                        value_channel,
                        stats.output(BOX_PLOT_MEDIAN_FIELD),
                    ))
                    .y2(value_with_expr(
                        value_channel,
                        stats.output(BOX_PLOT_MEDIAN_FIELD),
                    ))
                    .stroke("#1e3a8a")
                    .stroke_width(2.2)
                    .zindex(4),
            };
            let median = apply_rule_style(median, median_style);

            group.mark(box_mark).mark(median)
        },
    )
}

fn whisker_branch(
    x: &BoxPlotPositionChannel,
    y: &BoxPlotPositionChannel,
    orientation: BoxPlotOrientation,
    extent: f64,
    whisker_style: &BoxPlotRuleStyle,
    cap_style: &BoxPlotRuleStyle,
) -> MarkGroup<Cartesian> {
    let value_channel = orientation.value_channel(x, y);
    let group_channel = orientation.group_channel(x, y);
    MarkGroup::new().transform_no_output(
        Filter::new(inlier_predicate(
            value_channel.expr.clone(),
            col(BOX_PLOT_Q1_FIELD),
            col(BOX_PLOT_Q3_FIELD),
            extent,
        )),
        |group| {
            group.transform(
                boxplot_whisker_stats(group_key_exprs(group_channel), value_channel.expr.clone()),
                |group, whiskers| {
                    let whisker_band = style_band_midpoint(whisker_style, 0.5);
                    let whiskers_mark = match orientation {
                        BoxPlotOrientation::Horizontal => Rule::new()
                            .id("whiskers")
                            .x(value_with_expr(
                                value_channel,
                                whiskers.output(BOX_PLOT_WHISKER_LOW_FIELD),
                            ))
                            .x2(value_with_expr(
                                value_channel,
                                whiskers.output(BOX_PLOT_WHISKER_HIGH_FIELD),
                            ))
                            .y(position_band(group_channel, whisker_band))
                            .y2(position_band(group_channel, whisker_band))
                            .stroke("#475569")
                            .stroke_width(1.5)
                            .zindex(1),
                        BoxPlotOrientation::Vertical => Rule::new()
                            .id("whiskers")
                            .x(position_band(group_channel, whisker_band))
                            .x2(position_band(group_channel, whisker_band))
                            .y(value_with_expr(
                                value_channel,
                                whiskers.output(BOX_PLOT_WHISKER_LOW_FIELD),
                            ))
                            .y2(value_with_expr(
                                value_channel,
                                whiskers.output(BOX_PLOT_WHISKER_HIGH_FIELD),
                            ))
                            .stroke("#475569")
                            .stroke_width(1.5)
                            .zindex(1),
                    };
                    let whiskers_mark = apply_rule_style(whiskers_mark, whisker_style);
                    let (cap_band_start, cap_band_end) = style_band_pair(cap_style, 0.32, 0.68);
                    let lower_cap = match orientation {
                        BoxPlotOrientation::Horizontal => Rule::new()
                            .id("lower_cap")
                            .x(value_with_expr(
                                value_channel,
                                whiskers.output(BOX_PLOT_WHISKER_LOW_FIELD),
                            ))
                            .x2(value_with_expr(
                                value_channel,
                                whiskers.output(BOX_PLOT_WHISKER_LOW_FIELD),
                            ))
                            .y(position_band(group_channel, cap_band_start))
                            .y2(position_band(group_channel, cap_band_end))
                            .stroke("#475569")
                            .stroke_width(1.5)
                            .zindex(2),
                        BoxPlotOrientation::Vertical => Rule::new()
                            .id("lower_cap")
                            .x(position_band(group_channel, cap_band_start))
                            .x2(position_band(group_channel, cap_band_end))
                            .y(value_with_expr(
                                value_channel,
                                whiskers.output(BOX_PLOT_WHISKER_LOW_FIELD),
                            ))
                            .y2(value_with_expr(
                                value_channel,
                                whiskers.output(BOX_PLOT_WHISKER_LOW_FIELD),
                            ))
                            .stroke("#475569")
                            .stroke_width(1.5)
                            .zindex(2),
                    };
                    let lower_cap = apply_rule_style(lower_cap, cap_style);
                    let upper_cap = match orientation {
                        BoxPlotOrientation::Horizontal => Rule::new()
                            .id("upper_cap")
                            .x(value_with_expr(
                                value_channel,
                                whiskers.output(BOX_PLOT_WHISKER_HIGH_FIELD),
                            ))
                            .x2(value_with_expr(
                                value_channel,
                                whiskers.output(BOX_PLOT_WHISKER_HIGH_FIELD),
                            ))
                            .y(position_band(group_channel, cap_band_start))
                            .y2(position_band(group_channel, cap_band_end))
                            .stroke("#475569")
                            .stroke_width(1.5)
                            .zindex(2),
                        BoxPlotOrientation::Vertical => Rule::new()
                            .id("upper_cap")
                            .x(position_band(group_channel, cap_band_start))
                            .x2(position_band(group_channel, cap_band_end))
                            .y(value_with_expr(
                                value_channel,
                                whiskers.output(BOX_PLOT_WHISKER_HIGH_FIELD),
                            ))
                            .y2(value_with_expr(
                                value_channel,
                                whiskers.output(BOX_PLOT_WHISKER_HIGH_FIELD),
                            ))
                            .stroke("#475569")
                            .stroke_width(1.5)
                            .zindex(2),
                    };
                    let upper_cap = apply_rule_style(upper_cap, cap_style);

                    group.mark(whiskers_mark).mark(lower_cap).mark(upper_cap)
                },
            )
        },
    )
}

fn outlier_branch(
    x: &BoxPlotPositionChannel,
    y: &BoxPlotPositionChannel,
    orientation: BoxPlotOrientation,
    extent: f64,
    outlier_style: &BoxPlotOutlierStyle,
) -> MarkGroup<Cartesian> {
    let value_channel = orientation.value_channel(x, y);
    let group_channel = orientation.group_channel(x, y);
    MarkGroup::new().transform_no_output(
        Filter::new(outlier_predicate(
            value_channel.expr.clone(),
            col(BOX_PLOT_Q1_FIELD),
            col(BOX_PLOT_Q3_FIELD),
            extent,
        )),
        |group| {
            let outliers = match orientation {
                BoxPlotOrientation::Horizontal => Symbol::new()
                    .id("outliers")
                    .x(value_channel.value.clone())
                    .y(position_band(group_channel, 0.5))
                    .fill("#f97316")
                    .stroke("#ffffff")
                    .stroke_width(1.25)
                    .size(95.0)
                    .zindex(5),
                BoxPlotOrientation::Vertical => Symbol::new()
                    .id("outliers")
                    .x(position_band(group_channel, 0.5))
                    .y(value_channel.value.clone())
                    .fill("#f97316")
                    .stroke("#ffffff")
                    .stroke_width(1.25)
                    .size(95.0)
                    .zindex(5),
            };
            group.mark(apply_outlier_style(outliers, outlier_style))
        },
    )
}

fn value_with_expr(channel: &BoxPlotPositionChannel, expr: Expr) -> ChannelValue {
    channel.value.clone().with_expr(
        LogicalExprNode::from_expr(expr).expect("serialize generated BoxPlot channel expression"),
    )
}

fn position_band(channel: &BoxPlotPositionChannel, band: f64) -> ChannelValue {
    channel.value.clone().band(band)
}

fn style_band_pair<S>(style: &S, default_start: f64, default_end: f64) -> (f64, f64)
where
    S: BoxPlotBandStyle,
{
    (
        style.band_start().unwrap_or(default_start),
        style.band_end().unwrap_or(default_end),
    )
}

fn style_band_midpoint<S>(style: &S, default: f64) -> f64
where
    S: BoxPlotBandStyle,
{
    let (start, end) = style_band_pair(style, default, default);
    (start + end) / 2.0
}

trait BoxPlotBandStyle {
    fn band_start(&self) -> Option<f64>;
    fn band_end(&self) -> Option<f64>;
}

impl BoxPlotBandStyle for BoxPlotBoxStyle {
    fn band_start(&self) -> Option<f64> {
        self.band_start
    }

    fn band_end(&self) -> Option<f64> {
        self.band_end
    }
}

impl BoxPlotBandStyle for BoxPlotRuleStyle {
    fn band_start(&self) -> Option<f64> {
        self.band_start
    }

    fn band_end(&self) -> Option<f64> {
        self.band_end
    }
}

fn apply_box_style(mut mark: Rect<Cartesian>, style: &BoxPlotBoxStyle) -> Rect<Cartesian> {
    if let Some(stroke) = &style.stroke {
        mark = mark.stroke(stroke.clone());
    }
    if let Some(stroke_width) = &style.stroke_width {
        mark = mark.stroke_width(stroke_width.clone());
    }
    if let Some(opacity) = &style.opacity {
        mark = mark.opacity(opacity.clone());
    }
    mark
}

fn apply_rule_style(mut mark: Rule<Cartesian>, style: &BoxPlotRuleStyle) -> Rule<Cartesian> {
    if let Some(stroke) = &style.stroke {
        mark = mark.stroke(stroke.clone());
    }
    if let Some(stroke_width) = &style.stroke_width {
        mark = mark.stroke_width(stroke_width.clone());
    }
    if let Some(opacity) = &style.opacity {
        mark = mark.opacity(opacity.clone());
    }
    mark
}

fn apply_outlier_style(
    mut mark: Symbol<Cartesian>,
    style: &BoxPlotOutlierStyle,
) -> Symbol<Cartesian> {
    if let Some(size) = &style.size {
        mark = mark.size(size.clone());
    }
    if let Some(fill) = &style.fill {
        mark = mark.fill(fill.clone());
    }
    if let Some(stroke) = &style.stroke {
        mark = mark.stroke(stroke.clone());
    }
    if let Some(stroke_width) = &style.stroke_width {
        mark = mark.stroke_width(stroke_width.clone());
    }
    if let Some(shape) = &style.shape {
        mark = mark.shape(shape.clone());
    }
    if let Some(angle) = &style.angle {
        mark = mark.angle(angle.clone());
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

fn group_key_exprs(channel: &BoxPlotPositionChannel) -> Vec<Expr> {
    channel
        .value
        .get_nested_band_config()
        .map(|nested| {
            nested
                .source_columns
                .iter()
                .map(|column| col(column.clone()))
                .collect()
        })
        .unwrap_or_else(|| vec![channel.expr.clone()])
}

fn group_key_names(channel: &BoxPlotPositionChannel) -> Vec<String> {
    channel
        .value
        .get_nested_band_config()
        .map(|nested| nested.source_columns.clone())
        .unwrap_or_else(|| simple_column_name(&channel.expr).into_iter().collect())
}

fn validate_summary_style_channels(
    fill: Option<&BoxPlotStyleChannel>,
    box_style: &BoxPlotBoxStyle,
    median_style: &BoxPlotRuleStyle,
    whisker_style: &BoxPlotRuleStyle,
    cap_style: &BoxPlotRuleStyle,
    group_key_names: &[String],
) -> Result<(), AvengerChartError> {
    validate_aggregate_style_channel(
        "BoxPlot fill",
        fill.map(|fill| &fill.value),
        group_key_names,
    )?;
    validate_aggregate_style_channel("BoxPlot box fill", box_style.fill.as_ref(), group_key_names)?;
    validate_aggregate_style_channel(
        "BoxPlot box stroke",
        box_style.stroke.as_ref(),
        group_key_names,
    )?;
    validate_aggregate_style_channel(
        "BoxPlot box stroke_width",
        box_style.stroke_width.as_ref(),
        group_key_names,
    )?;
    validate_aggregate_style_channel(
        "BoxPlot box opacity",
        box_style.opacity.as_ref(),
        group_key_names,
    )?;
    validate_aggregate_style_channel(
        "BoxPlot median stroke",
        median_style.stroke.as_ref(),
        group_key_names,
    )?;
    validate_aggregate_style_channel(
        "BoxPlot median stroke_width",
        median_style.stroke_width.as_ref(),
        group_key_names,
    )?;
    validate_aggregate_style_channel(
        "BoxPlot median opacity",
        median_style.opacity.as_ref(),
        group_key_names,
    )?;
    validate_aggregate_style_channel(
        "BoxPlot whisker stroke",
        whisker_style.stroke.as_ref(),
        group_key_names,
    )?;
    validate_aggregate_style_channel(
        "BoxPlot whisker stroke_width",
        whisker_style.stroke_width.as_ref(),
        group_key_names,
    )?;
    validate_aggregate_style_channel(
        "BoxPlot whisker opacity",
        whisker_style.opacity.as_ref(),
        group_key_names,
    )?;
    validate_aggregate_style_channel(
        "BoxPlot cap stroke",
        cap_style.stroke.as_ref(),
        group_key_names,
    )?;
    validate_aggregate_style_channel(
        "BoxPlot cap stroke_width",
        cap_style.stroke_width.as_ref(),
        group_key_names,
    )?;
    validate_aggregate_style_channel(
        "BoxPlot cap opacity",
        cap_style.opacity.as_ref(),
        group_key_names,
    )?;
    Ok(())
}

fn validate_aggregate_style_channel(
    label: &str,
    value: Option<&ChannelValue>,
    group_key_names: &[String],
) -> Result<(), AvengerChartError> {
    let Some(value) = value else {
        return Ok(());
    };
    if matches!(value, ChannelValue::Value { .. }) {
        return Ok(());
    }

    let ctx = datafusion::prelude::SessionContext::new();
    let Some(expr) = value.expr(&ctx) else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{label} must be a scalar value or a preserved grouping column"
        )));
    };
    if matches!(expr, Expr::Placeholder(_)) {
        return Ok(());
    }
    let Some(column) = simple_column_name(&expr) else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{label} expressions must be scalar values or preserved grouping columns"
        )));
    };
    if group_key_names.iter().any(|name| name == &column) {
        Ok(())
    } else {
        Err(AvengerChartError::InvalidArgument(format!(
            "{label} references column '{column}', but box summary rows are grouped only by {}",
            if group_key_names.is_empty() {
                "the categorical position expression".to_string()
            } else {
                group_key_names.join(", ")
            }
        )))
    }
}

fn simple_column_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Column(column) => Some(column.name.clone()),
        _ => None,
    }
}

pub fn boxplot_fence_stats<I, E, V>(group_keys: I, value_expr: V) -> JoinAggregate
where
    I: IntoIterator<Item = E>,
    E: IntoExpr,
    V: IntoExpr,
{
    let group_keys = into_exprs(group_keys);
    let value_expr = value_expr.into_expr();
    JoinAggregate::new()
        .group_by(group_keys)
        .approx_percentile_cont(BOX_PLOT_Q1_FIELD, value_expr.clone(), 0.25)
        .approx_percentile_cont(BOX_PLOT_Q3_FIELD, value_expr, 0.75)
}

pub fn boxplot_summary_stats<I, E, V>(group_keys: I, value_expr: V) -> Aggregate
where
    I: IntoIterator<Item = E>,
    E: IntoExpr,
    V: IntoExpr,
{
    let group_keys = into_exprs(group_keys);
    let value_expr = value_expr.into_expr();
    Aggregate::new()
        .group_by(group_keys)
        .approx_percentile_cont(BOX_PLOT_Q1_FIELD, value_expr.clone(), 0.25)
        .median(BOX_PLOT_MEDIAN_FIELD, value_expr.clone())
        .approx_percentile_cont(BOX_PLOT_Q3_FIELD, value_expr, 0.75)
}

pub fn boxplot_whisker_stats<I, E, V>(group_keys: I, value_expr: V) -> Aggregate
where
    I: IntoIterator<Item = E>,
    E: IntoExpr,
    V: IntoExpr,
{
    let group_keys = into_exprs(group_keys);
    let value_expr = value_expr.into_expr();
    Aggregate::new()
        .group_by(group_keys)
        .min(BOX_PLOT_WHISKER_LOW_FIELD, value_expr.clone())
        .max(BOX_PLOT_WHISKER_HIGH_FIELD, value_expr)
}

pub fn lower_fence(q1: impl IntoExpr, q3: impl IntoExpr, k: f64) -> Expr {
    let q1 = q1.into_expr();
    let q3 = q3.into_expr();
    q1.clone() - (q3 - q1) * lit(k)
}

pub fn upper_fence(q1: impl IntoExpr, q3: impl IntoExpr, k: f64) -> Expr {
    let q1 = q1.into_expr();
    let q3 = q3.into_expr();
    q3.clone() + (q3 - q1) * lit(k)
}

pub fn inlier_predicate(
    value: impl IntoExpr,
    q1: impl IntoExpr,
    q3: impl IntoExpr,
    k: f64,
) -> Expr {
    let value = value.into_expr();
    let q1 = q1.into_expr();
    let q3 = q3.into_expr();
    value
        .clone()
        .gt_eq(lower_fence(q1.clone(), q3.clone(), k))
        .and(value.lt_eq(upper_fence(q1, q3, k)))
}

pub fn outlier_predicate(
    value: impl IntoExpr,
    q1: impl IntoExpr,
    q3: impl IntoExpr,
    k: f64,
) -> Expr {
    let value = value.into_expr();
    let q1 = q1.into_expr();
    let q3 = q3.into_expr();
    value
        .clone()
        .lt(lower_fence(q1.clone(), q3.clone(), k))
        .or(value.gt(upper_fence(q1, q3, k)))
}

fn into_exprs<I, E>(exprs: I) -> Vec<Expr>
where
    I: IntoIterator<Item = E>,
    E: IntoExpr,
{
    exprs.into_iter().map(IntoExpr::into_expr).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_chart_core::nested;
    use datafusion::prelude::col;

    #[test]
    fn fence_expressions_match_tukey_formula() {
        assert_eq!(
            lower_fence(col("q1"), col("q3"), 1.5),
            col("q1") - (col("q3") - col("q1")) * lit(1.5)
        );
        assert_eq!(
            upper_fence(col("q1"), col("q3"), 1.5),
            col("q3") + (col("q3") - col("q1")) * lit(1.5)
        );
    }

    #[test]
    fn predicate_expressions_match_fence_tests() {
        assert_eq!(
            inlier_predicate(col("value"), col("q1"), col("q3"), 1.5),
            col("value")
                .gt_eq(lower_fence(col("q1"), col("q3"), 1.5))
                .and(col("value").lt_eq(upper_fence(col("q1"), col("q3"), 1.5)))
        );
        assert_eq!(
            outlier_predicate(col("value"), col("q1"), col("q3"), 1.5),
            col("value")
                .lt(lower_fence(col("q1"), col("q3"), 1.5))
                .or(col("value").gt(upper_fence(col("q1"), col("q3"), 1.5)))
        );
    }

    #[test]
    fn nested_position_group_keys_use_source_columns() {
        let channel = configure_position_channel(nested(["category", "segment"]), |channel| {
            channel.level_band(1, 0.5)
        });

        assert_eq!(
            group_key_exprs(&channel),
            vec![col("category"), col("segment")]
        );
        assert_eq!(
            group_key_names(&channel),
            vec!["category".to_string(), "segment".to_string()]
        );
        assert_eq!(
            channel
                .value
                .get_nested_band_config()
                .expect("nested config")
                .source_columns,
            vec!["category", "segment"]
        );
    }

    #[test]
    fn fill_must_reference_preserved_group_column() {
        let grouping =
            configure_position_channel(nested(["category", "segment"]), |channel| channel);
        let names = group_key_names(&grouping);
        let valid = BoxPlotStyleChannel {
            value: ChannelValue::from(col("segment")),
        };
        validate_aggregate_style_channel("BoxPlot fill", Some(&valid.value), &names)
            .expect("segment is preserved");

        let invalid = BoxPlotStyleChannel {
            value: ChannelValue::from(col("region")),
        };
        let err = validate_aggregate_style_channel("BoxPlot fill", Some(&invalid.value), &names)
            .expect_err("region is not preserved");
        assert!(err.to_string().contains("region"), "{err}");
    }

    #[test]
    fn orientation_defaults_and_explicit_overrides() {
        let x_nested =
            configure_position_channel(nested(["category", "segment"]), |channel| channel);
        let y_value = configure_position_channel(col("value"), |channel| channel);
        assert_eq!(
            resolve_orientation(&x_nested, &y_value, None),
            BoxPlotOrientation::Vertical
        );
        assert_eq!(
            resolve_orientation(&y_value, &x_nested, None),
            BoxPlotOrientation::Horizontal
        );
        assert_eq!(
            resolve_orientation(&y_value, &x_nested, Some(BoxPlotOrientation::Vertical)),
            BoxPlotOrientation::Vertical
        );
    }

    fn box_plot_error(mark: BoxPlot) -> String {
        match mark.into_mark_group() {
            Ok(_) => panic!("box plot should fail validation"),
            Err(err) => err.to_string(),
        }
    }

    #[test]
    fn nested_band_value_axis_is_invalid_even_with_explicit_orientation() {
        let err = box_plot_error(
            BoxPlot::new()
                .horizontal()
                .x(nested(["category", "segment"]))
                .y(col("value")),
        );
        assert!(
            err.contains("x value channel cannot use nested band"),
            "{err}"
        );

        let err = box_plot_error(
            BoxPlot::new()
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
        let err = box_plot_error(BoxPlot::new().x(col("value")).y(lit("all")));
        assert!(
            err.contains("y grouping channel must be a source column or nested"),
            "{err}"
        );

        let err = box_plot_error(
            BoxPlot::new()
                .vertical()
                .x(col("a") + col("b"))
                .y(col("value")),
        );
        assert!(
            err.contains("x grouping channel must be a source column or nested"),
            "{err}"
        );
    }
}
