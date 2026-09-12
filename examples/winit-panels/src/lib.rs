//! Small-multiples explorer using the public avenger-panels API.
use async_trait::async_trait;
use avenger_app::{
    app::{AvengerApp, SceneGraphBuilder},
    error::AvengerAppError,
};
use avenger_eventstream::{
    manager::EventStreamHandler,
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{EventStreamConfig, UpdateStatus},
    window::{Key, MouseButton},
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;
use std::sync::Arc;

pub mod scene;
pub mod state;
#[cfg(target_arch = "wasm32")]
mod web;

struct Builder;
#[cfg_attr(target_arch="wasm32",async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl SceneGraphBuilder<state::State> for Builder {
    async fn build(&self, state: &mut state::State) -> Result<SceneGraph, AvengerAppError> {
        scene::build(state)
            .map(|out| out.scene)
            .map_err(AvengerAppError::InternalError)
    }
}
struct Input;
#[cfg_attr(target_arch="wasm32",async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl EventStreamHandler<state::State> for Input {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut state::State,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        match event {
            SceneGraphEvent::MouseDown(e) if e.button == MouseButton::Left => {
                let control = event
                    .mark_instance()
                    .and_then(|m| m.name.strip_prefix("control-"))
                    .and_then(|s| s.parse::<usize>().ok());
                if let Some(control) = control {
                    state.activate(control);
                } else {
                    return UpdateStatus::default();
                }
            }
            SceneGraphEvent::KeyPress(e) => {
                let Key::Character(key) = &e.key else {
                    return UpdateStatus::default();
                };
                let Some(index) = key
                    .to_digit(10)
                    .map(|n| n as usize)
                    .filter(|n| (1..=8).contains(n))
                else {
                    return UpdateStatus::default();
                };
                state.activate(index - 1);
            }
            SceneGraphEvent::WindowResize(e) => state.size = e.size,
            SceneGraphEvent::CanvasResize(e) => state.size = e.size,
            _ => return UpdateStatus::default(),
        }
        UpdateStatus {
            rerender: true,
            rebuild_geometry: true,
            ..Default::default()
        }
    }
}
/// Construct the app used by both native and browser hosts.
pub async fn make_app(state: state::State) -> Result<AvengerApp<state::State>, AvengerAppError> {
    let engine = state.engine.clone();
    AvengerApp::try_new_with_text_engine(
        state,
        Arc::new(Builder),
        vec![(
            EventStreamConfig {
                types: vec![
                    SceneGraphEventType::MouseDown,
                    SceneGraphEventType::KeyPress,
                    SceneGraphEventType::WindowResize,
                    SceneGraphEventType::CanvasResize,
                ],
                ..Default::default()
            },
            Arc::new(Input),
        )],
        engine,
    )
    .await
}
