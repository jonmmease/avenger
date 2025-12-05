//! Shared utilities for facet guide rendering
//!
//! This module provides common measurement and rendering functions used by all facet guide types
//! (FacetRowGuide and FacetColGuide) to eliminate code duplication and ensure
//! consistent behavior across faceting dimensions.

use crate::facet::band_positions::BandPosition;
use crate::layout::LayoutBounds;
use crate::theme::Theme;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rule::SceneRuleMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_text::measurement::{TextMeasurementConfig, TextMeasurer};
use avenger_text::types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline};
use datafusion::common::ScalarValue;
use indexmap::IndexMap;
use std::sync::Arc as StdArc;

/// Configuration for measuring facet label slab space requirements
///
/// This struct contains all information needed to measure the space required for
/// facet labels (with optional title and rule). Split from rendering config to
/// provide clear separation of concerns.
#[derive(Clone, Debug)]
pub struct FacetLabelMeasurementConfig {
    /// The labels to measure
    pub labels: Vec<String>,
    /// Whether labels are rotated 90 degrees (true for row facets, false for col facets)
    pub is_rotated: bool,
    /// Font family for labels
    pub font_family: String,
    /// Font size in pixels for labels
    pub font_size_px: f32,
    /// Optional title text
    pub title: Option<String>,
    /// Font family for title
    pub title_font_family: String,
    /// Font size in pixels for title
    pub title_font_size_px: f32,
    /// Whether to render/measure the title (false when nested and title should be edge-only)
    pub render_title: bool,
}

/// Configuration for rendering facet label slab
///
/// This struct contains all information needed to render facet labels, rule, and title.
/// Extends measurement config with positioning information.
#[derive(Clone, Debug)]
pub struct FacetLabelRenderConfig {
    /// The labels to render
    pub labels: Vec<String>,
    /// Band positions for centering labels
    pub band_positions: Vec<BandPosition>,
    /// Plot bounds for positioning
    pub plot_bounds: LayoutBounds,
    /// Whether labels are rotated 90 degrees (true for row facets, false for col facets)
    pub is_rotated: bool,
    /// Whether to place labels at the far edge (right for row, bottom for col)
    pub place_at_end: bool,
    /// Font family for labels
    pub font_family: String,
    /// Font size in pixels for labels
    pub font_size_px: f32,
    /// Optional title text
    pub title: Option<String>,
    /// Font family for title
    pub title_font_family: String,
    /// Font size in pixels for title
    pub title_font_size_px: f32,
    /// Whether to render the title (false when nested and title should be edge-only)
    pub render_title: bool,
}

/// Measure space required for facet label slab (labels + optional title + rule + ticks)
///
/// Returns the perpendicular dimension needed (width for row facets, height for col facets).
/// For rotated labels (row facets), this measures the horizontal footprint.
/// For horizontal labels (col facets), this measures the vertical footprint.
pub fn measure_facet_label_slab(config: &FacetLabelMeasurementConfig) -> f32 {
    let measurer = avenger_text::measurement::default_text_measurer();

    // Measure label dimensions
    let mut max_label_dimension = 0.0_f32;
    for label in &config.labels {
        let text_config = TextMeasurementConfig {
            text: label,
            font: &config.font_family,
            font_size: config.font_size_px,
            font_weight: &FontWeight::Name(FontWeightNameSpec::Normal),
            font_style: &FontStyle::Normal,
        };
        let bounds = measurer.measure_text_bounds(&text_config);

        // For rotated (row) labels: text height becomes horizontal footprint after rotation
        // For horizontal (col) labels: use text height directly
        max_label_dimension = max_label_dimension.max(if config.is_rotated {
            bounds.height
        } else {
            bounds.height
        });
    }

    // Start with label space
    let mut total_space = max_label_dimension;

    // Add space for rule + ticks ONLY when there are multiple labels AND no title
    // When title is present, the gap between labels and title already includes space for rule/ticks
    if config.labels.len() > 1 && !config.render_title {
        // Space from labels to rule (gap/2) + rule stroke + tick size
        let gap = 10.0_f32;
        let rule_stroke = 1.0_f32;
        let tick_size = 4.0_f32;
        total_space += gap / 2.0 + rule_stroke + tick_size;
    }

    // Measure title if present AND render_title is true
    // When nested, render_title=false so we don't include title space
    if config.render_title {
        if let Some(title_text) = &config.title {
            let title_config = TextMeasurementConfig {
                text: title_text,
                font: &config.title_font_family,
                font_size: config.title_font_size_px,
                font_weight: &FontWeight::Name(FontWeightNameSpec::Normal),
                font_style: &FontStyle::Normal,
            };
            let title_bounds = measurer.measure_text_bounds(&title_config);
            let title_dimension = if config.is_rotated {
                title_bounds.height
            } else {
                title_bounds.height
            };

            // Add gap + title + rule stroke (original logic - gap includes rule/tick space)
            let gap = 10.0_f32;
            total_space += gap + title_dimension + 1.0;
        }
    }

    total_space + 1.0 // Extra pixel for safety
}

/// Render facet label slab with labels, optional horizontal rule with ticks, and optional title
///
/// This function produces all scene marks needed for a complete facet label slab.
/// It handles both row facets (vertical, rotated labels) and column facets (horizontal labels).
pub fn render_facet_label_slab(
    config: &FacetLabelRenderConfig,
    theme: &Theme,
    theme_params: &IndexMap<String, ScalarValue>,
) -> Vec<SceneMark> {
    let mut marks = Vec::new();

    // Get font properties from theme
    let label_ctx = crate::theme::ThemeContext::new("guide", theme_params.clone())
        .child("facet")
        .child("label");

    // Calculate label positions
    let (x_positions, y_positions) = if config.is_rotated {
        // Row labels (vertical, on left or right side)
        calculate_row_label_positions(config, theme, theme_params)
    } else {
        // Col labels (horizontal, on top or bottom)
        calculate_col_label_positions(config)
    };

    // Render labels
    for (i, label) in config.labels.iter().enumerate() {
        let band_pos = &config.band_positions[i];
        let label_mark = create_label_mark(
            label,
            band_pos,
            config,
            x_positions[i],
            y_positions[i],
            theme,
            &label_ctx,
        );
        marks.push(SceneMark::Text(StdArc::new(label_mark)));
    }

    // Render rule with ticks when there are multiple labels
    // This provides visual structure connecting labels together, independent of title visibility
    if config.labels.len() > 1 {
        marks.extend(render_rule_with_ticks(config, theme, theme_params));
    }

    // Render title if present AND render_title is true
    // When nested, render_title=false so title appears only at outer edge
    if config.render_title {
        if let Some(title_text) = &config.title {
            let title_mark = render_facet_title(title_text, config, theme, theme_params);
            marks.push(SceneMark::Text(StdArc::new(title_mark)));
        }
    }

    marks
}

/// Calculate positions for row labels (vertical, rotated)
///
/// Returns (x_positions, y_positions) vectors with one entry per label.
/// For row labels, x is constant (left or right edge), y varies by band position.
fn calculate_row_label_positions(
    config: &FacetLabelRenderConfig,
    _theme: &Theme,
    _theme_params: &IndexMap<String, ScalarValue>,
) -> (Vec<f32>, Vec<f32>) {
    let measurer = avenger_text::measurement::default_text_measurer();

    let mut x_positions = Vec::new();
    let mut y_positions = Vec::new();

    for (i, label) in config.labels.iter().enumerate() {
        let band_pos = &config.band_positions[i];
        let y_center = config.plot_bounds.y + band_pos.center();

        // Measure this label to position its center
        let text_config = TextMeasurementConfig {
            text: label,
            font: &config.font_family,
            font_size: config.font_size_px,
            font_weight: &FontWeight::Name(FontWeightNameSpec::Normal),
            font_style: &FontStyle::Normal,
        };
        let bounds = measurer.measure_text_bounds(&text_config);

        // With rotation, horizontal span ≈ bounds.height; side decides x and angle
        let x_center = if config.place_at_end {
            config.plot_bounds.x + config.plot_bounds.width + 0.5 * bounds.height
        } else {
            config.plot_bounds.x - 0.5 * bounds.height
        };

        x_positions.push(x_center);
        y_positions.push(y_center);
    }

    (x_positions, y_positions)
}

/// Calculate positions for col labels (horizontal)
///
/// Returns (x_positions, y_positions) vectors with one entry per label.
/// For col labels, x varies by band position, y is constant (top or bottom edge).
fn calculate_col_label_positions(config: &FacetLabelRenderConfig) -> (Vec<f32>, Vec<f32>) {
    let mut x_positions = Vec::new();
    let mut y_positions = Vec::new();

    // Y position constant for all labels
    let y_label = if config.place_at_end {
        config.plot_bounds.y + config.plot_bounds.height // Start at bottom edge
    } else {
        config.plot_bounds.y // Start at top edge
    };

    for band_pos in &config.band_positions {
        let x_center = config.plot_bounds.x + band_pos.center();
        x_positions.push(x_center);
        y_positions.push(y_label);
    }

    (x_positions, y_positions)
}

/// Create a single label mark
fn create_label_mark(
    label: &str,
    _band_pos: &BandPosition,
    config: &FacetLabelRenderConfig,
    x: f32,
    y: f32,
    theme: &Theme,
    label_ctx: &crate::theme::ThemeContext,
) -> SceneTextMark {
    let angle = if config.is_rotated {
        if config.place_at_end {
            90.0_f32 // Right side: rotate clockwise
        } else {
            -90.0_f32 // Left side: rotate counter-clockwise
        }
    } else {
        0.0_f32 // Horizontal
    };

    let baseline = if config.is_rotated {
        TextBaseline::Middle
    } else if config.place_at_end {
        TextBaseline::Top
    } else {
        TextBaseline::Bottom
    };

    SceneTextMark {
        text: label.to_string().into(),
        x: x.into(),
        y: y.into(),
        align: TextAlign::Center.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        font: config.font_family.clone().into(),
        font_size: config.font_size_px.into(),
        color: avenger_common::types::ColorOrGradient::Color(
            theme.text_color(label_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]),
        )
        .into(),
        zindex: Some(5),
        ..Default::default()
    }
}

/// Render rule with ticks between labels and title
fn render_rule_with_ticks(
    config: &FacetLabelRenderConfig,
    theme: &Theme,
    theme_params: &IndexMap<String, ScalarValue>,
) -> Vec<SceneMark> {
    let mut marks = Vec::new();

    if config.labels.is_empty() || config.labels.len() <= 1 {
        return marks;
    }

    let measurer = avenger_text::measurement::default_text_measurer();

    // Measure label dimensions to position rule
    let mut max_label_dimension = 0.0_f32;
    for label in &config.labels {
        let text_config = TextMeasurementConfig {
            text: label,
            font: &config.font_family,
            font_size: config.font_size_px,
            font_weight: &FontWeight::Name(FontWeightNameSpec::Normal),
            font_style: &FontStyle::Normal,
        };
        let bounds = measurer.measure_text_bounds(&text_config);
        max_label_dimension = max_label_dimension.max(if config.is_rotated {
            bounds.height
        } else {
            bounds.height
        });
    }

    // Query rule styling from theme
    let rule_ctx = crate::theme::ThemeContext::new("guide", theme_params.clone())
        .child("facet")
        .child("rule");
    let mut rule_stroke = theme.text_color(&rule_ctx).unwrap_or([0.5, 0.5, 0.5, 1.0]);
    rule_stroke[3] = 1.0; // Ensure fully opaque
    let rule_stroke_width = theme
        .query(&rule_ctx, "stroke-width")
        .and_then(|v| v.as_number())
        .map(|n| n as f32)
        .unwrap_or(1.0);
    let tick_size = theme
        .query(&rule_ctx, "tick-size")
        .and_then(|v| v.as_number())
        .map(|n| n as f32)
        .unwrap_or(4.0);

    let gap = 10.0_f32;
    let half_stroke = rule_stroke_width / 2.0;

    if config.is_rotated {
        // Vertical rule for row labels
        let y_top = config.plot_bounds.y
            + config
                .band_positions
                .first()
                .map(|bp| bp.center())
                .unwrap_or(0.0);
        let y_bottom = config.plot_bounds.y
            + config
                .band_positions
                .last()
                .map(|bp| bp.center())
                .unwrap_or(0.0);

        let x_rule = if config.place_at_end {
            config.plot_bounds.x + config.plot_bounds.width + max_label_dimension + gap / 2.0
        } else {
            config.plot_bounds.x - (max_label_dimension + gap / 2.0)
        };

        // Create vertical rule
        let rule_mark = SceneRuleMark {
            x: x_rule.into(),
            y: (y_top - half_stroke).into(),
            x2: x_rule.into(),
            y2: (y_bottom + half_stroke).into(),
            stroke: avenger_common::types::ColorOrGradient::Color(rule_stroke).into(),
            stroke_width: rule_stroke_width.into(),
            zindex: Some(5),
            ..Default::default()
        };
        marks.push(SceneMark::Rule(rule_mark));

        // Add tick marks at each label position
        for band_pos in &config.band_positions {
            let y_center = config.plot_bounds.y + band_pos.center();

            let (x_tick_start, x_tick_end) = if config.place_at_end {
                (x_rule - tick_size, x_rule)
            } else {
                (x_rule, x_rule + tick_size)
            };

            let tick_mark = SceneRuleMark {
                x: x_tick_start.into(),
                y: y_center.into(),
                x2: x_tick_end.into(),
                y2: y_center.into(),
                stroke: avenger_common::types::ColorOrGradient::Color(rule_stroke).into(),
                stroke_width: rule_stroke_width.into(),
                zindex: Some(5),
                ..Default::default()
            };
            marks.push(SceneMark::Rule(tick_mark));
        }
    } else {
        // Horizontal rule for col labels
        let x_left = config.plot_bounds.x
            + config
                .band_positions
                .first()
                .map(|bp| bp.center())
                .unwrap_or(0.0);
        let x_right = config.plot_bounds.x
            + config
                .band_positions
                .last()
                .map(|bp| bp.center())
                .unwrap_or(0.0);

        let y_label = if config.place_at_end {
            config.plot_bounds.y + config.plot_bounds.height
        } else {
            config.plot_bounds.y
        };

        let y_rule = if config.place_at_end {
            y_label + max_label_dimension + gap / 2.0
        } else {
            y_label - max_label_dimension - gap / 2.0
        };

        // Create horizontal rule
        let rule_mark = SceneRuleMark {
            x: (x_left - half_stroke).into(),
            y: y_rule.into(),
            x2: (x_right + half_stroke).into(),
            y2: y_rule.into(),
            stroke: avenger_common::types::ColorOrGradient::Color(rule_stroke).into(),
            stroke_width: rule_stroke_width.into(),
            zindex: Some(5),
            ..Default::default()
        };
        marks.push(SceneMark::Rule(rule_mark));

        // Add tick marks at each label position
        for band_pos in &config.band_positions {
            let x_center = config.plot_bounds.x + band_pos.center();

            let (y_tick_start, y_tick_end) = if config.place_at_end {
                (y_rule - tick_size, y_rule)
            } else {
                (y_rule, y_rule + tick_size)
            };

            let tick_mark = SceneRuleMark {
                x: x_center.into(),
                y: y_tick_start.into(),
                x2: x_center.into(),
                y2: y_tick_end.into(),
                stroke: avenger_common::types::ColorOrGradient::Color(rule_stroke).into(),
                stroke_width: rule_stroke_width.into(),
                zindex: Some(5),
                ..Default::default()
            };
            marks.push(SceneMark::Rule(tick_mark));
        }
    }

    marks
}

/// Render facet title
fn render_facet_title(
    title_text: &str,
    config: &FacetLabelRenderConfig,
    theme: &Theme,
    theme_params: &IndexMap<String, ScalarValue>,
) -> SceneTextMark {
    let measurer = avenger_text::measurement::default_text_measurer();
    let title_ctx = crate::theme::ThemeContext::new("guide", theme_params.clone())
        .child("facet")
        .child("title");

    let title_config = TextMeasurementConfig {
        text: title_text,
        font: &config.title_font_family,
        font_size: config.title_font_size_px,
        font_weight: &FontWeight::Name(FontWeightNameSpec::Normal),
        font_style: &FontStyle::Normal,
    };
    let title_bounds = measurer.measure_text_bounds(&title_config);

    let gap = 10.0_f32;

    // Measure label dimensions for positioning
    let mut max_label_dimension = 0.0_f32;
    for label in &config.labels {
        let text_config = TextMeasurementConfig {
            text: label,
            font: &config.font_family,
            font_size: config.font_size_px,
            font_weight: &FontWeight::Name(FontWeightNameSpec::Normal),
            font_style: &FontStyle::Normal,
        };
        let bounds = measurer.measure_text_bounds(&text_config);
        max_label_dimension = max_label_dimension.max(if config.is_rotated {
            bounds.height
        } else {
            bounds.height
        });
    }

    let (x, y, angle, baseline) = if config.is_rotated {
        // Row title (vertical, rotated)
        let x_center = if config.place_at_end {
            config.plot_bounds.x
                + config.plot_bounds.width
                + max_label_dimension
                + gap
                + 0.5 * title_bounds.height
        } else {
            config.plot_bounds.x - (max_label_dimension + gap + 0.5 * title_bounds.height)
        };
        let y_center = config.plot_bounds.y + 0.5 * config.plot_bounds.height;
        let angle = if config.place_at_end {
            90.0_f32
        } else {
            -90.0_f32
        };
        (x_center, y_center, angle, TextBaseline::Middle)
    } else {
        // Col title (horizontal)
        let x_center = config.plot_bounds.x + 0.5 * config.plot_bounds.width;
        let y_title = if config.place_at_end {
            config.plot_bounds.y + config.plot_bounds.height + max_label_dimension + gap
        } else {
            config.plot_bounds.y - max_label_dimension - gap
        };
        let baseline = if config.place_at_end {
            TextBaseline::Top
        } else {
            TextBaseline::Bottom
        };
        (x_center, y_title, 0.0_f32, baseline)
    };

    SceneTextMark {
        text: title_text.to_string().into(),
        x: x.into(),
        y: y.into(),
        align: TextAlign::Center.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        font: config.title_font_family.clone().into(),
        font_size: config.title_font_size_px.into(),
        color: avenger_common::types::ColorOrGradient::Color(
            theme.text_color(&title_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]),
        )
        .into(),
        zindex: Some(6),
        ..Default::default()
    }
}
