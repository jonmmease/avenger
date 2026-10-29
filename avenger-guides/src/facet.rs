//! Facet guides: the values that split a chart into panels, as a label beside each panel with a
//! tick on a rule that spans the panels, and the title of the column that the values come from,
//! in two parts that a layout places in separate slabs.

use std::hash::{Hash, Hasher};

use arrow::array::ArrayRef;
use avenger_color::ColorOrGradient;
use avenger_common::types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode};
use avenger_format::PrecisionSharing;
use avenger_scenegraph::marks::{
    group::SceneGroup,
    mark::SceneMark,
    rule::SceneRuleMark,
    text::{text_label, text_style, SceneTextMark},
};
use avenger_typst_label::{LabelEngine, LabelOptions};

use crate::{
    axis::label_text,
    error::AvengerGuidesError,
    guide::{excess, fits, LENGTH_TOLERANCE},
    GuideSide, PartExtent,
};

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// How a facet guide draws its values and its title.
#[derive(Debug, Clone, PartialEq)]
pub struct FacetGuideConfig {
    /// The side of the panels that the guide stands on: the top or the bottom for the panels of a
    /// row, and the left or the right for the panels of a column.
    pub side: GuideSide,
    /// The pattern for numeric and temporal values, in the syntax of the text engine's number or
    /// datetime provider, as `AxisConfig::tick_format` is. Other values, such as strings, show
    /// as text.
    pub label_format: String,
    /// Typst markup for each label, where `#value` is the value as `label_format` formats it,
    /// such as `Grade #value`. Without it, labels are the formatted values.
    pub label: Option<String>,
    pub style: FacetGuideStyle,
}

impl Default for FacetGuideConfig {
    fn default() -> Self {
        Self {
            side: GuideSide::Top,
            label_format: String::new(),
            label: None,
            style: FacetGuideStyle::default(),
        }
    }
}

/// Facet guide appearance. `FacetGuideStyle::default()` is the default look, and
/// `..Default::default()` keeps it for the settings that aren't given. Every space and length is
/// in pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct FacetGuideStyle {
    /// The space between the panels' edge and the labels.
    pub label_padding: f32,
    /// Degrees clockwise. Without it, labels read along the guide: down beside a right guide, up
    /// beside a left one, and level above or below the panels.
    pub label_angle: Option<f32>,
    pub label_color: [f32; 4],
    pub label_font_family: String,
    pub label_font_size: f32,
    pub label_font_weight: FontWeight,
    /// The space between the labels and the rule, which the ticks cross.
    pub rule_gap: f32,
    /// How far each tick reaches from the rule toward its label.
    pub tick_length: f32,
    /// The color of the rule and the ticks.
    pub rule_color: [f32; 4],
    /// The width of the rule and the ticks.
    pub rule_width: f32,
    pub title_visible: bool,
    pub title_syntax: TextSyntaxMode,
    /// The space between the labels part, which ends at the rule or at the label of a guide with
    /// one value, and the title.
    pub title_padding: f32,
    pub title_color: [f32; 4],
    pub title_font_family: String,
    pub title_font_size: f32,
    pub title_font_weight: FontWeight,
}

impl Default for FacetGuideStyle {
    fn default() -> Self {
        Self {
            label_padding: 4.0,
            label_angle: None,
            label_color: BLACK,
            label_font_family: "sans-serif".to_string(),
            label_font_size: 12.0,
            label_font_weight: FontWeight::default(),
            rule_gap: 5.0,
            tick_length: 4.0,
            rule_color: [0.5, 0.5, 0.5, 1.0],
            rule_width: 1.0,
            title_visible: true,
            title_syntax: TextSyntaxMode::default(),
            title_padding: 5.0,
            title_color: BLACK,
            title_font_family: "sans-serif".to_string(),
            title_font_size: 12.0,
            title_font_weight: FontWeight::default(),
        }
    }
}

/// One part of a facet guide, which a layout puts in its own slab: equal labels collapse along
/// each lane, and the titles of guides with one title merge, whatever their values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FacetGuidePart {
    /// The labels, nearest the panels, and the rule beyond them with a tick toward each label.
    Labels,
    /// The title, beyond the rule, centered on its slab.
    Title,
}

/// A facet guide prepared from its values, a title, a configuration and a text engine, without
/// the positions of the panels.
///
/// The labels part labels each panel at its center, next to the panels' edge. A rule beyond the
/// labels runs from the first panel's center to the last panel's, with a tick toward each label.
/// A guide with one value has no rule and no ticks. The title part stands beyond the labels
/// part, centered on its slab, so a title that a container merges over several guides centers
/// on all of their panels. Each part is as thick at every length, and `labels` takes the panels'
/// positions from the solved layout.
#[derive(Debug)]
pub struct FacetGuide {
    config: FacetGuideConfig,
    labels: Vec<Typeset>,
    syntax: TextSyntaxMode,
    title: Option<Typeset>,
    label_angle: f32,
    /// The thickness of the widest label across the guide.
    label_depth: f32,
}

/// A label or the title, typeset.
#[derive(Debug, Clone)]
struct Typeset {
    text: String,
    width: f32,
    height: f32,
}

impl FacetGuide {
    /// A guide that labels each of `values`, one for each panel in order, with `title` beyond
    /// them, or no title when it is empty. It fails for a space, a length or an angle that isn't
    /// finite, for a negative space or length, and for a value, a label or a title that does not
    /// format or typeset.
    pub fn new(
        values: &ArrayRef,
        title: &str,
        config: &FacetGuideConfig,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesError> {
        let style = &config.style;
        for (name, value) in [
            ("label padding", style.label_padding),
            ("rule gap", style.rule_gap),
            ("tick length", style.tick_length),
            ("rule width", style.rule_width),
            ("title padding", style.title_padding),
        ] {
            if !(value.is_finite() && value >= 0.0) {
                return Err(AvengerGuidesError::FacetGuideSpacing { name, value });
            }
        }
        let label_angle = style.label_angle.unwrap_or(match config.side {
            GuideSide::Left => -90.0,
            GuideSide::Right => 90.0,
            GuideSide::Top | GuideSide::Bottom => 0.0,
        });
        if !label_angle.is_finite() {
            return Err(AvengerGuidesError::FacetGuideAngle(label_angle));
        }
        let formatted =
            engine.format_array(values, &config.label_format, PrecisionSharing::PerValue)?;
        let (texts, syntax) = label_text(formatted, config.label.as_deref(), "value")?;
        let typeset = |text: String,
                       syntax: TextSyntaxMode,
                       font: &str,
                       size: f32,
                       weight: FontWeight,
                       color: [f32; 4]|
         -> Result<Typeset, AvengerGuidesError> {
            let options = LabelOptions {
                text: text_style(font, size, weight, FontStyle::default(), color),
                ..Default::default()
            };
            let bounds = engine.bounds(&text_label(&text, syntax, options))?;
            Ok(Typeset {
                text,
                width: bounds.width,
                height: bounds.height,
            })
        };
        let labels: Vec<Typeset> = texts
            .into_iter()
            .map(|text| {
                typeset(
                    text,
                    syntax,
                    &style.label_font_family,
                    style.label_font_size,
                    style.label_font_weight,
                    style.label_color,
                )
            })
            .collect::<Result<_, _>>()?;
        let title = if style.title_visible && !title.is_empty() {
            Some(typeset(
                title.to_string(),
                style.title_syntax,
                &style.title_font_family,
                style.title_font_size,
                style.title_font_weight,
                style.title_color,
            )?)
        } else {
            None
        };
        let label_depth = labels
            .iter()
            .map(|label| rotated_extent(label, label_angle, config.side).0)
            .fold(0.0, f32::max);
        Ok(Self {
            config: config.clone(),
            labels,
            syntax,
            title,
            label_angle,
            label_depth,
        })
    }

    /// A content key for a part's slab in a layout: equal for guides whose part draws the same,
    /// and different otherwise. The labels' key follows the labels and their style, and the
    /// title's key follows only the title and its style, so guides over different values can
    /// share one title. The rows of a facet grid label their columns with equal labels, which a
    /// container's collapse rule keeps once above the grid, and the panels of a wrap each have a
    /// guide with its one value and the same title, which a wrap's merge rule draws once above it.
    pub fn content_key(&self, part: FacetGuidePart) -> String {
        // Each setting is in the key of the part that it changes. The labels' text holds the
        // format and the template, the resolved angle holds `label_angle`, and a hidden title is
        // `None`.
        let FacetGuideStyle {
            label_padding,
            label_angle: _,
            label_color,
            label_font_family,
            label_font_size,
            label_font_weight,
            rule_gap,
            tick_length,
            rule_color,
            rule_width,
            title_visible: _,
            title_syntax,
            title_padding,
            title_color,
            title_font_family,
            title_font_size,
            title_font_weight,
        } = &self.config.style;
        let side = self.config.side;
        let (name, drawn) = match part {
            FacetGuidePart::Labels => {
                let labels: Vec<&str> = self
                    .labels
                    .iter()
                    .map(|label| label.text.as_str())
                    .collect();
                let style = (
                    self.label_angle,
                    label_padding,
                    label_color,
                    label_font_family,
                    label_font_size,
                    label_font_weight,
                    rule_gap,
                    tick_length,
                    rule_color,
                    rule_width,
                );
                (
                    "labels",
                    format!("{labels:?}{:?}{side:?}{style:?}", self.syntax),
                )
            }
            FacetGuidePart::Title => {
                let title = self.title.as_ref().map(|title| title.text.as_str());
                let style = (
                    title_syntax,
                    title_padding,
                    title_color,
                    title_font_family,
                    title_font_size,
                    title_font_weight,
                );
                ("title", format!("{title:?}{side:?}{style:?}"))
            }
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        drawn.hash(&mut hasher);
        format!("facet-{name}-{:016x}", hasher.finish())
    }

    /// The part's thickness and overhang at `length`, and the lengths where they hold.
    ///
    /// The labels part is as thick at every length and reports no overhang: a label wider than
    /// its panel reaches past it, and past the slab's ends at the first and the last panel. It is
    /// empty without values. The title holds from its own length up, where it reaches past
    /// neither end, and otherwise only at `length`, past both ends by the same amount. It is
    /// empty without a title.
    pub fn extent(&self, part: FacetGuidePart, length: f32) -> PartExtent {
        match part {
            FacetGuidePart::Labels if self.labels.is_empty() => PartExtent::empty(),
            FacetGuidePart::Labels => PartExtent {
                thickness: self.labels_thickness(),
                overhang: [0.0; 2],
                holds: 0.0..=f32::INFINITY,
            },
            FacetGuidePart::Title => {
                let Some(title) = &self.title else {
                    return PartExtent::empty();
                };
                // The title turns a quarter beside a left or a right guide, so its width runs
                // along the guide on every side.
                let excess = excess(title.width, length) / 2.0;
                PartExtent {
                    thickness: self.config.style.title_padding + title.height,
                    overhang: [excess; 2],
                    holds: if fits(title.width, length) {
                        (title.width - LENGTH_TOLERANCE)..=f32::INFINITY
                    } else {
                        length..=length
                    },
                }
            }
        }
    }

    /// The origin for a part in the slab with rectangle `slot`, `[x, y, width, height]`: where
    /// the slab's inner edge, nearest the panels, meets its start.
    pub fn origin(&self, slot: [f32; 4]) -> [f32; 2] {
        self.config.side.slot_origin(slot)
    }

    /// The labels part in its slab's frame, where 0 along the guide is the slab's start and 0
    /// across it is its inner edge, or `None` without values. `bands` holds the start and the
    /// end of each panel along the slab, one for each value in order, as the solved layout
    /// places them.
    ///
    /// # Panics
    ///
    /// When `bands` does not hold one band for each value.
    pub fn labels(&self, bands: &[[f32; 2]]) -> Option<SceneGroup> {
        assert_eq!(
            bands.len(),
            self.labels.len(),
            "a facet guide takes one band for each of its values"
        );
        if self.labels.is_empty() {
            return None;
        }
        let side = self.config.side;
        let style = &self.config.style;
        let centers: Vec<f32> = bands
            .iter()
            .map(|[start, end]| (start + end) / 2.0)
            .collect();
        let mut marks = Vec::new();

        // Each label's box is centered on its panel and ends at the far side of the labels, so
        // every label stands the same distance from the ticks.
        let (mut x, mut y) = (Vec::new(), Vec::new());
        for (label, &center) in self.labels.iter().zip(&centers) {
            let depth = rotated_extent(label, self.label_angle, side).0;
            let [label_x, label_y] =
                side.point(center, style.label_padding + self.label_depth - depth / 2.0);
            x.push(label_x);
            y.push(label_y);
        }
        marks.push(SceneMark::from(SceneTextMark {
            len: self.labels.len() as u32,
            clip: false,
            text: self
                .labels
                .iter()
                .map(|label| label.text.clone())
                .collect::<Vec<_>>()
                .into(),
            text_syntax: self.syntax,
            x: x.into(),
            y: y.into(),
            align: TextAlign::Center.into(),
            baseline: TextBaseline::Middle.into(),
            angle: self.label_angle.into(),
            font: style.label_font_family.clone().into(),
            font_size: style.label_font_size.into(),
            font_weight: style.label_font_weight.into(),
            color: style.label_color.into(),
            ..Default::default()
        }));

        if let Some(rule) = self.rule() {
            // The rule reaches half its width past the first and the last tick, so that its
            // corners close around them.
            let first = centers.iter().copied().fold(f32::INFINITY, f32::min);
            let last = centers.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let half = style.rule_width / 2.0;
            let mut ends = vec![(
                side.point(first - half, rule),
                side.point(last + half, rule),
            )];
            for &center in &centers {
                ends.push((
                    side.point(center, rule - style.tick_length),
                    side.point(center, rule),
                ));
            }
            marks.push(SceneMark::from(SceneRuleMark {
                len: ends.len() as u32,
                x: ends
                    .iter()
                    .map(|(start, _)| start[0])
                    .collect::<Vec<_>>()
                    .into(),
                y: ends
                    .iter()
                    .map(|(start, _)| start[1])
                    .collect::<Vec<_>>()
                    .into(),
                x2: ends
                    .iter()
                    .map(|(_, end)| end[0])
                    .collect::<Vec<_>>()
                    .into(),
                y2: ends
                    .iter()
                    .map(|(_, end)| end[1])
                    .collect::<Vec<_>>()
                    .into(),
                stroke: ColorOrGradient::Color(style.rule_color).into(),
                stroke_width: style.rule_width.into(),
                ..Default::default()
            }));
        }

        Some(SceneGroup {
            marks,
            ..Default::default()
        })
    }

    /// The title part in the frame of its slab of `length`, centered on the slab, or `None`
    /// without a title. Beside a left or a right guide, it reads along the guide, as the labels
    /// do by default.
    pub fn title(&self, length: f32) -> Option<SceneGroup> {
        let title = self.title.as_ref()?;
        let side = self.config.side;
        let style = &self.config.style;
        let [x, y] = side.point(length / 2.0, style.title_padding + title.height / 2.0);
        let angle = match side {
            GuideSide::Left => -90.0,
            GuideSide::Right => 90.0,
            GuideSide::Top | GuideSide::Bottom => 0.0,
        };
        let mark = SceneTextMark {
            len: 1,
            clip: false,
            text: title.text.clone().into(),
            text_syntax: style.title_syntax,
            x: x.into(),
            y: y.into(),
            align: TextAlign::Center.into(),
            baseline: TextBaseline::Middle.into(),
            angle: angle.into(),
            font: style.title_font_family.clone().into(),
            font_size: style.title_font_size.into(),
            font_weight: style.title_font_weight.into(),
            color: style.title_color.into(),
            ..Default::default()
        };
        Some(SceneGroup {
            marks: vec![mark.into()],
            ..Default::default()
        })
    }

    /// The rule's center line, across the labels part, or `None` for a guide with one value.
    fn rule(&self) -> Option<f32> {
        let style = &self.config.style;
        (self.labels.len() > 1).then_some(style.label_padding + self.label_depth + style.rule_gap)
    }

    /// How far the labels part reaches across the guide: to the rule's outer edge, or to the far
    /// side of the label of a guide with one value.
    fn labels_thickness(&self) -> f32 {
        let style = &self.config.style;
        match self.rule() {
            Some(rule) => rule + style.rule_width / 2.0,
            None => style.label_padding + self.label_depth,
        }
    }
}

/// How far `label`, turned `angle` degrees clockwise about its center, reaches across a guide on
/// `side` and along it.
fn rotated_extent(label: &Typeset, angle: f32, side: GuideSide) -> (f32, f32) {
    // Quarter turns are exact.
    let (sin, cos) = match angle.rem_euclid(360.0) {
        0.0 => (0.0, 1.0),
        90.0 => (1.0, 0.0),
        180.0 => (0.0, -1.0),
        270.0 => (-1.0, 0.0),
        angle => angle.to_radians().sin_cos(),
    };
    let x = (label.width * cos).abs() + (label.height * sin).abs();
    let y = (label.width * sin).abs() + (label.height * cos).abs();
    if side.is_vertical() {
        (x, y)
    } else {
        (y, x)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Float64Array, StringArray};

    use super::*;

    const BANDS: [[f32; 2]; 3] = [[0.0, 100.0], [120.0, 220.0], [240.0, 340.0]];

    fn grades() -> ArrayRef {
        Arc::new(StringArray::from(vec!["A", "B", "C"]))
    }

    fn guide(values: &ArrayRef, title: &str, side: GuideSide) -> FacetGuide {
        let config = FacetGuideConfig {
            side,
            ..Default::default()
        };
        FacetGuide::new(values, title, &config, &crate::test_engine()).unwrap()
    }

    fn text(group: &SceneGroup, index: usize) -> &SceneTextMark {
        let SceneMark::Text(text) = &group.marks[index] else {
            panic!("a text mark at {index}");
        };
        text
    }

    fn rules(group: &SceneGroup) -> Vec<[f32; 4]> {
        let SceneMark::Rule(rule) = &group.marks[1] else {
            panic!("a rule mark");
        };
        let len = rule.len as usize;
        let [x, y, x2, y2] = [&rule.x, &rule.y, &rule.x2, &rule.y2].map(|v| v.as_vec(len, None));
        (0..len).map(|i| [x[i], y[i], x2[i], y2[i]]).collect()
    }

    fn positions(text: &SceneTextMark) -> Vec<[f32; 2]> {
        let len = text.len as usize;
        let x = text.x.as_vec(len, None);
        let y = text.y.as_vec(len, None);
        x.into_iter().zip(y).map(|(x, y)| [x, y]).collect()
    }

    #[test]
    fn a_right_guide_stacks_labels_and_rule_and_then_the_title_outward() {
        let guide = guide(&grades(), "Grade", GuideSide::Right);
        let style = FacetGuideStyle::default();
        // Labels beside a right guide read down, so their height is the guide's depth.
        let depth = guide.labels.iter().map(|l| l.height).fold(0.0, f32::max);
        assert_eq!(guide.label_angle, 90.0);
        assert_eq!(guide.label_depth, depth);
        let rule = style.label_padding + depth + style.rule_gap;
        let extent = guide.extent(FacetGuidePart::Labels, 500.0);
        assert_eq!(extent.thickness, rule + style.rule_width / 2.0);
        assert_eq!(extent.overhang, [0.0; 2]);
        assert_eq!(extent.holds, 0.0..=f32::INFINITY);

        let group = guide.labels(&BANDS).unwrap();
        let labels = text(&group, 0);
        assert_eq!(labels.text.as_vec(3, None), ["A", "B", "C"]);
        for (position, center) in positions(labels).into_iter().zip([50.0, 170.0, 290.0]) {
            assert_eq!(position[1], center);
            assert!(position[0] > style.label_padding && position[0] < rule - style.rule_gap);
        }
        let half = style.rule_width / 2.0;
        assert_eq!(
            rules(&group),
            [
                [rule, 50.0 - half, rule, 290.0 + half],
                [rule - style.tick_length, 50.0, rule, 50.0],
                [rule - style.tick_length, 170.0, rule, 170.0],
                [rule - style.tick_length, 290.0, rule, 290.0],
            ]
        );

        // The title's slab begins where the labels part ends, and the title centers on it.
        let title_height = guide.title.as_ref().unwrap().height;
        let extent = guide.extent(FacetGuidePart::Title, 340.0);
        assert_eq!(extent.thickness, style.title_padding + title_height);
        let group = guide.title(340.0).unwrap();
        let title = text(&group, 0);
        assert_eq!(
            positions(title),
            [[style.title_padding + title_height / 2.0, 170.0]]
        );
        assert_eq!(title.angle.as_vec(1, None), [90.0]);
    }

    #[test]
    fn a_left_guide_mirrors_a_right_one_and_reads_up() {
        let right = guide(&grades(), "Grade", GuideSide::Right);
        let left = guide(&grades(), "Grade", GuideSide::Left);
        assert_eq!(left.label_angle, -90.0);
        let mirror = |[x, y]: [f32; 2]| [-x, y];
        let (right_labels, left_labels) =
            (right.labels(&BANDS).unwrap(), left.labels(&BANDS).unwrap());
        assert_eq!(
            positions(text(&left_labels, 0)),
            positions(text(&right_labels, 0))
                .into_iter()
                .map(mirror)
                .collect::<Vec<_>>()
        );
        let mirrored: Vec<[f32; 4]> = rules(&right_labels)
            .into_iter()
            .map(|[x, y, x2, y2]| [-x, y, -x2, y2])
            .collect();
        assert_eq!(rules(&left_labels), mirrored);
        let (right_title, left_title) = (right.title(340.0).unwrap(), left.title(340.0).unwrap());
        assert_eq!(
            positions(text(&left_title, 0)),
            positions(text(&right_title, 0))
                .into_iter()
                .map(mirror)
                .collect::<Vec<_>>()
        );
        assert_eq!(text(&left_title, 0).angle.as_vec(1, None), [-90.0]);
    }

    #[test]
    fn a_top_guide_labels_level_above_the_panels() {
        let guide = guide(&grades(), "Grade", GuideSide::Top);
        assert_eq!(guide.label_angle, 0.0);
        let depth = guide.labels.iter().map(|l| l.height).fold(0.0, f32::max);
        assert_eq!(guide.label_depth, depth);
        let group = guide.labels(&BANDS).unwrap();
        for (position, center) in positions(text(&group, 0))
            .into_iter()
            .zip([50.0, 170.0, 290.0])
        {
            assert_eq!(position[0], center);
            assert!(position[1] < 0.0, "above the panels: {position:?}");
        }
        let rule = rules(&group)[0];
        assert_eq!(rule[1], rule[3]);
        assert!(rule[1] < -depth);
        let title = guide.title(340.0).unwrap();
        assert_eq!(text(&title, 0).angle.as_vec(1, None), [0.0]);
        assert!(positions(text(&title, 0))[0][1] < 0.0);
    }

    #[test]
    fn one_value_draws_no_rule_and_ends_the_labels_at_the_label() {
        let values: ArrayRef = Arc::new(StringArray::from(vec!["A"]));
        let guide = guide(&values, "Grade", GuideSide::Top);
        let style = FacetGuideStyle::default();
        let group = guide.labels(&[[0.0, 100.0]]).unwrap();
        assert_eq!(group.marks.len(), 1, "a label without a rule");
        assert_eq!(
            guide.extent(FacetGuidePart::Labels, 100.0).thickness,
            style.label_padding + guide.label_depth
        );
    }

    #[test]
    fn a_guide_without_values_draws_no_labels_and_one_without_a_title_no_title() {
        let none: ArrayRef = Arc::new(StringArray::from(Vec::<&str>::new()));
        let empty = guide(&none, "Grade", GuideSide::Top);
        assert_eq!(
            empty.extent(FacetGuidePart::Labels, 100.0),
            PartExtent::empty()
        );
        assert!(empty.labels(&[]).is_none());

        let untitled = guide(&grades(), "", GuideSide::Top);
        assert_eq!(
            untitled.extent(FacetGuidePart::Title, 100.0),
            PartExtent::empty()
        );
        assert!(untitled.title(100.0).is_none());
        let hidden = FacetGuide::new(
            &grades(),
            "Grade",
            &FacetGuideConfig {
                style: FacetGuideStyle {
                    title_visible: false,
                    ..Default::default()
                },
                ..Default::default()
            },
            &crate::test_engine(),
        )
        .unwrap();
        assert!(hidden.title(100.0).is_none());
    }

    #[test]
    fn a_title_longer_than_its_slab_overhangs_both_ends() {
        let guide = guide(&grades(), "A rather long facet guide title", GuideSide::Top);
        let width = guide.title.as_ref().unwrap().width;
        let fits = guide.extent(FacetGuidePart::Title, width + 20.0);
        assert_eq!(fits.overhang, [0.0; 2]);
        assert_eq!(fits.holds, (width - LENGTH_TOLERANCE)..=f32::INFINITY);
        // A slab short of the title by the last bit fits it.
        assert_eq!(
            guide
                .extent(FacetGuidePart::Title, width.next_down())
                .overhang,
            [0.0; 2]
        );
        let short = guide.extent(FacetGuidePart::Title, width - 20.0);
        assert_eq!(short.overhang, [10.0; 2]);
        assert_eq!(short.holds, (width - 20.0)..=(width - 20.0));
        assert_eq!(short.thickness, fits.thickness);
    }

    #[test]
    fn labels_format_numbers_and_bind_value_markup() {
        let values: ArrayRef = Arc::new(Float64Array::from(vec![1500.0, 2500.0]));
        let config = FacetGuideConfig {
            label_format: ",.0f".into(),
            label: Some("*#value* cyl".into()),
            ..Default::default()
        };
        let guide = FacetGuide::new(&values, "", &config, &crate::test_engine()).unwrap();
        let texts: Vec<&str> = guide.labels.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, [r#"*#("1,500")* cyl"#, r#"*#("2,500")* cyl"#]);
        assert_eq!(guide.syntax, TextSyntaxMode::TypstMarkup);
    }

    #[test]
    fn labels_keys_follow_the_labels_and_title_keys_only_the_title() {
        use FacetGuidePart::{Labels, Title};
        let key = |guide: &FacetGuide, part| guide.content_key(part);
        let a = guide(&grades(), "Grade", GuideSide::Top);
        let b = guide(&grades(), "Grade", GuideSide::Top);
        assert_eq!(key(&a, Labels), key(&b, Labels));
        assert_eq!(key(&a, Title), key(&b, Title));
        assert_ne!(key(&a, Labels), key(&a, Title));

        // Guides over other values keep their own labels but share the title.
        let other: ArrayRef = Arc::new(StringArray::from(vec!["A", "D"]));
        let other = guide(&other, "Grade", GuideSide::Top);
        assert_ne!(key(&a, Labels), key(&other, Labels));
        assert_eq!(key(&a, Title), key(&other, Title));

        // Another title changes only the title's key.
        let score = guide(&grades(), "Score", GuideSide::Top);
        assert_eq!(key(&a, Labels), key(&score, Labels));
        assert_ne!(key(&a, Title), key(&score, Title));

        // A side changes both, and each style setting changes only its own part's key.
        let bottom = guide(&grades(), "Grade", GuideSide::Bottom);
        assert_ne!(key(&a, Labels), key(&bottom, Labels));
        assert_ne!(key(&a, Title), key(&bottom, Title));
        let styled = |style: FacetGuideStyle| {
            let config = FacetGuideConfig {
                style,
                ..Default::default()
            };
            FacetGuide::new(&grades(), "Grade", &config, &crate::test_engine()).unwrap()
        };
        let big_title = styled(FacetGuideStyle {
            title_font_size: 16.0,
            ..Default::default()
        });
        assert_eq!(key(&a, Labels), key(&big_title, Labels));
        assert_ne!(key(&a, Title), key(&big_title, Title));
        let red_rule = styled(FacetGuideStyle {
            rule_color: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        });
        assert_ne!(key(&a, Labels), key(&red_rule, Labels));
        assert_eq!(key(&a, Title), key(&red_rule, Title));
    }

    #[test]
    fn spaces_and_angles_must_be_finite() {
        let engine = crate::test_engine();
        let with = |style: FacetGuideStyle| {
            let config = FacetGuideConfig {
                style,
                ..Default::default()
            };
            FacetGuide::new(&grades(), "Grade", &config, &engine)
        };
        let error = with(FacetGuideStyle {
            rule_gap: -1.0,
            ..Default::default()
        })
        .unwrap_err();
        assert!(matches!(
            error,
            AvengerGuidesError::FacetGuideSpacing {
                name: "rule gap",
                ..
            }
        ));
        let error = with(FacetGuideStyle {
            label_angle: Some(f32::NAN),
            ..Default::default()
        })
        .unwrap_err();
        assert!(matches!(error, AvengerGuidesError::FacetGuideAngle(_)));
    }

    #[test]
    #[should_panic(expected = "one band for each of its values")]
    fn labels_need_a_band_for_each_value() {
        guide(&grades(), "Grade", GuideSide::Top).labels(&BANDS[..2]);
    }
}
