//! Titles: a chart's title and an optional subtitle, on a side of the chart.

use std::sync::Arc;

use avenger_common::types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode};
use avenger_scenegraph::marks::{
    group::SceneGroup,
    mark::SceneMark,
    text::{text_label, text_style, SceneTextMark},
};
use avenger_typst_label::{LabelEngine, LabelOptions};

use crate::{
    error::AvengerGuidesError,
    guide::{excess, fits, LENGTH_TOLERANCE},
    GuideSide, PartExtent,
};

/// Where a title's lines align along its slab, in the direction that they read: from the left
/// above or below a chart, from the bottom on its left, and from the top on its right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TitleAlign {
    #[default]
    Start,
    Center,
    End,
}

/// How a title or a subtitle draws its text.
#[derive(Debug, Clone, PartialEq)]
pub struct TitleTextStyle {
    pub font_family: String,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub color: [f32; 4],
    /// How the text reads: plain, plain with newlines that end lines, or Typst markup.
    pub syntax: TextSyntaxMode,
}

/// How a title draws its title and its subtitle.
#[derive(Debug, Clone, PartialEq)]
pub struct TitleConfig {
    /// The side of the chart that the title stands on. Beside a left or a right chart, the
    /// title reads along the side, up on the left and down on the right.
    pub side: GuideSide,
    pub align: TitleAlign,
    pub title: TitleTextStyle,
    pub subtitle: TitleTextStyle,
    /// The space between the title and the subtitle, finite and non-negative.
    pub gap: f32,
}

impl Default for TitleConfig {
    fn default() -> Self {
        Self {
            side: GuideSide::Top,
            align: TitleAlign::default(),
            title: TitleTextStyle {
                font_family: "sans-serif".to_string(),
                font_size: 13.0,
                font_weight: FontWeight::BOLD,
                font_style: FontStyle::default(),
                color: [0.0, 0.0, 0.0, 1.0],
                syntax: TextSyntaxMode::default(),
            },
            subtitle: TitleTextStyle {
                font_family: "sans-serif".to_string(),
                font_size: 11.0,
                font_weight: FontWeight::default(),
                font_style: FontStyle::default(),
                color: [0.35, 0.35, 0.35, 1.0],
                syntax: TextSyntaxMode::default(),
            },
            gap: 2.0,
        }
    }
}

/// A title prepared from its text, a subtitle, a configuration and a text engine, without a
/// length.
///
/// It typesets the title and the subtitle once. The title reads above the subtitle, so the
/// title is the outer line on the top, the left and the right of a chart, and the inner line
/// below it. A title does not wrap, so it is as thick at every length, and a line longer than
/// the length reaches past the slab on the side that the alignment leaves open.
#[derive(Debug)]
pub struct Title {
    config: TitleConfig,
    /// The title, then the subtitle, without the ones that are empty.
    lines: Vec<Line>,
}

/// A title or a subtitle, typeset.
#[derive(Debug, Clone)]
struct Line {
    text: String,
    style: TitleTextStyle,
    width: f32,
    height: f32,
    /// How far the line's math reaches above its box and below it.
    overhang: [f32; 2],
}

impl Line {
    /// The height of the line's box and its overhang.
    fn full_height(&self) -> f32 {
        self.overhang[0] + self.height + self.overhang[1]
    }
}

impl Title {
    /// A title of `text` with `subtitle` below it. An empty text or subtitle draws nothing, and
    /// invalid markup measures and draws as its literal text, as every label does. It fails for
    /// a gap that isn't finite and non-negative, and for a text or a subtitle that the engine
    /// cannot typeset.
    pub fn new(
        text: &str,
        subtitle: Option<&str>,
        config: &TitleConfig,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesError> {
        if !(config.gap.is_finite() && config.gap >= 0.0) {
            return Err(AvengerGuidesError::TitleGap(config.gap));
        }
        let typeset = |text: &str, style: &TitleTextStyle| -> Result<Line, AvengerGuidesError> {
            let options = LabelOptions {
                text: text_style(
                    &style.font_family,
                    style.font_size,
                    style.font_weight,
                    style.font_style,
                    style.color,
                ),
                ..Default::default()
            };
            let bounds = engine.bounds(&text_label(text, style.syntax, options))?;
            Ok(Line {
                text: text.to_string(),
                style: style.clone(),
                width: bounds.width,
                height: bounds.height,
                overhang: bounds.overhang,
            })
        };
        let lines = [
            (text, &config.title),
            (subtitle.unwrap_or(""), &config.subtitle),
        ]
        .into_iter()
        .filter(|(text, _)| !text.is_empty())
        .map(|(text, style)| typeset(text, style))
        .collect::<Result<_, _>>()?;
        Ok(Self {
            config: config.clone(),
            lines,
        })
    }

    /// A content key for the title's slab in a layout: equal for titles with the same text,
    /// subtitle and configuration, which draw the same at every length, and different otherwise.
    pub fn content_key(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let texts: Vec<&str> = self.lines.iter().map(|line| line.text.as_str()).collect();
        format!("{texts:?}{:?}", self.config).hash(&mut hasher);
        format!("title-{:016x}", hasher.finish())
    }

    /// The title's thickness and overhang at `length`, and the lengths where they hold.
    ///
    /// The thickness is the height of the title, the gap and the subtitle's height, at every
    /// length, where the height of a line with math includes how far its math reaches past its
    /// box. A line longer than `length` reaches past the slab's end when the title aligns
    /// to its start, past its start when it aligns to its end, and half past each when it
    /// centers, and the title reports that as overhang, at `length` only. Otherwise it holds
    /// from the shortest length that fits its longest line. A title without text is empty at
    /// every length.
    pub fn extent(&self, length: f32) -> PartExtent {
        if self.lines.is_empty() {
            return PartExtent::empty();
        }
        let width = self.width();
        let excess = excess(width, length);
        // The overhang past the start and the end of the reading direction.
        let reading = match self.config.align {
            TitleAlign::Start => [0.0, excess],
            TitleAlign::Center => [excess / 2.0; 2],
            TitleAlign::End => [excess, 0.0],
        };
        // A title on the left reads up, from the slab's end.
        let overhang = match self.config.side {
            GuideSide::Left => [reading[1], reading[0]],
            _ => reading,
        };
        PartExtent {
            thickness: self.thickness(),
            overhang,
            holds: if fits(width, length) {
                (width - LENGTH_TOLERANCE)..=f32::INFINITY
            } else {
                length..=length
            },
        }
    }

    /// The origin for the title in the slab with rectangle `slot`, `[x, y, width, height]`:
    /// where the slab's inner edge, nearest the chart, meets its start.
    pub fn origin(&self, slot: [f32; 4]) -> [f32; 2] {
        self.config.side.slot_origin(slot)
    }

    /// The title at `length`, in its slab's frame, where 0 along the length is the slab's
    /// start and 0 across it is the inner edge, nearest the chart, or `None` without text.
    pub fn part(&self, length: f32) -> Option<SceneGroup> {
        if self.lines.is_empty() {
            return None;
        }
        let side = self.config.side;
        let (reading, align) = match self.config.align {
            TitleAlign::Start => (0.0, TextAlign::Left),
            TitleAlign::Center => (length / 2.0, TextAlign::Center),
            TitleAlign::End => (length, TextAlign::Right),
        };
        let along = match side {
            GuideSide::Left => length - reading,
            _ => reading,
        };
        let angle = match side {
            GuideSide::Left => -90.0,
            GuideSide::Right => 90.0,
            GuideSide::Top | GuideSide::Bottom => 0.0,
        };
        // The lines stack from the top of the text, which faces away from the chart on every
        // side but the bottom.
        let thickness = self.thickness();
        let mut top = 0.0;
        let marks = self
            .lines
            .iter()
            .map(|line| {
                let middle = top + line.overhang[0] + line.height / 2.0;
                top += line.full_height() + self.config.gap;
                let across = match side {
                    GuideSide::Bottom => middle,
                    _ => thickness - middle,
                };
                let [x, y] = side.point(along, across);
                let style = &line.style;
                SceneMark::Text(Arc::new(SceneTextMark {
                    len: 1,
                    clip: false,
                    text: line.text.clone().into(),
                    text_syntax: style.syntax,
                    x: x.into(),
                    y: y.into(),
                    align: align.into(),
                    baseline: TextBaseline::Middle.into(),
                    angle: angle.into(),
                    font: style.font_family.clone().into(),
                    font_size: style.font_size.into(),
                    font_weight: style.font_weight.into(),
                    font_style: style.font_style.into(),
                    color: style.color.into(),
                    ..Default::default()
                }))
            })
            .collect();
        Some(SceneGroup {
            marks,
            interactive: Some(false),
            ..Default::default()
        })
    }

    /// The title on its side of a chart of `size`, inside it, for a chart without a layout,
    /// which leaves the title's thickness free on that side.
    pub fn marks(&self, size: [f32; 2]) -> SceneGroup {
        let side = self.config.side;
        let length = if side.is_vertical() { size[1] } else { size[0] };
        let Some(mut group) = self.part(length) else {
            return SceneGroup::default();
        };
        let thickness = self.thickness();
        let slot = match side {
            GuideSide::Top => [0.0, 0.0, size[0], thickness],
            GuideSide::Bottom => [0.0, size[1] - thickness, size[0], thickness],
            GuideSide::Left => [0.0, 0.0, thickness, size[1]],
            GuideSide::Right => [size[0] - thickness, 0.0, thickness, size[1]],
        };
        group.origin = self.origin(slot);
        group
    }

    /// The heights of the lines, with the overhang of their math, and the gap between them.
    fn thickness(&self) -> f32 {
        let heights: f32 = self.lines.iter().map(Line::full_height).sum();
        heights + self.config.gap * self.lines.len().saturating_sub(1) as f32
    }

    /// The length of the longest line.
    fn width(&self) -> f32 {
        self.lines.iter().map(|line| line.width).fold(0.0, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(subtitle: Option<&str>, config: &TitleConfig) -> Title {
        Title::new("Penguins", subtitle, config, &crate::test_engine()).unwrap()
    }

    #[test]
    fn the_gap_must_be_finite_and_non_negative() {
        for gap in [-1.0, f32::NAN, f32::INFINITY] {
            let config = TitleConfig {
                gap,
                ..Default::default()
            };
            assert!(matches!(
                Title::new("Penguins", None, &config, &crate::test_engine()),
                Err(AvengerGuidesError::TitleGap(_))
            ));
        }
    }

    #[test]
    fn a_title_without_text_is_empty() {
        let empty =
            Title::new("", Some(""), &TitleConfig::default(), &crate::test_engine()).unwrap();
        assert_eq!(empty.extent(100.0), PartExtent::empty());
        assert!(empty.part(100.0).is_none());
        assert!(empty.marks([100.0, 100.0]).marks.is_empty());
    }

    #[test]
    fn an_empty_title_keeps_its_subtitle_without_a_gap() {
        let config = TitleConfig::default();
        let subtitle = Title::new("", Some("Palmer"), &config, &crate::test_engine()).unwrap();
        assert_eq!(subtitle.lines.len(), 1);
        assert_eq!(subtitle.extent(400.0).thickness, subtitle.lines[0].height);
    }

    #[test]
    fn content_keys_follow_the_text_and_the_configuration() {
        let config = TitleConfig::default();
        let key = |subtitle, config: &TitleConfig| title(subtitle, config).content_key();
        assert_eq!(key(None, &config), key(None, &config));
        assert_ne!(key(None, &config), key(Some("Palmer"), &config));
        let centered = TitleConfig {
            align: TitleAlign::Center,
            ..Default::default()
        };
        assert_ne!(key(None, &config), key(None, &centered));
    }

    #[test]
    fn marks_stand_inside_the_chart_on_each_side() {
        let size = [400.0, 300.0];
        for side in [
            GuideSide::Top,
            GuideSide::Bottom,
            GuideSide::Left,
            GuideSide::Right,
        ] {
            let config = TitleConfig {
                side,
                ..Default::default()
            };
            let title = title(Some("Palmer"), &config);
            let t = title.extent(400.0).thickness;
            let origin = match side {
                GuideSide::Top => [0.0, t],
                GuideSide::Bottom => [0.0, 300.0 - t],
                GuideSide::Left => [t, 0.0],
                GuideSide::Right => [400.0 - t, 0.0],
            };
            assert_eq!(title.marks(size).origin, origin, "{side:?}");
        }
    }
}
