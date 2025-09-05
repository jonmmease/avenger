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
            plot_background: Some("#1E1E1E".to_string()),
            canvas_background: Some("#121212".to_string()),
        };

        // Light text on dark background
        theme.typography.default_font = "Inter".to_string();

        // Adjusted color palette for dark backgrounds
        theme.colors = ColorPalettes {
            categorical: vec![
                "#64B5F6".to_string(), // Light Blue
                "#FFB74D".to_string(), // Light Orange
                "#81C784".to_string(), // Light Green
                "#FFE082".to_string(), // Light Yellow
                "#E57373".to_string(), // Light Red
                "#9575CD".to_string(), // Light Purple
                "#4DB6AC".to_string(), // Light Teal
                "#90A4AE".to_string(), // Blue Grey
            ],
            default_color: "#64B5F6".to_string(),
            ..ColorPalettes::default()
        };

        // Dark theme mark defaults
        let mut mark_defaults = IndexMap::new();

        // Symbol marks
        let mut symbol = IndexMap::new();
        symbol.insert(
            "fill".to_string(),
            ScalarValue::Utf8(Some("#64B5F6".to_string())),
        );
        symbol.insert(
            "stroke".to_string(),
            ScalarValue::Utf8(Some("#E0E0E0".to_string())),
        );
        mark_defaults.insert("symbol".to_string(), symbol);

        // Rect marks
        let mut rect = IndexMap::new();
        rect.insert(
            "fill".to_string(),
            ScalarValue::Utf8(Some("#64B5F6".to_string())),
        );
        rect.insert(
            "stroke".to_string(),
            ScalarValue::Utf8(Some("#424242".to_string())),
        );
        mark_defaults.insert("rect".to_string(), rect);

        // Text marks
        let mut text = IndexMap::new();
        text.insert(
            "fill".to_string(),
            ScalarValue::Utf8(Some("#E0E0E0".to_string())),
        );
        mark_defaults.insert("text".to_string(), text);

        // Line marks
        let mut line = IndexMap::new();
        line.insert(
            "stroke".to_string(),
            ScalarValue::Utf8(Some("#64B5F6".to_string())),
        );
        mark_defaults.insert("line".to_string(), line);

        theme.mark_defaults = MarkDefaults {
            defaults: mark_defaults,
        };

        // Axis styling for dark theme
        theme.axis = AxisDefaults {
            grid_color: "#424242".to_string(),
            domain_color: "#E0E0E0".to_string(),
            tick_color: "#E0E0E0".to_string(),
            ..AxisDefaults::default()
        };

        // Legend styling for dark theme
        theme.legend = LegendDefaults {
            background_fill: Some("#2A2A2A".to_string()),
            background_stroke: Some("#424242".to_string()),
            ..LegendDefaults::default()
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

    /// Colorblind-safe theme using optimized palettes
    pub fn colorblind_safe() -> Self {
        let mut theme = Theme::default();

        // Paul Tol's colorblind-safe palette
        theme.colors = ColorPalettes {
            categorical: vec![
                "#332288".to_string(), // Indigo
                "#88CCEE".to_string(), // Cyan
                "#44AA99".to_string(), // Teal
                "#117733".to_string(), // Green
                "#999933".to_string(), // Olive
                "#DDCC77".to_string(), // Sand
                "#CC6677".to_string(), // Rose
                "#882255".to_string(), // Wine
                "#AA4499".to_string(), // Purple
            ],
            default_color: "#332288".to_string(),
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
