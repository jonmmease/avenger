//! The query editor: egui's `TextEdit` with a layouter that colors the tokens of the query, a
//! line-number gutter, a wavy underline under the range of the query's error, ⌘/ (Ctrl+/ on
//! other platforms), which comments the lines of the selection out or back in, and Enter and Tab
//! that indent by the style.

use std::ops::Range;
use std::sync::Arc;

use eframe::egui::{
    self, text::CCursor, text::LayoutJob, text_selection::CCursorRange, Color32, FontId, Key,
    Modifiers, Pos2, Stroke, TextFormat, Vec2,
};

use avenger_sql_language::{Document, Fix, HighlightRange};

use crate::{
    assist::{self, Block, Popup, Span},
    comment,
    highlight::{self, Chars, Class},
    indent,
};

/// The width of the line-number gutter, in points.
const GUTTER: f32 = 40.0;

pub struct CodeEditor {
    id: egui::Id,
    /// The completion popup, while it is open.
    popup: Option<Popup>,
    /// The menu of the fixes of the error at the cursor, while it is open.
    fixes: Option<Fixes>,
    /// The last layout, so that egui lays out only after an edit, a new analysis, a move of the
    /// cursor to another name, or a new scale.
    cache: Option<Layout>,
    /// The character index that the next frame scrolls into view, after a jump to a definition.
    reveal: Option<usize>,
    /// The screen position of the middle of the character after the cursor, in the last frame.
    caret: Option<Pos2>,
}

/// A layout of the editor's text and what it was laid out with: the highlights of the analysis,
/// whether the text was the analyzed text, so that it took those highlights and not those of its
/// syntax alone, the ranges on the faint background of references, and the pixels per point of
/// the screen.
struct Layout {
    text: String,
    highlights: Vec<HighlightRange>,
    analyzed: bool,
    marked: Vec<Range<usize>>,
    pixels_per_point: f32,
    galley: Arc<egui::Galley>,
}

/// The open menu of the fixes of an error: the fixes, the selected one, and the text and the
/// cursor where it opened. The edits of the fixes are byte ranges of that text, so the menu closes
/// when the text or the cursor changes.
struct Fixes {
    fixes: Vec<Fix>,
    selected: usize,
    text: String,
    cursor: usize,
}

impl CodeEditor {
    pub fn new(id: impl std::hash::Hash) -> Self {
        Self {
            id: egui::Id::new(id),
            popup: None,
            fixes: None,
            cache: None,
            reveal: None,
            caret: None,
        }
    }

    /// Inserts `insert` at the cursor, or in place of the selection, and puts the cursor after
    /// it. Without a cursor, it goes at the end of the text.
    /// Focuses the editor with the cursor before the character at `index`.
    pub fn place(&self, ctx: &egui::Context, index: usize) {
        set_cursor(ctx, self.id, index);
        ctx.memory_mut(|memory| memory.request_focus(self.id));
    }

    /// The screen position of the middle of the character after the cursor, in the last frame.
    pub fn caret(&self) -> Option<Pos2> {
        self.caret
    }

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
        self.replace_with_cursor(ctx, text, replacement, |_| 0);
    }

    /// Replaces the text with `formatted`, the text in the style, as one step of the undo
    /// history, with the cursor on the same token.
    pub fn format(&mut self, ctx: &egui::Context, text: &mut String, formatted: &str) {
        let old = text.clone();
        self.replace_with_cursor(ctx, text, formatted, |index| {
            crate::format::cursor(&old, formatted, index)
        });
    }

    /// Replaces the whole text with `replacement` as one step of the undo history, and moves the
    /// cursor to `cursor` of its index before the replacement.
    fn replace_with_cursor(
        &mut self,
        ctx: &egui::Context,
        text: &mut String,
        replacement: &str,
        cursor: impl Fn(usize) -> usize,
    ) {
        let state = egui::text_edit::TextEditState::load(ctx, self.id).unwrap_or_default();
        let count = text.chars().count();
        let before = (state.cursor.char_range()).unwrap_or(CCursorRange::one(CCursor::new(count)));
        let index = cursor(before.primary.index);
        self.set_text(ctx, text, replacement.to_string(), index..index);
    }

    /// Replaces the text with `new` as one step of the undo history, with the selection
    /// `selection`, whose end is the cursor.
    fn set_text(
        &mut self,
        ctx: &egui::Context,
        text: &mut String,
        new: String,
        selection: Range<usize>,
    ) {
        let mut state = egui::text_edit::TextEditState::load(ctx, self.id).unwrap_or_default();
        let count = text.chars().count();
        let before = (state.cursor.char_range()).unwrap_or(CCursorRange::one(CCursor::new(count)));
        let after = CCursorRange::two(CCursor::new(selection.start), CCursor::new(selection.end));
        let mut undoer = state.undoer();
        undoer.add_undo(&(before, text.clone()));
        *text = new;
        undoer.add_undo(&(after, text.clone()));
        state.set_undoer(undoer);
        state.cursor.set_char_range(Some(after));
        state.store(ctx, self.id);
        ctx.memory_mut(|memory| memory.request_focus(self.id));
    }

    /// Shows the editor in the rest of `ui`, with the tokens colored by the highlights of
    /// `document`, the analysis of the text, a wavy underline under each of its errors, and its
    /// completions, fixes, and hovers. Returns whether the text changed.
    pub fn show(&mut self, ui: &mut egui::Ui, text: &mut String, document: &Document) -> bool {
        let analyzed = document.text().to_string();
        let highlights = document.highlights();
        let chars = Chars::new(&analyzed);
        let errors: Vec<Range<usize>> = (document.diagnostics().iter())
            .map(|diagnostic| chars.range(diagnostic.range))
            .collect();
        let (analyzed, highlights) = (analyzed.as_str(), highlights.as_slice());
        let focused = ui.memory(|memory| memory.has_focus(self.id));
        let toggled = focused
            && ui.input_mut(|input| input.consume_shortcut(&comment_shortcut()))
            && self.toggle_comments(ui.ctx(), text);
        // The keys of the popup and the fixes, before the text edit takes them.
        let mut edited = focused && text.as_str() == analyzed && self.keys(ui, text, document);
        // Enter and Tab by the style, when no menu takes them.
        if focused && self.popup.is_none() && self.fixes.is_none() {
            edited |= self.indent_keys(ui, text, document);
        }
        // The text and the cursor before the text edit, to find a typed character.
        let typing = focused.then(|| (text.clone(), self.cursor(ui.ctx())));
        if focused && text.as_str() == analyzed {
            let jump = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::F12));
            if let Some(cursor) = self.cursor(ui.ctx()).filter(|_| jump) {
                self.jump(ui.ctx(), document, cursor);
            }
        }
        // The definition and the uses of the name at the cursor, on a faint background.
        let marked = match text.as_str() == analyzed {
            true => (self.cursor(ui.ctx()))
                .map(|cursor| assist::references(document, cursor))
                .unwrap_or_default(),
            false => Vec::new(),
        };
        let font = FontId::monospace(13.0);
        let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _wrap: f32| {
            let source = buffer.as_str();
            let is_analyzed = source == analyzed;
            let pixels_per_point = ui.pixels_per_point();
            if let Some(cache) = &self.cache {
                if cache.text == source
                    && cache.highlights == highlights
                    && cache.analyzed == is_analyzed
                    && cache.marked == marked
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
            let marks: &[Range<usize>] = match is_analyzed {
                true => &marked,
                false => &[],
            };
            let job = layout_job(source, ranges, font.clone(), marks, REFERENCE);
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
            self.cache = Some(Layout {
                text: source.to_string(),
                highlights: highlights.to_vec(),
                analyzed: is_analyzed,
                marked: marked.clone(),
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
                        let output = egui::TextEdit::multiline(text)
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
                            .show(ui);
                        if let Some(index) = self.reveal.take() {
                            let rect = output.galley.pos_from_cursor(CCursor::new(index));
                            let rect = rect.translate(output.galley_pos.to_vec2()).expand(24.0);
                            ui.scroll_to_rect(rect, None);
                        }
                        output
                    })
                    .inner
            })
            .inner;
        paint_gutter(ui, &output);
        for error in &errors {
            paint_squiggle(ui, &output, error);
        }
        let cursor = (output.cursor_range).map(|range| range.primary.index);
        self.caret = cursor.map(|cursor| {
            let rect = output.galley.pos_from_cursor(CCursor::new(cursor));
            output.galley_pos + rect.center().to_vec2() + Vec2::new(3.0, 0.0)
        });
        let current = text.as_str() == analyzed;
        // ⌘-click (Ctrl-click) goes to the definition of the name that the click put the cursor on.
        let command = ui.input(|input| input.modifiers.command);
        if current && command && output.response.clicked() {
            if let Some(cursor) = cursor {
                self.jump(ui.ctx(), document, cursor);
            }
        }
        if let (true, Some((old, Some(before))), Some(after)) =
            (output.response.changed(), &typing, cursor)
        {
            if let Some(typed) = typed_char(old, text, *before, after) {
                if let Some((new, cursor)) = indent::typed(text, after, typed) {
                    self.set_text(ui.ctx(), text, new, cursor..cursor);
                    ui.ctx().request_repaint();
                    edited = true;
                }
            }
        }
        edited |= self.menus(ui, &output, text, cursor);
        if current {
            hover(&output, document);
        }
        toggled || edited || output.response.changed()
    }

    /// Takes Enter, Tab, and Shift+Tab before the text edit does: Enter starts a line at its depth,
    /// Tab re-indents the line or the lines of the selection, moves a cursor in the indentation
    /// to the first word, or opens the completion popup, and Shift+Tab does nothing. Returns
    /// whether the text changed.
    fn indent_keys(&mut self, ui: &egui::Ui, text: &mut String, document: &Document) -> bool {
        let ctx = ui.ctx().clone();
        let (enter, tab) = ui.input_mut(|input| {
            let enter = input.consume_key(Modifiers::NONE, Key::Enter);
            // Shift+Tab first, since a match of Tab ignores Shift.
            input.consume_key(Modifiers::SHIFT, Key::Tab);
            (enter, input.consume_key(Modifiers::NONE, Key::Tab))
        });
        if !enter && !tab {
            return false;
        }
        let state = egui::text_edit::TextEditState::load(&ctx, self.id).unwrap_or_default();
        let count = text.chars().count();
        let range = (state.cursor.char_range()).unwrap_or(CCursorRange::one(CCursor::new(count)));
        let (secondary, primary) = (range.secondary.index, range.primary.index);
        if enter {
            let (new, cursor) = indent::enter(text, secondary.min(primary)..secondary.max(primary));
            self.set_text(&ctx, text, new, cursor..cursor);
            return true;
        }
        match indent::tab(text, secondary..primary) {
            indent::Tab::Edit(new, selection) => {
                self.set_text(&ctx, text, new, selection);
                return true;
            }
            indent::Tab::Move(index) => set_cursor(&ctx, self.id, index),
            indent::Tab::Complete if text.as_str() == document.text() => {
                self.popup = Popup::open(document, primary);
                self.fixes = None;
            }
            indent::Tab::Complete | indent::Tab::Stay => {}
        }
        false
    }

    /// Selects the definition of the name at the character `cursor`, and scrolls it into view in
    /// the next frame.
    fn jump(&mut self, ctx: &egui::Context, document: &Document, cursor: usize) {
        let Some(target) = assist::definition(document, cursor) else {
            return;
        };
        let mut state = egui::text_edit::TextEditState::load(ctx, self.id).unwrap_or_default();
        let (start, end) = (CCursor::new(target.start), CCursor::new(target.end));
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(start, end)));
        state.store(ctx, self.id);
        self.reveal = Some(target.start);
    }

    /// The character index of the cursor, when the editor has one.
    fn cursor(&self, ctx: &egui::Context) -> Option<usize> {
        let state = egui::text_edit::TextEditState::load(ctx, self.id)?;
        state.cursor.char_range().map(|range| range.primary.index)
    }

    /// Takes the keys of the completion popup and the fixes menu before the text edit does:
    /// Option+Space or Ctrl+Space opens the popup, ⌘. (Ctrl+. off macOS) the fixes of the error
    /// at the cursor, the arrows move the selection, Enter or Tab inserts it, and Escape closes
    /// the menu. Returns whether the text changed.
    fn keys(&mut self, ui: &egui::Ui, text: &mut String, document: &Document) -> bool {
        let ctx = ui.ctx().clone();
        let Some(cursor) = self.cursor(&ctx) else {
            return false;
        };
        let complete = ui.input_mut(|input| {
            let pressed = input.consume_key(Modifiers::ALT, Key::Space)
                || input.consume_key(Modifiers::CTRL, Key::Space);
            // Option+Space types a no-break space on macOS, which the popup takes too.
            if pressed {
                input.events.retain(
                    |event| !matches!(event, egui::Event::Text(typed) if typed.trim().is_empty()),
                );
            }
            pressed
        });
        if complete {
            self.popup = Popup::open(document, cursor);
            self.fixes = None;
        }
        if ui.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::Period)) {
            let found = assist::fixes(document, cursor);
            self.fixes = (!found.is_empty()).then(|| Fixes {
                fixes: found,
                selected: 0,
                text: text.clone(),
                cursor,
            });
            self.popup = None;
        }
        let keys = |input: &mut egui::InputState| {
            let down = input.consume_key(Modifiers::NONE, Key::ArrowDown);
            let up = input.consume_key(Modifiers::NONE, Key::ArrowUp);
            let accept = input.consume_key(Modifiers::NONE, Key::Enter)
                || input.consume_key(Modifiers::NONE, Key::Tab);
            let escape = input.consume_key(Modifiers::NONE, Key::Escape);
            (down, up, accept, escape)
        };
        if let Some(mut popup) = self.popup.take() {
            let (down, up, accept, escape) = ui.input_mut(keys);
            popup.select(text, cursor, isize::from(down) - isize::from(up));
            if accept {
                if let Some((accepted, after)) = popup.accept(text, cursor) {
                    *text = accepted;
                    set_cursor(&ctx, self.id, after);
                    return true;
                }
            }
            if !escape {
                self.popup = Some(popup);
            }
        }
        let current = |menu: &Fixes| menu.text == *text && menu.cursor == cursor;
        if let Some(mut menu) = self.fixes.take().filter(current) {
            let (down, up, accept, escape) = ui.input_mut(keys);
            let step = isize::from(down) - isize::from(up);
            menu.selected =
                (menu.selected as isize + step).clamp(0, menu.fixes.len() as isize - 1) as usize;
            if accept {
                *text = assist::apply(text, &menu.fixes[menu.selected]);
                set_cursor(&ctx, self.id, cursor.min(text.chars().count()));
                return true;
            }
            if !escape {
                self.fixes = Some(menu);
            }
        }
        false
    }

    /// Draws the open completion popup or fixes menu under the cursor, and takes a click on one
    /// of its items. Both close when the editor loses the focus, the popup when the cursor leaves
    /// its name, and the fixes when the text or the cursor changes. Returns whether the text
    /// changed.
    fn menus(
        &mut self,
        ui: &egui::Ui,
        output: &egui::text_edit::TextEditOutput,
        text: &mut String,
        cursor: Option<usize>,
    ) -> bool {
        // A press on a menu takes the focus from the text edit before its click, so the menus
        // stay while the pointer is over them, at the cursor of the text edit's state.
        let ctx = ui.ctx().clone();
        let over = (ctx.memory(|memory| memory.area_rect(self.id.with("menu"))))
            .zip(ctx.pointer_hover_pos())
            .is_some_and(|(rect, pointer)| rect.contains(pointer));
        let cursor = match over {
            true => cursor.or_else(|| self.cursor(&ctx)),
            false => cursor,
        };
        let Some(cursor) = cursor else {
            self.popup = None;
            self.fixes = None;
            return false;
        };
        if (self.fixes.as_ref()).is_some_and(|menu| menu.text != *text || menu.cursor != cursor) {
            self.fixes = None;
        }
        if (self.popup.as_ref()).is_some_and(|popup| popup.closes(text, cursor)) {
            self.popup = None;
        }
        let at = output.galley_pos
            + output
                .galley
                .pos_from_cursor(CCursor::new(cursor))
                .left_bottom()
                .to_vec2();
        let menu = |labels: Vec<(String, Option<String>)>, selected: usize| -> Option<usize> {
            let mut clicked = None;
            egui::Area::new(self.id.with("menu"))
                .order(egui::Order::Foreground)
                .fixed_pos(at + Vec2::new(0.0, 2.0))
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        // A fixed width, which the window moves left near its edge, and one line
                        // for each item, whose detail ends in an ellipsis.
                        ui.set_width(MENU_WIDTH);
                        for (index, (label, detail)) in labels.into_iter().enumerate() {
                            let mut job = LayoutJob {
                                wrap: egui::text::TextWrapping::truncate_at_width(MENU_WIDTH),
                                ..Default::default()
                            };
                            job.append(
                                &label,
                                0.0,
                                TextFormat::simple(
                                    FontId::monospace(12.0),
                                    ui.visuals().strong_text_color(),
                                ),
                            );
                            if let Some(detail) = detail {
                                job.append(
                                    &format!("  {detail}"),
                                    0.0,
                                    TextFormat::simple(
                                        FontId::proportional(11.0),
                                        ui.visuals().weak_text_color(),
                                    ),
                                );
                            }
                            if ui.selectable_label(index == selected, job).clicked() {
                                clicked = Some(index);
                            }
                        }
                    });
                });
            clicked
        };
        if let Some(popup) = &mut self.popup {
            let items = popup.items(text, cursor);
            let selected = popup.selected(text, cursor);
            // The window of items that holds the selection.
            let first = selected.saturating_sub(assist::SHOWN - 1);
            let labels = (items.iter().skip(first).take(assist::SHOWN))
                .map(|item| (item.label.clone(), item.detail.clone()))
                .collect();
            if let Some(index) = menu(labels, selected - first) {
                popup.select_at(first + index);
                if let Some((accepted, after)) = popup.accept(text, cursor) {
                    *text = accepted;
                    set_cursor(&ctx, self.id, after);
                    ctx.memory_mut(|memory| memory.request_focus(self.id));
                    self.popup = None;
                    return true;
                }
            }
        }
        if let Some(fixes) = &self.fixes {
            let labels = (fixes.fixes.iter())
                .map(|fix| (fix.label.clone(), None))
                .collect();
            if let Some(index) = menu(labels, fixes.selected) {
                *text = assist::apply(text, &fixes.fixes[index]);
                set_cursor(&ctx, self.id, cursor.min(text.chars().count()));
                ctx.memory_mut(|memory| memory.request_focus(self.id));
                self.fixes = None;
                return true;
            }
        }
        false
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

/// The hover of the character under the pointer: the message and the help of an error there, or
/// else the document's hover of the name there, as a tooltip at the pointer. The pointer reads
/// only the character whose box holds it, so the space past the end of a line has none.
fn hover(output: &egui::text_edit::TextEditOutput, document: &Document) {
    let Some(pointer) = output.response.hover_pos() else {
        return;
    };
    let Some(index) = char_under(&output.galley, pointer - output.galley_pos) else {
        return;
    };
    let chars = Chars::new(document.text());
    let offset = chars.offset(index);
    let error = (document.diagnostics().into_iter()).find(|diagnostic| {
        let range = usize::from(diagnostic.range.start())..usize::from(diagnostic.range.end());
        range.contains(&offset) || (range.is_empty() && range.start == offset)
    });
    let blocks: Vec<Block> = match error {
        Some(error) => {
            let mut blocks = vec![Block::Paragraph(vec![Span::Plain(error.message)])];
            blocks.extend(
                error
                    .help
                    .as_deref()
                    .map(assist::blocks)
                    .unwrap_or_default(),
            );
            blocks
        }
        None => match document.hover((offset as u32).into()) {
            // The range of a hover holds the character, not only the position at its end.
            Some(hover) if usize::from(hover.range.end()) > offset => {
                assist::blocks(&hover.markdown)
            }
            _ => return,
        },
    };
    output.response.clone().on_hover_ui_at_pointer(|ui| {
        ui.set_max_width(TOOLTIP_WIDTH);
        show_blocks(ui, blocks);
    });
}

/// The index of the character of `galley` whose box holds `local`, a position relative to the
/// galley: the character before or after the cursor nearest `local`.
fn char_under(galley: &egui::Galley, local: Vec2) -> Option<usize> {
    let nearest = galley.cursor_from_pos(local).index;
    [nearest.checked_sub(1), Some(nearest)]
        .into_iter()
        .flatten()
        .find(|&index| {
            let left = galley.pos_from_cursor(CCursor::new(index));
            let right = galley.pos_from_cursor(CCursor::new(index + 1));
            // A character at the end of a row, such as a newline, ends on the next row.
            left.min.y == right.min.y
                && (left.min.x..right.min.x).contains(&local.x)
                && (left.min.y..left.max.y).contains(&local.y)
        })
}

/// The width of the completion popup and the fixes menu.
const MENU_WIDTH: f32 = 420.0;

/// The widest that a tooltip draws.
const TOOLTIP_WIDTH: f32 = 560.0;

/// Draws the blocks of a hover's Markdown: headings and strong spans in the strong color, code in
/// the monospace font, items of lists after a bullet, and blocks of code on a faint background.
fn show_blocks(ui: &mut egui::Ui, blocks: Vec<Block>) {
    let (text, strong) = (ui.visuals().text_color(), ui.visuals().strong_text_color());
    let job = |spans: Vec<Span>, size: f32, bullet: bool| {
        let mut job = LayoutJob::default();
        if bullet {
            job.append(
                "•  ",
                0.0,
                TextFormat::simple(FontId::proportional(size), text),
            );
        }
        for span in spans {
            let (part, format) = match span {
                Span::Plain(part) => (part, TextFormat::simple(FontId::proportional(size), text)),
                Span::Strong(part) => {
                    (part, TextFormat::simple(FontId::proportional(size), strong))
                }
                Span::Code(part) => (
                    part,
                    TextFormat::simple(FontId::monospace(size - 1.0), text),
                ),
            };
            job.append(&part, 0.0, format);
        }
        job.wrap.max_width = TOOLTIP_WIDTH;
        job
    };
    ui.spacing_mut().item_spacing.y = 6.0;
    for block in blocks {
        match block {
            Block::Heading(spans) => {
                let spans = (spans.into_iter())
                    .map(|span| match span {
                        Span::Plain(part) => Span::Strong(part),
                        span => span,
                    })
                    .collect();
                ui.label(job(spans, 15.0, false));
            }
            Block::Paragraph(spans) => {
                ui.label(job(spans, 13.0, false));
            }
            Block::Item(spans) => {
                ui.label(job(spans, 13.0, true));
            }
            Block::Code(code) => {
                egui::Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .inner_margin(4.0)
                    .corner_radius(3.0)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(code).monospace().size(12.0));
                    });
            }
        }
    }
}

/// The character that a keystroke typed, when `new` is `old` with one character inserted at the
/// cursor `before`, and the cursor `after` follows it.
fn typed_char(old: &str, new: &str, before: usize, after: usize) -> Option<char> {
    if after != before + 1 || new.chars().count() != old.chars().count() + 1 {
        return None;
    }
    let typed = new.chars().nth(before)?;
    let (at, end) = (byte_index(old, before), byte_index(new, after));
    (old[..at] == new[..at] && old[at..] == new[end..]).then_some(typed)
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

/// The background of the definition and the uses of the name at the cursor.
const REFERENCE: Color32 = Color32::from_rgba_premultiplied(0, 10, 22, 28);

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
        Class::Constant => (0, 112, 193),
        Class::Path => (129, 31, 63),
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
