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
    window::Key,
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
        self.build_with_effects(state).await.map(|b| b.scene_graph)
    }
    async fn build_with_effects(
        &self,
        state: &mut state::State,
    ) -> Result<avenger_app::app::SceneBuild, AvengerAppError> {
        let out = scene::build_live(state).map_err(AvengerAppError::InternalError)?;
        Ok(avenger_app::app::SceneBuild {
            scene_graph: out.scene,
            commands: out.widget_update.status.commands,
            rebuild_geometry: out.widget_update.status.rebuild_geometry,
        })
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
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let update = match state
            .widgets
            .handle(event, rtree, avenger_common::time::Instant::now())
        {
            Ok(update) => update,
            Err(_) => return UpdateStatus::default(),
        };
        let mut status = update.status;
        if state.apply_widgets(update.events) {
            status.rerender = true;
            status.rebuild_geometry = true;
        }
        if status.consume {
            return status;
        }
        match event {
            SceneGraphEvent::KeyPress(e) => {
                if e.repeat || e.modifiers.control || e.modifiers.alt || e.modifiers.meta {
                    return status;
                }
                let Key::Character(key) = &e.key else {
                    return status;
                };
                let Some(index) = key
                    .to_digit(10)
                    .map(|n| n as usize)
                    .filter(|n| (1..=8).contains(n))
                else {
                    return status;
                };
                state.activate(index - 1);
            }
            SceneGraphEvent::WindowResize(e) => state.size = e.size,
            SceneGraphEvent::CanvasResize(e) => state.size = e.size,
            _ => return status,
        }
        status.rerender = true;
        status.rebuild_geometry = true;
        status
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
                    SceneGraphEventType::MouseUp,
                    SceneGraphEventType::CursorMoved,
                    SceneGraphEventType::MarkMouseLeave,
                    SceneGraphEventType::KeyRelease,
                    SceneGraphEventType::FocusEntered,
                    SceneGraphEventType::PointerCaptureLost,
                    SceneGraphEventType::WindowFocused,
                    SceneGraphEventType::WindowCloseRequested,
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
