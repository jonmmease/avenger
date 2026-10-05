use avenger_common::types::{FontWeight, TextAlign, TextBaseline, TextSyntaxMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// The pattern for numeric and temporal ticks, in the syntax of the text engine's number or
    /// datetime provider, such as `",.0f"` or `"%b %d"`. An empty pattern takes the provider's
    /// default: for dates and times, its calendar patterns. Other ticks, such as strings, show
    /// as text.
    pub tick_format: String,
    /// Typst markup for each tick label, where `#tick` is the tick as `tick_format` formats it,
    /// such as `#tick m/s`. Without it, labels are the formatted ticks as plain text.
    pub tick_label: Option<String>,
    pub style: AxisStyle,
}

/// Axis appearance and tick placement. `AxisStyle::default()` is the default look, and
/// `..Default::default()` keeps it for the settings that aren't given.
#[derive(Debug, Clone)]
pub struct AxisStyle {
    /// About how many ticks to draw. Without it, the axis fits a count to its length.
    pub tick_count: Option<f64>,
    /// Ticks at a start value and fixed steps, in place of the scale's ticks.
    pub tick_start_step: Option<AxisTickInterval>,
    pub tick_length: f32,
    pub tick_color: [f32; 4],
    pub labels_visible: bool,
    /// Degrees clockwise.
    pub label_angle: f32,
    /// Without it, labels align to the end nearest their tick, for the axis side and label angle.
    pub label_align: Option<TextAlign>,
    /// Without it, labels align to the end nearest their tick, for the axis side and label angle.
    pub label_baseline: Option<TextBaseline>,
    pub label_color: [f32; 4],
    pub label_font_family: String,
    pub label_font_size: f32,
    pub label_font_weight: FontWeight,
    pub title_visible: bool,
    pub title_syntax: TextSyntaxMode,
    pub title_color: [f32; 4],
    pub title_font_family: String,
    pub title_font_size: f32,
    pub title_font_weight: FontWeight,
    pub domain_color: [f32; 4],
    pub grid_color: [f32; 4],
    pub grid_width: f32,
}

impl Default for AxisStyle {
    fn default() -> Self {
        const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
        Self {
            tick_count: None,
            tick_start_step: None,
            tick_length: 5.0,
            tick_color: BLACK,
            labels_visible: true,
            label_angle: 0.0,
            label_align: None,
            label_baseline: None,
            label_color: BLACK,
            label_font_family: "sans-serif".to_string(),
            label_font_size: 12.0,
            label_font_weight: FontWeight::default(),
            title_visible: true,
            title_syntax: TextSyntaxMode::default(),
            title_color: BLACK,
            title_font_family: "sans-serif".to_string(),
            title_font_size: 12.0,
            title_font_weight: FontWeight::default(),
            domain_color: BLACK,
            // #E0E0E0 at half opacity
            grid_color: [0.878, 0.878, 0.878, 0.5],
            grid_width: 0.5,
        }
    }
}

/// Ticks at a start value and whole multiples of a step before and after it, in place of the
/// scale's ticks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AxisTickInterval {
    Numeric {
        start: f64,
        step: f64,
    },
    /// A start instant and a calendar step with one part: `months`, `days`, or `nanos`, a whole
    /// number of milliseconds. Days and months step in the scale's civil time.
    Temporal {
        start_millis: i64,
        months: i32,
        days: i32,
        nanos: i64,
    },
}
