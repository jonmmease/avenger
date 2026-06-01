use std::{path::PathBuf, sync::Arc, time::Instant as StdInstant};

use async_trait::async_trait;
use avenger_common::time::Instant;
use avenger_eventstream::{
    manager::{EventStreamHandler, EventStreamManager},
    stream::{EventStreamConfig, UpdateStatus},
    window::WindowEvent,
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;

use crate::error::AvengerAppError;

#[async_trait]
pub trait SceneGraphBuilder<State: Clone + Send + Sync + 'static> {
    async fn build(&self, state: &mut State) -> Result<SceneGraph, AvengerAppError>;
}

#[derive(Clone)]
pub struct AppUpdate {
    pub scene_graph: Option<Arc<SceneGraph>>,
    pub status: UpdateStatus,
}

#[derive(Clone)]
pub struct AvengerApp<State>
where
    State: Clone + Send + Sync + 'static,
{
    scene_graph_builder: Arc<dyn SceneGraphBuilder<State>>,
    event_stream_manager: EventStreamManager<State>,
    rtree: SceneGraphRTree,
    scene_graph: Arc<SceneGraph>,
}

impl<State> AvengerApp<State>
where
    State: Clone + Send + Sync + 'static,
{
    /// Get a mutable reference to the app state
    pub fn app_state_mut(&mut self) -> &mut State {
        self.event_stream_manager.state_mut()
    }
    pub async fn try_new(
        initial_state: State,
        scene_graph_builder: Arc<dyn SceneGraphBuilder<State>>,
        stream_callbacks: Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)>,
    ) -> Result<Self, AvengerAppError> {
        let mut event_stream_manager = EventStreamManager::new(initial_state);
        for (config, handler) in stream_callbacks {
            event_stream_manager.register_handler(config, handler);
        }
        // Build initial scene graph and rtree
        let scene_graph = Arc::new(
            scene_graph_builder
                .build(event_stream_manager.state_mut())
                .await?,
        );
        let rtree = SceneGraphRTree::from_scene_graph(&scene_graph);

        Ok(Self {
            scene_graph_builder,
            event_stream_manager,
            rtree,
            scene_graph,
        })
    }

    pub fn get_watched_files(&self) -> Vec<PathBuf> {
        self.event_stream_manager.get_watched_files()
    }

    /// Update the state of the app without rebuilding the scene graph
    pub async fn update_state(&mut self, event: &WindowEvent, instant: Instant) -> UpdateStatus {
        self.event_stream_manager
            .dispatch_event(event, &self.rtree, instant)
            .await
    }

    /// Update the state of the app and rebuild the scene graph if needed
    pub async fn update(
        &mut self,
        event: &WindowEvent,
        instant: Instant,
    ) -> Result<Option<Arc<SceneGraph>>, AvengerAppError> {
        Ok(self.update_with_status(event, instant).await?.scene_graph)
    }

    /// Update the state of the app and return both scene and interaction status.
    pub async fn update_with_status(
        &mut self,
        event: &WindowEvent,
        instant: Instant,
    ) -> Result<AppUpdate, AvengerAppError> {
        tracing::debug!(target: "avenger_app::resize", "app.update start");
        let update_start = StdInstant::now();
        let dispatch_start = StdInstant::now();
        let update_status = self
            .event_stream_manager
            .dispatch_event(event, &self.rtree, instant)
            .await;
        let dispatch_elapsed = dispatch_start.elapsed();
        tracing::debug!(
            target: "avenger_app::resize",
            dispatch_ms = dispatch_elapsed.as_secs_f64() * 1000.0,
            rerender = update_status.rerender,
            rebuild_geometry = update_status.rebuild_geometry,
            "app.update dispatch"
        );

        // Reconstruct the scene graph if the need to rerender or rebuild geometry
        if update_status.rerender || update_status.rebuild_geometry {
            let scene_build_start = StdInstant::now();
            let scene_graph = match self
                .scene_graph_builder
                .build(self.event_stream_manager.state_mut())
                .await
            {
                Ok(scene_graph) => scene_graph,
                Err(e) => {
                    eprintln!("Failed to build scene graph: {e:?}");
                    return Err(AvengerAppError::InternalError(
                        "Failed to build scene graph".to_string(),
                    ));
                }
            };

            self.scene_graph = Arc::new(scene_graph);
            tracing::debug!(
                target: "avenger_app::resize",
                scene_build_ms = scene_build_start.elapsed().as_secs_f64() * 1000.0,
                "app.update scene build"
            );
        }

        // Rebuild the rtree if the need to rebuild geometry
        if update_status.rebuild_geometry {
            let rtree_start = StdInstant::now();
            self.rtree = SceneGraphRTree::from_scene_graph(&self.scene_graph);
            tracing::debug!(
                target: "avenger_app::resize",
                rtree_ms = rtree_start.elapsed().as_secs_f64() * 1000.0,
                "app.update rtree rebuild"
            );
        }

        // Return the scene graph if the need to rerender
        if update_status.rerender {
            let total_elapsed = update_start.elapsed();
            tracing::debug!(
                target: "avenger_app::resize",
                app_update_ms = total_elapsed.as_secs_f64() * 1000.0,
                rerender = true,
                "app.update complete"
            );
            Ok(AppUpdate {
                scene_graph: Some(self.scene_graph.clone()),
                status: update_status,
            })
        } else {
            tracing::debug!(
                target: "avenger_app::resize",
                app_update_ms = update_start.elapsed().as_secs_f64() * 1000.0,
                rerender = false,
                "app.update complete"
            );
            Ok(AppUpdate {
                scene_graph: None,
                status: update_status,
            })
        }
    }

    pub fn scene_graph(&self) -> &SceneGraph {
        &self.scene_graph
    }
}
