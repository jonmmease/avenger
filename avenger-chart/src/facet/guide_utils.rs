//! Shared utilities for facet guide rendering
//!
//! This module provides common measurement and rendering functions used by all facet guide types
//! (FacetRowGuide and FacetColGuide) to eliminate code duplication and ensure
//! consistent behavior across faceting dimensions.

use std::sync::Arc as StdArc;

use avenger_common::types::ColorOrGradient;
use avenger_scenegraph::marks::{mark::SceneMark, rule::SceneRuleMark, text::SceneTextMark};
use avenger_text::{
    measurement::{TextMeasurementConfig, TextMeasurer, default_text_measurer},
    types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline},
};
use datafusion::common::ScalarValue;
use indexmap::IndexMap;

use crate::{
    cartesian::axis::AxisPosition,
    facet::band_positions::BandPosition,
    facet::evaluated_facet_tree::EvaluatedFacetTree,
    layout::LayoutBounds,
    theme::{Theme, ThemeContext},
};

/// Format a ScalarValue for display as a facet label.
pub(crate) fn format_scalar_value(value: &ScalarValue) -> String {
    match value {
        ScalarValue::Utf8(Some(s))
        | ScalarValue::LargeUtf8(Some(s))
        | ScalarValue::Utf8View(Some(s)) => s.to_string(),
        ScalarValue::Int8(Some(n)) => n.to_string(),
        ScalarValue::Int16(Some(n)) => n.to_string(),
        ScalarValue::Int32(Some(n)) => n.to_string(),
        ScalarValue::Int64(Some(n)) => n.to_string(),
        ScalarValue::UInt8(Some(n)) => n.to_string(),
        ScalarValue::UInt16(Some(n)) => n.to_string(),
        ScalarValue::UInt32(Some(n)) => n.to_string(),
        ScalarValue::UInt64(Some(n)) => n.to_string(),
        ScalarValue::Float32(Some(n)) => format!("{n:.2}"),
        ScalarValue::Float64(Some(n)) => format!("{n:.2}"),
        ScalarValue::Boolean(Some(b)) => b.to_string(),
        _ => format!("{value:?}"),
    }
}

/// Determine whether facet guide labels should be visible for a specific facet cell.
///
/// This reuses channel-axis ownership logic so facet guide label ownership follows
/// sharing groups consistently with cartesian axis label ownership.
///
/// Invalid paths fall back to visible to preserve prior permissive behavior.
pub(crate) fn facet_guide_labels_visible_for_cell(
    facet_tree: &EvaluatedFacetTree,
    facet_path: &[ScalarValue],
    axis_position: AxisPosition,
    sharing_level: u8,
) -> bool {
    if facet_path.is_empty() {
        return true;
    }

    facet_tree
        .channel_axis_visibility_for_path_checked(facet_path, axis_position, sharing_level)
        .map(|visibility| visibility.show_labels)
        .unwrap_or(true)
}

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
    /// Optional absolute x-position override for column facet titles.
    ///
    /// When present, `render_facet_title` places horizontal titles at this x
    /// coordinate instead of deriving midpoint from visible band positions.
    pub col_title_x_override: Option<f32>,
}

/// Measure space required for facet label slab (labels + optional title + rule + ticks)
///
/// Returns the perpendicular dimension needed (width for row facets, height for col facets).
/// For rotated labels (row facets), this measures the horizontal footprint.
/// For horizontal labels (col facets), this measures the vertical footprint.
pub fn measure_facet_label_slab(config: &FacetLabelMeasurementConfig) -> f32 {
    let measurer = default_text_measurer();

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

        max_label_dimension = max_label_dimension.max(bounds.height);
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
            let title_dimension = title_bounds.height;

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
    let label_ctx = ThemeContext::new("guide", theme_params.clone())
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
    let measurer = default_text_measurer();

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
    label_ctx: &ThemeContext,
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
        color: ColorOrGradient::Color(theme.text_color(label_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]))
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

    let measurer = default_text_measurer();

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
        max_label_dimension = max_label_dimension.max(bounds.height);
    }

    // Query rule styling from theme
    let rule_ctx = ThemeContext::new("guide", theme_params.clone())
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
            stroke: ColorOrGradient::Color(rule_stroke).into(),
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
                stroke: ColorOrGradient::Color(rule_stroke).into(),
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
            stroke: ColorOrGradient::Color(rule_stroke).into(),
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
                stroke: ColorOrGradient::Color(rule_stroke).into(),
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
    let measurer = default_text_measurer();
    let title_ctx = ThemeContext::new("guide", theme_params.clone())
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
        max_label_dimension = max_label_dimension.max(bounds.height);
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
        // Center title over the rendered guide span so title alignment remains
        // stable when left/right subplot reserves are asymmetric.
        let x_center = if let Some(override_x) = config.col_title_x_override {
            override_x
        } else if let (Some(first), Some(last)) =
            (config.band_positions.first(), config.band_positions.last())
        {
            let first_center = config.plot_bounds.x + first.center();
            let last_center = config.plot_bounds.x + last.center();
            0.5 * (first_center + last_center)
        } else {
            config.plot_bounds.x + 0.5 * config.plot_bounds.width
        };
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
        color: ColorOrGradient::Color(theme.text_color(&title_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]))
            .into(),
        zindex: Some(6),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartesian::axis::AxisPosition;
    use crate::facet::band_positions::BandPosition;
    use crate::facet::evaluated_facet_tree::{EvaluatedFacetTree, PartitionNode};
    use crate::guide::FacetDirection;
    use avenger_scenegraph::marks::mark::SceneMark;
    use indexmap::IndexMap;

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    fn column_then_row_tree() -> EvaluatedFacetTree {
        let make_leaf = || {
            PartitionNode::leaf(
                FacetDirection::Row,
                255,
                "species".to_string(),
                None,
                vec![s("setosa"), s("versicolor"), s("virginica")],
            )
        };

        let mut children = IndexMap::new();
        children.insert(s("short"), Box::new(make_leaf()));
        children.insert(s("medium"), Box::new(make_leaf()));
        children.insert(s("long"), Box::new(make_leaf()));

        EvaluatedFacetTree::new(Some(PartitionNode::branch(
            FacetDirection::Column,
            255,
            "length_bin".to_string(),
            None,
            children,
        )))
    }

    fn title_x_from_marks(marks: &[SceneMark], expected_title: &str) -> f32 {
        marks
            .iter()
            .find_map(|mark| {
                let SceneMark::Text(text_mark) = mark else {
                    return None;
                };
                let text = text_mark.text_iter().next()?;
                if text == expected_title {
                    text_mark.x_iter().next().copied()
                } else {
                    None
                }
            })
            .expect("expected title text mark")
    }

    #[test]
    fn col_title_uses_guide_span_midpoint_when_bands_are_present() {
        let plot_bounds = LayoutBounds {
            x: 120.0,
            y: 80.0,
            width: 620.0,
            height: 300.0,
        };
        let band_positions = vec![
            BandPosition::new(ScalarValue::Utf8(Some("A".into())), 75.0, 160.0),
            BandPosition::new(ScalarValue::Utf8(Some("B".into())), 455.0, 140.0),
        ];
        let expected_center = {
            let first_center = plot_bounds.x + band_positions[0].center();
            let last_center = plot_bounds.x + band_positions[1].center();
            0.5 * (first_center + last_center)
        };
        let config = FacetLabelRenderConfig {
            labels: vec!["A".to_string(), "B".to_string()],
            band_positions,
            plot_bounds,
            is_rotated: false,
            place_at_end: false,
            font_family: "sans-serif".to_string(),
            font_size_px: 10.0,
            title: Some("species".to_string()),
            title_font_family: "sans-serif".to_string(),
            title_font_size_px: 12.0,
            render_title: true,
            col_title_x_override: None,
        };
        let marks = render_facet_label_slab(&config, &Theme::light(), &IndexMap::new());
        let title_x = title_x_from_marks(&marks, "species");
        assert!(
            (title_x - expected_center).abs() <= 0.01,
            "expected title x={} to match guide midpoint {}",
            title_x,
            expected_center
        );
    }

    #[test]
    fn col_title_falls_back_to_plot_bounds_center_without_bands() {
        let plot_bounds = LayoutBounds {
            x: 140.0,
            y: 50.0,
            width: 500.0,
            height: 280.0,
        };
        let expected_center = plot_bounds.x + 0.5 * plot_bounds.width;
        let config = FacetLabelRenderConfig {
            labels: vec![],
            band_positions: vec![],
            plot_bounds,
            is_rotated: false,
            place_at_end: false,
            font_family: "sans-serif".to_string(),
            font_size_px: 10.0,
            title: Some("species".to_string()),
            title_font_family: "sans-serif".to_string(),
            title_font_size_px: 12.0,
            render_title: true,
            col_title_x_override: None,
        };
        let marks = render_facet_label_slab(&config, &Theme::light(), &IndexMap::new());
        let title_x = title_x_from_marks(&marks, "species");
        assert!(
            (title_x - expected_center).abs() <= 0.01,
            "expected fallback title x={} to match plot center {}",
            title_x,
            expected_center
        );
    }

    #[test]
    fn col_title_uses_override_midpoint_when_present() {
        let plot_bounds = LayoutBounds {
            x: 100.0,
            y: 50.0,
            width: 500.0,
            height: 300.0,
        };
        let override_x = 412.5;
        let config = FacetLabelRenderConfig {
            labels: vec!["A".to_string()],
            band_positions: vec![BandPosition::new(
                ScalarValue::Utf8(Some("A".into())),
                40.0,
                180.0,
            )],
            plot_bounds,
            is_rotated: false,
            place_at_end: false,
            font_family: "sans-serif".to_string(),
            font_size_px: 10.0,
            title: Some("species".to_string()),
            title_font_family: "sans-serif".to_string(),
            title_font_size_px: 12.0,
            render_title: true,
            col_title_x_override: Some(override_x),
        };
        let marks = render_facet_label_slab(&config, &Theme::light(), &IndexMap::new());
        let title_x = title_x_from_marks(&marks, "species");
        assert!(
            (title_x - override_x).abs() <= 0.01,
            "expected override title x={} but got {}",
            override_x,
            title_x
        );
    }

    #[test]
    fn facet_guide_labels_shared_right_only_show_on_far_right_owner() {
        let tree = column_then_row_tree();

        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("short")],
            AxisPosition::Right,
            255
        ));
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("medium")],
            AxisPosition::Right,
            255
        ));
        assert!(facet_guide_labels_visible_for_cell(
            &tree,
            &[s("long")],
            AxisPosition::Right,
            255
        ));
    }

    #[test]
    fn facet_guide_labels_shared_left_only_show_on_far_left_owner() {
        let tree = column_then_row_tree();

        assert!(facet_guide_labels_visible_for_cell(
            &tree,
            &[s("short")],
            AxisPosition::Left,
            255
        ));
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("medium")],
            AxisPosition::Left,
            255
        ));
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("long")],
            AxisPosition::Left,
            255
        ));
    }

    #[test]
    fn facet_guide_labels_free_sharing_show_on_all_columns() {
        let tree = column_then_row_tree();
        for bucket in ["short", "medium", "long"] {
            assert!(facet_guide_labels_visible_for_cell(
                &tree,
                &[s(bucket)],
                AxisPosition::Right,
                0
            ));
        }
    }

    #[test]
    fn facet_guide_labels_jagged_groups_keep_group_local_owner_hiding() {
        let mut group_a = IndexMap::new();
        group_a.insert(
            s("C1"),
            Box::new(PartitionNode::leaf(
                FacetDirection::Row,
                0,
                "leaf".to_string(),
                None,
                vec![s("L")],
            )),
        );
        group_a.insert(
            s("C2"),
            Box::new(PartitionNode::leaf(
                FacetDirection::Row,
                0,
                "leaf".to_string(),
                None,
                vec![s("L")],
            )),
        );
        group_a.insert(
            s("C3"),
            Box::new(PartitionNode::leaf(
                FacetDirection::Row,
                0,
                "leaf".to_string(),
                None,
                vec![s("L")],
            )),
        );

        let mut group_b = IndexMap::new();
        group_b.insert(
            s("C1"),
            Box::new(PartitionNode::leaf(
                FacetDirection::Row,
                0,
                "leaf".to_string(),
                None,
                vec![s("L")],
            )),
        );
        group_b.insert(
            s("C2"),
            Box::new(PartitionNode::leaf(
                FacetDirection::Row,
                0,
                "leaf".to_string(),
                None,
                vec![s("L")],
            )),
        );

        let mut outer = IndexMap::new();
        outer.insert(
            s("A"),
            Box::new(PartitionNode::branch(
                FacetDirection::Column,
                255,
                "inner_col".to_string(),
                None,
                group_a,
            )),
        );
        outer.insert(
            s("B"),
            Box::new(PartitionNode::branch(
                FacetDirection::Column,
                255,
                "inner_col".to_string(),
                None,
                group_b,
            )),
        );

        let tree = EvaluatedFacetTree::new(Some(PartitionNode::branch(
            FacetDirection::Row,
            255,
            "outer_row".to_string(),
            None,
            outer,
        )));

        // Sharing level 1 means ownership is evaluated within each outer-row group.
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("A"), s("C1")],
            AxisPosition::Right,
            1
        ));
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("A"), s("C2")],
            AxisPosition::Right,
            1
        ));
        assert!(facet_guide_labels_visible_for_cell(
            &tree,
            &[s("A"), s("C3")],
            AxisPosition::Right,
            1
        ));

        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("B"), s("C1")],
            AxisPosition::Right,
            1
        ));
        assert!(facet_guide_labels_visible_for_cell(
            &tree,
            &[s("B"), s("C2")],
            AxisPosition::Right,
            1
        ));
    }
}
