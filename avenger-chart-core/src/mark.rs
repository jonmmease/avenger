use std::sync::Arc;

use datafusion::prelude::SessionContext;

use crate::{
    AvengerChartError, CompiledMark, CompiledMarkState, CoordinateSystemCore, DataContext,
    MarkState,
};

/// Core trait for all uncompiled mark types.
#[async_trait::async_trait]
pub trait Mark<C: CoordinateSystemCore>: Send + Sync + 'static {
    /// Get the mark's construction-time state.
    fn state(&self) -> &MarkState;

    /// Get mutable construction-time state.
    fn state_mut(&mut self) -> &mut MarkState;

    /// Get the mark's construction-time data/channel context.
    fn data_context(&self) -> &DataContext;

    /// Compile this mark into a render-capable object-safe compiled mark.
    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError>;

    /// Compile without applying a transformed DataFrame.
    async fn compile_untransformed(
        &self,
        ctx: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        let mark_state = self.state();
        let df = mark_state.data.dataframe().cloned();
        let compiled_state = CompiledMarkState::from_mark_state(mark_state, df);
        self.compile(compiled_state, ctx).await
    }
}
