use std::{collections::HashMap, sync::Arc};

use super::helpers::{assert_visual_match_default, assert_visual_match_with_image_resolver};
use avenger_chart::plot::CompiledPlot;
use avenger_chart::prelude::*;
use avenger_chart_webmercator::{Symbol, WebMercator, WebMercatorSymbolPositionChannels};
use avenger_image::{ImageResourceResolver, ImageResourceState, RgbaImage as AvengerRgbaImage};
use avenger_resource::ResourceKey;
use datafusion::prelude::{SessionContext, col};

const CATEGORY: &str = "webmercator";

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

fn landmark_symbols() -> Symbol<WebMercator> {
    Symbol::new()
        .longitude(col("lon"))
        .latitude(col("lat"))
        .size(col("size"))
        .fill_with(col("color"), |fill| {
            fill.no_scale().legend(|legend| legend.visible(false))
        })
        .stroke("#111827")
        .stroke_width(1.5)
}

fn webmercator_child(coord: WebMercator) -> Plot<WebMercator> {
    Plot::with_coord(coord)
        .plot_size(260.0, 220.0)
        .mark(landmark_symbols())
}

fn osm_tile_layer() -> avenger_chart_webmercator::RasterTileLayer {
    avenger_chart_webmercator::RasterTileLayer::xyz(
        "https://tile.openstreetmap.org/{z}/{x}/{y}.png",
    )
    .id("osm")
    .min_zoom(1)
    .max_zoom(1)
    .attribution("OpenStreetMap contributors")
    .zindex(-100)
}

#[tokio::test]
async fn symbol_lon_lat_fit() {
    let ctx = SessionContext::new();
    let plot = Plot::with_coord(WebMercator::new())
        .canvas_size(420.0, 340.0)
        .title("WebMercator fit")
        .data(landmarks(&ctx).await)
        .mark(landmark_symbols());

    let compiled = plot.compile(&ctx).await.expect("compile WebMercator fit");
    assert_visual_match_default(&compiled, &ctx, None, CATEGORY, "symbol_lon_lat_fit").await;
}

#[tokio::test]
async fn symbol_authored_center_zoom() {
    let ctx = SessionContext::new();
    let plot = Plot::with_coord(
        WebMercator::new()
            .center_lon_lat(-73.9857, 40.7484)
            .zoom(12.0),
    )
    .canvas_size(420.0, 340.0)
    .title("Authored center/zoom")
    .data(landmarks(&ctx).await)
    .mark(landmark_symbols());

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile authored WebMercator view");
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
    let plot = Plot::with_coord(WebMercator::new().center_lon_lat(-73.9857, 40.7484))
        .canvas_size(420.0, 340.0)
        .title("Fixed center, inferred zoom")
        .data(landmarks(&ctx).await)
        .mark(landmark_symbols());

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile fixed-center WebMercator view");
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
    let coord = WebMercator::new()
        .center_lon_lat(-73.9857, 40.7484)
        .zoom(12.0);
    let plot = Plot::<HConcat>::new()
        .canvas_size(640.0, 360.0)
        .widths([TrackSizing::Px(360.0), TrackSizing::Px(150.0)])
        .title("Same center/zoom, different plot shapes")
        .data(landmarks(&ctx).await)
        .mark(
            Subplot::new(webmercator_child(coord.clone()))
                .key("wide")
                .label("Wide"),
        )
        .mark(
            Subplot::new(webmercator_child(coord))
                .key("tall")
                .label("Tall"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile WebMercator shape comparison");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        CATEGORY,
        "symbol_wide_vs_tall_same_zoom",
    )
    .await;
}

#[tokio::test]
async fn tiles_wide_square_pixels() {
    let ctx = SessionContext::new();
    let plot = Plot::with_coord(
        WebMercator::new()
            .center_lon_lat(0.0, 30.0)
            .zoom(1.0)
            .tiles(osm_tile_layer()),
    )
    .plot_size(512.0, 256.0)
    .canvas_size(560.0, 340.0)
    .title("OSM wide viewport")
    .mark(
        Symbol::new()
            .longitude(0.0)
            .latitude(0.0)
            .size(140.0)
            .fill("#dc2626")
            .stroke("#111827"),
    );

    let compiled = plot.compile(&ctx).await.expect("compile wide OSM tiles");
    assert_osm_tile_visual(&compiled, &ctx, "tiles_wide_square_pixels").await;
}

#[tokio::test]
async fn tiles_tall_square_pixels() {
    let ctx = SessionContext::new();
    let plot = Plot::with_coord(
        WebMercator::new()
            .center_lon_lat(0.0, 0.0)
            .zoom(1.0)
            .tiles(osm_tile_layer()),
    )
    .plot_size(256.0, 512.0)
    .canvas_size(340.0, 590.0)
    .title("OSM tall viewport")
    .mark(
        Symbol::new()
            .longitude(0.0)
            .latitude(0.0)
            .size(140.0)
            .fill("#dc2626")
            .stroke("#111827"),
    );

    let compiled = plot.compile(&ctx).await.expect("compile tall OSM tiles");
    assert_osm_tile_visual(&compiled, &ctx, "tiles_tall_square_pixels").await;
}

#[tokio::test]
async fn tiles_partial_panned() {
    let ctx = SessionContext::new();
    let plot = Plot::with_coord(
        WebMercator::new()
            .center_lon_lat(55.0, 20.0)
            .zoom(1.0)
            .tiles(osm_tile_layer()),
    )
    .plot_size(360.0, 260.0)
    .canvas_size(430.0, 350.0)
    .title("OSM partial pan")
    .mark(
        Symbol::new()
            .longitude(55.0)
            .latitude(20.0)
            .size(140.0)
            .fill("#dc2626")
            .stroke("#111827"),
    );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile partial-panned OSM tiles");
    assert_osm_tile_visual(&compiled, &ctx, "tiles_partial_panned").await;
}

#[tokio::test]
async fn tiles_overzoom_max_zoom() {
    let ctx = SessionContext::new();
    let plot = Plot::with_coord(
        WebMercator::new()
            .center_lon_lat(0.0, 0.0)
            .zoom(3.0)
            .tiles(osm_tile_layer()),
    )
    .plot_size(360.0, 260.0)
    .canvas_size(430.0, 350.0)
    .title("OSM overzoomed from z=1")
    .mark(
        Symbol::new()
            .longitude(0.0)
            .latitude(0.0)
            .size(140.0)
            .fill("#dc2626")
            .stroke("#111827"),
    );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile overzoomed OSM tiles");
    assert_osm_tile_visual(&compiled, &ctx, "tiles_overzoom_max_zoom").await;
}

#[tokio::test]
async fn tiles_required_attribution() {
    let ctx = SessionContext::new();
    let plot = Plot::with_coord(
        WebMercator::new()
            .center_lon_lat(0.0, 0.0)
            .zoom(1.0)
            .tiles(osm_tile_layer()),
    )
    .plot_size(512.0, 512.0)
    .canvas_size(580.0, 600.0)
    .title("OSM attribution")
    .mark(
        Symbol::new()
            .longitude(0.0)
            .latitude(0.0)
            .size(140.0)
            .fill("#dc2626")
            .stroke("#111827"),
    );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile OSM attribution tiles");
    assert_osm_tile_visual(&compiled, &ctx, "tiles_required_attribution").await;
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
                ResourceKey::new(format!("webmercator/osm/1/{x}/{y}/256")),
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
