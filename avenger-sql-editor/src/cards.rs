//! The Gallery tab: a card for each chart script of the open project, in its section, with the
//! live thumbnail of its chart, and a search box over the charts. A card runs its chart the first
//! time it comes into view, and keeps the result until the chart's file or the tables that it
//! reads change.

use eframe::{egui, egui_wgpu::RenderState};

use crate::gallery::Entry;
use crate::result::QueryResult;
use crate::table::{CellKey, CellScene, Cells, Env};

/// The result id of the thumbnails' cells, apart from the ids of the runs of the tabs, which count
/// up from 1.
pub const THUMBNAILS: u64 = u64::MAX;
/// The size that a thumbnail's chart lays out at, in logical pixels, which the card shows at half.
pub const LAYOUT: egui::Vec2 = egui::vec2(480.0, 300.0);
/// The size of a thumbnail on its card.
const THUMBNAIL: egui::Vec2 = egui::vec2(240.0, 150.0);
/// The height of a card's title under the thumbnail.
const TITLE_HEIGHT: f32 = 24.0;
/// The space between the cards.
const GAP: f32 = 12.0;
/// The most charts of cards that run at once.
const QUERIES: usize = 4;

/// A card: its chart script, the count of the changes of its result, which keys its thumbnail,
/// and the state of its run.
struct Card {
    entry: Entry,
    version: usize,
    run: Run,
}

impl Card {
    /// Runs the card again when it is in view, under a new version, whose thumbnail draws anew.
    fn reset(&mut self) {
        self.version += 1;
        self.run = Run::Waiting;
    }
}

/// The state of a card's run.
enum Run {
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

/// A run of a card's chart that the app starts: the card's index and version, and the path and
/// the text of its chart script.
pub struct Start {
    pub index: usize,
    pub version: usize,
    pub path: String,
    pub text: String,
}

/// What the user did in the gallery this frame.
#[derive(Default)]
pub struct GalleryOutput {
    /// The runs of the cards that came into view.
    pub start: Vec<Start>,
    /// The path of the chart script of the card that the user clicked.
    pub open: Option<String>,
}

/// The cards of the open project's chart scripts, their thumbnails, and the search text.
pub struct Gallery {
    cards: Vec<Card>,
    running: usize,
    /// Whether a card in view waits for its run to start.
    waiting: bool,
    pub cells: Cells,
    pub search: String,
}

impl Default for Gallery {
    fn default() -> Self {
        Self {
            cards: Vec::new(),
            running: 0,
            waiting: false,
            cells: Cells::new(THUMBNAILS),
            search: String::new(),
        }
    }
}

impl Gallery {
    /// Shows the cards of `entries` in place of the cards before, whose thumbnails it drops. The
    /// runs of the cards before end unseen: the app tells them apart by the project's generation.
    pub fn set_entries(&mut self, entries: Vec<Entry>, state: &RenderState) {
        self.cards = (entries.into_iter())
            .map(|entry| Card {
                entry,
                version: 0,
                run: Run::Waiting,
            })
            .collect();
        self.running = 0;
        self.cells.reset(THUMBNAILS, state);
    }

    /// Takes `entry`, of a chart script whose text changed, whose card runs again when it is in
    /// view.
    pub fn update(&mut self, entry: Entry) {
        if let Some(card) = self
            .cards
            .iter_mut()
            .find(|card| card.entry.path == entry.path)
        {
            card.entry = entry;
            card.reset();
        }
    }

    /// Runs again the cards whose chart scripts `rerun` selects, when they are in view.
    pub fn rerun(&mut self, rerun: impl Fn(&Entry) -> bool) {
        for card in &mut self.cards {
            if rerun(&card.entry) {
                card.reset();
            }
        }
    }

    /// Keeps the result of the run of the card `index` at its `version`, unless the card changed
    /// since the run started.
    pub fn ran(&mut self, index: usize, version: usize, result: Result<QueryResult, String>) {
        self.running = self.running.saturating_sub(1);
        let Some(card) = self
            .cards
            .get_mut(index)
            .filter(|card| card.version == version)
        else {
            return;
        };
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
        card.run = Run::Ran(ran);
    }

    /// Keeps the scene of a thumbnail that a background evaluation finished.
    pub fn receive(&mut self, scene: CellScene) {
        self.cells.receive(scene);
    }

    /// Whether a run or an evaluation of a card in view runs, or waits to start.
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
                .hint_text("Search the charts")
                .desired_width(220.0),
        );
    }

    /// Shows the cards that match the search, by section. `ready` is whether the project's tables
    /// are loaded, so that the charts can run.
    pub fn show(&mut self, ui: &mut egui::Ui, env: &Env<'_>, ready: bool) -> GalleryOutput {
        let mut output = GalleryOutput::default();
        self.waiting = false;
        if self.cards.is_empty() {
            ui.weak(
                "The charts of the project show here. Add a .sql file under charts/ to draw one.",
            );
            return output;
        }
        let search = self.search.trim().to_lowercase();
        let matching: Vec<usize> = (self.cards.iter().enumerate())
            .filter(|(_, card)| search.is_empty() || card.entry.matches(&search))
            .map(|(index, _)| index)
            .collect();
        if matching.is_empty() {
            ui.weak(format!("No chart matches “{}”.", self.search.trim()));
            return output;
        }
        egui::ScrollArea::vertical()
            .id_salt("gallery")
            .auto_shrink(false)
            .show(ui, |ui| {
                let mut rest = matching.as_slice();
                while let Some(&first) = rest.first() {
                    let section = self.cards[first].entry.section.clone();
                    let count = (rest.iter())
                        .take_while(|&&index| self.cards[index].entry.section == section)
                        .count();
                    let (cards, after) = rest.split_at(count);
                    rest = after;
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.strong(&section);
                        ui.weak(count.to_string());
                    });
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
                        for &index in cards {
                            if self.card(ui, index, env, ready, &mut output) {
                                output.open = Some(self.cards[index].entry.path.clone());
                            }
                        }
                    });
                }
                ui.add_space(GAP);
            });
        output
    }

    /// Shows the card `index`, and starts its run when it comes into view. Returns whether the
    /// user clicked it.
    fn card(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        env: &Env<'_>,
        ready: bool,
        output: &mut GalleryOutput,
    ) -> bool {
        let size = THUMBNAIL + egui::vec2(0.0, TITLE_HEIGHT);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        if !ui.is_rect_visible(rect) {
            return false;
        }
        let card = &mut self.cards[index];
        if matches!(card.run, Run::Waiting) && ready {
            match self.running < QUERIES {
                true => {
                    self.running += 1;
                    card.run = Run::Running;
                    output.start.push(Start {
                        index,
                        version: card.version,
                        path: card.entry.path.clone(),
                        text: card.entry.text.clone(),
                    });
                }
                false => self.waiting = true,
            }
        }
        let card = &self.cards[index];
        let thumbnail = egui::Rect::from_min_size(rect.min, THUMBNAIL);
        let visuals = ui.visuals().clone();
        // The panel's color, which the charts' backgrounds take, so that a thumbnail and its card
        // are one surface.
        ui.painter().rect_filled(rect, 6.0, visuals.panel_fill);
        match &card.run {
            Run::Waiting | Run::Running => {
                egui::Spinner::new().paint_at(
                    ui,
                    egui::Rect::from_center_size(thumbnail.center(), egui::vec2(16.0, 16.0)),
                );
            }
            Run::Ran(Err(error)) => message(ui, thumbnail, error, visuals.error_fg_color),
            Run::Ran(Ok(ran)) => match ran.first {
                Some(key) => {
                    let chart = ran.result.chart(key.0, key.1).expect("the first chart");
                    let inner = thumbnail.shrink(1.0);
                    let key = (index, card.version);
                    self.cells.show(ui, inner, key, chart, LAYOUT, env);
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
            egui::Label::new(&card.entry.title)
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
            ui.strong(&card.entry.title);
            if !card.entry.description.is_empty() {
                ui.label(&card.entry.description);
            }
            ui.weak(&card.entry.path);
        });
        response.clicked()
    }
}

/// A short message in a thumbnail, such as the error of its chart.
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
