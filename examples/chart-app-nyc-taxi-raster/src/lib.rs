use std::{io::Cursor, sync::Arc, time::Duration};

use arrow::{datatypes::DataType, ipc::reader::FileReader, record_batch::RecordBatch};
use avenger_chart::prelude::*;
use avenger_chart_app::{
    CanvasConfig, ChartAppOptions, ChartResizeBinding, ChartRuntimeResources, WgpuImagePlaceholder,
    WgpuImageResourceConfig, WgpuMissingImagePolicy, WindowSceneSizing, WinitWgpuAvengerApp,
    WinitWgpuAvengerAppOptions, chart_avenger_app_with_default_runtime_resources,
};
use avenger_chart_geo::{
    Geo, GeoPanZoom, GeoUniformRaster2DChannels, RasterTileLayer, UniformRaster2D, crs,
};
use datafusion::{
    dataframe::DataFrame,
    datasource::MemTable,
    error::{DataFusionError, Result as DataFusionResult},
    execution::{
        config::SessionConfig,
        disk_manager::{DiskManagerBuilder, DiskManagerMode},
        runtime_env::RuntimeEnvBuilder,
    },
    functions::expr_fn::floor,
    logical_expr::{Expr, expr_fn::cast},
    prelude::SessionContext,
};
use palette::rgb::Srgba;
use winit::window::WindowAttributes;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const CARTO_TILE_TEMPLATE: &str =
    "https://basemaps.cartocdn.com/rastertiles/voyager_nolabels/{z}/{x}/{y}.png";
const TAXI_ARROW_URL: &str = "data/nyc_taxi_1m.arrow";
const TAXI_TABLE: &str = "taxi_pickups";
const TAXI_BATCH_ROWS: usize = 8192;
const TAXI_POINT_COUNT: usize = 1_000_000;
const RASTER_BLUE: Srgba = Srgba::new(0.031, 0.318, 0.612, 1.0);

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub async fn run() {
    init_diagnostics();

    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            let bundle = build_app().await;
            let options = app_options(&bundle.runtime_resources);
            let (mut app, event_loop) =
                WinitWgpuAvengerApp::new_and_event_loop_with_options(bundle.app, options);
            event_loop.run_app(&mut app).expect("run app");
        } else {
            let tokio_runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("build tokio runtime");
            let bundle = tokio_runtime.block_on(build_app());
            let options = app_options(&bundle.runtime_resources);
            let (mut app, event_loop) = WinitWgpuAvengerApp::new_and_event_loop_with_options(
                bundle.app,
                options,
                tokio_runtime,
            );
            event_loop.run_app(&mut app).expect("run app");
        }
    }
}

fn app_options(runtime_resources: &ChartRuntimeResources) -> WinitWgpuAvengerAppOptions {
    let canvas_config = CanvasConfig {
        image_resource_config: WgpuImageResourceConfig {
            resolver: Some(runtime_resources.image_resource_resolver.clone()),
            missing_policy: WgpuMissingImagePolicy::DrawPlaceholder,
            placeholder: WgpuImagePlaceholder::Checkerboard,
        },
        ..CanvasConfig::default()
    };
    WinitWgpuAvengerAppOptions::new(2.0)
        .window_attributes(
            WindowAttributes::default()
                .with_title("avenger-chart NYC taxi raster on Mercator tiles")
                .with_resizable(false),
        )
        .window_scene_sizing(WindowSceneSizing::MatchSceneGraph)
        .canvas_config(canvas_config)
        .render_invalidation_hub(runtime_resources.render_invalidation_hub.clone())
}

async fn build_app() -> avenger_chart_app::ChartAppBundle {
    let ctx = Arc::new(session_context().expect("create DataFusion session context"));
    let df = load_taxi_dataframe(&ctx)
        .await
        .expect("load NYC taxi Arrow fixture");

    let tiles = RasterTileLayer::xyz(CARTO_TILE_TEMPLATE)
        .id("carto")
        .max_zoom(19)
        .attribution("© CARTO, © OpenStreetMap contributors")
        .zindex(-10)
        .smooth_zoom();
    let geo = Geo::mercator()
        .viewport_id("nyc")
        .center_lon_lat(-73.977, 40.75)
        .zoom(11.0)
        .tiles(tiles);

    let plot = Chart::with_coord(geo.clone())
        .canvas_size(960.0, 720.0)
        .configure_title(
            "NYC taxi density: live rasterization ($n = 1,000,000$)",
            |title| title.typst(),
        )
        .mark(
            MarkGroup::<Geo>::new().data(df).view(
                View::cartesian()
                    .id("taxi_density")
                    .x_domain(col("pickup_x"))
                    .y_domain(col("pickup_y"))
                    .preview_cached(true)
                    .throttle(Duration::from_millis(100)),
                |group, v| group.mark(raster_mark(&v)),
            ),
        )
        .tool(GeoPanZoom::new().viewport_id("nyc").settle_exact(true));

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app_with_default_runtime_resources(
        compiled,
        ctx,
        ChartAppOptions {
            resize_binding: ChartResizeBinding::none(),
            resize_throttle_ms: None,
            exact_on_resize_settle: true,
            log_metrics: true,
        },
    )
    .await
    .expect("build chart app")
}

fn session_context() -> DataFusionResult<SessionContext> {
    let config = SessionConfig::new()
        .with_target_partitions(1)
        .with_repartition_aggregations(false)
        .with_repartition_file_scans(false)
        .with_repartition_joins(false)
        .with_repartition_sorts(false)
        .with_repartition_windows(false);
    let runtime = RuntimeEnvBuilder::new()
        .with_disk_manager_builder(
            DiskManagerBuilder::default().with_mode(DiskManagerMode::Disabled),
        )
        .build_arc()?;
    Ok(SessionContext::new_with_config_rt(config, runtime))
}

/// Authored-plane raw-unit expr -> EPSG:3857 meters.
fn meters_x(raw: Expr) -> Expr {
    crs::from_mercator_units_x(crs::EPSG_3857, raw).expect("epsg:3857 is a supported frame")
}

fn meters_y(raw: Expr) -> Expr {
    crs::from_mercator_units_y(crs::EPSG_3857, raw).expect("epsg:3857 is a supported frame")
}

fn raster_mark(v: &ViewRef) -> UniformRaster2D<Geo> {
    let x_start = meters_x(v.x().domain_start());
    let x_end = meters_x(v.x().domain_end());
    let y_start = meters_y(v.y().domain_start());
    let y_end = meters_y(v.y().domain_end());
    let x_bins = view_pixel_bins(v.x().pixels());
    let y_bins = view_pixel_bins(v.y().pixels());

    UniformRaster2D::new()
        .transform(
            Rasterize2D::new(col("pickup_x"), col("pickup_y"))
                .frame(crs::EPSG_3857)
                .x(|x| x.extent(x_start, x_end).bins(x_bins))
                .y(|y| y.extent(y_start, y_end).bins(y_bins))
                .value(lit(1.0))
                .agg("sum"),
            |mark, hist| {
                GeoUniformRaster2DChannels::raster_with(mark, hist.raster(), |r| {
                    r.x(hist.x_dim()).y(hist.y_dim()).fill(|fill| {
                        fill.scale_with::<Sqrt>(|scale| {
                            scale
                                .clamp(true)
                                .domain((0.0, 3.0))
                                .range_colors(vec![Srgba::new(0.93, 0.97, 0.76, 1.0), RASTER_BLUE])
                                .nice(false)
                                .zero(false)
                        })
                        .legend(|legend| legend.title("Pickups per cell"))
                    })
                })
            },
        )
        .opacity(0.82)
        .smooth(false)
}

/// Bin count = one raster cell per view pixel. Must stay whole-numbered:
/// Rasterize2D rejects fractional bin counts.
fn view_pixel_bins(view_pixels: Expr) -> Expr {
    floor(cast(view_pixels, DataType::Float64))
}

async fn load_taxi_dataframe(ctx: &SessionContext) -> DataFusionResult<DataFrame> {
    let bytes = load_arrow_bytes(TAXI_ARROW_URL).await?;
    let batches = decode_arrow(bytes)?;
    let rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
    if rows != TAXI_POINT_COUNT {
        log::warn!("expected {TAXI_POINT_COUNT} taxi rows, loaded {rows}");
    }

    register_record_batches(
        ctx,
        TAXI_TABLE,
        rechunk_record_batches(batches, TAXI_BATCH_ROWS)?,
    )
    .await
}

#[cfg(target_arch = "wasm32")]
async fn load_arrow_bytes(url: &str) -> DataFusionResult<Vec<u8>> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window()
        .ok_or_else(|| DataFusionError::External("browser fetch requires a window".into()))?;
    let response_value = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(js_error)?;
    let response = response_value
        .dyn_into::<web_sys::Response>()
        .map_err(|_| DataFusionError::Execution(format!("{url} did not return a Response")))?;
    if !response.ok() {
        return Err(DataFusionError::Execution(format!(
            "failed to fetch {url}: HTTP {}",
            response.status()
        )));
    }
    let buffer = response.array_buffer().map_err(js_error)?;
    let buffer = JsFuture::from(buffer).await.map_err(js_error)?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

#[cfg(target_arch = "wasm32")]
fn js_error(value: wasm_bindgen::JsValue) -> DataFusionError {
    DataFusionError::External(
        value
            .as_string()
            .unwrap_or_else(|| format!("{value:?}"))
            .into(),
    )
}

#[cfg(not(target_arch = "wasm32"))]
async fn load_arrow_bytes(url: &str) -> DataFusionResult<Vec<u8>> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(url);
    std::fs::read(&path).map_err(|error| {
        DataFusionError::IoError(std::io::Error::new(
            error.kind(),
            format!(
                "failed to read {}; run `cargo run --release -p chart-app-nyc-taxi-raster --bin prepare_data` first: {error}",
                path.display()
            ),
        ))
    })
}

fn decode_arrow(bytes: Vec<u8>) -> DataFusionResult<Vec<RecordBatch>> {
    let reader = FileReader::try_new(Cursor::new(bytes), None)?;
    reader
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| DataFusionError::ArrowError(Box::new(error), None))
}

async fn register_record_batches(
    ctx: &SessionContext,
    name: &str,
    batches: Vec<RecordBatch>,
) -> DataFusionResult<DataFrame> {
    let schema = batches
        .first()
        .map(RecordBatch::schema)
        .ok_or_else(|| DataFusionError::Execution("taxi Arrow file has no rows".to_string()))?;
    let partitions = partition_record_batches(batches, ctx.state().config().target_partitions());
    let table = Arc::new(MemTable::try_new(schema, partitions)?);
    ctx.register_table(name, table)?;
    ctx.table(name).await
}

fn rechunk_record_batches(
    batches: Vec<RecordBatch>,
    target_rows: usize,
) -> DataFusionResult<Vec<RecordBatch>> {
    if target_rows == 0 {
        return Err(DataFusionError::Execution(
            "target_rows must be greater than zero".to_string(),
        ));
    }
    let Some(schema) = batches.first().map(RecordBatch::schema) else {
        return Ok(Vec::new());
    };
    let mut output = Vec::new();
    let mut pending = Vec::new();
    let mut pending_rows = 0usize;

    for batch in batches {
        let mut offset = 0usize;
        while offset < batch.num_rows() {
            let available = batch.num_rows() - offset;
            let needed = target_rows - pending_rows;
            let take = available.min(needed);
            pending.push(batch.slice(offset, take));
            pending_rows += take;
            offset += take;

            if pending_rows == target_rows {
                output.push(arrow::compute::concat_batches(&schema, pending.iter())?);
                pending.clear();
                pending_rows = 0;
            }
        }
    }
    if !pending.is_empty() {
        output.push(arrow::compute::concat_batches(&schema, pending.iter())?);
    }
    Ok(output)
}

fn partition_record_batches(
    batches: Vec<RecordBatch>,
    target_partitions: usize,
) -> Vec<Vec<RecordBatch>> {
    let partition_count = target_partitions.max(1).min(batches.len().max(1));
    let mut partitions = vec![Vec::new(); partition_count];
    for (index, batch) in batches.into_iter().enumerate() {
        partitions[index % partition_count].push(batch);
    }
    partitions
        .into_iter()
        .filter(|partition| !partition.is_empty())
        .collect()
}

fn init_diagnostics() {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            std::panic::set_hook(Box::new(console_error_panic_hook::hook));
            console_log::init_with_level(log::Level::Warn).expect("initialize logger");
        } else if #[cfg(not(target_arch = "wasm32"))] {
            if std::env::var_os("RUST_LOG").is_some() {
                let _ = tracing_subscriber::fmt()
                    .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
                    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                    .try_init();
            } else {
                let _ = env_logger::try_init();
            }
        }
    }
}
