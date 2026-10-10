use avenger_common::types::TextSyntaxMode;
use avenger_format::PreparedFormatter;

#[derive(Debug, Clone, Copy)]
pub enum AxisOrientation {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub struct AxisConfig {
    pub orientation: AxisOrientation,
    pub dimensions: [f32; 2],
    pub grid: bool,
    pub format: PreparedFormatter,
    pub style: AxisStyle,
}

/// Optional axis styling, label templates, and tick placement. Use `..Default::default()` to set
/// a subset.
#[derive(Debug, Clone)]
pub struct AxisStyle {
    /// Typst markup for each tick label on continuous axes, where `#label` stands for the
    /// formatted tick, such as `v = #label m/s^2`. Numeric labels keep their exponent typesetting.
    pub tick_label: Option<String>,
    pub title_font_size: Option<f32>,
    // Theming
    pub domain_color: Option<[f32; 4]>,
    pub tick_color: Option<[f32; 4]>,
    pub grid_color: Option<[f32; 4]>,
    pub grid_width: Option<f32>,
    pub label_color: Option<[f32; 4]>,
    pub title_color: Option<[f32; 4]>,
    pub tick_length: Option<f32>,
    pub label_font_size: Option<f32>,
    pub label_font_weight: Option<f32>,
    pub label_angle: Option<f32>,
    pub title_font_weight: Option<f32>,
    pub label_font_family: Option<String>,
    pub title_font_family: Option<String>,
    pub title_syntax_mode: TextSyntaxMode,
    pub title_visible: Option<bool>,
    pub labels_visible: Option<bool>,
    pub tick_count: Option<f32>,
    pub tick_start_step: Option<AxisTickInterval>,
}

/// Ticks at a start value and fixed steps, in place of the scale's ticks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AxisTickInterval {
    Numeric {
        start: f32,
        step: f32,
    },
    Temporal {
        start_millis: i64,
        months: i32,
        days: i32,
        nanos: i64,
    },
}

impl Default for AxisStyle {
    fn default() -> Self {
        Self {
            tick_label: None,
            title_font_size: None,
            domain_color: None,
            tick_color: None,
            grid_color: None,
            grid_width: None,
            label_color: None,
            title_color: None,
            tick_length: None,
            label_font_size: None,
            label_font_weight: None,
            label_angle: None,
            title_font_weight: None,
            label_font_family: None,
            title_font_family: None,
            title_syntax_mode: TextSyntaxMode::Plain,
            title_visible: None,
            labels_visible: None,
            tick_count: None,
            tick_start_step: None,
        }
    }
}
