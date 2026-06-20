use std::sync::Arc;

use avenger_app::app::AvengerApp;
use avenger_chart::prelude::*;
use avenger_chart_app::{ChartAppOptions, ChartAppState, ChartResizeBinding, chart_avenger_app};
use avenger_common::canvas::CanvasDimensions;
use avenger_egui::{AvengerPlotHandle, Plot};
use avenger_scenegraph::scene_graph::SceneGraph;
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
use eframe::egui;

const POINT_COUNT: usize = 10_000;

fn main() -> eframe::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    let avenger_app = runtime.block_on(build_app());
    let scene = avenger_app.scene_graph_arc();
    let plot = AvengerPlotHandle::from_app(avenger_app);
    let native_options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Avenger egui basic chart",
        native_options,
        Box::new(move |creation| {
            assert!(
                creation.wgpu_render_state.is_some(),
                "basic_chart requires eframe's WGPU renderer"
            );
            Ok(Box::new(BasicChartApp {
                runtime,
                plot,
                scene,
                scene_generation: 0,
                rendered_scene_generation: None,
                rendered_dimensions: None,
                point_size: 120.0,
                show_points: true,
                last_error: None,
            }))
        }),
    )
}

struct BasicChartApp {
    runtime: tokio::runtime::Runtime,
    plot: AvengerPlotHandle,
    scene: Arc<SceneGraph>,
    scene_generation: u64,
    rendered_scene_generation: Option<u64>,
    rendered_dimensions: Option<CanvasDimensions>,
    point_size: f64,
    show_points: bool,
    last_error: Option<String>,
}

impl eframe::App for BasicChartApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.poll_latest_scene(ctx);
        self.last_error = self.plot.latest_scene_error();

        egui::SidePanel::left("controls").show(ctx, |ui| {
            ui.label(format!("{POINT_COUNT} points"));

            let changed = ui
                .add(egui::Slider::new(&mut self.point_size, 32.0..=240.0).text("Point size"))
                .changed();
            if changed {
                if self.plot.set_param("point_size", self.point_size).changed {
                    self.request_chart_rebuild(ctx);
                }
            }

            if ui.checkbox(&mut self.show_points, "Show points").changed() {
                if self.plot.set_param("show_points", self.show_points).changed {
                    self.request_chart_rebuild(ctx);
                }
            }

            if let Some(error) = &self.last_error {
                ui.colored_label(egui::Color32::RED, error);
            }

            let status = self.plot.frame_status();
            ui.label(format!(
                "texture generation: {}",
                status
                    .latest_generation
                    .map(|generation| generation.to_string())
                    .unwrap_or_else(|| "none".to_string())
            ));
            ui.label(format!(
                "scene generation: {} / requested {}",
                status
                    .latest_scene_generation
                    .map(|generation| generation.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                status
                    .requested_generation
                    .map(|generation| generation.to_string())
                    .unwrap_or_else(|| "none".to_string()),
            ));
            ui.label(if status.render_pending {
                "render pending"
            } else {
                "render idle"
            });
            ui.separator();
            self.plot.show_metrics(ui);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let desired_size = ui.available_size_before_wrap();
            let dimensions = scene_dimensions(&self.scene, ctx);

            self.render_latest_scene_if_needed(ctx, frame, dimensions);

            let output = Plot::new(&self.plot).desired_size(desired_size).show(ui);
            if !output.events.is_empty() {
                self.plot
                    .request_event_dispatch_with_repaint(self.runtime.handle(), ctx);
            }
            if output.response.dragged() {
                ctx.request_repaint();
            }
            if output.frame_status.render_pending {
                ctx.request_repaint();
            }
        });
    }
}

impl BasicChartApp {
    fn request_chart_rebuild(&mut self, ctx: &egui::Context) {
        self.plot
            .request_scene_rebuild_with_repaint(self.runtime.handle(), ctx, true);
        ctx.request_repaint();
    }

    fn poll_latest_scene(&mut self, ctx: &egui::Context) {
        if let Some(frame) = self.plot.latest_scene_frame() {
            let generation = frame.generation.get();
            if generation != self.scene_generation {
                self.scene = frame.payload.clone();
                self.scene_generation = generation;
                ctx.request_repaint();
            }
        }
    }

    fn render_latest_scene_if_needed(
        &mut self,
        ctx: &egui::Context,
        frame: &mut eframe::Frame,
        dimensions: CanvasDimensions,
    ) {
        let dimensions_changed = self
            .rendered_dimensions
            .is_none_or(|rendered| !canvas_dimensions_eq(rendered, dimensions));
        let scene_changed = self.rendered_scene_generation != Some(self.scene_generation);
        if !dimensions_changed && !scene_changed {
            return;
        }

        if let Some(render_state) = frame.wgpu_render_state()
            && let Err(err) = self
                .plot
                .request_background_scene_texture_with_repaint(
                    render_state,
                    ctx,
                    self.scene_generation,
                    self.scene.clone(),
                    dimensions,
                )
                .map(|status| {
                    if status.render_pending {
                        ctx.request_repaint();
                    }
                    if status.scene_generation == Some(self.scene_generation)
                        && status
                            .dimensions
                            .is_some_and(|rendered| canvas_dimensions_eq(rendered, dimensions))
                    {
                        self.rendered_scene_generation = Some(self.scene_generation);
                        self.rendered_dimensions = Some(dimensions);
                    }
                })
        {
            self.last_error = Some(err.to_string());
        }
    }
}

fn canvas_dimensions_eq(left: CanvasDimensions, right: CanvasDimensions) -> bool {
    left.size == right.size && left.scale == right.scale
}

fn scene_dimensions(scene: &SceneGraph, ctx: &egui::Context) -> CanvasDimensions {
    CanvasDimensions {
        size: [scene.width.max(1.0), scene.height.max(1.0)],
        scale: ctx.pixels_per_point(),
    }
}

async fn build_app() -> AvengerApp<ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let width = Param::new("width", ScalarValue::Float64(Some(760.0)));
    let height = Param::new("height", ScalarValue::Float64(Some(520.0)));
    let point_size = Param::new("point_size", ScalarValue::Float64(Some(120.0)));
    let show_points = Param::new("show_points", ScalarValue::Boolean(Some(true)));
    let size = when(show_points.expr(), point_size.expr())
        .otherwise(lit(0.0))
        .expect("build point-size conditional");
    let df = ctx
        .read_batch(make_points_batch(POINT_COUNT))
        .expect("read generated points");

    let plot = avenger_chart::prelude::Plot::<Cartesian>::new()
        .canvas_size(width.expr(), height.expr())
        .add_param(width)
        .add_param(height)
        .add_param(point_size.clone())
        .add_param(show_points)
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("group_name"))
                .size(size),
        )
        .tool(PanScrollZoom::cartesian().settle_exact(true));

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app(
        compiled,
        ctx,
        ChartAppOptions {
            resize_binding: ChartResizeBinding::width_height("width", "height"),
            resize_throttle_ms: None,
            exact_on_resize_settle: true,
            log_metrics: false,
        },
    )
    .await
    .expect("build chart app")
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
