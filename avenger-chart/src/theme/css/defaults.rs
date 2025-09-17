//! Default CSS themes

use crate::theme::css::simple_parser::parse_css_simple;
use crate::theme::css::types::{Theme, ThemeError};

impl Theme {
    /// Default light theme
    pub fn default() -> Result<Theme, ThemeError> {
        Self::light()
    }

    /// Light theme (default)
    pub fn light() -> Result<Theme, ThemeError> {
        let css = r#"
/* Root variables for parameterization */
:root {
    /* Core variables */
    --base-font-size: 12px;
    --scale: 1;

    /* Okabe-Ito colorblind-safe palette */
    --color-1: #0072B2; /* Blue */
    --color-2: #E69F00; /* Orange */
    --color-3: #009E73; /* Bluish Green */
    --color-4: #F0E442; /* Yellow */
    --color-5: #D55E00; /* Vermillion */
    --color-6: #56B4E9; /* Sky Blue */
    --color-7: #CC79A7; /* Reddish Purple */
    --color-8: #999999; /* Grey */

    /* Primary colors */
    --primary: var(--color-1);
    --default-color: #4682b4; /* Steel blue */

    /* Background colors */
    --background: transparent;
    --canvas-background: transparent;
    --plot-background: transparent;

    /* Text colors */
    --text-primary: #1a1a1a;
    --text-secondary: #2a2a2a;
    --text-subtitle: #4a4a4a;
    --text-muted: #5a5a5a;
    --text-label: #3C3C3C;

    /* Grid and axis colors */
    --grid-color: #e0e0e0;
    --domain-color: #000;
    --tick-color: #000;

    /* Typography - using system fonts with Atkinson Hyperlegible Next as default */
    --font-family: "Atkinson Hyperlegible Next", system-ui, -apple-system, sans-serif;
    --font-weight-light: 200;
    --font-weight-normal: 300;
    --font-weight-medium: 400;
    --font-weight-semibold: 500;

    /* Font scale multipliers */
    --title-scale: 1.5;      /* 18px @ 12px base */
    --subtitle-scale: 1.167;  /* 14px @ 12px base */
    --axis-title-scale: 1.0;  /* 12px @ 12px base */
    --legend-title-scale: 1.0;/* 12px @ 12px base */
    --legend-label-scale: 0.917; /* 11px @ 12px base */
    --axis-label-scale: 0.833;   /* 10px @ 12px base */
    --legend-tick-scale: 0.833;  /* 10px @ 12px base */
    --text-mark-scale: 1.0;      /* 12px @ 12px base */

    /* Spacing */
    --padding: 5px;
    --legend-spacing: 10px;
    --label-padding: 3px;

    /* Strokes and sizing */
    --stroke-width: 1px;
    --grid-opacity: 0.5;
    --grid-width: 0.5px;
    --tick-length: 5px;
}

/* Global defaults */
* {
    font-family: var(--font-family);
    font-size: var(--base-font-size);
}

/* Canvas background */
canvas {
    background-color: var(--canvas-background);
}

/* Title styles */
title {
    font-size: calc(var(--base-font-size) * var(--title-scale));
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
}

subtitle {
    font-size: calc(var(--base-font-size) * var(--subtitle-scale));
    font-weight: var(--font-weight-light);
    color: var(--text-subtitle);
}

/* Axis styles */
axis {
    grid-color: var(--grid-color);
    grid-opacity: var(--grid-opacity);
    grid-width: var(--grid-width);
    domain-color: var(--domain-color);
    domain-width: 1px;
    tick-color: var(--tick-color);
    tick-size: 5px;
    tick-length: var(--tick-length);
    label-padding: var(--label-padding);
    label-angle: 0;
}

axis.title {
    font-size: calc(var(--base-font-size) * var(--axis-title-scale));
    font-weight: var(--font-weight-medium);
    color: var(--text-secondary);
}

axis.label {
    font-size: calc(var(--base-font-size) * var(--axis-label-scale));
    font-weight: var(--font-weight-normal);
    color: var(--text-muted);
}

axis.tick {
    color: var(--tick-color);
}

/* Legend styles */
legend {
    background-fill: none;
    background-stroke: none;
    background-padding: 8px;
    background-corner-radius: 0;
    item-spacing: 10px;
    symbol-size: 100px;
    label-padding: 5px;
}

legend.title {
    font-size: calc(var(--base-font-size) * var(--legend-title-scale));
    font-weight: var(--font-weight-medium);
    color: #2C2C2C;
}

legend.label {
    font-size: calc(var(--base-font-size) * var(--legend-label-scale));
    font-weight: var(--font-weight-normal);
    color: var(--text-label);
}

legend.tick {
    font-size: calc(var(--base-font-size) * var(--legend-tick-scale));
    font-weight: var(--font-weight-normal);
    color: var(--text-muted);
}

/* Mark defaults */
mark {
    fill: var(--default-color);
    stroke: #000;
    stroke-width: var(--stroke-width);
    opacity: 1.0;
}

mark.symbol {
    fill: var(--default-color);
    stroke: #000;
    stroke-width: 1px;
    size: 50px;
}

mark.rect {
    fill: var(--default-color);
    stroke: none;
    stroke-width: 0;
}

mark.line {
    stroke: var(--default-color);
    stroke-width: 2px;
    fill: none;
}

mark.area {
    fill: var(--default-color);
    stroke: none;
    opacity: 0.7;
}

mark.text {
    fill: #000;
    font-size: calc(var(--base-font-size) * var(--text-mark-scale));
    font-weight: var(--font-weight-normal);
}

mark.arc {
    fill: var(--default-color);
    stroke: white;
    stroke-width: 1px;
}

/* Layout */
layout {
    padding: var(--padding);
    padding-top: var(--padding);
    padding-right: var(--padding);
    padding-bottom: var(--padding);
    padding-left: var(--padding);
}
        "#;

        parse_css_simple(css)
    }

    /// Dark theme optimized for dark backgrounds
    pub fn dark() -> Result<Theme, ThemeError> {
        let css = r#"
/* Root variables for dark theme */
:root {
    /* Core variables */
    --base-font-size: 12px;
    --scale: 1;

    /* Okabe-Ito colorblind-safe palette (optimized for dark) */
    --color-1: #56B4E9; /* Sky blue */
    --color-2: #E69F00; /* Orange */
    --color-3: #009E73; /* Green */
    --color-4: #F0E442; /* Yellow */
    --color-5: #0072B2; /* Blue */
    --color-6: #D55E00; /* Vermillion */
    --color-7: #CC79A7; /* Reddish purple */
    --color-8: #999999; /* Grey */

    /* Primary colors */
    --primary: var(--color-1);
    --default-color: #56B4E9; /* Sky blue for dark theme */

    /* Background colors */
    --plot-background: #1E1E1E;
    --canvas-background: #121212;

    /* Text colors */
    --text-primary: #FFFFFF;
    --text-secondary: #FFFFFF;
    --text-subtitle: #FFFFFF;
    --text-muted: #AAAAAA;
    --text-label: #E1E6EA;
    --text-mark: #C9D1D9;

    /* Grid and axis colors */
    --grid-color: #30363D;
    --domain-color: #FFFFFF;
    --tick-color: #FFFFFF;

    /* Typography */
    --font-family: "Atkinson Hyperlegible Next", system-ui, -apple-system, sans-serif;
    --font-weight-light: 200;
    --font-weight-normal: 300;
    --font-weight-medium: 400;
    --font-weight-semibold: 500;

    /* Font scale multipliers (same as light theme) */
    --title-scale: 1.5;
    --subtitle-scale: 1.167;
    --axis-title-scale: 1.0;
    --legend-title-scale: 1.0;
    --legend-label-scale: 0.917;
    --axis-label-scale: 0.833;
    --legend-tick-scale: 0.833;
    --text-mark-scale: 1.0;

    /* Spacing (same as light theme) */
    --padding: 5px;
    --legend-spacing: 10px;
    --label-padding: 3px;

    /* Strokes and sizing */
    --stroke-color: #30363D;
    --stroke-width: 0px;
    --grid-opacity: 0.5;
    --grid-width: 0.5px;
    --tick-length: 5px;
}

/* Global defaults */
* {
    font-family: var(--font-family);
    font-size: var(--base-font-size);
}

/* Canvas background */
canvas {
    background-color: var(--canvas-background);
}

/* Title styles */
title {
    font-size: calc(var(--base-font-size) * var(--title-scale));
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
}

subtitle {
    font-size: calc(var(--base-font-size) * var(--subtitle-scale));
    font-weight: var(--font-weight-light);
    color: var(--text-subtitle);
}

/* Axis styles */
axis {
    grid-color: var(--grid-color);
    grid-opacity: var(--grid-opacity);
    grid-width: var(--grid-width);
    domain-color: var(--domain-color);
    domain-width: 1px;
    tick-color: var(--tick-color);
    tick-size: 5px;
    tick-length: var(--tick-length);
    label-padding: var(--label-padding);
    label-angle: 0;
}

axis.title {
    font-size: calc(var(--base-font-size) * var(--axis-title-scale));
    font-weight: var(--font-weight-medium);
    color: var(--text-primary);
}

axis.label {
    font-size: calc(var(--base-font-size) * var(--axis-label-scale));
    font-weight: var(--font-weight-normal);
    color: var(--text-muted);
}

axis.tick {
    color: var(--tick-color);
}

/* Legend styles */
legend {
    background-fill: none;
    background-stroke: none;
    background-padding: 8px;
    background-corner-radius: 0;
    item-spacing: 10px;
    symbol-size: 100px;
    label-padding: 5px;
}

legend.title {
    font-size: calc(var(--base-font-size) * var(--legend-title-scale));
    font-weight: var(--font-weight-medium);
    color: var(--text-primary);
}

legend.label {
    font-size: calc(var(--base-font-size) * var(--legend-label-scale));
    font-weight: var(--font-weight-normal);
    color: var(--text-label);
}

legend.tick {
    font-size: calc(var(--base-font-size) * var(--legend-tick-scale));
    font-weight: var(--font-weight-normal);
    color: var(--text-muted);
}

/* Mark defaults */
mark {
    fill: var(--default-color);
    stroke: var(--stroke-color);
    stroke-width: var(--stroke-width);
    opacity: 1.0;
}

mark.symbol {
    fill: var(--default-color);
    stroke: var(--stroke-color);
    stroke-width: 0px;
    size: 50px;
}

mark.rect {
    fill: var(--default-color);
    stroke: var(--stroke-color);
    stroke-width: 0px;
}

mark.line {
    stroke: var(--default-color);
    stroke-width: 2px;
    fill: none;
}

mark.area {
    fill: var(--default-color);
    stroke: none;
    opacity: 0.7;
}

mark.text {
    fill: var(--text-mark);
    font-size: calc(var(--base-font-size) * var(--text-mark-scale));
    font-weight: var(--font-weight-normal);
}

mark.arc {
    fill: var(--default-color);
    stroke: var(--stroke-color);
    stroke-width: 1px;
}

/* Layout */
layout {
    padding: var(--padding);
    padding-top: var(--padding);
    padding-right: var(--padding);
    padding-bottom: var(--padding);
    padding-left: var(--padding);
}
        "#;

        parse_css_simple(css)
    }

    /// Get categorical colors
    pub fn categorical_colors() -> Vec<String> {
        vec![
            "#0072B2".to_string(), // Blue
            "#E69F00".to_string(), // Orange
            "#009E73".to_string(), // Bluish Green
            "#F0E442".to_string(), // Yellow
            "#D55E00".to_string(), // Vermillion
            "#56B4E9".to_string(), // Sky Blue
            "#CC79A7".to_string(), // Reddish Purple
            "#999999".to_string(), // Grey
        ]
    }

    /// Get shape names for categorical encoding
    pub fn shape_names() -> Vec<&'static str> {
        vec![
            "circle",
            "cross",
            "diamond",
            "square",
            "star",
            "triangle-up",
            "wye",
            "cushion",
        ]
    }

    /// Get dash pattern names
    pub fn dash_patterns() -> Vec<&'static str> {
        vec![
            "solid",
            "dashed",
            "dotted",
            "long-dash",
            "dash-dot",
            "long-short",
            "even-short",
            "double-dash",
        ]
    }
}
