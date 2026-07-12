use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, CompiledMark, CompiledMarkState, Mark, impl_mark_trait_common,
};
use avenger_chart_marks::{CompiledText, Text};

use crate::Cartesian;

pub type CompiledCartesianText = CompiledText;

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<Cartesian> for Text<Cartesian> {
    impl_mark_trait_common!(Text);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledText::new(
            compiled_state,
            self.mark_effects().clone(),
            self.text_syntax_mode(),
        )))
    }
}
