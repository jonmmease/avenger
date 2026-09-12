use async_trait::async_trait;
use avenger_app::{
    app::{AvengerApp, SceneBuild, SceneGraphBuilder},
    error::AvengerAppError,
};
use std::sync::Arc;

pub mod interaction;
pub mod reload;
pub mod scene;
pub mod state;
mod tasks;
pub mod text_edit;

#[cfg(target_arch = "wasm32")]
mod web;

struct Builder;
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl SceneGraphBuilder<state::State> for Builder {
    async fn build(&self, state: &mut state::State) -> Result<SceneBuild, AvengerAppError> {
        scene::build(state)
            .map(SceneBuild::new)
            .map_err(AvengerAppError::InternalError)
    }
}

pub async fn make_app(state: state::State) -> Result<AvengerApp<state::State>, AvengerAppError> {
    let engine = state.engine.clone();
    AvengerApp::try_new(
        state,
        Arc::new(Builder),
        interaction::registrations(),
        engine,
    )
    .await
}
