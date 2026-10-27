//! Symbol swatches: each entry's symbol and label.

use avenger_color::ColorOrGradient;
use avenger_common::pattern::PatternFills;
use avenger_common::{types::SymbolShape, value::ScalarOrArray};
use avenger_geometry::{
    marks::{MarkGeometryUtils, TextGeometryUtils},
    rtree::EnvelopeUtils,
};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark, symbol::SceneSymbolMark};
use avenger_typst_label::LabelEngine;

use super::{make_entry_label, make_hit_rect, LegendStyle};

/// Space above and below each symbol, within its row.
const ROW_PADDING: f32 = 0.5;

/// Each entry's symbol: a scalar for every entry, or an array with one value per entry.
#[derive(Debug, Clone)]
pub struct SymbolSwatches {
    pub shape: ScalarOrArray<SymbolShape>,
    pub size: ScalarOrArray<f32>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub stroke_width: Option<f32>,
    pub fill: ScalarOrArray<ColorOrGradient>,
    pub fill_pattern: PatternFills,
    pub angle: ScalarOrArray<f32>,
}

impl Default for SymbolSwatches {
    fn default() -> Self {
        Self {
            shape: ScalarOrArray::new_scalar(Default::default()),
            size: ScalarOrArray::new_scalar(20.0),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            stroke_width: None,
            fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            fill_pattern: Default::default(),
            angle: ScalarOrArray::new_scalar(0.0),
        }
    }
}

impl SymbolSwatches {
    /// The encodings' lengths, which must agree where they are arrays.
    pub(crate) fn lengths(&self) -> [usize; 6] {
        [
            self.shape.len(),
            self.size.len(),
            self.stroke.len(),
            self.fill.len(),
            self.fill_pattern.len(),
            self.angle.len(),
        ]
    }

    /// One entry group per label, each at its own origin: a hit rect, its symbol centered in
    /// the widest symbol's column, and its label after the column.
    pub(crate) fn entries(
        &self,
        texts: &[String],
        text_padding: f32,
        style: &LegendStyle,
        engine: &LabelEngine,
    ) -> Vec<SceneGroup> {
        let len = texts.len();
        let symbol_mark = SceneSymbolMark {
            len: len as u32,
            shapes: self.shape.as_vec(len, None),
            shape_index: ScalarOrArray::new_array((0..len).collect()),
            size: self.size.clone(),
            stroke: self.stroke.clone(),
            angle: self.angle.clone(),
            fill: self.fill.clone(),
            fill_pattern: self.fill_pattern.clone(),
            stroke_width: self.stroke_width,
            x: 0.0.into(),
            y: 0.0.into(),
            ..Default::default()
        };
        let max_width = symbol_mark.bounding_box().width().round();
        texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                symbol_group(
                    &symbol_mark,
                    text,
                    index,
                    max_width,
                    text_padding,
                    style,
                    engine,
                )
            })
            .collect()
    }
}

/// An entry's row: its symbol centered in the widest symbol's column, its label after the
/// column, and a hit rect over both that spans the row's height.
fn symbol_group(
    symbols: &SceneSymbolMark,
    text: &str,
    index: usize,
    max_width: f32,
    text_padding: f32,
    style: &LegendStyle,
    engine: &LabelEngine,
) -> SceneGroup {
    let mut symbol = symbols.single_symbol_mark(index);
    let x = (max_width / 2.0).round();
    symbol.x = x.into();
    let symbol_height = symbol.bounding_box().height();
    let y = (symbol_height / 2.0 + ROW_PADDING).round();
    symbol.y = y.into();
    // A pattern anchors to its own swatch's nominal square, where mark-anchored patterns start
    // too, whatever the symbol's shape and stroke.
    let half = symbol
        .size_iter()
        .next()
        .map_or(0.0, |size| size.sqrt() / 2.0);
    let pattern_origin = symbol
        .fill_pattern_iter()
        .any(Option::is_some)
        .then_some([x - half, y - half]);
    let content = vec![
        SceneMark::Symbol(symbol).with_interactive(false),
        make_entry_label(text, max_width + text_padding, y, style),
    ];
    let bounds = SceneGroup {
        marks: content.clone(),
        ..Default::default()
    }
    .bounding_box(engine);
    let row_height = (symbol_height + 2.0 * ROW_PADDING).round();
    let hit = make_hit_rect(
        bounds.lower()[0].min(0.0),
        bounds.lower()[1].min(0.0),
        bounds.upper()[0],
        bounds.upper()[1].max(row_height),
    );
    SceneGroup {
        pattern_origin,
        marks: std::iter::once(hit).chain(content).collect(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use crate::legend::{GuideLegendOutput, Legend, LegendConfig, LegendSwatches};

    /// A symbol legend in one column, long enough for every entry.
    fn symbol_legend(text: ScalarOrArray<String>, swatches: SymbolSwatches) -> GuideLegendOutput {
        Legend::new(
            &LegendConfig {
                text,
                swatches: LegendSwatches::Symbols(swatches),
                ..Default::default()
            },
            &avenger_typst_label::bundled_label_engine(),
        )
        .expect("symbol legend renders")
        .marks(1_000.0)
    }

    use super::*;

    #[test]
    fn default_pattern_anchor_resolves_in_translated_legend_swatches() {
        use avenger_common::pattern::{PatternFill, StripeLayer};
        use avenger_scenegraph::{
            render::{
                order::{SceneDisplayList, SceneDisplayMark},
                pattern::{build_layered_pattern_geometry, PatternRect, PatternRenderContext},
            },
            scene_graph::SceneGraph,
        };
        let mut legend = symbol_legend(
            vec!["A".to_string(), "B".to_string()].into(),
            SymbolSwatches {
                size: 100.0.into(),
                shape: SymbolShape::from_vega_str("square").unwrap().into(),
                fill_pattern: vec![2.0, 4.0]
                    .into_iter()
                    .map(|phase| {
                        let mut stripe = StripeLayer::new(0.0, 8.0, 2.0);
                        stripe.phase = phase;
                        Some(PatternFill {
                            layers: vec![stripe.into()],
                            ..Default::default()
                        })
                    })
                    .collect::<Vec<_>>()
                    .into(),
                ..Default::default()
            },
        )
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
                let half = mark.size_iter().next().unwrap().sqrt() / 2.0;
                let x = mark.x_iter().next().unwrap() + item.origin[0];
                let y = mark.y_iter().next().unwrap() + item.origin[1];
                assert_eq!(item.pattern_origin, [x - half, y - half]);
                let geometry = build_layered_pattern_geometry(
                    mark.fill_pattern_iter().next().unwrap().as_ref().unwrap(),
                    &PatternRenderContext {
                        group_origin: item.pattern_origin,
                        mark_origin: [host_bounds.x, host_bounds.y],
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
                phases.push(first.y - item.pattern_origin[1]);
            }
            assert_eq!(phases, [1.0, 3.0]);
        }
    }

    #[test]
    fn itemized_symbol_legend_reports_interactive_hit_rects() {
        let output = symbol_legend(
            ScalarOrArray::new_array(vec!["A".to_string(), "B".to_string()]),
            SymbolSwatches {
                fill: ScalarOrArray::new_array(vec![
                    ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0]),
                    ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]),
                ]),
                ..Default::default()
            },
        );

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
        let output = symbol_legend(
            ScalarOrArray::new_array(vec!["small".to_string(), "medium".to_string()]),
            SymbolSwatches {
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
        );

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
