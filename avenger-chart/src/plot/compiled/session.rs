//! Reusable evaluation session for a compiled plot.

use std::sync::Arc;

use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;

use crate::{
    error::AvengerChartError,
    render::{EvaluatedPlot, EvaluationMetrics, EvaluationMode, EvaluationOptions},
};

use super::CompiledPlot;

/// Request object for evaluating a `PlotSession`.
#[derive(Clone, Debug)]
pub struct EvaluationRequest {
    params: Option<IndexMap<String, ScalarValue>>,
    param_patch: Option<IndexMap<String, ScalarValue>>,
    mode: EvaluationMode,
    options: EvaluationOptions,
}

impl Default for EvaluationRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl EvaluationRequest {
    pub fn new() -> Self {
        Self {
            params: None,
            param_patch: None,
            mode: EvaluationMode::Exact,
            options: EvaluationOptions::default(),
        }
    }

    pub fn params(mut self, params: IndexMap<String, ScalarValue>) -> Self {
        self.params = Some(params);
        self
    }

    pub fn param_patch(mut self, param_patch: IndexMap<String, ScalarValue>) -> Self {
        self.param_patch = Some(param_patch);
        self
    }

    pub fn mode(mut self, mode: EvaluationMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn options(mut self, options: EvaluationOptions) -> Self {
        self.options = options;
        self
    }

    pub fn exact(self) -> Self {
        self.mode(EvaluationMode::Exact)
    }

    pub fn preview(self) -> Self {
        self.mode(EvaluationMode::Preview)
    }

    pub fn force_remeasure(self) -> Self {
        self.mode(EvaluationMode::ForceRemeasure)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EvaluationRequestSummary {
    mode: EvaluationMode,
    params: IndexMap<String, ScalarValue>,
}

/// Stateful runtime instance for evaluating one compiled plot repeatedly.
pub struct PlotSession {
    program: Arc<CompiledPlot>,
    ctx: Arc<SessionContext>,
    current_params: IndexMap<String, ScalarValue>,
    last_request: Option<EvaluationRequestSummary>,
    last_measurement: Option<()>,
    last_metrics: Option<EvaluationMetrics>,
}

impl PlotSession {
    pub(crate) fn new(program: Arc<CompiledPlot>, ctx: Arc<SessionContext>) -> Self {
        let current_params = program.get_default_params().clone();
        Self {
            program,
            ctx,
            current_params,
            last_request: None,
            last_measurement: None,
            last_metrics: None,
        }
    }

    pub fn params(&self) -> &IndexMap<String, ScalarValue> {
        &self.current_params
    }

    pub fn set_params(&mut self, params: IndexMap<String, ScalarValue>) {
        let mut merged = self.program.get_default_params().clone();
        merged.extend(params);
        self.current_params = merged;
    }

    pub fn apply_param_patch(&mut self, patch: IndexMap<String, ScalarValue>) {
        self.current_params.extend(patch);
    }

    pub fn last_metrics(&self) -> Option<&EvaluationMetrics> {
        self.last_metrics.as_ref()
    }

    pub fn take_metrics(&mut self) -> Option<EvaluationMetrics> {
        self.last_metrics.take()
    }

    pub async fn evaluate(
        &mut self,
        request: EvaluationRequest,
    ) -> Result<EvaluatedPlot, AvengerChartError> {
        let (evaluated, _) = self.evaluate_with_metrics(request).await?;
        Ok(evaluated)
    }

    pub async fn evaluate_with_metrics(
        &mut self,
        request: EvaluationRequest,
    ) -> Result<(EvaluatedPlot, EvaluationMetrics), AvengerChartError> {
        let mode = request.mode;
        let next_params = self.params_for_request(&request);
        let (evaluated, mut metrics) = self
            .program
            .evaluate_with_options_and_metrics(
                self.ctx.as_ref(),
                Some(next_params.clone()),
                request.options,
            )
            .await?;
        metrics.mode = mode;
        self.current_params = next_params.clone();
        self.last_request = Some(EvaluationRequestSummary {
            mode,
            params: next_params,
        });
        self.last_measurement = Some(());
        self.last_metrics = Some(metrics.clone());
        Ok((evaluated, metrics))
    }

    fn params_for_request(&self, request: &EvaluationRequest) -> IndexMap<String, ScalarValue> {
        let mut params = self.program.get_default_params().clone();
        params.extend(
            request
                .params
                .clone()
                .unwrap_or_else(|| self.current_params.clone()),
        );
        if let Some(patch) = &request.param_patch {
            params.extend(patch.clone());
        }
        params
    }
}

impl CompiledPlot {
    /// Instantiate this compiled plot as a reusable evaluation session.
    pub fn instantiate(self: Arc<Self>, ctx: Arc<SessionContext>) -> PlotSession {
        PlotSession::new(self, ctx)
    }

    /// Instantiate this compiled plot and initialize session params.
    pub fn instantiate_with_params(
        self: Arc<Self>,
        ctx: Arc<SessionContext>,
        params: IndexMap<String, ScalarValue>,
    ) -> PlotSession {
        let mut session = PlotSession::new(self, ctx);
        session.set_params(params);
        session
    }
}

#[cfg(test)]
mod tests {
    use datafusion::{prelude::SessionContext, scalar::ScalarValue};

    use crate::prelude::*;

    use super::*;

    async fn compile_session_test_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = ctx
            .sql("SELECT * FROM (VALUES (1.0, 2.0), (2.0, 3.0), (3.0, 5.0)) AS t(x, y)")
            .await?;
        Plot::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")).size(20.0))
            .compile(ctx)
            .await
    }

    #[tokio::test]
    async fn plot_session_exact_matches_one_shot() -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_session_test_plot(&ctx).await?);

        let one_shot = compiled.evaluate(&ctx, None).await?;
        let mut session = compiled.clone().instantiate(ctx.clone());
        let session_eval = session.evaluate(EvaluationRequest::new().exact()).await?;

        assert_eq!(session_eval.scene_graph.width, one_shot.scene_graph.width);
        assert_eq!(session_eval.scene_graph.height, one_shot.scene_graph.height);
        assert_eq!(
            session_eval.scene_graph.marks.len(),
            one_shot.scene_graph.marks.len()
        );
        assert_eq!(
            session.last_metrics().map(|metrics| metrics.mode),
            Some(EvaluationMode::Exact)
        );

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_instances_keep_independent_params() -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_session_test_plot(&ctx).await?);
        let mut left = compiled.clone().instantiate(ctx.clone());
        let mut right = compiled.instantiate(ctx);

        let mut left_patch = IndexMap::new();
        left_patch.insert("zoom".to_string(), ScalarValue::Float64(Some(1.0)));
        let mut right_patch = IndexMap::new();
        right_patch.insert("zoom".to_string(), ScalarValue::Float64(Some(2.0)));

        left.apply_param_patch(left_patch);
        right.apply_param_patch(right_patch);

        assert_eq!(
            left.params().get("zoom"),
            Some(&ScalarValue::Float64(Some(1.0)))
        );
        assert_eq!(
            right.params().get("zoom"),
            Some(&ScalarValue::Float64(Some(2.0)))
        );

        Ok(())
    }

    #[tokio::test]
    async fn plot_session_metrics_record_requested_modes() -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Arc::new(compile_session_test_plot(&ctx).await?);
        let mut session = compiled.instantiate(ctx);

        for mode in [
            EvaluationMode::Exact,
            EvaluationMode::Preview,
            EvaluationMode::ForceRemeasure,
        ] {
            let (_evaluated, metrics) = session
                .evaluate_with_metrics(EvaluationRequest::new().mode(mode))
                .await?;
            assert_eq!(metrics.mode, mode);
            assert_eq!(
                session.last_metrics().map(|metrics| metrics.mode),
                Some(mode)
            );
        }

        Ok(())
    }
}
