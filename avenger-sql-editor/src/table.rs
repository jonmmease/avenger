//! The result table on `egui_table`: a column for each column of the result, and in each chart
//! column, the static chart of each visible row at the cell's size. One cell at a time can hold
//! the live chart instead.

use avenger_common::time::{Duration, Instant};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_style::Cascade;
use avenger_typst_label::LabelEngine;
use eframe::{egui, egui_wgpu::RenderState};
use egui_table::{CellInfo, Column, HeaderCellInfo, HeaderRow, Table, TableDelegate};

use avenger_chart_eval::EvalError;

use crate::charts::StaticChart;
use crate::exec::Exec;
use crate::gpu::{self, Painter, Texture};
use crate::live::LiveChart;
use crate::result::QueryResult;

/// The size of a chart cell when a result arrives, in points.
pub const CELL_WIDTH: f32 = 360.0;
pub const ROW_HEIGHT: f32 = 240.0;
/// The shade of every other row: about 4% darker than the panel, twice the difference of egui's
/// `faint_bg_color`, which lightens by 5 of 255.
const STRIPE: egui::Color32 = egui::Color32::from_black_alpha(10);
/// The height of the strip along the bottom of a cell that resizes the rows.
const HANDLE: f32 = 6.0;
/// The side of the square expand button at the top right corner of a chart cell.
const EXPAND_SIZE: f32 = 22.0;
/// The height of a row without charts.
const TEXT_ROW_HEIGHT: f32 = 24.0;
/// The limits of the row height that the row handle sets.
const ROW_HEIGHTS: (f32, f32) = (60.0, 1200.0);
/// The scenes that a cell keeps, at its most recent sizes, so that a size it had draws at once.
const SCENES_KEPT: usize = 3;
/// The time that a frame spends on evaluations of chart cells, and on builds of their live
/// charts, which draw in the frame. The rest evaluate in the background, or build in a later
/// frame, and draw when they finish. A penguins chart prepares in about 20 µs and evaluates in
/// about 0.5 ms once the label engine has warmed up.
const FRAME_BUDGET: Duration = Duration::from_millis(8);

/// A cell of a chart column, by row and column.
pub type CellKey = (usize, usize);

/// A size in physical pixels.
type Pixels = [u32; 2];

/// The scene of a chart cell that a background evaluation finished, for the result `result`.
pub struct CellScene {
    pub result: u64,
    pub cell: CellKey,
    pub pixels: Pixels,
    pub scene: Result<Arc<SceneGraph>, String>,
}

/// What the chart cells share with the app: the GPU, the runtime of the evaluations, the channel
/// that brings their scenes back, and the stylesheet and the label engine.
pub struct Env<'a> {
    pub state: &'a RenderState,
    pub runtime: &'a Exec,
    pub scenes: &'a Sender<CellScene>,
    pub cascade: &'a Arc<Cascade>,
    pub engine: &'a LabelEngine,
}

#[derive(Default)]
struct Cell {
    /// The texture of the cell and the size that it drew at.
    texture: Option<(Texture, Pixels)>,
    scenes: VecDeque<(Pixels, Arc<SceneGraph>)>,
    error: Option<String>,
}

/// The chart cells of the result table, and the row height that their row handle sets.
pub struct Cells {
    result: u64,
    pub row_height: f32,
    cells: HashMap<CellKey, Cell>,
    evaluators: HashMap<CellKey, Arc<Mutex<Option<StaticChart>>>>,
    /// The cells that evaluate in the background, one evaluation at a time each.
    in_flight: HashSet<CellKey>,
    painter: Option<Painter>,
    /// The cells that this frame shows.
    shown: HashSet<CellKey>,
    /// The time that this frame spent on evaluations.
    spent: Duration,
    /// The row whose bottom edge the pointer is on or drags, last frame and this frame, which
    /// draws darker across every column.
    edge_row: Option<usize>,
    next_edge_row: Option<usize>,
}

impl Default for Cells {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Cells {
    /// The chart cells of the result `result`.
    pub fn new(result: u64) -> Self {
        Self {
            result,
            row_height: ROW_HEIGHT,
            cells: HashMap::new(),
            evaluators: HashMap::new(),
            in_flight: HashSet::new(),
            painter: None,
            shown: HashSet::new(),
            spent: Duration::ZERO,
            edge_row: None,
            next_edge_row: None,
        }
    }

    /// Drops the cells of the previous result, for the result `result`. The row height stays.
    pub fn reset(&mut self, result: u64, state: &RenderState) {
        for (_, cell) in self.cells.drain() {
            if let Some((texture, _)) = cell.texture {
                texture.free(state);
            }
        }
        self.evaluators.clear();
        self.in_flight.clear();
        self.result = result;
    }

    /// Whether an evaluation runs in the background.
    pub fn busy(&self) -> bool {
        !self.in_flight.is_empty()
    }

    /// Keeps the scene of a finished evaluation, when it belongs to the current result.
    pub fn receive(&mut self, scene: CellScene) {
        if scene.result != self.result {
            return;
        }
        self.in_flight.remove(&scene.cell);
        if !self.evaluators.contains_key(&scene.cell) {
            // The cell left the view while it evaluated.
            return;
        }
        let cell = self.cells.entry(scene.cell).or_default();
        match scene.scene {
            Ok(graph) => keep(cell, scene.pixels, graph),
            Err(error) => cell.error = Some(error),
        }
    }

    /// Ends a frame of the table: drops the cells that the frame did not show.
    pub fn end_frame(&mut self, state: &RenderState) {
        self.spent = Duration::ZERO;
        self.edge_row = self.next_edge_row.take();
        let shown = std::mem::take(&mut self.shown);
        let hidden: Vec<CellKey> = (self.cells.keys())
            .chain(self.evaluators.keys())
            .filter(|key| !shown.contains(key))
            .copied()
            .collect();
        for key in hidden {
            if let Some(Cell {
                texture: Some((texture, _)),
                ..
            }) = self.cells.remove(&key)
            {
                texture.free(state);
            }
            self.evaluators.remove(&key);
        }
    }

    /// Keeps the static chart of `key` while the cell shows its live chart, so that the cell
    /// shows its image at once when the table's charts stop being interactive.
    fn keep(&mut self, key: CellKey) {
        self.shown.insert(key);
    }

    /// Shows the static chart `chart` of `key` in `rect`, laid out at `layout` logical pixels and
    /// scaled to `rect`, so that a chart laid out larger than its cell draws smaller with the
    /// texture's pixels still one to one. The cell evaluates in the frame while the frame's
    /// budget lasts, so that a resize, or a cell that comes into view, draws at once. Over the
    /// budget, the cell evaluates in the background and stretches its last image to the cell
    /// until its scene arrives.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        key: CellKey,
        chart: &avenger_chart::Chart,
        layout: egui::Vec2,
        env: &Env<'_>,
    ) {
        self.shown.insert(key);
        let scale = ui.ctx().pixels_per_point() * rect.width() / layout.x;
        let dimensions = gpu::dimensions(layout, scale);
        let pixels = [
            dimensions.to_physical_width(),
            dimensions.to_physical_height(),
        ];
        let cell = self.cells.entry(key).or_default();
        let drawn = cell.texture.as_ref().map(|(_, drawn)| *drawn);
        if drawn != Some(pixels) && cell.error.is_none() {
            let mut scene = (cell.scenes.iter())
                .find(|(size, _)| *size == pixels)
                .map(|(_, scene)| scene.clone());
            if scene.is_none() && !self.in_flight.contains(&key) {
                let evaluator = self.evaluators.entry(key).or_default().clone();
                let started = Instant::now();
                let evaluated = match self.spent < FRAME_BUDGET {
                    true => (evaluator.try_lock().ok())
                        .map(|mut guard| scene_of(&mut guard, chart, env, dimensions.size)),
                    false => None,
                };
                if let Some(evaluated) = evaluated {
                    self.spent += started.elapsed();
                    match evaluated {
                        Ok(graph) => {
                            let graph = Arc::new(graph);
                            keep(cell, pixels, graph.clone());
                            scene = Some(graph);
                        }
                        Err(error) => cell.error = Some(error.to_string()),
                    }
                } else {
                    self.in_flight.insert(key);
                    evaluate(
                        evaluator,
                        chart.clone(),
                        self.result,
                        key,
                        pixels,
                        dimensions.size,
                        ui.ctx(),
                        env,
                    );
                }
            }
            if let Some(scene) = scene {
                let texture = match cell.texture.take() {
                    Some((mut texture, _)) => {
                        texture.resize(env.state, dimensions);
                        texture
                    }
                    None => Texture::new(env.state, dimensions),
                };
                let painter = self
                    .painter
                    .get_or_insert_with(|| Painter::new(env.state, None));
                painter.draw(env.state, &texture, [scene.as_ref()], env.engine);
                cell.texture = Some((texture, pixels));
            }
        }
        if let Some((texture, _)) = &cell.texture {
            // A texture at another size stretches to the cell until the new one draws.
            ui.painter()
                .image(texture.id, rect, texture.uv(), egui::Color32::WHITE);
        } else if let Some(error) = &cell.error {
            ui.painter().text(
                rect.left_top() + egui::vec2(6.0, 6.0),
                egui::Align2::LEFT_TOP,
                error,
                egui::FontId::proportional(11.0),
                ui.visuals().error_fg_color,
            );
        }
    }
}

/// The scene of `chart` at `size`, from the cell's evaluator, which prepares the chart on its
/// first evaluation and keeps it for the next sizes.
fn scene_of(
    evaluator: &mut Option<StaticChart>,
    chart: &avenger_chart::Chart,
    env: &Env<'_>,
    size: [f32; 2],
) -> Result<SceneGraph, EvalError> {
    match evaluator {
        Some(evaluator) => evaluator.scene(size),
        empty => prepare(
            empty,
            chart.clone(),
            env.cascade.clone(),
            env.engine.clone(),
            size,
        ),
    }
}

/// Prepares `chart` into the empty `evaluator`, and evaluates it at `size`.
fn prepare(
    evaluator: &mut Option<StaticChart>,
    chart: avenger_chart::Chart,
    cascade: Arc<Cascade>,
    engine: LabelEngine,
    size: [f32; 2],
) -> Result<SceneGraph, EvalError> {
    let mut prepared = StaticChart::new(chart, cascade, engine)?;
    let scene = prepared.scene(size);
    *evaluator = Some(prepared);
    scene
}

/// Keeps `scene` as the scene of `cell` at `pixels`, with at most [`SCENES_KEPT`] scenes.
fn keep(cell: &mut Cell, pixels: Pixels, scene: Arc<SceneGraph>) {
    cell.scenes.retain(|(size, _)| *size != pixels);
    cell.scenes.push_back((pixels, scene));
    while cell.scenes.len() > SCENES_KEPT {
        cell.scenes.pop_front();
    }
}

/// Evaluates the chart of `key` at `size` on the runtime's blocking threads, and sends its scene
/// back. The cell's evaluator prepares the chart on its first evaluation and keeps it for the
/// next sizes.
#[allow(clippy::too_many_arguments)]
fn evaluate(
    evaluator: Arc<Mutex<Option<StaticChart>>>,
    chart: avenger_chart::Chart,
    result: u64,
    key: CellKey,
    pixels: Pixels,
    size: [f32; 2],
    ctx: &egui::Context,
    env: &Env<'_>,
) {
    let (cascade, engine) = (env.cascade.clone(), env.engine.clone());
    let (scenes, ctx) = (env.scenes.clone(), ctx.clone());
    env.runtime.spawn_blocking(move || {
        let mut evaluator = evaluator
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let scene = match &mut *evaluator {
            Some(evaluator) => evaluator.scene(size),
            empty => prepare(empty, chart, cascade, engine, size),
        };
        let scene = scene.map(Arc::new).map_err(|error| error.to_string());
        let _ = scenes.send(CellScene {
            result,
            cell: key,
            pixels,
            scene,
        });
        ctx.request_repaint();
    });
}

/// The live charts of the chart cells while the table's charts are interactive: one for each
/// visible cell, built when the cell comes into view and dropped when it leaves.
#[derive(Default)]
pub struct LiveCells {
    result: u64,
    charts: HashMap<CellKey, LiveChart>,
    /// The cells whose live charts failed to build, which show their static charts.
    failed: HashSet<CellKey>,
    /// The cells that this frame shows, and the time that this frame spent building charts.
    shown: HashSet<CellKey>,
    spent: Duration,
}

impl LiveCells {
    /// Drops the live charts, for the result `result`.
    pub fn reset(&mut self, result: u64, state: &RenderState) {
        self.clear(state);
        self.failed.clear();
        self.result = result;
    }

    /// Drops the live charts.
    pub fn clear(&mut self, state: &RenderState) {
        for (_, chart) in self.charts.drain() {
            chart.free(state);
        }
    }

    pub fn busy(&self) -> bool {
        self.charts.values().any(LiveChart::busy)
    }

    /// Ends a frame of the table: drops the live charts of the cells that the frame did not show.
    pub fn end_frame(&mut self, state: &RenderState) {
        self.spent = Duration::ZERO;
        let shown = std::mem::take(&mut self.shown);
        let hidden: Vec<CellKey> = (self.charts.keys())
            .filter(|key| !shown.contains(key))
            .copied()
            .collect();
        for key in hidden {
            if let Some(chart) = self.charts.remove(&key) {
                chart.free(state);
            }
        }
    }

    /// Shows the live chart of `key` in `rect`, and builds it at the cell's size when the cell
    /// comes into view, within the frame's budget. Returns false when the cell has no live chart
    /// this frame, so that it shows its static chart.
    fn show(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        key: CellKey,
        result: &QueryResult,
        env: &Env<'_>,
    ) -> bool {
        self.shown.insert(key);
        if !self.charts.contains_key(&key) {
            let Some(chart) = result.chart(key.0, key.1) else {
                return false;
            };
            if self.failed.contains(&key) {
                return false;
            }
            if self.spent >= FRAME_BUDGET {
                ui.ctx().request_repaint();
                return false;
            }
            let started = Instant::now();
            let size = rect.size();
            let built = LiveChart::new(
                chart.clone(),
                env.cascade.clone(),
                env.engine.clone(),
                [size.x, size.y],
                egui::Id::new(("live cell", self.result, key)),
                env.runtime,
                ui.ctx(),
                env.state,
            );
            self.spent += started.elapsed();
            match built {
                Ok(chart) => {
                    self.charts.insert(key, chart);
                }
                Err(_) => {
                    self.failed.insert(key);
                    return false;
                }
            }
        }
        let chart = self
            .charts
            .get_mut(&key)
            .expect("the live chart of the cell");
        chart.show(ui, rect, env.state, env.runtime);
        true
    }
}

/// The delegate of the result table for one frame.
pub struct Grid<'a> {
    pub result: &'a QueryResult,
    pub cells: &'a mut Cells,
    pub live: &'a mut LiveCells,
    /// Whether the charts of the cells are live charts, which take the pointer and the wheel.
    pub interactive: bool,
    pub env: Env<'a>,
    /// The chart cell whose expand button the user clicked, or that the user double-clicked,
    /// this frame, which opens in the result pane.
    pub expand: Option<CellKey>,
}

impl Grid<'_> {
    /// Shows the table in the rest of `ui`.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let fields = self.result.schema().fields().clone();
        let columns: Vec<Column> = fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let id = egui::Id::new(("result column", field.name()));
                if self.result.is_chart_column(index) {
                    Column::new(CELL_WIDTH)
                        .range(egui::Rangef::new(120.0, 4000.0))
                        .resizable(true)
                        .id(id)
                } else {
                    Column::new(100.0)
                        .range(egui::Rangef::new(40.0, 480.0))
                        .resizable(true)
                        .id(id)
                }
            })
            .collect();
        // The columns before the first chart column, such as the keys of a `GROUP BY`, stay
        // in view while the charts scroll.
        let sticky = match self.result.chart_columns().first() {
            Some(&first) => first,
            None => 0,
        };
        Table::new()
            .id_salt("result")
            .num_rows(self.result.num_rows() as u64)
            .columns(columns)
            .num_sticky_cols(sticky)
            .headers([HeaderRow::new(40.0)])
            .show(ui, self);
    }

    /// The expand button, flush with the top-right corner of the chart cell `key`, while the
    /// pointer is over the cell. The button paints without taking space, so that the column can
    /// shrink.
    fn expand_button(&mut self, ui: &mut egui::Ui, rect: egui::Rect, key: CellKey) {
        if self.result.chart(key.0, key.1).is_none() || !ui.rect_contains_pointer(rect) {
            return;
        }
        let button = egui::Rect::from_min_size(
            egui::pos2(rect.right() - EXPAND_SIZE, rect.top()),
            egui::vec2(EXPAND_SIZE, EXPAND_SIZE),
        );
        let response = ui
            .interact(button, egui::Id::new(("expand", key)), egui::Sense::click())
            .on_hover_text("Expand");
        let visuals = ui.style().interact(&response);
        let painter = ui.painter();
        // The corner of the cell stays square, and the corner inside the cell is round.
        let corners = egui::CornerRadius {
            sw: 4,
            ..egui::CornerRadius::ZERO
        };
        painter.rect(
            button,
            corners,
            ui.visuals().window_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        expand_icon(painter, button.shrink(6.0), visuals.fg_stroke.color);
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            self.expand = Some(key);
        }
    }

    /// The bottom edge of the cell at `row` and `column`: a light line between the rows, and a
    /// strip that resizes every row when dragged. While the pointer is on the edge of a row, or
    /// drags it, the line of that row draws darker in every column. Each cell has its own strip,
    /// so that the cells of a row do not share an id.
    fn row_edge(&mut self, ui: &mut egui::Ui, rect: egui::Rect, row: usize, column: usize) {
        let strip = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - HANDLE),
            rect.right_bottom(),
        );
        let response = ui.interact(
            strip,
            egui::Id::new(("row handle", row, column)),
            egui::Sense::drag(),
        );
        let active = response.hovered() || response.dragged();
        if active {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeRow);
            self.cells.next_edge_row = Some(row);
        }
        let stroke = if active || self.cells.edge_row == Some(row) {
            egui::Stroke::new(1.5, ui.visuals().weak_text_color())
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        };
        ui.painter()
            .hline(rect.x_range(), rect.bottom() - stroke.width / 2.0, stroke);
        if response.dragged() {
            // The edge stays under the pointer: every row above it grows by the same amount.
            let delta = response.drag_delta().y / (row + 1) as f32;
            let (low, high) = ROW_HEIGHTS;
            self.cells.row_height = (self.cells.row_height + delta).clamp(low, high);
        }
    }
}

impl TableDelegate for Grid<'_> {
    fn header_cell_ui(&mut self, ui: &mut egui::Ui, cell: &HeaderCellInfo) {
        let column = cell.col_range.start;
        let Some(field) = self.result.schema().fields().get(column) else {
            return;
        };
        let kind = if self.result.is_chart_column(column) {
            "chart".to_string()
        } else {
            field.data_type().to_string()
        };
        underline(ui);
        padded(ui, |ui| {
            ui.vertical(|ui| {
                ui.add(egui::Label::new(egui::RichText::new(field.name()).strong()).extend());
                ui.add(egui::Label::new(egui::RichText::new(kind).small().weak()).extend());
            });
        });
    }

    fn row_ui(&mut self, ui: &mut egui::Ui, row: u64) {
        if row % 2 == 1 {
            ui.painter().rect_filled(ui.max_rect(), 0.0, STRIPE);
        }
    }

    fn cell_ui(&mut self, ui: &mut egui::Ui, cell: &CellInfo) {
        let (row, column) = (cell.row_nr as usize, cell.col_nr);
        let rect = ui.max_rect();
        if self.result.is_chart_column(column) {
            if ui.is_sizing_pass() {
                // The table sizes its columns to their contents on its first frame and on a
                // double click of a column edge. A chart cell paints without taking space, so
                // that the column can shrink when dragged, so here it takes the default width.
                ui.allocate_space(egui::vec2(CELL_WIDTH, 1.0));
                return;
            }
            let key = (row, column);
            let live = self.interactive && self.live.show(ui, rect, key, self.result, &self.env);
            if live {
                self.cells.keep(key);
            } else {
                if let Some(chart) = self.result.chart(row, column) {
                    self.cells
                        .show(ui, rect, key, chart, rect.size(), &self.env);
                }
                let response = ui.interact(
                    rect,
                    egui::Id::new(("chart cell", key)),
                    egui::Sense::click(),
                );
                if response.double_clicked() && self.result.chart(row, column).is_some() {
                    self.expand = Some(key);
                }
            }
            self.expand_button(ui, rect, key);
            self.row_edge(ui, rect, row, column);
            return;
        }
        padded(ui, |ui| match self.result.value_text(row, column) {
            Some(text) => ui.add(egui::Label::new(egui::RichText::new(text).monospace()).extend()),
            None => {
                ui.add(egui::Label::new(egui::RichText::new("null").weak().monospace()).extend())
            }
        });
        if !self.result.chart_columns().is_empty() {
            self.row_edge(ui, rect, row, column);
        }
    }

    fn default_row_height(&self) -> f32 {
        if self.result.chart_columns().is_empty() {
            TEXT_ROW_HEIGHT
        } else {
            self.cells.row_height
        }
    }
}

/// A hairline along the bottom of the ui's rect, in the stroke of the table's column lines.
fn underline(ui: &egui::Ui) {
    let rect = ui.max_rect();
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}

/// Paints the expand icon in `rect`: the two diagonals of `rect`, with an arrowhead at each corner
/// that points out of the rectangle.
fn expand_icon(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.5, color);
    let head = rect.width() * 0.4;
    painter.line_segment([rect.left_top(), rect.right_bottom()], stroke);
    painter.line_segment([rect.right_top(), rect.left_bottom()], stroke);
    for corner in [
        rect.left_top(),
        rect.right_top(),
        rect.left_bottom(),
        rect.right_bottom(),
    ] {
        // The arrowhead runs along the two edges of its corner, toward the inside of `rect`.
        let inward = rect.center() - corner;
        let along_x = egui::vec2(inward.x.signum() * head, 0.0);
        let along_y = egui::vec2(0.0, inward.y.signum() * head);
        painter.line_segment([corner, corner + along_x], stroke);
        painter.line_segment([corner, corner + along_y], stroke);
    }
}

/// Cell contents with room on either side, so that columns sized to their contents don't touch.
fn padded<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, add)
        .inner
}
