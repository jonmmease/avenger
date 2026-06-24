mod canvas;
mod event;
mod gpu;
mod metrics;

pub use canvas::{AvengerCanvasHandle, Canvas, CanvasOutput};
pub use event::{EguiEventTranslator, EguiResponseState, local_position};
pub use gpu::{FrameStatus, TextureRenderMode, TextureRenderStatus};
pub use metrics::{CanvasLatencyBottleneck, CanvasMetrics};

pub use egui;

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex as StdMutex},
        thread,
        time::{Duration, Instant as StdInstant},
    };

    use async_trait::async_trait;
    use avenger_app::{
        app::{AvengerApp, SceneGraphBuilder},
        error::AvengerAppError,
    };
    use avenger_common::canvas::CanvasDimensions;
    use avenger_eventstream::{
        manager::EventStreamHandler,
        scene::{SceneGraphEvent, SceneGraphEventType},
        stream::{EventStreamConfig, UpdateStatus},
        window::{
            CanvasResizeEvent, ElementState, Key, MouseButton, MouseScrollDelta, NamedKey,
            WindowCursorMoved, WindowEvent, WindowMouseInput, WindowMouseWheel,
        },
    };
    use avenger_resource::{
        RenderInvalidationReason, RenderInvalidationRequest, RenderInvalidationSchedule,
        RenderInvalidationSink,
    };
    use avenger_scenegraph::scene_graph::SceneGraph;
    use avenger_wgpu::{canvas::CanvasConfig, frame_publisher::FrameGeneration};
    use egui_wgpu::wgpu;

    use super::*;
    use crate::{
        event::{egui_key_to_avenger, egui_wheel_to_avenger},
        gpu::*,
    };

    #[derive(Clone)]
    struct TestState {
        width: Arc<StdMutex<f32>>,
    }

    impl TestState {
        fn new(width: f32) -> Self {
            Self {
                width: Arc::new(StdMutex::new(width)),
            }
        }

        fn width(&self) -> f32 {
            *self.width.lock().expect("test state width lock poisoned")
        }

        fn set_width(&self, width: f32) {
            *self.width.lock().expect("test state width lock poisoned") = width;
        }
    }

    struct TestSceneBuilder;

    #[async_trait]
    impl SceneGraphBuilder<TestState> for TestSceneBuilder {
        async fn build(&self, state: &mut TestState) -> Result<SceneGraph, AvengerAppError> {
            Ok(SceneGraph {
                marks: Vec::new(),
                width: state.width(),
                height: 1.0,
                origin: [0.0, 0.0],
            })
        }
    }

    struct ResizeHandler;

    #[async_trait]
    impl EventStreamHandler<TestState> for ResizeHandler {
        async fn handle(
            &self,
            event: &SceneGraphEvent,
            state: &mut TestState,
            _rtree: &avenger_geometry::rtree::SceneGraphRTree,
        ) -> UpdateStatus {
            if let SceneGraphEvent::CanvasResize(event) = event {
                state.set_width(event.size[0]);
                UpdateStatus {
                    rerender: true,
                    rebuild_geometry: true,
                    ..UpdateStatus::default()
                }
            } else {
                UpdateStatus::default()
            }
        }
    }

    async fn test_handle() -> AvengerCanvasHandle<TestState> {
        AvengerCanvasHandle::new(TestState::new(640.0))
    }

    async fn test_app_handle() -> AvengerCanvasHandle<TestState> {
        let app = AvengerApp::try_new(
            TestState::new(640.0),
            Arc::new(TestSceneBuilder),
            vec![(
                EventStreamConfig {
                    types: vec![SceneGraphEventType::CanvasResize],
                    ..EventStreamConfig::default()
                },
                Arc::new(ResizeHandler),
            )],
        )
        .await
        .expect("build egui test app");
        AvengerCanvasHandle::from_app(app)
    }

    async fn wait_for_scene_generation(
        handle: &AvengerCanvasHandle<TestState>,
        generation: FrameGeneration,
    ) {
        for _ in 0..1_000 {
            if handle
                .latest_scene_frame()
                .is_some_and(|frame| frame.generation == generation)
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!(
            "scene generation {} was not published; status={:?}, error={:?}",
            generation.get(),
            handle.frame_status(),
            handle.latest_scene_error()
        );
    }

    fn raw_input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        }
    }

    fn plot_events_on_ctx(
        ctx: &egui::Context,
        handle: &AvengerCanvasHandle<TestState>,
        raw_input: egui::RawInput,
    ) -> Vec<WindowEvent> {
        let mut output_events = Vec::new();
        let _ = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| {
                    output_events = Canvas::new(handle)
                        .desired_size(egui::vec2(320.0, 240.0))
                        .show(ui)
                        .events;
                })
                .inner;
        });
        output_events
    }

    fn pointer_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    fn key_event(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }
    }

    fn contains_mouse_wheel(events: &[WindowEvent]) -> bool {
        events
            .iter()
            .any(|event| matches!(event, WindowEvent::MouseWheel(_)))
    }

    fn contains_keyboard_input(events: &[WindowEvent]) -> bool {
        events
            .iter()
            .any(|event| matches!(event, WindowEvent::KeyboardInput(_)))
    }

    fn cursor_moved_count(events: &[WindowEvent]) -> usize {
        events
            .iter()
            .filter(|event| matches!(event, WindowEvent::CursorMoved(_)))
            .count()
    }

    fn response_for_size(size: egui::Vec2) -> egui::Response {
        let ctx = egui::Context::default();
        let mut response = None;
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| {
                    response = Some(ui.allocate_response(size, egui::Sense::click_and_drag()));
                })
                .inner;
        });
        response.expect("allocated response")
    }

    #[test]
    fn local_position_subtracts_widget_origin() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, 200.0));

        assert_eq!(local_position(rect, egui::pos2(35.0, 70.0)), [25.0, 50.0]);
    }

    #[test]
    fn wheel_mapping_preserves_point_and_line_units() {
        assert_eq!(
            egui_wheel_to_avenger(egui::MouseWheelUnit::Point, egui::vec2(4.0, -8.0)),
            MouseScrollDelta::PixelDelta(4.0, -8.0)
        );
        assert_eq!(
            egui_wheel_to_avenger(egui::MouseWheelUnit::Line, egui::vec2(1.0, -2.0)),
            MouseScrollDelta::LineDelta(1.0, -2.0)
        );
    }

    #[test]
    fn key_mapping_covers_named_and_character_keys() {
        assert_eq!(
            egui_key_to_avenger(egui::Key::ArrowLeft),
            Some(Key::Named(NamedKey::ArrowLeft))
        );
        assert_eq!(egui_key_to_avenger(egui::Key::A), Some(Key::Character('a')));
        assert_eq!(
            egui_key_to_avenger(egui::Key::Num7),
            Some(Key::Character('7'))
        );
    }

    #[test]
    fn render_request_key_tracks_scene_dimensions_and_format() {
        let dimensions = CanvasDimensions {
            size: [640.0, 480.0],
            scale: 2.0,
        };
        let key = RenderRequestKey::new(7, 0, dimensions, wgpu::TextureFormat::Rgba8Unorm);

        assert_eq!(
            key,
            RenderRequestKey::new(7, 0, dimensions, wgpu::TextureFormat::Rgba8Unorm)
        );
        assert_ne!(
            key,
            RenderRequestKey::new(8, 0, dimensions, wgpu::TextureFormat::Rgba8Unorm)
        );
        assert_ne!(
            key,
            RenderRequestKey::new(7, 1, dimensions, wgpu::TextureFormat::Rgba8Unorm)
        );
        assert_ne!(
            key,
            RenderRequestKey::new(
                7,
                0,
                CanvasDimensions {
                    size: [641.0, 480.0],
                    scale: 2.0,
                },
                wgpu::TextureFormat::Rgba8Unorm,
            )
        );
        assert_ne!(
            key,
            RenderRequestKey::new(7, 0, dimensions, wgpu::TextureFormat::Bgra8Unorm)
        );
    }

    #[tokio::test]
    async fn render_invalidation_subscription_updates_epoch_without_events() {
        let handle = test_handle().await;
        let hub = avenger_resource::RenderInvalidationHub::default();
        handle.set_render_invalidation_hub(Some(hub.clone()));
        let ctx = egui::Context::default();

        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| {
                    Canvas::new(&handle)
                        .desired_size(egui::vec2(320.0, 240.0))
                        .show(ui);
                })
                .inner;
        });
        handle.take_pending_events();

        hub.request_render(RenderInvalidationRequest {
            reason: RenderInvalidationReason::ResourceChanged { kind: "image" },
            schedule: RenderInvalidationSchedule::After(Duration::from_millis(1)),
        });

        assert_eq!(handle.render_invalidation_epoch(), 1);
        assert_eq!(handle.pending_event_count(), 0);
        assert_eq!(handle.frame_status().requested_generation, None);
        let metrics = handle.metrics();
        assert_eq!(metrics.render_invalidation_events_received, 1);
        assert_eq!(metrics.latest_render_invalidation_epoch, 1);
    }

    #[tokio::test]
    async fn render_invalidation_hub_set_observes_existing_epoch() {
        let handle = test_handle().await;
        let hub = avenger_resource::RenderInvalidationHub::default();
        hub.request_render(RenderInvalidationRequest::now(
            RenderInvalidationReason::ResourceChanged { kind: "image" },
        ));

        handle.set_render_invalidation_hub(Some(hub));

        assert_eq!(handle.render_invalidation_epoch(), 1);
        assert_eq!(handle.pending_event_count(), 0);
        assert_eq!(handle.frame_status().requested_generation, None);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn empty_scene_graph() -> Arc<SceneGraph> {
        Arc::new(SceneGraph {
            marks: Vec::new(),
            width: 1.0,
            height: 1.0,
            origin: [0.0, 0.0],
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn test_dimensions(width: f32, height: f32) -> CanvasDimensions {
        CanvasDimensions {
            size: [width, height],
            scale: 2.0,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn assert_dimensions_eq(left: CanvasDimensions, right: CanvasDimensions) {
        assert_eq!(left.size, right.size);
        assert_eq!(left.scale, right.scale);
    }

    #[cfg(not(target_arch = "wasm32"))]
    async fn make_test_wgpu_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let primary_options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        };
        let fallback_options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: true,
        };
        let adapter = match instance.request_adapter(&primary_options).await {
            Ok(adapter) => adapter,
            Err(_) => match instance.request_adapter(&fallback_options).await {
                Ok(adapter) => adapter,
                Err(err) => {
                    eprintln!("skipping native WGPU test: no adapter available: {err}");
                    return None;
                }
            },
        };

        match adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("avenger-egui background render test device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
        {
            Ok(handles) => Some(handles),
            Err(err) => {
                eprintln!("skipping native WGPU test: device request failed: {err}");
                None
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn wait_for_rendered_scene(
        controller: &BackgroundRenderController,
        scene_generation: u64,
    ) -> Arc<RenderedCanvasTexture> {
        let deadline = StdInstant::now() + Duration::from_secs(5);
        let mut state = controller
            .shared
            .state
            .lock()
            .expect("avenger egui render-worker lock poisoned");

        loop {
            if let Some(error) = &state.last_error {
                panic!("background render worker failed: {error}");
            }
            if let Some(rendered) = &state.latest_rendered
                && rendered.scene_generation == scene_generation
            {
                return rendered.clone();
            }

            let now = StdInstant::now();
            assert!(
                now < deadline,
                "timed out waiting for rendered scene generation {scene_generation}"
            );
            let (next_state, _) = controller
                .shared
                .notify
                .wait_timeout(state, deadline - now)
                .expect("avenger egui render-worker lock poisoned");
            state = next_state;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn mark_rendered_scene_consumed(
        controller: &BackgroundRenderController,
        rendered: &RenderedCanvasTexture,
    ) {
        let mut state = controller
            .shared
            .state
            .lock()
            .expect("avenger egui render-worker lock poisoned");
        state.mark_render_consumed(rendered.render_generation, rendered.target_generation);
        drop(state);
        controller.shared.notify.notify_one();
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn wait_for_in_progress_generation(
        controller: &BackgroundRenderController,
        render_generation: u64,
    ) {
        let deadline = StdInstant::now() + Duration::from_secs(5);
        loop {
            let state = controller
                .shared
                .state
                .lock()
                .expect("avenger egui render-worker lock poisoned");
            if let Some(error) = &state.last_error {
                panic!("background render worker failed: {error}");
            }
            if state.in_progress_generation == Some(render_generation) {
                return;
            }
            drop(state);

            assert!(
                StdInstant::now() < deadline,
                "timed out waiting for in-progress render generation {render_generation}"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_render_state_coalesces_latest_request() {
        let mut state = BackgroundRenderState::default();
        let dimensions = test_dimensions(640.0, 480.0);

        let first = state.enqueue_request(
            1,
            empty_scene_graph(),
            dimensions,
            0,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );

        assert_eq!(first.render_generation, Some(1));
        assert!(!first.coalesced_previous);
        assert_eq!(state.requested_scene_generation, Some(1));

        let duplicate = state.enqueue_request(
            1,
            empty_scene_graph(),
            dimensions,
            0,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );

        assert_eq!(duplicate, BackgroundEnqueueResult::default());
        assert_eq!(state.next_render_generation, 1);

        let second = state.enqueue_request(
            2,
            empty_scene_graph(),
            dimensions,
            0,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );

        assert_eq!(second.render_generation, Some(2));
        assert!(second.coalesced_previous);
        let pending = state.pending_request.as_ref().expect("pending request");
        assert_eq!(pending.render_generation, 2);
        assert_eq!(pending.scene_generation, 2);
        assert_eq!(state.requested_scene_generation, Some(2));
        assert!(state.is_render_generation_stale(1));
        assert!(!state.is_render_generation_stale(2));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_render_state_treats_render_invalidation_epoch_as_request_key() {
        let mut state = BackgroundRenderState::default();
        let dimensions = test_dimensions(640.0, 480.0);

        let first = state.enqueue_request(
            1,
            empty_scene_graph(),
            dimensions,
            0,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );
        let duplicate = state.enqueue_request(
            1,
            empty_scene_graph(),
            dimensions,
            0,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );
        let invalidated = state.enqueue_request(
            1,
            empty_scene_graph(),
            dimensions,
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );

        assert_eq!(first.render_generation, Some(1));
        assert_eq!(duplicate, BackgroundEnqueueResult::default());
        assert_eq!(invalidated.render_generation, Some(2));
        assert!(invalidated.coalesced_previous);
        assert_eq!(state.requested_scene_generation, Some(1));
        assert_eq!(state.requested_render_invalidation_epoch, Some(1));
        let pending = state.pending_request.as_ref().expect("pending request");
        assert_eq!(pending.scene_generation, 1);
        assert_eq!(pending.render_invalidation_epoch, 1);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_render_state_marks_in_progress_generation_stale_after_newer_request() {
        let mut state = BackgroundRenderState::default();
        let dimensions = test_dimensions(640.0, 480.0);
        state.enqueue_request(
            1,
            empty_scene_graph(),
            dimensions,
            0,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );
        let first = state
            .take_next_request_if_ready()
            .expect("first request should start");

        assert_eq!(first.render_generation, 1);
        assert_eq!(state.in_progress_generation, Some(1));
        assert!(state.is_pending());

        state.enqueue_request(
            2,
            empty_scene_graph(),
            dimensions,
            0,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );

        assert!(state.is_render_generation_stale(first.render_generation));
        state.finish_render_generation(first.render_generation);
        assert_eq!(state.in_progress_generation, None);

        let second = state
            .take_next_request_if_ready()
            .expect("newest request should start after stale finish");
        assert_eq!(second.render_generation, 2);
        assert_eq!(second.scene_generation, 2);
        assert!(!state.is_render_generation_stale(second.render_generation));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_render_state_replaces_pending_request_on_resize_key_change() {
        let mut state = BackgroundRenderState::default();
        let initial = test_dimensions(640.0, 480.0);
        let resized = test_dimensions(800.0, 480.0);

        state.enqueue_request(
            1,
            empty_scene_graph(),
            initial,
            0,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );
        let resized_result = state.enqueue_request(
            1,
            empty_scene_graph(),
            resized,
            0,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );

        assert_eq!(resized_result.render_generation, Some(2));
        assert!(resized_result.coalesced_previous);
        let pending = state.pending_request.as_ref().expect("pending request");
        assert_eq!(pending.render_generation, 2);
        assert_dimensions_eq(pending.dimensions, resized);

        let duplicate_resize = state.enqueue_request(
            1,
            empty_scene_graph(),
            resized,
            0,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );
        assert_eq!(duplicate_resize, BackgroundEnqueueResult::default());
        assert_eq!(state.next_render_generation, 2);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_render_state_tracks_front_target_after_consumption() {
        let mut state = BackgroundRenderState::default();

        state.mark_render_consumed(4, 12);

        assert_eq!(state.consumed_render_generation, Some(4));
        assert_eq!(state.front_target_generation, Some(12));
        assert!(!state.is_pending());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_render_worker_hooks_parse_env_values() {
        let hooks =
            BackgroundRenderWorkerHooks::from_env_values(Some("10"), Some(" 20 "), Some("invalid"));
        assert_eq!(hooks.before_set_scene_delay, Duration::from_millis(10));
        assert_eq!(hooks.after_set_scene_delay, Duration::from_millis(20));
        assert_eq!(hooks.after_encode_delay, Duration::ZERO);

        assert_eq!(
            BackgroundRenderWorkerHooks::from_env_values(None, Some("-1"), Some("")),
            BackgroundRenderWorkerHooks::default()
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn background_render_worker_avoids_front_target_and_handles_resize() {
        let Some((device, queue)) = make_test_wgpu_device().await else {
            return;
        };
        let poll_device = device.clone();
        let metrics = Arc::new(StdMutex::new(CanvasMetrics::default()));
        let controller = BackgroundRenderController::new(
            device,
            queue,
            wgpu::TextureFormat::Rgba8Unorm,
            CanvasConfig::default(),
            metrics.clone(),
        );

        let initial = test_dimensions(128.0, 96.0);
        controller.enqueue(
            1,
            empty_scene_graph(),
            initial,
            0,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            metrics.as_ref(),
        );
        let first = wait_for_rendered_scene(&controller, 1);
        assert_dimensions_eq(first.dimensions, initial);
        assert!(first.target_generation <= 3);
        mark_rendered_scene_consumed(&controller, &first);

        controller.enqueue(
            2,
            empty_scene_graph(),
            initial,
            0,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            metrics.as_ref(),
        );
        let second = wait_for_rendered_scene(&controller, 2);
        assert_dimensions_eq(second.dimensions, initial);
        assert!(second.target_generation <= 3);
        assert_ne!(
            second.target_generation, first.target_generation,
            "worker must not render into the target generation currently marked as front"
        );
        mark_rendered_scene_consumed(&controller, &second);

        let resized = test_dimensions(192.0, 96.0);
        controller.enqueue(
            3,
            empty_scene_graph(),
            resized,
            0,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            metrics.as_ref(),
        );
        let third = wait_for_rendered_scene(&controller, 3);
        assert_dimensions_eq(third.dimensions, resized);
        assert!(
            third.target_generation > 3,
            "resized render must publish a recreated target generation"
        );

        poll_device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll after background render");
        let metrics = metrics
            .lock()
            .expect("avenger egui metrics lock poisoned")
            .clone();
        assert_eq!(metrics.background_render_requests, 3);
        assert_eq!(metrics.background_render_frames_submitted, 3);
        assert_eq!(metrics.background_render_frames_published, 3);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn background_render_worker_enqueue_and_status_remain_fast_when_render_is_slow() {
        let Some((device, queue)) = make_test_wgpu_device().await else {
            return;
        };
        let metrics = Arc::new(StdMutex::new(CanvasMetrics::default()));
        let controller = BackgroundRenderController::new_with_test_hooks(
            device,
            queue,
            wgpu::TextureFormat::Rgba8Unorm,
            CanvasConfig::default(),
            metrics.clone(),
            BackgroundRenderWorkerHooks {
                before_set_scene_delay: Duration::from_millis(200),
                after_set_scene_delay: Duration::from_millis(200),
                after_encode_delay: Duration::from_millis(200),
            },
        );

        let dimensions = test_dimensions(128.0, 96.0);
        controller.enqueue(
            1,
            empty_scene_graph(),
            dimensions,
            0,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            metrics.as_ref(),
        );
        wait_for_in_progress_generation(&controller, 1);

        let enqueue_start = StdInstant::now();
        controller.enqueue(
            2,
            empty_scene_graph(),
            dimensions,
            0,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            metrics.as_ref(),
        );
        assert!(
            enqueue_start.elapsed() < Duration::from_millis(100),
            "enqueue should not wait for the slow background render"
        );

        let status_start = StdInstant::now();
        let status = controller.status();
        assert!(
            status_start.elapsed() < Duration::from_millis(100),
            "status should not wait for the slow background render"
        );
        assert!(status.render_pending);
        assert_eq!(status.requested_scene_generation, Some(2));

        let rendered = wait_for_rendered_scene(&controller, 2);
        assert_eq!(rendered.scene_generation, 2);

        let metrics = metrics
            .lock()
            .expect("avenger egui metrics lock poisoned")
            .clone();
        assert_eq!(metrics.background_render_requests, 2);
        assert_eq!(metrics.background_render_frames_published, 1);
        assert!(
            metrics.stale_background_render_frames_dropped >= 1,
            "first slow render should be dropped after a newer request arrives"
        );
    }

    #[tokio::test]
    async fn handle_queues_and_takes_pending_events() {
        let handle = test_handle().await;
        let event = WindowEvent::CanvasResize(CanvasResizeEvent {
            size: [300.0, 200.0],
        });

        handle.queue_events([event.clone()]);

        assert_eq!(handle.pending_event_count(), 1);
        assert_eq!(handle.take_pending_events(), vec![event]);
        assert_eq!(handle.pending_event_count(), 0);
        let metrics = handle.metrics();
        assert_eq!(metrics.routed_event_batches, 1);
        assert_eq!(metrics.routed_events, 1);
        assert_eq!(metrics.last_routed_event_count, 1);
    }

    #[tokio::test]
    async fn frame_event_queue_coalesces_skippable_events_while_render_pending() {
        let handle = test_handle().await;

        let queued = handle.queue_frame_events(
            [
                WindowEvent::CursorMoved(WindowCursorMoved {
                    position: [10.0, 20.0],
                }),
                WindowEvent::MouseWheel(WindowMouseWheel {
                    delta: MouseScrollDelta::PixelDelta(0.0, 8.0),
                }),
                WindowEvent::CursorMoved(WindowCursorMoved {
                    position: [30.0, 40.0],
                }),
                WindowEvent::MouseWheel(WindowMouseWheel {
                    delta: MouseScrollDelta::PixelDelta(0.0, 12.0),
                }),
            ],
            true,
        );

        assert!(queued.is_empty());
        assert_eq!(handle.pending_event_count(), 0);
        assert_eq!(handle.coalesced_render_pending_event_count(), 2);
        let metrics = handle.metrics();
        assert_eq!(metrics.render_pending_events_coalesced, 4);
        assert_eq!(metrics.render_pending_events_replayed, 0);
        assert_eq!(metrics.routed_events, 0);

        let replayed = handle.queue_frame_events([], false);

        assert_eq!(
            replayed,
            vec![
                WindowEvent::CursorMoved(WindowCursorMoved {
                    position: [30.0, 40.0],
                }),
                WindowEvent::MouseWheel(WindowMouseWheel {
                    delta: MouseScrollDelta::PixelDelta(0.0, 20.0),
                }),
            ]
        );
        assert_eq!(handle.pending_event_count(), 2);
        assert_eq!(handle.coalesced_render_pending_event_count(), 0);
        let metrics = handle.metrics();
        assert_eq!(metrics.render_pending_events_coalesced, 4);
        assert_eq!(metrics.render_pending_events_replayed, 2);
        assert_eq!(metrics.routed_events, 2);
    }

    #[tokio::test]
    async fn frame_event_queue_replays_coalesced_events_before_immediate_event() {
        let handle = test_handle().await;

        let queued = handle.queue_frame_events(
            [
                WindowEvent::CursorMoved(WindowCursorMoved {
                    position: [30.0, 40.0],
                }),
                WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                }),
            ],
            true,
        );

        assert_eq!(
            queued,
            vec![
                WindowEvent::CursorMoved(WindowCursorMoved {
                    position: [30.0, 40.0],
                }),
                WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                }),
            ]
        );
        assert_eq!(handle.pending_event_count(), 2);
        assert_eq!(handle.coalesced_render_pending_event_count(), 0);
        let metrics = handle.metrics();
        assert_eq!(metrics.render_pending_events_coalesced, 1);
        assert_eq!(metrics.render_pending_events_replayed, 1);
        assert_eq!(metrics.routed_events, 2);
    }

    #[tokio::test]
    async fn frame_status_counts_queued_events_as_render_pending() {
        let handle = test_handle().await;

        assert!(!handle.frame_status().render_pending);
        handle.queue_events([WindowEvent::CanvasResize(CanvasResizeEvent {
            size: [300.0, 200.0],
        })]);

        assert!(handle.frame_status().render_pending);
    }

    #[tokio::test]
    async fn reset_metrics_clears_canvas_counters() {
        let handle = test_handle().await;

        handle.queue_events([WindowEvent::CanvasResize(CanvasResizeEvent {
            size: [300.0, 200.0],
        })]);
        let metrics = handle.metrics();
        assert_eq!(metrics.routed_event_batches, 1);
        assert_eq!(metrics.routed_events, 1);

        handle.reset_metrics();
        assert_eq!(handle.metrics(), CanvasMetrics::default());
    }

    #[test]
    fn plot_metrics_reports_latency_bottleneck() {
        let mut metrics = CanvasMetrics::default();

        assert_eq!(
            metrics.latency_bottleneck(),
            CanvasLatencyBottleneck::NoSamples
        );
        assert_eq!(metrics.latency_bottleneck_us(), 0);

        metrics.last_scene_evaluation_us = 12_000;
        metrics.last_set_scene_us = 3_000;
        metrics.last_command_encode_us = 2_000;
        metrics.last_submit_us = 500;
        metrics.last_texture_publish_us = 20;
        assert_eq!(
            metrics.latency_bottleneck(),
            CanvasLatencyBottleneck::SceneEvaluation
        );
        assert_eq!(metrics.latency_bottleneck_us(), 12_000);

        metrics.last_command_encode_us = 20_000;
        assert_eq!(
            metrics.latency_bottleneck(),
            CanvasLatencyBottleneck::GpuRender
        );
        assert_eq!(metrics.latency_bottleneck_us(), 23_500);

        metrics.last_texture_publish_us = 30_000;
        assert_eq!(
            metrics.latency_bottleneck(),
            CanvasLatencyBottleneck::EguiTextureRegistration
        );
        assert_eq!(metrics.latency_bottleneck_us(), 30_000);

        metrics.last_background_queue_wait_us = 40_000;
        assert_eq!(
            metrics.latency_bottleneck(),
            CanvasLatencyBottleneck::BackgroundQueueWait
        );
        assert_eq!(metrics.latency_bottleneck_us(), 40_000);
    }

    #[tokio::test]
    async fn dispatch_without_owned_app_drains_events_without_updates() {
        let handle = test_handle().await;
        handle.queue_events([WindowEvent::CanvasResize(CanvasResizeEvent {
            size: [300.0, 200.0],
        })]);

        let updates = handle
            .dispatch_pending_events()
            .await
            .expect("dispatch without owned app");

        assert!(updates.is_empty());
        assert_eq!(handle.pending_event_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scene_rebuild_request_publishes_latest_scene() {
        let handle = test_app_handle().await;

        let generation = handle
            .request_scene_rebuild(&tokio::runtime::Handle::current(), true)
            .expect("request scene rebuild");

        assert_eq!(
            handle.frame_status().requested_generation,
            Some(generation.get())
        );
        wait_for_scene_generation(&handle, generation).await;

        let status = handle.frame_status();
        assert_eq!(status.latest_scene_generation, Some(generation.get()));
        assert!(!status.render_pending);
        assert!(handle.latest_scene_error().is_none());
        let metrics = handle.metrics();
        assert_eq!(metrics.scene_rebuild_requests, 1);
        assert_eq!(metrics.scene_frames_published, 1);
        assert_eq!(
            metrics.last_published_scene_generation,
            Some(generation.get())
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rapid_scene_rebuild_requests_publish_newest_generation() {
        let handle = test_app_handle().await;
        handle.app_state().set_width(700.0);
        let first = handle
            .request_scene_rebuild(&tokio::runtime::Handle::current(), true)
            .expect("request first scene rebuild");
        handle.app_state().set_width(720.0);
        let second = handle
            .request_scene_rebuild(&tokio::runtime::Handle::current(), true)
            .expect("request second scene rebuild");

        assert!(second > first);
        wait_for_scene_generation(&handle, second).await;

        let latest = handle
            .latest_scene_frame()
            .expect("latest scene after rapid rebuild requests");
        assert_eq!(latest.generation, second);
        assert_eq!(handle.app_state().width(), 720.0);
        assert_eq!(latest.payload.width, 720.0);
        let metrics = handle.metrics();
        assert_eq!(metrics.scene_rebuild_requests, 2);
        assert_eq!(metrics.last_requested_generation, Some(second.get()));
        assert_eq!(metrics.last_published_scene_generation, Some(second.get()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn event_dispatch_request_publishes_scene_for_routed_resize() {
        let handle = test_app_handle().await;
        handle.queue_events([WindowEvent::CanvasResize(CanvasResizeEvent {
            size: [720.0, 300.0],
        })]);

        let generation = handle
            .request_event_dispatch(&tokio::runtime::Handle::current())
            .expect("request event dispatch");
        wait_for_scene_generation(&handle, generation).await;

        assert_eq!(handle.pending_event_count(), 0);
        assert_eq!(handle.app_state().width(), 720.0);
        assert_eq!(
            handle
                .latest_scene_frame()
                .expect("latest scene after resize")
                .payload
                .width,
            720.0
        );
        let metrics = handle.metrics();
        assert_eq!(metrics.event_dispatch_requests, 1);
        assert_eq!(metrics.scene_frames_published, 1);
    }

    #[tokio::test]
    async fn plot_show_queues_translated_resize_events() {
        let handle = test_handle().await;
        let ctx = egui::Context::default();
        let mut output_events = Vec::new();

        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| {
                    output_events = Canvas::new(&handle)
                        .desired_size(egui::vec2(320.0, 240.0))
                        .show(ui)
                        .events;
                })
                .inner;
        });

        assert!(
            output_events
                .iter()
                .any(|event| matches!(event, WindowEvent::CanvasResize(_)))
        );
        assert_eq!(handle.pending_event_count(), output_events.len());
    }

    #[tokio::test]
    async fn wheel_events_route_only_while_plot_is_hovered() {
        let handle = test_handle().await;
        let ctx = egui::Context::default();
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 12.0),
            modifiers: egui::Modifiers::default(),
        };

        let outside = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![
                egui::Event::PointerMoved(egui::pos2(700.0, 500.0)),
                wheel.clone(),
            ]),
        );
        assert!(!contains_mouse_wheel(&outside));
        handle.take_pending_events();

        let inside = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![
                egui::Event::PointerMoved(egui::pos2(40.0, 40.0)),
                wheel,
            ]),
        );
        assert!(contains_mouse_wheel(&inside));
    }

    #[tokio::test]
    async fn pointer_capture_routes_release_after_pointer_leaves_plot() {
        let handle = test_handle().await;
        let ctx = egui::Context::default();

        let events = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![
                egui::Event::PointerMoved(egui::pos2(40.0, 40.0)),
                pointer_button(egui::pos2(40.0, 40.0), true),
                egui::Event::PointerMoved(egui::pos2(700.0, 500.0)),
                pointer_button(egui::pos2(700.0, 500.0), false),
            ]),
        );

        assert!(events.iter().any(|event| {
            matches!(
                event,
                WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                })
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event,
                WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                })
            )
        }));
    }

    #[tokio::test]
    async fn stationary_hover_does_not_emit_repeated_cursor_moved_events() {
        let handle = test_handle().await;
        let ctx = egui::Context::default();
        let pos = egui::pos2(40.0, 40.0);

        let first = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![egui::Event::PointerMoved(pos)]),
        );
        assert_eq!(cursor_moved_count(&first), 1);

        let second = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![egui::Event::PointerMoved(pos)]),
        );
        assert_eq!(cursor_moved_count(&second), 0);
    }

    #[tokio::test]
    async fn keyboard_events_route_only_after_plot_focus() {
        let handle = test_handle().await;
        let ctx = egui::Context::default();

        let unfocused = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![key_event(egui::Key::A, true)]),
        );
        assert!(!contains_keyboard_input(&unfocused));

        let _ = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![
                egui::Event::PointerMoved(egui::pos2(40.0, 40.0)),
                pointer_button(egui::pos2(40.0, 40.0), true),
                pointer_button(egui::pos2(40.0, 40.0), false),
            ]),
        );
        let focused = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![key_event(egui::Key::A, true)]),
        );

        assert!(contains_keyboard_input(&focused));
    }

    #[test]
    fn translator_emits_canvas_resize_once_per_size() {
        let mut translator = EguiEventTranslator::default();
        let response = response_for_size(egui::vec2(300.0, 200.0));
        let input = egui::InputState::default();

        let first = translator.translate_frame(
            response.rect,
            EguiResponseState::from_response(&response),
            &input,
        );
        assert_eq!(
            first.last(),
            Some(&WindowEvent::CanvasResize(CanvasResizeEvent {
                size: [300.0, 200.0],
            }))
        );

        let second = translator.translate_frame(
            response.rect,
            EguiResponseState::from_response(&response),
            &input,
        );
        assert!(
            !second
                .iter()
                .any(|event| matches!(event, WindowEvent::CanvasResize(_)))
        );
    }
}
