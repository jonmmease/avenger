use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use avenger_common::time::Instant;
use avenger_eventstream::{
    manager::{EventStreamHandler, EventStreamManager},
    stream::{EventStreamConfig, UpdateStatus},
    window::WindowEvent,
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;

use crate::{background::BackgroundTasks, error::AvengerAppError};

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait SceneGraphBuilder<State: Clone + Send + Sync + 'static>: Send + Sync {
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
    text_engine: avenger_text::TextEngine,
    background_tasks: Option<BackgroundTasks>,
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
        Self::try_new_with_text_engine(
            initial_state,
            scene_graph_builder,
            stream_callbacks,
            avenger_text::default_text_engine(),
        )
        .await
    }

    /// Build interaction geometry with the engine used by guides and rendering.
    pub async fn try_new_with_text_engine(
        initial_state: State,
        scene_graph_builder: Arc<dyn SceneGraphBuilder<State>>,
        stream_callbacks: Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)>,
        text_engine: avenger_text::TextEngine,
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
        let rtree = SceneGraphRTree::from_scene_graph_with_text_engine(&scene_graph, &text_engine);

        Ok(Self {
            scene_graph_builder,
            event_stream_manager,
            rtree,
            scene_graph,
            text_engine,
            background_tasks: None,
        })
    }

    /// Associate background work with this app's host lifetime.
    pub fn with_background_tasks(mut self, tasks: BackgroundTasks) -> Self {
        self.background_tasks = Some(tasks);
        self
    }

    /// Return the optional task group for host attachment.
    pub fn background_tasks(&self) -> Option<&BackgroundTasks> {
        self.background_tasks.as_ref()
    }

    pub fn text_engine(&self) -> &avenger_text::TextEngine {
        &self.text_engine
    }

    /// Rebuild current hit-test geometry when the host changes the text context.
    pub fn set_text_engine(&mut self, text_engine: avenger_text::TextEngine) {
        self.rtree =
            SceneGraphRTree::from_scene_graph_with_text_engine(&self.scene_graph, &text_engine);
        self.text_engine = text_engine;
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
        let update_start = Instant::now();
        let dispatch_start = Instant::now();
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
            let scene_build_start = Instant::now();
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
            let rtree_start = Instant::now();
            self.rtree = SceneGraphRTree::from_scene_graph_with_text_engine(
                &self.scene_graph,
                &self.text_engine,
            );
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
    pub fn scene_graph_arc(&self) -> Arc<SceneGraph> {
        self.scene_graph.clone()
    }

    pub async fn rebuild_scene_graph(
        &mut self,
        rebuild_geometry: bool,
    ) -> Result<Arc<SceneGraph>, AvengerAppError> {
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
        if rebuild_geometry {
            self.rtree = SceneGraphRTree::from_scene_graph_with_text_engine(
                &self.scene_graph,
                &self.text_engine,
            );
        }
        Ok(self.scene_graph.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_eventstream::{
        runtime::{RuntimeHostCommand, RuntimeWakeKey},
        scene::{SceneGraphEvent, SceneGraphEventType},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Builder(Arc<AtomicUsize>);
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    impl SceneGraphBuilder<()> for Builder {
        async fn build(&self, _: &mut ()) -> Result<SceneGraph, AvengerAppError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(SceneGraph {
                width: 1.0,
                height: 1.0,
                origin: [0.0; 2],
                marks: Vec::new(),
            })
        }
    }

    struct Schedule;
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    impl EventStreamHandler<()> for Schedule {
        async fn handle(
            &self,
            _: &SceneGraphEvent,
            _: &mut (),
            _: &SceneGraphRTree,
        ) -> UpdateStatus {
            UpdateStatus {
                commands: vec![RuntimeHostCommand::RequestWakeup {
                    key: RuntimeWakeKey::new("chart", 0, "refresh"),
                    deadline: Instant::now(),
                    generation: 1,
                }],
                ..Default::default()
            }
        }
    }

    #[test]
    fn commands_return_without_rebuilding_the_scene() {
        futures::executor::block_on(async {
            let builds = Arc::new(AtomicUsize::new(0));
            let mut app = AvengerApp::try_new(
                (),
                Arc::new(Builder(builds.clone())),
                vec![(
                    EventStreamConfig {
                        types: vec![SceneGraphEventType::WindowFocused],
                        ..Default::default()
                    },
                    Arc::new(Schedule),
                )],
            )
            .await
            .unwrap();
            let update = app
                .update_with_status(&WindowEvent::WindowFocused(true), Instant::now())
                .await
                .unwrap();
            assert!(update.scene_graph.is_none());
            assert_eq!(builds.load(Ordering::Relaxed), 1);
            assert!(matches!(
                update.status.commands.as_slice(),
                [RuntimeHostCommand::RequestWakeup { generation: 1, .. }]
            ));
        });
    }
}
