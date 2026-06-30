//! Internal helpers for labels owned by child-frame containers.

use std::sync::Arc;

use avenger_color::ColorOrGradient;
use avenger_scenegraph::marks::{mark::SceneMark, text::SceneTextMark};
use avenger_text::{
    TextEngine,
    measurement::{TextBounds, TextMeasurementConfig},
    types::{FontStyle, FontWeight, TextAlign, TextBaseline},
};
use datafusion::common::ScalarValue;
use indexmap::IndexMap;

use crate::{
    error::AvengerChartError,
    layout::{LayoutBounds, Size2D},
    theme::{Theme, ThemeContext},
};

use super::{ChildFrameContainerView, ChildFrameRegion};

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

/// Child-local geometry needed to project container labels into the parent
/// frame.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ContainerLabelChildFrame {
    pub(crate) plot_bounds: LayoutBounds,
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

/// Derive label anchors from solved child-frame regions.
pub(crate) fn container_label_items_from_child_regions<'a>(
    regions: &[ChildFrameRegion],
    mut child_frame: impl FnMut(usize) -> Result<ContainerLabelChildFrame, AvengerChartError>,
    mut child_label: impl FnMut(usize) -> Option<&'a str>,
) -> Result<Vec<ContainerLabelItem>, AvengerChartError> {
    let mut items = Vec::new();

    for region in regions {
        let child_frame = child_frame(region.child_index)?;
        let Some(label) = child_label(region.child_index).filter(|label| !label.trim().is_empty())
        else {
            continue;
        };

        let frame_bounds =
            region.project_child_bounds(child_frame.plot_bounds, child_frame.frame_bounds);

        items.push(ContainerLabelItem {
            text: label.to_string(),
            plot_origin: region.plot_origin(),
            plot_size: child_frame.plot_size,
            frame_bounds,
        });
    }

    Ok(items)
}

/// Derive label anchors from a measured child-frame container view.
pub(crate) fn container_label_items_from_child_frame_container(
    container: &ChildFrameContainerView<'_>,
) -> Result<Vec<ContainerLabelItem>, AvengerChartError> {
    container_label_items_from_child_regions(
        container.child_regions(),
        |child_index| {
            let child = container.child_measurement(child_index).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing child-frame measurement for child index {}",
                    child_index
                ))
            })?;
            Ok(ContainerLabelChildFrame {
                plot_bounds: *child.layout.plot_area_bounds(),
                plot_size: Size2D::new(child.plot_area_width, child.plot_area_height),
                frame_bounds: child.frame_allocation.rect,
            })
        },
        |child_index| container.child_label(child_index),
    )
}

/// Measure the extra slab required to draw labels outside the already measured
/// child-frame envelope.
pub(crate) fn measure_container_label_slab(
    placement: ContainerLabelPlacement,
    items: &[ContainerLabelItem],
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
) -> f32 {
    let text_engine = crate::fonts::default_chart_text_engine();
    let Some(max_label_extent) =
        measured_container_labels(items, &label_style(theme, params), &text_engine)
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
    let text_engine = crate::fonts::default_chart_text_engine();
    let style = label_style(theme, params);
    let measured = measured_container_labels(items, &style, &text_engine);
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
    text_engine: &TextEngine,
) -> Vec<MeasuredContainerLabel<'a>> {
    items
        .iter()
        .filter(|item| !item.text.trim().is_empty())
        .map(|item| {
            let config = TextMeasurementConfig {
                text: &item.text,
                font: &style.font_family,
                font_size: style.font_size,
                font_weight: style.font_weight,
                font_style: FontStyle::Normal,
                syntax_mode: avenger_text::types::TextSyntaxMode::Plain,
                params: avenger_text::empty_label_params(),
                number_locale: None,
            };
            MeasuredContainerLabel {
                item,
                bounds: text_engine.measure_bounds_with_plain_fallback_or_approx(&config),
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
            .unwrap_or_else(|| "Lato".to_string()),
        font_size: theme.font_size(&ctx).unwrap_or(DEFAULT_FONT_SIZE),
        font_weight: FontWeight::Number(theme.font_weight(&ctx).unwrap_or(DEFAULT_FONT_WEIGHT)),
        color: theme.text_color(&ctx).unwrap_or(DEFAULT_TEXT_COLOR),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_items_project_child_frames_and_skip_empty_labels() {
        let regions = vec![
            ChildFrameRegion {
                child_index: 10,
                content: LayoutBounds {
                    x: 20.0,
                    y: 30.0,
                    width: 100.0,
                    height: 80.0,
                },
                slot: LayoutBounds {
                    x: 20.0,
                    y: 30.0,
                    width: 100.0,
                    height: 80.0,
                },
                content_size_override: None,
                edge_targets: None,
            },
            ChildFrameRegion {
                child_index: 20,
                content: LayoutBounds {
                    x: 140.0,
                    y: 30.0,
                    width: 90.0,
                    height: 70.0,
                },
                slot: LayoutBounds {
                    x: 140.0,
                    y: 30.0,
                    width: 90.0,
                    height: 70.0,
                },
                content_size_override: None,
                edge_targets: None,
            },
        ];

        let items = container_label_items_from_child_regions(
            &regions,
            |child_index| match child_index {
                10 => Ok(ContainerLabelChildFrame {
                    plot_bounds: LayoutBounds {
                        x: 5.0,
                        y: 8.0,
                        width: 100.0,
                        height: 80.0,
                    },
                    plot_size: Size2D::new(100.0, 80.0),
                    frame_bounds: LayoutBounds {
                        x: 1.0,
                        y: 2.0,
                        width: 130.0,
                        height: 100.0,
                    },
                }),
                20 => Ok(ContainerLabelChildFrame {
                    plot_bounds: LayoutBounds {
                        x: 0.0,
                        y: 0.0,
                        width: 90.0,
                        height: 70.0,
                    },
                    plot_size: Size2D::new(90.0, 70.0),
                    frame_bounds: LayoutBounds {
                        x: 0.0,
                        y: 0.0,
                        width: 90.0,
                        height: 70.0,
                    },
                }),
                _ => unreachable!(),
            },
            |child_index| match child_index {
                10 => Some("First"),
                20 => Some("   "),
                _ => unreachable!(),
            },
        )
        .unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "First");
        assert_eq!(items[0].plot_origin, [20.0, 30.0]);
        assert_eq!(items[0].plot_size, Size2D::new(100.0, 80.0));
        assert_eq!(
            items[0].frame_bounds,
            LayoutBounds {
                x: 16.0,
                y: 24.0,
                width: 130.0,
                height: 100.0,
            }
        );
    }

    #[test]
    fn label_items_require_matching_child_geometry() {
        let regions = vec![ChildFrameRegion {
            child_index: 2,
            content: LayoutBounds {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 80.0,
            },
            slot: LayoutBounds {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 80.0,
            },
            content_size_override: None,
            edge_targets: None,
        }];

        let err = container_label_items_from_child_regions(
            &regions,
            |child_index| {
                Err(AvengerChartError::InternalError(format!(
                    "missing child {child_index}"
                )))
            },
            |_child_index| None,
        )
        .unwrap_err();

        match err {
            AvengerChartError::InternalError(message) => {
                assert!(message.contains("missing child 2"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
