use std::path::PathBuf;
use std::sync::Arc;
use avenger_common::time::Instant;

use async_trait::async_trait;
use avenger_eventstream::manager::{EventStreamHandler, EventStreamManager};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::WindowEvent;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;

use crate::error::AvengerAppError;

#[async_trait]
pub trait SceneGraphBuilder<State: Clone + Send + Sync + 'static> {
    async fn build(&self, state: &mut State) -> Result<SceneGraph, AvengerAppError>;
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
        let update_status = self
            .event_stream_manager
            .dispatch_event(event, &self.rtree, instant)
            .await;

        // Reconstruct the scene graph if the need to rerender or rebuild geometry
        if update_status.rerender || update_status.rebuild_geometry {
            let scene_graph = match self
                .scene_graph_builder
                .build(self.event_stream_manager.state_mut())
                .await
            {
                Ok(scene_graph) => scene_graph,
                Err(e) => {
                    eprintln!("Failed to build scene graph: {:?}", e);
                    return Err(AvengerAppError::InternalError(
                        "Failed to build scene graph".to_string(),
                    ));
                }
            };

            self.scene_graph = Arc::new(scene_graph);
        }

        // Rebuild the rtree if the need to rebuild geometry
        if update_status.rebuild_geometry {
            self.rtree = SceneGraphRTree::from_scene_graph(&self.scene_graph);
        }

        // Return the scene graph if the need to rerender
        if update_status.rerender {
            Ok(Some(self.scene_graph.clone()))
        } else {
            Ok(None)
        }
    }

    pub fn scene_graph(&self) -> &SceneGraph {
        &self.scene_graph
    }
}
