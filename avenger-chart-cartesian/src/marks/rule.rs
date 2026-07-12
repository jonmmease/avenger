use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, CompiledMark, CompiledMarkState, Mark, impl_mark_trait_common,
};
use avenger_chart_marks::{CompiledRule, Rule};

use crate::Cartesian;

pub type CompiledCartesianRule = CompiledRule;

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<Cartesian> for Rule<Cartesian> {
    impl_mark_trait_common!(Rule);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledRule::new(
            compiled_state,
            self.mark_effects().clone(),
        )))
    }
}
