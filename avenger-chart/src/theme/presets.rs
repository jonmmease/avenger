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
            plot_background: None,                          // Transparent plot area
            canvas_background: Some("#0D1117".to_string()), // GitHub dark mode background
        };

        // Keep default font
        theme.typography.default_font = theme.typography.default_font.clone();

        // Update text colors for dark mode
        theme.typography.title_color = "#E6EDF3".to_string(); // Light gray for titles
        theme.typography.subtitle_color = "#C9D1D9".to_string(); // Medium-light gray for subtitles

        // Adjusted color palette for dark backgrounds - vibrant colors
        theme.colors = ColorPalettes {
            categorical: vec![
                "#4C9ED9".to_string(), // Bright blue
                "#70E99D".to_string(), // Mint green
                "#F79E54".to_string(), // Orange
                "#D96BBD".to_string(), // Pink/purple
                "#F2E965".to_string(), // Yellow
                "#9DD5D5".to_string(), // Cyan
                "#FF6B6B".to_string(), // Red
                "#B4A7D6".to_string(), // Lavender
                "#8FD14F".to_string(), // Lime green
                "#FFB3BA".to_string(), // Light pink
            ],
            default_color: "#4C9ED9".to_string(),
            ..ColorPalettes::default()
        };

        // Update axis defaults for dark mode
        theme.axis.grid_color = "#30363D".to_string(); // Subtle dark gray grid lines
        theme.axis.grid_opacity = 0.5; // Keep same opacity
        theme.axis.domain_color = "#484F58".to_string(); // Medium-dark gray for axis lines
        theme.axis.tick_color = "#484F58".to_string(); // Same as domain
        theme.axis.label_color = "#8B949E".to_string(); // Medium gray for axis labels
        theme.axis.title_color = "#C9D1D9".to_string(); // Light gray for axis titles

        // Update legend defaults for dark mode - keep mostly transparent
        theme.legend.background_fill = None; // No background by default
        theme.legend.background_stroke = Some("#30363D".to_string()); // Dark border if needed

        // Dark theme mark defaults
        let mut mark_defaults = IndexMap::new();

        // Symbol marks
        let mut symbol = IndexMap::new();
        symbol.insert(
            "fill".to_string(),
            ScalarValue::Utf8(Some("#4C9ED9".to_string())), // Use bright blue
        );
        symbol.insert(
            "stroke".to_string(),
            ScalarValue::Utf8(Some("#30363D".to_string())), // Subtle dark stroke
        );
        mark_defaults.insert("symbol".to_string(), symbol);

        // Rect marks
        let mut rect = IndexMap::new();
        rect.insert(
            "fill".to_string(),
            ScalarValue::Utf8(Some("#4C9ED9".to_string())), // Bright blue
        );
        rect.insert(
            "stroke".to_string(),
            ScalarValue::Utf8(Some("#30363D".to_string())), // Subtle dark stroke
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
            ScalarValue::Utf8(Some("#4C9ED9".to_string())), // Bright blue
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
