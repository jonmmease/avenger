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

struct Builder;
#[async_trait]
impl SceneGraphBuilder<state::State> for Builder {
    async fn build(&self, state: &mut state::State) -> Result<SceneGraph, AvengerAppError> {
        scene::build(state).map_err(AvengerAppError::InternalError)
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
