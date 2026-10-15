pub mod colorbar;
pub mod line;
pub mod symbol;

use std::{collections::HashSet, sync::Arc};

use avenger_color::ColorOrGradient;
use avenger_common::types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode};
use avenger_scenegraph::marks::{
    group::SceneGroup,
    mark::SceneMark,
    rect::SceneRectMark,
    text::{text_label, text_style, SceneTextMark},
};
use avenger_typst_label::{LabelEngine, LabelOptions, TextStyle};

use crate::error::AvengerGuidesError;

/// A rendered legend entry: its label, the scene path of its group, and the path of the
/// transparent rect that takes its hits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideLegendItem {
    pub index: usize,
    pub label: String,
    pub group_path: Vec<usize>,
    pub hit_rect_path: Vec<usize>,
}

/// A rendered symbol or line legend with the entries that interactions can target.
#[derive(Debug, Clone)]
pub struct GuideLegendOutput {
    pub group: SceneGroup,
    pub items: Vec<GuideLegendItem>,
}

/// Legend appearance: its background, title, and labels. `LegendStyle::default()` is the default
/// look, and `..Default::default()` keeps it for the settings that aren't given.
#[derive(Debug, Clone)]
pub struct LegendStyle {
    pub background_fill: ColorOrGradient,
    /// Without it, the background has no outline.
    pub background_stroke: Option<ColorOrGradient>,
    pub background_corner_radius: f32,
    /// Space between the background's edges and the legend's content.
    pub background_padding: f32,
    pub title_color: [f32; 4],
    pub title_font_family: String,
    pub title_font_size: f32,
    pub title_font_weight: FontWeight,
    pub title_syntax: TextSyntaxMode,
    pub label_color: [f32; 4],
    pub label_font_family: String,
    pub label_font_size: f32,
    pub label_font_weight: FontWeight,
    pub label_syntax: TextSyntaxMode,
}

impl Default for LegendStyle {
    fn default() -> Self {
        Self {
            background_fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            background_stroke: None,
            background_corner_radius: 0.0,
            background_padding: 4.0,
            title_color: [0.173, 0.173, 0.173, 1.0],
            title_font_family: "sans-serif".to_string(),
            title_font_size: 12.0,
            title_font_weight: FontWeight::NORMAL,
            title_syntax: TextSyntaxMode::Plain,
            label_color: [0.235, 0.235, 0.235, 1.0],
            label_font_family: "sans-serif".to_string(),
            label_font_size: 11.0,
            label_font_weight: FontWeight(300),
            label_syntax: TextSyntaxMode::Plain,
        }
    }
}

impl LegendStyle {
    fn label_text_style(&self) -> TextStyle {
        text_style(
            &self.label_font_family,
            self.label_font_size,
            self.label_font_weight,
            FontStyle::default(),
            self.label_color,
        )
    }
}

/// The gap between a legend's title and its entries.
const TITLE_GAP: f32 = 4.0;

/// The legend's background at the group origin, behind its content. It is drawn, transparent
/// by default, so that the legend's size is the same whether the background shows or not, and
/// it never takes hits.
fn make_background(style: &LegendStyle, width: f32, height: f32) -> SceneMark {
    SceneMark::Rect(SceneRectMark {
        x: 0.0.into(),
        y: 0.0.into(),
        width: Some(width.round().into()),
        height: Some(height.round().into()),
        fill: style.background_fill.clone().into(),
        stroke: style
            .background_stroke
            .clone()
            .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]))
            .into(),
        stroke_width: if style.background_stroke.is_some() {
            1.0
        } else {
            0.0
        }
        .into(),
        corner_radius: style.background_corner_radius.into(),
        zindex: Some(0),
        interactive: Some(false),
        ..Default::default()
    })
}

/// The legend's title, at the top left of its content, and the space below it before the first
/// entry.
fn make_title(
    title: &str,
    style: &LegendStyle,
    text_engine: &LabelEngine,
) -> Result<(SceneMark, f32), AvengerGuidesError> {
    let font = text_style(
        &style.title_font_family,
        style.title_font_size,
        style.title_font_weight,
        FontStyle::default(),
        style.title_color,
    );
    let bounds = text_engine.bounds(&text_label(
        title,
        style.title_syntax,
        LabelOptions {
            text: font,
            ..Default::default()
        },
    ))?;
    let padding = style.background_padding;
    let mark = SceneTextMark {
        clip: false,
        text: title.to_string().into(),
        x: padding.into(),
        y: padding.into(),
        font_size: style.title_font_size.into(),
        font_weight: style.title_font_weight.into(),
        font: style.title_font_family.clone().into(),
        color: style.title_color.into(),
        align: TextAlign::Left.into(),
        baseline: TextBaseline::Top.into(),
        text_syntax: style.title_syntax,
        ..Default::default()
    };
    Ok((
        SceneMark::Text(Arc::new(mark)).with_interactive(false),
        (bounds.height + TITLE_GAP).round(),
    ))
}

/// An entry's label, left aligned and centered on `y`, which doesn't take hits.
fn make_entry_label(text: &str, x: f32, y: f32, style: &LegendStyle) -> SceneMark {
    SceneMark::Text(Arc::new(SceneTextMark {
        clip: false,
        text: text.to_string().into(),
        x: x.round().into(),
        y: y.into(),
        align: TextAlign::Left.into(),
        baseline: TextBaseline::Middle.into(),
        font_size: style.label_font_size.into(),
        font: style.label_font_family.clone().into(),
        font_weight: style.label_font_weight.into(),
        color: style.label_color.into(),
        text_syntax: style.label_syntax,
        ..Default::default()
    }))
    .with_interactive(false)
}

/// A transparent rect that takes an entry's hits.
fn make_hit_rect(x0: f32, y0: f32, x1: f32, y1: f32) -> SceneMark {
    SceneMark::Rect(SceneRectMark {
        x: x0.into(),
        y: y0.into(),
        width: Some((x1 - x0).into()),
        height: Some((y1 - y0).into()),
        fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
        stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
        stroke_width: 0.0.into(),
        ..Default::default()
    })
}

fn compute_encoding_length(lengths: &[usize]) -> Result<usize, AvengerGuidesError> {
    let lengths = lengths
        .iter()
        .cloned()
        .filter(|&len| len > 1)
        .collect::<HashSet<_>>();

    let len = if lengths.len() > 1 {
        return Err(AvengerGuidesError::InvalidLegendLength(lengths));
    } else if lengths.is_empty() {
        1
    } else {
        // Only one unique length greater than 1
        lengths.into_iter().next().unwrap_or(1)
    };

    Ok(len)
}
