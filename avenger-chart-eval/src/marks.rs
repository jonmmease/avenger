//! The scene marks of the 10 Cartesian marks: one function for each mark, from the encoded rows of
//! its layer, at the plot's size.

use std::{num::NonZeroUsize, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray},
    compute::cast,
    datatypes::DataType,
};
use avenger_chart::{
    layer::{LayerRef, MarkType},
    proto::{self, cartesian_layer, geo_layer, polar_layer, Channel},
};
use avenger_color::ColorOrGradient;
use avenger_common::{
    pattern::{PatternFill, PatternFills},
    types::{
        AreaOrientation, FontStyle, FontWeight, ImageAlign, ImageBaseline, PathTransform,
        SymbolShape, TextAlign, TextBaseline, TextSyntaxMode,
    },
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_format::PrecisionSharing;
use avenger_scales::scales::{coerce::Coercer, ConfiguredScale};
use avenger_scenegraph::marks::{
    area::SceneAreaMark,
    image::{SceneImageMark, SceneImageResource, SceneImageSource},
    line::SceneLineMark,
    mark::SceneMark,
    path::ScenePathMark,
    rect::SceneRectMark,
    rule::SceneRuleMark,
    symbol::SceneSymbolMark,
    text::SceneTextMark,
    trail::SceneTrailMark,
};
use avenger_style::{Computed, ComputedStyle, Property};
use avenger_typst_label::{escape_text, LabelValue, LabelWidth};
use lyon_path::geom::{euclid::Vector2D, Angle};

use crate::{
    encode::{at, scalar, subset, LayerRows, Result},
    style::{color as style_color, number as style_number},
    EvalError,
};

/// A scene mark, with the rows of its layer's table that draw its instances, in their order.
pub(crate) struct Drawn {
    pub(crate) mark: SceneMark,
    pub(crate) rows: arrow::array::RecordBatch,
}

/// The scene marks of a layer of a Cartesian plot, whose mark is `mark`.
pub(crate) fn marks(rows: &LayerRows, mark: &cartesian_layer::Mark) -> Result<Vec<Drawn>> {
    use cartesian_layer::Mark;
    match mark {
        Mark::Symbol(_) => symbols(rows),
        Mark::Line(_) => lines(rows),
        Mark::Trail(_) => trails(rows),
        Mark::Area(area) => areas(rows, area.orient),
        Mark::Bar(bar) => bars(rows, bar.orient),
        Mark::Rect(_) => rects(rows),
        Mark::Rule(_) => rules(rows),
        Mark::Text(text) => texts(rows, TextFormat::of(text)),
        Mark::Image(_) => images(rows),
        Mark::Path(_) => paths(rows),
    }
}

/// The content fields of a text mark that the texts of its rows read: the pattern of numeric and
/// temporal values and the syntax of the texts.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TextFormat<'a> {
    pub(crate) format: &'a str,
    pub(crate) syntax: i32,
}

impl<'a> TextFormat<'a> {
    fn of(text: &'a proto::TextMark) -> Self {
        Self {
            format: &text.format,
            syntax: text.syntax,
        }
    }

    /// The format of the text mark of `layer`, or `None` for a layer of another mark.
    pub(crate) fn of_layer(layer: LayerRef<'a>) -> Option<Self> {
        match layer {
            LayerRef::Cartesian(layer) => match layer.mark.as_ref()? {
                cartesian_layer::Mark::Text(text) => Some(Self::of(text)),
                _ => None,
            },
            LayerRef::Polar(layer) => match layer.mark.as_ref()? {
                polar_layer::Mark::Text(text) => Some(Self {
                    format: &text.format,
                    syntax: text.syntax,
                }),
                _ => None,
            },
            LayerRef::Geo(layer) => match layer.mark.as_ref()? {
                geo_layer::Mark::Text(text) => Some(Self {
                    format: &text.format,
                    syntax: text.syntax,
                }),
                _ => None,
            },
        }
    }
}

/// The patterns of each row as a scene mark takes them.
pub(crate) fn pattern_fills(patterns: Option<ScalarOrArray<Option<PatternFill>>>) -> PatternFills {
    match patterns {
        Some(patterns) => PatternFills(patterns),
        None => PatternFills::default(),
    }
}

/// The patterns of each row: the `pattern` channel, or the computed `pattern` of the mark node.
pub(crate) fn row_patterns(rows: &LayerRows) -> Result<Option<ScalarOrArray<Option<PatternFill>>>> {
    if let Some(patterns) = rows.patterns(Channel::Pattern)? {
        return Ok(Some(patterns));
    }
    Ok(match rows.style.string(Property::Pattern) {
        Some(text) => Some(scalar(Some(text.parse::<PatternFill>().map_err(
            |error| EvalError::Unsupported(format!("the pattern `{text}`: {error}")),
        )?))),
        None => None,
    })
}

/// The distinct shapes of `names`, and the index of each row's shape among them.
pub(crate) fn shapes(
    names: &ScalarOrArray<String>,
) -> Result<(Vec<SymbolShape>, ScalarOrArray<usize>)> {
    let parse = |name: &str| {
        SymbolShape::from_vega_str(name)
            .map_err(|error| EvalError::Unsupported(format!("the shape `{name}`: {error:?}")))
    };
    match names.value() {
        ScalarOrArrayValue::Scalar(name) => Ok((vec![parse(name)?], scalar(0))),
        ScalarOrArrayValue::Array(names) => {
            let mut distinct: Vec<&str> = Vec::new();
            let mut indices = Vec::with_capacity(names.len());
            for name in names.iter() {
                let index = match distinct.iter().position(|known| *known == name) {
                    Some(index) => index,
                    None => {
                        distinct.push(name);
                        distinct.len() - 1
                    }
                };
                indices.push(index);
            }
            let shapes = distinct
                .into_iter()
                .map(parse)
                .collect::<Result<Vec<_>>>()?;
            Ok((shapes, ScalarOrArray::new_array(indices)))
        }
    }
}

/// The names of the shapes of each row: the `shape` channel, or the computed `shape`.
pub(crate) fn shape_names(rows: &LayerRows) -> Result<ScalarOrArray<String>> {
    Ok(match rows.strings(Channel::Shape)? {
        Some(names) => names,
        None => scalar(
            (rows.style.string(Property::Shape))
                .expect("`shape` computes to a string")
                .to_string(),
        ),
    })
}

/// The stroke width of a mark whose stroke shows, or none.
pub(crate) fn stroke_width(rows: &LayerRows) -> Result<Option<f32>> {
    Ok(match rows.strokes()? {
        true => Some(style_number(rows.style, Property::StrokeWidth)),
        false => None,
    })
}

fn symbols(rows: &LayerRows) -> Result<Vec<Drawn>> {
    let mark = symbol_mark(rows, rows.position(Channel::X)?, rows.position(Channel::Y)?)?;
    Ok(vec![Drawn {
        mark: mark.into(),
        rows: rows.table.clone(),
    }])
}

/// The symbol mark of a layer's rows at `x` and `y`.
pub(crate) fn symbol_mark(
    rows: &LayerRows,
    x: ScalarOrArray<f32>,
    y: ScalarOrArray<f32>,
) -> Result<SceneSymbolMark> {
    let (fill, stroke) = rows.paint()?;
    let (shapes, shape_index) = shapes(&shape_names(rows)?)?;
    Ok(SceneSymbolMark {
        name: rows.name.clone(),
        len: rows.len() as u32,
        x,
        y,
        fill,
        stroke,
        stroke_width: match rows.numbers(Channel::StrokeWidth)? {
            Some(width) => width,
            None => scalar(stroke_width(rows)?.unwrap_or(0.0)),
        },
        // A percentage `size` is the diameter of a symbol as a fraction of its band.
        size: match (rows.band_size(), rows.size_bands()?) {
            (Some(fraction), Some(bands)) => bands.map(|(_, width)| (fraction * width).powi(2)),
            _ => rows.numbers_or(Channel::Size, Property::Size)?,
        },
        shapes,
        shape_index,
        angle: rows.numbers_or(Channel::Angle, Property::Angle)?,
        fill_pattern: pattern_fills(row_patterns(rows)?),
        ..Default::default()
    })
}

fn lines(rows: &LayerRows) -> Result<Vec<Drawn>> {
    let x = rows.position(Channel::X)?;
    let y = rows.position(Channel::Y)?;
    let (_, stroke) = rows.paint()?;
    let width = rows.numbers_or(Channel::StrokeWidth, Property::StrokeWidth)?;
    let dash = rows.dashes_or_style()?;
    (rows.series()?.into_iter().enumerate())
        .map(|(index, series)| {
            let first = series[0] as usize;
            let mark = SceneLineMark {
                name: format!("{}/{index}", rows.name),
                len: series.len() as u32,
                x: subset(&x, &series),
                y: subset(&y, &series),
                stroke: at(&stroke, first),
                stroke_width: at(&width, first),
                stroke_dash: (dash.as_ref())
                    .map(|dash| at(dash, first))
                    .filter(|dash| !dash.is_empty()),
                ..Default::default()
            };
            Ok(Drawn {
                mark: mark.into(),
                rows: rows.rows(&series)?,
            })
        })
        .collect()
}

fn trails(rows: &LayerRows) -> Result<Vec<Drawn>> {
    let x = rows.position(Channel::X)?;
    let y = rows.position(Channel::Y)?;
    let (_, stroke) = rows.paint()?;
    let size = rows.numbers_or(Channel::Size, Property::Size)?;
    (rows.series()?.into_iter().enumerate())
        .map(|(index, series)| {
            let first = series[0] as usize;
            let mark = SceneTrailMark {
                name: format!("{}/{index}", rows.name),
                len: series.len() as u32,
                x: subset(&x, &series),
                y: subset(&y, &series),
                size: subset(&size, &series),
                stroke: at(&stroke, first),
                ..Default::default()
            };
            Ok(Drawn {
                mark: mark.into(),
                rows: rows.rows(&series)?,
            })
        })
        .collect()
}

fn areas(rows: &LayerRows, orient: i32) -> Result<Vec<Drawn>> {
    let horizontal = rows.horizontal(orient);
    let (along, across, across2) = match horizontal {
        true => (Channel::Y, Channel::X, Channel::X2),
        false => (Channel::X, Channel::Y, Channel::Y2),
    };
    let along_values = rows.position(along)?;
    let across_values = rows.position(across)?;
    let base = match rows.has(across2) {
        true => rows.position(across2)?,
        false => scalar(rows.baseline(across)?),
    };
    let (fill, stroke) = rows.paint()?;
    let width = match rows.numbers(Channel::StrokeWidth)? {
        Some(width) => width,
        None => scalar(stroke_width(rows)?.unwrap_or(0.0)),
    };
    let dash = rows.dashes_or_style()?;
    let pattern = row_patterns(rows)?;
    (rows.series()?.into_iter().enumerate())
        .map(|(index, series)| {
            let first = series[0] as usize;
            let (x, y, x2, y2) = match horizontal {
                true => (
                    subset(&across_values, &series),
                    subset(&along_values, &series),
                    subset(&base, &series),
                    scalar(0.0),
                ),
                false => (
                    subset(&along_values, &series),
                    subset(&across_values, &series),
                    scalar(0.0),
                    subset(&base, &series),
                ),
            };
            let mark = SceneAreaMark {
                name: format!("{}/{index}", rows.name),
                len: series.len() as u32,
                orientation: match horizontal {
                    true => AreaOrientation::Horizontal,
                    false => AreaOrientation::Vertical,
                },
                x,
                y,
                x2,
                y2,
                fill: at(&fill, first),
                fill_pattern: pattern.as_ref().and_then(|pattern| at(pattern, first)),
                stroke: at(&stroke, first),
                stroke_width: at(&width, first),
                stroke_dash: (dash.as_ref())
                    .map(|dash| at(dash, first))
                    .filter(|dash| !dash.is_empty()),
                ..Default::default()
            };
            Ok(Drawn {
                mark: mark.into(),
                rows: rows.rows(&series)?,
            })
        })
        .collect()
}

/// The pixel span of each row along `channel`: from its position to the position of `second`,
/// across its band on a band or a nested band scale without `second`, the computed
/// `continuous-size` about its position on a continuous key without `second`, and the whole
/// `length` without the channel.
fn span(
    rows: &LayerRows,
    channel: Channel,
    second: Channel,
    length: f32,
) -> Result<(ScalarOrArray<f32>, ScalarOrArray<f32>)> {
    if !rows.has(channel) {
        return Ok((scalar(0.0), scalar(length)));
    }
    if rows.has(second) {
        return Ok((rows.position(channel)?, rows.position(second)?));
    }
    if let Some(bands) = rows.bands(channel)? {
        // A percentage `size` is the thickness across the band, at the mark's band position.
        if let Some(fraction) = rows.band_size() {
            let position = rows.band_position(channel);
            let side = |sign: f32| {
                bands.map(move |(start, width)| {
                    start + width * position + sign * fraction * width / 2.0
                })
            };
            return Ok((side(-1.0), side(1.0)));
        }
        return Ok((
            bands.map(|(start, _)| *start),
            bands.map(|(start, width)| start + width),
        ));
    }
    let start = rows.numbers(channel)?.unwrap_or_else(|| scalar(0.0));
    let half = style_number(rows.style, Property::ContinuousSize) / 2.0;
    Ok((
        start.clone().map(|value| value - half),
        start.map(|value| value + half),
    ))
}

/// A rect mark of the spans of each row, with `inset` given up at each side.
fn rect_mark(
    rows: &LayerRows,
    (x, x2): (ScalarOrArray<f32>, ScalarOrArray<f32>),
    (y, y2): (ScalarOrArray<f32>, ScalarOrArray<f32>),
    inset: f32,
) -> Result<Vec<Drawn>> {
    let (fill, stroke) = rows.paint()?;
    let (x, x2) = inset_spans(x, x2, inset, rows.len());
    let (y, y2) = inset_spans(y, y2, inset, rows.len());
    let width = match rows.numbers(Channel::StrokeWidth)? {
        Some(width) => width,
        None => scalar(stroke_width(rows)?.unwrap_or(0.0)),
    };
    let mark = SceneRectMark {
        name: rows.name.clone(),
        len: rows.len() as u32,
        x,
        y,
        x2: Some(x2),
        y2: Some(y2),
        fill,
        stroke,
        stroke_width: width,
        corner_radius: scalar(style_number(rows.style, Property::CornerRadius)),
        fill_pattern: pattern_fills(row_patterns(rows)?),
        ..Default::default()
    };
    Ok(vec![Drawn {
        mark: mark.into(),
        rows: rows.table.clone(),
    }])
}

/// The spans from `start` to `end`, ordered and with `inset` given up at each end, but never past
/// their middle.
fn inset_spans(
    start: ScalarOrArray<f32>,
    end: ScalarOrArray<f32>,
    inset: f32,
    len: usize,
) -> (ScalarOrArray<f32>, ScalarOrArray<f32>) {
    let pair = |a: f32, b: f32| {
        let (low, high) = (a.min(b), a.max(b));
        let inset = inset.min((high - low) / 2.0).max(0.0);
        (low + inset, high - inset)
    };
    match (start.value(), end.value()) {
        (ScalarOrArrayValue::Scalar(a), ScalarOrArrayValue::Scalar(b)) => {
            let (low, high) = pair(*a, *b);
            (scalar(low), scalar(high))
        }
        _ => {
            let (low, high): (Vec<f32>, Vec<f32>) = (0..len)
                .map(|row| pair(at(&start, row), at(&end, row)))
                .unzip();
            (
                ScalarOrArray::new_array(low),
                ScalarOrArray::new_array(high),
            )
        }
    }
}

fn bars(rows: &LayerRows, orient: i32) -> Result<Vec<Drawn>> {
    let horizontal = rows.horizontal(orient);
    let (key, key2, measure, measure2) = match horizontal {
        true => (Channel::Y, Channel::Y2, Channel::X, Channel::X2),
        false => (Channel::X, Channel::X2, Channel::Y, Channel::Y2),
    };
    let key_length = match horizontal {
        true => rows.size[1],
        false => rows.size[0],
    };
    let keys = span(rows, key, key2, key_length)?;
    // A bar that spans its bin, from a continuous key to the key's partner, gives up half of
    // `bin-spacing` at each edge along its key, in pixels after the scale, so neighboring bins
    // stand apart and each bar stays centered on its bin.
    let binned = rows.has(key2) && rows.kind(key) == Some(avenger_chart::FitKind::Interval);
    let keys = match binned {
        true => {
            let spacing = style_number(rows.style, Property::BinSpacing);
            inset_spans(keys.0, keys.1, spacing / 2.0, rows.len())
        }
        false => keys,
    };
    let start = rows.numbers(measure)?.unwrap_or_else(|| scalar(0.0));
    let end = match rows.has(measure2) {
        true => rows.numbers(measure2)?.unwrap_or_else(|| scalar(0.0)),
        false => scalar(rows.baseline(measure)?),
    };
    let measures = (start, end);
    let (xs, ys) = match horizontal {
        true => (measures, keys),
        false => (keys, measures),
    };
    rect_mark(rows, xs, ys, 0.0)
}

fn rects(rows: &LayerRows) -> Result<Vec<Drawn>> {
    let xs = span(rows, Channel::X, Channel::X2, rows.size[0])?;
    let ys = span(rows, Channel::Y, Channel::Y2, rows.size[1])?;
    let inset = style_number(rows.style, Property::Inset);
    rect_mark(rows, xs, ys, inset)
}

fn rules(rows: &LayerRows) -> Result<Vec<Drawn>> {
    let line = |channel: Channel, second: Channel, length: f32| -> Result<_> {
        Ok(match (rows.has(channel), rows.has(second)) {
            (false, _) => (scalar(0.0), scalar(length)),
            (true, false) => {
                let position = rows.position(channel)?;
                (position.clone(), position)
            }
            (true, true) => (rows.position(channel)?, rows.position(second)?),
        })
    };
    let (x, x2) = line(Channel::X, Channel::X2, rows.size[0])?;
    let (y, y2) = line(Channel::Y, Channel::Y2, rows.size[1])?;
    let (_, stroke) = rows.paint()?;
    let mark = SceneRuleMark {
        name: rows.name.clone(),
        len: rows.len() as u32,
        x,
        y,
        x2,
        y2,
        stroke,
        stroke_width: rows.numbers_or(Channel::StrokeWidth, Property::StrokeWidth)?,
        stroke_dash: rows.dashes_or_style()?,
        ..Default::default()
    };
    Ok(vec![Drawn {
        mark: mark.into(),
        rows: rows.table.clone(),
    }])
}

/// The text of each row as markup or plain text: strings as they are, and other values formatted
/// with the text mark's pattern through the engine's formatters, in the mark's syntax.
/// Where the text of a row comes from: the text that the chart gives, a formatted value as plain
/// text, or a formatted value as math markup, such as a number in scientific notation.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Text {
    Given,
    Formatted,
    Markup,
}

pub(crate) fn text_values(
    rows: &LayerRows,
    text: TextFormat,
) -> Result<(ScalarOrArray<String>, TextSyntaxMode)> {
    let engine = rows.engine;
    let format = text.format;
    let formatted = rows
        .resolve(
            Channel::Text,
            &|values, _scale: Option<&ConfiguredScale>| {
                let strings = matches!(
                    values.data_type(),
                    DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
                );
                if strings {
                    let text = cast(values, &DataType::Utf8)?;
                    return Ok(ScalarOrArray::new_array(
                        text.as_string::<i32>()
                            .iter()
                            .map(|value| (value.unwrap_or_default().to_string(), Text::Given))
                            .collect(),
                    ));
                }
                let values = engine.format_array(values, format, PrecisionSharing::PerValue)?;
                Ok(ScalarOrArray::new_array(
                    values
                        .into_iter()
                        .map(|value| match value {
                            LabelValue::Markup(markup) => (markup, Text::Markup),
                            LabelValue::Str(text) => (text, Text::Formatted),
                            _ => (String::new(), Text::Formatted),
                        })
                        .collect(),
                ))
            },
        )?
        .unwrap_or_else(|| scalar((String::new(), Text::Given)));
    let syntax = proto::TextSyntax::try_from(text.syntax).unwrap_or_default();
    // Under Typst syntax, the given text is markup, and a formatted value is plain text.
    if syntax == proto::TextSyntax::Typst {
        return Ok((
            formatted.map(|(text, kind)| match kind {
                Text::Formatted => escape_text(text),
                Text::Given | Text::Markup => text.clone(),
            }),
            TextSyntaxMode::TypstMarkup,
        ));
    }
    let markup = match formatted.value() {
        ScalarOrArrayValue::Scalar((_, kind)) => *kind == Text::Markup,
        ScalarOrArrayValue::Array(values) => values.iter().any(|(_, kind)| *kind == Text::Markup),
    };
    if markup {
        return Ok((
            formatted.map(|(text, kind)| match kind {
                Text::Markup => text.clone(),
                Text::Given | Text::Formatted => escape_text(text),
            }),
            TextSyntaxMode::TypstMarkup,
        ));
    }
    let syntax = match syntax {
        proto::TextSyntax::PlainLines => TextSyntaxMode::PlainLines,
        _ => TextSyntaxMode::Plain,
    };
    Ok((formatted.map(|(text, _)| text.clone()), syntax))
}

/// The computed `align` and `baseline` of a text mark, with `auto` at the center.
pub(crate) fn text_anchor(style: &ComputedStyle) -> (TextAlign, TextBaseline) {
    let align = match style.keyword(Property::Align) {
        Some("left") => TextAlign::Left,
        Some("right") => TextAlign::Right,
        _ => TextAlign::Center,
    };
    let baseline = match style.keyword(Property::Baseline) {
        Some("top" | "line-top") => TextBaseline::Top,
        Some("alphabetic") => TextBaseline::Alphabetic,
        Some("bottom" | "line-bottom") => TextBaseline::Bottom,
        _ => TextBaseline::Middle,
    };
    (align, baseline)
}

/// The first family of the computed `font-family` of a node.
pub(crate) fn font_family(style: &ComputedStyle) -> String {
    let family = match style.get(Property::FontFamily) {
        Computed::List(families) => families.first(),
        family => Some(family),
    };
    match family {
        Some(Computed::String(family)) => family.to_string(),
        Some(Computed::Keyword(family)) => family.to_string(),
        _ => unreachable!("`font-family` computes to a list of families"),
    }
}

/// The computed `font-weight` of a node.
pub(crate) fn font_weight(style: &ComputedStyle) -> FontWeight {
    match style.get(Property::FontWeight) {
        Computed::Keyword("bold") => FontWeight::BOLD,
        Computed::Number(weight) => FontWeight::from(*weight),
        _ => FontWeight::NORMAL,
    }
}

/// The computed `font-style` of a node.
pub(crate) fn font_style(style: &ComputedStyle) -> FontStyle {
    match style.keyword(Property::FontStyle) {
        Some("italic") => FontStyle::Italic,
        _ => FontStyle::Normal,
    }
}

/// The scene text mark of a layer's rows at `x` and `y`.
pub(crate) fn text_mark(
    rows: &LayerRows,
    text: TextFormat,
    x: ScalarOrArray<f32>,
    y: ScalarOrArray<f32>,
) -> Result<SceneTextMark> {
    let style = rows.style;
    let (values, syntax) = text_values(rows, text)?;
    let (align, baseline) = text_anchor(style);
    let (fill, _) = rows.paint()?;
    let fallback = style_color(style, Property::Color);
    let color = fill.map(|color| match color {
        ColorOrGradient::Color(color) => *color,
        ColorOrGradient::GradientIndex(_) => fallback,
    });
    // The width passes through as it is, as the font size does: one point of the text engine is
    // one pixel of the scene.
    let fixed = style.keyword(Property::WidthMode) == Some("fixed");
    let width = match rows.numbers(Channel::Width)? {
        Some(widths) => widths.map(|width| match fixed {
            true => LabelWidth::Fixed(*width),
            false => LabelWidth::Max(*width),
        }),
        None => scalar(LabelWidth::Auto),
    };
    let wraps = style.keyword(Property::WhiteSpace) != Some("nowrap");
    Ok(SceneTextMark {
        name: rows.name.clone(),
        len: rows.len() as u32,
        text: values,
        text_syntax: syntax,
        x,
        y,
        align: scalar(align),
        baseline: scalar(baseline),
        angle: rows.numbers_or(Channel::Angle, Property::Angle)?,
        color,
        font: scalar(font_family(style)),
        font_size: match rows.numbers(Channel::Size)? {
            Some(sizes) => sizes,
            None => scalar(style.font_size()),
        },
        font_weight: rows
            .resolve(Channel::FontWeight, &font_weights)?
            .unwrap_or_else(|| scalar(font_weight(style))),
        font_style: rows
            .resolve(Channel::FontStyle, &font_styles)?
            .unwrap_or_else(|| scalar(font_style(style))),
        wrap: wraps && rows.has(Channel::Width),
        max_lines: style
            .integer(Property::MaxLines)
            .and_then(|lines| NonZeroUsize::new(lines as usize)),
        ellipsis: style.keyword(Property::TextOverflow) == Some("ellipsis"),
        width,
        ..Default::default()
    })
}

/// The font weight of each value: a number from 100 to 900, or `normal` or `bold`.
fn font_weights(
    values: &arrow::array::ArrayRef,
    _scale: Option<&ConfiguredScale>,
) -> Result<ScalarOrArray<FontWeight>> {
    if values.data_type().is_numeric() {
        let weights = Coercer::default().to_numeric(values, None)?;
        return Ok(weights.map(|weight| FontWeight::from(*weight)));
    }
    let names = cast(values, &DataType::Utf8)?;
    let weights = (names.as_string::<i32>().iter())
        .map(|name| match name {
            Some("bold") => FontWeight::BOLD,
            _ => FontWeight::NORMAL,
        })
        .collect();
    Ok(ScalarOrArray::new_array(weights))
}

/// The font style of each value: `italic`, or else `normal`.
fn font_styles(
    values: &arrow::array::ArrayRef,
    _scale: Option<&ConfiguredScale>,
) -> Result<ScalarOrArray<FontStyle>> {
    let names = cast(values, &DataType::Utf8)?;
    let styles = (names.as_string::<i32>().iter())
        .map(|name| match name {
            Some("italic") => FontStyle::Italic,
            Some("oblique") => FontStyle::Oblique,
            _ => FontStyle::Normal,
        })
        .collect();
    Ok(ScalarOrArray::new_array(styles))
}

fn texts(rows: &LayerRows, text: TextFormat) -> Result<Vec<Drawn>> {
    let mark = text_mark(
        rows,
        text,
        rows.position(Channel::X)?,
        rows.position(Channel::Y)?,
    )?;
    Ok(vec![Drawn {
        mark: mark.into(),
        rows: rows.table.clone(),
    }])
}

/// The computed `align` and `baseline` of an image mark, with `auto` at the center.
pub(crate) fn image_anchor(style: &ComputedStyle) -> (ImageAlign, ImageBaseline) {
    let align = match style.keyword(Property::Align) {
        Some("left") => ImageAlign::Left,
        Some("right") => ImageAlign::Right,
        _ => ImageAlign::Center,
    };
    let baseline = match style.keyword(Property::Baseline) {
        Some("top" | "line-top") => ImageBaseline::Top,
        Some("bottom" | "line-bottom") => ImageBaseline::Bottom,
        _ => ImageBaseline::Middle,
    };
    (align, baseline)
}

/// The width and the height of each image: the channels, or the computed `width` and `height` of
/// the mark node. `auto` is the image's own size in pixels, and one `auto` dimension follows the
/// image's aspect from the other. A URL image, whose size is unknown before it loads, has no size
/// for `auto`, so it is NaN and the image does not draw.
pub(crate) fn image_sizes(rows: &LayerRows) -> Result<[ScalarOrArray<f32>; 2]> {
    let given = |channel: Channel, property: Property| -> Result<Option<ScalarOrArray<f32>>> {
        Ok(match rows.numbers(channel)? {
            Some(values) => Some(values),
            None => rows.style.number(property).map(scalar),
        })
    };
    let (width, height) = (
        given(Channel::Width, Property::Width)?,
        given(Channel::Height, Property::Height)?,
    );
    if let (Some(width), Some(height)) = (&width, &height) {
        return Ok([width.clone(), height.clone()]);
    }
    let len = rows.len();
    let own = own_sizes(rows)?;
    let (width, height) = (
        width.map(|width| width.as_vec(len, None)),
        height.map(|height| height.as_vec(len, None)),
    );
    let (widths, heights): (Vec<f32>, Vec<f32>) = (0..len)
        .map(|row| match own[row] {
            None => (f32::NAN, f32::NAN),
            Some((own_width, own_height)) => match (&width, &height) {
                (Some(width), _) => (width[row], width[row] * own_height / own_width),
                (None, Some(height)) => (height[row] * own_width / own_height, height[row]),
                (None, None) => (own_width, own_height),
            },
        })
        .unzip();
    Ok([
        ScalarOrArray::new_array(widths),
        ScalarOrArray::new_array(heights),
    ])
}

/// The size in pixels of the image of each row: of an inline image, or `None` for a URL image,
/// whose size is unknown before it loads.
fn own_sizes(rows: &LayerRows) -> Result<Vec<Option<(f32, f32)>>> {
    let convert = |values: &ArrayRef, _: Option<&ConfiguredScale>| {
        Ok(match values.data_type() {
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
                ScalarOrArray::new_array(vec![None; values.len()])
            }
            _ => Coercer::default()
                .to_image(values)?
                .map(|image| Some((image.width as f32, image.height as f32))),
        })
    };
    Ok(match rows.resolve(Channel::Image, &convert)? {
        Some(sizes) => sizes.as_vec(rows.len(), None),
        None => vec![None; rows.len()],
    })
}

/// Why an image does not draw: an `auto` size of a URL image, whose size is unknown before it
/// loads.
pub(crate) fn image_warning(rows: &LayerRows) -> Result<Option<String>> {
    if rows.mark.mark_type() != MarkType::Image || spans(rows) {
        return Ok(None);
    }
    let auto = |channel: Channel, property: Property| {
        !rows.has(channel) && rows.style.number(property).is_none()
    };
    if !auto(Channel::Width, Property::Width) && !auto(Channel::Height, Property::Height) {
        return Ok(None);
    }
    Ok(own_sizes(rows)?.iter().any(Option::is_none).then(|| {
        "an `auto` size of a URL image is unknown before the image loads, so the image does \
         not draw: give its width and height"
            .to_string()
    }))
}

/// The images of `values`: a URL of a resource of the size of its row, or the size of the largest
/// image of a constant, or an image inline.
fn image_sources(
    values: &ArrayRef,
    [width, height]: &[ScalarOrArray<f32>; 2],
    len: usize,
) -> Result<ScalarOrArray<SceneImageSource>> {
    let largest =
        |sizes: &ScalarOrArray<f32>| (0..len).map(|row| at(sizes, row)).fold(0.0f32, f32::max);
    let size = |sizes: &ScalarOrArray<f32>, index: usize| match values.len() == len {
        true => at(sizes, index),
        false => largest(sizes),
    };
    let images: Vec<SceneImageSource> = match values.data_type() {
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
            let urls = cast(values, &DataType::Utf8)?;
            (urls.as_string::<i32>().iter().enumerate())
                .map(|(index, url)| {
                    SceneImageSource::Resource(SceneImageResource {
                        url: url.unwrap_or_default().to_string(),
                        width: size(width, index).round() as u32,
                        height: size(height, index).round() as u32,
                        placeholder: None,
                    })
                })
                .collect()
        }
        _ => Coercer::default()
            .to_image(values)?
            .as_vec(values.len(), None)
            .into_iter()
            .map(|image| SceneImageSource::Inline(Arc::new(image)))
            .collect(),
    };
    Ok(ScalarOrArray::new_array(images))
}

/// Whether an image spans the rectangle of its `x`, `y`, `x2`, and `y2`, so that no width, height,
/// or anchor applies to it.
fn spans(rows: &LayerRows) -> bool {
    rows.has(Channel::X2) && rows.has(Channel::Y2)
}

fn images(rows: &LayerRows) -> Result<Vec<Drawn>> {
    let x = rows.position(Channel::X)?;
    let y = rows.position(Channel::Y)?;
    let mark = match spans(rows) {
        true => {
            let x2 = rows.position(Channel::X2)?;
            let y2 = rows.position(Channel::Y2)?;
            let len = rows.len();
            let (left, right): (Vec<f32>, Vec<f32>) = (0..len)
                .map(|row| (at(&x, row).min(at(&x2, row)), at(&x, row).max(at(&x2, row))))
                .unzip();
            let (top, bottom): (Vec<f32>, Vec<f32>) = (0..len)
                .map(|row| (at(&y, row).min(at(&y2, row)), at(&y, row).max(at(&y2, row))))
                .unzip();
            let widths = left.iter().zip(&right).map(|(l, r)| r - l).collect();
            let heights = top.iter().zip(&bottom).map(|(t, b)| b - t).collect();
            image_mark(
                rows,
                ScalarOrArray::new_array(left),
                ScalarOrArray::new_array(top),
                Some((
                    [
                        ScalarOrArray::new_array(widths),
                        ScalarOrArray::new_array(heights),
                    ],
                    (ImageAlign::Left, ImageBaseline::Top),
                )),
            )?
        }
        false => image_mark(rows, x, y, None)?,
    };
    Ok(mark
        .map(|mark| Drawn {
            mark: mark.into(),
            rows: rows.table.clone(),
        })
        .into_iter()
        .collect())
}

/// The image mark of a layer's rows at `x` and `y`, aligned as the mark node's computed `align`
/// and `baseline` say and of the sizes that its channels or its styles give, or in the boxes of
/// `boxes` with their anchor. `None` for a layer without images.
#[allow(clippy::type_complexity)]
pub(crate) fn image_mark(
    rows: &LayerRows,
    x: ScalarOrArray<f32>,
    y: ScalarOrArray<f32>,
    boxes: Option<([ScalarOrArray<f32>; 2], (ImageAlign, ImageBaseline))>,
) -> Result<Option<SceneImageMark>> {
    // The geometry comes first, so that a URL image asks for the size that it draws at.
    let ([width, height], (align, baseline)) = match boxes {
        Some(boxes) => boxes,
        None => (image_sizes(rows)?, image_anchor(rows.style)),
    };
    let len = rows.len();
    let sizes = [width.clone(), height.clone()];
    let convert =
        |values: &ArrayRef, _: Option<&ConfiguredScale>| image_sources(values, &sizes, len);
    let Some(images) = rows.resolve(Channel::Image, &convert)? else {
        return Ok(None);
    };
    Ok(Some(SceneImageMark {
        name: rows.name.clone(),
        len: rows.len() as u32,
        image: images,
        x,
        y,
        width,
        height,
        align: scalar(align),
        baseline: scalar(baseline),
        aspect: rows.style.keyword(Property::ObjectFit) != Some("fill"),
        smooth: rows.style.keyword(Property::ImageRendering) != Some("pixelated"),
        ..Default::default()
    }))
}

/// The paths of `values`, from SVG path text.
pub(crate) fn path_array(
    values: &ArrayRef,
    _: Option<&ConfiguredScale>,
) -> Result<ScalarOrArray<lyon_path::Path>> {
    Ok(Coercer::default().to_path(values)?)
}

/// The SVG paths of each row, with the first conditional value that selects it.
pub(crate) fn path_values(rows: &LayerRows) -> Result<ScalarOrArray<lyon_path::Path>> {
    Ok(rows
        .resolve(Channel::Path, &path_array)?
        .unwrap_or_else(|| scalar(lyon_path::Path::new())))
}

fn paths(rows: &LayerRows) -> Result<Vec<Drawn>> {
    let x = match rows.has(Channel::X) {
        true => rows.position(Channel::X)?,
        false => scalar(0.0),
    };
    let y = match rows.has(Channel::Y) {
        true => rows.position(Channel::Y)?,
        false => scalar(0.0),
    };
    Ok(vec![Drawn {
        mark: path_mark(rows, x, y)?.into(),
        rows: rows.table.clone(),
    }])
}

/// The path mark of a layer's rows, each path with its origin at `x` and `y` and turned by its
/// `angle` about it.
pub(crate) fn path_mark(
    rows: &LayerRows,
    x: ScalarOrArray<f32>,
    y: ScalarOrArray<f32>,
) -> Result<ScenePathMark> {
    let angle = rows.numbers_or(Channel::Angle, Property::Angle)?;
    let len = rows.len();
    let transform = ScalarOrArray::new_array(
        (0..len)
            .map(|row| {
                PathTransform::rotation(Angle::degrees(at(&angle, row)))
                    .then_translate(Vector2D::new(at(&x, row), at(&y, row)))
            })
            .collect(),
    );
    let (fill, stroke) = rows.paint()?;
    Ok(ScenePathMark {
        name: rows.name.clone(),
        len: len as u32,
        path: path_values(rows)?,
        fill,
        stroke,
        stroke_width: match rows.numbers(Channel::StrokeWidth)? {
            Some(width) => width,
            None => scalar(stroke_width(rows)?.unwrap_or(0.0)),
        },
        transform,
        fill_pattern: pattern_fills(row_patterns(rows)?),
        ..Default::default()
    })
}
