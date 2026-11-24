//! The query of a chart above its result, and its stages: the query's text, read-only and
//! highlighted, in a panel under the window's bar, where a click on a top-level `|>` opens the
//! table of the rows that enter its operator. In pipe syntax every prefix of a query is a query, so
//! a stage is the rows of the query before its operator, which its host runs.
//!
//! An open stage splits its line after the text before its `|>`: the `|>` starts the stage's row,
//! with the table to its right, and the operator moves below the table, so the rows read down into
//! it. The `|>` is the stage's disclosure arrow: it turns 90° clockwise as its stage opens, and
//! back as it closes. The panel opens and closes with the `|> Query` button of the bar, or ⌘E
//! (Ctrl+E off macOS), whose `|>` turns as the panel opens.
//!
//! The host gives the panel its [`Code`], runs the stages that it reports opening
//! ([`StageEvent`]), and gives back their rows or their errors. The open stages keep their
//! ordinals when the code changes, so a host runs them again, and an ordinal past the query's
//! operators closes.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use avenger_sql_syntax::{highlight, parse, Highlight, NameRole};
use eframe::egui::{self, epaint, Align2, Color32, FontId, Sense, TextFormat};

use crate::exec::Exec;
use crate::paged::{PagedTable, RowSource};
use crate::table::TEXT_ROW_HEIGHT;

/// The time of the panel's and the stages' animations, in seconds.
const ANIMATION: f32 = 0.15;

/// The rows that a stage's table shows before it scrolls.
const STAGE_ROWS: f32 = 10.0;

/// The size of the code's text.
const CODE_SIZE: f32 = 13.0;

/// The shortcut that opens and closes the panel: ⌘E, or Ctrl+E off macOS.
pub const QUERY_SHORTCUT: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::E);

/// A top-level operator of a query: its ordinal among the query's operators, from 1, the offset of
/// its `|>` in the code's text, and its name, as `|> WHERE`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeOperator {
    pub ordinal: usize,
    pub pipe: usize,
    pub name: String,
}

/// The query that the panel shows: its text, as its file holds it, its top-level operators in
/// order, and the ordinal of the operator that holds the place of an error of the query's run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Code {
    pub text: String,
    pub operators: Vec<CodeOperator>,
    pub error: Option<usize>,
}

/// What a reader did to a stage, for its host: opened it, so the host runs it, or closed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageEvent {
    Open(usize),
    Close(usize),
}

/// What an open stage shows: its run in progress, its rows, or its error.
enum Stage {
    Running,
    Rows(Box<PagedTable>),
    Failed(String),
}

/// The panel of a query and its stages.
#[derive(Default)]
pub struct QueryPanel {
    open: bool,
    code: Option<Code>,
    spans: Vec<(Range<usize>, Highlight)>,
    /// The stages that are open, by ordinal, and what each shows, which a stage keeps while it
    /// closes.
    opened: BTreeSet<usize>,
    stages: BTreeMap<usize, Stage>,
    /// The stage that a debugger stopped at, which the panel marks.
    current: Option<usize>,
    events: Vec<StageEvent>,
    /// The tables of the stages, numbered apart so a stage's table reads afresh when it runs again.
    tables: u64,
}

impl QueryPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the panel is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    /// The code that the panel shows.
    pub fn code(&self) -> Option<&Code> {
        self.code.as_ref()
    }

    /// The ordinals of the open stages.
    pub fn open_stages(&self) -> Vec<usize> {
        self.opened.iter().copied().collect()
    }

    /// Whether the query fails: its run has an error at an operator, or an open stage failed.
    pub fn fails(&self) -> bool {
        self.code.as_ref().is_some_and(|code| code.error.is_some())
            || (self.stages.values()).any(|stage| matches!(stage, Stage::Failed(_)))
    }

    /// Shows `code`, or nothing. A new text keeps the open stages by ordinal and asks the host to
    /// run them again, and closes those past its operators; the same text with another error
    /// changes only the operator that the error marks.
    pub fn set_code(&mut self, code: Option<Code>) {
        let same = match (&self.code, &code) {
            (Some(old), Some(new)) => old.text == new.text && old.operators == new.operators,
            (None, None) => true,
            _ => false,
        };
        if same {
            self.code = code;
            return;
        }
        self.spans = match &code {
            Some(code) => spans(&code.text),
            None => Vec::new(),
        };
        let count = code.as_ref().map_or(0, |code| code.operators.len());
        self.opened.retain(|ordinal| *ordinal <= count);
        self.stages.clear();
        for ordinal in &self.opened {
            self.stages.insert(*ordinal, Stage::Running);
            self.events.push(StageEvent::Open(*ordinal));
        }
        self.current = self.current.filter(|ordinal| *ordinal <= count);
        self.code = code;
    }

    /// Shows the rows of the stage `ordinal`, or its error, when it is open.
    pub fn set_stage(&mut self, ordinal: usize, rows: Result<Arc<dyn RowSource>, String>) {
        if !self.opened.contains(&ordinal) {
            return;
        }
        let stage = match rows {
            Ok(source) => Stage::Rows(Box::new(PagedTable::new(source))),
            Err(error) => Stage::Failed(error),
        };
        self.tables += 1;
        self.stages.insert(ordinal, stage);
    }

    /// Marks the stage `ordinal` as where a debugger stopped, open and with every other stage
    /// closed, or with none, every stage closed, as at a statement's result. The panel opens. The
    /// host gives the stage's rows with [`QueryPanel::set_stage`].
    pub fn set_current(&mut self, ordinal: Option<usize>) {
        self.current = ordinal;
        self.opened.clear();
        if let Some(ordinal) = ordinal {
            self.opened.insert(ordinal);
            self.stages.insert(ordinal, Stage::Running);
        }
        self.open = true;
    }

    /// The events of the stages since the last call, for the host.
    pub fn take_events(&mut self) -> Vec<StageEvent> {
        std::mem::take(&mut self.events)
    }

    /// Opens or closes the stage `ordinal`, as a click on its `|>` does.
    pub fn toggle_stage(&mut self, ordinal: usize) {
        match self.opened.remove(&ordinal) {
            true => self.events.push(StageEvent::Close(ordinal)),
            false => {
                self.opened.insert(ordinal);
                self.stages.insert(ordinal, Stage::Running);
                self.events.push(StageEvent::Open(ordinal));
            }
        }
    }

    /// The `|> Query` button, for the window's bar: it opens and closes the panel, its `|>`
    /// turns as the panel opens, and it takes the error color while the panel is closed and the
    /// query fails.
    pub fn button(&mut self, ui: &mut egui::Ui) {
        let color = match !self.open && self.fails() {
            true => ui.visuals().error_fg_color,
            false => ui.visuals().text_color(),
        };
        let openness = (ui.ctx()).animate_bool_with_time(
            egui::Id::new("query panel button"),
            self.open,
            ANIMATION,
        );
        let hover = format!(
            "The query and the rows that enter each of its operators ({})",
            ui.ctx().format_shortcut(&QUERY_SHORTCUT)
        );
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let font = FontId::monospace(ui.style().text_styles[&egui::TextStyle::Button].size);
            let pipe = turned_pipe(ui, openness, &font, color, &hover);
            let label = ui
                .selectable_label(self.open, egui::RichText::new("Query").color(color))
                .on_hover_text(&hover);
            if pipe.clicked() || label.clicked() {
                self.open = !self.open;
            }
        });
    }

    /// Shows the panel under the window's bar, sliding open and shut, with the stages' tables
    /// reading on `exec`. ⌘E or Ctrl+E opens and closes it.
    pub fn show(&mut self, ctx: &egui::Context, exec: &Exec) {
        if ctx.input_mut(|input| input.consume_shortcut(&QUERY_SHORTCUT)) {
            self.open = !self.open;
        }
        let height = ctx.content_rect().height();
        egui::TopBottomPanel::top("query panel")
            .resizable(true)
            .default_height((height * 0.4).min(360.0))
            .max_height(height * 0.7)
            .show_animated(ctx, self.open, |ui| {
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.show_code(ui, exec));
            });
    }

    /// The code, line by line, with each open stage on a row of its own.
    fn show_code(&mut self, ui: &mut egui::Ui, exec: &Exec) {
        let Some(code) = self.code.clone() else {
            ui.weak("No query");
            return;
        };
        let ctx = ui.ctx().clone();
        // Each stage's openness, from 0 to 1, which turns its `|>` and unfolds its table.
        let openness: BTreeMap<usize, f32> = (code.operators.iter())
            .map(|operator| {
                let id = egui::Id::new(("stage", operator.ordinal));
                let open = self.opened.contains(&operator.ordinal);
                (
                    operator.ordinal,
                    ctx.animate_bool_with_time(id, open, ANIMATION),
                )
            })
            .collect();
        // A stage that finished closing lets its table go, which stops its reading.
        let closed: Vec<usize> = (self.stages.keys())
            .filter(|ordinal| !self.opened.contains(ordinal) && openness[ordinal] == 0.0)
            .copied()
            .collect();
        for ordinal in closed {
            self.stages.remove(&ordinal);
        }
        let font = FontId::monospace(CODE_SIZE);
        let char_width = ui.fonts_mut(|fonts| fonts.glyph_width(&font, ' '));
        let rows = layout(&code, |ordinal| openness[&ordinal] > 0.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        for row in rows {
            match row {
                Row::Line(items) => {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        if items.is_empty() {
                            ui.add_space(0.0);
                            ui.label(egui::RichText::new(" ").font(font.clone()));
                        }
                        for item in items {
                            self.item(ui, &code, item, &font, char_width, &openness);
                        }
                    });
                }
                Row::Stage { indent, ordinal } => {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(indent as f32 * char_width);
                        self.pipe(ui, ordinal, &font, openness[&ordinal]);
                        ui.add_space(char_width);
                        ui.vertical(|ui| self.stage(ui, ordinal, openness[&ordinal], exec));
                    });
                }
            }
        }
    }

    /// An item of a line.
    fn item(
        &mut self,
        ui: &mut egui::Ui,
        code: &Code,
        item: Item,
        font: &FontId,
        char_width: f32,
        openness: &BTreeMap<usize, f32>,
    ) {
        match item {
            Item::Text(range) => {
                let job = self.job(ui, code, range, font);
                ui.add(egui::Label::new(job).selectable(true));
            }
            Item::Space(columns) => ui.add_space(columns as f32 * char_width),
            Item::Pipe(ordinal) => self.pipe(ui, ordinal, font, openness[&ordinal]),
        }
    }

    /// The `|>` of the stage `ordinal`, turned by its openness, which opens and closes it.
    fn pipe(&mut self, ui: &mut egui::Ui, ordinal: usize, font: &FontId, openness: f32) {
        let code = self.code.as_ref().expect("the code of a stage");
        let operator = &code.operators[ordinal - 1];
        let color = match (self.current == Some(ordinal), code.error == Some(ordinal)) {
            (true, _) => ui.visuals().selection.stroke.color,
            (false, true) => ui.visuals().error_fg_color,
            (false, false) => highlight_color(ui.visuals().dark_mode, Highlight::Operator)
                .unwrap_or(ui.visuals().text_color()),
        };
        let keyword = operator.name.trim_start_matches("|> ");
        let hover = format!("Rows entering {keyword}");
        if turned_pipe(ui, openness, font, color, &hover).clicked() {
            self.toggle_stage(ordinal);
        }
    }

    /// The table of the stage `ordinal`, its error, or its progress, unfolded by its openness.
    fn stage(&mut self, ui: &mut egui::Ui, ordinal: usize, openness: f32, exec: &Exec) {
        let full = 52.0 + STAGE_ROWS * TEXT_ROW_HEIGHT;
        let width = ui.available_width().clamp(240.0, 960.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, full * openness), Sense::hover());
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_size(rect.min, egui::vec2(width, full)))
                .id_salt(("stage body", ordinal)),
        );
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        let tables = self.tables;
        match self.stages.get_mut(&ordinal) {
            None | Some(Stage::Running) => {
                child.horizontal(|ui| {
                    ui.spinner();
                    ui.weak("Running the query before this operator…");
                });
            }
            Some(Stage::Failed(error)) => {
                let color = child.visuals().error_fg_color;
                child.label(egui::RichText::new(error.as_str()).monospace().color(color));
            }
            Some(Stage::Rows(table)) => {
                if let Some(error) = table.show(&mut child, exec, tables * 1000 + ordinal as u64) {
                    self.stages.insert(ordinal, Stage::Failed(error));
                }
            }
        }
    }

    /// The text of `range` of the code, highlighted, with the operator of the query's error
    /// underlined.
    fn job(
        &self,
        ui: &egui::Ui,
        code: &Code,
        range: Range<usize>,
        font: &FontId,
    ) -> egui::text::LayoutJob {
        let dark = ui.visuals().dark_mode;
        let text_color = ui.visuals().text_color();
        let error = code.error.and_then(|ordinal| {
            let operator = code.operators.get(ordinal - 1)?;
            let end = (code.operators.get(ordinal)).map_or(code.text.len(), |next| next.pipe);
            Some((operator.pipe..end, ui.visuals().error_fg_color))
        });
        let mut job = egui::text::LayoutJob::default();
        let mut at = range.start;
        while at < range.end {
            let span = self
                .spans
                .iter()
                .find(|(span, _)| span.start <= at && at < span.end);
            let next_span = (self.spans.iter())
                .map(|(span, _)| span.start)
                .filter(|start| *start > at)
                .min()
                .unwrap_or(usize::MAX);
            let end = match span {
                Some((span, _)) => span.end.min(range.end),
                None => next_span.min(range.end),
            };
            let color =
                (span.and_then(|(_, class)| highlight_color(dark, *class))).unwrap_or(text_color);
            let mut format = TextFormat::simple(font.clone(), color);
            if let Some((error, error_color)) = &error {
                if error.start <= at && at < error.end {
                    format.underline = egui::Stroke::new(1.0, *error_color);
                }
            }
            job.append(&code.text[at..end], 0.0, format);
            at = end;
        }
        job
    }
}

/// An item of a line of the code: its text, the space of columns, or the `|>` of a stage.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Item {
    Text(Range<usize>),
    Space(usize),
    Pipe(usize),
}

/// A row of the code as the panel lays it out: a line of items, or an open stage's row, its `|>`
/// at a column, with its table to its right.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Row {
    Line(Vec<Item>),
    Stage { indent: usize, ordinal: usize },
}

/// The rows of `code`, with the stages that `open` says are open on rows of their own. A stage's
/// `|>` keeps its column when only spaces come before it on its line, and else starts its row, and
/// its operator goes on the row after the stage's, at the column after its `|>`.
fn layout(code: &Code, open: impl Fn(usize) -> bool) -> Vec<Row> {
    let text = &code.text;
    let mut rows = Vec::new();
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let line_end = line_start + line.trim_end_matches(['\n', '\r']).len();
        let mut items = Vec::new();
        let mut at = line_start;
        for operator in
            (code.operators.iter()).filter(|op| line_start <= op.pipe && op.pipe < line_end)
        {
            if at < operator.pipe {
                items.push(Item::Text(at..operator.pipe));
            }
            match open(operator.ordinal) {
                false => items.push(Item::Pipe(operator.ordinal)),
                true => {
                    let before = &text[line_start..operator.pipe];
                    let blank = before.trim().is_empty();
                    let column = before.chars().count();
                    if !blank {
                        rows.push(Row::Line(std::mem::take(&mut items)));
                    }
                    rows.push(Row::Stage {
                        indent: if blank { column } else { 0 },
                        ordinal: operator.ordinal,
                    });
                    let after = if blank { column } else { 0 };
                    items = vec![Item::Space(after + 2)];
                }
            }
            at = operator.pipe + 2;
        }
        if at < line_end {
            items.push(Item::Text(at..line_end));
        }
        rows.push(Row::Line(items));
        line_start += line.len();
    }
    rows
}

/// The highlight spans of `text`, by the syntax's classes.
fn spans(text: &str) -> Vec<(Range<usize>, Highlight)> {
    let root = parse(text).syntax();
    (highlight(&root).into_iter())
        .map(|range| {
            let span = usize::from(range.range.start())..usize::from(range.range.end());
            (span, range.highlight)
        })
        .collect()
}

/// The color of a class of highlight, in a dark or a light palette, or none for the text's own.
fn highlight_color(dark: bool, class: Highlight) -> Option<Color32> {
    let pick = |on_dark: (u8, u8, u8), on_light: (u8, u8, u8)| match dark {
        true => Color32::from_rgb(on_dark.0, on_dark.1, on_dark.2),
        false => Color32::from_rgb(on_light.0, on_light.1, on_light.2),
    };
    Some(match class {
        Highlight::Keyword => pick((86, 156, 214), (0, 0, 220)),
        Highlight::Comment => pick((106, 153, 85), (0, 128, 0)),
        Highlight::String | Highlight::EnumValue | Highlight::Format | Highlight::Uri => {
            pick((206, 145, 120), (163, 21, 21))
        }
        Highlight::Number => pick((181, 206, 168), (9, 134, 88)),
        Highlight::Operator => pick((180, 180, 180), (90, 90, 90)),
        Highlight::Name(NameRole::Function(_)) => pick((220, 220, 170), (121, 94, 38)),
        Highlight::Name(NameRole::Type) => pick((78, 201, 176), (38, 127, 153)),
        Highlight::Name(NameRole::Parameter) => pick((156, 220, 254), (0, 16, 128)),
        _ => return None,
    })
}

/// A `|>` that turns by `openness`, from 0 to 1, a quarter turn clockwise about its center, and
/// takes a click, with a pointing hand and `hover` on hover.
fn turned_pipe(
    ui: &mut egui::Ui,
    openness: f32,
    font: &FontId,
    color: Color32,
    hover: &str,
) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap("|>".into(), font.clone(), color);
    let size = galley.size();
    let response = ui.allocate_response(size, Sense::click());
    if response.hovered() {
        let fill = ui
            .style()
            .interact_selectable(&response, false)
            .weak_bg_fill;
        ui.painter()
            .rect_filled(response.rect.expand(1.0), 2.0, fill);
    }
    let angle = openness * std::f32::consts::FRAC_PI_2;
    let shape = epaint::TextShape::new(response.rect.min, galley, color)
        .with_angle_and_anchor(angle, Align2::CENTER_CENTER);
    ui.painter().add(shape);
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(hover)
}

#[cfg(test)]
mod tests;
