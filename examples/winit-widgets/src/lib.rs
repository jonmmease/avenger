//! Plot Style Studio: reusable controls bound to ordinary application values.
use async_trait::async_trait;
use avenger_app::{
    app::{AvengerApp, SceneBuild, SceneGraphBuilder},
    error::AvengerAppError,
};
use avenger_common::time::Instant;
use avenger_eventstream::{
    manager::EventStreamHandler,
    scene::{SceneGraphEvent as Event, SceneGraphEventType as Type},
    stream::{EventStreamConfig, UpdateStatus},
    window::MouseButton,
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
    ) -> Result<SceneBuild, AvengerAppError> {
        scene::build(state).map_err(AvengerAppError::InternalError)
    }
}
struct Input;
#[cfg_attr(target_arch="wasm32",async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl EventStreamHandler<state::State> for Input {
    async fn handle(
        &self,
        event: &Event,
        state: &mut state::State,
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let now = Instant::now();
        let update = match state.widgets.handle(event, rtree, now) {
            Ok(update) => update,
            Err(error) => {
                state.last_action = error.to_string();
                return UpdateStatus {
                    rerender: true,
                    ..Default::default()
                };
            }
        };
        let mut status = update.status;
        status.commands.extend(state.apply(update.events, now));
        if status.consume {
            return status;
        }
        match event {
            Event::MouseDown(e)
                if e.button == MouseButton::Left
                    && e.mark_instance.as_ref().is_some_and(|m| m.name == "plot") =>
            {
                state.plot_drag = Some((e.position, state.pan));
                status.consume = true;
                status.suppress_click = true;
                status.commands.push(
                    avenger_eventstream::runtime::RuntimeHostCommand::SetPointerCapture {
                        captured: true,
                    },
                );
            }
            Event::CursorMoved(e) => {
                if let Some((start, pan)) = state.plot_drag {
                    state.pan = [
                        pan[0] + e.position[0] - start[0],
                        pan[1] + e.position[1] - start[1],
                    ];
                    status.rerender = true;
                    status.consume = true;
                    state.last_action = "Plot panned".into();
                }
            }
            Event::MouseUp(e) if e.button == MouseButton::Left => {
                if state.plot_drag.take().is_some() {
                    status.consume = true;
                    status.commands.push(
                        avenger_eventstream::runtime::RuntimeHostCommand::SetPointerCapture {
                            captured: false,
                        },
                    );
                }
            }
            Event::WindowFocused(false)
            | Event::PointerCaptureLost
            | Event::WindowCloseRequested => {
                state.plot_drag = None;
            }
            Event::WindowResize(e) => {
                state.size = e.size;
                status.rerender = true;
                status.rebuild_geometry = true;
            }
            Event::CanvasResize(e) => {
                state.size = e.size;
                status.rerender = true;
                status.rebuild_geometry = true;
            }
            _ => {}
        }
        status
    }
}
pub async fn make_app(state: state::State) -> Result<AvengerApp<state::State>, AvengerAppError> {
    let engine = state.engine.clone();
    AvengerApp::try_new_with_text_engine(
        state,
        Arc::new(Builder),
        vec![(
            EventStreamConfig {
                types: vec![
                    Type::MouseDown,
                    Type::MouseUp,
                    Type::CursorMoved,
                    Type::MarkMouseLeave,
                    Type::KeyPress,
                    Type::KeyRelease,
                    Type::TextInput,
                    Type::RuntimeWake,
                    Type::FocusEntered,
                    Type::PointerCaptureLost,
                    Type::WindowFocused,
                    Type::WindowCloseRequested,
                    Type::WindowResize,
                    Type::CanvasResize,
                ],
                ..Default::default()
            },
            Arc::new(Input),
        )],
        engine,
    )
    .await
}
