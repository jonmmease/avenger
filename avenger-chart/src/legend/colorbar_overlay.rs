use std::sync::Arc;

use avenger_chart_cartesian::Cartesian;
use avenger_chart_core::{
    AvengerChartError, CompiledMark, CompiledMarkState, DefaultLogicalExprNodeExt, Legend, Mark,
    MarkDataMode,
};
use datafusion::{common::ScalarValue, logical_expr::Expr, prelude::SessionContext};

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
            validate_overlay_mark(mark.as_ref(), session_context)?;
            let mark_state = mark.state();
            let df_opt = if mark_state.data_mode == MarkDataMode::Unit {
                None
            } else {
                mark_state.data.dataframe().cloned()
            };
            let compiled_state =
                CompiledMarkState::from_mark_state(mark_state, df_opt).with_mark_index(mark_index);
            let compiled_mark = mark.compile(compiled_state, session_context).await?;
            if compiled_mark.as_positioned_subplot().is_some() {
                return Err(AvengerChartError::InvalidArgument(
                    "Colorbar overlay marks must be ordinary Cartesian marks; positioned subplots are not supported in colorbar overlays".to_string(),
                ));
            }
            compiled.push(compiled_mark);
        }
        Ok(compiled)
    }
}

fn validate_overlay_mark(
    mark: &dyn Mark<Cartesian>,
    session_context: &SessionContext,
) -> Result<(), AvengerChartError> {
    for (channel, value) in mark.data_context().channels() {
        if matches!(channel.as_str(), "x" | "x2" | "y" | "y2") && value.get_scale_config().is_some()
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Colorbar overlay channel '{channel}' cannot define its own scale; colorbar overlays use the injected colorbar x/y scales"
            )));
        }
        if let Some(legend) = value.get_legend_config()
            && legend_requests_visible_output(legend, session_context)?
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Colorbar overlay channel '{channel}' cannot create a legend"
            )));
        }
    }
    Ok(())
}

fn legend_requests_visible_output(
    legend: &Legend,
    session_context: &SessionContext,
) -> Result<bool, AvengerChartError> {
    let Some(visible) = legend.visible.as_option() else {
        return Ok(true);
    };
    let Some(visible) = visible else {
        return Ok(false);
    };
    match visible.to_expr(session_context)? {
        Expr::Literal(ScalarValue::Boolean(Some(false)), _) => Ok(false),
        _ => Ok(true),
    }
}
