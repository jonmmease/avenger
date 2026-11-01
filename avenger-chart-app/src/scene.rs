//! The build of a chart app, which shows a chart that waits in the feed or the newest chart of its
//! task, and evaluates the chart at the size of the app.

use async_trait::async_trait;
use avenger_app::{
    app::{AppBuild, BuiltScene, Scene, Scenes},
    error::AvengerAppError,
};
use avenger_chart_eval::EvalError;

use crate::{state::ChartAppState, CHART_SCENE};

pub(crate) struct ChartBuild;

pub(crate) fn app_error(error: EvalError) -> AvengerAppError {
    AvengerAppError::InternalError(error.to_string())
}

#[async_trait]
impl AppBuild<ChartAppState> for ChartBuild {
    fn scenes(&self) -> Vec<Scene<ChartAppState>> {
        vec![Scene::new(CHART_SCENE, 0)]
    }

    async fn build(
        &self,
        state: &mut ChartAppState,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if let Some(chart) = state.feed.take() {
            state.swap(&chart).map_err(app_error)?;
        }
        let finished = state.charts.as_mut().and_then(|charts| charts.take());
        if let Some(chart) = finished {
            state.swap(&chart).map_err(app_error)?;
        }
        if !scenes.requested(CHART_SCENE) {
            return Ok(());
        }
        let evaluated = state.evaluator.evaluate(scenes.size()).map_err(app_error)?;
        state.plots = evaluated.plots;
        state.warnings = evaluated.warnings;
        scenes.set(
            CHART_SCENE,
            BuiltScene {
                scene: evaluated.scene,
                data: evaluated.data,
            },
        )
    }
}
