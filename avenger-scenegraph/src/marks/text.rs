use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    num::NonZeroUsize,
    sync::Arc,
};

use super::{mark::SceneMark, path::ScenePathMark};
use crate::render::fill::contours_to_path;
use crate::render::text::lyon_curve;
use avenger_color::{
    relative_luminance_srgb, shift_for_contrast, AbsoluteColor, ColorOrGradient, ColorSpace,
};
use avenger_common::types::PathTransform;
use lyon_path::geom::{euclid::Vector2D, Angle};

use avenger_common::lyon::lyon_path_values_equal;
use avenger_common::types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode};
use avenger_common::value::{hash_values, ScalarOrArray, ScalarOrArrayValue};
use avenger_typst_label::{
    BoxAnchor, Label, LabelAlign, LabelEngine, LabelError, LabelLineHeight, LabelOptions,
    LabelSource, LabelWidth, PathAnchor, PathBaseline, PathOffset, PathOverflow, TextBounds,
    TextPath, TextStyle,
};
use educe::Educe;
use itertools::izip;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Educe, Serialize, Deserialize)]
#[educe(Hash)]
#[serde(rename_all = "kebab-case")]
pub struct SceneTextMark {
    /// The mark's name, which events report.
    pub name: String,
    /// Whether hit tests can find the mark's labels.
    /// Whether hits find the mark. `None` takes the enclosing group's setting, and marks at the
    /// scene root are interactive.
    pub interactive: Option<bool>,
    /// Whether the enclosing group's clip applies.
    pub clip: bool,
    /// The number of labels.
    pub len: u32,
    /// Each label's text.
    pub text: ScalarOrArray<String>,
    /// Whether the text is plain, plain with newlines that end lines, or Typst markup.
    #[serde(default)]
    pub text_syntax: TextSyntaxMode,
    /// Each label's horizontal position.
    pub x: ScalarOrArray<f32>,
    /// Each label's vertical position.
    pub y: ScalarOrArray<f32>,
    /// Where each label's position lies across it.
    pub align: ScalarOrArray<TextAlign>,
    /// Where each label's position lies down it.
    pub baseline: ScalarOrArray<TextBaseline>,
    /// Each label's rotation about its position, in degrees.
    pub angle: ScalarOrArray<f32>,
    /// Each label's straight-alpha RGBA color. Text paint is solid.
    pub color: ScalarOrArray<[f32; 4]>,
    /// Each label's CSS-style list of font families.
    pub font: ScalarOrArray<String>,
    /// Each label's font size.
    pub font_size: ScalarOrArray<f32>,
    /// Each label's font weight.
    pub font_weight: ScalarOrArray<FontWeight>,
    /// Each label's font style.
    pub font_style: ScalarOrArray<FontStyle>,
    /// How wide each label is.
    #[educe(Hash(method(hash_values)))]
    pub width: ScalarOrArray<LabelWidth>,
    /// Whether lines wrap at the width.
    pub wrap: bool,
    /// The most lines each label keeps, or all of them.
    pub max_lines: Option<NonZeroUsize>,
    /// Whether "…" marks text that the width or `max_lines` cuts.
    pub ellipsis: bool,
    /// Whether a sign that starts a line hangs out of it, so numbers align by their digits.
    pub hanging_signs: bool,
    /// The distance between each label's baselines.
    #[educe(Hash(method(hash_values)))]
    pub line_height: ScalarOrArray<LabelLineHeight>,
    /// How each label's lines align within its box.
    #[educe(Hash(method(hash_values)))]
    pub line_align: ScalarOrArray<LabelAlign>,
    /// The labels to draw, in order, or all of them.
    pub indices: Option<Arc<Vec<usize>>>,
    /// The mark's drawing order: a higher zindex draws over a lower one.
    pub zindex: Option<i32>,
    /// A halo behind the labels, so that they read over lines and fills, or `None` for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub halo: Option<TextHalo>,
    /// The paths that the labels follow, or `None` for text in straight lines. With paths,
    /// `align` sets which point of each label's text lies at its path's offset (its start,
    /// middle, or end), `baseline` sets which part of the text lies on the path (`Middle`
    /// centers the text on it, and `Bottom` sets the text on top of it), and `angle` turns each
    /// path and its text about the label's position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_path: Option<TextPaths>,
}

/// The paths that the labels of a text mark follow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextPaths {
    /// Each label's path, in pixels, with the label's `x` and `y` as its origin, turned by its
    /// `angle`. A polar chart can set `x` and `y` to the circle's center and give every label on
    /// a ring one arc about the origin. The text follows the path's first subpath.
    pub path: ScalarOrArray<lyon_path::Path>,
    /// Where along each path the label's text lies, as `align` picks its point.
    pub offset: ScalarOrArray<PathOffset>,
    /// How far each label moves away from its path, in pixels, on the side where its
    /// `baseline` puts it: down for `Top` and `LineTop`, and up for the others, as the text
    /// reads. Half a line's stroke width, plus any padding, clears the line from either side.
    pub shift: ScalarOrArray<f32>,
    /// Whether a label runs along its path reversed where it would read upside down, as
    /// [`reads_reversed`](avenger_typst_label::reads_reversed) decides from the path's direction
    /// at the middle of the text, turned by the label's angle.
    pub upright: bool,
    /// What happens to a label that reaches past its path's ends.
    pub overflow: PathOverflow,
}

/// One label's part of a text mark's paths.
#[derive(Clone, Copy)]
struct PathOfLabel<'a> {
    paths: &'a TextPaths,
    path: &'a lyon_path::Path,
    offset: PathOffset,
    shift: f32,
}

impl TextPaths {
    /// Each label's part of the paths, in the order of the mark's labels.
    fn of_labels<'a>(
        &'a self,
        len: usize,
        indices: Option<&'a Arc<Vec<usize>>>,
    ) -> impl Iterator<Item = Option<PathOfLabel<'a>>> + 'a {
        izip!(
            self.path.as_iter(len, indices),
            self.offset.as_iter(len, indices),
            self.shift.as_iter(len, indices),
        )
        .map(move |(path, offset, shift)| {
            Some(PathOfLabel {
                paths: self,
                path,
                offset: *offset,
                shift: *shift,
            })
        })
    }
}

impl PathOfLabel<'_> {
    /// The label crate's path for a label with this alignment, baseline, and angle, which set
    /// the path's anchor, the part of the text on it, and the angle of its upright test.
    fn text_path(&self, align: TextAlign, baseline: TextBaseline, angle: f32) -> TextPath {
        TextPath {
            path: lyon_curve(self.path),
            offset: self.offset,
            anchor: match align {
                TextAlign::Left => PathAnchor::Start,
                TextAlign::Center => PathAnchor::Middle,
                TextAlign::Right => PathAnchor::End,
            },
            baseline: match baseline {
                TextBaseline::Alphabetic => PathBaseline::Alphabetic,
                TextBaseline::Top => PathBaseline::Top,
                TextBaseline::Middle => PathBaseline::Middle,
                TextBaseline::Bottom => PathBaseline::Bottom,
                TextBaseline::LineTop => PathBaseline::LineTop,
                TextBaseline::LineBottom => PathBaseline::LineBottom,
            },
            shift: self.shift,
            upright: self.paths.upright.then_some(angle),
            overflow: self.paths.overflow,
        }
    }
}

impl Hash for TextPaths {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
        let offsets: Vec<(u8, u32)> = self
            .offset
            .as_iter(1, None)
            .map(|offset| match *offset {
                PathOffset::Fraction(fraction) => (0, fraction.to_bits()),
                PathOffset::Distance(distance) => (1, distance.to_bits()),
            })
            .collect();
        offsets.hash(state);
        self.shift.hash(state);
        self.upright.hash(state);
        self.overflow.hash(state);
    }
}

impl PartialEq for TextPaths {
    fn eq(&self, other: &Self) -> bool {
        lyon_path_values_equal(&self.path, &other.path)
            && self.offset == other.offset
            && self.shift == other.shift
            && self.upright == other.upright
            && self.overflow == other.overflow
    }
}

/// A halo behind the labels of a text mark: the outline of each label's glyphs, grown by
/// `width` pixels with round corners, and filled with the label's `color`, which a halo usually
/// takes from the background with part of its opacity. The halos of a mark draw before its
/// labels, so a halo never covers a label of its mark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextHalo {
    /// Each label's straight-alpha RGBA halo color.
    pub color: ScalarOrArray<[f32; 4]>,
    /// How far the halo reaches past the glyphs, in pixels.
    pub width: f32,
}

impl Hash for TextHalo {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.color.hash(state);
        self.width.to_bits().hash(state);
    }
}

/// The text color of a label on each of `backgrounds`: `light` on backgrounds with Oklch lightness
/// up to `light_up_to`, and `dark` on lighter ones. Lightness runs from 0 for black to 1 for white.
/// 0.5 is midway between them, and a higher `light_up_to` suits labels with a halo that darkens
/// the background behind light text. Alpha is ignored.
pub fn contrast_text_colors(
    backgrounds: &ScalarOrArray<[f32; 4]>,
    dark: [f32; 4],
    light: [f32; 4],
    light_up_to: f32,
) -> ScalarOrArray<[f32; 4]> {
    backgrounds.map(|background| {
        let lightness = AbsoluteColor::from_rgba(*background)
            .to_color_space(ColorSpace::Oklch)
            .components[0];
        if lightness <= light_up_to {
            light
        } else {
            dark
        }
    })
}

/// The halo color of a label with each of `texts` on each of `backgrounds`: the background with
/// its Oklch lightness moved by the least amount that gives the text a minimum WCAG contrast ratio
/// with it, keeping its hue and its alpha. The minimum is `dark_min_ratio` for text darker than
/// the background in relative luminance and `light_min_ratio` for lighter text. Ratios run from 1,
/// the background, to 21, white or black. When the text can't reach its ratio, the halo stops at
/// white or black.
///
/// Arrays pair up by index, and a scalar pairs with each element. Each pair of colors takes a
/// search over lightness, which pairs that repeat share, so compute the halo colors when the
/// colors change rather than for each scene that draws them.
pub fn contrast_halo_colors(
    texts: &ScalarOrArray<[f32; 4]>,
    backgrounds: &ScalarOrArray<[f32; 4]>,
    dark_min_ratio: f32,
    light_min_ratio: f32,
) -> ScalarOrArray<[f32; 4]> {
    let mut found = HashMap::new();
    let mut halo = |text: &[f32; 4], background: &[f32; 4]| {
        let key = (text.map(f32::to_bits), background.map(f32::to_bits));
        *found.entry(key).or_insert_with(|| {
            let min_ratio = if luminance(*text) <= luminance(*background) {
                dark_min_ratio
            } else {
                light_min_ratio
            };
            shift_for_contrast(
                &AbsoluteColor::from_rgba(*background),
                &AbsoluteColor::from_rgba(*text),
                min_ratio,
            )
            .to_rgba()
        })
    };
    match (texts.value(), backgrounds.value()) {
        (ScalarOrArrayValue::Scalar(text), ScalarOrArrayValue::Scalar(background)) => {
            ScalarOrArray::new_scalar(halo(text, background))
        }
        _ => {
            let len = texts.len().max(backgrounds.len());
            let pairs = texts.as_iter(len, None).zip(backgrounds.as_iter(len, None));
            ScalarOrArray::new_array(
                pairs
                    .map(|(text, background)| halo(text, background))
                    .collect(),
            )
        }
    }
}

/// The WCAG relative luminance of a straight-alpha RGBA color, which ignores its alpha.
fn luminance(color: [f32; 4]) -> f32 {
    let [r, g, b, _] = AbsoluteColor::from_rgba(color).to_rgba();
    relative_luminance_srgb(r, g, b)
}

impl SceneTextMark {
    /// A path mark of the halo of each label, in the frame of the mark, or `None` when no label
    /// has a halo. Renderers draw it before the labels.
    pub fn halo_mark(&self, engine: &LabelEngine) -> Result<Option<ScenePathMark>, LabelError> {
        let Some(halo) = self.halo.as_ref().filter(|halo| halo.width > 0.0) else {
            return Ok(None);
        };
        let mut paths = Vec::new();
        let mut transforms = Vec::new();
        let mut fills = Vec::new();
        let colors = halo.color.as_iter(self.len as usize, self.indices.as_ref());
        for (label, color) in self.labels().zip(colors) {
            let finite = label
                .position
                .iter()
                .chain([&label.angle])
                .all(|value| value.is_finite());
            if !finite || color[3] <= 0.0 {
                continue;
            }
            let outline = engine.expanded_outline(&label.label, halo.width)?;
            if outline.contours.is_empty() {
                continue;
            }
            let bounds = engine.bounds(&label.label)?;
            let [left, top] = text_origin(&bounds, [0.0, 0.0], label.align, label.baseline);
            paths.push(contours_to_path(&outline.contours));
            fills.push(ColorOrGradient::Color(*color));
            transforms.push(
                PathTransform::translation(left, top)
                    .then_rotate(Angle::degrees(label.angle))
                    .then_translate(Vector2D::new(label.position[0], label.position[1])),
            );
        }
        if paths.is_empty() {
            return Ok(None);
        }
        Ok(Some(ScenePathMark {
            name: format!("{}/halo", self.name),
            interactive: Some(false),
            clip: self.clip,
            len: paths.len() as u32,
            path: ScalarOrArray::new_array(paths),
            fill: ScalarOrArray::new_array(fills),
            stroke: ColorOrGradient::Color([0.0; 4]).into(),
            stroke_width: None,
            transform: ScalarOrArray::new_array(transforms),
            zindex: self.zindex,
            ..Default::default()
        }))
    }

    pub fn text_iter(&self) -> Box<dyn Iterator<Item = &String> + '_> {
        self.text.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn align_iter(&self) -> Box<dyn Iterator<Item = &TextAlign> + '_> {
        self.align.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn baseline_iter(&self) -> Box<dyn Iterator<Item = &TextBaseline> + '_> {
        self.baseline
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.angle.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn color_iter(&self) -> Box<dyn Iterator<Item = &[f32; 4]> + '_> {
        self.color.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_iter(&self) -> Box<dyn Iterator<Item = &String> + '_> {
        self.font.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_size_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.font_size
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_weight_iter(&self) -> Box<dyn Iterator<Item = &FontWeight> + '_> {
        self.font_weight
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_style_iter(&self) -> Box<dyn Iterator<Item = &FontStyle> + '_> {
        self.font_style
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn width_iter(&self) -> Box<dyn Iterator<Item = &LabelWidth> + '_> {
        self.width.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn line_height_iter(&self) -> Box<dyn Iterator<Item = &LabelLineHeight> + '_> {
        self.line_height
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn line_align_iter(&self) -> Box<dyn Iterator<Item = &LabelAlign> + '_> {
        self.line_align
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    /// The mark's labels. A width or line height that the label crate would reject means none,
    /// since scene data shouldn't keep a label from drawing.
    pub fn labels(&self) -> impl Iterator<Item = TextLabel<'_>> + '_ {
        let paths: Box<dyn Iterator<Item = Option<PathOfLabel<'_>>>> = match &self.text_path {
            Some(paths) => Box::new(paths.of_labels(self.len as usize, self.indices.as_ref())),
            None => Box::new(std::iter::repeat(None)),
        };
        izip!(
            self.text_iter(),
            self.x_iter(),
            self.y_iter(),
            self.color_iter(),
            self.align_iter(),
            self.baseline_iter(),
            self.angle_iter(),
            self.font_iter(),
            self.font_size_iter(),
            self.font_weight_iter(),
            self.font_style_iter(),
            self.width_iter(),
            self.line_height_iter(),
            self.line_align_iter(),
            paths,
        )
        .map(
            move |(
                text,
                x,
                y,
                color,
                align,
                baseline,
                angle,
                font,
                font_size,
                font_weight,
                font_style,
                width,
                line_height,
                line_align,
                path,
            )| {
                let path = path.map(|path| path.text_path(*align, *baseline, *angle));
                let options = LabelOptions {
                    text: text_style(font, *font_size, *font_weight, *font_style, *color),
                    width: valid_width(*width),
                    wrap: self.wrap,
                    align: *line_align,
                    line_height: valid_line_height(*line_height),
                    max_lines: self.max_lines,
                    ellipsis: self.ellipsis,
                    hanging_signs: self.hanging_signs,
                    ..Default::default()
                };
                TextLabel {
                    label: Label {
                        path,
                        ..text_label(text, self.text_syntax, options)
                    },
                    position: [*x, *y],
                    align: *align,
                    baseline: *baseline,
                    angle: *angle,
                }
            },
        )
    }

    pub fn indices_iter(&self) -> Box<dyn Iterator<Item = usize> + '_> {
        if let Some(indices) = self.indices.as_ref() {
            Box::new(indices.iter().cloned())
        } else {
            Box::new(0..self.len as usize)
        }
    }
}

impl Default for SceneTextMark {
    fn default() -> Self {
        Self {
            interactive: None,
            name: "text_mark".to_string(),
            clip: true,
            len: 1,
            text: ScalarOrArray::new_scalar(String::new()),
            text_syntax: TextSyntaxMode::Plain,
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            align: ScalarOrArray::new_scalar(TextAlign::Left),
            baseline: ScalarOrArray::new_scalar(TextBaseline::Alphabetic),
            angle: ScalarOrArray::new_scalar(0.0),
            color: ScalarOrArray::new_scalar([0.0, 0.0, 0.0, 1.0]),
            font: ScalarOrArray::new_scalar("sans-serif".to_string()),
            font_size: ScalarOrArray::new_scalar(10.0),
            font_weight: ScalarOrArray::new_scalar(FontWeight::NORMAL),
            font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
            width: ScalarOrArray::new_scalar(LabelWidth::Auto),
            wrap: true,
            max_lines: None,
            ellipsis: false,
            hanging_signs: false,
            line_height: ScalarOrArray::new_scalar(LabelLineHeight::Auto),
            line_align: ScalarOrArray::new_scalar(LabelAlign::Start),
            indices: None,
            zindex: None,
            halo: None,
            text_path: None,
        }
    }
}

/// One label of a text mark: what it shows, and where it lies.
#[derive(Debug, Clone)]
pub struct TextLabel<'a> {
    pub label: Label<'a>,
    /// The position that `align` and `baseline` anchor, before rotation, or where the origin
    /// of the label's path lies.
    pub position: [f32; 2],
    pub align: TextAlign,
    pub baseline: TextBaseline,
    /// The rotation about the position, in degrees.
    pub angle: f32,
}

impl TextLabel<'_> {
    /// Return the label's placement in scene coordinates, using `bounds` and the group
    /// `origin`.
    pub fn placement(&self, bounds: &TextBounds, origin: [f32; 2]) -> LabelPlacement {
        let position = [self.position[0] + origin[0], self.position[1] + origin[1]];
        LabelPlacement::new(bounds, position, self.align, self.baseline, self.angle)
    }
}

impl From<SceneTextMark> for SceneMark {
    fn from(mark: SceneTextMark) -> Self {
        SceneMark::Text(Arc::new(mark))
    }
}

/// The label crate's text style for a font, size, weight, style and fill.
pub fn text_style(
    font: &str,
    font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
    color: [f32; 4],
) -> TextStyle {
    TextStyle {
        font_family: font.to_string(),
        font_size,
        fill: AbsoluteColor::from_rgba(color),
        font_weight: avenger_typst_label::FontWeight::from_number(font_weight.0),
        font_style: match font_style {
            FontStyle::Normal => avenger_typst_label::FontStyle::Normal,
            FontStyle::Italic => avenger_typst_label::FontStyle::Italic,
            FontStyle::Oblique => avenger_typst_label::FontStyle::Oblique,
        },
        ..Default::default()
    }
}

/// A label of a source that reads by a syntax: as literal text, whose newlines are spaces or
/// end lines, or as markup.
pub fn text_label(source: &str, syntax: TextSyntaxMode, options: LabelOptions) -> Label<'_> {
    let (source, newline_breaks) = match syntax {
        TextSyntaxMode::Plain => (LabelSource::Text(source), false),
        TextSyntaxMode::PlainLines => (LabelSource::Text(source), true),
        TextSyntaxMode::TypstMarkup => (LabelSource::Markup(source), false),
    };
    Label {
        source,
        options: LabelOptions {
            newline_breaks,
            ..options
        },
        path: None,
    }
}

/// The top left of a label's box, for a position that the alignment and baseline anchor.
///
/// Top, Middle and Bottom place the box, LineTop and LineBottom its line box, and Alphabetic
/// the first line's baseline. A label on a path lies with its path's origin at the position,
/// and its alignment and baseline have placed its text along the path already.
pub fn text_origin(
    bounds: &TextBounds,
    position: [f32; 2],
    align: TextAlign,
    baseline: TextBaseline,
) -> [f32; 2] {
    let (ascent, leading) = match bounds.anchor {
        BoxAnchor::Lines { ascent, leading } => (ascent, leading),
        BoxAnchor::Path(origin) => return [position[0] - origin.x, position[1] - origin.y],
    };
    let x = match align {
        TextAlign::Left => position[0],
        TextAlign::Center => position[0] - bounds.width / 2.0,
        TextAlign::Right => position[0] - bounds.width,
    };
    let y = match baseline {
        TextBaseline::Alphabetic => position[1] - ascent,
        TextBaseline::Top => position[1],
        TextBaseline::Middle => position[1] - bounds.height / 2.0,
        TextBaseline::Bottom => position[1] - bounds.height,
        TextBaseline::LineTop => position[1] + leading / 2.0,
        TextBaseline::LineBottom => position[1] - bounds.height - leading / 2.0,
    };
    [x, y]
}

/// A label's placement in scene coordinates, matching how renderers draw it. Hit-test geometry
/// and text selection use it. The label box rotates about its anchor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelPlacement {
    /// Top-left corner of the label box in scene coordinates, before rotation.
    pub origin: [f32; 2],
    /// Rotation anchor at the label's position in scene coordinates.
    pub anchor: [f32; 2],
    /// The rotation about the anchor, in degrees.
    pub angle: f32,
}

impl LabelPlacement {
    /// Place a label at `position` using its bounds, alignment, baseline, and angle in degrees.
    pub fn new(
        bounds: &TextBounds,
        position: [f32; 2],
        align: TextAlign,
        baseline: TextBaseline,
        angle: f32,
    ) -> Self {
        Self {
            origin: text_origin(bounds, position, align, baseline),
            anchor: position,
            angle,
        }
    }

    /// Transform `point` from label-box coordinates to scene coordinates.
    pub fn to_scene(&self, point: [f32; 2]) -> [f32; 2] {
        let point = [self.origin[0] + point[0], self.origin[1] + point[1]];
        rotate(point, self.anchor, self.angle)
    }

    /// Transform `point` from scene coordinates to label-box coordinates.
    pub fn to_label(&self, point: [f32; 2]) -> [f32; 2] {
        let [x, y] = rotate(point, self.anchor, -self.angle);
        [x - self.origin[0], y - self.origin[1]]
    }

    /// Transform a label-box rectangle `[x, y, width, height]` to scene coordinates.
    /// Return its corners clockwise from the top left.
    pub fn corners(&self, [x, y, width, height]: [f32; 4]) -> [[f32; 2]; 4] {
        let (right, bottom) = (x + width, y + height);
        [[x, y], [right, y], [right, bottom], [x, bottom]].map(|corner| self.to_scene(corner))
    }
}

/// Rotate `point` around `center` by `degrees`, clockwise on screen, where y points down.
fn rotate(point: [f32; 2], center: [f32; 2], degrees: f32) -> [f32; 2] {
    if degrees == 0.0 {
        return point;
    }
    let (sin, cos) = degrees.to_radians().sin_cos();
    let [dx, dy] = [point[0] - center[0], point[1] - center[1]];
    [
        center[0] + dx * cos - dy * sin,
        center[1] + dx * sin + dy * cos,
    ]
}

/// A width the label crate accepts: one it would reject means none.
fn valid_width(width: LabelWidth) -> LabelWidth {
    match width {
        LabelWidth::Max(width) | LabelWidth::Fixed(width) if !valid(width) => LabelWidth::Auto,
        width => width,
    }
}

/// A line height the label crate accepts: one it would reject means none.
fn valid_line_height(line_height: LabelLineHeight) -> LabelLineHeight {
    match line_height {
        LabelLineHeight::Fixed(value) | LabelLineHeight::Relative(value) if !valid(value) => {
            LabelLineHeight::Auto
        }
        line_height => line_height,
    }
}

fn valid(distance: f32) -> bool {
    distance.is_finite() && distance >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_take_the_layout_and_syntax() {
        let mark = SceneTextMark {
            text: "a\nb".into(),
            text_syntax: TextSyntaxMode::PlainLines,
            width: LabelWidth::Fixed(80.0).into(),
            wrap: false,
            max_lines: NonZeroUsize::new(2),
            ellipsis: true,
            hanging_signs: true,
            line_height: LabelLineHeight::Relative(1.1).into(),
            line_align: LabelAlign::Center.into(),
            ..Default::default()
        };
        let label = mark.labels().next().unwrap().label;
        assert_eq!(label.source, LabelSource::Text("a\nb"));
        let options = &label.options;
        assert_eq!(
            (
                options.width,
                options.wrap,
                options.max_lines,
                options.ellipsis,
                options.hanging_signs
            ),
            (
                LabelWidth::Fixed(80.0),
                false,
                NonZeroUsize::new(2),
                true,
                true
            )
        );
        assert_eq!(
            (options.line_height, options.align, options.newline_breaks),
            (LabelLineHeight::Relative(1.1), LabelAlign::Center, true)
        );
        for (syntax, source, newline_breaks) in [
            (TextSyntaxMode::Plain, LabelSource::Text("x"), false),
            (TextSyntaxMode::TypstMarkup, LabelSource::Markup("x"), false),
        ] {
            let label = text_label("x", syntax, LabelOptions::default());
            assert_eq!(
                (label.source, label.options.newline_breaks),
                (source, newline_breaks)
            );
        }

        // Distances the label crate would reject mean none.
        let invalid = SceneTextMark {
            width: LabelWidth::Max(f32::NAN).into(),
            line_height: LabelLineHeight::Fixed(-1.0).into(),
            ..Default::default()
        };
        let options = invalid.labels().next().unwrap().label.options;
        assert_eq!(
            (options.width, options.line_height),
            (LabelWidth::Auto, LabelLineHeight::Auto)
        );
    }

    /// A rotated placement maps points between scene and label coordinates and rotates the box
    /// around the label position.
    #[test]
    fn placements_map_points_into_the_box_and_back() {
        let bounds = TextBounds {
            width: 40.0,
            height: 12.0,
            anchor: BoxAnchor::Lines {
                ascent: 9.0,
                leading: 4.0,
            },
        };
        let placement = LabelPlacement::new(
            &bounds,
            [100.0, 50.0],
            TextAlign::Left,
            TextBaseline::Top,
            90.0,
        );
        let corner = placement.to_scene([0.0, 0.0]);
        assert!((corner[0] - 100.0).abs() < 1e-4 && (corner[1] - 50.0).abs() < 1e-4);
        let along = placement.to_scene([10.0, 0.0]);
        assert!((along[0] - 100.0).abs() < 1e-4 && (along[1] - 60.0).abs() < 1e-4);
        let back = placement.to_label(placement.to_scene([12.0, 5.0]));
        assert!((back[0] - 12.0).abs() < 1e-4 && (back[1] - 5.0).abs() < 1e-4);
    }

    /// The extent of points: `[left, top, right, bottom]`.
    fn extent(points: impl IntoIterator<Item = [f32; 2]>) -> [f32; 4] {
        points.into_iter().fold(
            [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ],
            |[x0, y0, x1, y1], [x, y]| [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        )
    }

    /// A halo grows each label's outline by its width, in the label's placement, and skips
    /// labels without glyphs.
    #[test]
    fn halos_grow_each_labels_outline_where_the_label_sits() {
        let engine = LabelEngine::new(avenger_typst_label::EngineOptions {
            fonts: avenger_typst_label::FontOptions {
                load_system_fonts: false,
                ..avenger_typst_label::bundled_font_options()
            },
        });
        // The middle label has no glyphs, so no halo.
        let colors = [
            [1.0, 1.0, 1.0, 0.5],
            [0.0, 0.0, 0.0, 1.0],
            [0.2, 0.4, 0.6, 0.5],
        ];
        let halo = TextHalo {
            color: ScalarOrArray::new_array(colors.to_vec()),
            width: 2.0,
        };
        let mark = SceneTextMark {
            name: "labels".into(),
            len: 3,
            text: ScalarOrArray::new_array(vec!["Ag".into(), " ".into(), "Ag".into()]),
            x: 100.0.into(),
            y: 50.0.into(),
            baseline: TextBaseline::Top.into(),
            angle: ScalarOrArray::new_array(vec![0.0, 0.0, 90.0]),
            halo: Some(halo.clone()),
            ..Default::default()
        };
        let halo_mark = mark.halo_mark(&engine).unwrap().unwrap();
        assert_eq!((halo_mark.name.as_str(), halo_mark.len), ("labels/halo", 2));
        assert_eq!(halo_mark.interactive, Some(false));
        assert_eq!(
            halo_mark.fill,
            ScalarOrArray::new_array(vec![
                ColorOrGradient::Color(colors[0]),
                ColorOrGradient::Color(colors[2]),
            ])
        );

        let label = mark.labels().next().unwrap().label;
        let [left, top, right, bottom] = extent(
            engine
                .outline(&label)
                .unwrap()
                .contours
                .iter()
                .flatten()
                .copied(),
        );
        let paths = halo_mark.path_iter().zip(halo_mark.transform_iter());
        let extents: Vec<_> = paths
            .map(|(path, transform)| {
                extent(path.iter().filter_map(|event| match event {
                    lyon_path::Event::Line { to, .. } => {
                        let at = transform.transform_point(to);
                        Some([at.x, at.y])
                    }
                    _ => None,
                }))
            })
            .collect();
        let w = halo.width;
        // The label's box has its top left at the position, and the turned label turns about it.
        let expected = [
            [
                100.0 + left - w,
                50.0 + top - w,
                100.0 + right + w,
                50.0 + bottom + w,
            ],
            [
                100.0 - bottom - w,
                50.0 + left - w,
                100.0 - top + w,
                50.0 + right + w,
            ],
        ];
        for (extent, expected) in extents.iter().zip(expected) {
            let close = extent
                .iter()
                .zip(expected)
                .all(|(a, b)| (a - b).abs() < 0.05);
            assert!(close, "{extent:?} != {expected:?}");
        }

        // A halo without width or opacity draws nothing.
        for halo in [
            None,
            Some(TextHalo {
                width: 0.0,
                ..halo.clone()
            }),
            Some(TextHalo {
                color: [1.0, 1.0, 1.0, 0.0].into(),
                ..halo.clone()
            }),
        ] {
            let mark = SceneTextMark {
                halo: halo.clone(),
                ..mark.clone()
            };
            assert!(mark.halo_mark(&engine).unwrap().is_none(), "{halo:?}");
        }
    }

    /// Contrast text takes the light color up to `light_up_to` and the dark one past it, and a
    /// contrast halo takes the background with its lightness moved just far enough for the text to
    /// reach the minimum ratio for dark or light text.
    #[test]
    fn contrast_colors_follow_each_background() {
        let (dark, light) = ([0.1, 0.1, 0.1, 1.0], [0.98, 0.98, 0.98, 1.0]);
        // Oklch lightnesses 0.29, 0.64, and 0.92.
        let fills = [
            [0.27, 0.0, 0.33, 1.0],
            [0.88, 0.34, 0.35, 1.0],
            [0.99, 0.91, 0.15, 1.0],
        ];
        let backgrounds = ScalarOrArray::new_array(fills.to_vec());
        let texts = contrast_text_colors(&backgrounds, dark, light, 0.5);
        assert_eq!(texts, ScalarOrArray::new_array(vec![light, dark, dark]));
        assert_eq!(
            contrast_text_colors(&backgrounds, dark, light, 0.7),
            ScalarOrArray::new_array(vec![light, light, dark])
        );

        let halos = contrast_halo_colors(&texts, &backgrounds, 7.0, 16.0).as_vec(3, None);
        let ratio = |a: [f32; 4], b: [f32; 4]| {
            avenger_color::contrast_ratio(
                &AbsoluteColor::from_rgba(a),
                &AbsoluteColor::from_rgba(b),
            )
        };
        let hue = |color: [f32; 4]| {
            AbsoluteColor::from_rgba(color)
                .to_color_space(ColorSpace::Oklch)
                .components[2]
        };
        // The purple, at 14.6:1 with the light text, darkens to 16:1. The yellow has 7:1 with
        // the dark text already, and the red lightens to it, keeping its hue.
        assert!(ratio(fills[0], light) < 16.0);
        assert!((16.0..16.01).contains(&ratio(halos[0], light)), "{halos:?}");
        assert!(halos[0][2] < fills[0][2]);
        assert_eq!(halos[2], fills[2]);
        assert!(ratio(fills[1], dark) < 7.0);
        assert!((7.0..7.01).contains(&ratio(halos[1], dark)), "{halos:?}");
        assert!(halos[1][1] > fills[1][1]);
        assert!((hue(halos[1]) - hue(fills[1])).abs() < 0.5, "{halos:?}");

        // A scalar text and background give a scalar halo color.
        let scalar = contrast_halo_colors(&dark.into(), &fills[1].into(), 7.0, 16.0);
        assert_eq!(scalar, ScalarOrArray::new_scalar(halos[1]));
    }

    #[test]
    fn text_origins_place_the_box_line_box_or_first_baseline() {
        let bounds = TextBounds {
            width: 100.0,
            height: 20.0,
            anchor: BoxAnchor::Lines {
                ascent: 15.0,
                leading: 4.0,
            },
        };
        let origin = |align, baseline| text_origin(&bounds, [10.0, 10.0], align, baseline);
        assert_eq!(origin(TextAlign::Left, TextBaseline::Top), [10.0, 10.0]);
        assert_eq!(
            origin(TextAlign::Center, TextBaseline::Middle),
            [-40.0, 0.0]
        );
        assert_eq!(
            origin(TextAlign::Right, TextBaseline::Bottom),
            [-90.0, -10.0]
        );
        assert_eq!(
            origin(TextAlign::Left, TextBaseline::Alphabetic),
            [10.0, -5.0]
        );
        // The line box has half the leading above the box and half below.
        assert_eq!(origin(TextAlign::Left, TextBaseline::LineTop), [10.0, 12.0]);
        assert_eq!(
            origin(TextAlign::Left, TextBaseline::LineBottom),
            [10.0, -12.0]
        );
        // A label on a path lies with its path's origin at the position, whatever its
        // alignment and baseline.
        let bent = TextBounds {
            anchor: BoxAnchor::Path(avenger_typst_label::Point::new(5.0, 7.0)),
            ..bounds
        };
        for (align, baseline) in [
            (TextAlign::Left, TextBaseline::Top),
            (TextAlign::Right, TextBaseline::Alphabetic),
        ] {
            assert_eq!(
                text_origin(&bent, [10.0, 10.0], align, baseline),
                [5.0, 3.0]
            );
        }
    }

    /// A path of a line, a quadratic, a cubic, and a close.
    fn every_segment() -> lyon_path::Path {
        use lyon_path::math::point;
        let mut builder = lyon_path::Path::builder();
        builder.begin(point(0.0, 0.0));
        builder.line_to(point(10.0, 0.0));
        builder.quadratic_bezier_to(point(15.0, 0.0), point(15.0, 5.0));
        builder.cubic_bezier_to(point(15.0, 8.0), point(12.0, 10.0), point(10.0, 10.0));
        builder.end(true);
        builder.build()
    }

    /// Each label's path carries the mark's alignment as its anchor, its baseline, and, with
    /// `upright`, its angle for the upright test, with the path's segments one for one.
    #[test]
    fn labels_follow_their_paths() {
        use avenger_typst_label::{CurveItem, Point};
        let mark = SceneTextMark {
            len: 2,
            text: ScalarOrArray::new_array(vec!["a".into(), "b".into()]),
            align: ScalarOrArray::new_array(vec![TextAlign::Center, TextAlign::Right]),
            baseline: TextBaseline::Bottom.into(),
            angle: ScalarOrArray::new_array(vec![0.0, 30.0]),
            text_path: Some(TextPaths {
                path: every_segment().into(),
                offset: ScalarOrArray::new_array(vec![
                    PathOffset::Distance(5.0),
                    PathOffset::Fraction(0.5),
                ]),
                shift: 2.0.into(),
                upright: true,
                overflow: PathOverflow::Fit,
            }),
            ..Default::default()
        };
        let paths: Vec<TextPath> = mark
            .labels()
            .map(|label| label.label.path.unwrap())
            .collect();
        assert_eq!(
            paths[0].path.0,
            [
                CurveItem::Move(Point::new(0.0, 0.0)),
                CurveItem::Line(Point::new(10.0, 0.0)),
                CurveItem::Quad(Point::new(15.0, 0.0), Point::new(15.0, 5.0)),
                CurveItem::Cubic(
                    Point::new(15.0, 8.0),
                    Point::new(12.0, 10.0),
                    Point::new(10.0, 10.0)
                ),
                CurveItem::Close,
            ]
        );
        let summary = |path: &TextPath| {
            (
                path.offset,
                path.anchor,
                path.baseline,
                path.shift,
                path.upright,
            )
        };
        assert_eq!(
            summary(&paths[0]),
            (
                PathOffset::Distance(5.0),
                PathAnchor::Middle,
                PathBaseline::Bottom,
                2.0,
                Some(0.0)
            )
        );
        assert_eq!(
            summary(&paths[1]),
            (
                PathOffset::Fraction(0.5),
                PathAnchor::End,
                PathBaseline::Bottom,
                2.0,
                Some(30.0)
            )
        );
        assert!(paths.iter().all(|path| path.overflow == PathOverflow::Fit));
    }

    /// The arc of `radius` about the origin from -60 to 60 degrees clockwise from the top.
    fn top_arc(radius: f32) -> lyon_path::Path {
        use lyon_path::math::{point, vector, Angle};
        let mut builder = lyon_path::Path::builder().with_svg();
        let start = (-150.0_f32).to_radians();
        builder.move_to(point(radius * start.cos(), radius * start.sin()));
        builder.arc(
            point(0.0, 0.0),
            vector(radius, radius),
            Angle::degrees(120.0),
            Angle::zero(),
        );
        builder.build()
    }

    /// A mark whose labels follow an arc, with a halo.
    fn arc_mark() -> SceneTextMark {
        SceneTextMark {
            text: "Along the arc".to_string().into(),
            x: 100.0.into(),
            y: 100.0.into(),
            font_size: 12.0.into(),
            align: TextAlign::Center.into(),
            baseline: TextBaseline::Middle.into(),
            halo: Some(TextHalo {
                color: [1.0, 1.0, 1.0, 1.0].into(),
                width: 2.0,
            }),
            text_path: Some(TextPaths {
                path: top_arc(60.0).into(),
                offset: PathOffset::Fraction(0.5).into(),
                shift: 0.0.into(),
                upright: true,
                overflow: PathOverflow::Extend,
            }),
            ..Default::default()
        }
    }

    /// A halo of a label on a path grows the bent label where it lies, along the path.
    #[test]
    fn halos_follow_labels_on_paths() {
        let engine = LabelEngine::new(avenger_typst_label::EngineOptions {
            fonts: avenger_typst_label::FontOptions {
                load_system_fonts: false,
                ..avenger_typst_label::bundled_font_options()
            },
        });
        let mark = arc_mark();
        let halo = mark.halo_mark(&engine).unwrap().unwrap();
        let (path, transform) = halo.path_iter().zip(halo.transform_iter()).next().unwrap();
        for event in path.iter() {
            if let lyon_path::Event::Line { to, .. } = event {
                let at = transform.transform_point(to);
                let distance = (at.x - 100.0).hypot(at.y - 100.0);
                assert!((50.0..70.0).contains(&distance), "{distance}");
            }
        }
    }

    /// A mark whose labels follow paths survives a round trip through serde, with its lyon paths
    /// and the paths' offsets, which compare by hand.
    #[test]
    fn marks_with_paths_round_trip_through_serde() {
        let mark = arc_mark();
        let json = serde_json::to_string(&mark).unwrap();
        let back: SceneTextMark = serde_json::from_str(&json).unwrap();
        assert_eq!(back, mark);
    }
}
