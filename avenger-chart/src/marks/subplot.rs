use std::{collections::HashMap, marker::PhantomData, sync::Arc};

use datafusion::{arrow::record_batch::RecordBatch, dataframe::DataFrame, prelude::SessionContext};
use serde::{Deserialize, Serialize};

use avenger_chart_core::{
    AvengerChartError, ChannelValue, ColumnDimensionConfig, CompiledMark, CompiledMarkState,
    CompiledSubplotChildPlot, CoordinateSystem, CoordinateSystemCore, DataContext,
    FacetDimensionConfig, FacetEmptyCellPolicy, FacetStrategy, Mark, MarkState, RowDimensionConfig,
    ScaleSharing, SubplotChildPlotSpec, SubplotDataSource,
};

/// Shared compiled state for a child plot owned by a container subplot mark.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledSubplotPayload {
    state: CompiledMarkState,
    compiled_subplot: Arc<dyn CompiledSubplotChildPlot>,
    label: Option<String>,
    key: Option<String>,
    data_source: SubplotDataSource,
}

impl CompiledSubplotPayload {
    pub fn new(
        state: CompiledMarkState,
        compiled_subplot: Arc<dyn CompiledSubplotChildPlot>,
        label: Option<String>,
        key: Option<String>,
        data_source: SubplotDataSource,
    ) -> Self {
        Self {
            state,
            compiled_subplot,
            label,
            key,
            data_source,
        }
    }

    pub fn compiled_child_plot(&self) -> &Arc<dyn CompiledSubplotChildPlot> {
        &self.compiled_subplot
    }

    pub fn compiled_state(&self) -> &CompiledMarkState {
        &self.state
    }

    pub fn compiled_state_mut(&mut self) -> &mut CompiledMarkState {
        &mut self.state
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    pub fn mark_index(&self) -> usize {
        self.state.mark_index()
    }

    pub fn data_source(&self) -> SubplotDataSource {
        self.data_source
    }

    pub fn inherits_parent_data(&self) -> bool {
        self.data_source == SubplotDataSource::InheritParent
    }

    pub fn has_explicit_child_data(&self) -> bool {
        self.data_source == SubplotDataSource::ExplicitChild
    }

    #[doc(hidden)]
    pub fn inherited_data_override(
        &self,
        data: Option<&RecordBatch>,
        session_context: &SessionContext,
    ) -> Result<Option<DataFrame>, AvengerChartError> {
        if !self.inherits_parent_data() {
            return Ok(None);
        }

        data.map(|batch| {
            session_context
                .read_batch(batch.clone())
                .map_err(AvengerChartError::DataFusionError)
        })
        .transpose()
    }
}

pub async fn compile_subplot_payload<OuterC: CoordinateSystemCore>(
    subplot: &Subplot<OuterC>,
    compiled_state: CompiledMarkState,
    session_context: &SessionContext,
) -> Result<CompiledSubplotPayload, AvengerChartError> {
    let data_source = if subplot.has_plot_level_data() {
        SubplotDataSource::ExplicitChild
    } else {
        SubplotDataSource::InheritParent
    };
    let compiled_subplot = subplot.compile_child_plot(session_context).await?;

    Ok(CompiledSubplotPayload::new(
        compiled_state,
        compiled_subplot,
        subplot.config.label.clone(),
        subplot.config.key.clone(),
        data_source,
    ))
}

#[derive(Clone, Default)]
pub(crate) struct SubplotConfig {
    pub(crate) label: Option<String>,
    pub(crate) key: Option<String>,
    pub(crate) plot_width: Option<f32>,
    pub(crate) plot_height: Option<f32>,
    pub(crate) facet_row_title: Option<String>,
    pub(crate) facet_col_title: Option<String>,
    pub(crate) facet_row_slot_sharing: Option<ScaleSharing>,
    pub(crate) facet_col_slot_sharing: Option<ScaleSharing>,
    pub(crate) facet_row_position: Option<String>,
    pub(crate) facet_col_position: Option<String>,
    pub(crate) facet_row_empty_cell_policy: Option<FacetEmptyCellPolicy>,
    pub(crate) facet_col_empty_cell_policy: Option<FacetEmptyCellPolicy>,
}

/// Mark that owns one child plot inside an outer coordinate system.
///
/// `Subplot<OuterC>` is parameterized by the coordinate system that positions
/// the child plot, not by the child plot's own coordinate system. This keeps
/// concat, facet, and coordinate-positioned subplots under one public mark
/// concept while still allowing mixed child coordinate systems.
#[derive(Clone)]
pub struct Subplot<OuterC: CoordinateSystemCore> {
    state: MarkState,
    subplot: Box<dyn SubplotChildPlotSpec>,
    config: SubplotConfig,
    _outer: PhantomData<fn() -> OuterC>,
}

impl<OuterC: CoordinateSystemCore> Subplot<OuterC> {
    pub fn new<P>(subplot: P) -> Self
    where
        P: SubplotChildPlotSpec + 'static,
    {
        Self {
            state: MarkState {
                data: DataContext::default(),
                facet_strategy: FacetStrategy::Filter,
                details: None,
                zindex: None,
                axis_configs: HashMap::new(),
            },
            subplot: Box::new(subplot),
            config: SubplotConfig::default(),
            _outer: PhantomData,
        }
    }

    /// Set explicit data for this subplot mark.
    pub fn data(mut self, dataframe: datafusion::dataframe::DataFrame) -> Self {
        self.state.data = DataContext::new(dataframe);
        self
    }

    /// Set zindex.
    pub fn zindex(mut self, zindex: i32) -> Self {
        self.state.zindex = Some(zindex);
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.config.label = Some(label.into());
        self
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.config.key = Some(key.into());
        self
    }

    #[doc(hidden)]
    pub fn data_context_ref(&self) -> &DataContext {
        &self.state.data
    }

    #[doc(hidden)]
    pub fn label_config(&self) -> Option<&str> {
        self.config.label.as_deref()
    }

    #[doc(hidden)]
    pub fn key_config(&self) -> Option<&str> {
        self.config.key.as_deref()
    }

    #[doc(hidden)]
    pub fn plot_width_config(&self) -> Option<f32> {
        self.config.plot_width
    }

    #[doc(hidden)]
    pub fn plot_height_config(&self) -> Option<f32> {
        self.config.plot_height
    }

    #[doc(hidden)]
    pub fn set_plot_width_config(&mut self, width: Option<f32>) {
        self.config.plot_width = width;
    }

    #[doc(hidden)]
    pub fn set_plot_height_config(&mut self, height: Option<f32>) {
        self.config.plot_height = height;
    }

    #[doc(hidden)]
    pub fn set_facet_row_options(
        &mut self,
        title: Option<String>,
        slot_sharing: Option<ScaleSharing>,
        position: Option<String>,
        empty_cell_policy: Option<FacetEmptyCellPolicy>,
    ) {
        self.config.facet_row_title = title;
        self.config.facet_row_slot_sharing = slot_sharing;
        self.config.facet_row_position = position;
        self.config.facet_row_empty_cell_policy = empty_cell_policy;
    }

    #[doc(hidden)]
    pub fn facet_row_title_config(&self) -> Option<&str> {
        self.config.facet_row_title.as_deref()
    }

    #[doc(hidden)]
    pub fn facet_row_slot_sharing_config(&self) -> Option<ScaleSharing> {
        self.config.facet_row_slot_sharing
    }

    #[doc(hidden)]
    pub fn facet_row_position_config(&self) -> Option<&str> {
        self.config.facet_row_position.as_deref()
    }

    #[doc(hidden)]
    pub fn facet_row_empty_cell_policy_config(&self) -> Option<FacetEmptyCellPolicy> {
        self.config.facet_row_empty_cell_policy
    }

    #[doc(hidden)]
    pub fn set_facet_col_options(
        &mut self,
        title: Option<String>,
        slot_sharing: Option<ScaleSharing>,
        position: Option<String>,
        empty_cell_policy: Option<FacetEmptyCellPolicy>,
    ) {
        self.config.facet_col_title = title;
        self.config.facet_col_slot_sharing = slot_sharing;
        self.config.facet_col_position = position;
        self.config.facet_col_empty_cell_policy = empty_cell_policy;
    }

    #[doc(hidden)]
    pub fn facet_col_title_config(&self) -> Option<&str> {
        self.config.facet_col_title.as_deref()
    }

    #[doc(hidden)]
    pub fn facet_col_slot_sharing_config(&self) -> Option<ScaleSharing> {
        self.config.facet_col_slot_sharing
    }

    #[doc(hidden)]
    pub fn facet_col_position_config(&self) -> Option<&str> {
        self.config.facet_col_position.as_deref()
    }

    #[doc(hidden)]
    pub fn facet_col_empty_cell_policy_config(&self) -> Option<FacetEmptyCellPolicy> {
        self.config.facet_col_empty_cell_policy
    }

    pub fn has_plot_level_data(&self) -> bool {
        self.subplot.has_plot_level_data()
    }

    pub async fn compile_child_plot(
        &self,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError> {
        self.subplot.compile_boxed(session_context).await
    }

    #[doc(hidden)]
    pub fn with_channel_value(mut self, channel_name: &'static str, value: ChannelValue) -> Self {
        self.state.data = self.state.data.with_channel_value(channel_name, value);
        self
    }

    #[doc(hidden)]
    pub fn validate_no_facet_channels(&self, outer_label: &str) -> Result<(), AvengerChartError> {
        let channels = self.state.data.channels();
        for channel_name in [
            RowDimensionConfig::channel_name(),
            ColumnDimensionConfig::channel_name(),
        ] {
            if channels.contains_key(channel_name) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "{outer_label} subplots do not support facet channel `{channel_name}`"
                )));
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
pub trait SubplotContainerCoordinateSystem: CoordinateSystem + Sized {
    /// Compile a `Subplot<Self>` mark for this coordinate system.
    ///
    /// Outer-coordinate-specific builder methods still live on `Subplot<Self>`
    /// extension impls. This hook only owns the final conversion from a generic
    /// subplot mark plus compiled mark state into the coordinate-system-specific
    /// compiled mark. The core layout engine keeps facet and concat behavior
    /// built in; external coordinate crates can implement this hook when their
    /// coordinate system supports positioned child plots.
    async fn compile_subplot_mark(
        subplot: &Subplot<Self>,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError>;
}

#[async_trait::async_trait]
impl<C> Mark<C> for Subplot<C>
where
    C: SubplotContainerCoordinateSystem,
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
        C::compile_subplot_mark(self, compiled_state, session_context).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::{
        arrow::{array::Float32Array, record_batch::RecordBatch},
        prelude::{SessionContext, col},
    };

    use super::*;
    use crate::{
        chart_core::CompiledMarkCore,
        chart_core::{FacetDimensionConfig, RowDimensionConfig},
        concat::{HConcat, compiled_subplot},
        plot::Plot,
        zerod::ZeroDCoord,
    };

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
        let subplot = Subplot::<HConcat>::new(Plot::<ZeroDCoord>::new())
            .label("overview")
            .key("overview-key");

        let compiled_state = CompiledMarkState::from_mark_state(&subplot.state, None);
        let compiled = <Subplot<HConcat> as Mark<HConcat>>::compile(&subplot, compiled_state, &ctx)
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
        let subplot = Subplot::<HConcat>::new(Plot::<ZeroDCoord>::new().data(child_data));

        let compiled_state = CompiledMarkState::from_mark_state(&subplot.state, None);
        let compiled = <Subplot<HConcat> as Mark<HConcat>>::compile(&subplot, compiled_state, &ctx)
            .await
            .unwrap();
        let compiled = compiled_subplot(compiled.as_ref()).unwrap();

        assert_eq!(compiled.data_source(), SubplotDataSource::ExplicitChild);
        assert!(compiled.has_explicit_child_data());
        assert!(compiled.compiled_subplot().data.is_some());
    }

    #[tokio::test]
    async fn concat_subplot_rejects_facet_channels() {
        let ctx = SessionContext::new();
        let mut subplot = Subplot::<HConcat>::new(Plot::<ZeroDCoord>::new());
        subplot.state.data = subplot
            .state
            .data
            .with_channel_value(RowDimensionConfig::channel_name(), col("group").into());

        let compiled_state = CompiledMarkState::from_mark_state(&subplot.state, None);
        let result =
            <Subplot<HConcat> as Mark<HConcat>>::compile(&subplot, compiled_state, &ctx).await;

        assert!(matches!(result, Err(AvengerChartError::InvalidArgument(_))));
    }

    #[tokio::test]
    async fn plot_compile_passes_parent_data_to_subplot_mark_state() {
        let ctx = SessionContext::new();
        let parent_data = single_column_df(&ctx, 2.0);
        let subplot = Subplot::<HConcat>::new(Plot::<ZeroDCoord>::new());

        let compiled_plot = Plot::<HConcat>::new()
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
        let compiled_plot = Plot::<HConcat>::new()
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
