use std::sync::Arc;

use avenger_color::ColorOrGradient;
use avenger_common::{types::SymbolShape, value::ScalarOrArray};
use avenger_geometry::{marks::MarkGeometryUtils, rtree::EnvelopeUtils};
use avenger_scenegraph::marks::{
    group::SceneGroup, mark::SceneMark, rect::SceneRectMark, symbol::SceneSymbolMark,
    text::SceneTextMark,
};
use avenger_text::{
    default_text_engine,
    measurement::TextMeasurementConfig,
    types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode},
    LabelParams,
};

use crate::{
    error::AvengerGuidesError,
    legend::{compute_encoding_length, GuideLegendItem, GuideLegendOutput},
};

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
    pub angle: ScalarOrArray<f32>,

    /// Width of the chart area that the legend may be placed next to
    pub inner_width: f32,

    /// Height of the chart area that the legend may be placed next to
    pub inner_height: f32,

    /// Margin around the legend, separating it from the chart area
    pub outer_margin: f32,

    /// Padding between the symbol and the text
    pub text_padding: f32,

    /// Background rect styling
    pub background_fill: Option<ColorOrGradient>,
    pub background_stroke: Option<ColorOrGradient>,
    pub background_corner_radius: Option<f32>,
    pub background_padding: Option<f32>,

    /// Text colors
    pub title_color: Option<[f32; 4]>,
    pub label_color: Option<[f32; 4]>,

    /// Typography configuration for title
    pub title_font_family: Option<String>,
    pub title_font_size: Option<f32>,
    pub title_font_weight: Option<FontWeight>,
    pub title_syntax_mode: TextSyntaxMode,
    pub title_text_params: LabelParams,

    /// Typography configuration for labels
    pub label_font_family: Option<String>,
    pub label_font_size: Option<f32>,
    pub label_font_weight: Option<FontWeight>,
    pub label_syntax_mode: TextSyntaxMode,
    pub label_text_params: LabelParams,
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
            background_fill: None,
            background_stroke: None,
            background_corner_radius: None,
            background_padding: None,
            title_color: None,
            label_color: None,
            title_font_family: None,
            title_font_size: None,
            title_font_weight: None,
            title_syntax_mode: TextSyntaxMode::Plain,
            title_text_params: LabelParams::default(),
            label_font_family: None,
            label_font_size: None,
            label_font_weight: None,
            label_syntax_mode: TextSyntaxMode::Plain,
            label_text_params: LabelParams::default(),
        }
    }
}

pub fn make_symbol_legend(config: &SymbolLegendConfig) -> Result<SceneGroup, AvengerGuidesError> {
    Ok(make_symbol_legend_itemized(config)?.group)
}

pub fn make_symbol_legend_with_text_engine(
    config: &SymbolLegendConfig,
    text_engine: &avenger_text::TextEngine,
) -> Result<SceneGroup, AvengerGuidesError> {
    Ok(make_symbol_legend_itemized_with_text_engine(config, text_engine)?.group)
}

pub fn make_symbol_legend_itemized(
    config: &SymbolLegendConfig,
) -> Result<GuideLegendOutput, AvengerGuidesError> {
    make_symbol_legend_itemized_with_text_engine(config, &default_text_engine())
}

pub fn make_symbol_legend_itemized_with_text_engine(
    config: &SymbolLegendConfig,
    text_engine: &avenger_text::TextEngine,
) -> Result<GuideLegendOutput, AvengerGuidesError> {
    // Compute the common encoding length
    let len = compute_encoding_length(&[
        config.text.len(),
        config.shape.len(),
        config.size.len(),
        config.stroke.len(),
        config.fill.len(),
        config.angle.len(),
    ])?;

    tracing::debug!(
        len = len,
        stroke_width = ?config.stroke_width,
        text_len = config.text.len(),
        text_values = ?config.text.as_vec(len, None),
        "make_symbol_legend"
    );

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

    let max_width = symbol_mark
        .bounding_box_with_text_engine(text_engine)
        .width()
        .round();
    let center_x = (max_width / 2.0).round();

    // Use fixed 4px padding for top/bottom
    let vertical_padding: f32 = 4.0;
    let horizontal_padding = config.background_padding.unwrap_or(4.0);

    // Position legend content with padding from the background rect origin
    // Round to pixel boundaries for crisp rendering
    let content_offset_x = (horizontal_padding + config.outer_margin).round();
    let mut content_offset_y = vertical_padding.round();

    let mut groups: Vec<SceneMark> = Vec::with_capacity(len + 1); // +1 for potential title

    // Add title if present (restore previous measurement-based Top baseline placement)
    if let Some(ref title_text) = config.title {
        let title_font_size = config.title_font_size.unwrap_or(12.0);
        let title_font = config
            .title_font_family
            .clone()
            .unwrap_or_else(|| "sans-serif".to_string());
        let title_font_weight = config
            .title_font_weight
            .unwrap_or(FontWeight::Number(400.0));

        // Measure the actual title text height
        let title_config = TextMeasurementConfig {
            text: title_text,
            font: &title_font,
            font_size: title_font_size,
            font_weight: title_font_weight,
            font_style: FontStyle::Normal,
            syntax_mode: config.title_syntax_mode,
            params: &config.title_text_params,
            number_locale: None,
            number_locale_specs: None,
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: None,
        };
        let title_bounds = text_engine.measure_bounds(&title_config)?;

        // Use Top baseline and position title at vertical padding from top
        let title_y = vertical_padding;
        let title_mark = SceneTextMark {
            clip: false,
            text: title_text.clone().into(),
            x: content_offset_x.into(),
            y: title_y.into(),
            font_size: title_font_size.into(),
            font_weight: title_font_weight.into(),
            font: title_font.into(),
            color: ColorOrGradient::Color(config.title_color.unwrap_or([0.173, 0.173, 0.173, 1.0]))
                .into(),
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Top.into(),
            text_syntax: config.title_syntax_mode,
            text_params: config.title_text_params.clone(),
            ..Default::default()
        };
        groups.push(SceneMark::Text(Arc::new(title_mark)).with_interactive(false));

        // Advance content offset by measured height plus a small gap (2px)
        content_offset_y = (title_y + title_bounds.height + 2.0).round();
    }

    let mut y = content_offset_y.round();

    let text_strs = config.text.as_vec(len, None);
    let mut items = Vec::with_capacity(len);

    for (i, text_str) in text_strs.iter().enumerate() {
        let group_path = vec![groups.len() + 1];
        let group = make_symbol_group(
            &symbol_mark,
            text_str,
            center_x,
            config.text_padding,
            max_width,
            i,
            [(config.inner_width + content_offset_x).round(), y.round()],
            config.label_color,
            config.label_font_family.as_deref(),
            config.label_font_size,
            config.label_font_weight.as_ref(),
            config.label_syntax_mode,
            &config.label_text_params,
            text_engine,
        )?;
        let height = group.bounding_box_with_text_engine(text_engine).height();
        if i == 0 {
            tracing::debug!(height = height, "First symbol group height");
        }
        groups.push(SceneMark::Group(group));
        items.push(GuideLegendItem {
            index: i,
            label: text_str.clone(),
            hit_rect_path: vec![group_path[0], 0],
            group_path,
        });
        // Round y position after adding height to stay on pixel boundaries
        y = (y + height).round();
    }

    // Measure the content bounds
    let temp_group = SceneGroup {
        marks: groups.clone(),
        ..Default::default()
    };
    let content_bbox = temp_group.bounding_box_with_text_engine(text_engine);

    // Calculate total dimensions including padding
    // The background rect always exists and defines our coordinate system
    // Use fixed vertical padding and configurable horizontal padding
    // Round to pixel boundaries for crisp rendering
    let bg_width = (content_bbox.width() + horizontal_padding * 2.0).round(); // Left + right padding
    let bg_height = (content_bbox.height() + vertical_padding * 2.0).round(); // Fixed 4px top + 4px bottom

    // Always create a background rect at origin (0, 0)
    // This provides consistent layout whether visible or not
    let bg = SceneRectMark {
        x: 0.0.into(),
        y: 0.0.into(),
        width: Some(bg_width.into()),
        height: Some(bg_height.into()),
        fill: config
            .background_fill
            .clone()
            .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])) // Transparent by default
            .into(),
        stroke: config
            .background_stroke
            .clone()
            .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])) // No stroke by default
            .into(),
        stroke_width: if config.background_stroke.is_some() {
            1.0.into()
        } else {
            0.0.into()
        },
        corner_radius: config.background_corner_radius.unwrap_or(0.0).into(),
        zindex: Some(0), // Background should be behind content
        interactive: false,
        ..Default::default()
    };

    // Insert background rect first, then legend content
    let mut final_marks = vec![SceneMark::Rect(bg)];
    final_marks.extend(groups);

    Ok(GuideLegendOutput {
        group: SceneGroup {
            marks: final_marks,
            clip: avenger_scenegraph::marks::group::Clip::None,
            ..Default::default()
        },
        items,
        continuous_surfaces: Vec::new(),
    })
}

#[allow(clippy::too_many_arguments)]
fn make_symbol_group(
    symbols_mark: &SceneSymbolMark,
    text: &str,
    center_x: f32,
    text_padding: f32,
    max_width: f32,
    index: usize,
    origin: [f32; 2],
    label_color: Option<[f32; 4]>,
    label_font_family: Option<&str>,
    label_font_size: Option<f32>,
    label_font_weight: Option<&FontWeight>,
    label_syntax_mode: TextSyntaxMode,
    label_text_params: &LabelParams,
    text_engine: &avenger_text::TextEngine,
) -> Result<SceneGroup, AvengerGuidesError> {
    //
    let mut single_symbol_mark = symbols_mark.single_symbol_mark(index);
    single_symbol_mark.x = center_x.into();

    if index == 0 && text.len() > 3 {
        // Debug symbol size for shape legend
        let sizes = single_symbol_mark.size.as_vec(1, None);
        tracing::debug!(text = text, size = sizes[0], "Symbol size");
    }

    let padding = 0.5; // Further reduced vertical padding between legend items
    let bbox = single_symbol_mark.bounding_box_with_text_engine(text_engine);
    let symbol_height = bbox.height();

    if index == 0 && text.len() > 3 {
        // Only debug for shape legend (has longer text like "circle")
        tracing::debug!(text = text, height = symbol_height, "Symbol height");
    }

    single_symbol_mark.y = ((symbol_height / 2.0 + padding).round()).into();

    tracing::debug!(text = text, "Creating legend text mark");
    let text_mark = SceneTextMark {
        clip: false,
        text: text.to_string().into(),
        x: ((max_width + text_padding).round()).into(),
        y: single_symbol_mark.y.clone(),
        align: TextAlign::Left.into(),
        baseline: TextBaseline::Middle.into(),
        font_size: label_font_size.unwrap_or(11.0).into(),
        font: label_font_family.unwrap_or("sans-serif").to_string().into(),
        font_weight: label_font_weight
            .cloned()
            .unwrap_or(FontWeight::Number(300.0))
            .into(),
        color: ColorOrGradient::Color(label_color.unwrap_or([0.235, 0.235, 0.235, 1.0])).into(),
        text_syntax: label_syntax_mode,
        text_params: label_text_params.clone(),
        ..Default::default()
    };
    let content_marks = vec![
        SceneMark::Symbol(single_symbol_mark).with_interactive(false),
        SceneMark::Text(Arc::new(text_mark)).with_interactive(false),
    ];
    let content_bbox = SceneGroup {
        marks: content_marks.clone(),
        ..Default::default()
    }
    .bounding_box_with_text_engine(text_engine);
    let reserved_row_height = (symbol_height + padding * 2.0).round();
    let hit_x0 = content_bbox.lower()[0].min(0.0);
    let hit_y0 = content_bbox.lower()[1].min(0.0);
    let hit_x1 = content_bbox.upper()[0];
    let hit_y1 = content_bbox.upper()[1].max(reserved_row_height);

    Ok(SceneGroup {
        origin,
        marks: std::iter::once(
            // Transparent hit rect for interactions. Its row height preserves the
            // pre-itemized legend spacing that used an invisible row rect.
            SceneMark::Rect(SceneRectMark {
                x: hit_x0.into(),
                y: hit_y0.into(),
                width: Some((hit_x1 - hit_x0).into()),
                height: Some((hit_y1 - hit_y0).into()),
                fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
                stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
                stroke_width: 0.0.into(),
                ..Default::default()
            }),
        )
        .chain(content_marks)
        .collect(),

        stroke_width: Some(1.0),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn itemized_symbol_legend_reports_interactive_hit_rects() {
        let output = make_symbol_legend_itemized(&SymbolLegendConfig {
            text: ScalarOrArray::new_array(vec!["A".to_string(), "B".to_string()]),
            fill: ScalarOrArray::new_array(vec![
                ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0]),
                ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]),
            ]),
            ..Default::default()
        })
        .expect("symbol legend renders");

        assert_eq!(output.items.len(), 2);
        assert_eq!(output.items[0].group_path, vec![1]);
        assert_eq!(output.items[0].hit_rect_path, vec![1, 0]);
        assert!(!output.group.marks[0].interactive());

        let SceneMark::Group(item_group) = &output.group.marks[1] else {
            panic!("legend item should be a group");
        };
        assert!(item_group.marks[0].interactive());
        assert!(!item_group.marks[1].interactive());
        assert!(!item_group.marks[2].interactive());
        let SceneMark::Rect(hit_rect) = &item_group.marks[0] else {
            panic!("first item mark should be hit rect");
        };
        assert!(hit_rect.width.is_some());
        assert!(hit_rect.height.is_some());
    }

    #[test]
    fn symbol_hit_rect_preserves_reserved_row_height() {
        let output = make_symbol_legend_itemized(&SymbolLegendConfig {
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
        })
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

    #[test]
    fn symbol_legend_forwards_title_and_label_text_params() {
        let mut title_text_params = LabelParams::default();
        title_text_params.insert(
            "series".to_string(),
            avenger_text::LabelParamValue::Str("Revenue".to_string()),
        );

        let mut label_text_params = LabelParams::default();
        label_text_params.insert("first".to_string(), avenger_text::LabelParamValue::Int(1));
        label_text_params.insert("second".to_string(), avenger_text::LabelParamValue::Int(2));

        let output = make_symbol_legend_itemized(&SymbolLegendConfig {
            title: Some("#series".to_string()),
            text: ScalarOrArray::new_array(vec!["#first".to_string(), "#second".to_string()]),
            fill: ScalarOrArray::new_array(vec![
                ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0]),
                ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]),
            ]),
            title_syntax_mode: TextSyntaxMode::TypstMarkup,
            title_text_params: title_text_params.clone(),
            label_syntax_mode: TextSyntaxMode::TypstMarkup,
            label_text_params: label_text_params.clone(),
            ..Default::default()
        })
        .expect("symbol legend renders");

        let text_marks = collect_text_marks(&output.group.marks);
        let title_mark = text_marks
            .iter()
            .find(|mark| mark.text.as_vec(1, None)[0] == "#series")
            .expect("title text mark");
        assert_eq!(title_mark.text_params, title_text_params);

        for label in ["#first", "#second"] {
            let label_mark = text_marks
                .iter()
                .find(|mark| mark.text.as_vec(1, None)[0] == label)
                .expect("label text mark");
            assert_eq!(label_mark.text_params, label_text_params);
        }
    }

    fn collect_text_marks(marks: &[SceneMark]) -> Vec<&SceneTextMark> {
        let mut text_marks = Vec::new();
        collect_text_marks_into(marks, &mut text_marks);
        text_marks
    }

    fn collect_text_marks_into<'a>(
        marks: &'a [SceneMark],
        text_marks: &mut Vec<&'a SceneTextMark>,
    ) {
        for mark in marks {
            match mark {
                SceneMark::Text(text) => text_marks.push(text),
                SceneMark::Group(group) => collect_text_marks_into(&group.marks, text_marks),
                _ => {}
            }
        }
    }
}
