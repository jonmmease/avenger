//! The query editor: egui's `TextEdit` with a layouter that colors the tokens of the query, a
//! line-number gutter, a wavy underline under the range of the query's error, and ⌘/ (Ctrl+/ on
//! other platforms), which comments the lines of the selection out or back in.

use std::ops::Range;
use std::sync::Arc;

use eframe::egui::{
    self, text::CCursor, text::LayoutJob, text_selection::CCursorRange, Color32, FontId, Pos2,
    Stroke, TextFormat, Vec2,
};

use avenger_sql_language::HighlightRange;

use crate::{
    comment,
    highlight::{self, Class},
};

/// The width of the line-number gutter, in points.
const GUTTER: f32 = 40.0;

pub struct CodeEditor {
    id: egui::Id,
    /// The last layout, so that egui lays out only after an edit, a new analysis, or a new scale.
    cache: Option<Layout>,
}

/// A layout of the text and what it was laid out with: the highlights of the analysis, whether
/// the text was the analyzed text, so that it took those highlights and not those of its syntax
/// alone, and the pixels per point of the screen.
struct Layout {
    text: String,
    highlights: Vec<HighlightRange>,
    analyzed: bool,
    pixels_per_point: f32,
    galley: Arc<egui::Galley>,
}

impl CodeEditor {
    pub fn new(id: impl std::hash::Hash) -> Self {
        Self {
            id: egui::Id::new(id),
            cache: None,
        }
    }

    /// Inserts `insert` at the cursor, or in place of the selection, and puts the cursor after
    /// it. Without a cursor, it goes at the end of the text.
    pub fn insert(&mut self, ctx: &egui::Context, text: &mut String, insert: &str) {
        let range = egui::text_edit::TextEditState::load(ctx, self.id)
            .and_then(|state| state.cursor.char_range())
            .map(|range| {
                let [a, b] = [range.primary.index, range.secondary.index];
                a.min(b)..a.max(b)
            });
        let count = text.chars().count();
        let range = range.unwrap_or(count..count);
        let (start, end) = (byte_index(text, range.start), byte_index(text, range.end));
        text.replace_range(start..end, insert);
        set_cursor(ctx, self.id, range.start + insert.chars().count());
        ctx.memory_mut(|memory| memory.request_focus(self.id));
    }

    /// Replaces the whole text with `replacement` as one step of the editor's undo history, so
    /// that an undo brings back the text before it, and puts the cursor at the start.
    pub fn replace(&mut self, ctx: &egui::Context, text: &mut String, replacement: &str) {
        let mut state = egui::text_edit::TextEditState::load(ctx, self.id).unwrap_or_default();
        let count = text.chars().count();
        let before = (state.cursor.char_range()).unwrap_or(CCursorRange::one(CCursor::new(count)));
        let after = CCursorRange::one(CCursor::new(0));
        let mut undoer = state.undoer();
        undoer.add_undo(&(before, text.clone()));
        *text = replacement.to_string();
        undoer.add_undo(&(after, text.clone()));
        state.set_undoer(undoer);
        state.cursor.set_char_range(Some(after));
        state.store(ctx, self.id);
        ctx.memory_mut(|memory| memory.request_focus(self.id));
    }

    /// Shows the editor in the rest of `ui`, with its tokens colored by `highlights`, the
    /// highlights of the text `analyzed`, and a wavy underline under each range of characters of
    /// `errors`. Returns whether the text changed.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        text: &mut String,
        analyzed: &str,
        highlights: &[HighlightRange],
        errors: &[Range<usize>],
    ) -> bool {
        let focused = ui.memory(|memory| memory.has_focus(self.id));
        let toggled = focused
            && ui.input_mut(|input| input.consume_shortcut(&comment_shortcut()))
            && self.toggle_comments(ui.ctx(), text);
        let font = FontId::monospace(13.0);
        let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _wrap: f32| {
            let source = buffer.as_str();
            let is_analyzed = source == analyzed;
            let pixels_per_point = ui.pixels_per_point();
            if let Some(cache) = &self.cache {
                if cache.text == source
                    && cache.highlights == highlights
                    && cache.analyzed == is_analyzed
                    && cache.pixels_per_point == pixels_per_point
                {
                    return cache.galley.clone();
                }
            }
            // A text that differs from the analysis, as in the frame of an edit, lays out with
            // the highlights of its syntax.
            let own;
            let ranges = match is_analyzed {
                true => highlights,
                false => {
                    own = highlight::syntax(source);
                    &own
                }
            };
            let job = layout_job(source, ranges, font.clone(), &[], Color32::TRANSPARENT);
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
            self.cache = Some(Layout {
                text: source.to_string(),
                highlights: highlights.to_vec(),
                analyzed: is_analyzed,
                pixels_per_point,
                galley: galley.clone(),
            });
            galley
        };
        // Solid scroll bars: a floating bar widens when the pointer comes near, and next to the
        // divider under the editor, the bar and the divider take the pointer from each other on
        // alternate frames, so the bar flashes. A solid bar keeps its size.
        let output = ui
            .scope(|ui| {
                ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                egui::ScrollArea::both()
                    .id_salt(self.id.with("scroll"))
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        egui::TextEdit::multiline(text)
                            .id(self.id)
                            .code_editor()
                            .lock_focus(true)
                            .frame(false)
                            .margin(egui::Margin {
                                left: GUTTER as i8,
                                right: 8,
                                top: 6,
                                bottom: 6,
                            })
                            .desired_width(f32::INFINITY)
                            .desired_rows(8)
                            .layouter(&mut layouter)
                            .show(ui)
                    })
                    .inner
            })
            .inner;
        paint_gutter(ui, &output);
        for error in errors {
            paint_squiggle(ui, &output, error);
        }
        toggled || output.response.changed()
    }

    /// Comments the lines of the selection out or back in, and keeps the selection on its
    /// characters. Returns whether the text changed.
    fn toggle_comments(&mut self, ctx: &egui::Context, text: &mut String) -> bool {
        let mut state = egui::text_edit::TextEditState::load(ctx, self.id).unwrap_or_default();
        let count = text.chars().count();
        let range = state
            .cursor
            .char_range()
            .unwrap_or(CCursorRange::one(CCursor::new(count)));
        let [primary, secondary] = [range.primary.index, range.secondary.index];
        let (toggled, selection) =
            comment::toggle(text, primary.min(secondary)..primary.max(secondary));
        if toggled == *text {
            return false;
        }
        *text = toggled;
        let (primary, secondary) = match primary <= secondary {
            true => (selection.start, selection.end),
            false => (selection.end, selection.start),
        };
        state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(secondary),
            CCursor::new(primary),
        )));
        state.store(ctx, self.id);
        true
    }
}

/// The shortcut that comments the lines of the selection out or back in.
pub(crate) fn comment_shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Slash)
}

fn paint_gutter(ui: &egui::Ui, output: &egui::text_edit::TextEditOutput) {
    let painter = ui.painter_at(output.text_clip_rect.expand2(Vec2::new(GUTTER, 0.0)));
    let color = ui.visuals().weak_text_color();
    let font = FontId::monospace(11.0);
    let mut line = 1;
    let mut starts_line = true;
    for row in &output.galley.rows {
        if starts_line {
            let pos = output.galley_pos + row.pos.to_vec2();
            painter.text(
                Pos2::new(pos.x - 10.0, pos.y + row.rect().height() / 2.0),
                egui::Align2::RIGHT_CENTER,
                line.to_string(),
                font.clone(),
                color,
            );
            line += 1;
        }
        starts_line = row.row.ends_with_newline;
    }
}

/// A wavy underline under `range`, split at line ends. An empty range gets one character's
/// width.
fn paint_squiggle(ui: &egui::Ui, output: &egui::text_edit::TextEditOutput, range: &Range<usize>) {
    let painter = ui.painter_at(output.text_clip_rect);
    let chars: Vec<char> = output.galley.text().chars().collect();
    let color = Color32::from_rgb(220, 50, 47);
    let mut start = range.start.min(chars.len());
    let end = range.end.min(chars.len()).max(start);
    let end = if end == start { start + 1 } else { end };
    while start < end {
        let line_end = (start..end)
            .find(|&index| chars.get(index) == Some(&'\n'))
            .unwrap_or(end);
        let a = output.galley.pos_from_cursor(CCursor::new(start));
        let b = output.galley.pos_from_cursor(CCursor::new(line_end));
        let width = (b.min.x - a.min.x).max(6.0);
        let y = output.galley_pos.y + a.max.y - 1.0;
        let x = output.galley_pos.x + a.min.x;
        painter.add(wave(x, x + width, y, color));
        start = line_end + 1;
    }
}

fn wave(x0: f32, x1: f32, y: f32, color: Color32) -> egui::Shape {
    let mut points = Vec::new();
    let mut x = x0;
    let mut up = true;
    while x <= x1 {
        points.push(Pos2::new(x, if up { y - 1.5 } else { y + 1.0 }));
        x += 2.5;
        up = !up;
    }
    egui::Shape::line(points, Stroke::new(1.2, color))
}

fn set_cursor(ctx: &egui::Context, id: egui::Id, index: usize) {
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(index))));
    state.store(ctx, id);
}

fn byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(index, _)| index)
}

/// The color of each class, on egui's light theme.
fn class_color(class: Class) -> Color32 {
    let (r, g, b) = match class {
        Class::Plain => (40, 40, 40),
        Class::Keyword => (0, 92, 197),
        Class::Function => (121, 94, 38),
        Class::Chart => (150, 60, 0),
        Class::Argument => (0, 110, 160),
        Class::Column => (30, 70, 120),
        Class::Table => (90, 40, 140),
        Class::Type => (0, 120, 120),
        Class::Variable => (140, 80, 20),
        Class::Number => (9, 134, 88),
        Class::String => (163, 21, 21),
        Class::Comment => (110, 120, 110),
        Class::Operator => (170, 55, 140),
    };
    Color32::from_rgb(r, g, b)
}

/// The layout job of `text`: each character in the color of its class by `highlights`, and the
/// characters of the ranges `marked` on the background `background`.
pub(crate) fn layout_job(
    text: &str,
    highlights: &[HighlightRange],
    font: FontId,
    marked: &[Range<usize>],
    background: Color32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let chars: Vec<char> = text.chars().collect();
    let mut classes = vec![Class::Plain; chars.len()];
    for span in highlight::spans(text, highlights) {
        let end = span.range.end.min(chars.len());
        for class in &mut classes[span.range.start.min(end)..end] {
            *class = span.class;
        }
    }
    let mut on = vec![false; chars.len()];
    for range in marked {
        let end = range.end.min(chars.len());
        for on in &mut on[range.start.min(end)..end] {
            *on = true;
        }
    }
    let mut start = 0;
    while start < chars.len() {
        let mut end = start + 1;
        while end < chars.len() && classes[end] == classes[start] && on[end] == on[start] {
            end += 1;
        }
        let format = TextFormat {
            font_id: font.clone(),
            color: class_color(classes[start]),
            italics: classes[start] == Class::Comment,
            background: match on[start] {
                true => background,
                false => Color32::TRANSPARENT,
            },
            ..Default::default()
        };
        job.append(&chars[start..end].iter().collect::<String>(), 0.0, format);
        start = end;
    }
    job
}
