use avenger_chart::prelude::{Plot, Rule, Text};
use avenger_chart::render::InteractionScopeKind;
use avenger_chart_webmercator::{
    Symbol, WebMercator, WebMercatorSymbolPositionChannels, project_lon_lat,
};
use avenger_scenegraph::marks::{
    mark::SceneMark, rule::SceneRuleMark, symbol::SceneSymbolMark, text::SceneTextMark,
};
use datafusion::logical_expr::lit;
use datafusion::prelude::{SessionContext, col};

#[tokio::test]
async fn symbol_lon_lat_compiles_and_evaluates_through_chart_facade() {
    let ctx = SessionContext::new();
    let df = ctx
        .sql("SELECT -73.9857 AS lon, 40.7484 AS lat")
        .await
        .expect("dataframe");
    let projected = project_lon_lat(-73.9857, 40.7484);

    let plot = Plot::with_coord(WebMercator::new()).data(df).mark(
        Symbol::new()
            .longitude(col("lon"))
            .latitude(col("lat"))
            .size(100.0),
    );

    let evaluated = plot
        .compile(&ctx)
        .await
        .expect("compile")
        .evaluate(&ctx, None)
        .await
        .expect("evaluate");

    let scope = evaluated
        .interaction
        .scopes
        .iter()
        .find(|scope| scope.kind == InteractionScopeKind::Coordinate)
        .expect("coordinate interaction scope");
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

    assert!(x_domain.0 as f64 <= projected.x && projected.x <= x_domain.1 as f64);
    assert!(y_domain.0 as f64 <= projected.y && projected.y <= y_domain.1 as f64);
}

#[tokio::test]
async fn symbol_radius_participates_in_webmercator_fit() {
    let small = fitted_domain_spans_for_symbol_size(1.0).await;
    let large = fitted_domain_spans_for_symbol_size(10_000.0).await;

    assert!(
        large.0 > small.0 * 1.1,
        "larger symbol should expand fitted x span: small={}, large={}",
        small.0,
        large.0
    );
    assert!(
        large.1 > small.1 * 1.1,
        "larger symbol should expand fitted y span: small={}, large={}",
        small.1,
        large.1
    );
}

async fn fitted_domain_spans_for_symbol_size(size: f64) -> (f64, f64) {
    let ctx = SessionContext::new();
    let df = ctx
        .sql("SELECT -1.0 AS lon, -1.0 AS lat UNION ALL SELECT 1.0 AS lon, 1.0 AS lat")
        .await
        .expect("dataframe");
    let evaluated = Plot::with_coord(WebMercator::new())
        .data(df)
        .plot_size(400.0, 400.0)
        .mark(
            Symbol::new()
                .longitude(col("lon"))
                .latitude(col("lat"))
                .size(size),
        )
        .compile(&ctx)
        .await
        .expect("compile")
        .evaluate(&ctx, None)
        .await
        .expect("evaluate");
    let scope = evaluated
        .interaction
        .scopes
        .iter()
        .find(|scope| scope.kind == InteractionScopeKind::Coordinate)
        .expect("coordinate interaction scope");
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

    (
        f64::from(x_domain.1) - f64::from(x_domain.0),
        f64::from(y_domain.1) - f64::from(y_domain.0),
    )
}

#[tokio::test]
async fn symbol_adjustments_use_rendered_webmercator_item_frame() {
    let base_x = rendered_symbol_x(
        Symbol::new()
            .unit_data()
            .projected_x(0.0)
            .projected_y(0.0)
            .size(100.0),
    )
    .await;
    let adjusted_x = rendered_symbol_x(
        Symbol::new()
            .unit_data()
            .projected_x(0.0)
            .projected_y(0.0)
            .size(100.0)
            .adjust(|point| point.x(point.channel("x") + lit(12.0))),
    )
    .await;

    assert_close(adjusted_x - base_x, 12.0, 1e-4);
}

#[tokio::test]
async fn symbol_can_derive_rule_from_rendered_webmercator_geometry() {
    let evaluated = evaluated_single_symbol_plot(
        Symbol::new()
            .unit_data()
            .projected_x(0.0)
            .projected_y(0.0)
            .size(100.0)
            .derive(|point| {
                Rule::<WebMercator>::new()
                    .with_channel_value("x", point.channel("x").into())
                    .with_channel_value("y", point.channel("y").into())
                    .with_channel_value("x2", (point.channel("x") + lit(18.0)).into())
                    .with_channel_value("y2", point.channel("y").into())
                    .stroke("#ef4444")
                    .stroke_width(2.0)
            }),
    )
    .await;
    let rule = first_rule(&evaluated.scene_graph.marks).expect("derived rule");
    let x = rule.x.as_vec(rule.len as usize, None)[0];
    let x2 = rule.x2.as_vec(rule.len as usize, None)[0];

    assert_close(x2 - x, 18.0, 1e-4);
}

#[tokio::test]
async fn symbol_can_derive_text_from_rendered_webmercator_geometry_and_source_data() {
    let ctx = SessionContext::new();
    let df = ctx
        .sql("SELECT 0.0 AS lon, 0.0 AS lat, 'origin' AS label")
        .await
        .expect("dataframe");
    let evaluated = Plot::with_coord(WebMercator::new().center_projected(0.0, 0.0).zoom(2.0))
        .plot_size(300.0, 300.0)
        .data(df)
        .mark(
            Symbol::new()
                .longitude(col("lon"))
                .latitude(col("lat"))
                .size(100.0)
                .derive(|point| {
                    Text::<WebMercator>::new()
                        .with_channel_value("x", (point.channel("x") + lit(6.0)).into())
                        .with_channel_value("y", point.channel("y").into())
                        .text(point.data("label"))
                        .font_size(14.0)
                }),
        )
        .compile(&ctx)
        .await
        .expect("compile")
        .evaluate(&ctx, None)
        .await
        .expect("evaluate");

    let text = first_text(&evaluated.scene_graph.marks).expect("derived text");
    assert_eq!(text.text.as_vec(text.len as usize, None)[0], "origin");
}

async fn rendered_symbol_x(mark: Symbol<WebMercator>) -> f32 {
    let evaluated = evaluated_single_symbol_plot(mark).await;
    let symbol = first_symbol(&evaluated.scene_graph.marks).expect("symbol");
    symbol.x.as_vec(symbol.len as usize, None)[0]
}

async fn evaluated_single_symbol_plot(
    mark: Symbol<WebMercator>,
) -> avenger_chart::render::EvaluatedPlot {
    let ctx = SessionContext::new();
    Plot::with_coord(WebMercator::new().center_projected(0.0, 0.0).zoom(2.0))
        .plot_size(300.0, 300.0)
        .mark(mark)
        .compile(&ctx)
        .await
        .expect("compile")
        .evaluate(&ctx, None)
        .await
        .expect("evaluate")
}

fn first_symbol(marks: &[SceneMark]) -> Option<&SceneSymbolMark> {
    marks.iter().find_map(|mark| match mark {
        SceneMark::Symbol(symbol) => Some(symbol),
        SceneMark::Group(group) => first_symbol(&group.marks),
        _ => None,
    })
}

fn first_rule(marks: &[SceneMark]) -> Option<&SceneRuleMark> {
    marks.iter().find_map(|mark| match mark {
        SceneMark::Rule(rule) => Some(rule),
        SceneMark::Group(group) => first_rule(&group.marks),
        _ => None,
    })
}

fn first_text(marks: &[SceneMark]) -> Option<&SceneTextMark> {
    marks.iter().find_map(|mark| match mark {
        SceneMark::Text(text) => Some(text.as_ref()),
        SceneMark::Group(group) => first_text(&group.marks),
        _ => None,
    })
}

fn assert_close(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {actual} to be within {tolerance} of {expected}"
    );
}
