//! Internal helpers for labels owned by child-frame containers.

use std::sync::Arc;

use avenger_common::types::ColorOrGradient;
use avenger_scenegraph::marks::{mark::SceneMark, text::SceneTextMark};
use avenger_text::{
    measurement::{TextBounds, TextMeasurementConfig, TextMeasurer, default_text_measurer},
    types::{FontStyle, FontWeight, TextAlign, TextBaseline},
};
use datafusion::common::ScalarValue;
use indexmap::IndexMap;

use crate::{
    layout::{LayoutBounds, Size2D},
    theme::{Theme, ThemeContext},
};

const LABEL_GAP: f32 = 4.0;
const DEFAULT_FONT_SIZE: f32 = 10.0;
const DEFAULT_FONT_WEIGHT: f32 = 300.0;
const DEFAULT_TEXT_COLOR: [f32; 4] = [0.35, 0.35, 0.35, 1.0];

/// Side where a container label slab is attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerLabelPlacement {
    Top,
    Left,
}

/// One label attached to a child frame.
#[derive(Debug, Clone)]
pub(crate) struct ContainerLabelItem {
    pub(crate) text: String,
    pub(crate) plot_origin: [f32; 2],
    pub(crate) plot_size: Size2D,
    pub(crate) frame_bounds: LayoutBounds,
}

#[derive(Debug, Clone)]
struct ContainerLabelStyle {
    font_family: String,
    font_size: f32,
    font_weight: FontWeight,
    color: [f32; 4],
}

#[derive(Debug)]
struct MeasuredContainerLabel<'a> {
    item: &'a ContainerLabelItem,
    bounds: TextBounds,
}

/// Measure the extra slab required to draw labels outside the already measured
/// child-frame envelope.
pub(crate) fn measure_container_label_slab(
    placement: ContainerLabelPlacement,
    items: &[ContainerLabelItem],
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
) -> f32 {
    let Some(max_label_extent) = measured_container_labels(items, &label_style(theme, params))
        .into_iter()
        .map(|label| match placement {
            ContainerLabelPlacement::Top => label.bounds.height,
            ContainerLabelPlacement::Left => label.bounds.width,
        })
        .reduce(f32::max)
    else {
        return 0.0;
    };

    max_label_extent + LABEL_GAP
}

/// Render child-frame container labels into the label slab measured by
/// `measure_container_label_slab`.
pub(crate) fn render_container_labels(
    placement: ContainerLabelPlacement,
    items: &[ContainerLabelItem],
    plot_bounds: &LayoutBounds,
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
) -> Vec<SceneMark> {
    let style = label_style(theme, params);
    let measured = measured_container_labels(items, &style);
    if measured.is_empty() {
        return Vec::new();
    }

    match placement {
        ContainerLabelPlacement::Top => render_top_labels(&measured, plot_bounds, &style),
        ContainerLabelPlacement::Left => render_left_labels(&measured, plot_bounds, &style),
    }
}

fn render_top_labels(
    labels: &[MeasuredContainerLabel<'_>],
    plot_bounds: &LayoutBounds,
    style: &ContainerLabelStyle,
) -> Vec<SceneMark> {
    let child_top = labels
        .iter()
        .map(|label| (-label.item.frame_bounds.y).max(0.0))
        .fold(0.0f32, f32::max);
    let y = plot_bounds.y - child_top - LABEL_GAP;

    labels
        .iter()
        .map(|label| {
            let x = plot_bounds.x + label.item.plot_origin[0] + label.item.plot_size.width / 2.0;
            SceneMark::Text(Arc::new(text_mark(
                &label.item.text,
                x,
                y,
                TextAlign::Center,
                TextBaseline::Bottom,
                0.0,
                style,
            )))
        })
        .collect()
}

fn render_left_labels(
    labels: &[MeasuredContainerLabel<'_>],
    plot_bounds: &LayoutBounds,
    style: &ContainerLabelStyle,
) -> Vec<SceneMark> {
    let child_left = labels
        .iter()
        .map(|label| (-label.item.frame_bounds.x).max(0.0))
        .fold(0.0f32, f32::max);
    let x = plot_bounds.x - child_left - LABEL_GAP;

    labels
        .iter()
        .map(|label| {
            let y = plot_bounds.y + label.item.plot_origin[1] + label.item.plot_size.height / 2.0;
            SceneMark::Text(Arc::new(text_mark(
                &label.item.text,
                x,
                y,
                TextAlign::Right,
                TextBaseline::Middle,
                0.0,
                style,
            )))
        })
        .collect()
}

fn measured_container_labels<'a>(
    items: &'a [ContainerLabelItem],
    style: &ContainerLabelStyle,
) -> Vec<MeasuredContainerLabel<'a>> {
    let measurer = default_text_measurer();
    items
        .iter()
        .filter(|item| !item.text.trim().is_empty())
        .map(|item| {
            let config = TextMeasurementConfig {
                text: &item.text,
                font: &style.font_family,
                font_size: style.font_size,
                font_weight: &style.font_weight,
                font_style: &FontStyle::Normal,
            };
            MeasuredContainerLabel {
                item,
                bounds: measurer.measure_text_bounds(&config),
            }
        })
        .collect()
}

fn text_mark(
    text: &str,
    x: f32,
    y: f32,
    align: TextAlign,
    baseline: TextBaseline,
    angle: f32,
    style: &ContainerLabelStyle,
) -> SceneTextMark {
    SceneTextMark {
        text: text.to_string().into(),
        x: x.into(),
        y: y.into(),
        align: align.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        font: style.font_family.clone().into(),
        font_size: style.font_size.into(),
        font_weight: style.font_weight.into(),
        color: ColorOrGradient::Color(style.color).into(),
        zindex: Some(5),
        clip: false,
        ..Default::default()
    }
}

fn label_style(theme: &Theme, params: &IndexMap<String, ScalarValue>) -> ContainerLabelStyle {
    let ctx = ThemeContext::new("guide", params.clone())
        .child("concat")
        .child("label");
    ContainerLabelStyle {
        font_family: theme
            .font_family(&ctx)
            .unwrap_or_else(|| "Atkinson Hyperlegible Next".to_string()),
        font_size: theme.font_size(&ctx).unwrap_or(DEFAULT_FONT_SIZE),
        font_weight: FontWeight::Number(theme.font_weight(&ctx).unwrap_or(DEFAULT_FONT_WEIGHT)),
        color: theme.text_color(&ctx).unwrap_or(DEFAULT_TEXT_COLOR),
    }
}
