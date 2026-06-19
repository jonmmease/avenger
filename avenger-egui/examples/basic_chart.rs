use std::sync::Arc;

use avenger_app::app::AvengerApp;
use avenger_chart::prelude::*;
use avenger_chart_app::{ChartAppOptions, ChartAppState, ChartResizeBinding, chart_avenger_app};
use avenger_common::canvas::CanvasDimensions;
use avenger_egui::{AvengerPlotHandle, Plot};
use avenger_scenegraph::scene_graph::SceneGraph;
use datafusion::{prelude::SessionContext, scalar::ScalarValue};
use eframe::egui;

fn main() -> eframe::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
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
                point_size: 120.0,
                last_error: None,
            }))
        }),
    )
}

struct BasicChartApp {
    runtime: tokio::runtime::Runtime,
    plot: AvengerPlotHandle,
    scene: Arc<SceneGraph>,
    point_size: f64,
    last_error: Option<String>,
}

impl eframe::App for BasicChartApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        for update in self
            .runtime
            .block_on(self.plot.dispatch_pending_events())
            .unwrap_or_default()
        {
            if let Some(scene) = update.scene_graph {
                self.scene = scene;
                ctx.request_repaint();
            }
            if update.status.rerender {
                ctx.request_repaint();
            }
        }

        egui::SidePanel::left("controls").show(ctx, |ui| {
            let changed = ui
                .add(egui::Slider::new(&mut self.point_size, 32.0..=240.0).text("Point size"))
                .changed();
            if changed {
                self.plot.set_param("point_size", self.point_size);
                match self.runtime.block_on(self.plot.rebuild_scene_graph(true)) {
                    Ok(Some(scene)) => {
                        self.scene = scene;
                        ctx.request_repaint();
                    }
                    Ok(None) => {}
                    Err(err) => self.last_error = Some(err.to_string()),
                }
            }

            if let Some(error) = &self.last_error {
                ui.colored_label(egui::Color32::RED, error);
            }

            let status = self.plot.frame_status();
            ui.label(format!(
                "generation: {}",
                status
                    .latest_generation
                    .map(|generation| generation.to_string())
                    .unwrap_or_else(|| "none".to_string())
            ));
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let desired_size = ui.available_size_before_wrap();
            let dimensions = CanvasDimensions {
                size: [desired_size.x.max(1.0), desired_size.y.max(1.0)],
                scale: ctx.pixels_per_point(),
            };

            if let Some(render_state) = frame.wgpu_render_state()
                && let Err(err) =
                    self.plot
                        .render_scene_to_texture(render_state, &self.scene, dimensions)
            {
                self.last_error = Some(err.to_string());
            }

            let output = Plot::new(&self.plot).desired_size(desired_size).show(ui);
            if output.response.dragged() {
                ctx.request_repaint();
            }
        });
    }
}

async fn build_app() -> AvengerApp<ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let point_size = Param::new("point_size", ScalarValue::Float64(Some(120.0)));
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
                (1.0, 2.0, 'A'),
                (2.0, 3.6, 'A'),
                (3.0, 4.2, 'B'),
                (4.0, 3.1, 'B'),
                (5.0, 5.2, 'C'),
                (6.0, 4.8, 'C'),
                (7.0, 6.5, 'D'),
                (8.0, 5.8, 'D')
            ) AS t(x, y, group_name)",
        )
        .await
        .expect("build data");

    let plot = avenger_chart::prelude::Plot::<Cartesian>::new()
        .canvas_size(760.0, 520.0)
        .add_param(point_size.clone())
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("group_name"))
                .size(point_size.expr()),
        )
        .tool(PanScrollZoom::cartesian().settle_exact(true));

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app(
        compiled,
        ctx,
        ChartAppOptions {
            resize_binding: ChartResizeBinding::none(),
            resize_throttle_ms: None,
            exact_on_resize_settle: true,
            log_metrics: false,
        },
    )
    .await
    .expect("build chart app")
}
