use std::sync::Arc;

use avenger_chart_cartesian::Cartesian;
use avenger_chart_core::{AvengerChartError, CompiledMark, CompiledMarkState, Mark, MarkDataMode};
use datafusion::prelude::SessionContext;

#[derive(Clone, Default)]
pub struct ColorbarOverlay {
    marks: Vec<Arc<dyn Mark<Cartesian>>>,
}

impl ColorbarOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mark<M>(mut self, mark: M) -> Self
    where
        M: Mark<Cartesian> + 'static,
    {
        self.marks.push(Arc::new(mark));
        self
    }

    pub(crate) async fn compile(
        &self,
        session_context: &SessionContext,
    ) -> Result<Vec<Arc<dyn CompiledMark>>, AvengerChartError> {
        let mut compiled = Vec::with_capacity(self.marks.len());
        for (mark_index, mark) in self.marks.iter().enumerate() {
            let mark_state = mark.state();
            let df_opt = if mark_state.data_mode == MarkDataMode::Unit {
                None
            } else {
                mark_state.data.dataframe().cloned()
            };
            let compiled_state =
                CompiledMarkState::from_mark_state(mark_state, df_opt).with_mark_index(mark_index);
            compiled.push(mark.compile(compiled_state, session_context).await?);
        }
        Ok(compiled)
    }
}
