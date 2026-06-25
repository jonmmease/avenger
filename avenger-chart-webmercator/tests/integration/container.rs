use avenger_chart::prelude::{
    CoordinationScope, FacetColumn, FacetColumnSubplotChannels, HConcat, Plot, RepeatGrid,
    RepeatVariable, ScaleChannelConfig, Subplot, SvgRenderer, col, repeat,
};
use avenger_chart::render::{EvaluatedInteractionScope, InteractionScopeKind};
use avenger_chart_webmercator::{
    Symbol, WebMercator, WebMercatorSymbolPositionChannels, project_lon_lat,
};
use datafusion::{common::ScalarValue, prelude::SessionContext};

#[tokio::test]
async fn facet_shared_webmercator_viewport_unions_projected_bounds() {
    let ctx = SessionContext::new();
    let evaluated = facet_webmercator_plot(&ctx, CoordinationScope::Shared)
        .await
        .expect("evaluate shared WebMercator facet");
    let scopes = coordinate_scopes(&evaluated);

    assert_eq!(scopes.len(), 2);
    assert_domains_close(x_domain(scopes[0]), x_domain(scopes[1]));
    assert_domains_close(y_domain(scopes[0]), y_domain(scopes[1]));

    let left = project_lon_lat(-1.0, 0.0);
    let right = project_lon_lat(21.0, 0.0);
    let shared_x = x_domain(scopes[0]);
    assert!(shared_x.0 <= left.x && right.x <= shared_x.1);
}

#[tokio::test]
async fn facet_shared_webmercator_fixed_center_infers_zoom_from_all_panels() {
    let ctx = SessionContext::new();
    let evaluated = facet_webmercator_plot_with_coord(
        &ctx,
        WebMercator::new().center_projected(0.0, 0.0),
        CoordinationScope::Shared,
    )
    .await
    .expect("evaluate fixed-center shared WebMercator facet");
    let scopes = coordinate_scopes(&evaluated);

    assert_eq!(scopes.len(), 2);
    assert_domains_close(x_domain(scopes[0]), x_domain(scopes[1]));
    assert_domains_close(y_domain(scopes[0]), y_domain(scopes[1]));
    assert_close(domain_center(x_domain(scopes[0])), 0.0);
    assert_close(domain_center(y_domain(scopes[0])), 0.0);

    let left = project_lon_lat(-1.0, 0.0);
    let right = project_lon_lat(21.0, 0.0);
    let shared_x = x_domain(scopes[0]);
    assert!(shared_x.0 <= left.x && right.x <= shared_x.1);
}

#[tokio::test]
async fn facet_free_webmercator_viewports_fit_each_panel_independently() {
    let ctx = SessionContext::new();
    let evaluated = facet_webmercator_plot(&ctx, CoordinationScope::Free)
        .await
        .expect("evaluate free WebMercator facet");
    let scopes = coordinate_scopes(&evaluated);

    assert_eq!(scopes.len(), 2);
    let left_scope = scope_for_panel(&scopes, "left");
    let right_scope = scope_for_panel(&scopes, "right");
    let left_x = x_domain(left_scope);
    let right_x = x_domain(right_scope);
    let left_point = project_lon_lat(-1.0, 0.0);
    let right_point = project_lon_lat(21.0, 0.0);

    assert!(left_x.0 <= left_point.x && left_point.x <= left_x.1);
    assert!(right_x.0 <= right_point.x && right_point.x <= right_x.1);
    assert!(
        right_x.0 > left_x.1,
        "free facet viewports should fit disjoint panels independently: left={left_x:?}, right={right_x:?}"
    );
}

#[tokio::test]
async fn generated_repeat_grid_shared_webmercator_viewports_use_repeat_domain_groups() {
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 0.0 AS row_a, 1.0 AS row_b, -1.0 AS near, 19.0 AS far \
             UNION ALL SELECT 0.0 AS row_a, 1.0 AS row_b, 1.0 AS near, 21.0 AS far",
        )
        .await
        .expect("dataframe");
    let cell = Plot::with_coord(WebMercator::new()).mark(
        Symbol::new()
            .longitude(repeat::column())
            .latitude(repeat::column())
            .size(100.0),
    );
    let evaluated = Plot::<RepeatGrid>::new()
        .plot_size(180.0, 140.0)
        .data(df)
        .rows([
            RepeatVariable::new("row_a", col("row_a")),
            RepeatVariable::new("row_b", col("row_b")),
        ])
        .columns([
            RepeatVariable::new("near", col("near")),
            RepeatVariable::new("far", col("far")),
        ])
        .matrix_domains()
        .cell(cell)
        .compile(&ctx)
        .await
        .expect("compile repeat grid")
        .evaluate(&ctx, None)
        .await
        .expect("evaluate repeat grid");
    let scopes = coordinate_scopes(&evaluated);

    assert_eq!(scopes.len(), 4);
    let near_top = scope_for_grid_cell(&scopes, 0, 0);
    let near_bottom = scope_for_grid_cell(&scopes, 1, 0);
    let far_top = scope_for_grid_cell(&scopes, 0, 1);
    let far_bottom = scope_for_grid_cell(&scopes, 1, 1);
    assert_domains_close(x_domain(near_top), x_domain(near_bottom));
    assert_domains_close(y_domain(near_top), y_domain(near_bottom));
    assert_domains_close(x_domain(far_top), x_domain(far_bottom));
    assert_domains_close(y_domain(far_top), y_domain(far_bottom));
    assert!(
        x_domain(far_top).0 > x_domain(near_top).1,
        "different repeated columns should keep distinct generated viewport groups"
    );
}

#[tokio::test]
async fn authored_concat_rejects_shared_webmercator_viewport_domains() {
    let ctx = SessionContext::new();
    let df = ctx
        .sql("SELECT -1.0 AS lon, 0.0 AS lat UNION ALL SELECT 1.0 AS lon, 0.0 AS lat")
        .await
        .expect("dataframe");
    let child = || {
        Plot::with_coord(WebMercator::new()).data(df.clone()).mark(
            Symbol::new()
                .longitude_with(col("lon"), |x| {
                    x.with_domain_scope(CoordinationScope::Shared)
                })
                .latitude_with(col("lat"), |y| {
                    y.with_domain_scope(CoordinationScope::Shared)
                })
                .size(100.0),
        )
    };
    let plot = Plot::<HConcat>::new()
        .plot_size(260.0, 180.0)
        .mark(Subplot::new(child()).key("left"))
        .mark(Subplot::new(child()).key("right"));
    let compiled = plot.compile(&ctx).await.expect("compile concat");

    let err = SvgRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .expect_err("authored concat shared WebMercator domains should be rejected");

    assert!(
        err.to_string()
            .contains("does not support authored concat/grid shared domains"),
        "{err}"
    );
}

async fn facet_webmercator_plot(
    ctx: &SessionContext,
    sharing: CoordinationScope,
) -> Result<avenger_chart::render::EvaluatedPlot, avenger_chart::prelude::AvengerChartError> {
    facet_webmercator_plot_with_coord(ctx, WebMercator::new(), sharing).await
}

async fn facet_webmercator_plot_with_coord(
    ctx: &SessionContext,
    coord: WebMercator,
    sharing: CoordinationScope,
) -> Result<avenger_chart::render::EvaluatedPlot, avenger_chart::prelude::AvengerChartError> {
    let df = ctx
        .sql(
            "SELECT 'left' AS panel, -1.0 AS lon, 0.0 AS lat \
             UNION ALL SELECT 'left' AS panel, 1.0 AS lon, 0.0 AS lat \
             UNION ALL SELECT 'right' AS panel, 19.0 AS lon, 0.0 AS lat \
             UNION ALL SELECT 'right' AS panel, 21.0 AS lon, 0.0 AS lat",
        )
        .await?;
    let child = Plot::with_coord(coord).mark(
        Symbol::new()
            .longitude_with(col("lon"), |x| x.with_domain_scope(sharing))
            .latitude_with(col("lat"), |y| y.with_domain_scope(sharing))
            .size(100.0),
    );
    Plot::<FacetColumn>::new()
        .plot_size(260.0, 180.0)
        .data(df)
        .mark(Subplot::new(child).column(col("panel")))
        .compile(ctx)
        .await?
        .evaluate(ctx, None)
        .await
}

fn coordinate_scopes(
    evaluated: &avenger_chart::render::EvaluatedPlot,
) -> Vec<&EvaluatedInteractionScope> {
    evaluated
        .interaction
        .scopes
        .iter()
        .filter(|scope| scope.kind == InteractionScopeKind::Coordinate)
        .collect()
}

fn scope_for_panel<'a>(
    scopes: &'a [&'a EvaluatedInteractionScope],
    panel: &str,
) -> &'a EvaluatedInteractionScope {
    scopes
        .iter()
        .copied()
        .find(|scope| {
            matches!(
                scope.logical_facet_values.first(),
                Some(ScalarValue::Utf8(Some(value))) if value == panel
            )
        })
        .unwrap_or_else(|| panic!("missing facet panel {panel}"))
}

fn scope_for_grid_cell<'a>(
    scopes: &'a [&'a EvaluatedInteractionScope],
    row: usize,
    column: usize,
) -> &'a EvaluatedInteractionScope {
    scopes
        .iter()
        .copied()
        .find(|scope| {
            scope
                .child_frame_path
                .iter()
                .any(|segment| segment.row == Some(row) && segment.column == Some(column))
        })
        .unwrap_or_else(|| panic!("missing repeat grid cell ({row}, {column})"))
}

fn x_domain(scope: &EvaluatedInteractionScope) -> (f64, f64) {
    numeric_domain(scope, "x")
}

fn y_domain(scope: &EvaluatedInteractionScope) -> (f64, f64) {
    numeric_domain(scope, "y")
}

fn numeric_domain(scope: &EvaluatedInteractionScope, channel: &str) -> (f64, f64) {
    let domain = scope
        .scales
        .get(channel)
        .unwrap_or_else(|| panic!("{channel} scale"))
        .numeric_interval_domain()
        .unwrap_or_else(|_| panic!("{channel} domain"));
    (f64::from(domain.0), f64::from(domain.1))
}

fn assert_domains_close(actual: (f64, f64), expected: (f64, f64)) {
    assert_close(actual.0, expected.0);
    assert_close(actual.1, expected.1);
}

fn domain_center(domain: (f64, f64)) -> f64 {
    (domain.0 + domain.1) / 2.0
}

fn assert_close(actual: f64, expected: f64) {
    let tolerance = (expected.abs() * 1e-6).max(0.5);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, got {actual}"
    );
}
