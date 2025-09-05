//! Default values for mark properties

use datafusion_common::ScalarValue;
use indexmap::IndexMap;

/// Mark defaults configuration
#[derive(Clone, Debug)]
pub struct MarkDefaults {
    /// Default values per mark type and channel
    /// e.g., "symbol" -> "fill" -> ScalarValue::Utf8(Some("#4682b4"))
    pub defaults: IndexMap<String, IndexMap<String, ScalarValue>>,
}

impl MarkDefaults {
    /// Create a new empty mark defaults
    pub fn new() -> Self {
        Self {
            defaults: IndexMap::new(),
        }
    }

    /// Get default value for a specific mark type and channel
    pub fn get(&self, mark_type: &str, channel: &str) -> Option<&ScalarValue> {
        self.defaults
            .get(mark_type)
            .and_then(|channels| channels.get(channel))
    }

    /// Builder method to set a default for a specific mark and channel
    pub fn with_default(mut self, mark_type: &str, channel: &str, value: ScalarValue) -> Self {
        self.defaults
            .entry(mark_type.to_string())
            .or_insert_with(IndexMap::new)
            .insert(channel.to_string(), value);
        self
    }
}

impl Default for MarkDefaults {
    fn default() -> Self {
        let mut defaults = IndexMap::new();

        // Symbol mark defaults
        let mut symbol_defaults = IndexMap::new();
        symbol_defaults.insert(
            "fill".to_string(),
            ScalarValue::Utf8(Some("#4682b4".to_string())),
        );
        symbol_defaults.insert(
            "stroke".to_string(),
            ScalarValue::Utf8(Some("#000000".to_string())),
        );
        symbol_defaults.insert("stroke_width".to_string(), ScalarValue::Float32(Some(1.0)));
        symbol_defaults.insert("size".to_string(), ScalarValue::Float32(Some(64.0)));
        symbol_defaults.insert(
            "shape".to_string(),
            ScalarValue::Utf8(Some("circle".to_string())),
        );
        symbol_defaults.insert("angle".to_string(), ScalarValue::Float32(Some(0.0)));
        symbol_defaults.insert("opacity".to_string(), ScalarValue::Float32(Some(1.0)));
        defaults.insert("symbol".to_string(), symbol_defaults);

        // Rect mark defaults
        let mut rect_defaults = IndexMap::new();
        rect_defaults.insert(
            "fill".to_string(),
            ScalarValue::Utf8(Some("#4682b4".to_string())),
        );
        rect_defaults.insert(
            "stroke".to_string(),
            ScalarValue::Utf8(Some("#000000".to_string())),
        );
        rect_defaults.insert("stroke_width".to_string(), ScalarValue::Float32(Some(0.0)));
        rect_defaults.insert("corner_radius".to_string(), ScalarValue::Float32(Some(0.0)));
        rect_defaults.insert("opacity".to_string(), ScalarValue::Float32(Some(1.0)));
        defaults.insert("rect".to_string(), rect_defaults);

        // Line mark defaults
        let mut line_defaults = IndexMap::new();
        line_defaults.insert(
            "stroke".to_string(),
            ScalarValue::Utf8(Some("#4682b4".to_string())),
        );
        line_defaults.insert("stroke_width".to_string(), ScalarValue::Float32(Some(2.0)));
        line_defaults.insert(
            "stroke_dash".to_string(),
            ScalarValue::Utf8(Some("solid".to_string())),
        );
        line_defaults.insert(
            "stroke_cap".to_string(),
            ScalarValue::Utf8(Some("round".to_string())),
        );
        line_defaults.insert(
            "stroke_join".to_string(),
            ScalarValue::Utf8(Some("round".to_string())),
        );
        line_defaults.insert("opacity".to_string(), ScalarValue::Float32(Some(1.0)));
        line_defaults.insert("defined".to_string(), ScalarValue::Boolean(Some(true)));
        defaults.insert("line".to_string(), line_defaults);

        // Area mark defaults
        let mut area_defaults = IndexMap::new();
        area_defaults.insert(
            "fill".to_string(),
            ScalarValue::Utf8(Some("#4682b4".to_string())),
        );
        area_defaults.insert("fill_opacity".to_string(), ScalarValue::Float32(Some(0.7)));
        area_defaults.insert(
            "stroke".to_string(),
            ScalarValue::Utf8(Some("#000000".to_string())),
        );
        area_defaults.insert("stroke_width".to_string(), ScalarValue::Float32(Some(0.0)));
        defaults.insert("area".to_string(), area_defaults);

        // Arc mark defaults
        let mut arc_defaults = IndexMap::new();
        arc_defaults.insert(
            "fill".to_string(),
            ScalarValue::Utf8(Some("#4682b4".to_string())),
        );
        arc_defaults.insert(
            "stroke".to_string(),
            ScalarValue::Utf8(Some("#000000".to_string())),
        );
        arc_defaults.insert("stroke_width".to_string(), ScalarValue::Float32(Some(1.0)));
        arc_defaults.insert("opacity".to_string(), ScalarValue::Float32(Some(1.0)));
        arc_defaults.insert("pad_angle".to_string(), ScalarValue::Float32(Some(0.0)));
        arc_defaults.insert("corner_radius".to_string(), ScalarValue::Float32(Some(0.0)));
        defaults.insert("arc".to_string(), arc_defaults);

        // Text mark defaults
        let mut text_defaults = IndexMap::new();
        text_defaults.insert(
            "fill".to_string(),
            ScalarValue::Utf8(Some("#000000".to_string())),
        );
        text_defaults.insert(
            "font".to_string(),
            ScalarValue::Utf8(Some("Atkinson Hyperlegible Next".to_string())),
        );
        text_defaults.insert("font_size".to_string(), ScalarValue::Float32(Some(12.0)));
        text_defaults.insert("font_weight".to_string(), ScalarValue::Float32(Some(400.0)));
        text_defaults.insert(
            "align".to_string(),
            ScalarValue::Utf8(Some("center".to_string())),
        );
        text_defaults.insert(
            "baseline".to_string(),
            ScalarValue::Utf8(Some("middle".to_string())),
        );
        defaults.insert("text".to_string(), text_defaults);

        Self { defaults }
    }
}
