use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, CompiledMark, CompiledMarkState, Mark, impl_mark_trait_common,
};
use avenger_chart_marks::{CompiledRect, Rect};

use crate::Cartesian;

pub type CompiledCartesianRect = CompiledRect;

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<Cartesian> for Rect<Cartesian> {
    impl_mark_trait_common!(Rect);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledRect::new(
            compiled_state,
            self.mark_effects().clone(),
        )))
    }
}
