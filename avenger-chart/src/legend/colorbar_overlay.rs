use avenger_chart_cartesian::Cartesian;
use avenger_chart_core::{
    AvengerChartError, DefaultLogicalExprNodeExt, IntoPlotMark, Legend, Mark, PlotMark,
};
use datafusion::{common::ScalarValue, logical_expr::Expr, prelude::SessionContext};

#[derive(Clone, Default)]
pub struct ColorbarOverlay {
    marks: Vec<PlotMark<Cartesian>>,
}

impl ColorbarOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mark<M>(mut self, mark: M) -> Self
    where
        M: IntoPlotMark<Cartesian>,
    {
        self.marks.extend(mark.into_plot_marks());
        self
    }

    pub(crate) fn marks(&self) -> &[PlotMark<Cartesian>] {
        &self.marks
    }
}

pub(crate) fn validate_overlay_mark(
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
