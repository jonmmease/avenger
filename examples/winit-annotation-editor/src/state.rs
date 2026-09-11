use std::sync::{Arc, Mutex, OnceLock, Weak};

use avenger_eventstream::runtime::{DebounceConfig, DebouncedCommit, RuntimeWakeKey};
use avenger_text::{
    measurement::TextMeasurementConfig,
    text_edit::{cursor_rect_for_offset, Action, ShapedLine, SingleLineEditor},
    types::{FontStyle, FontWeight, TextSyntaxMode},
    LabelParams, TextEngine,
};

use crate::reload::ReloadCoordinator;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    A,
    B,
}

impl Sample {
    pub fn name(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Point {
    pub name: String,
    pub position: [f32; 2],
    pub annotation: String,
    pub offset: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drag {
    Field,
    Annotation,
    Plot,
}

#[derive(Clone)]
pub struct State {
    pub sample: Sample,
    pub generation: u64,
    pub points: Vec<Point>,
    pub selected: usize,
    pub size: [f32; 2],
    pub pan: [f32; 2],
    pub editor: SingleLineEditor,
    pub composition_snapshot: Option<(String, avenger_text::text_edit::SelectionState)>,
    pub engine: TextEngine,
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
    pub load_feedback: Arc<Mutex<Option<Result<(), String>>>>,
    pub debounce: DebouncedCommit<String>,
    pub error: Option<String>,
    pub loading: Option<Sample>,
    pub reload: Weak<ReloadCoordinator>,
    pub scene_builds: usize,
}

impl State {
    pub fn new(sample: Sample, generation: u64, engine: TextEngine) -> Self {
        let positions = match sample {
            Sample::A => [
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
            ],
            Sample::B => [
                [10., 72.],
                [18., 63.],
                [25., 80.],
                [33., 57.],
                [40., 65.],
                [47., 43.],
                [54., 55.],
                [61., 38.],
                [69., 47.],
                [76., 26.],
                [83., 35.],
                [91., 21.],
            ],
        };
        let points: Vec<_> = positions
            .into_iter()
            .enumerate()
            .map(|(i, position)| Point {
                name: format!("Point {:02}", i + 1),
                position,
                annotation: if i == 7 {
                    "A point worth discussing".into()
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
            sample,
            generation,
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
            load_feedback: Arc::new(Mutex::new(None)),
            debounce: DebouncedCommit::new(DebounceConfig::new(350)),
            error: None,
            loading: None,
            reload: Weak::new(),
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
    pub fn point_position(&self, index: usize) -> [f32; 2] {
        let [x, y, w, h] = self.plot();
        let p = &self.points[index];
        [
            x + p.position[0] * w / 100.0 + self.pan[0],
            y + h - p.position[1] * h / 100.0 + self.pan[1],
        ]
    }
    pub fn key(&self, purpose: &str) -> RuntimeWakeKey {
        RuntimeWakeKey::new(
            "annotation-editor",
            self.generation,
            format!(
                "{}-{purpose}",
                if matches!(purpose, "hover" | "load") {
                    0
                } else {
                    self.session
                }
            ),
        )
    }
    pub fn tooltip_owner(&self) -> String {
        format!(
            "annotation-editor-{}-{}",
            self.generation, self.hover_generation
        )
    }
    pub fn pending(&self) -> bool {
        self.editor.committed_text().into_string() != self.points[self.selected].annotation
    }
    pub fn apply_action(&mut self, action: Action) -> bool {
        match self.editor.apply(action, &self.engine, &text_config()) {
            Ok(changed) => {
                self.error = None;
                changed
            }
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
    }
    pub fn shaped_line(&mut self) -> Result<ShapedLine, String> {
        self.editor
            .shape_line(&self.engine, &text_config())
            .cloned()
            .map_err(|e| e.to_string())
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

pub fn text_config() -> TextMeasurementConfig<'static> {
    static PARAMS: OnceLock<LabelParams> = OnceLock::new();
    TextMeasurementConfig {
        text: "",
        font: "sans-serif",
        font_size: 17.0,
        font_weight: FontWeight::default(),
        font_style: FontStyle::Normal,
        syntax_mode: TextSyntaxMode::Plain,
        params: PARAMS.get_or_init(Default::default),
        number_locale: None,
        number_locale_specs: None,
        datetime_locale: None,
        datetime_timezone: None,
        datetime_locale_specs: None,
    }
}
