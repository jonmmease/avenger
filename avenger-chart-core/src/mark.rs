use std::{any::Any, sync::Arc};

use datafusion::prelude::SessionContext;

use crate::{
    AvengerChartError, CompiledMark, CompiledMarkState, CoordinateSystemCore, DataContext,
    MarkScaleDomainChannel, MarkState,
};

/// Erased compile-time context passed through subplot compilation.
///
/// The top-level chart crate uses this to let root-owned authoring features,
/// such as tools, influence descendant child plots without making core depend
/// on the concrete feature types.
pub type CompileContext<'a> = &'a (dyn Any + Send + Sync);

/// Core trait for all uncompiled mark types.
#[async_trait::async_trait]
pub trait Mark<C: CoordinateSystemCore>: Send + Sync + 'static {
    /// Get the mark's construction-time state.
    fn state(&self) -> &MarkState;

    /// Get mutable construction-time state.
    fn state_mut(&mut self) -> &mut MarkState;

    /// Get the mark's construction-time data/channel context.
    fn data_context(&self) -> &DataContext;

    /// Return mark-owned scale-domain channels that are not ordinary render
    /// channels.
    ///
    /// This mirrors the compiled-mark hook and lets compile-time features such
    /// as tools discover scale targets before marks are compiled.
    fn scale_domain_channels(&self) -> Result<Vec<MarkScaleDomainChannel>, AvengerChartError> {
        Ok(Vec::new())
    }

    /// Compile this mark into a render-capable object-safe compiled mark.
    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError>;

    /// Compile with an optional erased compile-time context.
    async fn compile_with_context(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
        _compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        self.compile(compiled_state, session_context).await
    }

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
