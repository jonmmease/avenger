use std::sync::Arc;

use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};
use avenger_geometry::{marks::MarkGeometryUtils, rtree::EnvelopeUtils};
use avenger_scenegraph::marks::{
    group::SceneGroup,
    mark::SceneMark,
    rect::SceneRectMark,
    symbol::{SceneSymbolMark, SymbolShape},
    text::SceneTextMark,
};
use avenger_text::types::{TextAlign, TextBaseline};

use crate::{error::AvengerGuidesError, legend::compute_encoding_length};

/// Symbol legends
pub struct SymbolLegendConfig {
    pub title: Option<String>,
    pub text: ScalarOrArray<String>,
    pub shape: ScalarOrArray<SymbolShape>,
    pub size: ScalarOrArray<f32>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub stroke_width: Option<f32>,
    pub fill: ScalarOrArray<ColorOrGradient>,
    pub angle: ScalarOrArray<f32>,

    /// Width of the chart area that the legend may be placed next to
    pub inner_width: f32,

    /// Height of the chart area that the legend may be placed next to
    pub inner_height: f32,

    /// Margin around the legend, separating it from the chart area
    pub outer_margin: f32,

    /// Padding between the symbol and the text
    pub text_padding: f32,
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
            angle: ScalarOrArray::new_scalar(0.0),
            inner_width: 100.0,
            inner_height: 100.0,

            outer_margin: 4.0,
            text_padding: 2.0,
        }
    }
}

pub fn make_symbol_legend(config: &SymbolLegendConfig) -> Result<SceneGroup, AvengerGuidesError> {
    // Compute the common encoding length
    let len = compute_encoding_length(&[
        config.text.len(),
        config.shape.len(),
        config.size.len(),
        config.stroke.len(),
        config.fill.len(),
        config.angle.len(),
    ])?;

    // Compute the max width of all marks so that we can align the text next to them.
    let symbol_mark = SceneSymbolMark {
        len: len as u32,
        // Handle shapes and shape_index
        shapes: config.shape.as_vec(len, None),
        shape_index: ScalarOrArray::new_array((0..len).collect()),

        // Direct clone from config
        size: config.size.clone(),
        stroke: config.stroke.clone(),
        angle: config.angle.clone(),
        fill: config.fill.clone(),

        // Scalars
        stroke_width: config.stroke_width,

        // x and y or zero
        x: 0.0.into(),
        y: 0.0.into(),

        ..Default::default()
    };

    let max_width = symbol_mark.bounding_box().width();
    let center_x = max_width / 2.0;

    let mut groups: Vec<SceneMark> = Vec::with_capacity(len);
    let mut y = 0.0;

    let text_strs = config.text.as_vec(len, None);

    for i in 0..len {
        let group = make_symbol_group(
            &symbol_mark,
            &text_strs[i],
            center_x,
            config.text_padding,
            max_width,
            i,
            [config.inner_width + config.outer_margin, y],
        );
        let height = group.bounding_box().height();
        groups.push(SceneMark::Group(group));
        y += height;
    }

    return Ok(SceneGroup {
        marks: groups,
        ..Default::default()
    });
}

fn make_symbol_group(
    symbols_mark: &SceneSymbolMark,
    text: &str,
    center_x: f32,
    text_padding: f32,
    max_width: f32,
    index: usize,
    origin: [f32; 2],
) -> SceneGroup {
    //
    let mut single_symbol_mark = symbols_mark.single_symbol_mark(index);
    single_symbol_mark.x = center_x.into();

    let padding = 2.0;
    let bbox = single_symbol_mark.bounding_box();
    let symbol_height = bbox.height();
    let symbol_width = bbox.width();

    single_symbol_mark.y = (symbol_height / 2.0 + padding).into();

    let text_mark = SceneTextMark {
        text: text.to_string().into(),
        x: (max_width + text_padding).into(),
        y: single_symbol_mark.y.clone(),
        align: TextAlign::Left.into(),
        baseline: TextBaseline::Middle.into(),
        font_size: 10.0.into(),
        ..Default::default()
    };

    let group = SceneGroup {
        origin,
        marks: vec![
            // Rect is so that bounding box calculations on group are correct
            SceneMark::Rect(SceneRectMark {
                x: 0.0.into(),
                y: 0.0.into(),
                width: Some(symbol_width.into()),
                height: Some((symbol_height + padding * 2.0).into()),
                stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
                stroke_width: 0.1.into(),
                ..Default::default()
            }),
            SceneMark::Symbol(single_symbol_mark),
            SceneMark::Text(Arc::new(text_mark)),
        ],

        stroke_width: Some(1.0),
        ..Default::default()
    };
    group
}
