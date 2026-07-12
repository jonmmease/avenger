//! Root chart wrapper.
//!
//! [`Chart`] owns document-altitude authoring vocabulary while [`Plot`]
//! remains the position-neutral unit embedded by subplot containers. During
//! the facade phase the document fields still live on `Plot`; the methods here
//! deliberately delegate one-for-one so callers can migrate before storage
//! moves.

use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChartTool, CoordinationScope, FormattingContext, IntoExpr, IntoPlotMark,
    Param, Scale, Selection, Store, Theme, TimeContext,
};
use avenger_chart_scales::ScaleSpec as ScaleTypeSpec;
use datafusion::{dataframe::DataFrame, prelude::SessionContext};

use crate::{
    coords::CoordinateSystem,
    event::ChartEventBinding,
    layout::{CanvasConstraint, LayoutSpec, Margins, PlotConstraint},
    legend::Legend,
    scales::Auto,
};

use super::{CompiledPlot, Plot, PlotSubtitle, PlotTitle};

/// A root plot together with document-level furnishings.
#[derive(Clone)]
pub struct Chart<C: CoordinateSystem> {
    plot: Plot<C>,
}

impl<C: CoordinateSystem> Chart<C> {
    /// Construct a chart around an explicit coordinate system.
    pub fn with_coord(coord_system: C) -> Self {
        Self {
            plot: Plot::with_coord(coord_system),
        }
    }

    /// Promote an already-authored position-neutral plot to a root chart.
    pub fn from_plot(plot: Plot<C>) -> Self {
        Self { plot }
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
        self.plot = self.plot.title(text);
        self
    }

    /// Configure a chart title.
    pub fn configure_title<F>(mut self, text: impl IntoExpr, f: F) -> Self
    where
        F: FnOnce(PlotTitle) -> PlotTitle,
    {
        self.plot = self.plot.configure_title(text, f);
        self
    }

    /// Set a chart subtitle.
    pub fn subtitle(mut self, text: impl IntoExpr) -> Self {
        self.plot = self.plot.subtitle(text);
        self
    }

    /// Configure a chart subtitle.
    pub fn configure_subtitle<F>(mut self, text: impl IntoExpr, f: F) -> Self
    where
        F: FnOnce(PlotSubtitle) -> PlotSubtitle,
    {
        self.plot = self.plot.configure_subtitle(text, f);
        self
    }

    /// Access the configured title.
    pub fn get_title(&self) -> Option<&PlotTitle> {
        self.plot.get_title()
    }

    /// Access the configured subtitle.
    pub fn get_subtitle(&self) -> Option<&PlotSubtitle> {
        self.plot.get_subtitle()
    }

    /// Set fixed canvas dimensions.
    pub fn canvas_size<W: IntoExpr, H: IntoExpr>(mut self, width: W, height: H) -> Self {
        self.plot = self.plot.canvas_size(width, height);
        self
    }

    /// Set responsive canvas sizing.
    pub fn canvas_constraint(mut self, constraint: CanvasConstraint) -> Self {
        self.plot = self.plot.canvas_constraint(constraint);
        self
    }

    /// Set fixed root plot-area dimensions.
    pub fn plot_size<W: IntoExpr, H: IntoExpr>(mut self, width: W, height: H) -> Self {
        self.plot = self.plot.plot_size(width, height);
        self
    }

    /// Set responsive root plot-area sizing.
    pub fn plot_constraint(mut self, constraint: PlotConstraint) -> Self {
        self.plot = self.plot.plot_constraint(constraint);
        self
    }

    /// Set chart margins.
    pub fn margins(mut self, margins: Margins) -> Self {
        self.plot = self.plot.margins(margins);
        self
    }

    /// Access the chart layout specification.
    pub fn get_layout_spec(&self) -> &LayoutSpec {
        self.plot.get_layout_spec()
    }

    /// Set an explicit chart theme.
    pub fn theme(mut self, theme: Theme) -> Self {
        self.plot = self.plot.theme(theme);
        self
    }

    /// Supply an inherited theme only when no explicit theme is present.
    pub fn theme_if_unset(mut self, theme: Arc<Theme>) -> Self {
        if self.plot.theme.is_none() {
            self.plot.theme = Some(theme);
        }
        self
    }

    /// Access the effective configured theme.
    pub fn get_theme(&self) -> Arc<Theme> {
        self.plot.get_theme()
    }

    /// Set chart-wide temporal defaults.
    pub fn time_context(mut self, time_context: TimeContext) -> Self {
        self.plot = self.plot.time_context(time_context);
        self
    }

    /// Set chart-wide formatting defaults.
    pub fn formatting_context(mut self, formatting_context: FormattingContext) -> Self {
        self.plot = self.plot.formatting_context(formatting_context);
        self
    }

    /// Declare one globally shared chart parameter.
    pub fn param(mut self, param: Param) -> Self {
        self.plot = self.plot.add_param(param);
        self
    }

    /// Declare multiple globally shared chart parameters.
    pub fn params(mut self, params: impl IntoIterator<Item = Param>) -> Self {
        self.plot = self.plot.add_params(params);
        self
    }

    /// Declare a chart parameter with an explicit sharing scope.
    pub fn param_with_sharing(mut self, param: Param, sharing: CoordinationScope) -> Self {
        self.plot = self.plot.add_param_with_sharing(param, sharing);
        self
    }

    /// Declare a chart selection.
    pub fn selection(mut self, selection: Selection) -> Self {
        self.plot = self.plot.add_selection(selection);
        self
    }

    /// Declare one chart store.
    pub fn store(mut self, store: Store) -> Self {
        self.plot = self.plot.add_store(store);
        self
    }

    /// Declare multiple chart stores.
    pub fn stores(mut self, stores: impl IntoIterator<Item = Store>) -> Self {
        self.plot = self.plot.add_stores(stores);
        self
    }

    /// Mark a declared parameter as app cursor state.
    pub fn cursor_param(mut self, param: impl Into<String>) -> Self {
        self.plot = self.plot.cursor_param(param);
        self
    }

    /// Compile this root chart.
    pub async fn compile(
        self,
        session_context: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        self.plot.compile(session_context).await
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
    async fn chart_facade_compiles_identically_to_plot() {
        let session_context = SessionContext::new();
        let plot = Plot::<Cartesian>::new()
            .canvas_size(320.0, 200.0)
            .title("Facade parity");
        let chart = Chart::<Cartesian>::new()
            .canvas_size(320.0, 200.0)
            .title("Facade parity");

        let plot_bytes =
            bincode::serialize(&plot.compile(&session_context).await.unwrap()).unwrap();
        let chart_bytes =
            bincode::serialize(&chart.compile(&session_context).await.unwrap()).unwrap();
        assert_eq!(chart_bytes, plot_bytes);
    }

    #[test]
    fn canonical_state_methods_match_plot_storage() {
        let plot = Plot::<Cartesian>::new()
            .add_param(Param::new("shared", 1_i64))
            .add_param_with_sharing(Param::new("local", 2_i64), CoordinationScope::Free)
            .add_selection(Selection::new("picked"))
            .add_store(Store::empty("rows"))
            .cursor_param("cursor");
        let chart = Chart::<Cartesian>::new()
            .param(Param::new("shared", 1_i64))
            .param_with_sharing(Param::new("local", 2_i64), CoordinationScope::Free)
            .selection(Selection::new("picked"))
            .store(Store::empty("rows"))
            .cursor_param("cursor");

        assert_eq!(
            bincode::serialize(&chart.plot.param_specs).unwrap(),
            bincode::serialize(&plot.param_specs).unwrap()
        );
        assert_eq!(chart.plot.selections.len(), plot.selections.len());
        assert_eq!(chart.plot.stores.len(), plot.stores.len());
        assert_eq!(chart.plot.cursor_params, plot.cursor_params);
    }

    #[test]
    fn theme_if_unset_preserves_explicit_theme() {
        let inherited = Arc::new(Theme::dark());
        let filled = Chart::<Cartesian>::new().theme_if_unset(inherited.clone());
        assert!(Arc::ptr_eq(
            filled.plot.theme.as_ref().expect("inherited theme"),
            &inherited
        ));

        let explicit = Chart::<Cartesian>::new().theme(Theme::light());
        let explicit_theme = explicit.plot.theme.clone().expect("explicit theme");
        let explicit = explicit.theme_if_unset(inherited);
        assert!(Arc::ptr_eq(
            explicit.plot.theme.as_ref().expect("preserved theme"),
            &explicit_theme
        ));
        assert!(Chart::<Cartesian>::new().plot.theme.is_none());
    }

    #[test]
    fn facade_forwards_layout_and_escape_hatch() {
        let chart = Chart::<Cartesian>::new()
            .margins(Margins::uniform(12.0))
            .configure_plot(|plot| plot.canvas_size(400.0, 240.0));
        assert_eq!(
            chart.plot().get_layout_spec().margins,
            Margins::uniform(12.0)
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
        let width = Param::new("child_width", ScalarValue::Float64(Some(80.0)));
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
