use avenger_app::app::AvengerApp;
use avenger_chart::{prelude::Plot, render::InteractionScopeKind};
use avenger_chart_app::{
    ChartAppOptions, ChartAppState, ChartRuntimeResources, chart_avenger_app,
    chart_avenger_app_with_runtime_resources,
};
use avenger_chart_webmercator::{RasterTileLayer, WebMercator, WebMercatorPanZoom};
use avenger_common::time::{Duration, Instant};
use avenger_eventstream::window::{
    ElementState, Key, MouseButton, MouseScrollDelta, NamedKey, WindowCursorMoved, WindowEvent,
    WindowKeyboardInput, WindowMouseInput, WindowMouseWheel,
};
use avenger_image::{ImageResourceResolver, ImageResourceState};
use avenger_resource::{RenderInvalidationHub, ResourceKey, ResourceRequest};
use avenger_scenegraph::{
    marks::{
        image::{SceneImageMark, SceneImageSource},
        mark::SceneMark,
    },
    scene_graph::SceneGraph,
};
use datafusion::{common::ScalarValue, prelude::SessionContext};
use std::sync::{Arc, Mutex};

const VIEWPORT_ID: &str = "main";
const CENTER_X_PARAM: &str = "__webmercator_main_center_x";
const CENTER_Y_PARAM: &str = "__webmercator_main_center_y";
const UNITS_PER_PIXEL_PARAM: &str = "__webmercator_main_units_per_pixel";
const BOX_ACTIVE_PARAM: &str = "__tool_webmercator_pan_zoom__box_active";
const BOX_X0_PARAM: &str = "__tool_webmercator_pan_zoom__box_x0";
const BOX_X1_PARAM: &str = "__tool_webmercator_pan_zoom__box_x1";

#[tokio::test]
async fn pan_drag_updates_webmercator_viewport_params() {
    let (mut app, state) = app_with_webmercator_pan_zoom().await;
    let scope = coordinate_scope(&state).await;
    let center = scope_center(&scope);
    let initial_units = units_per_pixel(&scope);
    let start = Instant::now();

    dispatch_cursor(&mut app, center, start).await;
    dispatch_left_mouse(&mut app, ElementState::Pressed, start).await;
    let update = dispatch_cursor(
        &mut app,
        [center[0] + 40.0, center[1] - 20.0],
        start + Duration::from_millis(16),
    )
    .await;
    dispatch_left_mouse(
        &mut app,
        ElementState::Released,
        start + Duration::from_millis(32),
    )
    .await;

    assert!(update.status.rerender);
    assert!(!update.status.rebuild_geometry);
    assert_close(param_f64(&state, CENTER_X_PARAM), -40.0 * initial_units);
    assert_close(param_f64(&state, CENTER_Y_PARAM), -20.0 * initial_units);
    assert_close(param_f64(&state, UNITS_PER_PIXEL_PARAM), initial_units);
}

#[tokio::test]
async fn wheel_zoom_anchors_view_at_cursor() {
    let (mut app, state) = app_with_webmercator_pan_zoom().await;
    let scope = coordinate_scope(&state).await;
    let center = scope_center(&scope);
    let initial_units = units_per_pixel(&scope);
    let start = Instant::now();

    dispatch_cursor(&mut app, center, start).await;
    let update = dispatch_wheel(
        &mut app,
        MouseScrollDelta::LineDelta(0.0, 2.0),
        start + Duration::from_millis(16),
    )
    .await;

    assert!(update.status.rerender);
    assert!(!update.status.rebuild_geometry);
    assert_close(param_f64(&state, CENTER_X_PARAM), 0.0);
    assert_close(param_f64(&state, CENTER_Y_PARAM), 0.0);
    assert!(param_f64(&state, UNITS_PER_PIXEL_PARAM) < initial_units);
}

#[tokio::test]
async fn double_click_resets_webmercator_viewport_params() {
    let (mut app, state) = app_with_webmercator_pan_zoom().await;
    let scope = coordinate_scope(&state).await;
    let center = scope_center(&scope);
    let start = Instant::now();

    dispatch_cursor(&mut app, center, start).await;
    dispatch_wheel(
        &mut app,
        MouseScrollDelta::LineDelta(0.0, 2.0),
        start + Duration::from_millis(16),
    )
    .await;
    assert!(state.param_f64(UNITS_PER_PIXEL_PARAM).is_some());

    click_left(&mut app, start + Duration::from_millis(32)).await;
    let update = click_left(&mut app, start + Duration::from_millis(64)).await;

    assert!(update.status.rerender);
    assert!(update.status.rebuild_geometry);
    assert_null_param(&state, CENTER_X_PARAM);
    assert_null_param(&state, CENTER_Y_PARAM);
    assert_null_param(&state, UNITS_PER_PIXEL_PARAM);
}

#[tokio::test]
async fn shift_drag_box_zoom_commits_viewport_aspect_selection() {
    let (mut app, state) = app_with_webmercator_pan_zoom().await;
    let scope = coordinate_scope(&state).await;
    let bounds = scope.bounds;
    let center = scope_center(&scope);
    let initial_units = units_per_pixel(&scope);
    let start_pos = [
        center[0] - bounds.width * 0.25,
        center[1] + bounds.height * 0.25,
    ];
    let end_pos = [
        center[0] + bounds.width * 0.25,
        center[1] - bounds.height * 0.25,
    ];
    let start = Instant::now();

    dispatch_cursor(&mut app, start_pos, start).await;
    dispatch_shift(
        &mut app,
        ElementState::Pressed,
        start + Duration::from_millis(1),
    )
    .await;
    dispatch_left_mouse(
        &mut app,
        ElementState::Pressed,
        start + Duration::from_millis(2),
    )
    .await;
    dispatch_cursor(&mut app, end_pos, start + Duration::from_millis(16)).await;
    let update = dispatch_left_mouse(
        &mut app,
        ElementState::Released,
        start + Duration::from_millis(32),
    )
    .await;
    dispatch_shift(
        &mut app,
        ElementState::Released,
        start + Duration::from_millis(48),
    )
    .await;

    assert!(update.status.rerender);
    assert!(update.status.rebuild_geometry);
    assert_close(param_f64(&state, CENTER_X_PARAM), 0.0);
    assert_close(param_f64(&state, CENTER_Y_PARAM), 0.0);
    assert_close(
        param_f64(&state, UNITS_PER_PIXEL_PARAM),
        initial_units * 0.5,
    );
    assert!(!param_bool(&state, BOX_ACTIVE_PARAM));
}

#[tokio::test]
async fn shift_drag_box_zoom_previews_viewport_aspect_overlay_without_committing() {
    let (mut app, state) = app_with_webmercator_pan_zoom().await;
    let scope = coordinate_scope(&state).await;
    let bounds = scope.bounds;
    let center = scope_center(&scope);
    let start_pos = [
        center[0] - bounds.width * 0.25,
        center[1] + bounds.height * 0.25,
    ];
    let end_pos = [
        center[0] + bounds.width * 0.25,
        center[1] - bounds.height * 0.25,
    ];
    let start = Instant::now();

    dispatch_cursor(&mut app, start_pos, start).await;
    dispatch_shift(
        &mut app,
        ElementState::Pressed,
        start + Duration::from_millis(1),
    )
    .await;
    dispatch_left_mouse(
        &mut app,
        ElementState::Pressed,
        start + Duration::from_millis(2),
    )
    .await;
    let update = dispatch_cursor(&mut app, end_pos, start + Duration::from_millis(16)).await;

    assert!(update.status.rerender);
    assert!(!update.status.rebuild_geometry);
    assert!(param_bool(&state, BOX_ACTIVE_PARAM));
    assert_ne!(
        param_f64(&state, BOX_X0_PARAM),
        param_f64(&state, BOX_X1_PARAM)
    );
    assert_null_param(&state, CENTER_X_PARAM);
    assert_null_param(&state, CENTER_Y_PARAM);
    assert_null_param(&state, UNITS_PER_PIXEL_PARAM);
}

#[tokio::test]
async fn shift_drag_box_zoom_preview_is_nonblocking_with_pending_tiles() {
    let resolver = Arc::new(PendingImageResolver::default());
    let resources = ChartRuntimeResources::new(resolver.clone(), RenderInvalidationHub::default());
    let coord = WebMercator::new()
        .viewport_id(VIEWPORT_ID)
        .center_projected(0.0, 0.0)
        .zoom(1.0)
        .tiles(
            RasterTileLayer::xyz("https://example.com/tiles/{z}/{x}/{y}.png")
                .max_zoom(1)
                .attribution("Example"),
        );
    let (mut app, state) = app_with_webmercator_pan_zoom_and_resources(coord, resources).await;
    assert!(
        !state.last_resource_requests().await.is_empty(),
        "tile guide should request image resources"
    );
    assert!(
        !resolver.requests().is_empty(),
        "chart app should submit tile image requests to the resolver"
    );

    let scope = coordinate_scope(&state).await;
    let bounds = scope.bounds;
    let center = scope_center(&scope);
    let start_pos = [
        center[0] - bounds.width * 0.25,
        center[1] + bounds.height * 0.25,
    ];
    let end_pos = [
        center[0] + bounds.width * 0.25,
        center[1] - bounds.height * 0.25,
    ];
    let start = Instant::now();

    dispatch_cursor(&mut app, start_pos, start).await;
    dispatch_shift(
        &mut app,
        ElementState::Pressed,
        start + Duration::from_millis(1),
    )
    .await;
    dispatch_left_mouse(
        &mut app,
        ElementState::Pressed,
        start + Duration::from_millis(2),
    )
    .await;
    let update = dispatch_cursor(&mut app, end_pos, start + Duration::from_millis(16)).await;

    assert!(update.status.rerender);
    assert!(!update.status.rebuild_geometry);
    assert!(update.scene_graph.is_some());
    assert!(param_bool(&state, BOX_ACTIVE_PARAM));
}

#[tokio::test]
async fn pan_drag_repositions_webmercator_tiles_during_preview() {
    let resolver = Arc::new(PendingImageResolver::default());
    let resources = ChartRuntimeResources::new(resolver.clone(), RenderInvalidationHub::default());
    let coord = tiled_webmercator_coord();
    let (mut app, state) = app_with_webmercator_pan_zoom_and_resources(coord, resources).await;
    let initial_tiles = tile_image_signatures(app.scene_graph());
    assert!(
        !initial_tiles.is_empty(),
        "initial scene should contain tile image marks"
    );

    let scope = coordinate_scope(&state).await;
    let center = scope_center(&scope);
    let start = Instant::now();

    dispatch_cursor(&mut app, center, start).await;
    dispatch_left_mouse(&mut app, ElementState::Pressed, start).await;
    let update = dispatch_cursor(
        &mut app,
        [center[0] + 40.0, center[1] - 20.0],
        start + Duration::from_millis(16),
    )
    .await;

    assert!(update.status.rerender);
    assert!(!update.status.rebuild_geometry);
    let preview_scene = update
        .scene_graph
        .as_deref()
        .expect("pan preview should rebuild scene graph");
    assert_tile_signatures_changed(
        "pan preview",
        &initial_tiles,
        &tile_image_signatures(preview_scene),
    );
}

#[tokio::test]
async fn wheel_zoom_repositions_webmercator_tiles_during_preview() {
    let resolver = Arc::new(PendingImageResolver::default());
    let resources = ChartRuntimeResources::new(resolver.clone(), RenderInvalidationHub::default());
    let coord = tiled_webmercator_coord();
    let (mut app, state) = app_with_webmercator_pan_zoom_and_resources(coord, resources).await;
    let initial_tiles = tile_image_signatures(app.scene_graph());
    assert!(
        !initial_tiles.is_empty(),
        "initial scene should contain tile image marks"
    );

    let scope = coordinate_scope(&state).await;
    let center = scope_center(&scope);
    let start = Instant::now();

    dispatch_cursor(&mut app, center, start).await;
    let update = dispatch_wheel(
        &mut app,
        MouseScrollDelta::LineDelta(0.0, 2.0),
        start + Duration::from_millis(16),
    )
    .await;

    assert!(update.status.rerender);
    assert!(!update.status.rebuild_geometry);
    let preview_scene = update
        .scene_graph
        .as_deref()
        .expect("wheel preview should rebuild scene graph");
    assert_tile_signatures_changed(
        "wheel preview",
        &initial_tiles,
        &tile_image_signatures(preview_scene),
    );
}

async fn app_with_webmercator_pan_zoom() -> (AvengerApp<ChartAppState>, ChartAppState) {
    let coord = WebMercator::new()
        .viewport_id(VIEWPORT_ID)
        .center_projected(0.0, 0.0)
        .zoom(1.0);
    app_with_webmercator_pan_zoom_for_coord(coord).await
}

async fn app_with_webmercator_pan_zoom_for_coord(
    coord: WebMercator,
) -> (AvengerApp<ChartAppState>, ChartAppState) {
    let ctx = Arc::new(SessionContext::new());
    let compiled = Plot::with_coord(coord)
        .plot_size(400.0, 200.0)
        .tool(WebMercatorPanZoom::new().viewport_id(VIEWPORT_ID))
        .compile(ctx.as_ref())
        .await
        .expect("compile WebMercator pan/zoom plot");
    let mut app = chart_avenger_app(compiled, ctx, ChartAppOptions::default())
        .await
        .expect("create chart app");
    let state = app.app_state_mut().clone();
    (app, state)
}

async fn app_with_webmercator_pan_zoom_and_resources(
    coord: WebMercator,
    resources: ChartRuntimeResources,
) -> (AvengerApp<ChartAppState>, ChartAppState) {
    let ctx = Arc::new(SessionContext::new());
    let compiled = Plot::with_coord(coord)
        .plot_size(400.0, 200.0)
        .tool(WebMercatorPanZoom::new().viewport_id(VIEWPORT_ID))
        .compile(ctx.as_ref())
        .await
        .expect("compile WebMercator pan/zoom plot");
    let mut app = chart_avenger_app_with_runtime_resources(
        compiled,
        ctx,
        ChartAppOptions::default(),
        resources,
    )
    .await
    .expect("create chart app");
    let state = app.app_state_mut().clone();
    (app, state)
}

fn tiled_webmercator_coord() -> WebMercator {
    WebMercator::new()
        .viewport_id(VIEWPORT_ID)
        .center_projected(0.0, 0.0)
        .zoom(1.0)
        .tiles(
            RasterTileLayer::xyz("https://example.com/tiles/{z}/{x}/{y}.png")
                .max_zoom(1)
                .attribution("Example"),
        )
}

async fn coordinate_scope(
    state: &ChartAppState,
) -> avenger_chart::render::EvaluatedInteractionScope {
    state
        .interaction_scopes()
        .await
        .into_iter()
        .find(|scope| scope.kind == InteractionScopeKind::Coordinate)
        .expect("coordinate interaction scope")
}

fn scope_center(scope: &avenger_chart::render::EvaluatedInteractionScope) -> [f32; 2] {
    [
        scope.bounds.x + scope.bounds.width / 2.0,
        scope.bounds.y + scope.bounds.height / 2.0,
    ]
}

fn units_per_pixel(scope: &avenger_chart::render::EvaluatedInteractionScope) -> f64 {
    let x_domain = scope
        .scales
        .get("x")
        .expect("x scale")
        .numeric_interval_domain()
        .expect("x domain");
    (f64::from(x_domain.1) - f64::from(x_domain.0)) / f64::from(scope.plot_area_width)
}

async fn dispatch_cursor(
    app: &mut AvengerApp<ChartAppState>,
    position: [f32; 2],
    instant: Instant,
) -> avenger_app::app::AppUpdate {
    app.update_with_status(
        &WindowEvent::CursorMoved(WindowCursorMoved { position }),
        instant,
    )
    .await
    .expect("cursor event")
}

async fn dispatch_left_mouse(
    app: &mut AvengerApp<ChartAppState>,
    state: ElementState,
    instant: Instant,
) -> avenger_app::app::AppUpdate {
    app.update_with_status(
        &WindowEvent::MouseInput(WindowMouseInput {
            state,
            button: MouseButton::Left,
        }),
        instant,
    )
    .await
    .expect("left mouse event")
}

async fn dispatch_wheel(
    app: &mut AvengerApp<ChartAppState>,
    delta: MouseScrollDelta,
    instant: Instant,
) -> avenger_app::app::AppUpdate {
    app.update_with_status(
        &WindowEvent::MouseWheel(WindowMouseWheel { delta }),
        instant,
    )
    .await
    .expect("wheel event")
}

async fn dispatch_shift(
    app: &mut AvengerApp<ChartAppState>,
    state: ElementState,
    instant: Instant,
) -> avenger_app::app::AppUpdate {
    app.update_with_status(
        &WindowEvent::KeyboardInput(WindowKeyboardInput {
            key: Key::Named(NamedKey::Shift),
            state,
        }),
        instant,
    )
    .await
    .expect("shift key event")
}

async fn click_left(
    app: &mut AvengerApp<ChartAppState>,
    instant: Instant,
) -> avenger_app::app::AppUpdate {
    dispatch_left_mouse(app, ElementState::Pressed, instant).await;
    dispatch_left_mouse(
        app,
        ElementState::Released,
        instant + Duration::from_millis(1),
    )
    .await
}

#[derive(Debug, Clone, PartialEq)]
struct TileImageSignature {
    name: String,
    key: ResourceKey,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

fn tile_image_signatures(scene_graph: &SceneGraph) -> Vec<TileImageSignature> {
    let mut signatures = Vec::new();
    collect_tile_image_signatures(scene_graph.children(), &mut signatures);
    signatures.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.key.0.cmp(&b.key.0)));
    signatures
}

fn collect_tile_image_signatures(marks: &[SceneMark], out: &mut Vec<TileImageSignature>) {
    for mark in marks {
        match mark {
            SceneMark::Image(image) => {
                if let Some(signature) = tile_image_signature(image) {
                    out.push(signature);
                }
            }
            SceneMark::Group(group) => collect_tile_image_signatures(&group.marks, out),
            _ => {}
        }
    }
}

fn tile_image_signature(image: &SceneImageMark) -> Option<TileImageSignature> {
    if !image.name.starts_with("webmercator-tile-") {
        return None;
    }
    let key = image.image_source_iter().find_map(|source| match source {
        SceneImageSource::Resource(resource) => Some(resource.key.clone()),
        _ => None,
    })?;
    Some(TileImageSignature {
        name: image.name.clone(),
        key,
        x: first_f32(&image.x, image.len, "x"),
        y: first_f32(&image.y, image.len, "y"),
        width: first_f32(&image.width, image.len, "width"),
        height: first_f32(&image.height, image.len, "height"),
    })
}

fn first_f32(values: &avenger_common::value::ScalarOrArray<f32>, len: u32, channel: &str) -> f32 {
    values
        .as_vec(len as usize, None)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("tile image channel {channel} should have a value"))
}

fn assert_tile_signatures_changed(
    label: &str,
    initial: &[TileImageSignature],
    preview: &[TileImageSignature],
) {
    assert!(
        !preview.is_empty(),
        "{label} scene should contain tile image marks"
    );
    assert_ne!(
        initial, preview,
        "{label} should update tile geometry or resources during Preview"
    );
}

fn param_f64(state: &ChartAppState, name: &str) -> f64 {
    state
        .param_f64(name)
        .unwrap_or_else(|| panic!("missing numeric param {name}"))
}

fn param_bool(state: &ChartAppState, name: &str) -> bool {
    match state.param_snapshot().params.get(name) {
        Some(ScalarValue::Boolean(Some(value))) => *value,
        other => panic!("expected boolean param {name}, got {other:?}"),
    }
}

fn assert_null_param(state: &ChartAppState, name: &str) {
    let snapshot = state.param_snapshot();
    assert_eq!(snapshot.params.get(name), Some(&ScalarValue::Float64(None)));
}

#[derive(Default)]
struct PendingImageResolver {
    requests: Mutex<Vec<ResourceRequest>>,
}

impl PendingImageResolver {
    fn requests(&self) -> Vec<ResourceRequest> {
        self.requests
            .lock()
            .expect("pending resolver lock poisoned")
            .clone()
    }
}

impl ImageResourceResolver for PendingImageResolver {
    fn image_state(&self, _key: &ResourceKey) -> ImageResourceState {
        ImageResourceState::Pending
    }

    fn request_image(&self, request: &ResourceRequest) {
        self.requests
            .lock()
            .expect("pending resolver lock poisoned")
            .push(request.clone());
    }
}

fn assert_close(actual: f64, expected: f64) {
    let tolerance = (expected.abs() * 1e-6).max(0.5);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, got {actual}"
    );
}
