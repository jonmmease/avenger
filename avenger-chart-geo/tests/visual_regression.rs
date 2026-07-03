//! Visual baselines for the Geo coordinate system (scratch/geo phase 2+).
//!
//! Bless with `AVENGER_GEO_BLESS_BASELINES=1 cargo test --release -p
//! avenger-chart-geo --test visual_regression`. Failures write
//! `tests/failures/<name>_{actual,diff}.png`.

use std::path::{Path, PathBuf};

use avenger_chart::plot::Plot;
use avenger_chart_core::CoordinateSystem;
use avenger_chart_geo::{Geo, GraticuleStyle, SphereStyle};
use avenger_common::canvas::CanvasDimensions;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::prelude::SessionContext;
use image::RgbaImage;

const BASELINE_DIR: &str = "tests/baselines";
const FAILURE_DIR: &str = "tests/failures";
const BLESS_ENV: &str = "AVENGER_GEO_BLESS_BASELINES";
const DEFAULT_SCALE: f32 = 2.0;
const DEFAULT_THRESHOLD: f64 = 0.9999;

fn furnished(geo: Geo) -> Geo {
    geo.sphere(SphereStyle::default())
        .graticule(GraticuleStyle::default())
}

fn world_plot(geo: Geo) -> Plot<Geo> {
    Plot::with_coord(furnished(geo)).plot_size(520.0, 320.0)
}

#[tokio::test]
async fn graticule_sphere_equal_earth() {
    assert_visual_match(
        world_plot(Geo::equal_earth()),
        "graticule_sphere_equal_earth",
    )
    .await;
}

#[tokio::test]
async fn graticule_sphere_natural_earth1() {
    assert_visual_match(
        world_plot(Geo::natural_earth()),
        "graticule_sphere_natural_earth1",
    )
    .await;
}

#[tokio::test]
async fn graticule_sphere_winkel_tripel() {
    assert_visual_match(
        world_plot(Geo::winkel_tripel()),
        "graticule_sphere_winkel_tripel",
    )
    .await;
}

#[tokio::test]
async fn graticule_sphere_equirectangular() {
    assert_visual_match(
        world_plot(Geo::equirectangular()),
        "graticule_sphere_equirectangular",
    )
    .await;
}

#[tokio::test]
async fn graticule_sphere_mercator() {
    // Mercator world: the auto world-square clip bounds the poles.
    assert_visual_match(
        Plot::with_coord(furnished(Geo::mercator())).plot_size(360.0, 360.0),
        "graticule_sphere_mercator",
    )
    .await;
}

#[tokio::test]
async fn graticule_albers_conus() {
    // CONUS Albers aspect: centered on Kansas, zoomed so the lower 48 fill
    // the frame. Graticule curvature under the conic is the visual proof
    // the resampler works.
    let geo = Geo::albers_usa_conus()
        .center_projected(0.0031, 0.6410)
        .zoom(3.4);
    assert_visual_match(
        Plot::with_coord(furnished(geo)).plot_size(520.0, 360.0),
        "graticule_albers_conus",
    )
    .await;
}

#[tokio::test]
async fn graticule_conic_conformal_europe() {
    let geo = Geo::conic_conformal((35.0, 65.0))
        .rotate([-15.0, 0.0, 0.0])
        .center_projected(-0.0408, 1.0053)
        .zoom(6.5);
    assert_visual_match(
        Plot::with_coord(furnished(geo)).plot_size(460.0, 400.0),
        "graticule_conic_conformal_europe",
    )
    .await;
}

#[tokio::test]
async fn graticule_rotated_equal_earth() {
    // Oblique aspect: rotation exercises the antimeridian cut and the
    // three-axis rotation together.
    assert_visual_match(
        world_plot(Geo::equal_earth().rotate([15.0, -30.0, 12.0])),
        "graticule_rotated_equal_earth",
    )
    .await;
}

// ---------------------------------------------------------------------------
// Harness (treemap pattern; research/baseline-harness.md §5)
// ---------------------------------------------------------------------------

async fn assert_visual_match<C>(plot: Plot<C>, baseline_name: &str)
where
    C: CoordinateSystem,
{
    let ctx = SessionContext::new();
    let compiled = plot.compile(&ctx).await.expect("compile geo plot");
    let evaluated = compiled
        .evaluate(&ctx, None)
        .await
        .expect("evaluate geo plot");
    let image = render_scene_graph_to_wgpu_image(&evaluated.scene_graph).await;
    let baseline_path = PathBuf::from(BASELINE_DIR).join(format!("{baseline_name}.png"));
    if std::env::var_os(BLESS_ENV).is_some() {
        save_image(&baseline_path, &image);
        return;
    }
    compare_image(&baseline_path, baseline_name, &image, DEFAULT_THRESHOLD);
}

async fn render_scene_graph_to_wgpu_image(scene_graph: &SceneGraph) -> RgbaImage {
    let dimensions = CanvasDimensions {
        size: [scene_graph.width, scene_graph.height],
        scale: DEFAULT_SCALE,
    };
    let mut canvas = PngCanvas::new(dimensions, CanvasConfig::default())
        .await
        .expect("create visual test canvas");
    canvas.set_scene(scene_graph).expect("set scene graph");
    canvas.render().await.expect("render scene graph")
}

fn compare_image(baseline_path: &Path, baseline_name: &str, actual: &RgbaImage, threshold: f64) {
    let actual_path = PathBuf::from(FAILURE_DIR).join(format!("{baseline_name}_actual.png"));
    let diff_path = PathBuf::from(FAILURE_DIR).join(format!("{baseline_name}_diff.png"));

    if !baseline_path.exists() {
        save_image(&actual_path, actual);
        panic!(
            "No baseline at '{}'. Actual saved to '{}'. Bless with {}=1.",
            baseline_path.display(),
            actual_path.display(),
            BLESS_ENV
        );
    }

    let expected = image::open(baseline_path)
        .unwrap_or_else(|err| panic!("failed to load baseline: {err}"))
        .into_rgba8();

    if expected.dimensions() != actual.dimensions() {
        save_image(&actual_path, actual);
        panic!(
            "Dimension mismatch for {baseline_name}: baseline {:?}, actual {:?} (actual saved to {})",
            expected.dimensions(),
            actual.dimensions(),
            actual_path.display()
        );
    }

    let result = image_compare::rgba_hybrid_compare(&expected, actual).expect("image comparison");
    if result.score < threshold {
        save_image(&actual_path, actual);
        save_image(&diff_path, &result.image.to_color_map().into_rgba8());
        panic!(
            "Similarity {:.6} below threshold {threshold:.6} for {baseline_name}. Actual: {}, diff: {}",
            result.score,
            actual_path.display(),
            diff_path.display()
        );
    }
}

fn save_image(path: &Path, image: &RgbaImage) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create baseline directory");
    }
    image.save(path).expect("save image");
}

// ---------------------------------------------------------------------------
// Phase 3: point and line marks
// ---------------------------------------------------------------------------

mod phase3 {
    use super::*;
    use avenger_chart_core::GeometrySpace;
    use avenger_chart_geo::{GeoGeometrySpace, GeoPositionChannels, Line, Symbol};
    use datafusion::prelude::{DataFrame, col, lit};

    fn airports_path() -> String {
        format!(
            "{}/../avenger-chart/tests/data/airports.parquet",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    async fn conus_airports(ctx: &SessionContext) -> DataFrame {
        ctx.read_parquet(airports_path(), Default::default())
            .await
            .expect("read airports")
            .filter(
                col("latitude")
                    .gt_eq(lit(20.0))
                    .and(col("latitude").lt_eq(lit(55.0)))
                    .and(col("longitude").gt_eq(lit(-130.0)))
                    .and(col("longitude").lt_eq(lit(-60.0))),
            )
            .expect("filter airports")
    }

    /// JFK to five destinations; SYD crosses the antimeridian westward.
    async fn routes(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "SELECT * FROM (VALUES
                ('JFK-LHR', 0, -73.78, 40.64), ('JFK-LHR', 1, -0.45, 51.47),
                ('JFK-NRT', 0, -73.78, 40.64), ('JFK-NRT', 1, 140.39, 35.76),
                ('JFK-SYD', 0, -73.78, 40.64), ('JFK-SYD', 1, 151.18, -33.95),
                ('JFK-GRU', 0, -73.78, 40.64), ('JFK-GRU', 1, -46.47, -23.43),
                ('JFK-SIN', 0, -73.78, 40.64), ('JFK-SIN', 1, 103.99, 1.36)
            ) AS t(route, seq, lon, lat)",
        )
        .await
        .expect("route data")
    }

    async fn sfo_syd(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "SELECT * FROM (VALUES
                ('SFO-SYD', 0, -122.38, 37.62), ('SFO-SYD', 1, 151.18, -33.95)
            ) AS t(route, seq, lon, lat)",
        )
        .await
        .expect("route data")
    }

    #[tokio::test]
    async fn symbol_airports_albers_conus() {
        let ctx = SessionContext::new();
        let df = conus_airports(&ctx).await;
        let geo = Geo::albers_usa_conus().graticule(GraticuleStyle::default());
        let plot = Plot::with_coord(geo.clone())
            .plot_size(520.0, 360.0)
            .data(df)
            .mark(
                Symbol::new()
                    .lon_lat(&geo, "longitude", "latitude")
                    .size(6.0)
                    .fill("#1d4ed8")
                    .stroke_width(0.0)
                    .opacity(0.55),
            );
        assert_visual_match_ctx(&ctx, plot, "symbol_airports_albers_conus").await;
    }

    #[tokio::test]
    async fn symbol_airports_equal_earth() {
        let ctx = SessionContext::new();
        let df = ctx
            .read_parquet(airports_path(), Default::default())
            .await
            .expect("read airports");
        let geo = furnished(Geo::equal_earth());
        let plot = Plot::with_coord(geo.clone())
            .plot_size(520.0, 320.0)
            .data(df)
            .mark(
                Symbol::new()
                    .lon_lat(&geo, "longitude", "latitude")
                    .size(4.0)
                    .fill("#b91c1c")
                    .stroke_width(0.0)
                    .opacity(0.5),
            );
        assert_visual_match_ctx(&ctx, plot, "symbol_airports_equal_earth").await;
    }

    #[tokio::test]
    async fn line_great_circles_equal_earth() {
        let ctx = SessionContext::new();
        let df = routes(&ctx).await;
        // World view: great-circle arcs bulge poleward beyond their
        // endpoints' bbox, so fit-to-data would crop them.
        let geo = furnished(Geo::equal_earth().center_projected(0.0, 0.0).zoom(0.95));
        let plot = Plot::with_coord(geo.clone())
            .plot_size(520.0, 320.0)
            .data(df.clone())
            .mark(
                Line::new()
                    .lon_lat(&geo, "lon", "lat")
                    .details(["route"])
                    .order(col("seq"))
                    .stroke(col("route"))
                    .stroke_width(1.6),
            )
            .mark(
                Symbol::new()
                    .lon_lat(&geo, "lon", "lat")
                    .size(24.0)
                    .fill("#111827"),
            );
        assert_visual_match_ctx(&ctx, plot, "line_great_circles_equal_earth").await;
    }

    #[tokio::test]
    async fn line_antimeridian_route_equal_earth() {
        let ctx = SessionContext::new();
        let df = sfo_syd(&ctx).await;
        let geo = furnished(Geo::equal_earth().center_projected(0.0, 0.0).zoom(0.95));
        let plot = Plot::with_coord(geo.clone())
            .plot_size(520.0, 320.0)
            .data(df)
            .mark(
                Line::new()
                    .lon_lat(&geo, "lon", "lat")
                    .order(col("seq"))
                    .stroke("#dc2626")
                    .stroke_width(2.0),
            );
        assert_visual_match_ctx(&ctx, plot, "line_antimeridian_route_equal_earth").await;
    }

    #[tokio::test]
    async fn line_geometry_space_comparison_albers() {
        // Same routes twice: dashed straight display-space lines vs solid
        // great-circle arcs, on the CONUS albers view.
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('SEA-MIA', 0, -122.31, 47.45), ('SEA-MIA', 1, -80.29, 25.79),
                    ('LAX-JFK', 0, -118.41, 33.94), ('LAX-JFK', 1, -73.78, 40.64),
                    ('SFO-BOS', 0, -122.38, 37.62), ('SFO-BOS', 1, -71.01, 42.36)
                ) AS t(route, seq, lon, lat)",
            )
            .await
            .expect("route data");
        let geo = Geo::albers_usa_conus()
            .center_projected(0.0031, 0.6410)
            .zoom(3.4)
            .graticule(GraticuleStyle::default());
        let plot = Plot::with_coord(geo.clone())
            .plot_size(520.0, 360.0)
            .data(df)
            .mark(
                Line::new()
                    .lon_lat(&geo, "lon", "lat")
                    .details(["route"])
                    .order(col("seq"))
                    .geometry_space(GeometrySpace::Display)
                    .stroke("#9ca3af")
                    .stroke_dash("dashed")
                    .stroke_width(1.4),
            )
            .mark(
                Line::new()
                    .lon_lat(&geo, "lon", "lat")
                    .details(["route"])
                    .order(col("seq"))
                    .stroke("#1d4ed8")
                    .stroke_width(1.8),
            )
            .mark(
                Symbol::new()
                    .lon_lat(&geo, "lon", "lat")
                    .size(28.0)
                    .fill("#111827"),
            );
        assert_visual_match_ctx(&ctx, plot, "line_geometry_space_comparison_albers").await;
    }
}

/// Like assert_visual_match but with a caller-provided SessionContext (for
/// plots whose DataFrames were built on it).
async fn assert_visual_match_ctx<C>(ctx: &SessionContext, plot: Plot<C>, baseline_name: &str)
where
    C: CoordinateSystem,
{
    let compiled = plot.compile(ctx).await.expect("compile geo plot");
    let evaluated = compiled
        .evaluate(ctx, None)
        .await
        .expect("evaluate geo plot");
    let image = render_scene_graph_to_wgpu_image(&evaluated.scene_graph).await;
    let baseline_path = PathBuf::from(BASELINE_DIR).join(format!("{baseline_name}.png"));
    if std::env::var_os(BLESS_ENV).is_some() {
        save_image(&baseline_path, &image);
        return;
    }
    compare_image(&baseline_path, baseline_name, &image, DEFAULT_THRESHOLD);
}

// ---------------------------------------------------------------------------
// Phase 4: GeoJSON + GeoShape
// ---------------------------------------------------------------------------

mod phase4 {
    use super::*;
    use avenger_chart::prelude::{Log, ScaleChannelConfig};
    use avenger_chart_geo::{GeoPositionChannels, GeoShape, Line, Symbol, register_geojson};
    use datafusion::prelude::{col, lit};
    use palette::rgb::Srgba;

    fn orange_ramp() -> Vec<Srgba> {
        vec![
            Srgba::new(1.0, 0.96, 0.92, 1.0),
            Srgba::new(0.99, 0.68, 0.42, 1.0),
            Srgba::new(0.85, 0.28, 0.10, 1.0),
            Srgba::new(0.50, 0.14, 0.05, 1.0),
        ]
    }

    fn viridis_ramp() -> Vec<Srgba> {
        vec![
            Srgba::new(0.267, 0.005, 0.329, 1.0),
            Srgba::new(0.188, 0.407, 0.556, 1.0),
            Srgba::new(0.208, 0.719, 0.473, 1.0),
            Srgba::new(0.993, 0.906, 0.144, 1.0),
        ]
    }

    fn geo_data(file: &str) -> String {
        format!(
            "{}/../avenger-chart/tests/data/geo/{file}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    /// THE hero baseline: US states choropleth by population density on the
    /// CONUS Albers aspect (scratch/geo README headline deliverable).
    #[tokio::test]
    async fn choropleth_us_states_albers() {
        let ctx = SessionContext::new();
        let df = register_geojson(&ctx, "us_states", geo_data("us-states.json"))
            .await
            .expect("register us states");
        let geo = Geo::albers_usa_conus().graticule(GraticuleStyle::default());
        let plot = Plot::with_coord(geo.clone())
            .plot_size(560.0, 380.0)
            .title("Population density")
            .data(df)
            .mark(
                GeoShape::new()
                    .geometry(&geo, col("geometry"))
                    .fill_with(col("density"), |c| {
                        c.scale_with::<Log>(|s| s.range_colors(orange_ramp()))
                    })
                    .stroke("#ffffff")
                    .stroke_width(0.6),
            );
        assert_visual_match_ctx(&ctx, plot, "choropleth_us_states_albers").await;
    }

    /// CONUS-only variant: filter by bbox in SQL (geometry is ordinary
    /// data).
    #[tokio::test]
    async fn choropleth_conus_filtered() {
        let ctx = SessionContext::new();
        let df = register_geojson(&ctx, "us_states2", geo_data("us-states.json"))
            .await
            .expect("register us states")
            .filter(
                col("bbox_xmin")
                    .gt_eq(lit(-130.0))
                    .and(col("bbox_xmax").lt_eq(lit(-60.0)))
                    .and(col("bbox_ymin").gt_eq(lit(20.0))),
            )
            .expect("filter conus");
        let geo = Geo::albers_usa_conus();
        let plot = Plot::with_coord(geo.clone())
            .plot_size(560.0, 360.0)
            .data(df)
            .mark(
                GeoShape::new()
                    .geometry(&geo, col("geometry"))
                    .fill_with(col("density"), |c| {
                        c.scale_with::<Log>(|s| s.range_colors(orange_ramp()))
                    })
                    .stroke("#ffffff")
                    .stroke_width(0.6),
            );
        assert_visual_match_ctx(&ctx, plot, "choropleth_conus_filtered").await;
    }

    #[tokio::test]
    async fn choropleth_world_pop_equal_earth() {
        let ctx = SessionContext::new();
        let df = register_geojson(
            &ctx,
            "countries",
            geo_data("ne_110m_admin_0_countries.geojson"),
        )
        .await
        .expect("register countries");
        let geo = furnished(Geo::equal_earth());
        let plot = Plot::with_coord(geo.clone())
            .plot_size(560.0, 340.0)
            .data(df)
            .mark(
                GeoShape::new()
                    .geometry(&geo, col("geometry"))
                    .fill_with(col("pop_est"), |c| {
                        c.scale_with::<Log>(|s| s.range_colors(viridis_ramp()))
                    })
                    .stroke("#334155")
                    .stroke_width(0.3),
            );
        assert_visual_match_ctx(&ctx, plot, "choropleth_world_pop_equal_earth").await;
    }

    #[tokio::test]
    async fn geoshape_world_winkel_tripel() {
        let ctx = SessionContext::new();
        let df = register_geojson(
            &ctx,
            "countries2",
            geo_data("ne_110m_admin_0_countries.geojson"),
        )
        .await
        .expect("register countries");
        let geo = furnished(Geo::winkel_tripel());
        let plot = Plot::with_coord(geo.clone())
            .plot_size(560.0, 340.0)
            .data(df)
            .mark(
                GeoShape::new()
                    .geometry(&geo, col("geometry"))
                    .fill(col("continent"))
                    .stroke("#ffffff")
                    .stroke_width(0.4),
            );
        assert_visual_match_ctx(&ctx, plot, "geoshape_world_winkel_tripel").await;
    }

    /// Antarctica: pole-enclosing polygon renders as a cap, no streaks.
    #[tokio::test]
    async fn geoshape_antarctica_equal_earth() {
        let ctx = SessionContext::new();
        let df = register_geojson(
            &ctx,
            "countries3",
            geo_data("ne_110m_admin_0_countries.geojson"),
        )
        .await
        .expect("register countries")
        .filter(col("name").eq(lit("Antarctica")))
        .expect("filter antarctica");
        // World view so the whole cap and map edge are visible.
        let geo = furnished(Geo::equal_earth().center_projected(0.0, 0.0).zoom(0.95));
        let plot = Plot::with_coord(geo.clone())
            .plot_size(560.0, 340.0)
            .data(df)
            .mark(
                GeoShape::new()
                    .geometry(&geo, col("geometry"))
                    .fill("#94a3b8")
                    .stroke("#334155")
                    .stroke_width(0.5),
            );
        assert_visual_match_ctx(&ctx, plot, "geoshape_antarctica_equal_earth").await;
    }

    /// Fiji + Russia split cleanly at a centered antimeridian.
    #[tokio::test]
    async fn geoshape_antimeridian_fiji_russia() {
        let ctx = SessionContext::new();
        let df = register_geojson(
            &ctx,
            "countries4",
            geo_data("ne_110m_admin_0_countries.geojson"),
        )
        .await
        .expect("register countries")
        .filter(
            col("name")
                .eq(lit("Fiji"))
                .or(col("name").eq(lit("Russia"))),
        )
        .expect("filter");
        // Rotate the antimeridian to center screen.
        let geo = furnished(
            Geo::equirectangular()
                .rotate([-150.0, 0.0, 0.0])
                .center_projected(0.0, 0.0)
                .zoom(0.95),
        );
        let plot = Plot::with_coord(geo.clone())
            .plot_size(560.0, 300.0)
            .data(df)
            .mark(
                GeoShape::new()
                    .geometry(&geo, col("geometry"))
                    .fill("#60a5fa")
                    .stroke("#1e3a8a")
                    .stroke_width(0.5),
            );
        assert_visual_match_ctx(&ctx, plot, "geoshape_antimeridian_fiji_russia").await;
    }

    /// The composed-layers story: land + great-circle routes + airports.
    #[tokio::test]
    async fn layered_land_routes_points() {
        let ctx = SessionContext::new();
        let land = register_geojson(&ctx, "land", geo_data("ne_110m_land.geojson"))
            .await
            .expect("register land");
        let routes = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('JFK-LHR', 0, -73.78, 40.64), ('JFK-LHR', 1, -0.45, 51.47),
                    ('JFK-NRT', 0, -73.78, 40.64), ('JFK-NRT', 1, 140.39, 35.76),
                    ('JFK-GRU', 0, -73.78, 40.64), ('JFK-GRU', 1, -46.47, -23.43),
                    ('JFK-SIN', 0, -73.78, 40.64), ('JFK-SIN', 1, 103.99, 1.36)
                ) AS t(route, seq, lon, lat)",
            )
            .await
            .expect("routes");
        let geo = furnished(Geo::equal_earth().center_projected(0.0, 0.0).zoom(0.95));
        let plot = Plot::with_coord(geo.clone())
            .plot_size(560.0, 340.0)
            .mark(
                GeoShape::new()
                    .data(land)
                    .geometry(&geo, col("geometry"))
                    .fill("#d1d5db")
                    .stroke("#9ca3af")
                    .stroke_width(0.3),
            )
            .mark(
                Line::new()
                    .data(routes.clone())
                    .lon_lat(&geo, "lon", "lat")
                    .details(["route"])
                    .order(col("seq"))
                    .stroke("#dc2626")
                    .stroke_width(1.5),
            )
            .mark(
                Symbol::new()
                    .data(routes)
                    .lon_lat(&geo, "lon", "lat")
                    .size(22.0)
                    .fill("#111827"),
            );
        assert_visual_match_ctx(&ctx, plot, "layered_land_routes_points").await;
    }
}
