use avenger_chart::prelude::Plot;
use avenger_chart::render::InteractionScopeKind;
use avenger_chart_webmercator::{WebMercator, WebMercatorPanZoom};
use datafusion::common::ScalarValue;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;

#[tokio::test]
async fn pan_zoom_tool_registers_viewport_params_and_bindings() {
    let ctx = SessionContext::new();
    let compiled = Plot::with_coord(WebMercator::new().viewport_id("main"))
        .tool(WebMercatorPanZoom::new().viewport_id("main"))
        .compile(&ctx)
        .await
        .expect("compile");

    assert!(
        compiled
            .param_specs()
            .contains_key("__tool_webmercator_pan_zoom__enabled")
    );
    assert!(
        compiled
            .param_specs()
            .contains_key("__webmercator_main_center_x")
    );
    assert!(
        compiled
            .param_specs()
            .contains_key("__webmercator_main_center_y")
    );
    assert!(
        compiled
            .param_specs()
            .contains_key("__webmercator_main_units_per_pixel")
    );
    assert_eq!(compiled.event_bindings().len(), 4);
    assert!(compiled.tool_metadata().iter().any(|meta| {
        meta.id == "webmercator_pan_zoom"
            && meta.enabled_param.as_deref() == Some("__tool_webmercator_pan_zoom__enabled")
    }));
}

#[tokio::test]
async fn viewport_params_drive_evaluated_domains() {
    let ctx = SessionContext::new();
    let compiled = Plot::with_coord(WebMercator::new().viewport_id("main"))
        .tool(WebMercatorPanZoom::new().viewport_id("main"))
        .compile(&ctx)
        .await
        .expect("compile");
    let mut params = IndexMap::new();
    params.insert(
        "__webmercator_main_center_x".to_string(),
        ScalarValue::Float64(Some(100.0)),
    );
    params.insert(
        "__webmercator_main_center_y".to_string(),
        ScalarValue::Float64(Some(-50.0)),
    );
    params.insert(
        "__webmercator_main_units_per_pixel".to_string(),
        ScalarValue::Float64(Some(2.0)),
    );

    let evaluated = compiled
        .evaluate(&ctx, Some(params))
        .await
        .expect("evaluate");
    let scope = evaluated
        .interaction
        .scopes
        .iter()
        .find(|scope| scope.kind == InteractionScopeKind::Coordinate)
        .expect("coordinate scope");
    let x_domain = scope
        .scales
        .get("x")
        .expect("x scale")
        .numeric_interval_domain()
        .expect("x domain");
    let y_domain = scope
        .scales
        .get("y")
        .expect("y scale")
        .numeric_interval_domain()
        .expect("y domain");

    assert_close(
        f64::from(x_domain.0),
        100.0 - f64::from(scope.plot_area_width),
    );
    assert_close(
        f64::from(x_domain.1),
        100.0 + f64::from(scope.plot_area_width),
    );
    assert_close(
        f64::from(y_domain.0),
        -50.0 - f64::from(scope.plot_area_height),
    );
    assert_close(
        f64::from(y_domain.1),
        -50.0 + f64::from(scope.plot_area_height),
    );
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-5,
        "expected {expected}, got {actual}"
    );
}
