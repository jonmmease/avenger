//! The guides of a chart of one plot: an axis of each position scale that has one, the legends
//! and colorbars of its other scales, the plot's title, and its attribution, each configured from
//! the computed styles of its nodes and the content fields of the format.

use std::{num::NonZeroUsize, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray, Float64Array},
    compute::cast,
    datatypes::DataType,
};
use avenger_chart::{
    layer::MarkType,
    proto::{self, calendar_ticks, encoding, tick_interval, ticks, Channel},
    resolve::{scale_channel, Resolved, ScaleGroup},
    Chart,
};
use avenger_color::ColorOrGradient;
use avenger_common::{
    pattern::PatternFills,
    types::{SymbolShape, TextAlign, TextBaseline, TextSyntaxMode},
    value::ScalarOrArray,
};
use avenger_guides::{
    attribution::{Attribution, AttributionAlign, AttributionConfig},
    axis::{Axis, AxisConfig, AxisStyle, AxisTickInterval, AxisTicks, NestedBandAxisLevelConfig},
    legend::{
        Colorbar, ColorbarConfig, Legend, LegendConfig, LegendLayout, LegendStyle, LegendSwatches,
        LegendWrap, LineSwatches, SymbolSwatches,
    },
    title::{Title, TitleAlign, TitleConfig, TitleTextStyle},
    BackgroundStyle, GuideSide,
};
use avenger_scales::scales::{
    coerce::Coercer, domain_summary::SummaryKind, fitting::FittingScale, ConfiguredScale,
};
use avenger_style::{Computed, ComputedStyle, ComputedStyles, Paint, Property};
use avenger_typst_label::{bind, escape_text, LabelEngine, LabelValue, LabelValues};

use crate::{
    marks::{font_family, font_style, font_weight},
    style::tree::{GuideNodes, StyleNodes},
    style::{color as style_color, number as style_number},
    EvalError,
};

type Result<T> = std::result::Result<T, EvalError>;

/// The axis of a position scale group, on its side of the plot.
pub(crate) struct AxisGuide {
    /// The position of the group in `Resolved::scales`.
    pub(crate) group: usize,
    pub(crate) side: GuideSide,
    pub(crate) axis: Axis,
    /// Whether the axis draws grid lines across the plot.
    pub(crate) grid: bool,
    /// The computed `offset` of its `axis` node: its distance from the plot.
    pub(crate) offset: f32,
}

/// A legend or a colorbar.
pub(crate) enum LegendGuide {
    Legend(Box<Legend>),
    Colorbar(Box<Colorbar>),
}

/// A legend or a colorbar on its side of the plot, with the longest length of a colorbar.
pub(crate) struct LegendSlot {
    pub(crate) guide: LegendGuide,
    pub(crate) side: GuideSide,
    pub(crate) max_length: Option<f32>,
    /// The computed `offset` of its `legend` node: its distance from the axes of the plot, and from
    /// the legend before it on its side.
    pub(crate) offset: f32,
}

/// The plot's title on its side, with its distance from what stands inside it.
pub(crate) struct TitleGuide {
    pub(crate) title: Title,
    pub(crate) side: GuideSide,
    pub(crate) offset: f32,
}

/// The plot's attribution on its side, with its distance from what stands inside it.
pub(crate) struct AttributionGuide {
    pub(crate) attribution: Attribution,
    pub(crate) side: GuideSide,
    pub(crate) offset: f32,
}

/// The guides of a chart of one plot.
pub(crate) struct Guides {
    pub(crate) axes: Vec<AxisGuide>,
    pub(crate) legends: Vec<LegendSlot>,
    pub(crate) title: Option<TitleGuide>,
    pub(crate) attribution: Option<AttributionGuide>,
}

/// What the guides read: the chart, its models, its style nodes and their computed styles, the
/// fitting scale of each position group, and the scale of each other group.
pub(crate) struct GuideInputs<'a> {
    pub(crate) chart: &'a Chart,
    pub(crate) resolved: &'a Resolved<'a>,
    pub(crate) nodes: &'a StyleNodes,
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) engine: &'a LabelEngine,
    /// The fitting scale of each position group, by its position in `Resolved::scales`.
    pub(crate) positions: &'a [Option<FittingScale>],
    /// The scale of each non-position group, by its position in `Resolved::scales`.
    pub(crate) scales: &'a [Option<ConfiguredScale>],
}

// ---------------------------------------------------------------------------------------------
// Computed styles

/// The computed color of `property` of a node as a paint: a color, or transparent for none.
fn paint_color(style: &ComputedStyle, property: Property) -> [f32; 4] {
    match style.paint(property) {
        Paint::Color(color) => color,
        Paint::Contrast { otherwise } => otherwise,
        Paint::None => [0.0, 0.0, 0.0, 0.0],
    }
}

/// The computed text color of a node: its `fill`, which `auto` makes its `color`.
fn text_color(style: &ComputedStyle) -> [f32; 4] {
    match style.paint(Property::Fill) {
        Paint::None => style_color(style, Property::Color),
        _ => paint_color(style, Property::Fill),
    }
}

fn side(style: &ComputedStyle) -> GuideSide {
    match style.keyword(Property::Orient) {
        Some("top" | "top-left" | "top-right") => GuideSide::Top,
        Some("left") => GuideSide::Left,
        Some("right") => GuideSide::Right,
        _ => GuideSide::Bottom,
    }
}

/// The opposite side, along which a second axis of a direction stands.
fn opposite(side: GuideSide) -> GuideSide {
    match side {
        GuideSide::Top => GuideSide::Bottom,
        GuideSide::Bottom => GuideSide::Top,
        GuideSide::Left => GuideSide::Right,
        GuideSide::Right => GuideSide::Left,
    }
}

/// The computed background of a box: its `background`, its border when it shows, its radius, and
/// its padding.
fn background(style: &ComputedStyle) -> BackgroundStyle {
    let border = style
        .keyword(Property::BorderStyle)
        .is_some_and(|line| line != "none")
        && style_number(style, Property::BorderWidth) > 0.0;
    BackgroundStyle {
        fill: ColorOrGradient::Color(style_color(style, Property::Background)),
        stroke: border
            .then(|| style.color(Property::BorderColor))
            .flatten()
            .map(ColorOrGradient::Color),
        corner_radius: style_number(style, Property::BorderRadius),
        padding: style_number(style, Property::PaddingTop),
    }
}

/// The computed `offset` of a guide: its distance from what stands inside it.
fn offset(style: &ComputedStyle) -> f32 {
    style_number(style, Property::Offset)
}

fn visible(style: &ComputedStyle) -> bool {
    !style.is_hidden()
}

fn text_syntax(syntax: i32) -> TextSyntaxMode {
    match proto::TextSyntax::try_from(syntax).unwrap_or_default() {
        proto::TextSyntax::Typst => TextSyntaxMode::TypstMarkup,
        proto::TextSyntax::PlainLines => TextSyntaxMode::PlainLines,
        _ => TextSyntaxMode::Plain,
    }
}

// ---------------------------------------------------------------------------------------------
// Titles of scales

/// The default title of a scale group's guide: the distinct titles of the encodings of its
/// members, in tree order, joined by `, `, where a column's title is its name.
fn default_title(resolved: &Resolved, group: &ScaleGroup, columns: bool) -> String {
    let mut titles: Vec<String> = Vec::new();
    for member in &group.members {
        if scale_channel(member.channel) != member.channel {
            continue;
        }
        let Some(encoding) = resolved.tree.layers[member.layer]
            .mark
            .encoding(member.channel)
        else {
            continue;
        };
        let title = match (&encoding.title, &encoding.kind) {
            (Some(title), _) => title.clone(),
            (None, Some(encoding::Kind::DirectField(name) | encoding::Kind::ScaledField(name)))
                if columns =>
            {
                name.clone()
            }
            _ => String::new(),
        };
        if !title.is_empty() && !titles.contains(&title) {
            titles.push(title);
        }
    }
    titles.join(", ")
}

/// The title of a guide: its own text, or the default title of its scale.
fn guide_title(title: Option<&proto::Title>, default: impl FnOnce() -> String) -> String {
    match title.map(|title| title.text.clone()) {
        Some(text) => text,
        None => default(),
    }
}

// ---------------------------------------------------------------------------------------------
// Axes

fn axis_spec(group: &ScaleGroup) -> proto::Axis {
    group.spec.axis.clone().unwrap_or_default()
}

/// The ticks of a `Ticks` in place of the scale's: a list of values or an interval.
fn ticks(chart: &Chart, ticks: Option<&proto::Ticks>) -> Option<AxisTicks> {
    match ticks?.at.as_ref()? {
        ticks::At::Values(values) => Some(AxisTicks::Values(chart.list_elements(values))),
        ticks::At::Interval(interval) => match interval.interval.as_ref()? {
            tick_interval::Interval::Numeric(numeric) => {
                Some(AxisTicks::StartStep(AxisTickInterval::Numeric {
                    start: numeric.start,
                    step: numeric.step,
                }))
            }
            tick_interval::Interval::Calendar(calendar) => {
                let (months, days, nanos) = match calendar.step {
                    Some(calendar_ticks::Step::Months(months)) => (months, 0, 0),
                    Some(calendar_ticks::Step::Days(days)) => (0, days, 0),
                    Some(calendar_ticks::Step::Milliseconds(millis)) => (0, 0, millis * 1_000_000),
                    None => (0, 0, 0),
                };
                Some(AxisTicks::StartStep(AxisTickInterval::Temporal {
                    start_millis: calendar.start,
                    months,
                    days,
                    nanos,
                }))
            }
        },
        ticks::At::Count(_) => None,
    }
}

/// The style of an axis's parts from their computed styles.
#[allow(clippy::too_many_arguments)]
fn axis_style(
    chart: &Chart,
    axis: &ComputedStyle,
    title: &ComputedStyle,
    label: &ComputedStyle,
    tick: &ComputedStyle,
    rule: &ComputedStyle,
    grid: Option<&ComputedStyle>,
    spec_ticks: Option<&proto::Ticks>,
    title_syntax: TextSyntaxMode,
) -> AxisStyle {
    let label_align = match label.keyword(Property::TextAlign) {
        Some("start") => Some(TextAlign::Left),
        Some("center") => Some(TextAlign::Center),
        Some("end") => Some(TextAlign::Right),
        _ => None,
    };
    let label_baseline = match label.keyword(Property::VerticalAlign) {
        Some("top") => Some(TextBaseline::Top),
        Some("middle") => Some(TextBaseline::Middle),
        Some("alphabetic") => Some(TextBaseline::Alphabetic),
        Some("bottom") => Some(TextBaseline::Bottom),
        _ => None,
    };
    let ticks_shown = visible(tick);
    let mut style = AxisStyle {
        tick_count: axis.number(Property::TickCount).map(f64::from),
        ticks: ticks(chart, spec_ticks),
        tick_length: match ticks_shown {
            true => style_number(tick, Property::TickSize),
            false => 0.0,
        },
        tick_color: paint_color(tick, Property::Stroke),
        labels_visible: visible(label),
        label_angle: style_number(axis, Property::LabelAngle),
        label_align,
        label_baseline,
        label_color: text_color(label),
        label_font_family: font_family(label),
        label_font_size: label.font_size(),
        label_font_weight: font_weight(label),
        title_visible: visible(title),
        title_syntax,
        title_color: text_color(title),
        title_font_family: font_family(title),
        title_font_size: title.font_size(),
        title_font_weight: font_weight(title),
        domain_color: match visible(rule) {
            true => paint_color(rule, Property::Stroke),
            false => [0.0; 4],
        },
        ..AxisStyle::default()
    };
    if let Some(grid) = grid {
        style.grid_color = paint_color(grid, Property::Stroke);
        style.grid_width = style_number(grid, Property::StrokeWidth);
    }
    // `parity` and `greedy` both keep an even pattern of labels apart.
    style.label_thinning = match axis.keyword(Property::LabelOverlap) {
        Some("none") => avenger_guides::axis::LabelThinning::None,
        _ => avenger_guides::axis::LabelThinning::Stride,
    };
    style
}

/// The axes of the position groups that have one, with the side of each from the computed
/// `orient` of its `axis` node. A second axis of a direction whose side meets the first's stands
/// on the other side, and draws no grid lines.
fn axes(inputs: &GuideInputs) -> Result<Vec<AxisGuide>> {
    let mut axes: Vec<AxisGuide> = Vec::new();
    for (index, group) in inputs.resolved.scales.iter().enumerate() {
        let Some(Some(GuideNodes::Axis(nodes))) = inputs.nodes.guides.get(index) else {
            continue;
        };
        let Some(Some(fitting)) = inputs.positions.get(index) else {
            continue;
        };
        let styles = inputs.styles;
        // `display: none` on the axis hides its parts and its grid lines.
        if styles[nodes.axis].is_hidden() {
            continue;
        }
        let spec = axis_spec(group);
        let mut side = side(&styles[nodes.axis]);
        let first = axes
            .iter()
            .find(|axis| inputs.resolved.scales[axis.group].channel == group.channel);
        let second = first.is_some();
        if first.is_some_and(|first| first.side == side) {
            side = opposite(side);
        }
        let grid = visible(&styles[nodes.grid]) && !second;
        let title_syntax = text_syntax(spec.title.as_ref().map_or(0, |title| title.syntax));
        let style = axis_style(
            inputs.chart,
            &styles[nodes.axis],
            &styles[nodes.title],
            &styles[nodes.label],
            &styles[nodes.tick],
            &styles[nodes.rule],
            Some(&styles[nodes.grid]),
            spec.ticks.as_ref(),
            title_syntax,
        );
        let labels = spec.labels.as_ref();
        let config = AxisConfig {
            side,
            tick_format: labels.map_or_else(String::new, |labels| labels.format.clone()),
            tick_label: labels
                .map(|labels| labels.markup.clone())
                .filter(|markup| !markup.is_empty()),
            style,
        };
        let nested = matches!(group.spec.r#type, Some(proto::scale::Type::NestedBand(_)));
        let title = guide_title(spec.title.as_ref(), || {
            default_title(inputs.resolved, group, !nested)
        });
        let engine = inputs.engine;
        // The axis follows the scale's implementation, so a custom scale of categories draws
        // the axis of the categories of its kind.
        let axis = match fitting.scale().scale_impl.scale_type() {
            "band" => Axis::band(fitting.clone(), &title, &config, engine)?,
            "point" => Axis::point(fitting.clone(), &title, &config, engine)?,
            "nested_band" => {
                let levels: Vec<NestedBandAxisLevelConfig> = (nodes.levels.iter())
                    .enumerate()
                    .map(|(position, (level, label))| {
                        let message = spec.levels.get(position);
                        let title = message.and_then(|level| level.title.clone());
                        NestedBandAxisLevelConfig {
                            visible: visible(&styles[*label]),
                            // An empty title leaves the level out of the axis's title.
                            title_visible: title.as_deref() != Some(""),
                            title: title.filter(|title| !title.is_empty()),
                            label_angle: styles[*level].number(Property::LabelAngle),
                            tick_format: message
                                .map(|level| level.format.clone())
                                .filter(|format| !format.is_empty()),
                        }
                    })
                    .collect();
                Axis::nested_band(fitting.clone(), &title, &levels, &config, engine)?
            }
            _ => Axis::continuous(fitting.clone(), &title, &config, engine)?,
        };
        axes.push(AxisGuide {
            group: index,
            side,
            axis,
            grid,
            offset: offset(&styles[nodes.axis]),
        });
    }
    Ok(axes)
}

// ---------------------------------------------------------------------------------------------
// Legends

/// The swatch values of one channel of a legend's entries.
enum Swatch {
    Color(Vec<ColorOrGradient>),
    Fill(Vec<ColorOrGradient>),
    Stroke(Vec<ColorOrGradient>),
    Opacity(Vec<f32>),
    Size(Vec<f32>),
    StrokeWidth(Vec<f32>),
    Angle(Vec<f32>),
    Shape(Vec<String>),
    StrokeDash(Vec<Vec<f32>>),
    Pattern(PatternFills),
}

impl Swatch {
    /// Repeats the swatch of the last entry for one more entry, which the legend hides.
    fn repeat_last(&mut self) {
        fn repeat<T: Clone>(values: &mut Vec<T>) {
            if let Some(last) = values.last().cloned() {
                values.push(last);
            }
        }
        match self {
            Self::Color(values) | Self::Fill(values) | Self::Stroke(values) => repeat(values),
            Self::Opacity(values)
            | Self::Size(values)
            | Self::StrokeWidth(values)
            | Self::Angle(values) => repeat(values),
            Self::Shape(values) => repeat(values),
            Self::StrokeDash(values) => repeat(values),
            Self::Pattern(fills) => {
                let len = match fills.0.value() {
                    avenger_common::value::ScalarOrArrayValue::Array(values) => values.len(),
                    avenger_common::value::ScalarOrArrayValue::Scalar(_) => 1,
                };
                let mut values = fills.0.as_vec(len, None);
                values.push(None);
                *fills = PatternFills(ScalarOrArray::new_array(values));
            }
        }
    }
}

/// The values of `values` through `scale` as the swatch of `channel`, or `None` for a channel
/// that a swatch does not draw.
fn swatch(channel: Channel, scale: &ConfiguredScale, values: &ArrayRef) -> Result<Option<Swatch>> {
    let len = values.len();
    let numbers = || -> Result<Vec<f32>> { Ok(scale.scale_to_numeric(values)?.as_vec(len, None)) };
    let colors =
        || -> Result<Vec<ColorOrGradient>> { Ok(scale.scale_to_color(values)?.as_vec(len, None)) };
    Ok(Some(match channel {
        Channel::Color => Swatch::Color(colors()?),
        Channel::Fill => Swatch::Fill(colors()?),
        Channel::Stroke => Swatch::Stroke(colors()?),
        Channel::Opacity | Channel::FillOpacity | Channel::StrokeOpacity => {
            Swatch::Opacity(numbers()?)
        }
        Channel::Size => Swatch::Size(numbers()?),
        Channel::StrokeWidth => Swatch::StrokeWidth(numbers()?),
        Channel::Angle => Swatch::Angle(numbers()?),
        Channel::Shape => Swatch::Shape(scale.scale_to_string(values)?.as_vec(len, None)),
        Channel::StrokeDash => Swatch::StrokeDash(
            Coercer::default()
                .to_stroke_dash(&scale.scale(values)?)?
                .as_vec(len, None),
        ),
        Channel::Pattern => Swatch::Pattern(PatternFills(ScalarOrArray::new_array(
            scale.scale_to_pattern(values)?.as_vec(len, None),
        ))),
        _ => return Ok(None),
    }))
}

/// The label text of `values` through the labels' markup, where `#value` is the formatted value,
/// or the formatted values, as plain text unless one of them is markup.
fn label_text(values: Vec<LabelValue>, markup: &str) -> Result<(Vec<String>, TextSyntaxMode)> {
    if !markup.is_empty() {
        let text = (values.into_iter())
            .map(|value| bind(markup, &LabelValues::from([("value".into(), value)])))
            .collect::<std::result::Result<_, _>>()?;
        return Ok((text, TextSyntaxMode::TypstMarkup));
    }
    let any_markup = values
        .iter()
        .any(|value| matches!(value, LabelValue::Markup(_)));
    let text = (values.into_iter())
        .map(|value| match value {
            LabelValue::Markup(markup) => markup,
            LabelValue::Str(text) if any_markup => escape_text(&text),
            LabelValue::Str(text) => text,
            _ => String::new(),
        })
        .collect();
    let syntax = match any_markup {
        true => TextSyntaxMode::TypstMarkup,
        false => TextSyntaxMode::Plain,
    };
    Ok((text, syntax))
}

fn numbers(values: &ArrayRef) -> Result<Vec<f64>> {
    let values = cast(values, &DataType::Float64)?;
    Ok((values.as_any())
        .downcast_ref::<Float64Array>()
        .expect("a Float64 array")
        .iter()
        .flatten()
        .collect())
}

/// The entries of the legend of a scale: its categories, the bins of a quantize, a quantile, or a
/// threshold scale, or the ticks of another continuous scale at `tick_count`, or the scale's default
/// count without one, with each entry's input to the scale and its label.
fn entries(
    scale: &ConfiguredScale,
    spec: &proto::Legend,
    tick_count: Option<f64>,
    chart: &Chart,
    engine: &LabelEngine,
) -> Result<(ArrayRef, Vec<String>, TextSyntaxMode)> {
    let labels = spec.labels.as_ref();
    let pattern = labels.map_or("", |labels| labels.format.as_str());
    let markup = labels.map_or("", |labels| labels.markup.as_str());
    let formatted = |values: &ArrayRef| -> Result<Vec<LabelValue>> {
        Ok(engine.format_array(values, pattern, scale.tick_precision_sharing())?)
    };
    let scale_type = scale.scale_impl.scale_type();
    match scale_type {
        "quantize" | "quantile" | "threshold" => {
            // The edges between bins, and for a quantize scale, the ends of its domain.
            let edges: Vec<f64> = match scale_type {
                "quantize" => {
                    let (low, high) = scale.numeric_interval_domain()?;
                    let bins = scale.range().len().max(1);
                    (0..=bins)
                        .map(|bin| low + (high - low) * bin as f64 / bins as f64)
                        .collect()
                }
                "quantile" => numbers(&scale.ticks(None)?)?,
                _ => numbers(scale.domain())?,
            };
            let edge_values: ArrayRef = Arc::new(Float64Array::from(edges.clone()));
            let (text, syntax) = label_text(formatted(&edge_values)?, markup)?;
            let between = |pair: &[String]| format!("{} – {}", pair[0], pair[1]);
            // Each bin's entry is the scale at a value inside it: its middle for a quantize
            // scale, and its start, or below the first edge, for the others.
            let (inside, labels): (Vec<f64>, Vec<String>) = if scale_type == "quantize" {
                (
                    edges
                        .windows(2)
                        .map(|pair| (pair[0] + pair[1]) / 2.0)
                        .collect(),
                    text.windows(2).map(between).collect(),
                )
            } else if let (Some(first), Some(last)) = (text.first(), text.last()) {
                let mut inside = vec![edges[0] - edges[0].abs().max(1.0)];
                inside.extend(&edges);
                let mut labels = vec![format!("< {first}")];
                labels.extend(text.windows(2).map(between));
                labels.push(format!("≥ {last}"));
                (inside, labels)
            } else {
                (vec![0.0], vec![String::new()])
            };
            let values: ArrayRef = Arc::new(Float64Array::from(inside));
            let values = cast(&values, scale.domain().data_type()).unwrap_or(values);
            Ok((values, labels, syntax))
        }
        _ if matches!(scale_type, "band" | "point" | "nested_band" | "ordinal") => {
            let values = scale.domain().clone();
            let (labels, syntax) = label_text(formatted(&values)?, markup)?;
            Ok((values, labels, syntax))
        }
        _ => {
            let values = match &spec.entries {
                Some(proto::legend::Entries::Values(values)) => chart.list_elements(values),
                _ => scale.ticks(tick_count)?,
            };
            let (labels, syntax) = label_text(formatted(&values)?, markup)?;
            Ok((values, labels, syntax))
        }
    }
}

/// The value of the constant encoding of `channel` of a mark, when it has no conditional values:
/// the value that every row of the layer draws.
fn constant_value<'c>(
    chart: &'c Chart,
    mark: &dyn avenger_chart::layer::MarkMessage,
    channel: Channel,
) -> Option<&'c ArrayRef> {
    let encoding = mark.encoding(channel)?;
    match &encoding.kind {
        Some(encoding::Kind::DirectValue(value)) if encoding.conditional.is_empty() => {
            Some(chart.value(value))
        }
        _ => None,
    }
}

fn constant_color(
    chart: &Chart,
    mark: &dyn avenger_chart::layer::MarkMessage,
    channel: Channel,
) -> Result<Option<ColorOrGradient>> {
    constant_value(chart, mark, channel)
        .map(|value| {
            Ok(crate::encode::at(
                &Coercer::default().to_color(value, None)?,
                0,
            ))
        })
        .transpose()
}

fn constant_number(
    chart: &Chart,
    mark: &dyn avenger_chart::layer::MarkMessage,
    channel: Channel,
) -> Result<Option<f32>> {
    constant_value(chart, mark, channel)
        .map(|value| {
            Ok(crate::encode::at(
                &Coercer::default().to_numeric(value, None)?,
                0,
            ))
        })
        .transpose()
}

fn constant_string(
    chart: &Chart,
    mark: &dyn avenger_chart::layer::MarkMessage,
    channel: Channel,
) -> Result<Option<String>> {
    constant_value(chart, mark, channel)
        .map(|value| {
            let text = cast(value, &DataType::Utf8)?;
            Ok(text.as_string::<i32>().value(0).to_string())
        })
        .transpose()
}

/// The distinct columns that the encodings of `group` scale, in order.
fn scaled_columns(resolved: &Resolved, group: &ScaleGroup) -> Vec<String> {
    let mut columns: Vec<String> = (group.members.iter())
        .filter_map(|member| {
            let encoding = resolved.tree.layers[member.layer]
                .mark
                .encoding(member.channel)?;
            match &encoding.kind {
                Some(encoding::Kind::ScaledField(name)) => Some(name.clone()),
                _ => None,
            }
        })
        .collect();
    columns.sort();
    columns.dedup();
    columns
}

/// A symbol legend before its swatches are made.
struct LegendDraft {
    /// The scale group of its first scale.
    group: usize,
    /// Whether the last entry counts the entries past `max-entries`, which draws no swatch.
    rest: bool,
    spec: proto::Legend,
    /// The categories of a discrete scale, which merge legends with equal categories.
    categories: Option<ArrayRef>,
    /// The columns that the scale's encodings scale, which merge legends of one column.
    columns: Vec<String>,
    title: String,
    labels: Vec<String>,
    label_syntax: TextSyntaxMode,
    swatches: Vec<Swatch>,
}

/// A legend or a colorbar before the legends merge.
enum Pending {
    Colorbar(LegendSlot),
    Legend(Box<LegendDraft>),
}

/// The legends and the colorbars of the plot, in the order of their scales. Symbol legends whose
/// scales encode the same columns and hold the same categories in the same order, and whose specs
/// are equal, merge, and their titles join, so the `color` and the `shape` of one column show one
/// legend, and two columns whose categories happen to be equal show two.
fn legends(inputs: &GuideInputs) -> Result<Vec<LegendSlot>> {
    let mut pending: Vec<Pending> = Vec::new();
    for (index, group) in inputs.resolved.scales.iter().enumerate() {
        let Some(Some(scale)) = inputs.scales.get(index) else {
            continue;
        };
        let hidden = match inputs.nodes.guides.get(index) {
            Some(Some(GuideNodes::Colorbar(nodes))) => inputs.styles[nodes.legend].is_hidden(),
            Some(Some(GuideNodes::Legend(nodes))) => inputs.styles[nodes.legend].is_hidden(),
            _ => false,
        };
        if hidden {
            continue;
        }
        match inputs.nodes.guides.get(index) {
            Some(Some(GuideNodes::Colorbar(_))) => {
                pending.push(Pending::Colorbar(colorbar(inputs, index, group, scale)?));
            }
            Some(Some(GuideNodes::Legend(nodes))) => {
                let spec = group.spec.legend.clone().unwrap_or_default();
                let tick_count = inputs.styles[nodes.legend]
                    .number(Property::TickCount)
                    .map(f64::from);
                let (values, mut labels, label_syntax) =
                    entries(scale, &spec, tick_count, inputs.chart, inputs.engine)?;
                // A legend longer than `max-entries` ends with an entry that counts the rest.
                let max_entries = inputs.styles[nodes.legend].integer(Property::MaxEntries);
                let rest = match max_entries {
                    Some(max) if labels.len() > max as usize => labels.len() - (max as usize - 1),
                    _ => 0,
                };
                let values = match rest {
                    0 => values,
                    rest => {
                        labels.truncate(labels.len() - rest);
                        labels.push(format!("…{rest} more"));
                        values.slice(0, values.len() - rest)
                    }
                };
                let Some(mut swatch) = swatch(group.channel, scale, &values)? else {
                    continue;
                };
                if rest > 0 {
                    swatch.repeat_last();
                }
                let title = guide_title(spec.title.as_ref(), || {
                    default_title(inputs.resolved, group, true)
                });
                let categories = (scale.scale_impl.summary_kind() == Some(SummaryKind::Categories))
                    .then(|| scale.domain().clone());
                let columns = scaled_columns(inputs.resolved, group);
                let merged = pending.iter_mut().find_map(|pending| match pending {
                    // Two scales of one kind of swatch, such as two independent color scales,
                    // keep their legends apart, so each shows its own swatches.
                    Pending::Legend(draft)
                        if draft.spec == spec
                            && !columns.is_empty()
                            && draft.columns == columns
                            && matches!((&draft.categories, &categories),
                                (Some(a), Some(b)) if a.as_ref() == b.as_ref())
                            && !(draft.swatches.iter()).any(|known| {
                                std::mem::discriminant(known) == std::mem::discriminant(&swatch)
                            }) =>
                    {
                        Some(draft)
                    }
                    _ => None,
                });
                match merged {
                    Some(draft) => {
                        if !title.is_empty() && !draft.title.split(", ").any(|known| known == title)
                        {
                            draft.title = match draft.title.is_empty() {
                                true => title,
                                false => format!("{}, {title}", draft.title),
                            };
                        }
                        draft.swatches.push(swatch);
                    }
                    None => pending.push(Pending::Legend(Box::new(LegendDraft {
                        group: index,
                        rest: rest > 0,
                        spec,
                        categories,
                        columns,
                        title,
                        labels,
                        label_syntax,
                        swatches: vec![swatch],
                    }))),
                }
            }
            _ => {}
        }
    }
    pending
        .into_iter()
        .map(|pending| match pending {
            Pending::Colorbar(slot) => Ok(slot),
            Pending::Legend(draft) => legend(inputs, *draft),
        })
        .collect()
}

/// A symbol legend from its draft: its swatches draw the mark of its first scale's first layer,
/// with that layer's computed style for the channels that the legend does not show.
fn legend(inputs: &GuideInputs, draft: LegendDraft) -> Result<LegendSlot> {
    let group = &inputs.resolved.scales[draft.group];
    let Some(GuideNodes::Legend(nodes)) = &inputs.nodes.guides[draft.group] else {
        unreachable!("a symbol legend has the nodes of one");
    };
    let styles = inputs.styles;
    let layer_index = group.members[0].layer;
    let layer = &inputs.resolved.tree.layers[layer_index];
    let mark_style = &styles[inputs.nodes.marks[layer_index]];
    let len = draft.labels.len();
    let line = matches!(
        layer.mark.mark_type(),
        MarkType::Line | MarkType::Trail | MarkType::Rule
    );
    // The swatches draw the constant encodings of the layer, as its marks do.
    let constant = |channel| constant_color(inputs.chart, layer.mark, channel);
    let number = |channel| constant_number(inputs.chart, layer.mark, channel);
    let constant_opacity = || -> Result<Option<Vec<f32>>> {
        Ok(number(Channel::Opacity)?.map(|opacity| vec![opacity; len]))
    };
    let mark_color = match constant(Channel::Color)? {
        Some(color) => color,
        None => ColorOrGradient::Color(style_color(mark_style, Property::Color)),
    };
    // A swatch paints as its marks do: a paint that reads `currentColor` takes the color of the
    // swatch, and a gradient takes the place of the paint as it is.
    let mark_node = inputs.nodes.marks[layer_index];
    let through = |property: Property, color: &ColorOrGradient| -> ColorOrGradient {
        match color {
            ColorOrGradient::Color(color) => match styles.paint_with(mark_node, property, *color) {
                Paint::None => ColorOrGradient::transparent(),
                Paint::Color(color) | Paint::Contrast { otherwise: color } => {
                    ColorOrGradient::Color(color)
                }
            },
            gradient => gradient.clone(),
        }
    };
    let follows = |property: Property| {
        matches!(
            mark_style.get(property),
            Computed::Keyword("auto") | Computed::CurrentColor { .. }
        )
    };
    let styled = |property: Property| through(property, &mark_color);
    let entries = |property: Property, colors: &[ColorOrGradient]| {
        ScalarOrArray::new_array(
            colors
                .iter()
                .map(|color| through(property, color))
                .collect(),
        )
    };
    let swatches = if line {
        let mut swatches = LineSwatches {
            stroke: ScalarOrArray::new_scalar(
                constant(Channel::Stroke)?.unwrap_or_else(|| styled(Property::Stroke)),
            ),
            stroke_width: ScalarOrArray::new_scalar(match number(Channel::StrokeWidth)? {
                Some(width) => width,
                None => style_number(mark_style, Property::StrokeWidth),
            }),
            ..Default::default()
        };
        let mut opacity = constant_opacity()?;
        for swatch in draft.swatches {
            match swatch {
                Swatch::Color(colors) => swatches.stroke = entries(Property::Stroke, &colors),
                Swatch::Stroke(colors) | Swatch::Fill(colors) => {
                    swatches.stroke = ScalarOrArray::new_array(colors)
                }
                Swatch::StrokeWidth(widths) | Swatch::Size(widths) => {
                    swatches.stroke_width = ScalarOrArray::new_array(widths)
                }
                Swatch::StrokeDash(dashes) => {
                    swatches.stroke_dash = ScalarOrArray::new_array(
                        (dashes.into_iter())
                            .map(|dash| (!dash.is_empty()).then_some(dash))
                            .collect(),
                    )
                }
                Swatch::Opacity(values) => opacity = Some(values),
                _ => {}
            }
        }
        if let Some(opacity) = opacity {
            swatches.stroke = faded(&swatches.stroke, &opacity, len);
        }
        if draft.rest {
            swatches.stroke = faded(&swatches.stroke, &shown(len), len);
        }
        LegendSwatches::Lines(swatches)
    } else {
        let swatch_style = &styles[nodes.swatch];
        let shape = match layer.mark.mark_type() {
            MarkType::Bar | MarkType::Rect | MarkType::Area => "square".to_string(),
            MarkType::Symbol => match constant_string(inputs.chart, layer.mark, Channel::Shape)? {
                Some(shape) => shape,
                None => mark_style
                    .string(Property::Shape)
                    .expect("a mark has a shape")
                    .to_string(),
            },
            _ => swatch_style
                .string(Property::Shape)
                .expect("a swatch has a shape")
                .to_string(),
        };
        let shape = SymbolShape::from_vega_str(&shape).unwrap_or_default();
        let fill = constant(Channel::Fill)?.unwrap_or_else(|| styled(Property::Fill));
        let stroke = constant(Channel::Stroke)?.unwrap_or_else(|| styled(Property::Stroke));
        let strokes = matches!(&stroke, ColorOrGradient::Color(color) if color[3] > 0.0);
        let mut swatches = SymbolSwatches {
            shape: ScalarOrArray::new_scalar(shape),
            size: ScalarOrArray::new_scalar(match number(Channel::Size)? {
                Some(size) => size,
                None => mark_style.number(Property::Size).unwrap_or(0.0),
            }),
            angle: ScalarOrArray::new_scalar(match number(Channel::Angle)? {
                Some(angle) => angle,
                None => style_number(mark_style, Property::Angle),
            }),
            fill: ScalarOrArray::new_scalar(fill),
            stroke: ScalarOrArray::new_scalar(stroke),
            stroke_width: ScalarOrArray::new_scalar(
                match (strokes, number(Channel::StrokeWidth)?) {
                    (false, _) => 0.0,
                    (true, Some(width)) => width,
                    (true, None) => style_number(mark_style, Property::StrokeWidth),
                },
            ),
            ..Default::default()
        };
        let fills = !follows(Property::Stroke) || follows(Property::Fill);
        let mut opacity = constant_opacity()?;
        for swatch in draft.swatches {
            match swatch {
                Swatch::Color(colors) => {
                    swatches.fill = entries(Property::Fill, &colors);
                    if strokes {
                        swatches.stroke = entries(Property::Stroke, &colors);
                    }
                }
                Swatch::Fill(colors) => swatches.fill = ScalarOrArray::new_array(colors),
                Swatch::Stroke(colors) => swatches.stroke = ScalarOrArray::new_array(colors),
                Swatch::Size(sizes) => swatches.size = ScalarOrArray::new_array(sizes),
                Swatch::Shape(names) => {
                    let shapes = (names.iter())
                        .map(|name| SymbolShape::from_vega_str(name).unwrap_or_default())
                        .collect();
                    swatches.shape = ScalarOrArray::new_array(shapes);
                }
                Swatch::Angle(angles) => swatches.angle = ScalarOrArray::new_array(angles),
                Swatch::Pattern(patterns) => {
                    swatches.fill_pattern = patterns;
                    if !fills {
                        swatches.fill = ScalarOrArray::new_scalar(ColorOrGradient::transparent());
                    }
                }
                Swatch::Opacity(values) => opacity = Some(values),
                Swatch::StrokeWidth(widths) => {
                    swatches.stroke_width = ScalarOrArray::new_array(widths)
                }
                Swatch::StrokeDash(_) => {}
            }
        }
        if let Some(opacity) = opacity {
            swatches.fill = faded(&swatches.fill, &opacity, len);
            swatches.stroke = faded(&swatches.stroke, &opacity, len);
        }
        if draft.rest {
            swatches.fill = faded(&swatches.fill, &shown(len), len);
            swatches.stroke = faded(&swatches.stroke, &shown(len), len);
        }
        LegendSwatches::Symbols(swatches)
    };
    let legend_style = &styles[nodes.legend];
    let title_style = &styles[nodes.title];
    let label_style = &styles[nodes.label];
    let spec = &draft.spec;
    let style = LegendStyle {
        background: background(legend_style),
        title_color: text_color(title_style),
        title_font_family: font_family(title_style),
        title_font_size: title_style.font_size(),
        title_font_weight: font_weight(title_style),
        title_syntax: text_syntax(spec.title.as_ref().map_or(0, |title| title.syntax)),
        label_color: text_color(label_style),
        label_font_family: font_family(label_style),
        label_font_size: label_style.font_size(),
        label_font_weight: font_weight(label_style),
        label_syntax: draft.label_syntax,
        column_gap: style_number(legend_style, Property::ColumnGap),
        row_gap: style_number(legend_style, Property::RowGap),
    };
    let side = side(legend_style);
    let wrap = match legend_style.keyword(Property::Wrap) {
        Some("grid") => LegendWrap::Grid {
            columns: legend_style
                .integer(Property::Columns)
                .and_then(|columns| NonZeroUsize::new(columns as usize)),
        },
        _ => LegendWrap::Flow,
    };
    let title = (visible(title_style) && !draft.title.is_empty()).then_some(draft.title);
    let config = LegendConfig {
        title,
        text: ScalarOrArray::new_array(draft.labels),
        swatches,
        text_padding: style_number(legend_style, Property::LabelPadding),
        layout: LegendLayout { side, wrap },
        style,
    };
    let legend = Legend::new(&config, inputs.engine)?;
    Ok(LegendSlot {
        guide: LegendGuide::Legend(Box::new(legend)),
        side,
        max_length: None,
        offset: offset(legend_style),
    })
}

/// The opacity of the swatches of a legend of `len` entries whose last entry counts the rest: every
/// swatch but the last.
fn shown(len: usize) -> Vec<f32> {
    let mut opacity = vec![1.0; len];
    if let Some(last) = opacity.last_mut() {
        *last = 0.0;
    }
    opacity
}

/// `colors` with their alpha times `opacity`.
fn faded(
    colors: &ScalarOrArray<ColorOrGradient>,
    opacity: &[f32],
    len: usize,
) -> ScalarOrArray<ColorOrGradient> {
    let colors = colors.as_vec(len, None);
    ScalarOrArray::new_array(
        (colors.iter().zip(opacity))
            .map(|(color, opacity)| avenger_color::apply_opacity_to_color(color, *opacity))
            .collect(),
    )
}

/// The colorbar of a continuous color scale, configured from the computed styles of its
/// `legend[type=colorbar]` node and its parts.
fn colorbar(
    inputs: &GuideInputs,
    index: usize,
    group: &ScaleGroup,
    scale: &ConfiguredScale,
) -> Result<LegendSlot> {
    let Some(GuideNodes::Colorbar(nodes)) = &inputs.nodes.guides[index] else {
        unreachable!("a colorbar has the nodes of one");
    };
    let styles = inputs.styles;
    let axis_spec = group.spec.legend.clone().unwrap_or_default();
    let legend_style = &styles[nodes.legend];
    let side = side(legend_style);
    let style = axis_style(
        inputs.chart,
        legend_style,
        &styles[nodes.title],
        &styles[nodes.label],
        &styles[nodes.tick],
        &styles[nodes.rule],
        None,
        axis_spec.ticks.as_ref(),
        text_syntax(axis_spec.title.as_ref().map_or(0, |title| title.syntax)),
    );
    let labels = axis_spec.labels.as_ref();
    let ramp = &styles[nodes.ramp];
    let config = ColorbarConfig {
        axis: AxisConfig {
            side,
            tick_format: labels.map_or_else(String::new, |labels| labels.format.clone()),
            tick_label: labels
                .map(|labels| labels.markup.clone())
                .filter(|markup| !markup.is_empty()),
            style,
        },
        thickness: style_number(ramp, Property::Thickness),
        axis_gap: style_number(legend_style, Property::AxisGap),
        background: background(legend_style),
    };
    let title = guide_title(axis_spec.title.as_ref(), || {
        default_title(inputs.resolved, group, true)
    });
    let colorbar = Colorbar::new(scale, &title, &config, inputs.engine)?;
    Ok(LegendSlot {
        guide: LegendGuide::Colorbar(Box::new(colorbar)),
        side,
        max_length: ramp.number(Property::MaxLength),
        offset: offset(legend_style),
    })
}

// ---------------------------------------------------------------------------------------------
// Titles and attributions

fn title_text_style(style: &ComputedStyle, syntax: TextSyntaxMode) -> TitleTextStyle {
    TitleTextStyle {
        font_family: font_family(style),
        font_size: style.font_size(),
        font_weight: font_weight(style),
        font_style: font_style(style),
        color: text_color(style),
        syntax,
    }
}

/// The plot's title, with its subtitle, from the computed styles of their nodes.
fn title(inputs: &GuideInputs) -> Result<Option<TitleGuide>> {
    let plot = inputs.resolved.tree.plots[0].plot;
    let (Some(title), Some(node)) = (&plot.title, inputs.nodes.title) else {
        return Ok(None);
    };
    let styles = inputs.styles;
    let style = &styles[node];
    if style.is_hidden() {
        return Ok(None);
    }
    let syntax = text_syntax(title.syntax);
    let side = match style.keyword(Property::Orient) {
        Some("bottom" | "bottom-left" | "bottom-right") => GuideSide::Bottom,
        Some("left") => GuideSide::Left,
        Some("right") => GuideSide::Right,
        _ => GuideSide::Top,
    };
    let align = match style.keyword(Property::TextAlign) {
        Some("center") => TitleAlign::Center,
        Some("end") => TitleAlign::End,
        _ => TitleAlign::Start,
    };
    let subtitle_style = inputs
        .nodes
        .subtitle
        .map(|node| title_text_style(&styles[node], syntax));
    let config = TitleConfig {
        side,
        align,
        title: title_text_style(style, syntax),
        subtitle: subtitle_style.unwrap_or_else(|| TitleConfig::default().subtitle),
        ..TitleConfig::default()
    };
    let subtitle_hidden = (inputs.nodes.subtitle).is_some_and(|node| styles[node].is_hidden());
    let subtitle =
        (!title.subtitle.is_empty() && !subtitle_hidden).then_some(title.subtitle.as_str());
    Ok(Some(TitleGuide {
        title: Title::new(&title.text, subtitle, &config, inputs.engine)?,
        side,
        offset: offset(style),
    }))
}

/// The plot's attribution, from the computed style of its node.
fn attribution(inputs: &GuideInputs) -> Result<Option<AttributionGuide>> {
    let plot = inputs.resolved.tree.plots[0].plot;
    let (Some(spec), Some(node)) = (&plot.attribution, inputs.nodes.attribution) else {
        return Ok(None);
    };
    let style = &inputs.styles[node];
    if style.is_hidden() {
        return Ok(None);
    }
    let side = match side(style) {
        GuideSide::Top => GuideSide::Top,
        _ => GuideSide::Bottom,
    };
    let config = AttributionConfig {
        side,
        align: match style.keyword(Property::TextAlign) {
            Some("start") => AttributionAlign::Start,
            _ => AttributionAlign::End,
        },
        separator: (style.string(Property::Separator))
            .expect("`separator` computes to a string")
            .to_string(),
        separator_space: style_number(style, Property::SeparatorSpace),
        font_family: font_family(style),
        font_size: style.font_size(),
        font_weight: font_weight(style),
        color: text_color(style),
        line_gap: style_number(style, Property::LineGap),
        background: background(style),
    };
    Ok(Some(AttributionGuide {
        attribution: Attribution::new(&spec.credits, &config, inputs.engine)?,
        side,
        offset: offset(style),
    }))
}

/// The guides of a chart of one plot.
pub(crate) fn guides(inputs: &GuideInputs) -> Result<Guides> {
    Ok(Guides {
        axes: axes(inputs)?,
        legends: legends(inputs)?,
        title: title(inputs)?,
        attribution: attribution(inputs)?,
    })
}

#[cfg(test)]
mod tests;
