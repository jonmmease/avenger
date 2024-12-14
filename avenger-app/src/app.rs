use std::sync::Arc;
use std::time::Instant;

use avenger_eventstream::manager::{EventStreamHandler, EventStreamManager};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::WindowEvent;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;

pub trait SceneGraphBuilder<State: Clone + Send + Sync + 'static> {
    fn build(&self, state: &State) -> SceneGraph;
}

impl<State, F> SceneGraphBuilder<State> for F
where
    State: Clone + Send + Sync + 'static,
    F: Fn(&State) -> SceneGraph + 'static,
{
    fn build(&self, state: &State) -> SceneGraph {
        self(state)
    }
}

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
    pub fn new(
        initial_state: State,
        scene_graph_builder: Arc<dyn SceneGraphBuilder<State>>,
        stream_callbacks: Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)>,
    ) -> Self {
        let mut event_stream_manager = EventStreamManager::new(initial_state);
        for (config, handler) in stream_callbacks {
            event_stream_manager.register_handler(config, handler);
        }
        // Build initial scene graph and rtree
        let scene_graph = Arc::new(scene_graph_builder.build(event_stream_manager.state()));
        let rtree = SceneGraphRTree::from_scene_graph(&scene_graph);

        Self {
            scene_graph_builder,
            event_stream_manager,
            rtree,
            scene_graph,
        }
    }

    /// Update the state of the app without rebuilding the scene graph
    pub fn update_state(&mut self, event: &WindowEvent, instant: Instant) -> UpdateStatus {
        self.event_stream_manager
            .dispatch_event(event, &self.rtree, instant)
    }

    /// Update the state of the app and rebuild the scene graph if needed
    pub fn update(&mut self, event: &WindowEvent, instant: Instant) -> Option<Arc<SceneGraph>> {
        let update_status = self
            .event_stream_manager
            .dispatch_event(event, &self.rtree, instant);

        // Reconstruct the scene graph if the need to rerender or rebuild geometry
        if update_status.rerender || update_status.rebuild_geometry {
            self.scene_graph = Arc::new(
                self.scene_graph_builder
                    .build(self.event_stream_manager.state()),
            );
        }

        // Rebuild the rtree if the need to rebuild geometry
        if update_status.rebuild_geometry {
            self.rtree = SceneGraphRTree::from_scene_graph(&self.scene_graph);
        }

        // Return the scene graph if the need to rerender
        if update_status.rerender {
            Some(self.scene_graph.clone())
        } else {
            None
        }
    }

    pub fn scene_graph(&self) -> &SceneGraph {
        &self.scene_graph
    }
}
