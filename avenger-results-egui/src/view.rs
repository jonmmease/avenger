//! The view of a result: one chart that fills the view, or the table of the result, whose chart
//! cells open their charts over the table until Back.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use avenger_chart::results::{QueryResult, Shape};
use avenger_chart::Chart;
use avenger_style::{Cascade, Preferences};
use avenger_typst_label::LabelEngine;
use eframe::{egui, egui_wgpu::RenderState};

use crate::charts::Images;
use crate::exec::Exec;
use crate::live::LiveChart;
use crate::paged::{PagedTable, RowSource};
use crate::table::{CellKey, CellScene, Cells, Env, Grid, LiveCells};

/// What the charts of a view share with their host: the GPU, the runtime of their evaluations and
/// their apps, the cascade and the reader's preferences, the label engine, and the images.
pub struct Shared<'a> {
    pub state: &'a RenderState,
    pub runtime: &'a Exec,
    pub cascade: &'a Arc<Cascade>,
    pub preferences: &'a Preferences,
    pub engine: &'a LabelEngine,
    pub images: &'a Images,
}

/// The view of a result. A result of one chart shows the live chart, which keeps its views and
/// selections when the result changes to another chart. Another result shows its table: the
/// static chart of each visible chart cell at the cell's size, the columns and the rows that
/// resize, and with [`ResultView::interactive`], a live chart in the cell under the pointer. The
/// expand button of a chart cell, or a double click, opens the cell's chart over the table, with
/// a Back bar of the cell's row, until Back or Escape.
pub struct ResultView {
    /// The result, with the id that its cells' scenes carry.
    result: Option<(u64, Arc<QueryResult>)>,
    /// A table of rows that a source reads as it scrolls, in place of a result, with its id.
    paged: Option<(u64, PagedTable)>,
    next: u64,
    /// The live chart of a result of one chart.
    pane: Pane,
    /// The chart cell that the user opened, which fills the view over the table until Back.
    opened: Option<(CellKey, Pane)>,
    cells: Cells,
    live_cells: LiveCells,
    /// The channel that brings the scenes of the cells' background evaluations back.
    scenes: (Sender<CellScene>, Receiver<CellScene>),
    /// Whether the chart cells of the table run their charts' apps, so that they pan, zoom, and
    /// show tooltips, one cell at a time.
    pub interactive: bool,
}

impl Default for ResultView {
    fn default() -> Self {
        Self {
            result: None,
            paged: None,
            next: 1,
            pane: Pane::default(),
            opened: None,
            cells: Cells::default(),
            live_cells: LiveCells::default(),
            scenes: channel(),
            interactive: false,
        }
    }
}

impl ResultView {
    pub fn new() -> Self {
        Self::default()
    }

    /// The result that the view shows.
    pub fn result(&self) -> Option<&Arc<QueryResult>> {
        self.result.as_ref().map(|(_, result)| result)
    }

    /// Shows `result` in place of the result before it, as after the chart script runs again: a
    /// live chart of one chart takes the new chart and keeps its views and selections, and the
    /// cells of a table draw again.
    pub fn set_result(&mut self, result: Arc<QueryResult>, state: &RenderState) {
        let id = self.next;
        self.next += 1;
        let chart = match result.shape() {
            Shape::Chart => result.chart(0, 0).cloned(),
            Shape::Table => None,
        };
        self.cells.reset(id, state);
        self.live_cells.reset(id, state);
        if let Some((_, mut opened)) = self.opened.take() {
            opened.clear(state);
        }
        match (chart, &self.pane.chart) {
            (Some(chart), Some(pane)) => pane.set_chart(chart),
            // A new chart of the pane builds when the view lays out, at the view's size.
            _ => self.pane.clear(state),
        }
        self.result = Some((id, result));
        self.paged = None;
    }

    /// Shows the rows of `source` in place of the result before them, as a table that reads its
    /// rows a page at a time as it scrolls, so a large table never collects. A table without
    /// charts shows this way.
    pub fn set_rows(&mut self, source: Arc<dyn RowSource>, state: &RenderState) {
        self.clear(state);
        let id = self.next;
        self.next += 1;
        self.paged = Some((id, PagedTable::new(source)));
    }

    /// Drops the result and the textures of its charts.
    pub fn clear(&mut self, state: &RenderState) {
        self.result = None;
        self.paged = None;
        self.cells.reset(0, state);
        self.live_cells.reset(0, state);
        if let Some((_, mut opened)) = self.opened.take() {
            opened.clear(state);
        }
        self.pane.clear(state);
    }

    /// Restyles the charts with `shared`'s cascade and preferences, as when the color scheme or a
    /// stylesheet changes: the live charts keep their views and selections, and the static charts
    /// of the cells evaluate again.
    pub fn restyle(&mut self, shared: &Shared<'_>) {
        let panes = std::iter::once(&self.pane).chain(self.opened.as_ref().map(|(_, pane)| pane));
        for chart in panes.filter_map(|pane| pane.chart.as_ref()) {
            chart.restyle(shared.cascade.clone(), shared.preferences.clone());
        }
        self.live_cells.restyle(shared.cascade, shared.preferences);
        self.cells.restyle(shared.state);
    }

    /// Whether the view shows a table with chart cells, the cells that
    /// [`ResultView::interactive`] makes live, and not a single chart or a cell's opened chart.
    pub fn shows_chart_cells(&self) -> bool {
        self.opened.is_none()
            && (self.result()).is_some_and(|result| {
                result.shape() == Shape::Table && !result.chart_columns().is_empty()
            })
    }

    /// Whether a chart of the view still works, such as a cell that evaluates in the background
    /// or a live chart whose images load, so that the host asks for another frame.
    pub fn busy(&self) -> bool {
        self.cells.busy()
            || self.pane.busy()
            || self.opened.as_ref().is_some_and(|(_, pane)| pane.busy())
            || self.live_cells.busy()
    }

    /// Shows the view in the rest of `ui`, and returns the error of a live chart that fails to
    /// build, for the host to show.
    pub fn show(&mut self, ui: &mut egui::Ui, shared: &Shared<'_>) -> Option<String> {
        while let Ok(scene) = self.scenes.1.try_recv() {
            self.cells.receive(scene);
        }
        if let Some((id, table)) = &mut self.paged {
            return table.show(ui, shared.runtime, *id);
        }
        let (id, result) = self.result.clone()?;
        let env = Env {
            state: shared.state,
            runtime: shared.runtime,
            scenes: &self.scenes.0,
            cascade: shared.cascade,
            preferences: shared.preferences,
            engine: shared.engine,
            images: shared.images,
        };
        if result.shape() == Shape::Chart {
            let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
            let chart = result.chart(0, 0).expect("a single chart");
            return self
                .pane
                .show(ui, rect, chart, egui::Id::new(("live chart", id)), &env);
        }
        if let Some((key, pane)) = &mut self.opened {
            let key = *key;
            let mut back = false;
            ui.horizontal(|ui| {
                back = ui
                    .button("Back")
                    .on_hover_text("Back to the table (Esc)")
                    .clicked();
                ui.label(row_label(&result, key));
            });
            let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
            let mut error = None;
            if let Some(chart) = result.chart(key.0, key.1) {
                let chart_id = egui::Id::new(("opened chart", id, key));
                error = pane.show(ui, rect, chart, chart_id, &env);
            }
            // Escape goes back unless it is the chart's: the chart hears keys while it has the
            // focus or the pointer.
            let escape = ui.input(|input| input.key_pressed(egui::Key::Escape))
                && ui.memory(|memory| memory.focused().is_none())
                && !ui.rect_contains_pointer(rect);
            if back || escape {
                if let Some((_, mut pane)) = self.opened.take() {
                    pane.clear(shared.state);
                }
            }
            return error;
        }
        let mut grid = Grid {
            result: &result,
            cells: &mut self.cells,
            live: &mut self.live_cells,
            interactive: self.interactive,
            env,
            expand: None,
        };
        grid.show(ui);
        let expand = grid.expand;
        self.cells.end_frame(shared.state);
        self.live_cells.end_frame(shared.state);
        if let Some(key) = expand {
            self.opened = Some((key, Pane::default()));
        }
        None
    }
}

/// The live chart that fills the view: the chart of a single-chart result, or a chart cell that
/// the user opened. It builds at the view's size the first time it shows.
#[derive(Default)]
struct Pane {
    chart: Option<LiveChart>,
    /// Whether the chart failed to build, so that it builds once.
    failed: bool,
}

impl Pane {
    /// Shows `chart` in `rect`, and builds it there the first time. Returns the error of a build
    /// that fails.
    fn show(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        chart: &Chart,
        id: egui::Id,
        env: &Env<'_>,
    ) -> Option<String> {
        let mut error = None;
        if self.chart.is_none() && !self.failed {
            let size = rect.size();
            let built = LiveChart::new(
                chart.clone(),
                env.cascade.clone(),
                env.preferences.clone(),
                env.engine.clone(),
                env.images,
                [size.x, size.y],
                id,
                env.runtime,
                ui.ctx(),
                env.state,
            );
            match built {
                Ok(chart) => self.chart = Some(chart),
                Err(message) => {
                    self.failed = true;
                    error = Some(format!("The chart cannot run: {message}"));
                }
            }
        }
        if let Some(chart) = &mut self.chart {
            chart.show(ui, rect, env.state, env.runtime);
        }
        error
    }

    fn busy(&self) -> bool {
        self.chart.as_ref().is_some_and(LiveChart::busy)
    }

    /// Drops the chart and its texture.
    fn clear(&mut self, state: &RenderState) {
        if let Some(chart) = self.chart.take() {
            chart.free(state);
        }
        self.failed = false;
    }
}

/// The values of the columns of `row` that are not chart columns, then the name of the chart
/// column of `key`, as the bar over an opened chart shows them.
fn row_label(result: &QueryResult, (row, column): CellKey) -> String {
    let fields = result.schema().fields();
    let mut parts: Vec<String> = (0..fields.len())
        .filter(|&index| !result.is_chart_column(index))
        .map(|index| {
            let value = result.value_text(row, index);
            format!(
                "{} = {}",
                fields[index].name(),
                value.as_deref().unwrap_or("null")
            )
        })
        .collect();
    parts.push(fields[column].name().clone());
    parts.join(" · ")
}
