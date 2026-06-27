//! Internal renderer for banded child-frame container guide strips.
//!
//! Facet row/column strips are the first consumer, but the geometry here is
//! generic: a set of band positions, labels, optional rule/ticks, and an
//! optional title attached to one side of a container content rectangle.

use std::sync::Arc as StdArc;

use avenger_color::ColorOrGradient;
use avenger_scenegraph::marks::{mark::SceneMark, rule::SceneRuleMark, text::SceneTextMark};
use avenger_text::{
    TextEngine, default_text_engine,
    measurement::TextMeasurementConfig,
    types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline},
};
use datafusion::common::ScalarValue;
use indexmap::IndexMap;

use crate::{
    layout::{BandPosition, LayoutBounds},
    theme::{Theme, ThemeContext},
};

/// Configuration for measuring band-guide slab space requirements.
#[derive(Clone, Debug)]
pub(crate) struct ContainerBandGuideMeasurementConfig {
    /// Labels to measure.
    pub labels: Vec<String>,
    /// Font family for labels.
    pub font_family: String,
    /// Font size in pixels for labels.
    pub font_size_px: f32,
    /// Optional title text.
    pub title: Option<String>,
    /// Font family for title.
    pub title_font_family: String,
    /// Font size in pixels for title.
    pub title_font_size_px: f32,
    /// Whether to render/measure the title.
    pub render_title: bool,
}

/// Configuration for rendering a band-guide label slab.
#[derive(Clone, Debug)]
pub(crate) struct ContainerBandGuideRenderConfig {
    /// Labels to render.
    pub labels: Vec<String>,
    /// Band positions for centering labels.
    pub band_positions: Vec<BandPosition>,
    /// Parent content bounds for positioning.
    pub plot_bounds: LayoutBounds,
    /// Whether labels are rotated 90 degrees.
    pub labels_rotated: bool,
    /// Whether to place labels at the far edge.
    pub place_at_end: bool,
    /// Theme child under `guide` for labels, rules, and title.
    pub theme_component: String,
    /// Font family for labels.
    pub font_family: String,
    /// Font size in pixels for labels.
    pub font_size_px: f32,
    /// Optional title text.
    pub title: Option<String>,
    /// Font family for title.
    pub title_font_family: String,
    /// Font size in pixels for title.
    pub title_font_size_px: f32,
    /// Whether to render the title.
    pub render_title: bool,
    /// Optional absolute x-position override for horizontal titles.
    pub title_x_override: Option<f32>,
}

pub(crate) fn measure_container_band_guide_slab(
    config: &ContainerBandGuideMeasurementConfig,
) -> f32 {
    let text_engine = default_text_engine();
    measure_container_band_guide_slab_uncached(config, &text_engine)
}

fn measure_container_band_guide_slab_uncached(
    config: &ContainerBandGuideMeasurementConfig,
    text_engine: &TextEngine,
) -> f32 {
    let mut max_label_dimension = 0.0_f32;
    for label in &config.labels {
        max_label_dimension = max_label_dimension.max(measure_text_height(
            label,
            &config.font_family,
            config.font_size_px,
            text_engine,
        ));
    }

    let mut total_space = max_label_dimension;

    if config.labels.len() > 1 && !config.render_title {
        let gap = 10.0_f32;
        let rule_stroke = 1.0_f32;
        let tick_size = 4.0_f32;
        total_space += gap / 2.0 + rule_stroke + tick_size;
    }

    if config.render_title
        && let Some(title_text) = &config.title
    {
        let title_dimension = measure_text_height(
            title_text,
            &config.title_font_family,
            config.title_font_size_px,
            text_engine,
        );

        let gap = 10.0_f32;
        total_space += gap + title_dimension + 1.0;
    }

    total_space + 1.0
}

pub(crate) fn render_container_band_guide_slab(
    config: &ContainerBandGuideRenderConfig,
    theme: &Theme,
    theme_params: &IndexMap<String, ScalarValue>,
) -> Vec<SceneMark> {
    let text_engine = default_text_engine();
    let mut marks = Vec::new();
    let label_dimensions = measure_label_dimensions(
        &config.labels,
        &config.font_family,
        config.font_size_px,
        &text_engine,
    );
    let max_label_dimension = label_dimensions.iter().copied().fold(0.0_f32, f32::max);

    let label_ctx = ThemeContext::new("guide", theme_params.clone())
        .child(config.theme_component.as_str())
        .child("label");
    let label_color = theme.text_color(&label_ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]);

    let (x_positions, y_positions) = if config.labels_rotated {
        calculate_rotated_label_positions(config, &label_dimensions)
    } else {
        calculate_horizontal_label_positions(config)
    };

    for (i, label) in config.labels.iter().enumerate() {
        let band_pos = &config.band_positions[i];
        let label_mark = create_label_mark(
            label,
            band_pos,
            config,
            x_positions[i],
            y_positions[i],
            label_color,
        );
        marks.push(SceneMark::Text(StdArc::new(label_mark)));
    }

    if config.labels.len() > 1 {
        marks.extend(render_rule_with_ticks(
            config,
            theme,
            theme_params,
            max_label_dimension,
        ));
    }

    if config.render_title
        && let Some(title_text) = &config.title
    {
        let title_mark = render_container_band_title(
            title_text,
            config,
            theme,
            theme_params,
            max_label_dimension,
            &text_engine,
        );
        marks.push(SceneMark::Text(StdArc::new(title_mark)));
    }

    marks
}

fn measure_label_dimensions(
    labels: &[String],
    font_family: &str,
    font_size_px: f32,
    text_engine: &TextEngine,
) -> Vec<f32> {
    labels
        .iter()
        .map(|label| measure_text_height(label, font_family, font_size_px, text_engine))
        .collect()
}

fn calculate_rotated_label_positions(
    config: &ContainerBandGuideRenderConfig,
    label_dimensions: &[f32],
) -> (Vec<f32>, Vec<f32>) {
    let mut x_positions = Vec::new();
    let mut y_positions = Vec::new();

    for i in 0..config.labels.len() {
        let band_pos = &config.band_positions[i];
        let y_center = config.plot_bounds.y + band_pos.center();

        let x_center = if config.place_at_end {
            config.plot_bounds.x + config.plot_bounds.width + 0.5 * label_dimensions[i]
        } else {
            config.plot_bounds.x - 0.5 * label_dimensions[i]
        };

        x_positions.push(x_center);
        y_positions.push(y_center);
    }

    (x_positions, y_positions)
}

fn calculate_horizontal_label_positions(
    config: &ContainerBandGuideRenderConfig,
) -> (Vec<f32>, Vec<f32>) {
    let mut x_positions = Vec::new();
    let mut y_positions = Vec::new();

    let y_label = if config.place_at_end {
        config.plot_bounds.y + config.plot_bounds.height
    } else {
        config.plot_bounds.y
    };

    for band_pos in &config.band_positions {
        let x_center = config.plot_bounds.x + band_pos.center();
        x_positions.push(x_center);
        y_positions.push(y_label);
    }

    (x_positions, y_positions)
}

fn create_label_mark(
    label: &str,
    _band_pos: &BandPosition,
    config: &ContainerBandGuideRenderConfig,
    x: f32,
    y: f32,
    label_color: [f32; 4],
) -> SceneTextMark {
    let angle = if config.labels_rotated {
        if config.place_at_end {
            90.0_f32
        } else {
            -90.0_f32
        }
    } else {
        0.0_f32
    };

    let baseline = if config.labels_rotated {
        TextBaseline::Middle
    } else if config.place_at_end {
        TextBaseline::Top
    } else {
        TextBaseline::Bottom
    };

    SceneTextMark {
        clip: false,
        text: label.to_string().into(),
        x: x.into(),
        y: y.into(),
        align: TextAlign::Center.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        font: config.font_family.clone().into(),
        font_size: config.font_size_px.into(),
        color: ColorOrGradient::Color(label_color).into(),
        zindex: Some(5),
        ..Default::default()
    }
}

fn render_rule_with_ticks(
    config: &ContainerBandGuideRenderConfig,
    theme: &Theme,
    theme_params: &IndexMap<String, ScalarValue>,
    max_label_dimension: f32,
) -> Vec<SceneMark> {
    let mut marks = Vec::new();

    if config.labels.is_empty() || config.labels.len() <= 1 {
        return marks;
    }

    let rule_ctx = ThemeContext::new("guide", theme_params.clone())
        .child(config.theme_component.as_str())
        .child("rule");
    let mut rule_stroke = theme.text_color(&rule_ctx).unwrap_or([0.5, 0.5, 0.5, 1.0]);
    rule_stroke[3] = 1.0;
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

    if config.labels_rotated {
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

fn render_container_band_title(
    title_text: &str,
    config: &ContainerBandGuideRenderConfig,
    theme: &Theme,
    theme_params: &IndexMap<String, ScalarValue>,
    max_label_dimension: f32,
    text_engine: &TextEngine,
) -> SceneTextMark {
    let title_ctx = ThemeContext::new("guide", theme_params.clone())
        .child(config.theme_component.as_str())
        .child("title");

    let title_height = measure_text_height(
        title_text,
        &config.title_font_family,
        config.title_font_size_px,
        text_engine,
    );

    let gap = 10.0_f32;

    let (x, y, angle, baseline) = if config.labels_rotated {
        let x_center = if config.place_at_end {
            config.plot_bounds.x
                + config.plot_bounds.width
                + max_label_dimension
                + gap
                + 0.5 * title_height
        } else {
            config.plot_bounds.x - (max_label_dimension + gap + 0.5 * title_height)
        };
        let y_center = config.plot_bounds.y + 0.5 * config.plot_bounds.height;
        let angle = if config.place_at_end {
            90.0_f32
        } else {
            -90.0_f32
        };
        (x_center, y_center, angle, TextBaseline::Middle)
    } else {
        let x_center = if let Some(override_x) = config.title_x_override {
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
        clip: false,
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

fn measure_text_height(
    text: &str,
    font_family: &str,
    font_size_px: f32,
    text_engine: &TextEngine,
) -> f32 {
    let text_config = TextMeasurementConfig {
        text,
        font: font_family,
        font_size: font_size_px,
        font_weight: FontWeight::Name(FontWeightNameSpec::Normal),
        font_style: FontStyle::Normal,
        syntax_mode: avenger_text::types::TextSyntaxMode::Plain,
    };
    text_engine
        .measure_bounds_with_plain_fallback_or_approx(&text_config)
        .height
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_scenegraph::marks::mark::SceneMark;
    use datafusion::common::ScalarValue;
    use indexmap::IndexMap;

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

    fn render_config(
        plot_bounds: LayoutBounds,
        band_positions: Vec<BandPosition>,
        title_x_override: Option<f32>,
    ) -> ContainerBandGuideRenderConfig {
        ContainerBandGuideRenderConfig {
            labels: band_positions
                .iter()
                .map(|position| position.value.to_string())
                .collect(),
            band_positions,
            plot_bounds,
            labels_rotated: false,
            place_at_end: false,
            theme_component: "facet".to_string(),
            font_family: "sans-serif".to_string(),
            font_size_px: 10.0,
            title: Some("species".to_string()),
            title_font_family: "sans-serif".to_string(),
            title_font_size_px: 12.0,
            render_title: true,
            title_x_override,
        }
    }

    fn render_with_plain_text(config: &ContainerBandGuideRenderConfig) -> Vec<SceneMark> {
        render_container_band_guide_slab(config, &Theme::light(), &IndexMap::new())
    }

    #[test]
    fn horizontal_title_uses_guide_span_midpoint_when_bands_are_present() {
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
        let config = render_config(plot_bounds, band_positions, None);
        let marks = render_with_plain_text(&config);
        let title_x = title_x_from_marks(&marks, "species");
        assert!(
            (title_x - expected_center).abs() <= 0.01,
            "expected title x={} to match guide midpoint {}",
            title_x,
            expected_center
        );
    }

    #[test]
    fn horizontal_title_falls_back_to_plot_bounds_center_without_bands() {
        let plot_bounds = LayoutBounds {
            x: 140.0,
            y: 50.0,
            width: 500.0,
            height: 280.0,
        };
        let expected_center = plot_bounds.x + 0.5 * plot_bounds.width;
        let config = render_config(plot_bounds, vec![], None);
        let marks = render_with_plain_text(&config);
        let title_x = title_x_from_marks(&marks, "species");
        assert!(
            (title_x - expected_center).abs() <= 0.01,
            "expected fallback title x={} to match plot center {}",
            title_x,
            expected_center
        );
    }

    #[test]
    fn horizontal_title_uses_override_midpoint_when_present() {
        let plot_bounds = LayoutBounds {
            x: 100.0,
            y: 50.0,
            width: 500.0,
            height: 300.0,
        };
        let override_x = 412.5;
        let config = render_config(
            plot_bounds,
            vec![BandPosition::new(
                ScalarValue::Utf8(Some("A".into())),
                40.0,
                180.0,
            )],
            Some(override_x),
        );
        let marks = render_with_plain_text(&config);
        let title_x = title_x_from_marks(&marks, "species");
        assert!(
            (title_x - override_x).abs() <= 0.01,
            "expected override title x={} but got {}",
            override_x,
            title_x
        );
    }
}
