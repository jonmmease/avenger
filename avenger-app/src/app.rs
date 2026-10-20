use avenger_common::time::Instant;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use avenger_eventstream::manager::{EventStreamHandler, EventStreamManager};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::WindowEvent;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_image::{ImageCache, ImageResolver};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

use crate::alarm::Alarm;
use crate::error::AvengerAppError;
use crate::waker::{AppWaker, Refresh};

/// What the host does after [`AvengerApp::refresh`].
#[derive(Debug, Clone)]
pub enum Refreshed {
    /// Nothing was pending.
    Nothing,
    /// Draw the scene again.
    Redraw,
    /// Install the new scene and draw it.
    Scene(Arc<SceneGraph>),
}

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
    text_engine: LabelEngine,
    waker: AppWaker,
    /// Wakes the host when the next wake that the event streams wait for is due.
    alarm: Alarm,
    images: Option<Arc<dyn ImageResolver>>,
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
        text_engine: LabelEngine,
    ) -> Result<Self, AvengerAppError> {
        let mut event_stream_manager = EventStreamManager::new(initial_state);
        for (config, handler) in stream_callbacks {
            event_stream_manager.register_handler(config, handler);
        }
        // Build initial scene graph and rtree
        let scene_graph = scene_graph_builder
            .build(event_stream_manager.state_mut())
            .await?;
        let rtree = SceneGraphRTree::from_scene_graph(&scene_graph, &text_engine);
        let waker = AppWaker::new();

        Ok(Self {
            scene_graph_builder,
            event_stream_manager,
            rtree,
            scene_graph: Arc::new(scene_graph),
            text_engine,
            alarm: Alarm::new(waker.clone()),
            waker,
            images: None,
        })
    }

    /// Draw the scenes' image resources through `images`. Hosts give the cache to their
    /// canvases, and each image that arrives or fails requests a redraw on the app's waker.
    pub fn with_images(mut self, images: ImageCache) -> Self {
        let waker = self.waker.clone();
        images.set_on_change(move || waker.request(Refresh::Redraw));
        self.images = Some(Arc::new(images));
        self
    }

    /// The resolver of the scenes' image resources, for the host's canvas.
    pub fn images(&self) -> Option<Arc<dyn ImageResolver>> {
        self.images.clone()
    }

    /// The waker through which background work asks the host to refresh, for the app's state
    /// to hold.
    pub fn waker(&self) -> AppWaker {
        self.waker.clone()
    }

    /// Take the waker's pending refresh, run the wakes that are due, and do what the refresh and
    /// the wakes' handlers ask, as [`AvengerApp::update`] does for an event's handlers. A rebuild
    /// builds the scene and its hit-testing tree again. Returns what the host must do.
    pub async fn refresh(&mut self) -> Result<Refreshed, AvengerAppError> {
        let pending = self.waker.take();
        let status = self
            .event_stream_manager
            .dispatch_due_wakes(&self.rtree, Instant::now())
            .await;
        self.set_alarm();
        let rebuild = pending == Some(Refresh::Rebuild);
        if rebuild || status.rerender || status.rebuild_geometry {
            self.build_scene().await?;
            if rebuild || status.rebuild_geometry {
                self.rtree =
                    SceneGraphRTree::from_scene_graph(&self.scene_graph, &self.text_engine);
            }
            return Ok(Refreshed::Scene(self.scene_graph.clone()));
        }
        Ok(match pending {
            Some(Refresh::Redraw) => Refreshed::Redraw,
            _ => Refreshed::Nothing,
        })
    }

    /// Set the alarm for the next wake that the event streams wait for.
    fn set_alarm(&self) {
        if let Some(at) = self.event_stream_manager.next_wake() {
            self.alarm.set(at);
        }
    }

    /// Build the scene from the state. The pending refresh clears first, since the new scene
    /// shows the state as it is, and a request during the build raises it again.
    async fn build_scene(&mut self) -> Result<(), AvengerAppError> {
        self.waker.take();
        match self
            .scene_graph_builder
            .build(self.event_stream_manager.state_mut())
            .await
        {
            Ok(scene_graph) => {
                self.scene_graph = Arc::new(scene_graph);
                Ok(())
            }
            Err(e) => {
                eprintln!("Failed to build scene graph: {e:?}");
                Err(AvengerAppError::InternalError(
                    "Failed to build scene graph".to_string(),
                ))
            }
        }
    }

    pub fn get_watched_files(&self) -> Vec<PathBuf> {
        self.event_stream_manager.get_watched_files()
    }

    /// Update the state of the app without rebuilding the scene graph
    pub async fn update_state(&mut self, event: &WindowEvent, instant: Instant) -> UpdateStatus {
        let update_status = self
            .event_stream_manager
            .dispatch_event(event, &self.rtree, instant)
            .await;
        self.set_alarm();
        update_status
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
        self.set_alarm();

        // Reconstruct the scene graph if the need to rerender or rebuild geometry
        if update_status.rerender || update_status.rebuild_geometry {
            self.build_scene().await?;
        }

        // Rebuild the rtree if the need to rebuild geometry
        if update_status.rebuild_geometry {
            self.rtree = SceneGraphRTree::from_scene_graph(&self.scene_graph, &self.text_engine);
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

    /// The engine that measures the app's text, to draw its scenes with.
    pub fn text_engine(&self) -> &LabelEngine {
        &self.text_engine
    }
}
