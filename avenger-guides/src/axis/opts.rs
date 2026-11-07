use arrow::array::ArrayRef;
use avenger_common::types::{FontWeight, TextAlign, TextBaseline, TextSyntaxMode};

use crate::GuideSide;

/// What an axis shows and how it looks, at any length.
#[derive(Debug, Clone)]
pub struct AxisConfig {
    /// The side of the plot that the axis stands on.
    pub side: GuideSide,
    /// The pattern for numeric and temporal ticks, in the syntax of the text engine's number or
    /// datetime provider, such as `",.0f"` or `"%b %d"`. An empty pattern takes the provider's
    /// default: for dates and times, its calendar patterns. Other ticks, such as strings, show
    /// as text.
    pub tick_format: String,
    /// Typst markup for each tick label, where `#value` is the tick as `tick_format` formats it,
    /// such as `#value m/s`. Without it, labels are the formatted ticks as plain text.
    pub tick_label: Option<String>,
    pub style: AxisStyle,
}

/// Axis appearance and tick placement. `AxisStyle::default()` is the default look, and
/// `..Default::default()` keeps it for the settings that aren't given.
#[derive(Debug, Clone)]
pub struct AxisStyle {
    /// About how many ticks to draw. Without it, the axis fits a count to its length, with ticks
    /// at least `tick_spacing` apart, and a continuous axis takes coarser ticks where their labels
    /// would overlap. `ticks` replaces it.
    pub tick_count: Option<f64>,
    /// The least distance between the ticks of a continuous axis that fits its tick count to its
    /// length, in pixels, along a horizontal axis and a vertical one alike. When absent, 3.5 times
    /// the font size of the labels, so the ticks spread with the size of their labels.
    pub tick_spacing: Option<f32>,
    /// Ticks in place of the scale's ticks. A band or point axis takes only values, and a nested
    /// band axis takes neither kind.
    pub ticks: Option<AxisTicks>,
    pub tick_length: f32,
    pub tick_color: [f32; 4],
    pub labels_visible: bool,
    /// Degrees clockwise.
    pub label_angle: f32,
    /// The angle, in degrees clockwise, that the labels of a horizontal band or point axis turn
    /// to at a length where one of them is wider than the step between its ticks, such as 270. The
    /// angle depends only on the length, so a layout that measures the axis at one length always
    /// finds one thickness. When absent, the labels keep `label_angle` at every length.
    pub label_turn: Option<f32>,
    /// Without it, labels align to the end nearest their tick, for the axis side and label angle.
    pub label_align: Option<TextAlign>,
    /// Without it, labels align to the end nearest their tick, for the axis side and label angle.
    pub label_baseline: Option<TextBaseline>,
    /// How labels that overlap drop out.
    pub label_thinning: LabelThinning,
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
            tick_spacing: None,
            ticks: None,
            tick_length: 5.0,
            tick_color: BLACK,
            labels_visible: true,
            label_angle: 0.0,
            label_turn: None,
            label_align: None,
            label_baseline: None,
            label_thinning: LabelThinning::default(),
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

/// How an axis drops tick labels that overlap, after a continuous axis that fits its tick count
/// has taken coarser ticks. Labels that show keep a pixel apart, and every tick draws whether its
/// label shows or not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LabelThinning {
    /// Every label shows, overlapping or not.
    None,
    /// Every label, or else every second, every third, and so on from the first, at the smallest
    /// stride whose labels keep apart, so that the labels that show keep an even pattern.
    #[default]
    Stride,
}

/// Ticks in place of the scale's ticks. The axis labels each of them.
#[derive(Debug, Clone)]
pub enum AxisTicks {
    /// Ticks at a start value and whole multiples of a step before and after it, across the
    /// domain.
    StartStep(AxisTickInterval),
    /// These values, in the type of the scale's domain: numbers, dates, or timestamps, of which a
    /// continuous axis draws the ones inside its domain at each length, or categories, of which a
    /// band or point axis draws the ones in its domain, in this order. Nulls draw no tick.
    Values(ArrayRef),
    /// The edges of bins, ascending, in the type of the scale's domain, of which a continuous axis
    /// draws the ones inside its domain: every edge where they stand apart at the axis's tick
    /// count, and otherwise every 2nd, 5th, 10th, 20th, and so on from the first, so the ticks
    /// stay on edges.
    Edges(ArrayRef),
}

/// The start value and the step of `AxisTicks::StartStep`.
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
