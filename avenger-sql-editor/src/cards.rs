//! The Examples tab: a card for each example of the gallery, in its section, with the live
//! thumbnail of its chart, and a search box over the examples. A card runs its query the first
//! time it comes into view, and keeps the result for the session.

use eframe::{egui, egui_wgpu::RenderState};

use crate::gallery::{examples, Example};
use crate::result::QueryResult;
use crate::table::{CellKey, CellScene, Cells, Env};

/// The result id of the thumbnails' cells, apart from the ids of the queries that the user runs,
/// which count up from 1.
pub const THUMBNAILS: u64 = u64::MAX;
/// The size that a thumbnail's chart lays out at, in logical pixels, which the card shows at half.
pub const LAYOUT: egui::Vec2 = egui::vec2(480.0, 300.0);
/// The size of a thumbnail on its card.
const THUMBNAIL: egui::Vec2 = egui::vec2(240.0, 150.0);
/// The height of a card's title under the thumbnail.
const TITLE_HEIGHT: f32 = 24.0;
/// The space between the cards.
const GAP: f32 = 12.0;
/// The most queries of cards that run at once.
const QUERIES: usize = 4;

/// The state of a card's query.
enum Card {
    /// Not yet in view.
    Waiting,
    Running,
    /// The result, with the first chart and the number of charts.
    Ran(Result<Ran, String>),
}

struct Ran {
    result: QueryResult,
    first: Option<CellKey>,
    charts: usize,
}

/// What the user did in the gallery this frame.
#[derive(Default)]
pub struct GalleryOutput {
    /// The examples whose queries the app starts, which came into view.
    pub start: Vec<usize>,
    /// The example that the user clicked.
    pub open: Option<usize>,
}

/// The cards of the examples, their thumbnails, and the search text.
pub struct Gallery {
    cards: Vec<Card>,
    running: usize,
    /// Whether a card in view waits for a query to start.
    waiting: bool,
    pub cells: Cells,
    pub search: String,
}

impl Default for Gallery {
    fn default() -> Self {
        Self {
            cards: examples().iter().map(|_| Card::Waiting).collect(),
            running: 0,
            waiting: false,
            cells: Cells::new(THUMBNAILS),
            search: String::new(),
        }
    }
}

impl Gallery {
    /// Keeps the result of the query of the example `index`.
    pub fn ran(&mut self, index: usize, result: Result<QueryResult, String>) {
        self.running = self.running.saturating_sub(1);
        let ran = result.map(|result| {
            let charts: Vec<CellKey> = (0..result.num_rows())
                .flat_map(|row| {
                    (result.chart_columns().into_iter()).map(move |column| (row, column))
                })
                .filter(|&(row, column)| result.chart(row, column).is_some())
                .collect();
            Ran {
                first: charts.first().copied(),
                charts: charts.len(),
                result,
            }
        });
        self.cards[index] = Card::Ran(ran);
    }

    /// Keeps the scene of a thumbnail that a background evaluation finished.
    pub fn receive(&mut self, scene: CellScene) {
        self.cells.receive(scene);
    }

    /// Whether a query or an evaluation of a card in view runs, or waits to start.
    pub fn busy(&self) -> bool {
        self.running > 0 || self.waiting || self.cells.busy()
    }

    /// Ends a frame: drops the thumbnails that the frame did not show, every one when the tab is
    /// not in view.
    pub fn end_frame(&mut self, state: &RenderState) {
        self.cells.end_frame(state);
    }

    /// The search box, for the tab row.
    pub fn search_box(&mut self, ui: &mut egui::Ui) {
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search the examples")
                .desired_width(220.0),
        );
    }

    /// Shows the cards that match the search, by section. `ready` is whether the samples are in
    /// the session, so that the queries can run.
    pub fn show(&mut self, ui: &mut egui::Ui, env: &Env<'_>, ready: bool) -> GalleryOutput {
        let mut output = GalleryOutput::default();
        self.waiting = false;
        let search = self.search.trim().to_lowercase();
        let matching: Vec<usize> = (examples().iter().enumerate())
            .filter(|(_, example)| search.is_empty() || matches(example, &search))
            .map(|(index, _)| index)
            .collect();
        if matching.is_empty() {
            ui.weak(format!("No example matches “{}”.", self.search.trim()));
            return output;
        }
        egui::ScrollArea::vertical()
            .id_salt("examples")
            .auto_shrink(false)
            .show(ui, |ui| {
                let mut rest = matching.as_slice();
                while let Some(&first) = rest.first() {
                    let section = &examples()[first].section;
                    let count = (rest.iter())
                        .take_while(|&&index| examples()[index].section == *section)
                        .count();
                    let (cards, after) = rest.split_at(count);
                    rest = after;
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.strong(section);
                        ui.weak(count.to_string());
                    });
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
                        for &index in cards {
                            if self.card(ui, index, env, ready, &mut output) {
                                output.open = Some(index);
                            }
                        }
                    });
                }
                ui.add_space(GAP);
            });
        output
    }

    /// Shows the card of the example `index`, and starts its query when it comes into view.
    /// Returns whether the user clicked it.
    fn card(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        env: &Env<'_>,
        ready: bool,
        output: &mut GalleryOutput,
    ) -> bool {
        let example = &examples()[index];
        let size = THUMBNAIL + egui::vec2(0.0, TITLE_HEIGHT);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        if !ui.is_rect_visible(rect) {
            return false;
        }
        if matches!(self.cards[index], Card::Waiting) && ready {
            match self.running < QUERIES {
                true => {
                    self.running += 1;
                    self.cards[index] = Card::Running;
                    output.start.push(index);
                }
                false => self.waiting = true,
            }
        }
        let thumbnail = egui::Rect::from_min_size(rect.min, THUMBNAIL);
        let visuals = ui.visuals().clone();
        // The panel's color, which the charts' backgrounds take, so that a thumbnail and its card
        // are one surface.
        ui.painter().rect_filled(rect, 6.0, visuals.panel_fill);
        match &self.cards[index] {
            Card::Waiting | Card::Running => {
                egui::Spinner::new().paint_at(
                    ui,
                    egui::Rect::from_center_size(thumbnail.center(), egui::vec2(16.0, 16.0)),
                );
            }
            Card::Ran(Err(error)) => message(ui, thumbnail, error, visuals.error_fg_color),
            Card::Ran(Ok(ran)) => match ran.first {
                Some(key) => {
                    let chart = ran.result.chart(key.0, key.1).expect("the first chart");
                    let inner = thumbnail.shrink(1.0);
                    self.cells.show(ui, inner, (index, 0), chart, LAYOUT, env);
                    if ran.charts > 1 {
                        badge(ui, thumbnail, ran.charts);
                    }
                }
                None => message(ui, thumbnail, "No chart", visuals.weak_text_color()),
            },
        }
        let title = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 8.0, thumbnail.bottom()),
            rect.right_bottom() - egui::vec2(8.0, 0.0),
        );
        // A child ui, which takes no space in the row of cards.
        let mut label = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(title)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        label.add(
            egui::Label::new(&example.title)
                .truncate()
                .selectable(false),
        );
        let stroke = match response.hovered() {
            true => egui::Stroke::new(1.5, visuals.widgets.hovered.fg_stroke.color),
            false => visuals.widgets.noninteractive.bg_stroke,
        };
        ui.painter()
            .rect_stroke(rect, 6.0, stroke, egui::StrokeKind::Inside);
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let response = response.on_hover_ui(|ui| {
            ui.set_max_width(360.0);
            ui.strong(&example.title);
            ui.label(&example.description);
        });
        response.clicked()
    }
}

/// Whether `example` holds the lowercase text `search` in its title, its description, its
/// section, or its query, ignoring case.
fn matches(example: &Example, search: &str) -> bool {
    [
        &example.title,
        &example.description,
        &example.section,
        &example.sql,
    ]
    .iter()
    .any(|text| text.to_lowercase().contains(search))
}

/// A short message in a thumbnail, such as the error of its query.
fn message(ui: &egui::Ui, rect: egui::Rect, text: &str, color: egui::Color32) {
    let galley = ui.painter().layout(
        text.to_string(),
        egui::FontId::proportional(11.0),
        color,
        rect.width() - 16.0,
    );
    let clip = rect.shrink(8.0);
    ui.painter()
        .with_clip_rect(clip)
        .galley(clip.left_top(), galley, color);
}

/// The number of charts of a result of several, at the thumbnail's top-right corner.
fn badge(ui: &egui::Ui, rect: egui::Rect, charts: usize) {
    let painter = ui.painter();
    let text = format!("×{charts}");
    let galley =
        painter.layout_no_wrap(text, egui::FontId::proportional(11.0), egui::Color32::WHITE);
    let size = galley.size() + egui::vec2(10.0, 4.0);
    let badge = egui::Rect::from_min_size(
        egui::pos2(rect.right() - size.x - 6.0, rect.top() + 6.0),
        size,
    );
    painter.rect_filled(badge, 4.0, egui::Color32::from_black_alpha(160));
    painter.galley(
        badge.center() - galley.size() / 2.0,
        galley,
        egui::Color32::WHITE,
    );
}
