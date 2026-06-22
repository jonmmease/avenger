use std::{marker::PhantomData, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompileContext, CompiledDataContext, CompiledMark,
    CompiledMarkCore, CompiledMarkState, CompiledSubplotPayload, CoordinateSlotOverlayMarkCore,
    DataContext, FacetDataScope, IntoPlotMark, Mark, MarkDataMode, MarkRuntimeContext, MarkState,
    PlotMark, RenderedMarkData, SubplotChildPlotSpec, SubplotDataSource, validate_structural_id,
};
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::{arrow::record_batch::RecordBatch, prelude::SessionContext};
use serde::{Deserialize, Serialize};

use crate::Parallel;

/// Child plot rendered over one dimension axis in a parallel-coordinate chart.
///
/// The child plot is measured as a fixed-width frame whose y scale is supplied
/// by the selected parallel dimension. This lets authors draw ordinary
/// Cartesian content such as brush rectangles, symbols, box plots, or violins
/// directly on top of a parallel axis.
#[derive(Clone)]
pub struct ParallelAxisOverlay<C = Parallel> {
    state: MarkState,
    dimension_id: String,
    subplot: Box<dyn SubplotChildPlotSpec>,
    width_px: f32,
    x_offset_px: f32,
    clip: bool,
    show_child_chrome: bool,
    _phantom: PhantomData<C>,
}

impl ParallelAxisOverlay<Parallel> {
    pub fn new<P>(dimension_id: impl Into<String>, subplot: P) -> Self
    where
        P: SubplotChildPlotSpec + 'static,
    {
        Self {
            state: MarkState {
                id: None,
                data: DataContext::default(),
                data_mode: MarkDataMode::Unit,
                facet_data_scope: FacetDataScope::FILTERED,
                exclude_from_scale_domains: true,
                visible: None,
                details: None,
                zindex: None,
                geometry_space: None,
                axis_configs: Default::default(),
            },
            dimension_id: dimension_id.into(),
            subplot: Box::new(subplot),
            width_px: 64.0,
            x_offset_px: 0.0,
            clip: true,
            show_child_chrome: false,
            _phantom: PhantomData,
        }
    }

    /// Set a structural id used by chart interaction targeting.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.state.id = Some(id.into());
        self
    }

    /// Set the child plot-area width in pixels.
    pub fn width_px(mut self, width_px: f32) -> Self {
        self.width_px = width_px.max(1.0);
        self
    }

    /// Shift the overlay frame relative to the axis center.
    pub fn x_offset_px(mut self, x_offset_px: f32) -> Self {
        self.x_offset_px = x_offset_px;
        self
    }

    /// Clip the entire child frame to the overlay plot-area rectangle.
    pub fn clip(mut self, clip: bool) -> Self {
        self.clip = clip;
        self
    }

    /// Show guides, legends, titles, and subtitles emitted by the child plot.
    ///
    /// Child chrome is hidden by default because axis overlays are usually
    /// narrow annotation surfaces whose y scale is already represented by the
    /// parent parallel axis.
    pub fn show_child_chrome(mut self, show_child_chrome: bool) -> Self {
        self.show_child_chrome = show_child_chrome;
        self
    }

    /// Set rendering order.
    pub fn zindex(mut self, zindex: i32) -> Self {
        self.state.zindex = Some(zindex);
        self
    }
}

impl IntoPlotMark<Parallel> for ParallelAxisOverlay<Parallel> {
    fn into_plot_marks(self) -> Vec<PlotMark<Parallel>> {
        vec![PlotMark::from_mark(self)]
    }
}

#[async_trait::async_trait]
impl Mark<Parallel> for ParallelAxisOverlay<Parallel> {
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
        compile_parallel_axis_overlay(self, compiled_state, session_context, None).await
    }

    async fn compile_with_context(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
        compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        compile_parallel_axis_overlay(self, compiled_state, session_context, compile_context).await
    }
}

async fn compile_parallel_axis_overlay(
    overlay: &ParallelAxisOverlay<Parallel>,
    compiled_state: CompiledMarkState,
    session_context: &SessionContext,
    compile_context: Option<CompileContext<'_>>,
) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
    validate_structural_id("parallel axis overlay dimension", &overlay.dimension_id)?;
    let data_source = if overlay.subplot.has_plot_level_data() {
        SubplotDataSource::ExplicitChild
    } else {
        SubplotDataSource::InheritParent
    };
    let compiled_subplot = overlay
        .subplot
        .compile_boxed_with_context(session_context, compile_context)
        .await?;
    let payload = CompiledSubplotPayload::new(
        compiled_state,
        compiled_subplot,
        None,
        Some(overlay.dimension_id.clone()),
        data_source,
    );
    Ok(Arc::new(CompiledParallelAxisOverlay {
        payload,
        dimension_id: overlay.dimension_id.clone(),
        width_px: overlay.width_px.max(1.0),
        x_offset_px: overlay.x_offset_px,
        clip: overlay.clip,
        show_child_chrome: overlay.show_child_chrome,
    }))
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledParallelAxisOverlay {
    payload: CompiledSubplotPayload,
    dimension_id: String,
    width_px: f32,
    x_offset_px: f32,
    clip: bool,
    #[serde(default)]
    show_child_chrome: bool,
}

impl CompiledParallelAxisOverlay {
    pub fn payload(&self) -> &CompiledSubplotPayload {
        &self.payload
    }

    pub fn dimension_id(&self) -> &str {
        &self.dimension_id
    }

    pub fn width_px(&self) -> f32 {
        self.width_px
    }

    pub fn x_offset_px(&self) -> f32 {
        self.x_offset_px
    }

    pub fn clip(&self) -> bool {
        self.clip
    }

    pub fn show_child_chrome(&self) -> bool {
        self.show_child_chrome
    }
}

impl CompiledMarkCore for CompiledParallelAxisOverlay {
    fn state(&self) -> &CompiledMarkState {
        self.payload.compiled_state()
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        self.payload.compiled_state_mut()
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.payload.compiled_state().data
    }

    fn mark_type(&self) -> &str {
        "parallel_axis_overlay"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_coordinate_slot_overlay(&self) -> Option<&dyn CoordinateSlotOverlayMarkCore> {
        Some(self)
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        Vec::new()
    }
}

impl CoordinateSlotOverlayMarkCore for CompiledParallelAxisOverlay {
    fn payload(&self) -> &CompiledSubplotPayload {
        &self.payload
    }

    fn slot_id(&self) -> &str {
        &self.dimension_id
    }

    fn width_px(&self) -> f32 {
        self.width_px
    }

    fn x_offset_px(&self) -> f32 {
        self.x_offset_px
    }

    fn clip_child_frame(&self) -> bool {
        self.clip
    }

    fn show_child_chrome(&self) -> bool {
        self.show_child_chrome
    }

    fn overlay_label(&self) -> &'static str {
        "ParallelAxisOverlay"
    }

    fn scene_group_prefix(&self) -> &'static str {
        "parallel_axis_overlay"
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledParallelAxisOverlay {
    async fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &dyn MarkRuntimeContext,
        _coord: &dyn avenger_chart_core::CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "ParallelAxisOverlay marks require the top-level layout render dispatcher".to_string(),
        ))
    }

    async fn render_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn avenger_chart_core::CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        self.render_from_data(data, scalars, context, coord)
            .await
            .map(RenderedMarkData::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_chart_core::CompiledSubplotChildPlot;
    use std::any::Any;

    #[derive(Clone)]
    struct FakeChildSpec {
        has_plot_level_data: bool,
    }

    #[async_trait::async_trait]
    impl SubplotChildPlotSpec for FakeChildSpec {
        fn clone_box(&self) -> Box<dyn SubplotChildPlotSpec> {
            Box::new(self.clone())
        }

        fn has_plot_level_data(&self) -> bool {
            self.has_plot_level_data
        }

        async fn compile_boxed(
            &self,
            _session_context: &SessionContext,
        ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError> {
            Ok(Arc::new(FakeCompiledChildPlot))
        }
    }

    #[derive(Clone, Serialize, Deserialize)]
    struct FakeCompiledChildPlot;

    #[typetag::serde]
    impl CompiledSubplotChildPlot for FakeCompiledChildPlot {
        fn as_any(&self) -> &dyn Any {
            self
        }

        fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
            self
        }
    }

    #[test]
    fn axis_overlay_compile_stores_dimension_geometry_and_payload() {
        let overlay = ParallelAxisOverlay::new(
            "speed",
            FakeChildSpec {
                has_plot_level_data: true,
            },
        )
        .id("speed_overlay")
        .width_px(28.0)
        .x_offset_px(3.0)
        .clip(true)
        .show_child_chrome(true)
        .zindex(12);
        let compiled_state =
            CompiledMarkState::from_mark_state(overlay.state(), None).with_mark_index(4);
        let compiled =
            futures::executor::block_on(overlay.compile(compiled_state, &SessionContext::new()))
                .expect("compile overlay");
        let compiled = compiled
            .as_any()
            .downcast_ref::<CompiledParallelAxisOverlay>()
            .expect("compiled overlay");

        assert_eq!(compiled.mark_type(), "parallel_axis_overlay");
        assert_eq!(compiled.dimension_id(), "speed");
        assert_eq!(compiled.width_px(), 28.0);
        assert_eq!(compiled.x_offset_px(), 3.0);
        assert!(compiled.clip());
        assert!(compiled.show_child_chrome());
        assert_eq!(compiled.state().id.as_deref(), Some("speed_overlay"));
        assert_eq!(compiled.state().mark_index(), 4);
        assert_eq!(
            compiled.payload().data_source(),
            SubplotDataSource::ExplicitChild
        );
        assert_eq!(compiled.payload().key(), Some("speed"));
    }

    #[test]
    fn axis_overlay_rejects_invalid_dimension_id() {
        let overlay = ParallelAxisOverlay::new(
            "bad.id",
            FakeChildSpec {
                has_plot_level_data: false,
            },
        );
        let compiled_state = CompiledMarkState::from_mark_state(overlay.state(), None);
        let err = match futures::executor::block_on(
            overlay.compile(compiled_state, &SessionContext::new()),
        ) {
            Ok(_) => panic!("invalid id should fail"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("Invalid parallel axis overlay"));
    }
}
