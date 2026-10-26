//! A text input that edits one line of plain text.

mod edit;

use std::ops::Range;
use std::sync::Arc;

use avenger_common::cursor::CursorIcon;
use avenger_common::types::{FontStyle, FontWeight, TextBaseline};
use avenger_eventstream::scene::{SceneComposingEvent, SceneGraphEvent, SceneKeyPressEvent};
use avenger_eventstream::stream::{Disposition, Rebuild};
use avenger_eventstream::window::{Key, MouseButton, NamedKey};
use avenger_layout::{Direction, Measurement, Rect};
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::text::{text_style, SceneTextMark};
use avenger_typst_label::{Affinity, LabelError, TextLayout, TextPosition};

use crate::binding::{self, Get, Set};
use crate::draw::{cover, focus_ring, label_color, rect, Bounds, Text};
use crate::style::WidgetStyle;
use crate::widget::{Look, Placed, Widget, WidgetContext, WidgetError, WidgetEvent};
use edit::{boundary, Edit, Reach};

/// A text input with an optional label above its box. It edits one line of plain text.
///
/// Typed text replaces the selection. Left and Right move the caret, by word with Alt or Ctrl
/// and to the text's ends with Cmd, as Home and End do. Shift extends the selection. Backspace
/// and Delete delete the selection, or reach as far as the same modifiers move. Cmd or Ctrl
/// with A selects all, with C copies the selection and with X cuts it. A paste inserts its text
/// with each line break or tab as a space. A press places the caret, a drag or a press with
/// Shift selects, a double press selects a word and a triple press selects all. A drag after a
/// double press selects whole words. Keyboard focus selects all.
pub struct TextInput<State> {
    id: String,
    label: Option<Text>,
    placeholder: Option<Text>,
    value: Option<Get<State, String>>,
    on_change: Option<Set<State, String>>,
    on_submit: Option<Set<State, String>>,
    enabled: Option<Get<State, bool>>,
}

impl<State> TextInput<State> {
    /// Create a text input named `id`, without a label.
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            label: None,
            placeholder: None,
            value: None,
            on_change: None,
            on_submit: None,
            enabled: None,
        }
    }

    /// Show the plain-text `label` above the box.
    pub fn label(mut self, label: &str) -> Self {
        self.label = Some(Text::plain(label));
        self
    }

    /// Show the plain-text `placeholder` in the box while the value is empty.
    pub fn placeholder(mut self, placeholder: &str) -> Self {
        self.placeholder = Some(Text::plain(placeholder));
        self
    }

    /// Read the value from app state. Defaults to empty.
    pub fn value(mut self, value: impl Fn(&State) -> String + Send + Sync + 'static) -> Self {
        self.value = Some(Arc::new(value));
        self
    }

    /// Update app state for each edit and return the scenes to rebuild.
    pub fn on_change(
        mut self,
        on_change: impl Fn(&mut State, String) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.on_change = Some(Arc::new(on_change));
        self
    }

    /// Run a callback with the value on Enter and return the scenes to rebuild. Without one,
    /// Enter reaches the app.
    pub fn on_submit(
        mut self,
        on_submit: impl Fn(&mut State, String) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.on_submit = Some(Arc::new(on_submit));
        self
    }

    /// Read the input's enabled state from app state. Defaults to enabled.
    pub fn enabled(mut self, enabled: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        self.enabled = Some(Arc::new(enabled));
        self
    }

    fn current(&self, app: &State) -> String {
        self.value
            .as_ref()
            .map_or(String::new(), |value| value(app))
    }

    /// The label's width, and its height with the gap below it, or zero without a label.
    fn label_size(&self, cx: &WidgetContext<'_>) -> [f32; 2] {
        match &self.label {
            Some(label) => {
                let [width, height] = label.size(cx);
                [width, height + cx.style.label_gap]
            }
            None => [0.0; 2],
        }
    }

    /// The input's parts in `rect`.
    fn parts(&self, rect: Rect, cx: &WidgetContext<'_>) -> Parts {
        let [_, label_height] = self.label_size(cx);
        let [ascent, line] = line(cx);
        let [pad_x, pad_y] = cx.style.button_padding;
        let top = rect.y + label_height;
        Parts {
            field: [rect.x, top, rect.width, line + 2.0 * pad_y],
            text: [
                rect.x + pad_x,
                top + pad_y,
                (rect.width - 2.0 * pad_x).max(0.0),
                line,
            ],
            baseline: top + pad_y + ascent,
        }
    }

    /// What a key press does, or `None` for a key press that the app receives instead.
    fn action<'a>(&self, press: &'a SceneKeyPressEvent) -> Option<Action<'a>> {
        let modifiers = press.modifiers;
        // Cmd or Ctrl makes a shortcut, except with Alt, as Windows reports AltGr.
        let command = modifiers.meta || (modifiers.control && !modifiers.alt);
        let reach = match modifiers {
            _ if modifiers.meta => Reach::Line,
            _ if modifiers.alt || modifiers.control => Reach::Word,
            _ => Reach::Grapheme,
        };
        Some(match press.key {
            Key::Named(NamedKey::ArrowLeft) => Action::Move(reach, false),
            Key::Named(NamedKey::ArrowRight) => Action::Move(reach, true),
            Key::Named(NamedKey::Home) => Action::Move(Reach::Line, false),
            Key::Named(NamedKey::End) => Action::Move(Reach::Line, true),
            Key::Named(NamedKey::Backspace) => Action::Delete(reach, false),
            Key::Named(NamedKey::Delete) => Action::Delete(reach, true),
            Key::Named(NamedKey::Enter) if self.on_submit.is_some() => Action::Submit,
            Key::Character(key) if command => match key.to_ascii_lowercase() {
                'a' => Action::SelectAll,
                'x' => Action::Cut,
                'c' | 'v' => Action::Clipboard,
                _ => return None,
            },
            _ if command => return None,
            _ => Action::Insert(press.text.as_deref()?),
        })
    }

    /// Carry out a key press's action and return the scenes to rebuild.
    fn key(
        &self,
        press: &SceneKeyPressEvent,
        app: &mut State,
        own: &mut TextInputState,
    ) -> Rebuild {
        let edit = &mut own.edit;
        match self.action(press) {
            Some(Action::Move(reach, forward)) => {
                edit.move_caret(reach, forward, press.modifiers.shift)
            }
            Some(Action::Delete(reach, forward)) => edit.delete(reach, forward),
            Some(Action::SelectAll) => edit.select_all(),
            Some(Action::Cut) => edit.insert(""),
            Some(Action::Insert(text)) => edit.insert(text),
            Some(Action::Submit) => {
                if let Some(on_submit) = &self.on_submit {
                    return on_submit(app, edit.text.clone());
                }
            }
            Some(Action::Clipboard) | None => {}
        }
        Rebuild::none()
    }

    /// Place the caret or select at a pointer `event`.
    fn point(
        &self,
        event: &SceneGraphEvent,
        parts: &Parts,
        cx: &WidgetContext<'_>,
        own: &mut TextInputState,
    ) -> Result<(), LabelError> {
        let shown = Shown::new(&own.edit.text, cx)?;
        let scroll = shown.scroll(own.edit.caret, own.scroll, parts, cx);
        let at = |position: [f32; 2]| {
            let point = [
                position[0] - parts.text[0] + scroll,
                position[1] - parts.text[1],
            ];
            shown.layout.position_at(point)
        };
        match event {
            SceneGraphEvent::MouseDown(down) if down.button == MouseButton::Left => {
                let count = down.count;
                let unit = match count {
                    1 => {
                        let offset = shown.offset(at(down.position));
                        own.edit.place(offset, down.modifiers.shift);
                        own.edit.anchor..own.edit.anchor
                    }
                    count => {
                        let unit = shown.unit(at(down.position), count);
                        own.edit.select(unit.clone());
                        unit
                    }
                };
                own.drag = Some(Drag { count, unit });
            }
            SceneGraphEvent::CursorMoved(moved) => {
                if let Some(drag) = &own.drag {
                    let to = shown.unit(at(moved.position), drag.count);
                    own.edit.span(drag.unit.clone(), to);
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// What a key press does to a text input.
enum Action<'a> {
    Move(Reach, bool),
    Delete(Reach, bool),
    SelectAll,
    Cut,
    /// Copy or paste, which the host carries out.
    Clipboard,
    Submit,
    Insert(&'a str),
}

/// Where a text input's parts lie in its rectangle.
struct Parts {
    /// The box below the label.
    field: Bounds,
    /// The text's line inside the box's padding.
    text: Bounds,
    baseline: f32,
}

/// A text input's text, selection, scroll and drag.
#[derive(Debug, Default)]
pub struct TextInputState {
    edit: Edit,
    /// How far the text scrolls left to keep the caret in the box.
    scroll: f32,
    drag: Option<Drag>,
    composition: Option<Composition>,
}

/// Text that an input method is composing, such as the accent after Option+E, which the input
/// shows in place of the selection until the input method commits or cancels it.
#[derive(Debug, Clone, PartialEq)]
struct Composition {
    text: String,
    /// The input method's cursor in `text`, as a byte range on character boundaries, or `None`
    /// to hide the caret.
    cursor: Option<Range<usize>>,
}

impl Composition {
    /// The composition that `event` shows, or `None` for empty text, which ends it.
    fn of(event: &SceneComposingEvent) -> Option<Self> {
        let text = event.text.clone();
        if text.is_empty() {
            return None;
        }
        let floor = |offset: usize| {
            (0..=offset.min(text.len()))
                .rev()
                .find(|&offset| text.is_char_boundary(offset))
                .unwrap_or(0)
        };
        let cursor = (event.cursor.as_ref()).map(|cursor| {
            let start = floor(cursor.start);
            start..floor(cursor.end).max(start)
        });
        Some(Self { text, cursor })
    }
}

/// What the input's line shows: the value, with the text that an input method is composing in
/// place of the selection.
struct Line {
    text: String,
    /// Where the caret is, which the scroll keeps in the box and input methods place their
    /// candidates beside, as a byte offset in `text`.
    caret: usize,
    /// Whether the caret shows while the selection is empty.
    caret_shown: bool,
    selection: Range<usize>,
    /// The composed text's range in `text`, which the input underlines.
    composing: Option<Range<usize>>,
}

impl Line {
    fn new(value: &str, anchor: usize, caret: usize, composition: Option<&Composition>) -> Self {
        let selection = anchor.min(caret)..anchor.max(caret);
        let Some(composition) = composition else {
            return Self {
                text: value.to_string(),
                caret,
                caret_shown: true,
                selection,
                composing: None,
            };
        };
        let start = selection.start;
        let text = format!(
            "{}{}{}",
            &value[..start],
            composition.text,
            &value[selection.end..]
        );
        let end = start + composition.text.len();
        let (caret, selection) = match &composition.cursor {
            Some(cursor) => (start + cursor.end, start + cursor.start..start + cursor.end),
            None => (end, end..end),
        };
        Self {
            text,
            caret,
            caret_shown: composition.cursor.is_some(),
            selection,
            composing: Some(start..end),
        }
    }
}

/// A press that began in the input, which extends the selection as the pointer moves by the
/// press's unit: a character for one press, a word for two, and all of the text for three.
#[derive(Debug)]
struct Drag {
    count: u32,
    /// What the press selected, which the selection keeps whole: the anchor, a word or all of
    /// the text.
    unit: Range<usize>,
}

/// What a text input shows.
#[derive(Debug, Clone, PartialEq)]
pub struct TextInputView {
    value: String,
    /// The selection's anchor and caret, on grapheme boundaries of the value.
    anchor: usize,
    caret: usize,
    scroll: f32,
    composition: Option<Composition>,
}

impl<State: Send + Sync + 'static> Widget<State> for TextInput<State> {
    type State = TextInputState;
    type View = TextInputView;

    fn id(&self) -> &str {
        &self.id
    }

    fn max_content(&self, direction: Direction, cx: &WidgetContext<'_>) -> f32 {
        let [label_width, label_height] = self.label_size(cx);
        match direction {
            Direction::Horizontal => label_width.max(cx.style.input_width[0]),
            Direction::Vertical => label_height + line(cx)[1] + 2.0 * cx.style.button_padding[1],
        }
    }

    /// The larger of the label width and the box's minimum width.
    fn width(&self, _height: f32, cx: &WidgetContext<'_>) -> Measurement {
        let [label_width, _] = self.label_size(cx);
        Measurement::always(label_width.max(cx.style.input_width[1]))
    }

    fn view(&self, app: &State, own: &TextInputState) -> TextInputView {
        let value = self.current(app);
        TextInputView {
            anchor: boundary(&value, own.edit.anchor),
            caret: boundary(&value, own.edit.caret),
            scroll: own.scroll,
            composition: own.composition.clone(),
            value,
        }
    }

    fn draw(
        &self,
        view: &TextInputView,
        placed: &Placed,
        cx: &WidgetContext<'_>,
    ) -> Result<Vec<SceneMark>, WidgetError> {
        let style = cx.style;
        let look = placed.look;
        let r = placed.rect;
        let parts = self.parts(r, cx);
        let [x, top, width, height] = parts.text;
        let line = Line::new(
            &view.value,
            view.anchor,
            view.caret,
            view.composition.as_ref(),
        );
        let shown = Shown::new(&line.text, cx)?;
        let scroll = shown.scroll(line.caret, view.scroll, &parts, cx);
        let color = label_color(style, look);
        let mut marks: Vec<SceneMark> = Vec::new();
        if let Some(label) = &self.label {
            label.bounds(cx)?;
            marks.push(
                label
                    .mark("text_input_label", [r.x, r.y], color, style)
                    .into(),
            );
        }
        let field = style.neutral.paint(Look {
            hovered: false,
            pressed: false,
            ..look
        });
        marks.push(
            rect(
                "field",
                parts.field,
                field.fill,
                field.border,
                style.border_width,
                style.corner_radius,
            )
            .into(),
        );

        let mut marks_in_line: Vec<SceneMark> = Vec::new();
        // Boxes under `range` of the line's text, each `[top, height]` within the line.
        let spans = |name: &str, range: Range<usize>, [top, height]: [f32; 2], fill| {
            let range = shown.shown_offset(range.start)..shown.shown_offset(range.end);
            (shown.layout.selection_rects(range).into_iter())
                .map(|span| {
                    let bounds = [x - scroll + span.x, top, span.width, height];
                    SceneMark::from(rect(name, bounds, fill, [0.0; 4], 0.0, 0.0))
                })
                .collect::<Vec<_>>()
        };
        if look.focused && !line.selection.is_empty() {
            let selection = line.selection.clone();
            let color = style.selection_color;
            marks_in_line.extend(spans("selection", selection, [top, height], color));
        }
        let text = match (&self.placeholder, line.text.is_empty()) {
            (Some(placeholder), true) => {
                Some((placeholder.clone(), style.neutral.disabled.foreground))
            }
            (_, true) => None,
            (_, false) => Some((Text::plain(shown.text.clone()), color)),
        };
        if let Some((text, color)) = text {
            let mark = SceneTextMark {
                baseline: TextBaseline::Alphabetic.into(),
                ..text.mark("text", [x - scroll, parts.baseline], color, style)
            };
            marks_in_line.push(mark.into());
        }
        if let Some(composing) = line.composing.clone() {
            let thickness = style.caret_width;
            let under = [top + height - thickness, thickness];
            marks_in_line.extend(spans("composing", composing, under, color));
        }
        if look.focused && line.selection.is_empty() && line.caret_shown {
            let caret = caret_box(&parts, &shown, &line, scroll, style);
            marks_in_line.push(rect("caret", caret, color, [0.0; 4], 0.0, 0.0).into());
        }
        let [_, field_top, _, field_height] = parts.field;
        let clip = Clip::Rect {
            x,
            y: field_top,
            width,
            height: field_height,
        };
        marks.push(
            SceneGroup {
                name: "line".to_string(),
                clip,
                marks: marks_in_line,
                ..Default::default()
            }
            .into(),
        );
        if look.focused {
            marks.push(focus_ring(parts.field, style.corner_radius, style));
        }
        marks.push(cover(parts.field));
        Ok(marks)
    }

    fn enabled(&self, app: &State) -> bool {
        binding::enabled(&self.enabled, app)
    }

    fn disposition(
        &self,
        event: &SceneGraphEvent,
        _placed: &Placed,
        _app: &State,
        _own: &TextInputState,
    ) -> Disposition {
        match event {
            SceneGraphEvent::KeyPress(press) if self.action(press).is_some() => {
                Disposition::Consume
            }
            SceneGraphEvent::KeyPress(_) | SceneGraphEvent::KeyRelease(_) => Disposition::Ignore,
            _ => Disposition::Consume,
        }
    }

    fn event(
        &self,
        event: WidgetEvent<'_>,
        placed: &Placed,
        cx: &WidgetContext<'_>,
        app: &mut State,
        own: &mut TextInputState,
    ) -> Rebuild {
        let value = self.current(app);
        own.edit.sync(&value);
        let parts = self.parts(placed.rect, cx);
        let mut rebuild = Rebuild::none();
        match event {
            WidgetEvent::Focus => {
                own.edit.select_all();
                own.composition = None;
            }
            WidgetEvent::Blur | WidgetEvent::Cancel => {
                own.drag = None;
                own.composition = None;
            }
            WidgetEvent::Input(SceneGraphEvent::KeyPress(press)) => {
                rebuild = self.key(press, app, own);
            }
            WidgetEvent::Input(SceneGraphEvent::Paste(paste)) => {
                own.composition = None;
                own.edit.insert(&single_line(&paste.text));
            }
            WidgetEvent::Input(SceneGraphEvent::Composing(composing)) => {
                own.composition = Composition::of(composing);
            }
            WidgetEvent::Input(SceneGraphEvent::Composed(composed)) => {
                own.composition = None;
                own.edit.insert(&single_line(&composed.text));
            }
            WidgetEvent::Input(SceneGraphEvent::MouseUp(up)) if up.button == MouseButton::Left => {
                own.drag = None;
            }
            // A press or drag where the value fails to typeset leaves the selection. A press
            // stops showing a composition, which the input method's next event shows again.
            WidgetEvent::Input(event) => {
                if matches!(event, SceneGraphEvent::MouseDown(_)) {
                    own.composition = None;
                }
                let _ = self.point(event, &parts, cx, own);
            }
        }
        if own.edit.text != value {
            if let Some(on_change) = &self.on_change {
                rebuild.merge(on_change(app, own.edit.text.clone()));
            }
            own.edit.sync(&self.current(app));
        }
        let edit = &own.edit;
        let line = Line::new(
            &edit.text,
            edit.anchor,
            edit.caret,
            own.composition.as_ref(),
        );
        if let Ok(shown) = Shown::new(&line.text, cx) {
            own.scroll = shown.scroll(line.caret, own.scroll, &parts, cx);
        }
        rebuild
    }

    fn cursor(&self, _look: Look, _own: &TextInputState) -> Option<CursorIcon> {
        Some(CursorIcon::Text)
    }

    fn copy_text<'a>(&self, view: &'a TextInputView) -> Option<&'a str> {
        let selection = view.anchor.min(view.caret)..view.anchor.max(view.caret);
        (!selection.is_empty()).then(|| &view.value[selection])
    }

    /// The caret's box, or the start of the text's line where the text fails to typeset.
    fn text_entry(
        &self,
        view: &TextInputView,
        placed: &Placed,
        cx: &WidgetContext<'_>,
    ) -> Option<[f32; 4]> {
        let parts = self.parts(placed.rect, cx);
        let line = Line::new(
            &view.value,
            view.anchor,
            view.caret,
            view.composition.as_ref(),
        );
        let [x, top, _, height] = parts.text;
        let caret = match Shown::new(&line.text, cx) {
            Ok(shown) => {
                let scroll = shown.scroll(line.caret, view.scroll, &parts, cx);
                caret_box(&parts, &shown, &line, scroll, cx.style)
            }
            Err(_) => [x, top, cx.style.caret_width, height],
        };
        Some(caret)
    }
}

/// The caret's box in `line`, shown with `scroll`.
fn caret_box(
    parts: &Parts,
    shown: &Shown<'_>,
    line: &Line,
    scroll: f32,
    style: &WidgetStyle,
) -> Bounds {
    let [x, top, _, height] = parts.text;
    let caret_x = x - scroll + shown.x(line.caret);
    [caret_x, top, style.caret_width, height]
}

/// A line's text as the input shows it, with each space, tab or line break as a no-break space,
/// which the label engine neither collapses nor trims, and its layout.
struct Shown<'a> {
    value: &'a str,
    text: String,
    layout: Arc<TextLayout>,
}

impl<'a> Shown<'a> {
    fn new(value: &'a str, cx: &WidgetContext<'_>) -> Result<Self, LabelError> {
        let text: String = (value.chars())
            .map(|c| if spaced(c) { '\u{a0}' } else { c })
            .collect();
        let layout = Text::plain(text.clone()).layout(cx)?;
        Ok(Self {
            value,
            text,
            layout,
        })
    }

    /// The offset in the shown text of the value's `offset`.
    fn shown_offset(&self, offset: usize) -> usize {
        offset + self.value[..offset].chars().filter(|&c| spaced(c)).count()
    }

    /// The value's offset of the shown text's `offset`.
    fn offset_of(&self, offset: usize) -> usize {
        let mut shown = 0;
        for (index, c) in self.value.char_indices() {
            if shown >= offset {
                return index;
            }
            shown += if spaced(c) {
                '\u{a0}'.len_utf8()
            } else {
                c.len_utf8()
            };
        }
        self.value.len()
    }

    /// The value's offset at a position in the shown text.
    fn offset(&self, position: TextPosition) -> usize {
        self.offset_of(position.offset)
    }

    /// The value's range that `count` presses at a position in the shown text select: the
    /// offset for one press, the word or the character between words there for two, and all of
    /// the text for three or more.
    fn unit(&self, position: TextPosition, count: u32) -> Range<usize> {
        match count {
            ..=1 => {
                let offset = self.offset(position);
                offset..offset
            }
            2 => {
                let word = self.layout.word_at(position);
                self.offset_of(word.start)..self.offset_of(word.end)
            }
            _ => 0..self.value.len(),
        }
    }

    /// The caret's distance from the text's start at the value's `offset`.
    fn x(&self, offset: usize) -> f32 {
        let position = TextPosition::new(self.shown_offset(offset), Affinity::Downstream);
        self.layout.caret(position).x
    }

    /// The distance from the text's start to its farthest edge, which is its logical start for
    /// right-to-left text.
    fn width(&self) -> f32 {
        (self.layout.selection_rects(0..self.text.len()).iter())
            .map(|span| span.x + span.width)
            .fold(0.0, f32::max)
    }

    /// The scroll nearest `scroll` that shows the caret at the value's `caret` inside the box,
    /// and leaves no space after the text's farthest edge while the text is wider than the box.
    fn scroll(&self, caret: usize, scroll: f32, parts: &Parts, cx: &WidgetContext<'_>) -> f32 {
        let room = (parts.text[2] - cx.style.caret_width).max(0.0);
        let caret = self.x(caret);
        let end = self.width();
        scroll
            .clamp(caret - room, caret)
            .clamp(0.0, (end - room).max(0.0))
    }
}

/// Whether the input shows `c` as a no-break space.
fn spaced(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// `text` with each line break or tab as a space, and without other control characters.
fn single_line(text: &str) -> String {
    (text.replace("\r\n", " ").chars())
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

/// The distance from the top of a line of text to its baseline, and the line's height, in the
/// style's font. Without the font's metrics, both are the font size.
fn line(cx: &WidgetContext<'_>) -> [f32; 2] {
    let style = cx.style;
    let text = text_style(
        &style.font_family,
        style.font_size,
        FontWeight::NORMAL,
        FontStyle::Normal,
        [0.0; 4],
    );
    (cx.engine.font_metrics(&text)).map_or([style.font_size; 2], |font| {
        [font.ascent, font.ascent + font.descent]
    })
}
