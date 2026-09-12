use async_trait::async_trait;
use avenger_app::{
    app::{AvengerApp, SceneGraphBuilder},
    error::AvengerAppError,
};
use avenger_scenegraph::scene_graph::SceneGraph;
use std::sync::Arc;

pub mod interaction;
pub mod reload;
pub mod scene;
pub mod state;
mod tasks;

#[cfg(target_arch = "wasm32")]
mod web;

struct Builder;
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl SceneGraphBuilder<state::State> for Builder {
    async fn build(&self, state: &mut state::State) -> Result<SceneGraph, AvengerAppError> {
        self.build_with_effects(state).await.map(|b| b.scene_graph)
    }
    async fn build_with_effects(
        &self,
        state: &mut state::State,
    ) -> Result<avenger_app::app::SceneBuild, AvengerAppError> {
        scene::build_with_effects(state).map_err(AvengerAppError::InternalError)
    }
}

pub async fn make_app(state: state::State) -> Result<AvengerApp<state::State>, AvengerAppError> {
    let engine = state.engine.clone();
    AvengerApp::try_new_with_text_engine(
        state,
        Arc::new(Builder),
        interaction::registrations(),
        engine,
    )
    .await
}
