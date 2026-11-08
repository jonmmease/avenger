//! The build of a chart app, which applies the updates and the commands of its handle in the order
//! of their calls, writes the moves of the scales whose domains read a selection, and evaluates
//! the chart at the size of the app.

use async_trait::async_trait;
use avenger_app::{
    app::{AppBuild, BuiltScene, Scene, Scenes},
    error::AvengerAppError,
};
use avenger_chart_eval::EvalError;
use avenger_style::HostInputs;

use crate::{
    selections,
    state::{ChartAppState, ChartChange},
    tools,
    updates::Applied,
    CHART_SCENE,
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
        for applied in state.updates.take() {
            match applied {
                Applied::Chart(update) => state.install(*update).map_err(app_error)?,
                Applied::Styles(cascade) => state.evaluator.set_cascade(cascade),
                Applied::Mode(mode) => state.evaluator.set_mode(mode),
                Applied::Selection(name, selection) => state.set_selection(&name, selection),
            }
        }
        tools::write_moved(state, None);
        state.report(ChartChange::default());
        if !scenes.requested(CHART_SCENE) {
            return Ok(());
        }
        let [width, height] = scenes.size();
        let evaluated = (state.evaluator)
            .evaluate(&HostInputs::sized(width, height))
            .map_err(app_error)?;
        state.plots = evaluated.plots;
        state.warnings = evaluated.warnings;
        state.controls.set(evaluated.widgets);
        selections::refocus(state);
        scenes.set(
            CHART_SCENE,
            BuiltScene {
                scene: evaluated.scene,
                data: evaluated.data,
            },
        )
    }
}
