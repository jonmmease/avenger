#![allow(dead_code)]

use std::{future::Future, sync::Arc};

use avenger_app::app::AvengerApp;
use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartAppState, ChartResizeBinding, WinitWgpuAvengerApp,
    WinitWgpuAvengerAppOptions,
};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    dataframe::DataFrame,
    prelude::{SessionContext, col, lit},
    scalar::ScalarValue,
};
use winit::{dpi::LogicalSize, window::WindowAttributes};

pub const CANVAS_SIZE: [f32; 2] = [1040.0, 620.0];
pub const PLOT_SIZE: [f32; 2] = [800.0, 390.0];
pub const AXIS_GROUP_FIELD: &str = "axis_group";
pub const AXIS_GROUP_VALUE: &str = "All";

pub const NUMERIC_DIMENSIONS: [ParallelDemoDimension; 5] = [
    ParallelDemoDimension {
        id: "speed",
        field: "speed",
        title: "Speed",
    },
    ParallelDemoDimension {
        id: "efficiency",
        field: "efficiency",
        title: "Efficiency",
    },
    ParallelDemoDimension {
        id: "stability",
        field: "stability",
        title: "Stability",
    },
    ParallelDemoDimension {
        id: "cost",
        field: "cost",
        title: "Cost",
    },
    ParallelDemoDimension {
        id: "quality",
        field: "quality",
        title: "Quality",
    },
];

#[derive(Clone, Copy)]
pub struct ParallelDemoDimension {
    pub id: &'static str,
    pub field: &'static str,
    pub title: &'static str,
}

pub fn init_diagnostics() {
    if std::env::var_os("RUST_LOG").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
            .try_init();
    } else {
        let _ = env_logger::try_init();
    }
}

pub fn run_fixed_window_app<F>(title: &str, build_app: F)
where
    F: Future<Output = AvengerApp<ChartAppState>>,
{
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app);
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title(title)
            .with_inner_size(LogicalSize::new(
                f64::from(CANVAS_SIZE[0]),
                f64::from(CANVAS_SIZE[1]),
            ))
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

pub fn app_options() -> ChartAppOptions {
    ChartAppOptions {
        resize_binding: ChartResizeBinding::none(),
        resize_throttle_ms: None,
        exact_on_resize_settle: true,
        log_metrics: true,
    }
}

pub fn demo_dataframe(ctx: &SessionContext) -> DataFrame {
    ctx.read_batch(source_batch()).expect("read demo data")
}

pub fn demo_parallel() -> Parallel {
    NUMERIC_DIMENSIONS
        .iter()
        .fold(Parallel::new(), |coord, dimension| {
            coord.dimension_with(dimension.id, |d| d.axis(|axis| axis.title(dimension.title)))
        })
}

pub fn demo_parallel_with_segment_axis() -> Parallel {
    demo_parallel().dimension_with("segment", |d| d.axis(|axis| axis.title("Segment")))
}

pub fn demo_parallel_line() -> ParallelLine {
    NUMERIC_DIMENSIONS
        .iter()
        .fold(ParallelLine::new(), |mark, dimension| {
            mark.dimension(dimension.id, col(dimension.field))
        })
}

pub fn demo_parallel_symbol() -> ParallelSymbol {
    NUMERIC_DIMENSIONS
        .iter()
        .fold(ParallelSymbol::new(), |mark, dimension| {
            mark.dimension(dimension.id, col(dimension.field))
        })
}

pub fn interval_dataframe(ctx: &SessionContext, value_min: f64, value_max: f64) -> DataFrame {
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x_min", DataType::Float64, false),
            Field::new("x_max", DataType::Float64, false),
            Field::new("value_min", DataType::Float64, false),
            Field::new("value_max", DataType::Float64, false),
        ])),
        vec![
            Arc::new(Float64Array::from(vec![0.0])) as ArrayRef,
            Arc::new(Float64Array::from(vec![1.0])),
            Arc::new(Float64Array::from(vec![value_min])),
            Arc::new(Float64Array::from(vec![value_max])),
        ],
    )
    .expect("parallel interval data");
    ctx.read_batch(batch).expect("read interval data")
}

pub fn string_list_scalar(values: &[&str]) -> ScalarValue {
    ScalarValue::List(ScalarValue::new_list(
        &values
            .iter()
            .map(|value| ScalarValue::Utf8(Some((*value).to_string())))
            .collect::<Vec<_>>(),
        &DataType::Utf8,
        true,
    ))
}

pub fn order_literal(values: &[&str]) -> datafusion::prelude::Expr {
    lit(string_list_scalar(values))
}

fn source_batch() -> RecordBatch {
    let mut sample_id = Vec::new();
    let mut segment = Vec::new();
    let mut axis_group = Vec::new();
    let mut speed = Vec::new();
    let mut efficiency = Vec::new();
    let mut stability = Vec::new();
    let mut cost = Vec::new();
    let mut quality = Vec::new();

    let segments = ["Platform", "Retail", "Operations"];
    for i in 0..120 {
        let t = i as f64;
        let segment_index = i % segments.len();
        let segment_name = segments[segment_index];
        let segment_offset = segment_index as f64 - 1.0;
        let group_wave = ((i % 10) as f64 - 4.5) * 0.82;
        let speed_value = 42.0 + (t * 0.39).sin() * 17.0 + group_wave + segment_offset * 5.0;
        let efficiency_value =
            0.58 + (t * 0.21).cos() * 0.16 - group_wave * 0.005 + segment_offset * 0.045;
        let stability_value =
            72.0 + (t * 0.16).sin() * 15.0 + (t * 0.07).cos() * 7.0 - segment_offset * 4.0;
        let cost_value = 110.0 + speed_value * 1.25 - stability_value * 0.38
            + (t * 0.31).sin() * 8.5
            - segment_offset * 7.5;
        let quality_value = stability_value * 0.53 + efficiency_value * 52.0 - cost_value * 0.075
            + speed_value * 0.13;

        sample_id.push(format!("s{i:03}"));
        segment.push(segment_name.to_string());
        axis_group.push(AXIS_GROUP_VALUE.to_string());
        speed.push(speed_value);
        efficiency.push(efficiency_value);
        stability.push(stability_value);
        cost.push(cost_value);
        quality.push(quality_value);
    }

    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("sample_id", DataType::Utf8, false),
            Field::new("segment", DataType::Utf8, false),
            Field::new(AXIS_GROUP_FIELD, DataType::Utf8, false),
            Field::new("speed", DataType::Float64, false),
            Field::new("efficiency", DataType::Float64, false),
            Field::new("stability", DataType::Float64, false),
            Field::new("cost", DataType::Float64, false),
            Field::new("quality", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(sample_id)) as ArrayRef,
            Arc::new(StringArray::from(segment)),
            Arc::new(StringArray::from(axis_group)),
            Arc::new(Float64Array::from(speed)),
            Arc::new(Float64Array::from(efficiency)),
            Arc::new(Float64Array::from(stability)),
            Arc::new(Float64Array::from(cost)),
            Arc::new(Float64Array::from(quality)),
        ],
    )
    .expect("parallel demo data")
}
