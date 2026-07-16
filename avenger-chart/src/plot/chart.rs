//! Root chart wrapper.
//!
//! [`Chart`] owns document-altitude authoring vocabulary while [`Plot`]
//! remains the position-neutral unit embedded by subplot containers. Plot-local
//! authoring methods are forwarded to the wrapped plot; root furnishings are
//! retained here and supplied explicitly at compilation.

use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChartTool, ChartWidget, CompiledParamSpec, CoordinationScope,
    FormattingContext, IntoExpr, IntoPlotMark, NativeWidget, Param, PixelFrame,
    PositionedChartWidget, PositionedNativeWidget, Scale, Selection, Store, Theme, TimeContext,
};
use avenger_chart_scales::ScaleSpec as ScaleTypeSpec;
use datafusion::{dataframe::DataFrame, prelude::SessionContext};

use crate::{
    coords::CoordinateSystem,
    event::{ChartEventBinding, ChartParamChangeBinding},
    layout::{CanvasConstraint, LayoutSpec, Margins, PlotConstraint},
    legend::Legend,
    scales::Auto,
};

use super::{CompiledPlot, Plot, PlotSubtitle, PlotTitle, RootChartFurnishings};

/// A root plot together with document-level furnishings.
#[derive(Clone)]
pub struct Chart<C: CoordinateSystem> {
    plot: Plot<C>,
    theme: Option<Arc<Theme>>,
    time_context: TimeContext,
    formatting_context: FormattingContext,
    layout_spec: LayoutSpec,
    title: Option<PlotTitle>,
    subtitle: Option<PlotSubtitle>,
    param_specs: Vec<CompiledParamSpec>,
    selections: Vec<Selection>,
    stores: Vec<Store>,
}

impl<C: CoordinateSystem> Chart<C> {
    /// Construct a chart around an explicit coordinate system.
    pub fn with_coord(coord_system: C) -> Self {
        Self {
            plot: Plot::with_coord(coord_system),
            theme: None,
            time_context: TimeContext::default(),
            formatting_context: FormattingContext::default(),
            layout_spec: LayoutSpec::default(),
            title: None,
            subtitle: None,
            param_specs: Vec::new(),
            selections: Vec::new(),
            stores: Vec::new(),
        }
    }

    /// Promote an already-authored position-neutral plot to a root chart.
    pub fn from_plot(plot: Plot<C>) -> Self {
        Self {
            plot,
            theme: None,
            time_context: TimeContext::default(),
            formatting_context: FormattingContext::default(),
            layout_spec: LayoutSpec::default(),
            title: None,
            subtitle: None,
            param_specs: Vec::new(),
            selections: Vec::new(),
            stores: Vec::new(),
        }
    }

    /// Apply plot-level configuration without adding another forwarding method.
    pub fn configure_plot(mut self, f: impl FnOnce(Plot<C>) -> Plot<C>) -> Self {
        self.plot = f(self.plot);
        self
    }

    /// Borrow the wrapped plot for introspection.
    pub fn plot(&self) -> &Plot<C> {
        &self.plot
    }

    /// Apply coordinate-system configuration through the coordinate builder.
    pub fn configure_coord(mut self, f: impl FnOnce(C) -> C) -> Self {
        self.plot = self.plot.configure_coord(f);
        self
    }

    /// Set chart data inherited by plot marks.
    pub fn data(mut self, data: DataFrame) -> Self {
        self.plot = self.plot.data(data);
        self
    }

    /// Add a plot mark.
    pub fn mark<M>(mut self, mark: M) -> Self
    where
        M: IntoPlotMark<C>,
    {
        self.plot = self.plot.mark(mark);
        self
    }

    /// Add one chart event binding.
    pub fn event_binding(mut self, binding: ChartEventBinding) -> Self {
        self.plot = self.plot.event_binding(binding);
        self
    }

    /// Add multiple chart event bindings.
    pub fn event_bindings(mut self, bindings: impl IntoIterator<Item = ChartEventBinding>) -> Self {
        self.plot = self.plot.event_bindings(bindings);
        self
    }

    /// Add one reaction to a registered shared parameter change.
    pub fn param_change_binding(mut self, binding: ChartParamChangeBinding) -> Self {
        self.plot = self.plot.param_change_binding(binding);
        self
    }

    /// Add multiple reactions to registered shared parameter changes.
    pub fn param_change_bindings(
        mut self,
        bindings: impl IntoIterator<Item = ChartParamChangeBinding>,
    ) -> Self {
        self.plot = self.plot.param_change_bindings(bindings);
        self
    }

    /// Add one authoring-time chart tool.
    pub fn tool<T: ChartTool<C>>(mut self, tool: T) -> Self {
        self.plot = self.plot.tool(tool);
        self
    }

    /// Add authoring-time chart tools of one concrete type.
    pub fn tools<T: ChartTool<C>>(mut self, tools: impl IntoIterator<Item = T>) -> Self {
        self.plot = self.plot.tools(tools);
        self
    }

    /// Attach a composed widget to a chart chrome side.
    pub fn widget<W: ChartWidget>(mut self, widget: PositionedChartWidget<W>) -> Self {
        self.plot = self.plot.widget(widget);
        self
    }

    /// Attach a native widget to a chart chrome side.
    pub fn native_widget<N: NativeWidget>(mut self, widget: PositionedNativeWidget<N>) -> Self {
        self.plot = self.plot.native_widget(widget);
        self
    }

    /// Configure the coordinate guide.
    pub fn configure_guide(mut self, guide: C::Guide) -> Self {
        self.plot = self.plot.configure_guide(guide);
        self
    }

    /// Configure a legend by channel name.
    pub fn legend<F>(mut self, channel: &str, f: F) -> Self
    where
        F: FnOnce(Legend) -> Legend,
    {
        self.plot = self.plot.legend(channel, f);
        self
    }

    /// Configure an inferred scale by channel name.
    pub fn scale<F>(mut self, channel: &str, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        self.plot = self.plot.scale(channel, f);
        self
    }

    /// Configure an explicitly typed scale by channel name.
    pub fn scale_with<S: ScaleTypeSpec + Default>(
        mut self,
        channel: &str,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        self.plot = self.plot.scale_with(channel, f);
        self
    }

    /// Set a chart title.
    pub fn title(mut self, text: impl IntoExpr) -> Self {
        self.title = Some(PlotTitle::new(text));
        self
    }

    /// Configure a chart title.
    pub fn configure_title<F>(mut self, text: impl IntoExpr, f: F) -> Self
    where
        F: FnOnce(PlotTitle) -> PlotTitle,
    {
        self.title = Some(f(PlotTitle::new(text)));
        self
    }

    /// Set a chart subtitle.
    pub fn subtitle(mut self, text: impl IntoExpr) -> Self {
        self.subtitle = Some(PlotSubtitle::new(text));
        self
    }

    /// Configure a chart subtitle.
    pub fn configure_subtitle<F>(mut self, text: impl IntoExpr, f: F) -> Self
    where
        F: FnOnce(PlotSubtitle) -> PlotSubtitle,
    {
        self.subtitle = Some(f(PlotSubtitle::new(text)));
        self
    }

    /// Access the configured title.
    pub fn get_title(&self) -> Option<&PlotTitle> {
        self.title.as_ref()
    }

    /// Access the configured subtitle.
    pub fn get_subtitle(&self) -> Option<&PlotSubtitle> {
        self.subtitle.as_ref()
    }

    /// Set fixed canvas dimensions.
    pub fn canvas_size<W: IntoExpr, H: IntoExpr>(mut self, width: W, height: H) -> Self {
        self.layout_spec.canvas = crate::layout::SizeMode::Fixed {
            width: crate::serialization::serializable_expr_from_expr(
                width.into_expr(),
                "canvas width",
            ),
            height: crate::serialization::serializable_expr_from_expr(
                height.into_expr(),
                "canvas height",
            ),
        };
        self
    }

    /// Set responsive canvas sizing.
    pub fn canvas_constraint(mut self, constraint: CanvasConstraint) -> Self {
        self.layout_spec.canvas = constraint.into();
        self
    }

    /// Set fixed root plot-area dimensions.
    pub fn plot_size<W: IntoExpr, H: IntoExpr>(mut self, width: W, height: H) -> Self {
        self.layout_spec.plot_area = crate::layout::SizeMode::Fixed {
            width: crate::serialization::serializable_expr_from_expr(
                width.into_expr(),
                "plot width",
            ),
            height: crate::serialization::serializable_expr_from_expr(
                height.into_expr(),
                "plot height",
            ),
        };
        self
    }

    /// Set responsive root plot-area sizing.
    pub fn plot_constraint(mut self, constraint: PlotConstraint) -> Self {
        self.layout_spec.plot_area = match constraint {
            PlotConstraint::Auto => crate::layout::SizeMode::Auto,
            PlotConstraint::Width(width) => crate::layout::SizeMode::Width(
                crate::serialization::serializable_expr_from_expr(width, "plot width constraint"),
            ),
            PlotConstraint::Height(height) => crate::layout::SizeMode::Height(
                crate::serialization::serializable_expr_from_expr(height, "plot height constraint"),
            ),
        };
        self
    }

    /// Set chart margins.
    pub fn margins(mut self, margins: Margins) -> Self {
        self.layout_spec.margins = margins;
        self
    }

    /// Access the chart layout specification.
    pub fn get_layout_spec(&self) -> &LayoutSpec {
        &self.layout_spec
    }

    /// Set an explicit chart theme.
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = Some(Arc::new(theme));
        self
    }

    /// Supply an inherited theme only when no explicit theme is present.
    pub fn theme_if_unset(mut self, theme: Arc<Theme>) -> Self {
        if self.theme.is_none() {
            self.theme = Some(theme);
        }
        self
    }

    /// Access the effective configured theme.
    pub fn get_theme(&self) -> Arc<Theme> {
        self.theme
            .clone()
            .unwrap_or_else(|| Arc::new(Theme::light()))
    }

    /// Set chart-wide temporal defaults.
    pub fn time_context(mut self, time_context: TimeContext) -> Self {
        self.time_context = time_context;
        self
    }

    /// Set chart-wide formatting defaults.
    pub fn formatting_context(mut self, formatting_context: FormattingContext) -> Self {
        self.formatting_context = formatting_context;
        self
    }

    /// Declare one globally shared chart parameter.
    pub fn param(mut self, param: Param) -> Self {
        self.param_specs.push(CompiledParamSpec::shared(&param));
        self
    }

    /// Declare multiple globally shared chart parameters.
    pub fn params(mut self, params: impl IntoIterator<Item = Param>) -> Self {
        self.param_specs.extend(
            params
                .into_iter()
                .map(|param| CompiledParamSpec::shared(&param)),
        );
        self
    }

    /// Declare a chart parameter with an explicit sharing scope.
    pub fn param_with_sharing(mut self, param: Param, sharing: CoordinationScope) -> Self {
        self.param_specs
            .push(CompiledParamSpec::new(&param, sharing));
        self
    }

    /// Declare a chart selection.
    pub fn selection(mut self, selection: Selection) -> Self {
        self.selections.push(selection);
        self
    }

    /// Declare one chart store.
    pub fn store(mut self, store: Store) -> Self {
        self.stores.push(store);
        self
    }

    /// Declare multiple chart stores.
    pub fn stores(mut self, stores: impl IntoIterator<Item = Store>) -> Self {
        self.stores.extend(stores);
        self
    }

    /// Compile this root chart.
    pub async fn compile(
        self,
        session_context: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        self.plot
            .compile(
                session_context,
                RootChartFurnishings {
                    theme: self.theme,
                    time_context: self.time_context,
                    formatting_context: self.formatting_context,
                    layout_spec: self.layout_spec,
                    title: self.title,
                    subtitle: self.subtitle,
                    param_specs: self.param_specs,
                    selections: self.selections,
                    stores: self.stores,
                },
            )
            .await
    }
}

impl Chart<PixelFrame> {
    /// Attach a composed widget whose frame is supplied at evaluation time.
    pub fn host_widget<W: ChartWidget>(mut self, widget: W) -> Self {
        self.plot = self.plot.host_widget(widget);
        self
    }

    /// Attach a native widget whose frame is supplied at evaluation time.
    pub fn host_native_widget<N: NativeWidget>(mut self, widget: N) -> Self {
        self.plot = self.plot.host_native_widget(widget);
        self
    }
}

impl<C: CoordinateSystem + Default> Chart<C> {
    /// Construct a chart with the coordinate system's default configuration.
    pub fn new() -> Self {
        Self::default()
    }
}

impl<C: CoordinateSystem + Default> Default for Chart<C> {
    fn default() -> Self {
        Self::from_plot(Plot::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_chart_cartesian::CartesianSubplotPositionChannels;
    use avenger_chart_core::{DefaultLogicalExprNodeExt, RepeatVariable, ZeroDCoord};
    use avenger_chart_marks::{Subplot, Symbol};
    use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
    use datafusion::common::ScalarValue;
    use datafusion::prelude::lit;
    use datafusion_proto::protobuf::LogicalExprNode;

    use crate::{
        cartesian::Cartesian,
        concat::{GridConcat, HConcat, compiled_subplot},
        layout::{Margins, SizeMode},
        repeat::{RepeatCell, RepeatGrid},
    };

    fn child_plot(compiled: &CompiledPlot, index: usize) -> &CompiledPlot {
        compiled_subplot(compiled.marks()[index].as_ref())
            .expect("compiled child subplot")
            .compiled_subplot()
    }

    fn width_expr(plot: &CompiledPlot) -> LogicalExprNode {
        match &plot.layout_spec.plot_area {
            SizeMode::Width(width) => width.clone().into(),
            other => panic!("expected width-only child size, got {other:?}"),
        }
    }

    fn find_group<'a>(marks: &'a [SceneMark], prefix: &str) -> Option<&'a SceneGroup> {
        for mark in marks {
            if let SceneMark::Group(group) = mark {
                if group.name.starts_with(prefix) {
                    return Some(group);
                }
                if let Some(found) = find_group(&group.marks, prefix) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[tokio::test]
    async fn chart_facade_compiles_identically_from_new_or_promoted_plot() {
        let session_context = SessionContext::new();
        let promoted = Chart::from_plot(Plot::<Cartesian>::new())
            .canvas_size(320.0, 200.0)
            .title("Facade parity");
        let chart = Chart::<Cartesian>::new()
            .canvas_size(320.0, 200.0)
            .title("Facade parity");

        let promoted_bytes =
            bincode::serialize(&promoted.compile(&session_context).await.unwrap()).unwrap();
        let chart_bytes =
            bincode::serialize(&chart.compile(&session_context).await.unwrap()).unwrap();
        assert_eq!(chart_bytes, promoted_bytes);
    }

    #[tokio::test]
    async fn canonical_state_methods_compile_to_root_artifact() {
        let chart = Chart::<Cartesian>::new()
            .param({
                let __avenger_param_name = "shared";
                let __avenger_param_default: datafusion::common::ScalarValue = (1_i64).into();
                Param::typed(
                    __avenger_param_name,
                    __avenger_param_default.data_type(),
                    __avenger_param_default,
                )
                .expect("a parameter default must match its selected physical type")
            })
            .param_with_sharing(
                {
                    let __avenger_param_name = "local";
                    let __avenger_param_default: datafusion::common::ScalarValue = (2_i64).into();
                    Param::typed(
                        __avenger_param_name,
                        __avenger_param_default.data_type(),
                        __avenger_param_default,
                    )
                    .expect("a parameter default must match its selected physical type")
                },
                CoordinationScope::Free,
            )
            .selection(Selection::new("picked"))
            .store(Store::empty("rows"));
        let compiled = chart.compile(&SessionContext::new()).await.unwrap();

        assert_eq!(
            compiled.param_specs()["shared"].sharing,
            CoordinationScope::Shared
        );
        assert_eq!(
            compiled.param_specs()["local"].sharing,
            CoordinationScope::Free
        );
        assert!(compiled.selection_specs().contains_key("picked"));
        assert!(compiled.store_specs().contains_key("rows"));
    }

    #[test]
    fn theme_if_unset_preserves_explicit_theme() {
        let inherited = Arc::new(Theme::dark());
        let filled = Chart::<Cartesian>::new().theme_if_unset(inherited.clone());
        assert!(Arc::ptr_eq(
            filled.theme.as_ref().expect("inherited theme"),
            &inherited
        ));

        let explicit = Chart::<Cartesian>::new().theme(Theme::light());
        let explicit_theme = explicit.theme.clone().expect("explicit theme");
        let explicit = explicit.theme_if_unset(inherited);
        assert!(Arc::ptr_eq(
            explicit.theme.as_ref().expect("preserved theme"),
            &explicit_theme
        ));
        assert!(Chart::<Cartesian>::new().theme.is_none());
    }

    #[tokio::test]
    async fn root_context_reaches_concat_repeat_grandchild_without_baking_fallback_theme() {
        fn repeated_child() -> Plot<RepeatGrid> {
            Plot::<RepeatGrid>::new().configure_coord(|repeat| {
                repeat
                    .rows([RepeatVariable::new("row", lit(1_i64))])
                    .columns([RepeatVariable::new("column", lit(2_i64))])
                    .cell(Plot::<ZeroDCoord>::new().mark(Symbol::new().fill("#0072b2").size(20.0)))
            })
        }

        let ctx = SessionContext::new();
        let time = TimeContext::new()
            .timezone("America/New_York")
            .week_start(avenger_chart_core::WeekStart::Monday);
        let formatting = FormattingContext::new()
            .number_locale("test-number")
            .datetime_locale("test-datetime")
            .datetime_timezone("America/New_York");
        let themed = Chart::<HConcat>::new()
            .theme(Theme::dark())
            .time_context(time.clone())
            .formatting_context(formatting.clone())
            .mark(Subplot::new(repeated_child()))
            .compile(&ctx)
            .await
            .unwrap();
        let repeat = child_plot(&themed, 0);
        let grandchild = child_plot(repeat, 0);
        let root_theme = themed.theme.as_ref().expect("root theme");
        assert!(Arc::ptr_eq(
            root_theme,
            repeat.theme.as_ref().expect("repeat theme")
        ));
        assert!(Arc::ptr_eq(
            root_theme,
            grandchild.theme.as_ref().expect("grandchild theme")
        ));
        assert_eq!(repeat.time_context, time);
        assert_eq!(grandchild.time_context, time);
        assert_eq!(repeat.formatting_context, formatting);
        assert_eq!(grandchild.formatting_context, formatting);

        let unthemed = Chart::<HConcat>::new()
            .mark(Subplot::new(repeated_child()))
            .compile(&ctx)
            .await
            .unwrap();
        let unthemed_repeat = child_plot(&unthemed, 0);
        let unthemed_grandchild = child_plot(unthemed_repeat, 0);
        assert!(unthemed.theme.is_none());
        assert!(unthemed_repeat.theme.is_none());
        assert!(unthemed_grandchild.theme.is_none());
    }

    #[test]
    fn root_layout_is_independent_of_plot_escape_hatch() {
        let chart = Chart::<Cartesian>::new()
            .margins(Margins::uniform(12.0))
            .canvas_size(400.0, 240.0)
            .configure_plot(|plot| plot);
        assert_eq!(chart.get_layout_spec().margins, Margins::uniform(12.0));
    }

    #[test]
    fn root_layout_builders_retain_fixed_expression_and_constraint_modes() {
        let width = {
            let __avenger_param_name = "root_width";
            let __avenger_param_default: datafusion::common::ScalarValue = (320.0_f64).into();
            Param::typed(
                __avenger_param_name,
                __avenger_param_default.data_type(),
                __avenger_param_default,
            )
            .expect("a parameter default must match its selected physical type")
        };
        let fixed = Chart::<Cartesian>::new().plot_size(width.expr(), 200.0);
        let SizeMode::Fixed {
            width: fixed_width,
            height: _,
        } = &fixed.get_layout_spec().plot_area
        else {
            panic!("expected fixed root plot size");
        };
        let fixed_width: LogicalExprNode = fixed_width.clone().into();
        assert_eq!(
            fixed_width.to_default_expr(&SessionContext::new()).unwrap(),
            width.expr()
        );

        let constrained = Chart::<Cartesian>::new()
            .canvas_constraint(CanvasConstraint::width(640.0))
            .plot_constraint(PlotConstraint::height(180.0));
        assert!(matches!(
            constrained.get_layout_spec().canvas,
            SizeMode::Width(_)
        ));
        assert!(matches!(
            constrained.get_layout_spec().plot_area,
            SizeMode::Height(_)
        ));
    }

    #[tokio::test]
    async fn configured_root_title_and_subtitle_compile_from_chart_storage() {
        let ctx = SessionContext::new();
        let compiled = Chart::<Cartesian>::new()
            .configure_title("Root title", |title| title.font_size(24.0).typst())
            .configure_subtitle("Root subtitle", |subtitle| subtitle.font_size(14.0))
            .compile(&ctx)
            .await
            .unwrap();

        let title = compiled.get_title().expect("root title");
        assert_eq!(title.text.to_default_expr(&ctx).unwrap(), lit("Root title"));
        assert_eq!(
            title
                .font_size
                .as_option()
                .and_then(|font_size| font_size.as_ref())
                .expect("title font size")
                .to_default_expr(&ctx)
                .unwrap(),
            lit(24.0)
        );
        assert_eq!(
            title.syntax_mode,
            avenger_text::types::TextSyntaxMode::TypstMarkup
        );
        let subtitle = compiled.get_subtitle().expect("root subtitle");
        assert_eq!(
            subtitle.text.to_default_expr(&ctx).unwrap(),
            lit("Root subtitle")
        );
    }

    #[tokio::test]
    async fn subplot_caption_and_label_keep_independent_semantics() {
        let ctx = SessionContext::new();
        let compiled = Chart::<HConcat>::new()
            .mark(
                Subplot::new(
                    Plot::<ZeroDCoord>::new().mark(Symbol::new().fill("#0072b2").size(20.0)),
                )
                .label("Band metadata")
                .configure_caption("Child caption", |caption| caption.typst())
                .configure_size(|size| size.width(240.0)),
            )
            .compile(&ctx)
            .await
            .unwrap();
        let subplot = compiled_subplot(compiled.marks()[0].as_ref()).unwrap();
        assert_eq!(subplot.label(), Some("Band metadata"));
        let child = subplot.compiled_subplot();
        let caption = child.get_title().expect("child caption");
        assert_eq!(
            caption.syntax_mode,
            avenger_text::types::TextSyntaxMode::TypstMarkup
        );
        assert_eq!(
            caption.text.to_default_expr(&ctx).unwrap(),
            lit("Child caption")
        );
        assert_eq!(width_expr(child).to_default_expr(&ctx).unwrap(), lit(240.0));
    }

    #[tokio::test]
    async fn repeat_cell_resolves_caption_and_one_axis_size_like_subplot() {
        let ctx = SessionContext::new();
        let ordinary = Chart::<HConcat>::new()
            .mark(
                Subplot::new(Plot::<ZeroDCoord>::new())
                    .caption("Row title")
                    .configure_size(|size| size.width(240_i64)),
            )
            .compile(&ctx)
            .await
            .unwrap();
        let repeated = Chart::<RepeatGrid>::new()
            .configure_coord(|repeat| {
                repeat
                    .rows([RepeatVariable::new("row", lit(1_i64)).title("Row title")])
                    .columns([RepeatVariable::new("column", lit(240_i64))])
                    .cell(
                        RepeatCell::from(Plot::<ZeroDCoord>::new())
                            .caption(crate::repeat::row_title())
                            .configure_size(|size| {
                                size.width(crate::repeat::column().into_data_expr())
                            }),
                    )
            })
            .compile(&ctx)
            .await
            .unwrap();
        assert!(repeated.coord_transform.as_any().is::<GridConcat>());
        let ordinary_child = child_plot(&ordinary, 0);
        let repeated_child = child_plot(&repeated, 0);
        assert_eq!(
            bincode::serialize(&ordinary_child.get_title()).unwrap(),
            bincode::serialize(&repeated_child.get_title()).unwrap()
        );
        assert_eq!(
            width_expr(ordinary_child).to_default_expr(&ctx).unwrap(),
            width_expr(repeated_child).to_default_expr(&ctx).unwrap()
        );
    }

    #[tokio::test]
    async fn positioned_subplot_size_expression_evaluates_from_params() {
        let ctx = SessionContext::new();
        let width = {
            let __avenger_param_name = "child_width";
            let __avenger_param_default: datafusion::common::ScalarValue =
                ScalarValue::Float64(Some(80.0));
            Param::typed(
                __avenger_param_name,
                __avenger_param_default.data_type(),
                __avenger_param_default,
            )
            .expect("a parameter default must match its selected physical type")
        };
        let compiled = Chart::<Cartesian>::new()
            .plot_size(320.0, 200.0)
            .param(width.clone())
            .mark(
                Subplot::new(
                    Plot::<ZeroDCoord>::new().mark(Symbol::new().fill("#0072b2").size(20.0)),
                )
                .id("mini")
                .subplot_x(avenger_chart_core::ChannelValue::from(lit(0.0)).no_scale())
                .subplot_y(avenger_chart_core::ChannelValue::from(lit(0.0)).no_scale())
                .plot_width(width.expr())
                .plot_height(40.0),
            )
            .compile(&ctx)
            .await
            .unwrap();
        let params = indexmap::indexmap! {
            "child_width".to_string() => ScalarValue::Float64(Some(123.0)),
        };
        let evaluated = compiled.evaluate(&ctx, Some(params)).await.unwrap();
        let positioned = find_group(&evaluated.scene_graph.marks, "cartesian_subplot_")
            .expect("positioned child scene group");
        let SceneMark::Group(data_marks) = &positioned.marks[0] else {
            panic!("positioned child should start with its data-marks group");
        };
        let frame = data_marks
            .pattern_reference_frame
            .as_ref()
            .expect("child plot-area reference frame");
        assert_eq!(frame.width, 123.0);
        assert_eq!(frame.height, 40.0);
    }
}
