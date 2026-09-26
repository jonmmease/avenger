//! Retain one million point positions; pan and zoom by changing four scalar inputs.
use avenger_chart::{Chart, ChartAppState, RenderOptions};
use avenger_chart_definition::{
    dataflow::{
        arrow::{array::Float64Array, datatypes::DataType, record_batch::RecordBatch},
        datafusion::common::ScalarValue,
        *,
    },
    *,
};
use avenger_eventstream::{
    manager::EventStreamHandler,
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{EventStreamConfig, UpdateStatus},
    window::{Key, MouseButton, MouseScrollDelta, NamedKey},
};
use avenger_geometry::rtree::SceneGraphRTree;
use std::sync::{Arc, Mutex};

const NAMES: [&str; 4] = ["x_min", "x_max", "y_min", "y_max"];
fn definition(point_count: usize) -> anyhow::Result<ChartDefinition> {
    let mut seed = 42u64;
    let mut random = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((seed >> 32) as f64) / (u32::MAX as f64)
    };
    let mut x = Vec::with_capacity(point_count);
    let mut y = Vec::with_capacity(point_count);
    for i in 0..point_count {
        let r = 3.4 * random().sqrt();
        let a = (i % 4) as f64 * std::f64::consts::FRAC_PI_2 + 2.4 * r + (random() - 0.5) * 0.9;
        x.push(r * a.cos());
        y.push(r * a.sin());
    }
    let batch = RecordBatch::try_from_iter(vec![
        ("x", Arc::new(Float64Array::from(x)) as _),
        ("y", Arc::new(Float64Array::from(y)) as _),
    ])?;
    let mut flow = DataflowBuilder::new();
    let points = flow.table_snapshot(
        "points",
        TableSnapshot::from_batches(batch.schema(), vec![batch])?,
    )?;
    let points = flow.table_output("points", &points)?;
    let mut inputs = vec![];
    let mut outputs = vec![];
    for name in NAMES {
        let input = flow.scalar_input(name, DataType::Float64)?;
        let node = flow.add_scalar(format!("{name}_value"), input.expr_ref())?;
        outputs.push(flow.scalar_output(name, &node)?);
        inputs.push(input);
    }
    let mut chart = ChartDefinition::builder(flow.finish()?);
    chart.title(format!(
        "{point_count} points · Drag to pan · Scroll to zoom · Escape to reset"
    ));
    for (i, input) in inputs.iter().enumerate() {
        chart.parameter(
            NAMES[i],
            input,
            ScalarValue::Float64(Some(if i % 2 == 0 { -4. } else { 4. })),
        )?;
    }
    chart.plot("scatter", |plot| {
        plot.content_size(800., 560.);
        plot.guide_reservations(Edges::new(12., 24., 55., 65.));
        let x = plot.scale(
            "x",
            Scale::linear(Domain::bounds(&outputs[0], &outputs[1]), Range::PlotWidth),
        )?;
        let y = plot.scale(
            "y",
            Scale::linear(
                Domain::bounds(&outputs[2], &outputs[3]),
                Range::PlotHeightReversed,
            ),
        )?;
        plot.symbol(
            "points",
            &points,
            SymbolEncoding::new()
                .x(x.field("x"))
                .y(y.field("y"))
                .size(1.)
                .fill("rgba(76, 120, 168, 0.15)"),
        )?;
        plot.axis(Axis::bottom(&x).title("x").format(".2f"))?;
        plot.axis(Axis::left(&y).title("y").format(".2f"))?;
        Ok(())
    })?;
    Ok(chart.finish()?)
}
struct Drag {
    position: [f32; 2],
    bounds: [f64; 4],
}
#[derive(Default)]
struct Controls(Mutex<Option<Drag>>);
fn bounds(state: &ChartAppState) -> anyhow::Result<[f64; 4]> {
    let mut result = [0.; 4];
    for (i, name) in NAMES.iter().enumerate() {
        let ScalarValue::Float64(Some(v)) = state.parameter(name)? else {
            anyhow::bail!("expected numeric viewport")
        };
        result[i] = *v;
    }
    Ok(result)
}
fn update(state: &mut ChartAppState, bounds: [f64; 4]) -> anyhow::Result<()> {
    state.set_parameters(
        NAMES
            .into_iter()
            .zip(bounds)
            .map(|(name, v)| (name.into(), ScalarValue::Float64(Some(v)))),
    )?;
    Ok(())
}
#[async_trait::async_trait]
impl EventStreamHandler<ChartAppState> for Controls {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartAppState,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        let result = (|| -> anyhow::Result<()> {
            let rect = state.rendered().plot(&["scatter"], None).unwrap().rect;
            let inside = |p: [f32; 2]| {
                p[0] >= rect.x
                    && p[0] <= rect.x + rect.width
                    && p[1] >= rect.y
                    && p[1] <= rect.y + rect.height
            };
            let mut drag = self.0.lock().unwrap();
            match event {
                SceneGraphEvent::MouseDown(e)
                    if e.button == MouseButton::Left && inside(e.position) =>
                {
                    *drag = Some(Drag {
                        position: e.position,
                        bounds: bounds(state)?,
                    });
                }
                SceneGraphEvent::CursorMoved(e) => {
                    if let Some(d) = drag.as_ref() {
                        let dx = (e.position[0] - d.position[0]) as f64 / rect.width as f64;
                        let dy = (e.position[1] - d.position[1]) as f64 / rect.height as f64;
                        let b = d.bounds;
                        let x = dx * (b[1] - b[0]);
                        let y = dy * (b[3] - b[2]);
                        update(state, [b[0] - x, b[1] - x, b[2] + y, b[3] + y])?;
                    }
                }
                SceneGraphEvent::MouseUp(_) | SceneGraphEvent::PointerCaptureLost => {
                    *drag = None;
                }
                SceneGraphEvent::MouseWheel(e) if inside(e.position) => {
                    let delta = match e.delta {
                        MouseScrollDelta::LineDelta(_, y) => y as f64 * 20.,
                        MouseScrollDelta::PixelDelta(_, y) => y,
                    };
                    let factor = (delta * 0.002).clamp(-1., 1.).exp();
                    let mut b = bounds(state)?;
                    for (axis, anchor) in [
                        ((e.position[0] - rect.x) / rect.width) as f64,
                        1. - ((e.position[1] - rect.y) / rect.height) as f64,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let i = axis * 2;
                        let span = b[i + 1] - b[i];
                        let next = (span * factor).clamp(0.08, 800.);
                        let center = b[i] + anchor * span;
                        b[i] = center - anchor * next;
                        b[i + 1] = b[i] + next;
                    }
                    update(state, b)?;
                    *drag = None;
                }
                SceneGraphEvent::KeyPress(e) if e.key == Key::Named(NamedKey::Escape) => {
                    *drag = None;
                    update(state, [-4., 4., -4., 4.])?;
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("{e:#}");
        }
        UpdateStatus::default()
    }
}
fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let args: Vec<_> = std::env::args().collect();
    let points = args
        .iter()
        .position(|a| a == "--points")
        .map(|i| {
            args.get(i + 1)
                .ok_or_else(|| anyhow::anyhow!("--points needs a count"))?
                .parse::<usize>()
                .map_err(anyhow::Error::from)
        })
        .transpose()?
        .unwrap_or(1_000_000);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let chart = runtime.block_on(Chart::prepare(definition(points)?, chart_options()))?;
    let mut app = runtime.block_on(chart.into_app(RenderOptions::default()))?;
    let frame = app.app_state_mut().rendered();
    println!(
        "Initial frame: {:?}\n{:?}",
        frame.geometry_report(),
        frame.report()
    );
    app.register_handler(
        EventStreamConfig {
            types: vec![
                SceneGraphEventType::MouseDown,
                SceneGraphEventType::MouseUp,
                SceneGraphEventType::CursorMoved,
                SceneGraphEventType::MouseWheel,
                SceneGraphEventType::KeyPress,
                SceneGraphEventType::PointerCaptureLost,
            ],
            ..Default::default()
        },
        Arc::new(Controls::default()),
    );
    let mut options =
        avenger_winit_wgpu::WinitWgpuAvengerAppOptions::new(if cfg!(target_os = "macos") {
            2.
        } else {
            1.
        })
        .window_attributes(
            winit::window::WindowAttributes::default()
                .with_title("Chart definition · retained points")
                .with_resizable(false),
        );
    // Small, dense points do not need multisampling. Text uses its own antialiasing.
    options.canvas_config.sample_count = Some(1);
    let (mut host, event_loop) =
        avenger_winit_wgpu::WinitWgpuAvengerApp::try_new_and_event_loop_with_options(
            app, options, runtime,
        )?;
    event_loop.run_app(&mut host)?;
    if let Some(e) = host.take_fatal_error() {
        anyhow::bail!(e);
    }
    Ok(())
}

fn d3_formatting() -> avenger_scales::formatter::ScaleFormatting {
    avenger_scales::formatter::ScaleFormatting::d3(Default::default(), Default::default())
}

fn chart_options() -> avenger_chart::ChartOptions {
    avenger_chart::ChartOptions::default().with_formatting(d3_formatting())
}
