//! Headless profiling companion for `basic_chart`.
//!
//! This drives the same 100k-point chart through `AvengerPlotHandle` param
//! updates and scene rebuilds without opening an egui window. It is intended to
//! profile the scene-evaluation side of the egui integration, which is the
//! dominant cost for the 100k-point pan/zoom baseline.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-egui --release --example basic_chart_scene_profile
//! ```
//!
//! Optional knobs:
//! - `AVENGER_EGUI_PROFILE_POINTS=100000`
//! - `AVENGER_EGUI_PROFILE_FRAMES=8`

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use avenger_app::app::AvengerApp;
use avenger_chart::prelude::*;
use avenger_chart_app::{ChartAppOptions, ChartAppState, ChartResizeBinding, chart_avenger_app};
use avenger_chart_egui::AvengerPlotHandle;
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    logical_expr::when,
    prelude::SessionContext,
    scalar::ScalarValue,
};

const DEFAULT_POINT_COUNT: usize = 100_000;
const DEFAULT_PROFILE_FRAMES: usize = 8;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_diagnostics();

    let point_count = env_usize("AVENGER_EGUI_PROFILE_POINTS", DEFAULT_POINT_COUNT);
    let frame_count = env_usize("AVENGER_EGUI_PROFILE_FRAMES", DEFAULT_PROFILE_FRAMES);
    let app = build_app(point_count).await?;
    let handle = AvengerPlotHandle::from_app(app);
    let runtime = tokio::runtime::Handle::current();

    println!(
        "egui_scene_profile points={} frames={}",
        point_count, frame_count
    );

    let warm_generation = handle
        .request_scene_rebuild(&runtime, true)
        .expect("request warm scene rebuild");
    let warm_elapsed = wait_for_scene_generation(&handle, warm_generation.get()).await?;
    println!(
        "egui_scene_profile seq=0 kind=warm generation={} request_to_publish={:.2}ms {}",
        warm_generation.get(),
        ms(warm_elapsed),
        metrics_summary(&handle)
    );
    handle.reset_metrics();

    let mut request_total = Duration::ZERO;
    let mut eval_total_us = 0_u64;
    for idx in 0..frame_count {
        let point_size = 64.0 + ((idx % 6) as f64 * 24.0);
        let changed = handle
            .set_param("point_size", point_size)
            .is_ok_and(|result| result.changed);
        let generation = handle
            .request_scene_rebuild(&runtime, true)
            .expect("request scene rebuild");
        let request_elapsed = wait_for_scene_generation(&handle, generation.get()).await?;
        let metrics = handle.metrics();

        request_total += request_elapsed;
        eval_total_us += metrics.last_scene_evaluation_us;
        println!(
            "egui_scene_profile seq={} point_size={:.1} changed={} generation={} request_to_publish={:.2}ms scene_eval={:.2}ms published={} dropped={} bottleneck={}({:.2}ms)",
            idx + 1,
            point_size,
            changed,
            generation.get(),
            ms(request_elapsed),
            us_to_ms(metrics.last_scene_evaluation_us),
            metrics.scene_frames_published,
            metrics.stale_scene_frames_dropped,
            metrics.latency_bottleneck().as_str(),
            us_to_ms(metrics.latency_bottleneck_us()),
        );
        handle.reset_metrics();
    }

    if frame_count > 0 {
        println!(
            "egui_scene_profile summary frames={} avg_request_to_publish={:.2}ms avg_scene_eval={:.2}ms",
            frame_count,
            ms(request_total) / frame_count as f64,
            us_to_ms(eval_total_us / frame_count as u64),
        );
    }

    Ok(())
}

async fn wait_for_scene_generation(
    handle: &AvengerPlotHandle,
    generation: u64,
) -> Result<Duration, Box<dyn std::error::Error>> {
    let start = Instant::now();
    let deadline = start + Duration::from_secs(30);
    loop {
        if let Some(error) = handle.latest_scene_error() {
            return Err(error.into());
        }
        if let Some(frame) = handle.latest_scene_frame()
            && frame.generation.get() >= generation
        {
            return Ok(start.elapsed());
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for scene generation {generation}").into());
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

async fn build_app(
    point_count: usize,
) -> Result<AvengerApp<ChartAppState>, Box<dyn std::error::Error>> {
    let ctx = Arc::new(SessionContext::new());
    let width = Param::new("width", ScalarValue::Float64(Some(760.0)));
    let height = Param::new("height", ScalarValue::Float64(Some(520.0)));
    let point_size = Param::new("point_size", ScalarValue::Float64(Some(120.0)));
    let show_points = Param::new("show_points", ScalarValue::Boolean(Some(true)));
    let size = when(show_points.expr(), point_size.expr())
        .otherwise(lit(0.0))
        .expect("build point-size conditional");
    let df = ctx.read_batch(make_points_batch(point_count))?;

    let plot = avenger_chart::prelude::Chart::<Cartesian>::new()
        .canvas_size(width.expr(), height.expr())
        .param(width)
        .param(height)
        .param(point_size.clone())
        .param(show_points)
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("group_name"))
                .size(size),
        )
        .tool(PanScrollZoom::cartesian().settle_exact(true));

    let compiled = plot.compile(&ctx).await?;
    Ok(chart_avenger_app(
        compiled,
        ctx,
        ChartAppOptions {
            resize_binding: ChartResizeBinding::width_height("width", "height"),
            resize_throttle_ms: None,
            exact_on_resize_settle: true,
            log_metrics: false,
        },
    )
    .await?)
}

fn make_points_batch(point_count: usize) -> RecordBatch {
    let columns = (point_count as f64).sqrt().ceil() as usize;
    let rows = point_count.div_ceil(columns);
    let mut xs = Vec::with_capacity(point_count);
    let mut ys = Vec::with_capacity(point_count);
    let mut groups = Vec::with_capacity(point_count);
    let group_names = ["A", "B", "C", "D"];

    for row in 0..rows {
        for col in 0..columns {
            if xs.len() == point_count {
                break;
            }
            let idx = row * columns + col;
            let jitter_x = (((idx * 37 + 11) % 100) as f64 - 50.0) / 120.0;
            let jitter_y = (((idx * 53 + 7) % 100) as f64 - 50.0) / 120.0;
            let x = col as f64 + jitter_x;
            let wave = (col as f64 / 16.0).sin() * 12.0 + (col as f64 / 37.0).cos() * 6.0;
            let y = row as f64 + wave + jitter_y;
            xs.push(x);
            ys.push(y);
            groups.push(group_names[idx % group_names.len()]);
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("group_name", DataType::Utf8, false),
    ]));

    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(xs)) as ArrayRef,
            Arc::new(Float64Array::from(ys)),
            Arc::new(StringArray::from(groups)),
        ],
    )
    .expect("build generated point batch")
}

fn metrics_summary(handle: &AvengerPlotHandle) -> String {
    let metrics = handle.metrics();
    format!(
        "scene_eval={:.2}ms published={} dropped={} bottleneck={}({:.2}ms)",
        us_to_ms(metrics.last_scene_evaluation_us),
        metrics.scene_frames_published,
        metrics.stale_scene_frames_dropped,
        metrics.latency_bottleneck().as_str(),
        us_to_ms(metrics.latency_bottleneck_us()),
    )
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn us_to_ms(us: u64) -> f64 {
    us as f64 / 1000.0
}

fn init_diagnostics() {
    if std::env::var_os("RUST_LOG").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();
    }
}
