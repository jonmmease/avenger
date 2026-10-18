//! Legends and colorbars prepared once and drawn at any length.

mod arrange;
pub(crate) use arrange::{arrange, Holds};
mod colorbar;
mod line;
mod prepared;
mod symbol;

use std::{collections::HashSet, num::NonZeroUsize, sync::Arc};

use avenger_common::value::ScalarOrArray;

pub use colorbar::{Colorbar, ColorbarConfig, ColorbarOutput};
pub use line::LineSwatches;
pub use prepared::Legend;
pub use symbol::SymbolSwatches;

use avenger_color::ColorOrGradient;
use avenger_common::types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode};
use avenger_scenegraph::marks::{
    group::SceneGroup,
    mark::SceneMark,
    rect::SceneRectMark,
    text::{text_label, text_style, SceneTextMark},
};
use avenger_typst_label::{LabelEngine, LabelOptions};

use crate::{error::AvengerGuidesError, BackgroundStyle, GuideSide};

/// A symbol or line legend's settings: its title, each entry's label and swatch, and how the
/// entries wrap on the legend's side.
#[derive(Debug, Clone)]
pub struct LegendConfig {
    /// The title above the entries, or `None` for a legend without one.
    pub title: Option<String>,
    /// Each entry's label: a scalar for every entry, or an array with one value per entry.
    pub text: ScalarOrArray<String>,
    /// Each entry's symbol or line.
    pub swatches: LegendSwatches,
    /// The space between a swatch and its label.
    pub text_padding: f32,
    /// The legend's side and how its entries wrap there.
    pub layout: LegendLayout,
    /// The title and label fonts, the gaps between entries, and the background.
    pub style: LegendStyle,
}

impl Default for LegendConfig {
    fn default() -> Self {
        Self {
            title: None,
            text: ScalarOrArray::new_scalar(String::new()),
            swatches: LegendSwatches::default(),
            text_padding: 2.0,
            layout: LegendLayout::default(),
            style: LegendStyle::default(),
        }
    }
}

/// Each entry's swatch: a symbol or a line segment.
#[derive(Debug, Clone)]
pub enum LegendSwatches {
    /// A symbol before each label.
    Symbols(SymbolSwatches),
    /// A line segment before each label.
    Lines(LineSwatches),
}

impl Default for LegendSwatches {
    fn default() -> Self {
        Self::Symbols(SymbolSwatches::default())
    }
}

/// The legend's side, and how its entries wrap along it.
///
/// Lines of entries run along the side: across the slab's width on a top or bottom side, and
/// down its height on a left or right side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegendLayout {
    /// The side of the plot that the legend stands on.
    pub side: GuideSide,
    /// How the entries fill the lines along the side.
    pub wrap: LegendWrap,
}

impl Default for LegendLayout {
    /// Beside the plot, in as many rows of equal cells as fit the plot's height.
    fn default() -> Self {
        Self {
            side: GuideSide::Right,
            wrap: LegendWrap::Grid { columns: None },
        }
    }
}

/// How a legend's entries fill the lines along its side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegendWrap {
    /// Each entry at its own size, with a line breaking before an entry that would pass the
    /// length, as avenger-layout's `Flow` packs its children.
    Flow,
    /// Equal cells, as wide as the widest entry and as tall as the tallest, filled along the
    /// side first. `columns` fixes the column count whatever the length, and without it the
    /// legend takes as many cells along the side as fit.
    Grid { columns: Option<NonZeroUsize> },
}

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
    pub background: BackgroundStyle,
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
    /// The space between entries across the legend: between columns.
    pub column_gap: f32,
    /// The space between entries down the legend: between rows.
    pub row_gap: f32,
}

impl Default for LegendStyle {
    fn default() -> Self {
        Self {
            background: BackgroundStyle::default(),
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
            column_gap: 10.0,
            row_gap: 0.0,
        }
    }
}

/// The gap between a legend's title and its entries.
const TITLE_GAP: f32 = 4.0;

/// The legend's background at the group origin, behind its content. It is drawn, transparent
/// by default, so that the legend's size is the same whether the background shows or not, and
/// it never takes hits.
pub(crate) fn make_background(style: &BackgroundStyle, width: f32, height: f32) -> SceneMark {
    SceneMark::Rect(SceneRectMark {
        x: 0.0.into(),
        y: 0.0.into(),
        width: Some(width.round().into()),
        height: Some(height.round().into()),
        fill: style.fill.clone().into(),
        stroke: style
            .stroke
            .clone()
            .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]))
            .into(),
        stroke_width: if style.stroke.is_some() { 1.0 } else { 0.0 }.into(),
        corner_radius: style.corner_radius.into(),
        zindex: Some(0),
        interactive: Some(false),
        ..Default::default()
    })
}

/// The font of a legend's or colorbar's title.
pub(crate) struct TitleFont<'a> {
    pub(crate) family: &'a str,
    pub(crate) size: f32,
    pub(crate) weight: FontWeight,
    pub(crate) color: [f32; 4],
    pub(crate) syntax: TextSyntaxMode,
}

impl LegendStyle {
    fn title_font(&self) -> TitleFont<'_> {
        TitleFont {
            family: &self.title_font_family,
            size: self.title_font_size,
            weight: self.title_font_weight,
            color: self.title_color,
            syntax: self.title_syntax,
        }
    }
}

/// The title above a legend's entries or a colorbar's gradient.
#[derive(Debug)]
pub(crate) struct Title {
    /// The title as a text mark at the origin.
    pub(crate) mark: SceneMark,
    pub(crate) width: f32,
    /// The title's height and the space below it.
    pub(crate) block: f32,
}

/// Typesets `title` in `font`, or fails when the title does not typeset.
pub(crate) fn make_title(
    title: &str,
    font: &TitleFont,
    text_engine: &LabelEngine,
) -> Result<Title, AvengerGuidesError> {
    let style = text_style(
        font.family,
        font.size,
        font.weight,
        FontStyle::default(),
        font.color,
    );
    let bounds = text_engine.bounds(&text_label(
        title,
        font.syntax,
        LabelOptions {
            text: style,
            ..Default::default()
        },
    ))?;
    let mark = SceneTextMark {
        clip: false,
        text: title.to_string().into(),
        font_size: font.size.into(),
        font_weight: font.weight.into(),
        font: font.family.to_string().into(),
        color: font.color.into(),
        align: TextAlign::Left.into(),
        baseline: TextBaseline::Top.into(),
        text_syntax: font.syntax,
        ..Default::default()
    };
    Ok(Title {
        mark: SceneMark::Text(Arc::new(mark)).with_interactive(false),
        width: bounds.width,
        block: (bounds.height + TITLE_GAP).round(),
    })
}

/// The title as a group at `[padding, padding]` inside a background, which takes no hits.
fn title_group(title: &Title, padding: f32) -> SceneMark {
    SceneMark::Group(SceneGroup {
        origin: [padding, padding],
        marks: vec![title.mark.clone()],
        interactive: Some(false),
        ..Default::default()
    })
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
