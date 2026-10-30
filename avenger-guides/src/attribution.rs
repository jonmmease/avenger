//! Attributions: the credits for a chart's data and map tiles, in small text outside the plot.

use std::sync::Arc;

use avenger_common::types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode};
use avenger_scenegraph::marks::{
    group::SceneGroup,
    mark::SceneMark,
    text::{text_label, text_style, SceneTextMark},
};
use avenger_typst_label::{BoxAnchor, LabelEngine, LabelOptions};

use crate::{
    error::AvengerGuidesError,
    guide::excess,
    legend::{arrange, make_background, Holds, LegendWrap},
    BackgroundStyle, GuideSide, PartExtent,
};

/// Which end of its length an attribution's lines align to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttributionAlign {
    Start,
    /// The end, as web maps and figure captions place credits.
    #[default]
    End,
}

/// How an attribution draws its credits.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributionConfig {
    /// The side of the plot that the attribution stands on: the top or the bottom.
    pub side: GuideSide,
    pub align: AttributionAlign,
    /// The Typst markup between two credits on a line.
    pub separator: String,
    /// The space on each side of the separator, finite and non-negative.
    pub separator_space: f32,
    pub font_family: String,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub color: [f32; 4],
    /// The space between lines, finite and non-negative.
    pub line_gap: f32,
    /// The box behind the credits, transparent by default. Its padding, finite and
    /// non-negative, is the space around them in a slab and in the corner of a plot.
    pub background: BackgroundStyle,
}

impl Default for AttributionConfig {
    fn default() -> Self {
        Self {
            side: GuideSide::Bottom,
            align: AttributionAlign::default(),
            separator: "·".to_string(),
            separator_space: 4.0,
            font_family: "sans-serif".to_string(),
            font_size: 9.0,
            font_weight: FontWeight::default(),
            color: [0.35, 0.35, 0.35, 1.0],
            line_gap: 2.0,
            background: BackgroundStyle::default(),
        }
    }
}

/// An attribution prepared from its credits, a configuration and a text engine, without a
/// length.
///
/// It typesets each credit and the separator once. At a length, the credits fill lines in
/// order, and a line breaks only between credits, before a credit that would pass the length.
/// Every line is as tall as the largest ascent and the largest descent of the credits and the
/// separator, so no text reaches past its line.
/// A narrower length therefore gives more lines and a thicker attribution, and the answer
/// states exactly the lengths over which the lines hold, as a [`Legend`](crate::legend::Legend)
/// states its arrangement's.
#[derive(Debug)]
pub struct Attribution {
    config: AttributionConfig,
    credits: Vec<Typeset>,
    separator: Typeset,
}

/// A credit or the separator, typeset.
#[derive(Debug, Clone)]
struct Typeset {
    markup: String,
    width: f32,
    /// The distance from the top of the text's box down to its baseline.
    ascent: f32,
    /// The distance from the baseline down to the bottom of the text's box.
    descent: f32,
}

/// The credits at one length, in the box frame: the text block's top left corner is at the
/// background's padding.
struct Placed {
    /// Each credit's left edge, aligned in its line.
    left: Vec<f32>,
    /// Each line's top and baseline.
    lines: Vec<(f32, f32)>,
    /// The line of each credit.
    line_of: Vec<usize>,
    extent: PartExtent,
}

impl Attribution {
    /// An attribution of `credits`, each a string of Typst markup, such as
    /// `"© OpenStreetMap contributors"`. It fails for a side that isn't the top or the bottom,
    /// for a padding or a space that isn't finite and non-negative, and for a credit or a
    /// separator that does not typeset.
    pub fn new(
        credits: &[String],
        config: &AttributionConfig,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesError> {
        if config.side.is_vertical() {
            return Err(AvengerGuidesError::AttributionSide(config.side));
        }
        for (name, value) in [
            ("background padding", config.background.padding),
            ("separator space", config.separator_space),
            ("line gap", config.line_gap),
        ] {
            if !(value.is_finite() && value >= 0.0) {
                return Err(AvengerGuidesError::AttributionSpacing { name, value });
            }
        }
        let typeset = |markup: &str| -> Result<Typeset, AvengerGuidesError> {
            let options = LabelOptions {
                text: text_style(
                    &config.font_family,
                    config.font_size,
                    config.font_weight,
                    FontStyle::default(),
                    config.color,
                ),
                ..Default::default()
            };
            let bounds =
                engine.bounds(&text_label(markup, TextSyntaxMode::TypstMarkup, options))?;
            let BoxAnchor::Lines { ascent, .. } = bounds.anchor else {
                unreachable!("a credit has no path");
            };
            Ok(Typeset {
                markup: markup.to_string(),
                width: bounds.width,
                ascent,
                descent: bounds.height - ascent,
            })
        };
        Ok(Self {
            credits: credits
                .iter()
                .map(|credit| typeset(credit))
                .collect::<Result<_, _>>()?,
            separator: typeset(&config.separator)?,
            config: config.clone(),
        })
    }

    /// A content key for the attribution's slab in a layout: equal for attributions with the
    /// same credits and configuration, which draw the same at every length, and different
    /// otherwise. Slabs with a tag and equal keys collapse and merge under a container's rules,
    /// as maps that credit the same sources share one attribution below them.
    pub fn content_key(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let credits: Vec<&str> = self
            .credits
            .iter()
            .map(|credit| credit.markup.as_str())
            .collect();
        format!("{credits:?}{:?}", self.config).hash(&mut hasher);
        format!("attribution-{:016x}", hasher.finish())
    }

    /// The attribution's thickness and overhang at `length`, and the lengths where they hold.
    ///
    /// The range is exact: the credits keep their lines at every length in it, and move just
    /// past either end. A credit longer than the length, less the padding, reaches past the
    /// slab on the side away from the alignment, and the attribution reports that as overhang,
    /// at `length` only. An attribution without credits is empty at every length.
    pub fn extent(&self, length: f32) -> PartExtent {
        self.placed(length).extent
    }

    /// The origin for the attribution in the slab with rectangle `slot`, `[x, y, width,
    /// height]`: its inner edge, nearest the plot, lies on the slab's inner edge, at its start.
    pub fn origin(&self, slot: [f32; 4]) -> [f32; 2] {
        let thickness = self.extent(slot[2]).thickness;
        self.config.side.box_origin(slot, thickness)
    }

    /// The attribution at `length`, in its slab's frame with the slab's top left corner at 0,
    /// or `None` without credits. Its background spans the credits' lines.
    pub fn part(&self, length: f32) -> Option<SceneGroup> {
        let placed = self.placed(length);
        self.group(&placed)
    }

    /// The attribution in the bottom corner of a plot of `size`, at the alignment's end,
    /// inside the plot, for a chart without a layout. Its lines fit the plot's width. Give its
    /// background a fill, so that it reads over the plot's content.
    pub fn marks(&self, size: [f32; 2]) -> SceneGroup {
        let placed = self.placed(size[0]);
        let Some(mut group) = self.group(&placed) else {
            return SceneGroup::default();
        };
        group.origin = [0.0, size[1] - placed.extent.thickness];
        group
    }

    fn group(&self, placed: &Placed) -> Option<SceneGroup> {
        if self.credits.is_empty() {
            return None;
        }
        let padding = self.config.background.padding;
        let space = self.config.separator_space;
        let (mut text, mut x, mut y) = (Vec::new(), Vec::new(), Vec::new());
        for (index, credit) in self.credits.iter().enumerate() {
            let line = placed.line_of[index];
            let baseline = placed.lines[line].1;
            if index > 0 && placed.line_of[index - 1] == line {
                // The separator stands between the previous credit and this one.
                text.push(self.separator.markup.clone());
                x.push(placed.left[index] - space - self.separator.width);
                y.push(baseline);
            }
            text.push(credit.markup.clone());
            x.push(placed.left[index]);
            y.push(baseline);
        }
        let config = &self.config;
        let mark = SceneTextMark {
            len: text.len() as u32,
            clip: false,
            text: text.into(),
            text_syntax: TextSyntaxMode::TypstMarkup,
            x: x.into(),
            y: y.into(),
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Alphabetic.into(),
            font: config.font_family.clone().into(),
            font_size: config.font_size.into(),
            font_weight: config.font_weight.into(),
            color: config.color.into(),
            ..Default::default()
        };
        // The background spans the lines, from the first credit's left edge.
        let left = placed.left.iter().copied().fold(f32::INFINITY, f32::min) - padding;
        let right = self
            .credits
            .iter()
            .zip(&placed.left)
            .map(|(credit, left)| left + credit.width)
            .fold(f32::NEG_INFINITY, f32::max)
            + padding;
        let mut background =
            make_background(&config.background, right - left, placed.extent.thickness);
        if let SceneMark::Rect(rect) = &mut background {
            rect.x = left.into();
        }
        Some(SceneGroup {
            marks: vec![background, SceneMark::Text(Arc::new(mark))],
            interactive: Some(false),
            ..Default::default()
        })
    }

    fn placed(&self, length: f32) -> Placed {
        let config = &self.config;
        let padding = config.background.padding;
        if self.credits.is_empty() {
            return Placed {
                left: Vec::new(),
                lines: Vec::new(),
                line_of: Vec::new(),
                extent: PartExtent {
                    thickness: 0.0,
                    overhang: [0.0; 2],
                    holds: 0.0..=f32::INFINITY,
                },
            };
        }
        let typeset = || self.credits.iter().chain([&self.separator]);
        let ascent = typeset().map(|text| text.ascent).fold(0.0f32, f32::max);
        let descent = typeset().map(|text| text.descent).fold(0.0f32, f32::max);
        let sizes: Vec<[f32; 2]> = self
            .credits
            .iter()
            .map(|credit| [credit.width, ascent + descent])
            .collect();
        let gap = self.separator.width + 2.0 * config.separator_space;
        let available = length - 2.0 * padding;
        let arrangement = arrange(
            &sizes,
            [gap, config.line_gap],
            LegendWrap::Flow,
            false,
            available,
        );

        // Group credits into lines by their offset across.
        let mut line_of = Vec::with_capacity(self.credits.len());
        let mut tops: Vec<f32> = Vec::new();
        for position in &arrangement.positions {
            if tops.last() != Some(&position[1]) {
                tops.push(position[1]);
            }
            line_of.push(tops.len() - 1);
        }
        let lines: Vec<(f32, f32)> = tops
            .iter()
            .map(|top| (padding + top, padding + top + ascent))
            .collect();
        // Each line's length, for aligning it.
        let mut line_lengths = vec![0.0f32; tops.len()];
        for ((position, size), line) in arrangement.positions.iter().zip(&sizes).zip(&line_of) {
            line_lengths[*line] = line_lengths[*line].max(position[0] + size[0]);
        }
        let left: Vec<f32> = arrangement
            .positions
            .iter()
            .zip(&line_of)
            .map(|(position, line)| {
                let shift = match config.align {
                    AttributionAlign::Start => 0.0,
                    AttributionAlign::End => available - line_lengths[*line],
                };
                padding + shift + position[0]
            })
            .collect();

        let thickness = arrangement.size[1] + 2.0 * padding;
        // Only a credit that is longer than the length by itself makes its line pass it.
        let excess = excess(arrangement.size[0], available);
        let (overhang, holds) = if excess > 0.0 {
            let overhang = match config.align {
                AttributionAlign::Start => [0.0, excess],
                AttributionAlign::End => [excess, 0.0],
            };
            (overhang, length..=length)
        } else {
            (
                [0.0; 2],
                exact_holds(arrangement.holds, 2.0 * padding, length),
            )
        };
        Placed {
            left,
            lines,
            line_of,
            extent: PartExtent {
                thickness,
                overhang,
                holds,
            },
        }
    }
}

/// The lengths whose available length, the length less `padding`, keeps the arrangement that
/// `holds` describes, found in f32 as [`Attribution::extent`] subtracts.
fn exact_holds(holds: Holds, padding: f32, length: f32) -> std::ops::RangeInclusive<f32> {
    let Holds::From { low, below } = holds else {
        return length..=length;
    };
    let mut start = low + padding;
    while start - padding < low {
        start = start.next_up();
    }
    while start.next_down() - padding >= low {
        start = start.next_down();
    }
    if !below.is_finite() {
        return start..=f32::INFINITY;
    }
    let mut end = below + padding;
    while end - padding >= below {
        end = end.next_down();
    }
    while end.next_up() - padding < below {
        end = end.next_up();
    }
    start..=end
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_common::value::ScalarOrArray;

    const CREDITS: [&str; 3] = [
        "Countries: Natural Earth",
        "Earthquakes: U.S. Geological Survey",
        "Plates: Bird (2003)",
    ];

    fn attribution(align: AttributionAlign) -> Attribution {
        let credits: Vec<String> = CREDITS.map(String::from).into();
        let config = AttributionConfig {
            align,
            ..Default::default()
        };
        Attribution::new(&credits, &config, &crate::test_engine()).unwrap()
    }

    /// The text mark's items, with their left edges.
    fn items(group: &SceneGroup) -> Vec<(String, f32, f32)> {
        let SceneMark::Text(text) = &group.marks[1] else {
            panic!("a text mark");
        };
        let len = text.len as usize;
        let strings = text.text.as_vec(len, None);
        let x = text.x.as_vec(len, None);
        let y = text.y.as_vec(len, None);
        strings
            .into_iter()
            .zip(x)
            .zip(y)
            .map(|((text, x), y)| (text, x, y))
            .collect()
    }

    /// The credits of each line.
    fn breaks(attribution: &Attribution, length: f32) -> Vec<Vec<String>> {
        let mut lines: Vec<(f32, Vec<String>)> = Vec::new();
        for (text, _, baseline) in items(&attribution.part(length).unwrap()) {
            if text == "·" {
                continue;
            }
            match lines.last_mut() {
                Some((last, credits)) if *last == baseline => credits.push(text),
                _ => lines.push((baseline, vec![text])),
            }
        }
        lines.into_iter().map(|(_, credits)| credits).collect()
    }

    fn lines(attribution: &Attribution, length: f32) -> usize {
        breaks(attribution, length).len()
    }

    #[test]
    fn credits_break_only_between_credits_with_separators_inside_lines() {
        let attribution = attribution(AttributionAlign::End);
        for length in [800.0, 300.0, 120.0] {
            let items = items(&attribution.part(length).unwrap());
            let texts: Vec<&str> = items.iter().map(|item| item.0.as_str()).collect();
            let credits: Vec<&str> = texts.iter().copied().filter(|t| *t != "·").collect();
            assert_eq!(credits, CREDITS, "{length}");
            let separators = texts.len() - credits.len();
            assert_eq!(
                separators,
                CREDITS.len() - lines(&attribution, length),
                "{length}"
            );
        }
        assert_eq!(lines(&attribution, 800.0), 1);
        assert_eq!(lines(&attribution, 120.0), 3);
    }

    #[test]
    fn the_thickness_grows_with_the_lines_and_holds_exactly() {
        let attribution = attribution(AttributionAlign::End);
        let one = attribution.extent(800.0);
        let three = attribution.extent(150.0);
        assert!(
            three.thickness > 2.5 * (one.thickness - 8.0),
            "{one:?} {three:?}"
        );
        // At each length, every credit fits by itself.
        for length in [800.0, 300.0, 255.39, 250.0, 150.0] {
            let extent = attribution.extent(length);
            let (low, high) = (*extent.holds.start(), *extent.holds.end());
            assert!(low <= length && length <= high, "{length}: {extent:?}");
            // Just past either end, the credits break differently or one passes the length.
            let state = |length: f32| {
                (
                    breaks(&attribution, length),
                    attribution.extent(length).overhang,
                )
            };
            let held = state(length);
            assert_eq!(state(low), held, "{length} at {low}");
            assert_ne!(state(low.next_down()), held, "{length} below {low}");
            if high.is_finite() {
                assert_eq!(state(high), held, "{length} at {high}");
                assert_ne!(state(high.next_up()), held, "{length} past {high}");
            }
            assert_eq!(attribution.extent(low).thickness, extent.thickness);
            assert_eq!(extent.overhang, [0.0; 2]);
        }
    }

    #[test]
    fn lines_align_to_their_end_or_start() {
        let padding = BackgroundStyle::default().padding;
        let end = attribution(AttributionAlign::End);
        let group = end.part(800.0).unwrap();
        let last = items(&group).pop().unwrap();
        let width = end.credits[2].width;
        assert!(
            (last.1 + width - (800.0 - padding)).abs() < 1e-3,
            "{last:?}"
        );
        let start = attribution(AttributionAlign::Start);
        let first = items(&start.part(800.0).unwrap()).remove(0);
        assert_eq!(first.1, padding);
    }

    #[test]
    fn an_overlong_credit_overhangs_away_from_its_alignment() {
        let padding = BackgroundStyle::default().padding;
        let end = attribution(AttributionAlign::End);
        let widest = end.credits[1].width;
        let length = widest / 2.0;
        let extent = end.extent(length);
        let excess = widest + 2.0 * padding - length;
        assert!((extent.overhang[0] - excess).abs() < 1e-3, "{extent:?}");
        assert_eq!(extent.overhang[1], 0.0);
        assert_eq!(extent.holds, length..=length);
        let start = attribution(AttributionAlign::Start).extent(length);
        assert_eq!(start.overhang[0], 0.0);
        assert!((start.overhang[1] - excess).abs() < 1e-3, "{start:?}");
    }

    #[test]
    fn the_corner_box_sits_inside_the_plot() {
        let attribution = attribution(AttributionAlign::End);
        let group = attribution.marks([400.0, 300.0]);
        let thickness = attribution.extent(400.0).thickness;
        assert_eq!(group.origin, [0.0, 300.0 - thickness]);
        let SceneMark::Rect(background) = &group.marks[0] else {
            panic!("a background");
        };
        let value = |values: &ScalarOrArray<f32>| values.as_vec(1, None)[0];
        let x = value(&background.x);
        let width = background.width.as_ref().map_or(0.0, value);
        assert!(x >= 0.0 && x + width <= 400.5, "{x} {width}");
    }

    #[test]
    fn content_keys_follow_the_credits_and_the_configuration() {
        let key = |credits: &[&str], config: &AttributionConfig| {
            let credits: Vec<String> = credits.iter().map(|credit| credit.to_string()).collect();
            Attribution::new(&credits, config, &crate::test_engine())
                .unwrap()
                .content_key()
        };
        let config = AttributionConfig::default();
        assert_eq!(key(&CREDITS, &config), key(&CREDITS, &config));
        assert_ne!(key(&CREDITS, &config), key(&CREDITS[..2], &config));
        let larger = AttributionConfig {
            font_size: 12.0,
            ..Default::default()
        };
        assert_ne!(key(&CREDITS, &config), key(&CREDITS, &larger));
    }

    /// A line is as tall as the largest ascent and the largest descent, so a credit that sits
    /// lower than a taller one stays inside the line.
    #[test]
    fn lines_hold_the_deepest_descent() {
        let credits = ["Data".to_string(), "$x_2$".to_string()];
        let attribution = Attribution::new(
            &credits,
            &AttributionConfig::default(),
            &crate::test_engine(),
        )
        .unwrap();
        let [tall, deep] = &attribution.credits[..] else {
            panic!("two credits");
        };
        assert!(tall.ascent > deep.ascent && deep.descent > tall.descent);
        let placed = attribution.placed(800.0);
        let bottom = placed.extent.thickness - attribution.config.background.padding;
        for (credit, line) in attribution.credits.iter().zip(&placed.line_of) {
            assert!(placed.lines[*line].1 + credit.descent <= bottom + 1e-4);
        }
    }

    #[test]
    fn spacing_must_be_finite_and_non_negative() {
        let engine = crate::test_engine();
        let credits = ["Data".to_string()];
        for config in [
            AttributionConfig {
                background: BackgroundStyle {
                    padding: -1.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            AttributionConfig {
                separator_space: f32::INFINITY,
                ..Default::default()
            },
            AttributionConfig {
                line_gap: f32::NAN,
                ..Default::default()
            },
        ] {
            assert!(matches!(
                Attribution::new(&credits, &config, &engine),
                Err(AvengerGuidesError::AttributionSpacing { .. })
            ));
        }
    }

    #[test]
    fn attributions_stand_above_or_below_and_may_be_empty() {
        let config = AttributionConfig {
            side: GuideSide::Left,
            ..Default::default()
        };
        assert!(Attribution::new(&[], &config, &crate::test_engine()).is_err());
        let empty =
            Attribution::new(&[], &AttributionConfig::default(), &crate::test_engine()).unwrap();
        assert_eq!(empty.extent(100.0).thickness, 0.0);
        assert!(empty.part(100.0).is_none());
        assert!(empty.marks([100.0, 100.0]).marks.is_empty());
    }
}
