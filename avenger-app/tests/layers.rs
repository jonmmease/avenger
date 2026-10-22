//! An app is a stack of layers. A layer's status builds that layer and the layers above it, never
//! those below, and each layer builds over the scenes and trees of the layers below it.

use std::sync::Arc;

use async_trait::async_trait;
use avenger_app::app::{AvengerApp, BuildContext, Layer};
use avenger_app::error::AvengerAppError;
use avenger_app::waker::Refresh;
use avenger_common::time::Instant;
use avenger_eventstream::manager::{EventStreamHandler, LayerTrees};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::{
    ElementState, Key, WindowCursorMoved, WindowEvent, WindowKeyboardInput,
};
use avenger_geometry::query::{Query, QueryShape, Relation};
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::scene_graph::SceneGraph;

#[derive(Clone, Default)]
struct State {
    /// The builds of each layer, by layer.
    builds: [usize; 2],
    /// The scenes and trees below the top layer, and the width of the scene below it, as its
    /// last build saw them.
    below: Option<(usize, usize, f32)>,
    /// The name of the mark in the chart's tree, as the top layer's last build saw it.
    chart_tree: Option<String>,
    /// Whether the top layer's build fails.
    fail_top: bool,
}

/// Asks its layer for a new scene.
struct Rebuild;

#[async_trait]
impl EventStreamHandler<State> for Rebuild {
    async fn handle(&self, _: &SceneGraphEvent, _: &mut State, _: &LayerTrees<'_>) -> UpdateStatus {
        UpdateStatus {
            rerender: true,
            ..Default::default()
        }
    }
}

fn scene(width: f32) -> SceneGraph {
    SceneGraph {
        marks: Vec::new(),
        width,
        height: 10.0,
        origin: [0.0; 2],
    }
}

/// The chart, which rebuilds its scene but not its tree on a key. Its one mark is named for its
/// build.
struct Chart;

#[async_trait]
impl Layer<State> for Chart {
    async fn build(
        &self,
        state: &mut State,
        _: &BuildContext<'_>,
    ) -> Result<SceneGraph, AvengerAppError> {
        state.builds[0] += 1;
        let rect = SceneRectMark {
            name: format!("chart {}", state.builds[0]),
            width: Some(20.0.into()),
            height: Some(10.0.into()),
            ..Default::default()
        };
        Ok(SceneGraph {
            marks: vec![rect.into()],
            ..scene(20.0)
        })
    }

    fn streams(&self) -> Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> {
        let key = EventStreamConfig {
            types: vec![SceneGraphEventType::KeyPress],
            ..Default::default()
        };
        vec![(key, Arc::new(Rebuild))]
    }
}

/// A layer over the chart, which rebuilds on each move.
struct Top;

#[async_trait]
impl Layer<State> for Top {
    async fn build(
        &self,
        state: &mut State,
        context: &BuildContext<'_>,
    ) -> Result<SceneGraph, AvengerAppError> {
        state.builds[1] += 1;
        if state.fail_top {
            return Err(AvengerAppError::InternalError("top".to_string()));
        }
        state.below = Some((
            context.scenes_below().len(),
            context.trees_below().len(),
            context.scenes_below()[0].width,
        ));
        let mark = Query::new(QueryShape::Point([5.0, 5.0]), Relation::Intersects).limit(1);
        state.chart_tree = (context.trees_below()[0].query(&mark).first())
            .map(|instance| instance.mark_instance.name.clone());
        Ok(scene(20.0))
    }

    fn streams(&self) -> Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> {
        let moves = EventStreamConfig {
            types: vec![SceneGraphEventType::CursorMoved],
            ..Default::default()
        };
        vec![(moves, Arc::new(Rebuild))]
    }
}

async fn app() -> AvengerApp<State> {
    AvengerApp::try_new(
        State::default(),
        vec![Arc::new(Chart), Arc::new(Top)],
        avenger_typst_label::LabelEngine::new(Default::default()),
    )
    .await
    .unwrap()
}

fn moved() -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved {
        position: [5.0, 5.0],
    })
}

fn key() -> WindowEvent {
    WindowEvent::KeyboardInput(WindowKeyboardInput {
        key: Key::Character('a'),
        state: ElementState::Pressed,
    })
}

/// The app builds every layer from the bottom, and a layer builds over the scenes and trees of
/// the layers below it.
#[tokio::test(flavor = "current_thread")]
async fn layers_build_over_the_layers_below() {
    let mut app = app().await;
    assert_eq!(app.scenes().len(), 2);
    assert_eq!(app.app_state_mut().builds, [1, 1]);
    assert_eq!(app.app_state_mut().below, Some((1, 1, 20.0)));
}

/// A status from the top layer's stream builds the top layer alone, and one from the chart's
/// stream builds the chart and the layer above it.
#[tokio::test(flavor = "current_thread")]
async fn a_status_rebuilds_its_layer_and_those_above() {
    let mut app = app().await;
    let now = Instant::now();
    assert_eq!(app.update(&moved(), now).await.layers, [1]);
    assert_eq!(app.app_state_mut().builds, [1, 2]);
    assert_eq!(app.update(&key(), now).await.layers, [0, 1]);
    assert_eq!(app.app_state_mut().builds, [2, 3]);
}

/// A status that asks for a new scene but not a new tree keeps its layer's tree.
#[tokio::test(flavor = "current_thread")]
async fn a_rerender_keeps_its_layers_tree() {
    let mut app = app().await;
    let now = Instant::now();
    app.update(&moved(), now).await;
    assert_eq!(app.update(&key(), now).await.layers, [0, 1]);
    assert_eq!(app.app_state_mut().builds, [2, 3]);
    assert_eq!(app.app_state_mut().chart_tree.as_deref(), Some("chart 1"));
}

/// A rebuild that the waker asks for builds every layer and every tree.
#[tokio::test(flavor = "current_thread")]
async fn a_waker_rebuild_builds_every_layer() {
    let mut app = app().await;
    app.waker().request(Refresh::Rebuild);
    assert_eq!(app.refresh().await.layers, [0, 1]);
    assert_eq!(app.app_state_mut().builds, [2, 2]);
    assert_eq!(app.app_state_mut().chart_tree.as_deref(), Some("chart 2"));
}

/// A layer whose build fails keeps its scene, and the layers below it still change.
#[tokio::test(flavor = "current_thread")]
async fn a_failed_build_keeps_its_scene() {
    let mut app = app().await;
    let top = app.scenes()[1].clone();
    app.app_state_mut().fail_top = true;
    let now = Instant::now();
    app.update(&moved(), now).await;
    assert_eq!(app.update(&key(), now).await.layers, [0]);
    assert!(Arc::ptr_eq(&app.scenes()[1], &top));
}
