//! WebMercator parity suite (scratch/geo phase 7): the
//! avenger-chart-webmercator integration tests ported to `Geo::mercator()`.
//!
//! Domains and viewport params are in the projection's raw planar units
//! (radian-scale, world spans `[-π, π]`) rather than WebMercator meters,
//! so expected values come from `Projection::project_raw_units` and
//! tolerances are scaled accordingly.

use avenger_chart::prelude::Plot;
use avenger_chart::render::{EvaluatedInteractionScope, InteractionScopeKind};
use avenger_chart_geo::Geo;

const RAW_TOLERANCE: f64 = 1e-7; // ≈ 0.6 m on the earth's surface

fn assert_close(actual: f64, expected: f64) {
    let tolerance = (expected.abs() * 1e-6).max(RAW_TOLERANCE);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, got {actual}"
    );
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

fn numeric_domain(scope: &EvaluatedInteractionScope, channel: &str) -> (f64, f64) {
    let domain = scope
        .scales
        .get(channel)
        .unwrap_or_else(|| panic!("{channel} scale"))
        .numeric_interval_domain()
        .unwrap_or_else(|_| panic!("{channel} domain"));
    (f64::from(domain.0), f64::from(domain.1))
}

fn x_domain(scope: &EvaluatedInteractionScope) -> (f64, f64) {
    numeric_domain(scope, "x")
}

fn y_domain(scope: &EvaluatedInteractionScope) -> (f64, f64) {
    numeric_domain(scope, "y")
}

fn assert_domains_close(actual: (f64, f64), expected: (f64, f64)) {
    assert_close(actual.0, expected.0);
    assert_close(actual.1, expected.1);
}

fn project(lon: f64, lat: f64) -> (f64, f64) {
    Geo::mercator().projection().project_raw_units(lon, lat)
}

mod container {
    use avenger_chart::prelude::{
        CoordinationScope, FacetColumn, FacetColumnSubplotChannels, HConcat, RepeatGrid,
        RepeatVariable, ScaleChannelConfig, Subplot, SvgRenderer, col, repeat,
    };
    use avenger_chart_geo::{GeoPositionChannels, Symbol};
    use datafusion::{common::ScalarValue, prelude::SessionContext};

    use super::*;

    #[tokio::test]
    async fn facet_shared_geo_viewport_unions_projected_bounds() {
        let ctx = SessionContext::new();
        let evaluated = facet_geo_plot(&ctx, CoordinationScope::Shared)
            .await
            .expect("evaluate shared Geo facet");
        let scopes = coordinate_scopes(&evaluated);

        assert_eq!(scopes.len(), 2);
        assert_domains_close(x_domain(scopes[0]), x_domain(scopes[1]));
        assert_domains_close(y_domain(scopes[0]), y_domain(scopes[1]));

        let left = project(-1.0, 0.0);
        let right = project(21.0, 0.0);
        let shared_x = x_domain(scopes[0]);
        assert!(shared_x.0 <= left.0 && right.0 <= shared_x.1);
    }

    #[tokio::test]
    async fn facet_shared_geo_fixed_center_infers_zoom_from_all_panels() {
        let ctx = SessionContext::new();
        let evaluated = facet_geo_plot_with_coord(
            &ctx,
            Geo::mercator().center_projected(0.0, 0.0),
            CoordinationScope::Shared,
        )
        .await
        .expect("evaluate fixed-center shared Geo facet");
        let scopes = coordinate_scopes(&evaluated);

        assert_eq!(scopes.len(), 2);
        assert_domains_close(x_domain(scopes[0]), x_domain(scopes[1]));
        assert_domains_close(y_domain(scopes[0]), y_domain(scopes[1]));
        assert_close(domain_center(x_domain(scopes[0])), 0.0);
        assert_close(domain_center(y_domain(scopes[0])), 0.0);

        let left = project(-1.0, 0.0);
        let right = project(21.0, 0.0);
        let shared_x = x_domain(scopes[0]);
        assert!(shared_x.0 <= left.0 && right.0 <= shared_x.1);
    }

    #[tokio::test]
    async fn facet_free_geo_viewports_fit_each_panel_independently() {
        let ctx = SessionContext::new();
        let evaluated = facet_geo_plot(&ctx, CoordinationScope::Free)
            .await
            .expect("evaluate free Geo facet");
        let scopes = coordinate_scopes(&evaluated);

        assert_eq!(scopes.len(), 2);
        let left_scope = scope_for_panel(&scopes, "left");
        let right_scope = scope_for_panel(&scopes, "right");
        let left_x = x_domain(left_scope);
        let right_x = x_domain(right_scope);
        let left_point = project(-1.0, 0.0);
        let right_point = project(21.0, 0.0);

        assert!(left_x.0 <= left_point.0 && left_point.0 <= left_x.1);
        assert!(right_x.0 <= right_point.0 && right_point.0 <= right_x.1);
        assert!(
            right_x.0 > left_x.1,
            "free facet viewports should fit disjoint panels independently: left={left_x:?}, right={right_x:?}"
        );
    }

    #[tokio::test]
    async fn generated_repeat_grid_shared_geo_viewports_use_repeat_domain_groups() {
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT 0.0 AS row_a, 1.0 AS row_b, -1.0 AS near, 19.0 AS far \
                 UNION ALL SELECT 0.0 AS row_a, 1.0 AS row_b, 1.0 AS near, 21.0 AS far",
            )
            .await
            .expect("dataframe");
        let geo = Geo::mercator();
        let cell = Plot::with_coord(geo.clone()).mark(
            Symbol::new()
                .lon_lat(&geo, repeat::column(), repeat::column())
                .size(100.0),
        );
        let evaluated = Plot::<RepeatGrid>::new()
            .plot_size(180.0, 140.0)
            .data(df)
            .configure_coord(|c| {
                c.rows([
                    RepeatVariable::new("row_a", col("row_a")),
                    RepeatVariable::new("row_b", col("row_b")),
                ])
                .columns([
                    RepeatVariable::new("near", col("near")),
                    RepeatVariable::new("far", col("far")),
                ])
                .matrix_domains()
                .cell(cell)
            })
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
    async fn authored_concat_rejects_shared_geo_viewport_domains() {
        let ctx = SessionContext::new();
        let df = ctx
            .sql("SELECT -1.0 AS lon, 0.0 AS lat UNION ALL SELECT 1.0 AS lon, 0.0 AS lat")
            .await
            .expect("dataframe");
        let geo = Geo::mercator();
        let child = || {
            Plot::with_coord(geo.clone()).data(df.clone()).mark(
                Symbol::new()
                    .lon_lat_with(
                        &geo,
                        col("lon"),
                        col("lat"),
                        |x| x.with_domain_scope(CoordinationScope::Shared),
                        |y| y.with_domain_scope(CoordinationScope::Shared),
                    )
                    .size(100.0),
            )
        };
        let plot = Plot::<HConcat>::new()
            .plot_size(260.0, 180.0)
            .mark(Subplot::new(child()).name("left"))
            .mark(Subplot::new(child()).name("right"));
        let compiled = plot.compile(&ctx).await.expect("compile concat");

        let err = SvgRenderer::new()
            .render(&compiled, &ctx, None)
            .await
            .expect_err("authored concat shared Geo domains should be rejected");

        assert!(
            err.to_string()
                .contains("does not support authored concat/grid shared domains"),
            "{err}"
        );
    }

    async fn facet_geo_plot(
        ctx: &SessionContext,
        sharing: CoordinationScope,
    ) -> Result<avenger_chart::render::EvaluatedPlot, avenger_chart::prelude::AvengerChartError>
    {
        facet_geo_plot_with_coord(ctx, Geo::mercator(), sharing).await
    }

    async fn facet_geo_plot_with_coord(
        ctx: &SessionContext,
        coord: Geo,
        sharing: CoordinationScope,
    ) -> Result<avenger_chart::render::EvaluatedPlot, avenger_chart::prelude::AvengerChartError>
    {
        // Unlike the webmercator original, the panels carry a real
        // latitude span: a zero-span y extent picks up the scale
        // machinery's ±1-unit degenerate expansion, which is negligible
        // in meters but spans ~57° in raw radian units and would
        // dominate the fit.
        let df = ctx
            .sql(
                "SELECT 'left' AS panel, -1.0 AS lon, -1.0 AS lat \
                 UNION ALL SELECT 'left' AS panel, 1.0 AS lon, 1.0 AS lat \
                 UNION ALL SELECT 'right' AS panel, 19.0 AS lon, -1.0 AS lat \
                 UNION ALL SELECT 'right' AS panel, 21.0 AS lon, 1.0 AS lat",
            )
            .await?;
        let child = Plot::with_coord(coord.clone()).mark(
            Symbol::new()
                .lon_lat_with(
                    &coord,
                    col("lon"),
                    col("lat"),
                    |x| x.with_domain_scope(sharing),
                    |y| y.with_domain_scope(sharing),
                )
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

    fn domain_center(domain: (f64, f64)) -> f64 {
        (domain.0 + domain.1) / 2.0
    }
}

mod symbol {
    use avenger_chart::prelude::{Rule, Text};
    use avenger_chart_geo::{GeoPositionChannels, Symbol};
    use avenger_scenegraph::marks::{
        mark::SceneMark, rule::SceneRuleMark, symbol::SceneSymbolMark, text::SceneTextMark,
    };
    use datafusion::logical_expr::lit;
    use datafusion::prelude::{SessionContext, col};

    use super::*;

    #[tokio::test]
    async fn symbol_lon_lat_compiles_and_evaluates_through_chart_facade() {
        let ctx = SessionContext::new();
        let df = ctx
            .sql("SELECT -73.9857 AS lon, 40.7484 AS lat")
            .await
            .expect("dataframe");
        let projected = project(-73.9857, 40.7484);

        let geo = Geo::mercator();
        let plot = Plot::with_coord(geo.clone()).data(df).mark(
            Symbol::new()
                .lon_lat(&geo, col("lon"), col("lat"))
                .size(100.0),
        );

        let evaluated = plot
            .compile(&ctx)
            .await
            .expect("compile")
            .evaluate(&ctx, None)
            .await
            .expect("evaluate");

        let scope = coordinate_scopes(&evaluated)[0];
        let x = x_domain(scope);
        let y = y_domain(scope);
        assert!(x.0 <= projected.0 && projected.0 <= x.1);
        assert!(y.0 <= projected.1 && projected.1 <= y.1);
    }

    #[tokio::test]
    async fn symbol_radius_participates_in_geo_fit() {
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
        let geo = Geo::mercator();
        let evaluated = Plot::with_coord(geo.clone())
            .data(df)
            .plot_size(400.0, 400.0)
            .mark(
                Symbol::new()
                    .lon_lat(&geo, col("lon"), col("lat"))
                    .size(size),
            )
            .compile(&ctx)
            .await
            .expect("compile")
            .evaluate(&ctx, None)
            .await
            .expect("evaluate");
        let scope = coordinate_scopes(&evaluated)[0];
        let x = x_domain(scope);
        let y = y_domain(scope);
        (x.1 - x.0, y.1 - y.0)
    }

    #[tokio::test]
    async fn symbol_adjustments_use_rendered_geo_item_frame() {
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

        assert_close_f32(adjusted_x - base_x, 12.0, 1e-4);
    }

    #[tokio::test]
    async fn symbol_can_derive_rule_from_rendered_geo_geometry() {
        let evaluated = evaluated_single_symbol_plot(
            Symbol::new()
                .unit_data()
                .projected_x(0.0)
                .projected_y(0.0)
                .size(100.0)
                .derive(|point| {
                    Rule::<Geo>::new()
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

        assert_close_f32(x2 - x, 18.0, 1e-4);
    }

    #[tokio::test]
    async fn symbol_can_derive_text_from_rendered_geo_geometry_and_source_data() {
        let ctx = SessionContext::new();
        let df = ctx
            .sql("SELECT 0.0 AS lon, 0.0 AS lat, 'origin' AS label")
            .await
            .expect("dataframe");
        let geo = Geo::mercator().center_projected(0.0, 0.0).zoom(2.0);
        let evaluated = Plot::with_coord(geo.clone())
            .plot_size(300.0, 300.0)
            .data(df)
            .mark(
                Symbol::new()
                    .lon_lat(&geo, col("lon"), col("lat"))
                    .size(100.0)
                    .derive(|point| {
                        Text::<Geo>::new()
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

    /// Pre-projected planar input (the EPSG:3857 workflow): position by
    /// raw units directly, no spherical channels involved.
    #[tokio::test]
    async fn pre_projected_inputs_drive_domains_without_projection() {
        let ctx = SessionContext::new();
        // EPSG:3857 meters scaled into raw units by the caller (here the
        // values are already raw units for simplicity).
        let df = ctx
            .sql("SELECT 0.5 AS px, -0.25 AS py UNION ALL SELECT 1.5 AS px, 0.75 AS py")
            .await
            .expect("dataframe");
        let evaluated = Plot::with_coord(Geo::mercator())
            .data(df)
            .plot_size(200.0, 200.0)
            .mark(
                Symbol::new()
                    .projected_x(col("px"))
                    .projected_y(col("py"))
                    .size(64.0),
            )
            .compile(&ctx)
            .await
            .expect("compile")
            .evaluate(&ctx, None)
            .await
            .expect("evaluate");
        let scope = coordinate_scopes(&evaluated)[0];
        let x = x_domain(scope);
        let y = y_domain(scope);
        assert!(x.0 <= 0.5 && 1.5 <= x.1);
        assert!(y.0 <= -0.25 && 0.75 <= y.1);
    }

    async fn rendered_symbol_x(mark: Symbol<Geo>) -> f32 {
        let evaluated = evaluated_single_symbol_plot(mark).await;
        let symbol = first_symbol(&evaluated.scene_graph.marks).expect("symbol");
        symbol.x.as_vec(symbol.len as usize, None)[0]
    }

    async fn evaluated_single_symbol_plot(
        mark: Symbol<Geo>,
    ) -> avenger_chart::render::EvaluatedPlot {
        let ctx = SessionContext::new();
        Plot::with_coord(Geo::mercator().center_projected(0.0, 0.0).zoom(2.0))
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

    fn assert_close_f32(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {actual} to be within {tolerance} of {expected}"
        );
    }
}

mod tile_guide {
    use avenger_chart::{
        doc::render::render_evaluated_plot_to_png,
        layout::Margins,
        render::{EvaluatedPlot, PdfRenderer, SvgRenderer},
    };
    use avenger_chart_geo::{RasterTileLayer, TileLoadingPolicy};
    use avenger_resource::{ResourceKey, ResourceRequestPurpose, ResourceSource};
    use avenger_scenegraph::marks::{
        group::{Clip, SceneGroup},
        image::{SceneImageMark, SceneImageSource, SceneImageUnavailablePolicy},
        mark::SceneMark,
        text::SceneTextMark,
    };
    use datafusion::prelude::SessionContext;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    const TINY_PNG_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAG0lEQVR4nGO4o6b2XzX59X8GscVe/3+dEf0PAE8fCXZKLiUkAAAAAElFTkSuQmCC";

    #[tokio::test]
    async fn tile_guide_requests_resources_and_renders_image_marks() {
        let evaluated = evaluated_tile_plot().await;

        assert_eq!(evaluated.resource_requests.len(), 1);
        let request = &evaluated.resource_requests[0];
        assert_eq!(request.key, ResourceKey::new("geo/base/0/0/0/256"));
        assert!(matches!(request.source, ResourceSource::DataUri { .. }));

        let image_marks = collect_image_marks(evaluated.scene_graph.children());
        assert!(!image_marks.is_empty());
        for image_mark in image_marks {
            assert!(!image_mark.interactive);
            assert_eq!(image_mark.zindex, Some(-4));
            let image = image_mark.image_source_iter().next().expect("image source");
            let SceneImageSource::Resource(resource) = image else {
                panic!("expected resource-backed tile image");
            };
            assert_eq!(resource.key, ResourceKey::new("geo/base/0/0/0/256"));
        }

        let text_marks = collect_text_marks(evaluated.scene_graph.children());
        assert_eq!(text_marks.len(), 1);
        assert_eq!(text_marks[0].text_iter().next().unwrap(), "Example tiles");
    }

    #[tokio::test]
    async fn tile_guide_honors_plot_area_origin_and_clip() {
        let ctx = SessionContext::new();
        let coord = Geo::mercator().center_lon_lat(0.0, 0.0).zoom(0.0).tiles(
            RasterTileLayer::xyz(TINY_PNG_DATA_URI)
                .id("base")
                .max_zoom(0),
        );

        let evaluated = Plot::with_coord(coord)
            .canvas_size(300.0, 260.0)
            .margins(Margins::uniform(20.0))
            .compile(&ctx)
            .await
            .expect("compile")
            .evaluate(&ctx, None)
            .await
            .expect("evaluate");

        let guide_group =
            find_group(evaluated.scene_graph.children(), "geo-guide").expect("guide group");
        assert_eq!(guide_group.origin, [20.0, 20.0]);
        assert_eq!(
            guide_group.clip,
            Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: 260.0,
                height: 220.0,
            }
        );

        let data_group =
            first_root_child_group(evaluated.scene_graph.children()).expect("data group");
        assert_eq!(data_group.origin, [20.0, 20.0]);
        assert_eq!(
            data_group.clip,
            Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: 260.0,
                height: 220.0,
            }
        );
    }

    #[tokio::test]
    async fn tile_resources_render_to_svg_pdf_and_png_exports() {
        let evaluated = evaluated_tile_plot().await;

        let svg = SvgRenderer::new()
            .render_evaluated_plot(&evaluated)
            .expect("render svg");
        assert!(svg.contains("data:image/png;base64"));

        let pdf = PdfRenderer::new()
            .render_evaluated_plot(&evaluated)
            .expect("render pdf");
        assert!(pdf.starts_with(b"%PDF-"));

        let png_path = unique_png_path();
        render_evaluated_plot_to_png(&evaluated, &png_path)
            .await
            .expect("render png");
        let metadata = std::fs::metadata(&png_path).expect("png metadata");
        assert!(metadata.len() > 0);
        let _ = std::fs::remove_file(png_path);
    }

    #[tokio::test]
    async fn smooth_zoom_tile_guide_renders_fallback_marks_and_prefetch_requests() {
        let evaluated = evaluated_smooth_tile_plot().await;
        let image_marks = collect_image_marks(evaluated.scene_graph.children());
        assert!(
            image_marks
                .iter()
                .all(|mark| mark.unavailable_policy == SceneImageUnavailablePolicy::Skip),
            "smooth tile marks should skip pending images instead of drawing placeholders"
        );

        let rendered_keys = image_marks
            .iter()
            .filter_map(|mark| {
                mark.image_source_iter().find_map(|source| match source {
                    SceneImageSource::Resource(resource) => Some(resource.key.clone()),
                    _ => None,
                })
            })
            .collect::<Vec<_>>();
        let prefetch_requests = evaluated
            .resource_requests
            .iter()
            .filter(|request| request.purpose == ResourceRequestPurpose::Prefetch)
            .collect::<Vec<_>>();
        let required_keys = evaluated
            .resource_requests
            .iter()
            .filter(|request| request.purpose == ResourceRequestPurpose::Required)
            .map(|request| request.key.clone())
            .collect::<Vec<_>>();

        assert!(!prefetch_requests.is_empty());
        // Target tiles are the only Required fetches; prefetch never
        // duplicates them.
        assert!(
            prefetch_requests
                .iter()
                .all(|request| !required_keys.contains(&request.key))
        );
        // Fallback-zoom tiles are rendered WITHOUT being Required — their
        // pixels arrive via the overlapping prefetch covers (cache-only
        // fallback rendering).
        assert!(
            rendered_keys.iter().any(|key| !required_keys.contains(key)),
            "expected rendered fallback tiles beyond the Required targets"
        );
        assert!(
            prefetch_requests
                .iter()
                .any(|request| rendered_keys.contains(&request.key)),
            "expected prefetch to overlap rendered fallback tiles"
        );
        assert!(
            prefetch_requests
                .iter()
                .any(|request| tile_key_zoom(&request.key) == 2),
            "expected smooth tile guide to prefetch at the target zoom for short pan gestures"
        );
        assert!(
            evaluated
                .resource_requests
                .iter()
                .any(|request| request.purpose == ResourceRequestPurpose::Required)
        );
    }

    async fn evaluated_tile_plot() -> EvaluatedPlot {
        let ctx = SessionContext::new();
        let coord = Geo::mercator().center_lon_lat(0.0, 0.0).zoom(0.0).tiles(
            RasterTileLayer::xyz(TINY_PNG_DATA_URI)
                .id("base")
                .max_zoom(0)
                .attribution("Example tiles"),
        );

        Plot::with_coord(coord)
            .compile(&ctx)
            .await
            .expect("compile")
            .evaluate(&ctx, None)
            .await
            .expect("evaluate")
    }

    async fn evaluated_smooth_tile_plot() -> EvaluatedPlot {
        let ctx = SessionContext::new();
        let coord = Geo::mercator().center_lon_lat(0.0, 0.0).zoom(2.0).tiles(
            RasterTileLayer::xyz(TINY_PNG_DATA_URI)
                .id("base")
                .max_zoom(3)
                .loading_policy(TileLoadingPolicy::SmoothZoom {
                    fallback_below: 1,
                    fallback_above: 0,
                    prefetch_below: 1,
                    prefetch_above: 1,
                    pan_prefetch_margin_tiles: 1,
                    prefetch_coarse_delta: None,
                    max_rendered_fallback_tiles: 128,
                    max_prefetch_tiles: 128,
                }),
        );

        Plot::with_coord(coord)
            .plot_size(256.0, 256.0)
            .compile(&ctx)
            .await
            .expect("compile")
            .evaluate(&ctx, None)
            .await
            .expect("evaluate")
    }

    fn unique_png_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time since epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("avenger-geo-tile-{nanos}.png"))
    }

    fn tile_key_zoom(key: &ResourceKey) -> u8 {
        key.0
            .split('/')
            .nth(2)
            .expect("tile key zoom")
            .parse()
            .expect("tile key zoom number")
    }

    fn collect_image_marks(marks: &[SceneMark]) -> Vec<&SceneImageMark> {
        let mut out = Vec::new();
        collect_image_marks_inner(marks, &mut out);
        out
    }

    fn collect_image_marks_inner<'a>(marks: &'a [SceneMark], out: &mut Vec<&'a SceneImageMark>) {
        for mark in marks {
            match mark {
                SceneMark::Image(image) => out.push(image),
                SceneMark::Group(group) => collect_image_marks_inner(&group.marks, out),
                _ => {}
            }
        }
    }

    fn collect_text_marks(marks: &[SceneMark]) -> Vec<&SceneTextMark> {
        let mut out = Vec::new();
        collect_text_marks_inner(marks, &mut out);
        out
    }

    fn collect_text_marks_inner<'a>(marks: &'a [SceneMark], out: &mut Vec<&'a SceneTextMark>) {
        for mark in marks {
            match mark {
                SceneMark::Text(text) => out.push(text),
                SceneMark::Group(group) => collect_text_marks_inner(&group.marks, out),
                _ => {}
            }
        }
    }

    fn find_group<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneGroup> {
        for mark in marks {
            let SceneMark::Group(group) = mark else {
                continue;
            };
            if group.name == name {
                return Some(group);
            }
            if let Some(group) = find_group(&group.marks, name) {
                return Some(group);
            }
        }
        None
    }

    fn first_root_child_group(marks: &[SceneMark]) -> Option<&SceneGroup> {
        let SceneMark::Group(root) = marks.first()? else {
            return None;
        };
        root.marks.iter().find_map(|mark| match mark {
            SceneMark::Group(group) if group.name != "geo-guide" => Some(group),
            _ => None,
        })
    }
}

mod tool {
    use avenger_chart_geo::GeoPanZoom;
    use datafusion::common::ScalarValue;
    use datafusion::prelude::SessionContext;
    use indexmap::IndexMap;

    use super::*;

    #[tokio::test]
    async fn pan_zoom_tool_registers_viewport_params_and_bindings() {
        let ctx = SessionContext::new();
        let compiled = Plot::with_coord(Geo::mercator().viewport_id("main"))
            .tool(GeoPanZoom::new().viewport_id("main"))
            .compile(&ctx)
            .await
            .expect("compile");

        for param in [
            "__tool_geo_pan_zoom__enabled",
            "__geo_main_center_x",
            "__geo_main_center_y",
            "__geo_main_units_per_pixel",
            "__tool_geo_pan_zoom__box_active",
            "__tool_geo_pan_zoom__box_x0",
        ] {
            assert!(
                compiled.param_specs().contains_key(param),
                "missing param {param}"
            );
        }
        assert_eq!(compiled.event_bindings().len(), 7);
        assert!(compiled.tool_metadata().iter().any(|meta| {
            meta.id == "geo_pan_zoom"
                && meta.enabled_param.as_deref() == Some("__tool_geo_pan_zoom__enabled")
        }));
    }

    #[tokio::test]
    async fn viewport_params_drive_evaluated_domains() {
        let ctx = SessionContext::new();
        let compiled = Plot::with_coord(Geo::mercator().viewport_id("main"))
            .tool(GeoPanZoom::new().viewport_id("main"))
            .compile(&ctx)
            .await
            .expect("compile");
        let mut params = IndexMap::new();
        params.insert(
            "__geo_main_center_x".to_string(),
            ScalarValue::Float64(Some(1.0)),
        );
        params.insert(
            "__geo_main_center_y".to_string(),
            ScalarValue::Float64(Some(-0.5)),
        );
        params.insert(
            "__geo_main_units_per_pixel".to_string(),
            ScalarValue::Float64(Some(0.002)),
        );

        let evaluated = compiled
            .evaluate(&ctx, Some(params))
            .await
            .expect("evaluate");
        let scope = coordinate_scopes(&evaluated)[0];
        let x = x_domain(scope);
        let y = y_domain(scope);
        let half_width = 0.002 * f64::from(scope.plot_area_width) / 2.0;
        let half_height = 0.002 * f64::from(scope.plot_area_height) / 2.0;

        assert_close(x.0, 1.0 - half_width);
        assert_close(x.1, 1.0 + half_width);
        assert_close(y.0, -0.5 - half_height);
        assert_close(y.1, -0.5 + half_height);
    }

    /// The interaction frame is identity (None) without a blend and a
    /// ~15° rotation for a fully blended Albers view over California.
    #[tokio::test]
    async fn interaction_frame_reports_blend_rotation() {
        use avenger_chart_core::{
            CoordinateMeasureRequest, CoordinateMeasurementProvider, CoordinateSystemTransformCore,
        };
        use avenger_chart_geo::{BlendConfig, GeoCoordMeasurement};

        async fn measurement_for(geo: Geo) -> avenger_chart_geo::GeoCoordMeasurement {
            let ctx = SessionContext::new();
            let measurement = geo
                .measure_coordinate(CoordinateMeasureRequest {
                    plot_width: 500.0,
                    plot_height: 420.0,
                    params: &Default::default(),
                    session_context: &ctx,
                    data: None,
                    compiled_marks: &[],
                    facet_path: &[],
                    scales: Default::default(),
                })
                .await
                .expect("measure")
                .expect("geo measurement");
            GeoCoordMeasurement::downcast(measurement.as_ref())
                .expect("downcast")
                .clone()
        }

        // No blend: identity frame.
        let unblended = measurement_for(
            Geo::albers_usa_conus()
                .center_lon_lat(-121.5, 38.0)
                .zoom(6.5),
        )
        .await;
        assert!(unblended.interaction_frame().is_none());

        // Fully blended: the frame maps display deltas (north-up plane)
        // into authored Albers deltas — a rotation by the meridian
        // convergence at the anchor (~15.4° at 121.5°W).
        let blended = measurement_for(
            Geo::albers_usa_conus()
                .center_lon_lat(-121.5, 38.0)
                .zoom(6.5)
                .adaptive_blend(BlendConfig {
                    force_t: Some(1.0),
                    ..Default::default()
                }),
        )
        .await;
        let frame = blended.interaction_frame().expect("active frame");
        // Near-similarity: the determinant is the authored/displayed area
        // ratio at the anchor — close to 1 but carrying the small
        // equal-area-vs-conformal anisotropy.
        let det = frame[0] * frame[3] - frame[1] * frame[2];
        assert!((det - 1.0).abs() < 0.05, "det {det}");
        let angle = frame[2].atan2(frame[0]).to_degrees();
        assert!(
            (angle.abs() - 15.37).abs() < 0.5,
            "expected ~15.4° of frame rotation, got {angle}"
        );

        // The trait entry point reconstructs the same frame from scales.
        let linear_scale = |domain: (f32, f32), range: (f32, f32)| {
            use avenger_scales::scales::{ConfiguredScale, ScaleConfig, linear::LinearScale};
            ConfiguredScale {
                scale_impl: std::sync::Arc::new(LinearScale),
                config: ScaleConfig::empty(),
            }
            .with_domain_interval(domain)
            .with_range_interval(range)
        };
        let scales = {
            let mut scales = std::collections::HashMap::new();
            let view = blended.view;
            scales.insert(
                "x".to_string(),
                linear_scale(
                    (view.x_domain.0 as f32, view.x_domain.1 as f32),
                    (0.0, 500.0),
                ),
            );
            scales.insert(
                "y".to_string(),
                linear_scale(
                    (view.y_domain.0 as f32, view.y_domain.1 as f32),
                    (420.0, 0.0),
                ),
            );
            scales
        };
        let geo = Geo::albers_usa_conus()
            .center_lon_lat(-121.5, 38.0)
            .zoom(6.5)
            .adaptive_blend(BlendConfig {
                force_t: Some(1.0),
                ..Default::default()
            });
        let via_trait = geo
            .interaction_frame(&scales, 500.0, 420.0)
            .expect("trait frame");
        for (a, b) in frame.iter().zip(via_trait.iter()) {
            assert!((a - b).abs() < 1e-6, "trait frame mismatch: {a} vs {b}");
        }
    }

    /// Interaction inversion (the tooltip path): the measurement inverts
    /// plot pixels back to lon/lat.
    #[tokio::test]
    async fn measurement_inverts_pixels_to_lon_lat() {
        use avenger_chart_core::{CoordinateMeasureRequest, CoordinateMeasurementProvider};

        let ctx = SessionContext::new();
        let geo = Geo::mercator().center_lon_lat(10.0, 20.0).zoom(3.0);
        let measurement = geo
            .measure_coordinate(CoordinateMeasureRequest {
                plot_width: 400.0,
                plot_height: 300.0,
                params: &Default::default(),
                session_context: &ctx,
                data: None,
                compiled_marks: &[],
                facet_path: &[],
                scales: Default::default(),
            })
            .await
            .expect("measure")
            .expect("geo measurement");
        let measurement = avenger_chart_geo::GeoCoordMeasurement::downcast(measurement.as_ref())
            .expect("downcast");

        // The plot center inverts to the authored view center.
        let (lon, lat) = measurement
            .invert_pixel(200.0, 150.0)
            .expect("invert center");
        assert!((lon - 10.0).abs() < 1e-6, "lon {lon}");
        assert!((lat - 20.0).abs() < 1e-6, "lat {lat}");

        // Round trip an off-center pixel through the forward projector.
        let projector = measurement.view_projector();
        let (px, py) = projector.project(12.0, 18.0).expect("project");
        let (lon, lat) = measurement
            .invert_pixel(px as f32, py as f32)
            .expect("invert off-center");
        assert!((lon - 12.0).abs() < 1e-5, "lon {lon}");
        assert!((lat - 18.0).abs() < 1e-5, "lat {lat}");
    }
}

mod tool_app {
    use avenger_app::app::AvengerApp;
    use avenger_chart_app::{
        ChartAppOptions, ChartAppState, ChartRuntimeResources, chart_avenger_app,
        chart_avenger_app_with_runtime_resources,
    };
    use avenger_chart_geo::{GeoPanZoom, RasterTileLayer};
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

    use super::*;

    const VIEWPORT_ID: &str = "main";
    const CENTER_X_PARAM: &str = "__geo_main_center_x";
    const CENTER_Y_PARAM: &str = "__geo_main_center_y";
    const UNITS_PER_PIXEL_PARAM: &str = "__geo_main_units_per_pixel";
    const BOX_ACTIVE_PARAM: &str = "__tool_geo_pan_zoom__box_active";
    const BOX_X0_PARAM: &str = "__tool_geo_pan_zoom__box_x0";
    const BOX_X1_PARAM: &str = "__tool_geo_pan_zoom__box_x1";

    #[tokio::test]
    async fn pan_drag_updates_geo_viewport_params() {
        let (mut app, state) = app_with_geo_pan_zoom().await;
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
        let (mut app, state) = app_with_geo_pan_zoom().await;
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
    async fn double_click_resets_geo_viewport_params() {
        let (mut app, state) = app_with_geo_pan_zoom().await;
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
        let (mut app, state) = app_with_geo_pan_zoom().await;
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
        let (mut app, state) = app_with_geo_pan_zoom().await;
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
        let resources =
            ChartRuntimeResources::new(resolver.clone(), RenderInvalidationHub::default());
        let coord = Geo::mercator()
            .viewport_id(VIEWPORT_ID)
            .center_projected(0.0, 0.0)
            .zoom(1.0)
            .tiles(
                RasterTileLayer::xyz("https://example.com/tiles/{z}/{x}/{y}.png")
                    .max_zoom(1)
                    .attribution("Example"),
            );
        let (mut app, state) = app_with_geo_pan_zoom_and_resources(coord, resources).await;
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
    async fn pan_drag_repositions_geo_tiles_during_preview() {
        let resolver = Arc::new(PendingImageResolver::default());
        let resources =
            ChartRuntimeResources::new(resolver.clone(), RenderInvalidationHub::default());
        let coord = tiled_geo_coord();
        let (mut app, state) = app_with_geo_pan_zoom_and_resources(coord, resources).await;
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
    async fn wheel_zoom_repositions_geo_tiles_during_preview() {
        let resolver = Arc::new(PendingImageResolver::default());
        let resources =
            ChartRuntimeResources::new(resolver.clone(), RenderInvalidationHub::default());
        let coord = tiled_geo_coord();
        let (mut app, state) = app_with_geo_pan_zoom_and_resources(coord, resources).await;
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

    /// Pan under a fully-blended (north-up) Albers view: the drag must
    /// move the view to the geography that was under the dragged-from
    /// point, i.e. gesture deltas are mapped through the rotated frame
    /// rather than applied along authored axes.
    #[tokio::test]
    async fn pan_drag_tracks_geography_under_active_blend() {
        use avenger_chart_core::{CoordinateMeasureRequest, CoordinateMeasurementProvider};
        use avenger_chart_geo::{BlendConfig, GeoCoordMeasurement};

        let geo = || {
            Geo::albers_usa_conus()
                .viewport_id(VIEWPORT_ID)
                .center_lon_lat(-121.5, 38.0)
                .zoom(6.5)
                .adaptive_blend(BlendConfig {
                    force_t: Some(1.0),
                    ..Default::default()
                })
        };
        let (mut app, state) = app_with_geo_pan_zoom_for_coord(geo()).await;
        let scope = coordinate_scope(&state).await;
        let center = scope_center(&scope);
        let start = Instant::now();

        // Ground truth: the geography currently displayed 40px left and
        // 20px below the plot center (screen coords), via the blended
        // inverse of the same measurement the app realized.
        let ctx = SessionContext::new();
        let measurement = geo()
            .measure_coordinate(CoordinateMeasureRequest {
                plot_width: scope.plot_area_width,
                plot_height: scope.plot_area_height,
                params: &Default::default(),
                session_context: &ctx,
                data: None,
                compiled_marks: &[],
                facet_path: &[],
                scales: Default::default(),
            })
            .await
            .expect("measure")
            .expect("geo measurement");
        let measurement = GeoCoordMeasurement::downcast(measurement.as_ref()).expect("downcast");
        let (lon, lat) = measurement
            .invert_pixel(
                scope.plot_area_width / 2.0 - 40.0,
                scope.plot_area_height / 2.0 + 20.0,
            )
            .expect("invert drag origin");
        let expected = measurement.projection.project_raw_units(lon, lat);

        // Drag right 40px and up 20px.
        dispatch_cursor(&mut app, center, start).await;
        dispatch_left_mouse(&mut app, ElementState::Pressed, start).await;
        dispatch_cursor(
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

        let actual = (
            param_f64(&state, CENTER_X_PARAM),
            param_f64(&state, CENTER_Y_PARAM),
        );
        // First-order frame accuracy over a 45px displacement: allow a
        // small fraction of the drag distance in authored units.
        let upp = units_per_pixel(&coordinate_scope(&state).await);
        let tolerance = 45.0 * upp * 0.02;
        assert!(
            (actual.0 - expected.0).abs() < tolerance && (actual.1 - expected.1).abs() < tolerance,
            "panned center {actual:?} should track geography at {expected:?} (tolerance {tolerance})"
        );
        // And the naive axis-aligned update would have been ~15° off:
        // ensure we are meaningfully closer than that error would allow.
        let naive_error = (40.0_f64.hypot(20.0)) * upp * (15.0_f64.to_radians().sin());
        assert!(
            (actual.0 - expected.0).hypot(actual.1 - expected.1) < naive_error / 3.0,
            "frame correction should beat the naive update"
        );
    }

    async fn app_with_geo_pan_zoom() -> (AvengerApp<ChartAppState>, ChartAppState) {
        let coord = Geo::mercator()
            .viewport_id(VIEWPORT_ID)
            .center_projected(0.0, 0.0)
            .zoom(1.0);
        app_with_geo_pan_zoom_for_coord(coord).await
    }

    async fn app_with_geo_pan_zoom_for_coord(
        coord: Geo,
    ) -> (AvengerApp<ChartAppState>, ChartAppState) {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Plot::with_coord(coord)
            .plot_size(400.0, 200.0)
            .tool(GeoPanZoom::new().viewport_id(VIEWPORT_ID))
            .compile(ctx.as_ref())
            .await
            .expect("compile Geo pan/zoom plot");
        let mut app = chart_avenger_app(compiled, ctx, ChartAppOptions::default())
            .await
            .expect("create chart app");
        let state = app.app_state_mut().clone();
        (app, state)
    }

    async fn app_with_geo_pan_zoom_and_resources(
        coord: Geo,
        resources: ChartRuntimeResources,
    ) -> (AvengerApp<ChartAppState>, ChartAppState) {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Plot::with_coord(coord)
            .plot_size(400.0, 200.0)
            .tool(GeoPanZoom::new().viewport_id(VIEWPORT_ID))
            .compile(ctx.as_ref())
            .await
            .expect("compile Geo pan/zoom plot");
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

    fn tiled_geo_coord() -> Geo {
        Geo::mercator()
            .viewport_id(VIEWPORT_ID)
            .center_projected(0.0, 0.0)
            .zoom(1.0)
            .tiles(
                RasterTileLayer::xyz("https://example.com/tiles/{z}/{x}/{y}.png")
                    .max_zoom(1)
                    .attribution("Example"),
            )
    }

    async fn coordinate_scope(state: &ChartAppState) -> EvaluatedInteractionScope {
        state
            .interaction_scopes()
            .await
            .into_iter()
            .find(|scope| scope.kind == InteractionScopeKind::Coordinate)
            .expect("coordinate interaction scope")
    }

    fn scope_center(scope: &EvaluatedInteractionScope) -> [f32; 2] {
        [
            scope.bounds.x + scope.bounds.width / 2.0,
            scope.bounds.y + scope.bounds.height / 2.0,
        ]
    }

    fn units_per_pixel(scope: &EvaluatedInteractionScope) -> f64 {
        let x = x_domain(scope);
        (x.1 - x.0) / f64::from(scope.plot_area_width)
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
        if !image.name.starts_with("geo-tile-") {
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

    fn first_f32(
        values: &avenger_common::value::ScalarOrArray<f32>,
        len: u32,
        channel: &str,
    ) -> f32 {
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
}

/// Phase-7 performance record: Geo(mercator) rendering + pan/zoom of a
/// 100k-point scatter. Before avenger-chart-webmercator was retired this
/// benchmark compared both coordinate systems head to head; the recorded
/// result (2026-07-03, M-series laptop, release): build ratio 1.030,
/// pan ratio 0.983, wheel ratio 0.987 — within the 10% parity gate.
/// Ignored by default; run explicitly to track the geo numbers:
/// `cargo test --release -p avenger-chart-geo --test parity benchmark -- --ignored --nocapture`
mod benchmark {
    use std::sync::Arc;
    use std::time::Instant as StdInstant;

    use avenger_app::app::AvengerApp;
    use avenger_chart_app::{ChartAppOptions, ChartAppState, chart_avenger_app};
    use avenger_chart_geo::{GeoPanZoom, GeoPositionChannels, Symbol};
    use avenger_common::time::{Duration, Instant};
    use avenger_eventstream::window::{
        ElementState, MouseButton, MouseScrollDelta, WindowCursorMoved, WindowEvent,
        WindowMouseInput, WindowMouseWheel,
    };
    use datafusion::arrow::array::Float64Array;
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::record_batch::RecordBatch;
    use datafusion::prelude::{SessionContext, col};

    use super::*;

    const POINTS: usize = 100_000;

    fn taxi_like_points(ctx: &SessionContext) {
        // Deterministic LCG scatter around NYC (no rand dependency).
        let mut state = 0x2545F4914F6CDD1D_u64;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 11) as f64 / (1_u64 << 53) as f64
        };
        let mut lons = Vec::with_capacity(POINTS);
        let mut lats = Vec::with_capacity(POINTS);
        for _ in 0..POINTS {
            lons.push(-74.3 + 0.6 * next());
            lats.push(40.4 + 0.6 * next());
        }
        let schema = Arc::new(Schema::new(vec![
            Field::new("lon", DataType::Float64, false),
            Field::new("lat", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Float64Array::from(lons)),
                Arc::new(Float64Array::from(lats)),
            ],
        )
        .expect("record batch");
        ctx.register_batch("points", batch)
            .expect("register points");
    }

    struct BenchTimes {
        build_ms: f64,
        pan_ms: f64,
        wheel_ms: f64,
    }

    async fn run_gestures(app: &mut AvengerApp<ChartAppState>) -> (f64, f64) {
        let start = Instant::now();
        let center = [200.0_f32, 200.0_f32];
        let mut when = start;
        let step = Duration::from_millis(16);

        let pan_start = StdInstant::now();
        app.update_with_status(
            &WindowEvent::CursorMoved(WindowCursorMoved { position: center }),
            when,
        )
        .await
        .expect("cursor");
        when += step;
        app.update_with_status(
            &WindowEvent::MouseInput(WindowMouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
            }),
            when,
        )
        .await
        .expect("press");
        for i in 0..20 {
            when += step;
            let dx = ((i % 5) as f32 - 2.0) * 12.0;
            app.update_with_status(
                &WindowEvent::CursorMoved(WindowCursorMoved {
                    position: [center[0] + dx, center[1] - dx / 2.0],
                }),
                when,
            )
            .await
            .expect("drag");
        }
        when += step;
        app.update_with_status(
            &WindowEvent::MouseInput(WindowMouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
            }),
            when,
        )
        .await
        .expect("release");
        let pan_ms = pan_start.elapsed().as_secs_f64() * 1000.0;

        let wheel_start = StdInstant::now();
        for i in 0..20 {
            when += step;
            let direction = if i % 2 == 0 { 1.0 } else { -1.0 };
            app.update_with_status(
                &WindowEvent::MouseWheel(WindowMouseWheel {
                    delta: MouseScrollDelta::LineDelta(0.0, direction),
                }),
                when,
            )
            .await
            .expect("wheel");
        }
        let wheel_ms = wheel_start.elapsed().as_secs_f64() * 1000.0;
        (pan_ms, wheel_ms)
    }

    async fn bench_geo() -> BenchTimes {
        let ctx = Arc::new(SessionContext::new());
        taxi_like_points(&ctx);
        let geo = Geo::mercator().viewport_id("bench");
        let build_start = StdInstant::now();
        let compiled = Plot::with_coord(geo.clone())
            .plot_size(400.0, 400.0)
            .data(ctx.table("points").await.expect("points table"))
            .mark(
                Symbol::new()
                    .lon_lat(&geo, col("lon"), col("lat"))
                    .size(4.0),
            )
            .tool(GeoPanZoom::new().viewport_id("bench"))
            .compile(ctx.as_ref())
            .await
            .expect("compile");
        let mut app = chart_avenger_app(compiled, ctx, ChartAppOptions::default())
            .await
            .expect("app");
        let build_ms = build_start.elapsed().as_secs_f64() * 1000.0;
        let (pan_ms, wheel_ms) = run_gestures(&mut app).await;
        BenchTimes {
            build_ms,
            pan_ms,
            wheel_ms,
        }
    }

    #[ignore = "benchmark; run explicitly with --ignored --nocapture"]
    #[tokio::test]
    async fn benchmark_100k_points_geo_mercator() {
        let _ = bench_geo().await; // warm-up
        let geo = bench_geo().await;
        println!(
            "geo: build {:8.1} ms   pan(21 ev) {:8.1} ms   wheel(20 ev) {:8.1} ms",
            geo.build_ms, geo.pan_ms, geo.wheel_ms
        );
    }
}
