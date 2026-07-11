//! `Geo::mercator()` visual suite (scratch/geo phase 7): the
//! test_webmercator.rs scenes ported to the Geo coordinate system.
//!
//! Plot construction (including title strings) intentionally matches the
//! WebMercator originals so `ported_baselines_match_webmercator_originals`
//! can gate every baseline against the retired coordinate system's image
//! at ≥ 0.9999 similarity.

use std::{collections::HashMap, sync::Arc};

use super::helpers::{
    assert_visual_match_default, assert_visual_match_wgpu_only_with_canvas_config,
    assert_visual_match_with_image_resolver,
};
use avenger_chart::plot::CompiledPlot;
use avenger_chart::prelude::*;
use avenger_chart_geo::{Geo, GeoPositionChannels, Symbol, TileLoadingPolicy};
use avenger_image::{ImageResourceResolver, ImageResourceState, RgbaImage as AvengerRgbaImage};
use avenger_resource::ResourceKey;
use avenger_wgpu::{
    canvas::CanvasConfig,
    image_resources::{WgpuImagePlaceholder, WgpuImageResourceConfig, WgpuMissingImagePolicy},
};
use datafusion::prelude::{SessionContext, col};

const CATEGORY: &str = "geo_mercator";

async fn landmarks(ctx: &SessionContext) -> datafusion::prelude::DataFrame {
    ctx.sql(
        "SELECT * FROM (VALUES
            (-73.9857, 40.7484, '#ef4444', 180.0),
            (-73.9772, 40.7527, '#2563eb', 130.0),
            (-73.9680, 40.7851, '#16a34a', 150.0),
            (-74.0445, 40.6892, '#f59e0b', 170.0)
        ) AS t(lon, lat, color, size)",
    )
    .await
    .expect("landmark data")
}

async fn facet_points(ctx: &SessionContext) -> datafusion::prelude::DataFrame {
    ctx.sql(
        "SELECT * FROM (VALUES
            ('Near', -1.0, -0.8, '#2563eb', 130.0),
            ('Near',  1.0,  0.8, '#2563eb', 180.0),
            ('Far',  19.0, -0.8, '#dc2626', 130.0),
            ('Far',  21.0,  0.8, '#dc2626', 180.0)
        ) AS t(panel, lon, lat, color, size)",
    )
    .await
    .expect("facet Geo data")
}

async fn repeat_points(ctx: &SessionContext) -> datafusion::prelude::DataFrame {
    ctx.sql(
        "SELECT * FROM (VALUES
            (-1.0, 19.0, -1.2, 2.2),
            ( 1.0, 21.0,  1.2, 4.2)
        ) AS t(near_lon, far_lon, south_lat, north_lat)",
    )
    .await
    .expect("repeat Geo data")
}

fn landmark_symbols(geo: &Geo) -> Symbol<Geo> {
    Symbol::new()
        .lon_lat(geo, col("lon"), col("lat"))
        .size(col("size"))
        .fill_with(col("color"), |fill| {
            fill.no_scale().legend(|legend| legend.visible(false))
        })
        .stroke("#111827")
        .stroke_width(1.5)
}

fn geo_child(coord: Geo) -> Plot<Geo> {
    let symbols = landmark_symbols(&coord);
    Plot::with_coord(coord)
        .plot_size(260.0, 220.0)
        .mark(symbols)
}

fn osm_tile_layer() -> avenger_chart_geo::RasterTileLayer {
    avenger_chart_geo::RasterTileLayer::xyz("https://tile.openstreetmap.org/{z}/{x}/{y}.png")
        .id("osm")
        .min_zoom(1)
        .max_zoom(1)
        .attribution("OpenStreetMap contributors")
        .zindex(-100)
}

fn osm_tile_reference_plot(title: &'static str) -> Plot<Geo> {
    let geo = Geo::mercator()
        .center_lon_lat(0.0, 30.0)
        .zoom(1.0)
        .tiles(osm_tile_layer());
    let marker = Symbol::new()
        .lon_lat(&geo, 0.0, 0.0)
        .size(140.0)
        .fill("#dc2626")
        .stroke("#111827");
    Plot::with_coord(geo)
        .plot_size(512.0, 256.0)
        .canvas_size(560.0, 340.0)
        .title(title)
        .mark(marker)
}

#[tokio::test]
async fn symbol_lon_lat_fit() {
    let ctx = SessionContext::new();
    let geo = Geo::mercator();
    let symbols = landmark_symbols(&geo);
    let plot = Plot::with_coord(geo)
        .canvas_size(420.0, 340.0)
        .title("WebMercator fit")
        .data(landmarks(&ctx).await)
        .mark(symbols);

    let compiled = plot.compile(&ctx).await.expect("compile Geo mercator fit");
    assert_visual_match_default(&compiled, &ctx, None, CATEGORY, "symbol_lon_lat_fit").await;
}

#[tokio::test]
async fn symbol_authored_center_zoom() {
    let ctx = SessionContext::new();
    let geo = Geo::mercator().center_lon_lat(-73.9857, 40.7484).zoom(12.0);
    let symbols = landmark_symbols(&geo);
    let plot = Plot::with_coord(geo)
        .canvas_size(420.0, 340.0)
        .title("Authored center/zoom")
        .data(landmarks(&ctx).await)
        .mark(symbols);

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile authored Geo mercator view");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        CATEGORY,
        "symbol_authored_center_zoom",
    )
    .await;
}

#[tokio::test]
async fn symbol_fixed_center_fit_zoom() {
    let ctx = SessionContext::new();
    let geo = Geo::mercator().center_lon_lat(-73.9857, 40.7484);
    let symbols = landmark_symbols(&geo);
    let plot = Plot::with_coord(geo)
        .canvas_size(420.0, 340.0)
        .title("Fixed center, inferred zoom")
        .data(landmarks(&ctx).await)
        .mark(symbols);

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile fixed-center Geo mercator view");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        CATEGORY,
        "symbol_fixed_center_fit_zoom",
    )
    .await;
}

#[tokio::test]
async fn symbol_wide_vs_tall_same_zoom() {
    let ctx = SessionContext::new();
    let coord = Geo::mercator().center_lon_lat(-73.9857, 40.7484).zoom(12.0);
    let plot = Plot::<HConcat>::new()
        .canvas_size(640.0, 360.0)
        .configure_coord(|c| c.widths([TrackSizing::Px(360.0), TrackSizing::Px(150.0)]))
        .title("Same center/zoom, different plot shapes")
        .data(landmarks(&ctx).await)
        .mark(
            Subplot::new(geo_child(coord.clone()))
                .name("wide")
                .label("Wide"),
        )
        .mark(Subplot::new(geo_child(coord)).name("tall").label("Tall"));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile Geo mercator shape comparison");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        CATEGORY,
        "symbol_wide_vs_tall_same_zoom",
    )
    .await;
}

fn faceted_symbols(geo: &Geo, scope: CoordinationScope) -> Symbol<Geo> {
    Symbol::new()
        .lon_lat_with(
            geo,
            col("lon"),
            col("lat"),
            |x| x.with_domain_scope(scope),
            |y| y.with_domain_scope(scope),
        )
        .size(col("size"))
        .fill_with(col("color"), |fill| {
            fill.no_scale().legend(|legend| legend.visible(false))
        })
        .stroke("#111827")
        .stroke_width(1.2)
}

#[tokio::test]
async fn facet_shared_viewport() {
    let ctx = SessionContext::new();
    let geo = Geo::mercator();
    let child =
        Plot::with_coord(geo.clone()).mark(faceted_symbols(&geo, CoordinationScope::Shared));
    let plot = Plot::<FacetColumn>::new()
        .plot_size(250.0, 190.0)
        .title("Shared WebMercator viewport")
        .data(facet_points(&ctx).await)
        .mark(Subplot::new(child).column(col("panel")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile shared Geo mercator facet");
    assert_visual_match_default(&compiled, &ctx, None, CATEGORY, "facet_shared_viewport").await;
}

#[tokio::test]
async fn facet_free_viewports() {
    let ctx = SessionContext::new();
    let geo = Geo::mercator();
    let child = Plot::with_coord(geo.clone()).mark(faceted_symbols(&geo, CoordinationScope::Free));
    let plot = Plot::<FacetColumn>::new()
        .plot_size(250.0, 190.0)
        .title("Free WebMercator viewports")
        .data(facet_points(&ctx).await)
        .mark(Subplot::new(child).column(col("panel")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile free Geo mercator facet");
    assert_visual_match_default(&compiled, &ctx, None, CATEGORY, "facet_free_viewports").await;
}

#[tokio::test]
async fn repeat_shared_fit() {
    let ctx = SessionContext::new();
    let geo = Geo::mercator();
    let cell = Plot::with_coord(geo.clone()).plot_size(190.0, 150.0).mark(
        Symbol::new()
            .lon_lat(&geo, repeat::column(), repeat::row())
            .size(150.0)
            .fill("#0f766e")
            .stroke("#111827")
            .stroke_width(1.2),
    );
    let plot = Plot::<RepeatGrid>::new()
        .canvas_size(560.0, 420.0)
        .title("Repeat WebMercator shared fit")
        .data(repeat_points(&ctx).await)
        .configure_coord(|c| {
            c.rows([
                RepeatVariable::new("south_lat", col("south_lat")).title("South"),
                RepeatVariable::new("north_lat", col("north_lat")).title("North"),
            ])
            .columns([
                RepeatVariable::new("near_lon", col("near_lon")).title("Near"),
                RepeatVariable::new("far_lon", col("far_lon")).title("Far"),
            ])
            .matrix_domains()
            .cell(cell)
        });

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile repeated Geo mercator grid");
    assert_visual_match_default(&compiled, &ctx, None, CATEGORY, "repeat_shared_fit").await;
}

#[tokio::test]
async fn tiles_wide_square_pixels() {
    let ctx = SessionContext::new();
    let plot = osm_tile_reference_plot("OSM wide viewport");

    let compiled = plot.compile(&ctx).await.expect("compile wide OSM tiles");
    assert_osm_tile_visual(&compiled, &ctx, "tiles_wide_square_pixels").await;
}

#[tokio::test]
async fn tiles_placeholder() {
    let ctx = SessionContext::new();
    let plot = osm_tile_reference_plot("OSM tile placeholders");
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile placeholder OSM tiles");
    let resolver: Arc<dyn ImageResourceResolver> = Arc::new(PendingImageResolver);
    let (direct_status, serialized_status) = assert_visual_match_wgpu_only_with_canvas_config(
        &compiled,
        &ctx,
        None,
        CATEGORY,
        "tiles_placeholder",
        0.9999,
        CanvasConfig {
            image_resource_config: WgpuImageResourceConfig {
                resolver: Some(resolver),
                missing_policy: WgpuMissingImagePolicy::DrawPlaceholder,
                placeholder: WgpuImagePlaceholder::Checkerboard,
            },
            ..Default::default()
        },
    )
    .await;

    assert!(
        !direct_status.pending.is_empty(),
        "placeholder render should report pending tile resources"
    );
    assert_eq!(direct_status.pending, serialized_status.pending);
    assert!(direct_status.missing.is_empty());
    assert!(direct_status.failed.is_empty());
    assert!(serialized_status.missing.is_empty());
    assert!(serialized_status.failed.is_empty());
}

#[tokio::test]
async fn tiles_ready_resource() {
    let ctx = SessionContext::new();
    let plot = osm_tile_reference_plot("OSM ready resources");
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile ready-resource OSM tiles");
    let resolver: Arc<dyn ImageResourceResolver> = Arc::new(OsmFixtureResolver::new());
    let (direct_status, serialized_status) = assert_visual_match_wgpu_only_with_canvas_config(
        &compiled,
        &ctx,
        None,
        CATEGORY,
        "tiles_ready_resource",
        0.9999,
        CanvasConfig {
            image_resource_config: WgpuImageResourceConfig {
                resolver: Some(resolver),
                missing_policy: WgpuMissingImagePolicy::DrawPlaceholder,
                placeholder: WgpuImagePlaceholder::Checkerboard,
            },
            ..Default::default()
        },
    )
    .await;

    assert!(direct_status.is_empty(), "{direct_status:?}");
    assert!(serialized_status.is_empty(), "{serialized_status:?}");
}

#[tokio::test]
async fn tiles_smooth_zoom_ready_fallback_pending_target() {
    let ctx = SessionContext::new();
    let geo = Geo::mercator().center_lon_lat(0.0, 0.0).zoom(1.0).tiles(
        avenger_chart_geo::RasterTileLayer::xyz("https://tiles.example/{z}/{x}/{y}.png")
            .id("smooth")
            .max_zoom(2)
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
    let marker = Symbol::new()
        .lon_lat(&geo, 0.0, 0.0)
        .size(140.0)
        .fill("#dc2626")
        .stroke("#111827");
    let plot = Plot::with_coord(geo)
        .plot_size(256.0, 256.0)
        .canvas_size(320.0, 330.0)
        .title("Smooth tile fallback")
        .mark(marker);
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile smooth tile fallback");
    let resolver: Arc<dyn ImageResourceResolver> = Arc::new(SmoothZoomFallbackResolver::new());
    let (direct_status, serialized_status) = assert_visual_match_wgpu_only_with_canvas_config(
        &compiled,
        &ctx,
        None,
        CATEGORY,
        "tiles_smooth_zoom_ready_fallback_pending_target",
        0.9999,
        CanvasConfig {
            image_resource_config: WgpuImageResourceConfig {
                resolver: Some(resolver),
                missing_policy: WgpuMissingImagePolicy::DrawPlaceholder,
                placeholder: WgpuImagePlaceholder::Checkerboard,
            },
            ..Default::default()
        },
    )
    .await;

    assert!(
        !direct_status.pending.is_empty(),
        "target tiles should still be reported pending"
    );
    assert!(direct_status.missing.is_empty());
    assert!(direct_status.failed.is_empty());
    assert_eq!(direct_status.pending, serialized_status.pending);
}

#[tokio::test]
async fn tiles_tall_square_pixels() {
    let ctx = SessionContext::new();
    let geo = Geo::mercator()
        .center_lon_lat(0.0, 0.0)
        .zoom(1.0)
        .tiles(osm_tile_layer());
    let marker = Symbol::new()
        .lon_lat(&geo, 0.0, 0.0)
        .size(140.0)
        .fill("#dc2626")
        .stroke("#111827");
    let plot = Plot::with_coord(geo)
        .plot_size(256.0, 512.0)
        .canvas_size(340.0, 590.0)
        .title("OSM tall viewport")
        .mark(marker);

    let compiled = plot.compile(&ctx).await.expect("compile tall OSM tiles");
    assert_osm_tile_visual(&compiled, &ctx, "tiles_tall_square_pixels").await;
}

#[tokio::test]
async fn tiles_partial_panned() {
    let ctx = SessionContext::new();
    let geo = Geo::mercator()
        .center_lon_lat(55.0, 20.0)
        .zoom(1.0)
        .tiles(osm_tile_layer());
    let marker = Symbol::new()
        .lon_lat(&geo, 55.0, 20.0)
        .size(140.0)
        .fill("#dc2626")
        .stroke("#111827");
    let plot = Plot::with_coord(geo)
        .plot_size(360.0, 260.0)
        .canvas_size(430.0, 350.0)
        .title("OSM partial pan")
        .mark(marker);

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile partial-panned OSM tiles");
    assert_osm_tile_visual(&compiled, &ctx, "tiles_partial_panned").await;
}

#[tokio::test]
async fn tiles_overzoom_max_zoom() {
    let ctx = SessionContext::new();
    let geo = Geo::mercator()
        .center_lon_lat(0.0, 0.0)
        .zoom(3.0)
        .tiles(osm_tile_layer());
    let marker = Symbol::new()
        .lon_lat(&geo, 0.0, 0.0)
        .size(140.0)
        .fill("#dc2626")
        .stroke("#111827");
    let plot = Plot::with_coord(geo)
        .plot_size(360.0, 260.0)
        .canvas_size(430.0, 350.0)
        .title("OSM overzoomed from z=1")
        .mark(marker);

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile overzoomed OSM tiles");
    assert_osm_tile_visual(&compiled, &ctx, "tiles_overzoom_max_zoom").await;
}

#[tokio::test]
async fn tiles_required_attribution() {
    let ctx = SessionContext::new();
    let geo = Geo::mercator()
        .center_lon_lat(0.0, 0.0)
        .zoom(1.0)
        .tiles(osm_tile_layer());
    let marker = Symbol::new()
        .lon_lat(&geo, 0.0, 0.0)
        .size(140.0)
        .fill("#dc2626")
        .stroke("#111827");
    let plot = Plot::with_coord(geo)
        .plot_size(512.0, 512.0)
        .canvas_size(580.0, 600.0)
        .title("OSM attribution")
        .mark(marker);

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile OSM attribution tiles");
    assert_osm_tile_visual(&compiled, &ctx, "tiles_required_attribution").await;
}

/// Phase-7 replacement gate: every ported baseline must match the retired
/// WebMercator suite's baseline image. Run with `--nocapture` to record the
/// scores.
///
/// Attribution-bearing tile scenes carry a relaxed threshold: the map
/// attribution text became italic after the port (`b20e86d30`), an
/// intentional geo-side styling change the retired WebMercator baselines
/// never received. Their divergence is confined to the attribution strip
/// (measured parity 0.994-0.997); everything else must stay at >= 0.9999.
#[test]
fn ported_baselines_match_webmercator_originals() {
    const FULL_PARITY: f64 = 0.9999;
    const ITALIC_ATTRIBUTION_PARITY: f64 = 0.99;
    let names = [
        ("symbol_lon_lat_fit", FULL_PARITY),
        ("symbol_authored_center_zoom", FULL_PARITY),
        ("symbol_fixed_center_fit_zoom", FULL_PARITY),
        ("symbol_wide_vs_tall_same_zoom", FULL_PARITY),
        ("facet_shared_viewport", FULL_PARITY),
        ("facet_free_viewports", FULL_PARITY),
        ("repeat_shared_fit", FULL_PARITY),
        ("tiles_wide_square_pixels", ITALIC_ATTRIBUTION_PARITY),
        ("tiles_placeholder", ITALIC_ATTRIBUTION_PARITY),
        ("tiles_ready_resource", ITALIC_ATTRIBUTION_PARITY),
        (
            "tiles_smooth_zoom_ready_fallback_pending_target",
            FULL_PARITY,
        ),
        ("tiles_tall_square_pixels", ITALIC_ATTRIBUTION_PARITY),
        ("tiles_partial_panned", ITALIC_ATTRIBUTION_PARITY),
        ("tiles_overzoom_max_zoom", ITALIC_ATTRIBUTION_PARITY),
        ("tiles_required_attribution", ITALIC_ATTRIBUTION_PARITY),
    ];
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/baselines");
    let mut failures = Vec::new();
    for (name, threshold) in names {
        let old_path = base.join("webmercator").join(format!("{name}.png"));
        let new_path = base.join(CATEGORY).join(format!("{name}.png"));
        let old = image::open(&old_path)
            .unwrap_or_else(|err| panic!("load {}: {err}", old_path.display()))
            .into_rgba8();
        let new = image::open(&new_path)
            .unwrap_or_else(|err| panic!("load {}: {err}", new_path.display()))
            .into_rgba8();
        if old.dimensions() != new.dimensions() {
            failures.push(format!(
                "{name}: dimensions {:?} vs {:?}",
                old.dimensions(),
                new.dimensions()
            ));
            continue;
        }
        let score = image_compare::rgba_hybrid_compare(&old, &new)
            .expect("image comparison")
            .score;
        println!("webmercator parity {name}: {score:.6}");
        if score < threshold {
            failures.push(format!("{name}: {score:.6} < {threshold}"));
        }
    }
    assert!(
        failures.is_empty(),
        "ported baselines diverge from webmercator originals:\n{}",
        failures.join("\n")
    );
}

async fn assert_osm_tile_visual(
    compiled: &CompiledPlot,
    ctx: &SessionContext,
    baseline_name: &str,
) {
    let resolver = OsmFixtureResolver::new();
    assert_visual_match_with_image_resolver(
        compiled,
        ctx,
        None,
        CATEGORY,
        baseline_name,
        0.9999,
        &resolver,
    )
    .await;
}

struct OsmFixtureResolver {
    images: HashMap<ResourceKey, Arc<AvengerRgbaImage>>,
}

impl OsmFixtureResolver {
    fn new() -> Self {
        let mut images = HashMap::new();
        for (x, y, bytes) in [
            (
                0,
                0,
                include_bytes!("../data/webmercator/osm/1/0/0.png").as_slice(),
            ),
            (
                0,
                1,
                include_bytes!("../data/webmercator/osm/1/0/1.png").as_slice(),
            ),
            (
                1,
                0,
                include_bytes!("../data/webmercator/osm/1/1/0.png").as_slice(),
            ),
            (
                1,
                1,
                include_bytes!("../data/webmercator/osm/1/1/1.png").as_slice(),
            ),
        ] {
            let image = image::load_from_memory(bytes)
                .expect("decode OSM tile fixture")
                .into_rgba8();
            images.insert(
                ResourceKey::new(format!("geo/osm/1/{x}/{y}/256")),
                Arc::new(AvengerRgbaImage::from_image(&image)),
            );
        }
        Self { images }
    }
}

impl ImageResourceResolver for OsmFixtureResolver {
    fn image_state(&self, key: &ResourceKey) -> ImageResourceState {
        self.images
            .get(key)
            .cloned()
            .map(ImageResourceState::Ready)
            .unwrap_or(ImageResourceState::Missing)
    }

    fn request_image(&self, _request: &avenger_resource::ResourceRequest) {}
}

struct PendingImageResolver;

impl ImageResourceResolver for PendingImageResolver {
    fn image_state(&self, _key: &ResourceKey) -> ImageResourceState {
        ImageResourceState::Pending
    }

    fn request_image(&self, _request: &avenger_resource::ResourceRequest) {}
}

struct SmoothZoomFallbackResolver {
    fallback: Arc<AvengerRgbaImage>,
}

impl SmoothZoomFallbackResolver {
    fn new() -> Self {
        Self {
            fallback: Arc::new(checker_fallback_tile()),
        }
    }
}

impl ImageResourceResolver for SmoothZoomFallbackResolver {
    fn image_state(&self, key: &ResourceKey) -> ImageResourceState {
        if key == &ResourceKey::new("geo/smooth/0/0/0/256") {
            ImageResourceState::Ready(self.fallback.clone())
        } else {
            ImageResourceState::Pending
        }
    }

    fn request_image(&self, _request: &avenger_resource::ResourceRequest) {}
}

fn checker_fallback_tile() -> AvengerRgbaImage {
    let mut data = Vec::with_capacity(256 * 256 * 4);
    for y in 0..256 {
        for x in 0..256 {
            let color = if ((x / 32) + (y / 32)) % 2 == 0 {
                [30, 100, 180, 255]
            } else {
                [70, 150, 210, 255]
            };
            data.extend_from_slice(&color);
        }
    }
    AvengerRgbaImage {
        width: 256,
        height: 256,
        data,
    }
}
