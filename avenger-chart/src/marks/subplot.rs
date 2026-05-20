use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use datafusion::{
    arrow::record_batch::RecordBatch, dataframe::DataFrame, logical_expr::Expr,
    prelude::SessionContext,
};
use serde::{Deserialize, Serialize};

use crate::{
    concat::concat_coord_ref,
    coords::{CoordinateSystem, CoordinateSystemTransform},
    error::AvengerChartError,
    legend::LegendRenderer,
    marks::{
        ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkState, DataContext,
        FacetStrategy, Mark, MarkState, RadiusExpression,
    },
    plot::{CompiledPlot, Plot},
    render::RenderContext,
    scales::{ResolvedDomain, ScaleRange, ScaleSpec},
    theme::Theme,
};

/// Marker trait for coordinate systems that place child plot frames.
///
/// Facets have a specialized subplot mark today. General composition containers
/// such as concat should implement this trait and render `Subplot` marks through
/// their coordinate-system measurement.
pub trait SubplotContainerCoordinateSystem: CoordinateSystem {}

/// Data source selected for a compiled subplot's child plot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubplotDataSource {
    /// The child plot has explicit plot-level data.
    ExplicitChild,
    /// The child plot has no plot-level data and should inherit container data.
    InheritParent,
}

/// Mark that owns one child plot inside a container coordinate system.
#[derive(Clone)]
pub struct Subplot<InnerC: CoordinateSystem> {
    state: MarkState,
    subplot: Plot<InnerC>,
    label: Option<String>,
    key: Option<String>,
}

impl<InnerC: CoordinateSystem> Subplot<InnerC> {
    pub fn new(subplot: Plot<InnerC>) -> Self {
        Self {
            state: MarkState {
                data: DataContext::default(),
                facet_strategy: FacetStrategy::Filter,
                details: None,
                zindex: None,
                axis_configs: HashMap::new(),
            },
            subplot,
            label: None,
            key: None,
        }
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn subplot(&self) -> &Plot<InnerC> {
        &self.subplot
    }

    pub fn label_value(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub fn key_value(&self) -> Option<&str> {
        self.key.as_deref()
    }
}

#[async_trait::async_trait]
impl<C, InnerC> Mark<C> for Subplot<InnerC>
where
    C: SubplotContainerCoordinateSystem,
    InnerC: CoordinateSystem + Clone,
{
    fn state(&self) -> &MarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    fn data_context(&self) -> &DataContext {
        &self.state.data
    }

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        let data_source = if self.subplot.data.is_some() {
            SubplotDataSource::ExplicitChild
        } else {
            SubplotDataSource::InheritParent
        };
        let compiled_subplot = Arc::new(self.subplot.clone().compile(session_context).await?);

        Ok(Arc::new(CompiledSubplot {
            state: compiled_state,
            compiled_subplot,
            label: self.label.clone(),
            key: self.key.clone(),
            data_source,
        }))
    }
}

/// Compiled child-plot mark for container coordinate systems.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledSubplot {
    state: CompiledMarkState,
    compiled_subplot: Arc<CompiledPlot>,
    label: Option<String>,
    key: Option<String>,
    data_source: SubplotDataSource,
}

impl CompiledSubplot {
    pub fn compiled_subplot(&self) -> &Arc<CompiledPlot> {
        &self.compiled_subplot
    }

    pub fn compiled_state(&self) -> &CompiledMarkState {
        &self.state
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    pub fn data_source(&self) -> SubplotDataSource {
        self.data_source
    }

    pub fn child_index(&self) -> usize {
        self.state.mark_index()
    }

    pub fn inherits_parent_data(&self) -> bool {
        self.data_source == SubplotDataSource::InheritParent
    }

    pub fn has_explicit_child_data(&self) -> bool {
        self.data_source == SubplotDataSource::ExplicitChild
    }

    fn group_name(&self) -> String {
        match self.key() {
            Some(key) => format!("concat_subplot_{}_{}", self.child_index(), key),
            None => format!("concat_subplot_{}", self.child_index()),
        }
    }

    fn inherited_data_override(
        &self,
        data: Option<&RecordBatch>,
        context: &RenderContext<'_>,
    ) -> Result<Option<DataFrame>, AvengerChartError> {
        if !self.inherits_parent_data() {
            return Ok(None);
        }

        data.map(|batch| {
            context
                .session_context()
                .read_batch(batch.clone())
                .map_err(AvengerChartError::DataFusionError)
        })
        .transpose()
    }
}

pub fn compiled_subplot(mark: &dyn CompiledMark) -> Option<&CompiledSubplot> {
    mark.as_any().downcast_ref::<CompiledSubplot>()
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledSubplot {
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
        "subplot"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        Vec::new()
    }

    fn wants_full_data_batch(&self) -> bool {
        true
    }

    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        context: &RenderContext,
        _coord: Box<dyn CoordinateSystemTransform>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let concat_measurement =
            concat_coord_ref(context.coord_measurement()).ok_or_else(|| {
                AvengerChartError::InternalError(
                    "Subplot marks require ConcatCoordMeasurement in coord_measurement".to_string(),
                )
            })?;
        let child = concat_measurement
            .child(self.child_index())
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing concat child measurement for subplot child index {}",
                    self.child_index()
                ))
            })?;
        let child_frame_placement = concat_measurement.child_frame_placement();
        let render_placement =
            child_frame_placement
                .child(self.child_index())
                .ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Missing concat child-frame placement for subplot child index {}",
                        self.child_index()
                    ))
                })?;

        let mut params = self.compiled_subplot.get_default_params().clone();
        params.extend(context.eval.params.clone());
        let child_eval_ctx = context.eval.with_params(params);
        let data_override = self.inherited_data_override(data, context)?;
        let components = self
            .compiled_subplot
            .build_plot_components(
                &child_eval_ctx,
                &child.measurement,
                data_override.as_ref(),
                true,
                context.facet_path,
            )
            .await?;

        let data_marks_group = SceneGroup {
            origin: [0.0, 0.0],
            marks: components.data_marks,
            clip: components.clip,
            zindex: Some(0),
            ..Default::default()
        };
        let mut all_marks = vec![SceneMark::Group(data_marks_group)];
        all_marks.extend(components.guide_marks);
        all_marks.extend(components.legend_marks);
        all_marks.extend(components.title_marks);
        all_marks.extend(components.subtitle_marks);
        all_marks.extend(components.debug_marks);

        Ok(vec![SceneMark::Group(SceneGroup {
            name: self.group_name(),
            origin: render_placement.origin,
            clip: avenger_scenegraph::marks::group::Clip::None,
            marks: all_marks,
            gradients: Vec::new(),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_offset: None,
            zindex: None,
        })])
    }

    fn preferred_legend_renderer(
        &self,
        _channel: &str,
        _scale: &ConfiguredScale,
    ) -> Option<Arc<dyn LegendRenderer>> {
        None
    }

    fn radius_expression(
        &self,
        _dimension: &str,
        _resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        None
    }

    fn preferred_scale_type(
        &self,
        _channel: &str,
        _data_type: &datafusion::arrow::datatypes::DataType,
    ) -> Option<Box<dyn ScaleSpec>> {
        None
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
        _data_type: &datafusion::arrow::datatypes::DataType,
    ) -> HashMap<String, Expr> {
        HashMap::new()
    }

    fn default_channel_range(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
        _domain: &ResolvedDomain,
        _data_type: &datafusion::arrow::datatypes::DataType,
        _theme: &Theme,
        _params: &indexmap::IndexMap<String, datafusion::scalar::ScalarValue>,
    ) -> Option<ScaleRange> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::{
        arrow::{array::Float32Array, record_batch::RecordBatch},
        prelude::SessionContext,
    };

    use super::*;
    use crate::zerod::ZeroDCoord;

    impl SubplotContainerCoordinateSystem for ZeroDCoord {}

    fn single_column_df(ctx: &SessionContext, value: f32) -> datafusion::dataframe::DataFrame {
        let batch = RecordBatch::try_from_iter(vec![(
            "x",
            Arc::new(Float32Array::from(vec![value])) as Arc<dyn datafusion::arrow::array::Array>,
        )])
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    #[tokio::test]
    async fn subplot_compilation_preserves_label_key_and_child_plot() {
        let ctx = SessionContext::new();
        let subplot = Subplot::new(Plot::<ZeroDCoord>::new())
            .label("overview")
            .key("overview-key");

        let compiled_state = CompiledMarkState::from_mark_state(&subplot.state, None);
        let compiled =
            <Subplot<ZeroDCoord> as Mark<ZeroDCoord>>::compile(&subplot, compiled_state, &ctx)
                .await
                .unwrap();
        let compiled = compiled_subplot(compiled.as_ref()).unwrap();

        assert_eq!(compiled.mark_type(), "subplot");
        assert_eq!(compiled.label(), Some("overview"));
        assert_eq!(compiled.key(), Some("overview-key"));
        assert_eq!(compiled.data_source(), SubplotDataSource::InheritParent);
        assert!(compiled.inherits_parent_data());
        assert_eq!(compiled.compiled_subplot().marks().len(), 0);
    }

    #[tokio::test]
    async fn subplot_compilation_keeps_explicit_child_plot_data() {
        let ctx = SessionContext::new();
        let child_data = single_column_df(&ctx, 1.0);
        let subplot = Subplot::new(Plot::<ZeroDCoord>::new().data(child_data));

        let compiled_state = CompiledMarkState::from_mark_state(&subplot.state, None);
        let compiled =
            <Subplot<ZeroDCoord> as Mark<ZeroDCoord>>::compile(&subplot, compiled_state, &ctx)
                .await
                .unwrap();
        let compiled = compiled_subplot(compiled.as_ref()).unwrap();

        assert_eq!(compiled.data_source(), SubplotDataSource::ExplicitChild);
        assert!(compiled.has_explicit_child_data());
        assert!(compiled.compiled_subplot().data.is_some());
    }

    #[tokio::test]
    async fn plot_compile_passes_parent_data_to_subplot_mark_state() {
        let ctx = SessionContext::new();
        let parent_data = single_column_df(&ctx, 2.0);
        let subplot = Subplot::new(Plot::<ZeroDCoord>::new());

        let compiled_plot = Plot::<ZeroDCoord>::new()
            .data(parent_data)
            .mark(subplot)
            .compile(&ctx)
            .await
            .unwrap();
        let compiled = compiled_subplot(compiled_plot.marks()[0].as_ref()).unwrap();

        assert_eq!(compiled.data_source(), SubplotDataSource::InheritParent);
        assert!(
            compiled
                .compiled_state()
                .data
                .dataframe_with_context(&ctx)
                .is_some()
        );
        assert!(compiled.compiled_subplot().data.is_none());
    }

    #[tokio::test]
    async fn repeated_subplot_marks_receive_stable_child_indexes() {
        let ctx = SessionContext::new();
        let compiled_plot = Plot::<ZeroDCoord>::new()
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("first"))
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("second"))
            .compile(&ctx)
            .await
            .unwrap();

        let first = compiled_subplot(compiled_plot.marks()[0].as_ref()).unwrap();
        let second = compiled_subplot(compiled_plot.marks()[1].as_ref()).unwrap();

        assert_eq!(first.child_index(), 0);
        assert_eq!(second.child_index(), 1);
        assert_eq!(first.key(), Some("first"));
        assert_eq!(second.key(), Some("second"));
    }
}
