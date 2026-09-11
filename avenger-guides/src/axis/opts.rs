use std::sync::Arc;

use avenger_format_datetime::DateTimeLocaleRegistry;
use avenger_format_number::NumberLocaleRegistry;
use avenger_text::{types::TextSyntaxMode, DateTimeLocaleSpecs, LabelParams, NumberLocaleSpecs};

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
    pub format_number: Option<String>,
    pub format_datetime: Option<String>,
    pub tick_label: Option<String>,
    pub number_locale: Option<String>,
    pub number_locale_registry: Option<Arc<NumberLocaleRegistry>>,
    pub number_locale_specs: NumberLocaleSpecs,
    pub datetime_locale: Option<String>,
    pub datetime_timezone: Option<String>,
    pub datetime_locale_registry: Option<Arc<DateTimeLocaleRegistry>>,
    pub datetime_locale_specs: DateTimeLocaleSpecs,
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
    pub title_text_params: LabelParams,
    pub title_visible: Option<bool>,
    pub labels_visible: Option<bool>,
    pub tick_count: Option<f32>,
    pub tick_start_step: Option<AxisTickSpacing>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AxisTickSpacing {
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

impl Default for AxisConfig {
    fn default() -> Self {
        Self {
            orientation: AxisOrientation::Bottom,
            dimensions: [100.0, 100.0],
            grid: false,
            format_number: None,
            format_datetime: None,
            tick_label: None,
            number_locale: None,
            number_locale_registry: None,
            number_locale_specs: NumberLocaleSpecs::default(),
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_registry: None,
            datetime_locale_specs: DateTimeLocaleSpecs::default(),
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
            title_text_params: LabelParams::default(),
            title_visible: None,
            labels_visible: None,
            tick_count: None,
            tick_start_step: None,
        }
    }
}
