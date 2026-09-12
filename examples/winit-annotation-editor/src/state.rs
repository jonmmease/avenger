use std::sync::{Arc, Mutex, OnceLock, Weak};

use avenger_eventstream::runtime::RuntimeWakeKey;
use avenger_scales::{
    error::AvengerScaleError,
    scales::{linear::LinearScale, ConfiguredScale},
};
use avenger_text::{
    measurement::TextMeasurementConfig,
    types::{FontStyle, FontWeight, TextSyntaxMode},
    LabelParams, TextEngine,
};
use avenger_widgets::{TextShortcuts, WidgetRuntime};

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
    pub draft: String,
    pub widgets: WidgetRuntime,
    pub engine: TextEngine,
    pub window_focused: bool,
    pub hover_generation: u64,
    pub hover_point: Option<usize>,
    pub hover_visible: bool,
    pub pointer: [f32; 2],
    pub drag: Option<Drag>,
    pub drag_origin: [f32; 2],
    pub load_feedback: Arc<Mutex<Option<Result<(), String>>>>,
    pub error: Option<String>,
    pub annotation_error: Option<String>,
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
                    "*Radius* $sqrt(x^2+y^2)$".into()
                } else {
                    String::new()
                },
                offset: [-118.0, -52.0],
            })
            .collect();
        Self {
            draft: points[7].annotation.clone(),
            widgets: WidgetRuntime::new().with_text_shortcuts(if uses_mac_shortcuts() {
                TextShortcuts::Mac
            } else {
                TextShortcuts::Control
            }),
            points,
            selected: 7,
            sample,
            generation,
            engine,
            size: [1000.0, 680.0],
            pan: [0.0; 2],
            window_focused: true,
            hover_generation: 0,
            hover_point: None,
            hover_visible: false,
            pointer: [0.0; 2],
            drag: None,
            drag_origin: [0.0; 2],
            load_feedback: Arc::new(Mutex::new(None)),
            error: None,
            annotation_error: None,
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
        RuntimeWakeKey::new("annotation-editor", self.generation, format!("0-{purpose}"))
    }
    pub fn tooltip_owner(&self) -> String {
        format!(
            "annotation-editor-{}-{}",
            self.generation, self.hover_generation
        )
    }
    pub fn pending(&self) -> bool {
        self.draft != self.points[self.selected].annotation
    }
    pub fn focused(&self) -> bool {
        self.widgets.focused().is_some()
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

// Validation and the chart annotation use the same typesetting configuration.
pub fn annotation_config(text: &str) -> TextMeasurementConfig<'_> {
    TextMeasurementConfig {
        text,
        font_size: 16.0,
        syntax_mode: TextSyntaxMode::TypstMarkup,
        ..text_config()
    }
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
