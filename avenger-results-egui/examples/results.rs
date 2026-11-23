//! The results of two chart scripts of avenger-sql-project's fixture project in a window: a chart
//! alone, and a table of a chart for each origin of the cars. `T` switches between them, and `D`
//! between the light and the dark visuals.
//!
//! ```bash
//! cargo run --release -p avenger-results-egui --example results
//! ```

use std::path::Path;
use std::sync::Arc;

use avenger_chart::results::QueryResult;
use avenger_image::ImageCacheOptions;
use avenger_results_egui::{Exec, Images, ResultView, Shared, Surface};
use avenger_sql_project::charts::label_engine;
use avenger_sql_project::{Loader, ProjectFiles};
use avenger_style::{Cascade, Preferences, Sheet};
use avenger_typst_label::LabelEngine;
use datafusion::prelude::SessionContext;
use eframe::egui;

/// A chart for each origin of the cars, beside their count.
const TABLE: &str = "FROM vega_datasets.cars
|> AGGREGATE COUNT(*) AS cars, mark_symbol(x => Horsepower, y => Miles_per_Gallon) AS chart
   GROUP BY Origin
|> ORDER BY Origin";

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 720.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "avenger-results-egui",
        options,
        Box::new(|cc| Ok(Box::new(Results::new(cc).map_err(|e| e.to_string())?))),
    )
}

struct Results {
    runtime: Exec,
    /// The results, and the one that the view shows.
    results: [Arc<QueryResult>; 2],
    shown: usize,
    view: ResultView,
    images: Images,
    engine: LabelEngine,
    sheet: Option<Sheet>,
    surface: Surface,
    cascade: Arc<Cascade>,
    preferences: Preferences,
}

impl Results {
    fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
        let runtime = Exec::new().map_err(|error| error.to_string())?;
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../avenger-sql-project/tests/projects/vega");
        let (results, sheet) = runtime.block_on(async {
            let files = ProjectFiles::find(&root).await.map_err(|e| e.to_string())?;
            let sheet = files
                .load_stylesheet()
                .await
                .map_err(|e| e.to_string())?
                .sheet;
            let source = files.source().clone();
            let (loader, _) =
                (Loader::load(files, SessionContext::new()).await).map_err(|e| e.to_string())?;
            let origins = source
                .read_text("charts/origins.av.sql")
                .await
                .map_err(|e| e.to_string())?;
            let mut results = Vec::new();
            for (path, text) in [
                ("charts/origins.av.sql", origins.as_str()),
                ("charts/by-origin.av.sql", TABLE),
            ] {
                let frame = loader
                    .run_chart(path, text)
                    .await
                    .map_err(|e| e.to_string())?;
                let schema = Arc::new(frame.schema().as_arrow().clone());
                let batches = frame.collect().await.map_err(|e| e.to_string())?;
                results.push(Arc::new(
                    QueryResult::new(schema, batches).map_err(|e| e.to_string())?,
                ));
            }
            Ok::<_, String>((results, sheet))
        })?;
        let surface = Surface::of(&cc.egui_ctx.style().visuals);
        let state = cc
            .wgpu_render_state
            .as_ref()
            .ok_or("the window has no wgpu renderer")?;
        let mut view = ResultView::new();
        view.set_result(results[0].clone(), state);
        Ok(Self {
            runtime,
            results: [results[0].clone(), results[1].clone()],
            shown: 0,
            view,
            images: Images::new(&cc.egui_ctx, ImageCacheOptions::default()),
            engine: label_engine(),
            cascade: surface.cascade(sheet.iter()),
            preferences: surface.preferences(),
            sheet,
            surface,
        })
    }
}

impl eframe::App for Results {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let Some(state) = frame.wgpu_render_state() else {
            return;
        };
        let (switch, dark) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::T),
                input.key_pressed(egui::Key::D),
            )
        });
        if dark {
            let visuals = match ctx.style().visuals.dark_mode {
                true => egui::Visuals::light(),
                false => egui::Visuals::dark(),
            };
            ctx.set_visuals(visuals);
        }
        // The charts follow the visuals: their background and their color scheme.
        let surface = Surface::of(&ctx.style().visuals);
        if surface != self.surface {
            self.surface = surface;
            self.cascade = surface.cascade(self.sheet.iter());
            self.preferences = surface.preferences();
            let shared = Shared {
                state,
                runtime: &self.runtime,
                cascade: &self.cascade,
                preferences: &self.preferences,
                engine: &self.engine,
                images: &self.images,
            };
            self.view.restyle(&shared);
        }
        if switch {
            self.shown = 1 - self.shown;
            self.view
                .set_result(self.results[self.shown].clone(), state);
        }
        egui::TopBottomPanel::top("keys").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("T: chart or table · D: light or dark");
                if self.view.shows_chart_cells() {
                    ui.checkbox(&mut self.view.interactive, "Interactive cells");
                }
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            let shared = Shared {
                state,
                runtime: &self.runtime,
                cascade: &self.cascade,
                preferences: &self.preferences,
                engine: &self.engine,
                images: &self.images,
            };
            if let Some(error) = self.view.show(ui, &shared) {
                eprintln!("{error}");
            }
        });
        if self.view.busy() {
            ctx.request_repaint();
        }
    }
}
