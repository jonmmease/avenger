//! Preset themes for common visualization styles

use super::*;
use datafusion_common::ScalarValue;
use indexmap::IndexMap;

impl Theme {
    /// Dark theme optimized for dark backgrounds
    pub fn dark() -> Self {
        let mut theme = Theme::default();

        // Dark background colors
        theme.background = BackgroundDefaults {
            plot_background: Some("#1E1E1E".to_string()), // Dark plot area
            canvas_background: Some("#121212".to_string()), // Darker canvas background
        };

        // Keep default font
        theme.typography.default_font = theme.typography.default_font.clone();

        // Update text colors for dark mode
        theme.typography.title_color = "#FFFFFF".to_string(); // Bright white for titles
        theme.typography.subtitle_color = "#FFFFFF".to_string(); // Bright white for subtitles
        theme.typography.legend_title_color = "#FFFFFF".to_string(); // White for legend titles
        theme.typography.legend_label_color = "#E1E6EA".to_string(); // Light gray for legend labels (matching axis tick labels)

        // Okabe-Ito colorblind-safe palette (optimized for dark backgrounds)
        theme.colors = ColorPalettes {
            categorical: vec![
                "#56B4E9".to_string(), // Sky blue
                "#E69F00".to_string(), // Orange
                "#009E73".to_string(), // Green
                "#F0E442".to_string(), // Yellow
                "#0072B2".to_string(), // Blue
                "#D55E00".to_string(), // Vermillion
                "#CC79A7".to_string(), // Reddish purple
                "#999999".to_string(), // Grey
            ],
            default_color: "#56B4E9".to_string(),
            ..ColorPalettes::default()
        };

        theme.axis.grid_color = "#30363D".to_string();
        theme.axis.grid_opacity = 0.5;

        theme.axis.domain_color = "#FFFFFF".to_string();
        theme.axis.tick_color = "#FFFFFF".to_string();

        theme.axis.label_color = "#AAAAAA".to_string();
        theme.axis.title_color = "#FFFFFF".to_string();

        // Update legend defaults for dark mode
        theme.legend.background_fill = None; // No background by default
        theme.legend.background_stroke = None; // No stroke outline by default

        // Dark theme mark defaults
        let mut mark_defaults = IndexMap::new();

        // Symbol marks
        let mut symbol = IndexMap::new();
        symbol.insert(
            "fill".to_string(),
            ScalarValue::Utf8(Some("#56B4E9".to_string())), // Sky blue from Okabe-Ito
        );
        symbol.insert(
            "stroke".to_string(),
            ScalarValue::Utf8(Some("#30363D".to_string())), // Subtle dark stroke
        );
        symbol.insert(
            "stroke_width".to_string(),
            ScalarValue::Float64(Some(0.0)), // No stroke by default
        );
        mark_defaults.insert("symbol".to_string(), symbol);

        // Rect marks
        let mut rect = IndexMap::new();
        rect.insert(
            "fill".to_string(),
            ScalarValue::Utf8(Some("#56B4E9".to_string())), // Sky blue from Okabe-Ito
        );
        rect.insert(
            "stroke".to_string(),
            ScalarValue::Utf8(Some("#30363D".to_string())), // Subtle dark stroke
        );
        rect.insert(
            "stroke_width".to_string(),
            ScalarValue::Float64(Some(0.0)), // No stroke by default
        );
        mark_defaults.insert("rect".to_string(), rect);

        // Text marks
        let mut text = IndexMap::new();
        text.insert(
            "fill".to_string(),
            ScalarValue::Utf8(Some("#C9D1D9".to_string())), // Light gray text
        );
        mark_defaults.insert("text".to_string(), text);

        // Line marks - copy all defaults from default theme then override colors
        let mut line = theme
            .mark_defaults
            .defaults
            .get("line")
            .cloned()
            .unwrap_or_else(IndexMap::new);
        line.insert(
            "stroke".to_string(),
            ScalarValue::Utf8(Some("#56B4E9".to_string())), // Sky blue from Okabe-Ito
        );
        mark_defaults.insert("line".to_string(), line);

        theme.mark_defaults = MarkDefaults {
            defaults: mark_defaults,
        };

        theme
    }

    /// Light theme (equivalent to default)
    pub fn light() -> Self {
        Self::default()
    }

    /// High contrast theme for accessibility
    pub fn high_contrast() -> Self {
        let mut theme = Theme::default();

        // High contrast colors
        theme.colors = ColorPalettes {
            categorical: vec![
                "#000000".to_string(), // Black
                "#FF0000".to_string(), // Red
                "#00FF00".to_string(), // Green
                "#0000FF".to_string(), // Blue
                "#FFFF00".to_string(), // Yellow
                "#FF00FF".to_string(), // Magenta
                "#00FFFF".to_string(), // Cyan
                "#808080".to_string(), // Gray
            ],
            default_color: "#000000".to_string(),
            ..ColorPalettes::default()
        };

        // Bold fonts
        theme.typography.default_weight = FontWeight::Named(NamedFontWeight::Bold);
        theme.typography.axis_label_weight = FontWeight::Named(NamedFontWeight::SemiBold);

        // Thicker lines
        let mut mark_defaults = IndexMap::new();

        let mut line = IndexMap::new();
        line.insert("stroke_width".to_string(), ScalarValue::Float32(Some(3.0)));
        mark_defaults.insert("line".to_string(), line);

        let mut symbol = IndexMap::new();
        symbol.insert("stroke_width".to_string(), ScalarValue::Float32(Some(2.0)));
        symbol.insert("size".to_string(), ScalarValue::Float32(Some(100.0)));
        mark_defaults.insert("symbol".to_string(), symbol);

        theme.mark_defaults = MarkDefaults {
            defaults: mark_defaults,
        };

        // High contrast axis
        theme.axis = AxisDefaults {
            grid_color: "#000000".to_string(),
            grid_width: 2.0,
            domain_color: "#000000".to_string(),
            domain_width: 2.0,
            tick_color: "#000000".to_string(),
            ..AxisDefaults::default()
        };

        theme
    }

    /// Colorblind-safe theme using Okabe-Ito palette
    pub fn colorblind_safe() -> Self {
        let mut theme = Theme::default();

        // Okabe-Ito colorblind-safe palette
        theme.colors = ColorPalettes {
            categorical: vec![
                "#0072B2".to_string(), // Blue
                "#E69F00".to_string(), // Orange
                "#009E73".to_string(), // Green
                "#F0E442".to_string(), // Yellow
                "#56B4E9".to_string(), // Sky blue
                "#D55E00".to_string(), // Vermillion
                "#CC79A7".to_string(), // Reddish purple
                "#999999".to_string(), // Grey
            ],
            default_color: "#0072B2".to_string(),
            ..ColorPalettes::default()
        };

        theme
    }

    /// Publication-ready theme with minimal styling
    pub fn publication() -> Self {
        let mut theme = Theme::default();

        // Conservative font choices
        theme.typography = Typography {
            default_font: "Helvetica".to_string(),
            title_font: Some("Helvetica".to_string()),
            title_size: 14.0,
            subtitle_size: 12.0,
            axis_label_size: 10.0,
            axis_tick_size: 9.0,
            legend_title_size: 10.0,
            legend_item_size: 9.0,
            ..Typography::default()
        };

        // Grayscale palette
        theme.colors = ColorPalettes {
            categorical: vec![
                "#000000".to_string(),
                "#404040".to_string(),
                "#808080".to_string(),
                "#BFBFBF".to_string(),
                "#E0E0E0".to_string(),
            ],
            default_color: "#000000".to_string(),
            ..ColorPalettes::default()
        };

        // Minimal styling
        theme.axis = AxisDefaults {
            grid_color: "#CCCCCC".to_string(),
            grid_width: 0.5,
            domain_color: "#000000".to_string(),
            domain_width: 1.0,
            tick_color: "#000000".to_string(),
            ..AxisDefaults::default()
        };

        // No legend background
        theme.legend = LegendDefaults {
            background_fill: None,
            background_stroke: None,
            ..LegendDefaults::default()
        };

        theme
    }
}
