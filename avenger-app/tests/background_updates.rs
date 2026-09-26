use async_trait::async_trait;
use avenger_app::{
    app::{AvengerApp, SceneGraphBuilder},
    background::{
        host::{Attachment, Executor},
        BackgroundTask, BackgroundTasks,
    },
    error::AvengerAppError,
};
use avenger_common::time::Instant;
use avenger_eventstream::{
    manager::EventStreamHandler,
    runtime::RuntimeWakeEvent,
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{EventStreamConfig, UpdateStatus},
    window::WindowEvent,
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;
use futures::{future::BoxFuture, FutureExt};
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct ManualExecutor {
    jobs: Mutex<Vec<BoxFuture<'static, ()>>>,
    wakes: Mutex<Vec<RuntimeWakeEvent>>,
}

impl Executor for ManualExecutor {
    fn spawn(&self, future: BoxFuture<'static, ()>) {
        self.jobs.lock().unwrap().push(future);
    }
    fn wake(&self, event: RuntimeWakeEvent) {
        self.wakes.lock().unwrap().push(event);
    }
}

impl ManualExecutor {
    fn finish_query(&self) -> WindowEvent {
        let jobs = std::mem::take(&mut *self.jobs.lock().unwrap());
        for job in jobs {
            assert!(job.now_or_never().is_some());
        }
        WindowEvent::RuntimeWake(self.wakes.lock().unwrap().pop().unwrap())
    }
}

#[derive(Clone)]
struct State {
    query: BackgroundTask<u32>,
    value: u32,
    fail: bool,
}

struct Builder;
#[async_trait]
impl SceneGraphBuilder<State> for Builder {
    async fn build(&self, state: &mut State) -> Result<SceneGraph, AvengerAppError> {
        if state.fail {
            return Err(AvengerAppError::InternalError(
                "intentional build failure".into(),
            ));
        }
        Ok(SceneGraph {
            marks: vec![],
            width: state.value as f32,
            height: 10.,
            origin: [0.; 2],
        })
    }
}

struct Input;
#[async_trait]
impl EventStreamHandler<State> for Input {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut State,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        let rerender = match event {
            SceneGraphEvent::RuntimeWake(wake) => {
                if let Some(result) = state.query.handle_wake(wake) {
                    state.value = *result.unwrap();
                    true
                } else {
                    false
                }
            }
            SceneGraphEvent::WindowFocused(true) => {
                state
                    .query
                    .submit(async { Ok::<_, Infallible>(42) })
                    .unwrap();
                state.fail = true;
                true
            }
            _ => false,
        };
        UpdateStatus {
            rerender,
            ..Default::default()
        }
    }
}

async fn app() -> (AvengerApp<State>, Arc<ManualExecutor>, Attachment) {
    let tasks = BackgroundTasks::new();
    let executor = Arc::new(ManualExecutor::default());
    let attachment = Attachment::new(&tasks, executor.clone()).unwrap();
    attachment.activate();
    let app = AvengerApp::try_new(
        State {
            query: tasks.task(),
            value: 1,
            fail: false,
        },
        Arc::new(Builder),
        vec![(
            EventStreamConfig {
                types: vec![
                    SceneGraphEventType::RuntimeWake,
                    SceneGraphEventType::WindowFocused,
                ],
                ..Default::default()
            },
            Arc::new(Input),
        )],
    )
    .await
    .unwrap()
    .with_background_tasks(tasks);
    (app, executor, attachment)
}

#[test]
fn failed_rebuild_keeps_completion_available_for_a_retried_wake() {
    futures::executor::block_on(async {
        let (mut app, executor, _attachment) = app().await;
        app.app_state_mut()
            .query
            .submit(async { Ok::<_, Infallible>(7) })
            .unwrap();
        let wake = executor.finish_query();
        let previous_scene = app.scene_graph_arc();
        app.app_state_mut().fail = true;
        assert!(app.update_with_status(&wake, Instant::now()).await.is_err());
        assert_eq!(app.app_state_mut().value, 1);
        assert!(app.app_state_mut().query.is_pending());
        assert!(Arc::ptr_eq(&previous_scene, &app.scene_graph_arc()));

        app.app_state_mut().fail = false;
        let update = app.update_with_status(&wake, Instant::now()).await.unwrap();
        assert_eq!(update.scene_graph.unwrap().width, 7.);
        assert_eq!(app.app_state_mut().value, 7);
        assert!(!app.app_state_mut().query.is_pending());
        let duplicate = app.update_with_status(&wake, Instant::now()).await.unwrap();
        assert!(duplicate.scene_graph.is_none());
        assert!(!duplicate.status.rerender);
    });
}

#[test]
fn rejected_state_cannot_deliver_its_submitted_request_into_installed_state() {
    futures::executor::block_on(async {
        let (mut app, executor, _attachment) = app().await;
        assert!(app
            .update_with_status(&WindowEvent::WindowFocused(true), Instant::now())
            .await
            .is_err());
        assert!(!app.app_state_mut().fail);
        assert!(!app.app_state_mut().query.is_pending());
        let wake = executor.finish_query();
        let update = app.update_with_status(&wake, Instant::now()).await.unwrap();
        assert!(update.scene_graph.is_none());
        assert_eq!(app.app_state_mut().value, 1);

        app.app_state_mut()
            .query
            .submit(async { Ok::<_, Infallible>(8) })
            .unwrap();
        let wake = executor.finish_query();
        app.update_with_status(&wake, Instant::now()).await.unwrap();
        assert_eq!(app.app_state_mut().value, 8);
    });
}
