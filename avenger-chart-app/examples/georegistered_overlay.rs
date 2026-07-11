//! Georegistered image overlay: a pre-rasterized, frame-stamped external
//! image placed with no `Rasterize2D` at all — the CRS tag, not the
//! transform, is what makes a raster displayable.
//!
//! A checked-in CARTO basemap tile PNG (z4 x3 y5, the fixture the offline
//! visual tests use) is decoded, warm-tinted so it reads as an overlay,
//! wrapped in a raster struct with its true EPSG:3857 tile extents, and
//! rendered with `UniformRaster2D<Geo>` on the Albers CONUS aspect over
//! live CARTO tiles. If the georegistration is right, the tinted overlay's
//! features warp into exact alignment with the basemap under it. Drag to
//! pan, scroll to zoom, double-click to reset.
//!
//! Run with:
//! ```bash
//! cargo run --release -p avenger-chart-app --example georegistered_overlay --features winit-wgpu
//! ```

use std::{f64::consts::PI, sync::Arc};

use avenger_chart::prelude::*;
use avenger_chart_app::{
    CanvasConfig, ChartAppOptions, ChartResizeBinding, ChartRuntimeResources, WgpuImagePlaceholder,
    WgpuImageResourceConfig, WgpuMissingImagePolicy, WinitWgpuAvengerApp,
    WinitWgpuAvengerAppOptions, chart_avenger_app_with_runtime_resources,
};
use avenger_chart_geo::{
    Geo, GeoPanZoom, GeoUniformRaster2DChannels, GraticuleStyle, RasterTileLayer, UniformRaster2D,
    crs,
};
use avenger_image::ImageResourceCache;
use avenger_resource::RenderInvalidationHub;
use datafusion::arrow::{
    array::{
        ArrayRef, Float32Builder, Float64Array, ListArray, ListBuilder, StringArray, StringBuilder,
        StructArray, UInt32Array,
    },
    buffer::OffsetBuffer,
    datatypes::{DataType, Field},
    record_batch::RecordBatch,
};
use datafusion::prelude::SessionContext;
use winit::window::WindowAttributes;

const CARTO_TILE_TEMPLATE: &str =
    "https://basemaps.cartocdn.com/rastertiles/voyager_nolabels/{z}/{x}/{y}.png";
/// The overlay: the offline test fixture for CARTO tile z4 x3 y5.
const OVERLAY_PNG: &[u8] = include_bytes!("../../avenger-chart/tests/data/geo/carto/4/3/5.png");
const OVERLAY_Z: u32 = 4;
const OVERLAY_X: u32 = 3;
const OVERLAY_Y: u32 = 5;

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let invalidations = RenderInvalidationHub::default();
    let image_cache = Arc::new(
        ImageResourceCache::new().with_render_invalidation_sink(Arc::new(invalidations.clone())),
    );
    let runtime_resources = ChartRuntimeResources::new(image_cache.clone(), invalidations.clone());
    let canvas_config = CanvasConfig {
        image_resource_config: WgpuImageResourceConfig {
            resolver: Some(image_cache),
            missing_policy: WgpuMissingImagePolicy::DrawPlaceholder,
            placeholder: WgpuImagePlaceholder::Checkerboard,
        },
        ..CanvasConfig::default()
    };
    let avenger_app = tokio_runtime.block_on(build_app(runtime_resources));
    let options = WinitWgpuAvengerAppOptions::new(2.0)
        .window_attributes(
            WindowAttributes::default()
                .with_title("avenger-chart Geo: georegistered overlay")
                .with_resizable(false),
        )
        .canvas_config(canvas_config)
        .render_invalidation_hub(invalidations);
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app(
    runtime_resources: ChartRuntimeResources,
) -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let df = ctx
        .read_batch(overlay_raster_batch())
        .expect("read overlay raster");

    let tiles = RasterTileLayer::xyz(CARTO_TILE_TEMPLATE)
        .id("carto")
        .max_zoom(19)
        .attribution("© OpenStreetMap contributors © CARTO")
        .smooth_zoom();
    let geo = Geo::albers_usa_conus()
        .viewport_id("us")
        .center_lon_lat(-96.0, 38.5)
        .zoom(4.2)
        .graticule(GraticuleStyle::default())
        .tiles(tiles);
    let plot = Chart::with_coord(geo.clone())
        .canvas_size(860.0, 600.0)
        .title("Tinted georegistered tile over the basemap")
        .data(df)
        .mark(
            UniformRaster2D::<Geo>::new()
                .raster_with(col("raster"), |r| {
                    r.x(dim("x")).y(dim("y")).fill(|fill| fill.no_scale())
                })
                .smooth(true)
                .opacity(0.9),
        )
        .tool(GeoPanZoom::new().viewport_id("us"));

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app_with_runtime_resources(
        compiled,
        ctx,
        ChartAppOptions {
            resize_binding: ChartResizeBinding::none(),
            resize_throttle_ms: None,
            exact_on_resize_settle: true,
            log_metrics: false,
        },
        runtime_resources,
    )
    .await
    .expect("build chart app")
}

/// Decode the fixture PNG, warm-tint it, and wrap the pixels in a raster
/// struct stamped with the tile's true EPSG:3857 extents.
fn overlay_raster_batch() -> RecordBatch {
    let png = image::load_from_memory(OVERLAY_PNG)
        .expect("decode overlay PNG")
        .into_rgba8();
    let (width, height) = png.dimensions();

    // Tile bounds in authored raw units, scaled to 3857 meters. The raster
    // y dimension is ascending (south -> north) while PNG row 0 is the
    // north edge, so rows are reversed into cell order.
    let n = f64::from(1u32 << OVERLAY_Z);
    let span = 2.0 * PI / n;
    let x_west = (-PI + f64::from(OVERLAY_X) * span) * crs::WEB_MERCATOR_RADIUS_M;
    let x_east = (-PI + f64::from(OVERLAY_X + 1) * span) * crs::WEB_MERCATOR_RADIUS_M;
    let y_top = (PI - f64::from(OVERLAY_Y) * span) * crs::WEB_MERCATOR_RADIUS_M;
    let y_bottom = (PI - f64::from(OVERLAY_Y + 1) * span) * crs::WEB_MERCATOR_RADIUS_M;

    let mut cells = ListBuilder::new(ListBuilder::new(Float32Builder::new()));
    for cell_row in 0..height {
        let png_row = height - 1 - cell_row;
        for px in 0..width {
            let pixel = png.get_pixel(px, png_row);
            // Warm tint: keep red, damp green/blue so the overlay reads as
            // a distinct layer while its features align with the basemap.
            let rgba = [
                f32::from(pixel[0]) / 255.0,
                f32::from(pixel[1]) / 255.0 * 0.62,
                f32::from(pixel[2]) / 255.0 * 0.55,
                f32::from(pixel[3]) / 255.0,
            ];
            for component in rgba {
                cells.values().values().append_value(component);
            }
            cells.values().append(true);
        }
    }
    cells.append(true);
    let values = Arc::new(cells.finish()) as ArrayRef;

    let one_row_list = |array: ArrayRef| -> ArrayRef {
        let offsets = OffsetBuffer::from_lengths([array.len()]);
        Arc::new(
            ListArray::try_new(
                Arc::new(Field::new_list_field(array.data_type().clone(), true)),
                offsets,
                array,
                None,
            )
            .expect("list array"),
        ) as ArrayRef
    };
    let coords = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("kind", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["uniform", "uniform"])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("sampling", DataType::Utf8, true)),
            Arc::new(StringArray::from(vec![None::<&str>, None])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("start", DataType::Float64, true)),
            Arc::new(Float64Array::from(vec![Some(x_west), Some(y_bottom)])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("stop", DataType::Float64, true)),
            Arc::new(Float64Array::from(vec![Some(x_east), Some(y_top)])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("count", DataType::UInt32, true)),
            Arc::new(UInt32Array::from(vec![Some(width), Some(height)])) as ArrayRef,
        ),
    ])) as ArrayRef;
    let dimensions = one_row_list(Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("name", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["x", "y"])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("coords", coords.data_type().clone(), false)),
            coords,
        ),
    ])) as ArrayRef);
    let geometry = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("kind", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["grid"])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("crs", DataType::Utf8, true)),
            Arc::new(StringArray::from(vec![Some(crs::EPSG_3857)])) as ArrayRef,
        ),
        (
            Arc::new(Field::new(
                "dimensions",
                dimensions.data_type().clone(),
                false,
            )),
            dimensions,
        ),
    ])) as ArrayRef;
    let dims = {
        let mut builder = ListBuilder::new(StringBuilder::new());
        builder.values().append_value("y");
        builder.values().append_value("x");
        builder.append(true);
        Arc::new(builder.finish()) as ArrayRef
    };
    let values_struct = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("dims", dims.data_type().clone(), false)),
            dims,
        ),
        (
            Arc::new(Field::new("data", values.data_type().clone(), false)),
            values,
        ),
    ])) as ArrayRef;
    let raster = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("geometry", geometry.data_type().clone(), false)),
            geometry,
        ),
        (
            Arc::new(Field::new(
                "values",
                values_struct.data_type().clone(),
                false,
            )),
            values_struct,
        ),
    ])) as ArrayRef;
    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("overlay raster batch")
}

fn init_diagnostics() {
    if std::env::var_os("RUST_LOG").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
            .try_init();
    } else {
        let _ = env_logger::try_init();
    }
}
