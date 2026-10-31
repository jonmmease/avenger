//! The build of a chart app, which installs the update that waits in the feed and the newest update
//! of its task, applies the feed's mode and selections and the moves of the scales whose domains
//! are selection domains, and evaluates the chart at the size of the app.

use async_trait::async_trait;
use avenger_app::{
    app::{AppBuild, BuiltScene, Scene, Scenes},
    error::AvengerAppError,
};
use avenger_chart_eval::EvalError;

use crate::{
    feed::Pending,
    state::{ChartAppState, ChartChange},
    tools, CHART_SCENE,
};

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
        let waiting = state.feed.take();
        if let Some(pending) = waiting.update {
            let update = match pending {
                Pending::Prepared(update) => *update,
                Pending::Unprepared(update) => state.prepare(*update).map_err(app_error)?,
            };
            state.install(update).map_err(app_error)?;
        }
        let finished = state.charts.as_mut().and_then(|charts| charts.take());
        if let Some(update) = finished {
            state.install(update).map_err(app_error)?;
        }
        if let Some(mode) = waiting.mode {
            state.mode = mode;
        }
        for (name, contributions) in waiting.selections {
            state.set_selection(&name, contributions);
        }
        tools::write_moved(state, None);
        state.report(ChartChange::default());
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
