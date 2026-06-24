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
    .expect("facet WebMercator data")
}

async fn repeat_points(ctx: &SessionContext) -> datafusion::prelude::DataFrame {
    ctx.sql(
        "SELECT * FROM (VALUES
            (-1.0, 19.0, -1.2, 2.2),
            ( 1.0, 21.0,  1.2, 4.2)
        ) AS t(near_lon, far_lon, south_lat, north_lat)",
    )
    .await
    .expect("repeat WebMercator data")
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

fn colored_symbols() -> Symbol<WebMercator> {
    Symbol::new()
        .longitude(col("lon"))
        .latitude(col("lat"))
        .size(col("size"))
        .fill_with(col("color"), |fill| {
            fill.no_scale().legend(|legend| legend.visible(false))
        })
        .stroke("#111827")
        .stroke_width(1.2)
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
async fn facet_shared_viewport() {
    let ctx = SessionContext::new();
    let child = Plot::with_coord(WebMercator::new()).mark(
        colored_symbols()
            .longitude_with(col("lon"), |x| {
                x.with_domain_scope(CoordinationScope::Shared)
            })
            .latitude_with(col("lat"), |y| {
                y.with_domain_scope(CoordinationScope::Shared)
            }),
    );
    let plot = Plot::<FacetColumn>::new()
        .plot_size(250.0, 190.0)
        .title("Shared WebMercator viewport")
        .data(facet_points(&ctx).await)
        .mark(Subplot::new(child).column(col("panel")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile shared WebMercator facet");
    assert_visual_match_default(&compiled, &ctx, None, CATEGORY, "facet_shared_viewport").await;
}

#[tokio::test]
async fn facet_free_viewports() {
    let ctx = SessionContext::new();
    let child = Plot::with_coord(WebMercator::new()).mark(
        colored_symbols()
            .longitude_with(col("lon"), |x| x.with_domain_scope(CoordinationScope::Free))
            .latitude_with(col("lat"), |y| y.with_domain_scope(CoordinationScope::Free)),
    );
    let plot = Plot::<FacetColumn>::new()
        .plot_size(250.0, 190.0)
        .title("Free WebMercator viewports")
        .data(facet_points(&ctx).await)
        .mark(Subplot::new(child).column(col("panel")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile free WebMercator facet");
    assert_visual_match_default(&compiled, &ctx, None, CATEGORY, "facet_free_viewports").await;
}

#[tokio::test]
async fn repeat_shared_fit() {
    let ctx = SessionContext::new();
    let cell = Plot::with_coord(WebMercator::new())
        .plot_size(190.0, 150.0)
        .mark(
            Symbol::new()
                .longitude(repeat::column())
                .latitude(repeat::row())
                .size(150.0)
                .fill("#0f766e")
                .stroke("#111827")
                .stroke_width(1.2),
        );
    let plot = Plot::<RepeatGrid>::new()
        .canvas_size(560.0, 420.0)
        .title("Repeat WebMercator shared fit")
        .data(repeat_points(&ctx).await)
        .rows([
            RepeatVariable::new("south_lat", col("south_lat")).title("South"),
            RepeatVariable::new("north_lat", col("north_lat")).title("North"),
        ])
        .columns([
            RepeatVariable::new("near_lon", col("near_lon")).title("Near"),
            RepeatVariable::new("far_lon", col("far_lon")).title("Far"),
        ])
        .matrix_domains()
        .cell(cell);

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile repeated WebMercator grid");
    assert_visual_match_default(&compiled, &ctx, None, CATEGORY, "repeat_shared_fit").await;
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
