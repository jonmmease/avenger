use avenger_color::ColorOrGradient;
use avenger_common::{types::SymbolShape, value::ScalarOrArray};
use avenger_geometry::{
    marks::{MarkGeometryUtils, TextGeometryUtils},
    rtree::EnvelopeUtils,
};
use avenger_scenegraph::marks::{
    group::SceneGroup,
    mark::SceneMark,
    pattern::{default_no_fill_pattern, PatternFill, PatternReferenceFrame},
    symbol::SceneSymbolMark,
};
use avenger_typst_label::LabelEngine;

use super::{
    compute_encoding_length, make_background, make_entry_label, make_hit_rect, make_title,
    GuideLegendItem, GuideLegendOutput, LegendStyle,
};
use crate::error::AvengerGuidesError;

/// Space above and below each symbol, within its row.
const ROW_PADDING: f32 = 0.5;

/// Symbol legends
#[derive(Debug, Clone)]
pub struct SymbolLegendConfig {
    pub title: Option<String>,
    pub text: ScalarOrArray<String>,
    pub shape: ScalarOrArray<SymbolShape>,
    pub size: ScalarOrArray<f32>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub stroke_width: Option<f32>,
    pub fill: ScalarOrArray<ColorOrGradient>,
    pub fill_pattern: ScalarOrArray<Option<PatternFill>>,
    pub angle: ScalarOrArray<f32>,

    /// Padding between the symbol and the text
    pub text_padding: f32,

    pub style: LegendStyle,
}

impl Default for SymbolLegendConfig {
    fn default() -> Self {
        Self {
            title: None,
            text: ScalarOrArray::new_scalar("".to_string()),
            shape: ScalarOrArray::new_scalar(Default::default()),
            size: ScalarOrArray::new_scalar(20.0),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            stroke_width: None,
            fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            fill_pattern: default_no_fill_pattern(),
            angle: ScalarOrArray::new_scalar(0.0),
            text_padding: 2.0,
            style: LegendStyle::default(),
        }
    }
}

/// A symbol legend at the group origin, with a row for each entry and the scene paths that
/// interactions can target.
pub fn make_symbol_legend(
    config: &SymbolLegendConfig,
    text_engine: &LabelEngine,
) -> Result<GuideLegendOutput, AvengerGuidesError> {
    // Compute the common encoding length
    let len = compute_encoding_length(&[
        config.text.len(),
        config.shape.len(),
        config.size.len(),
        config.stroke.len(),
        config.fill.len(),
        config.fill_pattern.len(),
        config.angle.len(),
    ])?;

    // Compute the max width of all marks so that we can align the text next to them.
    let symbol_mark = SceneSymbolMark {
        len: len as u32,
        shapes: config.shape.as_vec(len, None),
        shape_index: ScalarOrArray::new_array((0..len).collect()),
        size: config.size.clone(),
        stroke: config.stroke.clone(),
        angle: config.angle.clone(),
        fill: config.fill.clone(),
        fill_pattern: config.fill_pattern.clone(),
        stroke_width: config.stroke_width,
        x: 0.0.into(),
        y: 0.0.into(),
        ..Default::default()
    };
    let max_width = symbol_mark.bounding_box().width().round();

    let padding = config.style.background_padding;
    let mut marks = Vec::with_capacity(len + 1);
    let mut y = padding;
    if let Some(title) = &config.title {
        let (title, height) = make_title(title, &config.style, text_engine)?;
        marks.push(title);
        y += height;
    }

    let mut items = Vec::with_capacity(len);
    for (index, text) in config.text.as_vec(len, None).into_iter().enumerate() {
        let group = make_symbol_group(
            &symbol_mark,
            &text,
            index,
            max_width,
            [padding, y.round()],
            config,
            text_engine,
        );
        y = (y + group.bounding_box(text_engine).height()).round();
        // The background comes first, so entry paths start at 1.
        let group_path = vec![marks.len() + 1];
        items.push(GuideLegendItem {
            index,
            label: text,
            hit_rect_path: vec![group_path[0], 0],
            group_path,
        });
        marks.push(SceneMark::Group(group));
    }

    let content = SceneGroup {
        marks: marks.clone(),
        ..Default::default()
    }
    .bounding_box(text_engine);
    let background = make_background(
        &config.style,
        content.width() + 2.0 * padding,
        content.height() + 2.0 * padding,
    );
    Ok(GuideLegendOutput {
        group: SceneGroup {
            marks: std::iter::once(background).chain(marks).collect(),
            ..Default::default()
        },
        items,
    })
}

/// An entry's row: its symbol centered in the widest symbol's column, its label after the
/// column, and a hit rect over both that spans the row's height.
fn make_symbol_group(
    symbols: &SceneSymbolMark,
    text: &str,
    index: usize,
    max_width: f32,
    origin: [f32; 2],
    config: &SymbolLegendConfig,
    text_engine: &LabelEngine,
) -> SceneGroup {
    let mut symbol = symbols.single_symbol_mark(index);
    symbol.x = (max_width / 2.0).round().into();
    let symbol_height = symbol.bounding_box().height();
    let y = (symbol_height / 2.0 + ROW_PADDING).round();
    symbol.y = y.into();
    // A pattern anchors to its own swatch, not to the legend.
    let pattern_reference_frame = symbol.fill_pattern_iter().any(Option::is_some).then(|| {
        let bounds = symbol.bounding_box();
        PatternReferenceFrame {
            x: bounds.lower()[0],
            y: bounds.lower()[1],
            width: bounds.width(),
            height: bounds.height(),
        }
    });
    let content = vec![
        SceneMark::Symbol(symbol).with_interactive(false),
        make_entry_label(text, max_width + config.text_padding, y, &config.style),
    ];
    let bounds = SceneGroup {
        marks: content.clone(),
        ..Default::default()
    }
    .bounding_box(text_engine);
    let row_height = (symbol_height + 2.0 * ROW_PADDING).round();
    let hit = make_hit_rect(
        bounds.lower()[0].min(0.0),
        bounds.lower()[1].min(0.0),
        bounds.upper()[0],
        bounds.upper()[1].max(row_height),
    );
    SceneGroup {
        origin,
        pattern_reference_frame,
        marks: std::iter::once(hit).chain(content).collect(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_typst_label::bundled_label_engine;

    #[test]
    fn default_pattern_anchor_resolves_in_translated_legend_swatches() {
        use avenger_scenegraph::{
            marks::pattern::{PatternLayer, StripePatternLayer},
            pattern_geometry::{build_layered_pattern_geometry, PatternRect, PatternRenderContext},
            render_order::{SceneDisplayList, SceneDisplayMark},
            scene_graph::SceneGraph,
        };
        let mut legend = make_symbol_legend(
            &SymbolLegendConfig {
                text: vec!["A".to_string(), "B".to_string()].into(),
                size: 100.0.into(),
                shape: SymbolShape::from_vega_str("square").unwrap().into(),
                fill_pattern: vec![2.0, 4.0]
                    .into_iter()
                    .map(|phase| {
                        let mut stripe = StripePatternLayer::new(0.0, 8.0, 2.0);
                        stripe.phase = phase;
                        Some(PatternFill {
                            layers: vec![PatternLayer::Stripe(stripe)],
                            ..Default::default()
                        })
                    })
                    .collect::<Vec<_>>()
                    .into(),
                ..Default::default()
            },
            &bundled_label_engine(),
        )
        .unwrap()
        .group;
        for origin in [[0.0, 0.0], [37.0, 53.0]] {
            legend.origin = origin;
            let scene = SceneGraph {
                width: 400.0,
                height: 200.0,
                origin: [0.0; 2],
                marks: vec![legend.clone().into()],
            };
            let list = SceneDisplayList::from_scene_graph(&scene);
            let mut phases = Vec::new();
            for item in &list.items {
                let SceneDisplayMark::Borrowed(SceneMark::Symbol(mark)) = &item.mark else {
                    continue;
                };
                let bounds = mark.bounding_box();
                let host_bounds = PatternRect::new(
                    bounds.lower()[0] + item.origin[0],
                    bounds.lower()[1] + item.origin[1],
                    bounds.width(),
                    bounds.height(),
                );
                let frame = item
                    .pattern_reference_frame
                    .as_ref()
                    .expect("swatch supplies a frame");
                assert_eq!(
                    [frame.x, frame.y, frame.width, frame.height],
                    [
                        host_bounds.x,
                        host_bounds.y,
                        host_bounds.width,
                        host_bounds.height
                    ]
                );
                let geometry = build_layered_pattern_geometry(
                    mark.fill_pattern_iter().next().unwrap().as_ref().unwrap(),
                    &PatternRenderContext {
                        chart_bounds: PatternRect::new(0.0, 0.0, scene.width, scene.height),
                        plot_bounds: Some(PatternRect::new(
                            frame.x,
                            frame.y,
                            frame.width,
                            frame.height,
                        )),
                        host_bounds,
                        host_fill: mark.fill_iter().next().unwrap(),
                        gradients: &[],
                    },
                )
                .unwrap()
                .unwrap();
                let first = geometry.layers[0].primitives[0]
                    .path()
                    .first_endpoint()
                    .unwrap()
                    .0;
                phases.push(first.y - host_bounds.y);
            }
            assert_eq!(phases, [1.0, 3.0]);
        }
    }

    #[test]
    fn itemized_symbol_legend_reports_interactive_hit_rects() {
        let output = make_symbol_legend(
            &SymbolLegendConfig {
                text: ScalarOrArray::new_array(vec!["A".to_string(), "B".to_string()]),
                fill: ScalarOrArray::new_array(vec![
                    ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0]),
                    ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]),
                ]),
                ..Default::default()
            },
            &avenger_typst_label::bundled_label_engine(),
        )
        .expect("symbol legend renders");

        assert_eq!(output.items.len(), 2);
        assert_eq!(output.items[0].group_path, vec![1]);
        assert_eq!(output.items[0].hit_rect_path, vec![1, 0]);
        assert_eq!(output.group.marks[0].interactive(), Some(false));

        let SceneMark::Group(item_group) = &output.group.marks[1] else {
            panic!("legend item should be a group");
        };
        assert_eq!(item_group.marks[0].interactive(), None);
        assert_eq!(item_group.marks[1].interactive(), Some(false));
        assert_eq!(item_group.marks[2].interactive(), Some(false));
        let SceneMark::Rect(hit_rect) = &item_group.marks[0] else {
            panic!("first item mark should be hit rect");
        };
        assert!(hit_rect.width.is_some());
        assert!(hit_rect.height.is_some());
    }

    #[test]
    fn symbol_hit_rect_preserves_reserved_row_height() {
        let output = make_symbol_legend(
            &SymbolLegendConfig {
                text: ScalarOrArray::new_array(vec!["small".to_string(), "medium".to_string()]),
                shape: ScalarOrArray::new_array(vec![
                    SymbolShape::Circle,
                    SymbolShape::from_vega_str("square").expect("square shape"),
                ]),
                size: ScalarOrArray::new_array(vec![30.0, 120.0]),
                stroke_width: Some(1.0),
                fill: ScalarOrArray::new_array(vec![
                    ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]),
                    ColorOrGradient::Color([0.0, 1.0, 0.0, 1.0]),
                ]),
                ..Default::default()
            },
            &avenger_typst_label::bundled_label_engine(),
        )
        .expect("symbol legend renders");

        let SceneMark::Group(item_group) = &output.group.marks[2] else {
            panic!("second legend item should be a group");
        };
        let SceneMark::Rect(hit_rect) = &item_group.marks[0] else {
            panic!("first item mark should be hit rect");
        };
        let SceneMark::Symbol(symbol) = &item_group.marks[1] else {
            panic!("second item visual should be a symbol");
        };

        let hit_y = hit_rect.y.as_vec(1, None)[0];
        let hit_height = hit_rect
            .height
            .as_ref()
            .expect("hit rect has height")
            .as_vec(1, None)[0];
        let reserved_row_height = (symbol.bounding_box().height() + 1.0).round();
        assert!(
            hit_y + hit_height >= reserved_row_height,
            "symbol legend hit rect should preserve reserved row height"
        );
    }
}
