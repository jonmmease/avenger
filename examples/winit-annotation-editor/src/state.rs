use std::sync::{Arc, Mutex};

use avenger_common::types::TextSyntaxMode;
use avenger_eventstream::runtime::{DebounceConfig, DebouncedCommit, RuntimeWakeKey};
use avenger_scales::{
    error::AvengerScaleError,
    scales::{linear::LinearScale, ConfiguredScale},
};
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_typst_label::{LabelEngine, LabelError, TextBounds, TextStyle};

use crate::text_edit::{
    cursor_rect_for_offset, shape_line, Action, SelectionState, ShapedLine, SingleLineEditor,
};

#[derive(Clone, Debug)]
pub struct Point {
    pub name: String,
    pub position: [f32; 2],
    pub annotation: String,
    pub offset: [f32; 2],
}

pub struct PlotScales {
    pub x: ConfiguredScale,
    pub y: ConfiguredScale,
}

impl PlotScales {
    pub fn position(&self, [x, y]: [f32; 2]) -> Result<[f32; 2], AvengerScaleError> {
        Ok([
            self.x.scale_scalar(&x)?.as_f32()?,
            self.y.scale_scalar(&y)?.as_f32()?,
        ])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drag {
    Field,
    Annotation,
    Plot,
}

#[derive(Clone)]
pub struct State {
    pub points: Vec<Point>,
    pub selected: usize,
    pub size: [f32; 2],
    pub pan: [f32; 2],
    pub editor: SingleLineEditor,
    pub composition_snapshot: Option<(String, SelectionState)>,
    pub engine: LabelEngine,
    pub focused: bool,
    pub window_focused: bool,
    pub caret_visible: bool,
    pub scroll: f32,
    pub session: u64,
    pub blink_generation: u64,
    pub hover_generation: u64,
    pub hover_point: Option<usize>,
    pub hover_visible: bool,
    pub pointer: [f32; 2],
    pub drag: Option<Drag>,
    pub drag_origin: [f32; 2],
    pub debounce: DebouncedCommit<String>,
    pub error: Option<String>,
    pub annotation_error: Option<String>,
    pub clipboard_text: Arc<Mutex<String>>,
    pub mac_shortcuts: bool,
    pub scene_builds: usize,
}

impl State {
    pub fn new(engine: LabelEngine) -> Self {
        let positions = [
            [12., 20.],
            [20., 32.],
            [28., 27.],
            [34., 49.],
            [43., 43.],
            [51., 58.],
            [58., 51.],
            [64., 71.],
            [71., 64.],
            [78., 83.],
            [85., 73.],
            [91., 88.],
        ];
        let points: Vec<_> = positions
            .into_iter()
            .enumerate()
            .map(|(i, position)| Point {
                name: format!("Point {:02}", i + 1),
                position,
                annotation: if i == 7 {
                    "*Radius* $sqrt(x^2+y^2)$".into()
                } else {
                    String::new()
                },
                offset: [-118.0, -52.0],
            })
            .collect();
        Self {
            editor: SingleLineEditor::new(points[7].annotation.clone()),
            composition_snapshot: None,
            points,
            selected: 7,
            engine,
            size: [1000.0, 680.0],
            pan: [0.0; 2],
            focused: false,
            window_focused: true,
            caret_visible: false,
            scroll: 0.0,
            session: 0,
            blink_generation: 0,
            hover_generation: 0,
            hover_point: None,
            hover_visible: false,
            pointer: [0.0; 2],
            drag: None,
            drag_origin: [0.0; 2],
            debounce: DebouncedCommit::new(DebounceConfig::new(350)),
            error: None,
            annotation_error: None,
            clipboard_text: Default::default(),
            mac_shortcuts: uses_mac_shortcuts(),
            scene_builds: 0,
        }
    }

    pub fn plot(&self) -> [f32; 4] {
        [60.0, 142.0, self.size[0] - 420.0, self.size[1] - 242.0]
    }
    pub fn field(&self) -> [f32; 4] {
        [self.size[0] - 284.0, 252.0, 252.0, 44.0]
    }
    pub fn field_text_origin(&self) -> [f32; 2] {
        [self.field()[0] + 10.0 - self.scroll, self.field()[1] + 11.0]
    }
    pub fn scales(&self) -> Result<PlotScales, AvengerScaleError> {
        let [x, y, w, h] = self.plot();
        // Pan the visible domains without rounding them. The reversed y range
        // needs the opposite pan fraction to follow the pointer down the screen.
        Ok(PlotScales {
            x: LinearScale::configured((0.0, 100.0), (x, x + w)).pan(self.pan[0] / w)?,
            y: LinearScale::configured((0.0, 100.0), (y + h, y)).pan(-self.pan[1] / h)?,
        })
    }
    pub fn point_position(&self, index: usize) -> Result<[f32; 2], AvengerScaleError> {
        self.scales()?.position(self.points[index].position)
    }
    pub fn key(&self, purpose: &str) -> RuntimeWakeKey {
        RuntimeWakeKey::new(
            "annotation-editor",
            0,
            format!(
                "{}-{purpose}",
                if purpose == "hover" { 0 } else { self.session }
            ),
        )
    }
    pub fn tooltip_owner(&self) -> String {
        format!("annotation-editor-{}", self.hover_generation)
    }
    pub fn pending(&self) -> bool {
        self.editor.committed_text() != self.points[self.selected].annotation
    }
    pub fn apply_action(&mut self, action: Action) -> bool {
        let before = self.editor.text().to_string();
        match self.editor.apply(action, &self.engine, &editor_style()) {
            Ok(changed) => {
                if before != self.editor.text() {
                    self.annotation_error = None;
                }
                self.error = None;
                changed
            }
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
    }
    pub fn shaped_line(&self) -> Result<ShapedLine, String> {
        shape_line(&self.engine, self.editor.text(), &editor_style()).map_err(|e| e.to_string())
    }
    pub fn keep_caret_visible(&mut self) {
        let cursor = self.editor.selection().head;
        if let Ok(line) = self.shaped_line() {
            let caret = cursor_rect_for_offset(&line, cursor.index, cursor.affinity);
            let width = self.field()[2] - 22.0;
            if caret.x < self.scroll {
                self.scroll = caret.x;
            }
            if caret.x > self.scroll + width {
                self.scroll = caret.x - width;
            }
            self.scroll = self
                .scroll
                .max(0.0)
                .min((line.bounds.width - width).max(0.0));
        }
    }
}

/// The editor field's text style.
pub fn editor_style() -> TextStyle {
    TextStyle {
        font_family: "sans-serif".into(),
        font_size: 17.0,
        ..Default::default()
    }
}

// Validation and the chart annotation typeset the same mark.
pub fn annotation_mark(text: &str) -> SceneTextMark {
    SceneTextMark {
        text: text.to_string().into(),
        text_syntax: TextSyntaxMode::TypstMarkup,
        font: "sans-serif".to_string().into(),
        font_size: 16.0.into(),
        ..Default::default()
    }
}

/// The annotation's bounds, or why its markup doesn't typeset.
pub fn annotation_bounds(engine: &LabelEngine, text: &str) -> Result<TextBounds, LabelError> {
    let mark = annotation_mark(text);
    let label = mark
        .labels()
        .next()
        .expect("a scalar text mark has one label");
    let options = &label.label.options;
    // Unlike `bounds`, which measures invalid markup as the literal text the chart draws.
    let metrics = engine.measure(text, options)?;
    Ok(TextBounds::new(&metrics, options.text.font_size))
}

fn uses_mac_shortcuts() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|window| window.navigator().platform().ok())
            .is_some_and(|platform| platform.starts_with("Mac") || platform.starts_with("iP"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        cfg!(target_os = "macos")
    }
}
