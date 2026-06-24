use avenger_chart::prelude::Plot;
use avenger_chart::render::InteractionScopeKind;
use avenger_chart_webmercator::{
    Symbol, WebMercator, WebMercatorSymbolPositionChannels, project_lon_lat,
};
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
