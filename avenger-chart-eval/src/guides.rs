//! The prepared guides of a plot: its axes, its legends and colorbars, its title, and its
//! attribution. The layout measures each through its `extent`, and the scene draws each at the
//! same length.

use std::{collections::BTreeMap, ops::RangeInclusive, sync::Arc};

use arrow::{
    array::{Array, ArrayRef, Float64Array},
    compute::cast,
    datatypes::DataType,
};
use avenger_color::{parse_color_string, ColorOrGradient};
use avenger_common::{
    pattern::PatternFills,
    types::{FontWeight, TextAlign, TextBaseline, TextSyntaxMode},
    value::ScalarOrArray,
};
use avenger_format::PrecisionSharing;
use avenger_guides::{
    attribution::{Attribution, AttributionAlign, AttributionConfig},
    axis::{Axis, AxisConfig, AxisPart, AxisTickInterval, AxisTicks},
    legend::{
        Colorbar, ColorbarConfig, Legend, LegendConfig, LegendLayout, LegendSwatches, LegendWrap,
        LineSwatches, SymbolSwatches,
    },
    BackgroundStyle, GuideSide, PartExtent,
};
use avenger_scales::scales::{coerce::Coercer, fitting::FittingScale, ConfiguredScale};
use avenger_scenegraph::marks::{group::SceneGroup, text::SceneTextMark};
use avenger_typst_label::{bind, escape_text, LabelEngine, LabelValue, LabelValues, TextBounds};

use avenger_chart::{
    proto::{
        self, axis, calendar_ticks, encoding, guide, legend_wrap, mark, tick_interval, Channel,
    },
    resolve::{scale_channel, ScaleGroup, UnitRef},
    Chart, FitKind,
};

use crate::{
    encode::{at, symbol_filled},
    scale::eval_error,
    theme::{TitleAlign, TitleStyle},
    EvalError, Theme,
};

type Result<T> = std::result::Result<T, EvalError>;

fn guide_error(error: impl std::fmt::Display) -> EvalError {
    eval_error("a guide", error)
}

/// A CSS color of the chart.
fn css(color: &str) -> Result<[f32; 4]> {
    parse_color_string(color)
        .ok_or_else(|| EvalError::Evaluation(format!("`{color}` is not a CSS color")))
}

fn side(side: i32, default: GuideSide) -> GuideSide {
    match proto::Side::try_from(side) {
        Ok(proto::Side::Bottom) => GuideSide::Bottom,
        Ok(proto::Side::Top) => GuideSide::Top,
        Ok(proto::Side::Left) => GuideSide::Left,
        Ok(proto::Side::Right) => GuideSide::Right,
        _ => default,
    }
}

fn syntax(syntax: i32, default: TextSyntaxMode) -> TextSyntaxMode {
    match proto::TextSyntax::try_from(syntax) {
        Ok(proto::TextSyntax::Plain) => TextSyntaxMode::Plain,
        Ok(proto::TextSyntax::PlainLines) => TextSyntaxMode::PlainLines,
        Ok(proto::TextSyntax::Typst) => TextSyntaxMode::TypstMarkup,
        _ => default,
    }
}

/// A text style of the chart over the font family, size, weight, and color of the theme.
fn text_style(
    style: Option<&proto::TextStyle>,
    family: &mut String,
    size: &mut f32,
    weight: &mut FontWeight,
    color: &mut [f32; 4],
) -> Result<()> {
    let Some(style) = style else {
        return Ok(());
    };
    if !style.font_family.is_empty() {
        *family = style.font_family.clone();
    }
    if let Some(font_size) = style.font_size {
        *size = font_size;
    }
    if let Some(font_weight) = style.font_weight {
        *weight = FontWeight(font_weight as u16);
    }
    if !style.color.is_empty() {
        *color = css(&style.color)?;
    }
    Ok(())
}

fn background(
    background: Option<&proto::Background>,
    theme: &BackgroundStyle,
) -> Result<BackgroundStyle> {
    let mut style = theme.clone();
    if let Some(background) = background {
        if !background.fill.is_empty() {
            style.fill = ColorOrGradient::Color(css(&background.fill)?);
        }
        if !background.stroke.is_empty() {
            style.stroke = Some(ColorOrGradient::Color(css(&background.stroke)?));
        }
        if let Some(radius) = background.corner_radius {
            style.corner_radius = radius;
        }
        if let Some(padding) = background.padding {
            style.padding = padding;
        }
    }
    Ok(style)
}

/// The title of a guide that sets none: the distinct titles of the encodings of its group, in
/// tree order, joined by `, `, where an encoding without a title takes the name of its column.
pub(crate) fn default_title(units: &[UnitRef], group: &ScaleGroup) -> String {
    let mut titles: Vec<String> = Vec::new();
    for member in &group.members {
        if scale_channel(member.channel) != member.channel {
            continue;
        }
        let Some(encoding) = units[member.unit]
            .unit
            .encodings
            .iter()
            .find(|encoding| encoding.channel == member.channel as i32)
        else {
            continue;
        };
        let title = match (&encoding.title, &encoding.value) {
            (Some(title), _) => title.clone(),
            (None, Some(encoding::Value::Column(name))) => name.clone(),
            _ => String::new(),
        };
        if !title.is_empty() && !titles.contains(&title) {
            titles.push(title);
        }
    }
    titles.join(", ")
}

/// Label text and how it reads: each value through `template`, Typst markup in which `#tick` is
/// the value, or else the values as plain text unless one of them is markup.
fn label_text(values: Vec<LabelValue>, template: &str) -> Result<(Vec<String>, TextSyntaxMode)> {
    if !template.is_empty() {
        let text = values
            .into_iter()
            .map(|value| bind(template, &LabelValues::from([("tick".into(), value)])))
            .collect::<std::result::Result<_, _>>()
            .map_err(guide_error)?;
        return Ok((text, TextSyntaxMode::TypstMarkup));
    }
    let markup = values
        .iter()
        .any(|value| matches!(value, LabelValue::Markup(_)));
    let text = values
        .into_iter()
        .map(|value| match value {
            LabelValue::Markup(markup) => markup,
            LabelValue::Str(text) if markup => escape_text(&text),
            LabelValue::Str(text) => text,
            _ => String::new(),
        })
        .collect();
    let syntax = if markup {
        TextSyntaxMode::TypstMarkup
    } else {
        TextSyntaxMode::Plain
    };
    Ok((text, syntax))
}

/// The formatted text of `values` through `pattern`.
fn formatted(engine: &LabelEngine, values: &ArrayRef, pattern: &str) -> Result<Vec<LabelValue>> {
    engine
        .format_array(values, pattern, PrecisionSharing::Shared)
        .map_err(guide_error)
}

/// An axis of a position group, on its side.
pub(crate) struct AxisGuide {
    pub(crate) axis: Axis,
    pub(crate) side: GuideSide,
    /// Whether the plot draws grid lines at the axis's ticks.
    pub(crate) grid: bool,
}

/// A legend or a colorbar.
pub(crate) enum LegendGuide {
    Legend(Box<Legend>),
    Colorbar(Box<Colorbar>),
}

/// A legend or a colorbar on its side of the plot.
pub(crate) struct LegendSlot {
    pub(crate) guide: LegendGuide,
    pub(crate) side: GuideSide,
    /// The longest that a colorbar runs along its side.
    pub(crate) max_length: Option<f32>,
}

impl LegendSlot {
    pub(crate) fn extent(&self, length: f32) -> PartExtent {
        match &self.guide {
            LegendGuide::Legend(legend) => legend.extent(length),
            LegendGuide::Colorbar(colorbar) => colorbar.extent(length),
        }
    }

    pub(crate) fn origin(&self, slot: [f32; 4]) -> [f32; 2] {
        match &self.guide {
            LegendGuide::Legend(legend) => legend.origin(slot),
            LegendGuide::Colorbar(colorbar) => colorbar.origin(slot),
        }
    }

    pub(crate) fn group(&self, length: f32) -> SceneGroup {
        match &self.guide {
            LegendGuide::Legend(legend) => legend.marks(length).group,
            LegendGuide::Colorbar(colorbar) => colorbar.marks(length).group,
        }
    }

    pub(crate) fn max_content(&self) -> Option<f32> {
        match &self.guide {
            LegendGuide::Legend(legend) => Some(legend.max_content()),
            LegendGuide::Colorbar(_) => None,
        }
    }
}

/// The prepared guides of a plot.
pub(crate) struct Guides {
    pub(crate) axes: Vec<AxisGuide>,
    pub(crate) legends: Vec<LegendSlot>,
    pub(crate) title: Option<TitleGuide>,
    pub(crate) attribution: Option<Attribution>,
    /// The side of the attribution: the top or the bottom.
    pub(crate) attribution_side: GuideSide,
}

/// What the guides of a plot are made from.
pub(crate) struct GuideInputs<'a> {
    pub(crate) chart: &'a Chart,
    pub(crate) units: &'a [UnitRef<'a>],
    pub(crate) groups: &'a [ScaleGroup],
    /// The fitting scale of each position group, by group.
    pub(crate) positions: &'a BTreeMap<usize, FittingScale>,
    /// The configured scale of each non-position group, by group.
    pub(crate) scales: &'a BTreeMap<usize, ConfiguredScale>,
    pub(crate) theme: &'a Theme,
    pub(crate) engine: &'a LabelEngine,
}

/// The guides of a plot: an axis for each position group whose guide is an axis, a legend or a
/// colorbar for each non-position group whose guide is one, with the discrete legends of one
/// owner and equal entries merged into one, and the title and the attribution of its unit.
pub(crate) fn guides(inputs: &GuideInputs) -> Result<Guides> {
    let mut axes = Vec::new();
    for (index, fitting) in inputs.positions {
        if let Some(axis) = axis_guide(inputs, &inputs.groups[*index], fitting)? {
            axes.push(axis);
        }
    }
    let legends = legends(inputs)?;
    let unit = inputs.units.first().map(|unit| unit.unit);
    let title = match unit.and_then(|unit| unit.title.as_ref()) {
        Some(title) if !title.text.is_empty() => {
            Some(TitleGuide::new(title, &inputs.theme.title, inputs.engine)?)
        }
        _ => None,
    };
    let spec = unit
        .and_then(|unit| unit.attribution.as_ref())
        .filter(|attribution| !attribution.credits.is_empty());
    let attribution = spec
        .map(|spec| attribution_guide(spec, &inputs.theme.attribution, inputs.engine))
        .transpose()?;
    let attribution_side = match spec.map(|spec| side(spec.side, inputs.theme.attribution.side)) {
        Some(GuideSide::Top) => GuideSide::Top,
        Some(_) => GuideSide::Bottom,
        None => inputs.theme.attribution.side,
    };
    Ok(Guides {
        axes,
        legends,
        title,
        attribution,
        attribution_side,
    })
}

fn axis_guide(
    inputs: &GuideInputs,
    group: &ScaleGroup,
    fitting: &FittingScale,
) -> Result<Option<AxisGuide>> {
    let spec_axis = match group
        .spec
        .guide
        .as_ref()
        .and_then(|guide| guide.guide.as_ref())
    {
        Some(guide::Guide::Axis(axis)) => axis.clone(),
        Some(_) => return Ok(None),
        None if matches!(group.channel, Channel::X | Channel::Y) => proto::Axis::default(),
        None => return Ok(None),
    };
    let default_side = if group.channel == Channel::Y {
        GuideSide::Left
    } else {
        GuideSide::Bottom
    };
    let side = side(spec_axis.side, default_side);
    let scale_type = group.spec.r#type.as_str();
    let continuous = FitKind::of(scale_type) == FitKind::Interval;
    let numeric = numeric(scale_type);
    let (config, grid) = axis_config(
        inputs.chart,
        &spec_axis,
        side,
        continuous,
        numeric,
        inputs.theme,
    )?;
    let title = spec_axis
        .title
        .clone()
        .unwrap_or_else(|| default_title(inputs.units, group));
    let engine = inputs.engine;
    let axis = match scale_type {
        "band" => Axis::band(fitting.clone(), &title, &config, engine),
        "point" => Axis::point(fitting.clone(), &title, &config, engine),
        "nested_band" => Axis::nested_band(fitting.clone(), &title, &[], &config, engine),
        _ => Axis::continuous(fitting.clone(), &title, &config, engine),
    }
    .map_err(guide_error)?;
    Ok(Some(AxisGuide { axis, side, grid }))
}

/// The configuration of an axis from its spec over the theme, and whether it draws grid lines.
fn axis_config(
    chart: &Chart,
    spec: &proto::Axis,
    side: GuideSide,
    continuous: bool,
    numeric: bool,
    theme: &Theme,
) -> Result<(AxisConfig, bool)> {
    let mut style = theme.axis.clone();
    if let Some(count) = spec.tick_count {
        style.tick_count = Some(count);
    }
    match &spec.ticks {
        Some(axis::Ticks::TickInterval(interval)) => {
            style.ticks = match &interval.interval {
                Some(tick_interval::Interval::Numeric(numeric)) => {
                    Some(AxisTicks::StartStep(AxisTickInterval::Numeric {
                        start: numeric.start,
                        step: numeric.step,
                    }))
                }
                Some(tick_interval::Interval::Calendar(calendar)) => {
                    let (months, days, nanos) = match calendar.step {
                        Some(calendar_ticks::Step::Months(months)) => (months, 0, 0),
                        Some(calendar_ticks::Step::Days(days)) => (0, days, 0),
                        Some(calendar_ticks::Step::Milliseconds(millis)) => {
                            (0, 0, millis * 1_000_000)
                        }
                        None => (0, 0, 0),
                    };
                    Some(AxisTicks::StartStep(AxisTickInterval::Temporal {
                        start_millis: calendar.start,
                        months,
                        days,
                        nanos,
                    }))
                }
                None => None,
            }
        }
        Some(axis::Ticks::TickValues(index)) => {
            style.ticks = Some(AxisTicks::Values(chart.list(*index)));
        }
        None => {}
    }
    if let Some(length) = spec.tick_length {
        style.tick_length = length;
    }
    if !spec.tick_color.is_empty() {
        style.tick_color = css(&spec.tick_color)?;
    }
    if let Some(labels) = spec.labels {
        style.labels_visible = labels;
    }
    if let Some(angle) = spec.label_angle {
        style.label_angle = angle;
    }
    style.label_align = match proto::TextAlign::try_from(spec.label_align) {
        Ok(proto::TextAlign::Left) => Some(TextAlign::Left),
        Ok(proto::TextAlign::Center) => Some(TextAlign::Center),
        Ok(proto::TextAlign::Right) => Some(TextAlign::Right),
        _ => style.label_align,
    };
    style.label_baseline = match proto::TextBaseline::try_from(spec.label_baseline) {
        Ok(proto::TextBaseline::Top) => Some(TextBaseline::Top),
        Ok(proto::TextBaseline::Middle) => Some(TextBaseline::Middle),
        Ok(proto::TextBaseline::Alphabetic) => Some(TextBaseline::Alphabetic),
        Ok(proto::TextBaseline::Bottom) => Some(TextBaseline::Bottom),
        _ => style.label_baseline,
    };
    text_style(
        spec.label_style.as_ref(),
        &mut style.label_font_family,
        &mut style.label_font_size,
        &mut style.label_font_weight,
        &mut style.label_color,
    )?;
    style.title_syntax = syntax(spec.title_syntax, style.title_syntax);
    text_style(
        spec.title_style.as_ref(),
        &mut style.title_font_family,
        &mut style.title_font_size,
        &mut style.title_font_weight,
        &mut style.title_color,
    )?;
    if !spec.domain_color.is_empty() {
        style.domain_color = css(&spec.domain_color)?;
    }
    if !spec.grid_color.is_empty() {
        style.grid_color = css(&spec.grid_color)?;
    }
    if let Some(width) = spec.grid_width {
        style.grid_width = width;
    }
    let grid = spec.grid.unwrap_or(theme.grid && continuous);
    Ok((
        AxisConfig {
            side,
            tick_format: format(&spec.format, numeric, theme),
            tick_label: (!spec.label.is_empty()).then(|| spec.label.clone()),
            style,
        },
        grid,
    ))
}

/// Whether a scale of `scale_type` maps numbers along a continuous domain, whose labels take
/// the theme's number format when they set no format.
fn numeric(scale_type: &str) -> bool {
    matches!(
        scale_type,
        "linear" | "log" | "pow" | "symlog" | "quantize" | "quantile" | "threshold"
    )
}

/// The pattern of labels: the spec's, or the theme's number format for numbers.
fn format(pattern: &str, numeric: bool, theme: &Theme) -> String {
    if pattern.is_empty() && numeric {
        theme.number_format.clone()
    } else {
        pattern.to_string()
    }
}

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

/// A legend before its swatches are made: its entries' labels, its title, and the swatch values
/// of each of its channels.
struct LegendDraft {
    unit: usize,
    spec: proto::Legend,
    /// The categories of a discrete scale, which merge legends with equal categories.
    categories: Option<ArrayRef>,
    title: String,
    labels: Vec<String>,
    label_syntax: TextSyntaxMode,
    swatches: Vec<Swatch>,
}

/// The values of `values` through `scale` as the swatch of `channel`.
fn swatch(channel: Channel, scale: &ConfiguredScale, values: &ArrayRef) -> Result<Swatch> {
    let len = values.len();
    let numbers = || -> Result<Vec<f32>> {
        Ok(scale
            .scale_to_numeric(values)
            .map_err(guide_error)?
            .as_vec(len, None))
    };
    let colors = || -> Result<Vec<ColorOrGradient>> {
        Ok(scale
            .scale_to_color(values)
            .map_err(guide_error)?
            .as_vec(len, None))
    };
    Ok(match channel {
        Channel::Color => Swatch::Color(colors()?),
        Channel::Fill => Swatch::Fill(colors()?),
        Channel::Stroke => Swatch::Stroke(colors()?),
        Channel::Opacity | Channel::FillOpacity | Channel::StrokeOpacity => {
            Swatch::Opacity(numbers()?)
        }
        Channel::Size => Swatch::Size(numbers()?),
        Channel::StrokeWidth => Swatch::StrokeWidth(numbers()?),
        Channel::Angle => Swatch::Angle(numbers()?),
        Channel::Shape => Swatch::Shape(
            scale
                .scale_to_string(values)
                .map_err(guide_error)?
                .as_vec(len, None),
        ),
        Channel::StrokeDash => Swatch::StrokeDash(
            Coercer::default()
                .to_stroke_dash(&scale.scale(values).map_err(guide_error)?)
                .map_err(guide_error)?
                .as_vec(len, None),
        ),
        Channel::Pattern => Swatch::Pattern(PatternFills(ScalarOrArray::new_array(
            scale
                .scale_to_pattern(values)
                .map_err(guide_error)?
                .as_vec(len, None),
        ))),
        other => {
            return Err(EvalError::Evaluation(format!(
                "a legend of `{}` has no swatch",
                other.as_str_name()
            )))
        }
    })
}

/// The entries of the legend of a scale: its categories, the bins of a quantize, a quantile, or
/// a threshold scale, or the ticks of another continuous scale, with each entry's input to the
/// scale and its label.
fn entries(
    scale: &ConfiguredScale,
    spec: &proto::Legend,
    chart: &Chart,
    engine: &LabelEngine,
    theme: &Theme,
) -> Result<(ArrayRef, Vec<String>, TextSyntaxMode)> {
    let template = spec.label.as_str();
    let scale_type = scale.scale_impl.scale_type();
    let pattern = format(&spec.format, numeric(scale_type), theme);
    match scale_type {
        "quantize" | "quantile" | "threshold" => {
            // The edges between bins, and for a quantize scale, the ends of its domain.
            let edges: Vec<f64> = match scale_type {
                "quantize" => {
                    let (low, high) = scale.numeric_interval_domain().map_err(guide_error)?;
                    let bins = scale.range().len().max(1);
                    (0..=bins)
                        .map(|bin| low + (high - low) * bin as f64 / bins as f64)
                        .collect()
                }
                "quantile" => numbers(&scale.ticks(None).map_err(guide_error)?)?,
                _ => numbers(scale.domain())?,
            };
            let text = formatted(
                engine,
                &(Arc::new(Float64Array::from(edges.clone())) as ArrayRef),
                &pattern,
            )?;
            let (text, syntax) = label_text(text, template)?;
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
        _ if FitKind::of(scale_type) == FitKind::Categories => {
            let values = scale.domain().clone();
            let (labels, syntax) = label_text(formatted(engine, &values, &spec.format)?, template)?;
            Ok((values, labels, syntax))
        }
        _ => {
            let values = match spec.values {
                Some(index) => chart.list(index),
                None => scale
                    .ticks(Some(spec.tick_count.unwrap_or(5.0)))
                    .map_err(guide_error)?,
            };
            let (labels, syntax) = label_text(formatted(engine, &values, &pattern)?, template)?;
            Ok((values, labels, syntax))
        }
    }
}

fn numbers(values: &ArrayRef) -> Result<Vec<f64>> {
    let values = cast(values, &DataType::Float64)?;
    Ok(values
        .as_any()
        .downcast_ref::<Float64Array>()
        .expect("a Float64 array")
        .iter()
        .flatten()
        .collect())
}

fn legends(inputs: &GuideInputs) -> Result<Vec<LegendSlot>> {
    let mut drafts: Vec<LegendDraft> = Vec::new();
    let mut slots = Vec::new();
    for (index, scale) in inputs.scales {
        let group = &inputs.groups[*index];
        let ticked = matches!(
            group.spec.r#type.as_str(),
            "linear" | "log" | "pow" | "symlog" | "time"
        );
        let color = matches!(
            group.channel,
            Channel::Color | Channel::Fill | Channel::Stroke
        );
        let guide = match group
            .spec
            .guide
            .as_ref()
            .and_then(|guide| guide.guide.as_ref())
        {
            Some(guide) => guide.clone(),
            None if color && ticked => guide::Guide::Colorbar(proto::Colorbar::default()),
            None => guide::Guide::Legend(proto::Legend::default()),
        };
        let unit = group.members.first().map_or(0, |member| member.unit);
        match guide {
            guide::Guide::Colorbar(colorbar) => {
                slots.push(colorbar_guide(inputs, group, scale, &colorbar)?);
            }
            guide::Guide::Legend(spec) => {
                let (values, labels, label_syntax) =
                    entries(scale, &spec, inputs.chart, inputs.engine, inputs.theme)?;
                let swatch = swatch(group.channel, scale, &values)?;
                let title = spec
                    .title
                    .clone()
                    .unwrap_or_else(|| default_title(inputs.units, group));
                let categories = (FitKind::of(&group.spec.r#type) == FitKind::Categories)
                    .then(|| scale.domain().clone());
                // The discrete legends of one owner with equal categories and equal specs merge.
                let merged = drafts.iter_mut().find(|draft| {
                    draft.unit == unit
                        && draft.spec == spec
                        && matches!((&draft.categories, &categories),
                            (Some(a), Some(b)) if a.as_ref() == b.as_ref())
                });
                match merged {
                    Some(draft) => {
                        if !title.is_empty() && !draft.title.split(", ").any(|known| known == title)
                        {
                            if draft.title.is_empty() {
                                draft.title = title;
                            } else {
                                draft.title = format!("{}, {title}", draft.title);
                            }
                        }
                        draft.swatches.push(swatch);
                    }
                    None => drafts.push(LegendDraft {
                        unit,
                        spec,
                        categories,
                        title,
                        labels,
                        label_syntax,
                        swatches: vec![swatch],
                    }),
                }
            }
            _ => {}
        }
    }
    for draft in drafts {
        slots.push(legend_guide(inputs, draft)?);
    }
    Ok(slots)
}

/// The color of the constant encoding of `channel` of `unit`, when it has no conditions: the
/// color that every row of the unit draws.
fn constant_color(
    chart: &Chart,
    unit: &proto::Unit,
    channel: Channel,
) -> Result<Option<ColorOrGradient>> {
    let Some(encoding) = unit
        .encodings
        .iter()
        .find(|encoding| encoding.channel == channel as i32)
    else {
        return Ok(None);
    };
    match encoding.value {
        Some(encoding::Value::Constant(index)) if encoding.conditions.is_empty() => {
            let colors = Coercer::default()
                .to_color(chart.value(index), None)
                .map_err(guide_error)?;
            Ok(Some(at(&colors, 0)))
        }
        _ => Ok(None),
    }
}

fn legend_guide(inputs: &GuideInputs, draft: LegendDraft) -> Result<LegendSlot> {
    let theme = inputs.theme;
    let spec = &draft.spec;
    let unit = inputs.units[draft.unit].unit;
    let mark = unit.mark.as_ref().and_then(|mark| mark.mark.as_ref());
    let len = draft.labels.len();
    let line = matches!(
        mark,
        Some(mark::Mark::Line(_) | mark::Mark::Trail(_) | mark::Mark::Rule(_))
    );
    // The swatches draw the mark as its unit does: a channel that the legend does not show takes
    // the unit's constant color, or the theme's.
    let constant = |channel| constant_color(inputs.chart, unit, channel);
    let mark_color = constant(Channel::Color)?.unwrap_or(ColorOrGradient::Color(theme.mark_color));
    let swatches = if line {
        let mut swatches = LineSwatches {
            stroke: ScalarOrArray::new_scalar(constant(Channel::Stroke)?.unwrap_or(mark_color)),
            stroke_width: ScalarOrArray::new_scalar(theme.stroke_width),
            ..Default::default()
        };
        let mut opacity: Option<Vec<f32>> = None;
        for swatch in draft.swatches {
            match swatch {
                Swatch::Color(colors) | Swatch::Stroke(colors) | Swatch::Fill(colors) => {
                    swatches.stroke = ScalarOrArray::new_array(colors)
                }
                Swatch::StrokeWidth(widths) | Swatch::Size(widths) => {
                    swatches.stroke_width = ScalarOrArray::new_array(widths)
                }
                Swatch::StrokeDash(dashes) => {
                    swatches.stroke_dash = ScalarOrArray::new_array(
                        dashes
                            .into_iter()
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
        LegendSwatches::Lines(swatches)
    } else {
        let filled = match mark {
            Some(mark::Mark::Symbol(symbol)) => symbol_filled(symbol),
            _ => true,
        };
        let shape = match mark {
            Some(mark::Mark::Bar(_) | mark::Mark::Rect(_) | mark::Mark::Area(_)) => "square",
            _ => "circle",
        };
        let shape = avenger_common::types::SymbolShape::from_vega_str(shape).unwrap_or_default();
        let mut swatches = SymbolSwatches {
            shape: ScalarOrArray::new_scalar(shape),
            size: ScalarOrArray::new_scalar(theme.symbol_size.max(20.0)),
            ..Default::default()
        };
        let transparent = ColorOrGradient::transparent();
        let fill = constant(Channel::Fill)?;
        let stroke = constant(Channel::Stroke)?;
        if filled {
            swatches.fill = ScalarOrArray::new_scalar(fill.unwrap_or(mark_color));
            if let Some(stroke) = stroke {
                swatches.stroke = ScalarOrArray::new_scalar(stroke);
                swatches.stroke_width = Some(1.5);
            }
        } else {
            swatches.stroke = ScalarOrArray::new_scalar(stroke.unwrap_or(mark_color));
            swatches.stroke_width = Some(1.5);
            if let Some(fill) = fill {
                swatches.fill = ScalarOrArray::new_scalar(fill);
            }
        }
        let mut opacity: Option<Vec<f32>> = None;
        for swatch in draft.swatches {
            match swatch {
                Swatch::Color(colors) if filled => swatches.fill = ScalarOrArray::new_array(colors),
                Swatch::Color(colors) => swatches.stroke = ScalarOrArray::new_array(colors),
                Swatch::Fill(colors) => swatches.fill = ScalarOrArray::new_array(colors),
                Swatch::Stroke(colors) => {
                    swatches.stroke = ScalarOrArray::new_array(colors);
                    swatches.stroke_width.get_or_insert(1.5);
                }
                Swatch::Size(sizes) => swatches.size = ScalarOrArray::new_array(sizes),
                Swatch::Shape(names) => {
                    let shapes = names
                        .iter()
                        .map(|name| {
                            avenger_common::types::SymbolShape::from_vega_str(name)
                                .unwrap_or_default()
                        })
                        .collect();
                    swatches.shape = ScalarOrArray::new_array(shapes);
                }
                Swatch::Angle(angles) => swatches.angle = ScalarOrArray::new_array(angles),
                Swatch::Pattern(patterns) => {
                    swatches.fill_pattern = patterns;
                    if !filled {
                        swatches.fill = ScalarOrArray::new_scalar(transparent.clone());
                    }
                }
                Swatch::Opacity(values) => opacity = Some(values),
                Swatch::StrokeWidth(widths) => {
                    swatches.stroke_width = widths.iter().copied().reduce(f32::max)
                }
                Swatch::StrokeDash(_) => {}
            }
        }
        if let Some(opacity) = opacity {
            swatches.fill = faded(&swatches.fill, &opacity, len);
            swatches.stroke = faded(&swatches.stroke, &opacity, len);
        }
        LegendSwatches::Symbols(swatches)
    };
    let mut style = theme.legend.clone();
    style.title_syntax = syntax(spec.title_syntax, style.title_syntax);
    text_style(
        spec.title_style.as_ref(),
        &mut style.title_font_family,
        &mut style.title_font_size,
        &mut style.title_font_weight,
        &mut style.title_color,
    )?;
    style.label_syntax = syntax(spec.label_syntax, draft.label_syntax);
    text_style(
        spec.label_style.as_ref(),
        &mut style.label_font_family,
        &mut style.label_font_size,
        &mut style.label_font_weight,
        &mut style.label_color,
    )?;
    if let Some(gap) = spec.column_gap {
        style.column_gap = gap;
    }
    if let Some(gap) = spec.row_gap {
        style.row_gap = gap;
    }
    style.background = background(spec.background.as_ref(), &style.background)?;
    let side = side(spec.side, GuideSide::Right);
    let wrap = match spec.wrap.as_ref().and_then(|wrap| wrap.wrap.as_ref()) {
        Some(legend_wrap::Wrap::Flow(_)) => LegendWrap::Flow,
        Some(legend_wrap::Wrap::Grid(grid)) => LegendWrap::Grid {
            columns: grid
                .columns
                .and_then(|columns| std::num::NonZeroUsize::new(columns as usize)),
        },
        None => LegendWrap::Grid { columns: None },
    };
    let config = LegendConfig {
        title: (!draft.title.is_empty()).then_some(draft.title),
        text: ScalarOrArray::new_array(draft.labels),
        swatches,
        text_padding: spec.label_padding.unwrap_or(2.0),
        layout: LegendLayout { side, wrap },
        style,
    };
    let legend = Legend::new(&config, inputs.engine).map_err(guide_error)?;
    Ok(LegendSlot {
        guide: LegendGuide::Legend(Box::new(legend)),
        side,
        max_length: None,
    })
}

/// `colors` with their alpha times `opacity`.
fn faded(
    colors: &ScalarOrArray<ColorOrGradient>,
    opacity: &[f32],
    len: usize,
) -> ScalarOrArray<ColorOrGradient> {
    let colors = colors.as_vec(len, None);
    ScalarOrArray::new_array(
        colors
            .iter()
            .zip(opacity)
            .map(|(color, opacity)| avenger_color::apply_opacity_to_color(color, *opacity))
            .collect(),
    )
}

fn colorbar_guide(
    inputs: &GuideInputs,
    group: &ScaleGroup,
    scale: &ConfiguredScale,
    spec: &proto::Colorbar,
) -> Result<LegendSlot> {
    let theme = inputs.theme;
    let axis_spec = spec.axis.clone().unwrap_or_default();
    let side = side(axis_spec.side, theme.colorbar.axis.side);
    let numeric = numeric(&group.spec.r#type);
    let (axis, _) = axis_config(inputs.chart, &axis_spec, side, true, numeric, theme)?;
    let mut config: ColorbarConfig = theme.colorbar.clone();
    config.axis = AxisConfig {
        style: avenger_guides::axis::AxisStyle {
            label_font_size: theme.legend.label_font_size,
            title_font_size: theme.legend.title_font_size,
            title_font_weight: theme.legend.title_font_weight,
            ..axis.style
        },
        ..axis
    };
    if let Some(thickness) = spec.thickness {
        config.thickness = thickness;
    }
    if let Some(gap) = spec.axis_gap {
        config.axis_gap = gap;
    }
    config.background = background(spec.background.as_ref(), &config.background)?;
    let title = axis_spec
        .title
        .clone()
        .unwrap_or_else(|| default_title(inputs.units, group));
    let colorbar = Colorbar::new(scale, &title, &config, inputs.engine).map_err(guide_error)?;
    Ok(LegendSlot {
        guide: LegendGuide::Colorbar(Box::new(colorbar)),
        side,
        max_length: spec.max_length.or(theme.colorbar_length),
    })
}

fn attribution_guide(
    spec: &proto::Attribution,
    theme: &AttributionConfig,
    engine: &LabelEngine,
) -> Result<Attribution> {
    let mut config = theme.clone();
    config.side = match side(spec.side, GuideSide::Bottom) {
        GuideSide::Top => GuideSide::Top,
        _ => GuideSide::Bottom,
    };
    config.align = match proto::AttributionAlign::try_from(spec.align) {
        Ok(proto::AttributionAlign::Start) => AttributionAlign::Start,
        Ok(proto::AttributionAlign::End) => AttributionAlign::End,
        _ => config.align,
    };
    if !spec.separator.is_empty() {
        config.separator = spec.separator.clone();
    }
    if let Some(space) = spec.separator_space {
        config.separator_space = space;
    }
    text_style(
        spec.text_style.as_ref(),
        &mut config.font_family,
        &mut config.font_size,
        &mut config.font_weight,
        &mut config.color,
    )?;
    if let Some(gap) = spec.line_gap {
        config.line_gap = gap;
    }
    config.background = background(spec.background.as_ref(), &config.background)?;
    Attribution::new(&spec.credits, &config, engine).map_err(guide_error)
}

/// The title of a node, with its subtitle, on one side of the node.
pub(crate) struct TitleGuide {
    pub(crate) side: GuideSide,
    align: TitleAlign,
    /// The title's line and the subtitle's line, each with its measured bounds.
    lines: Vec<(SceneTextMark, TextBounds)>,
    gap: f32,
    offset: f32,
}

impl TitleGuide {
    fn new(title: &proto::Title, theme: &TitleStyle, engine: &LabelEngine) -> Result<Self> {
        let side = match proto::TitleSide::try_from(title.side) {
            Ok(proto::TitleSide::Bottom) => GuideSide::Bottom,
            Ok(proto::TitleSide::Left) => GuideSide::Left,
            Ok(proto::TitleSide::Right) => GuideSide::Right,
            _ => GuideSide::Top,
        };
        let align = match proto::TitleAlign::try_from(title.align) {
            Ok(proto::TitleAlign::Center) => TitleAlign::Center,
            Ok(proto::TitleAlign::End) => TitleAlign::End,
            Ok(proto::TitleAlign::Start) => TitleAlign::Start,
            _ => theme.align,
        };
        let text_syntax = syntax(title.syntax, TextSyntaxMode::Plain);
        let line = |text: &str,
                    style: Option<&proto::TextStyle>,
                    mut family: String,
                    mut size: f32,
                    mut weight: FontWeight,
                    mut color: [f32; 4]|
         -> Result<(SceneTextMark, TextBounds)> {
            text_style(style, &mut family, &mut size, &mut weight, &mut color)?;
            let mark = SceneTextMark {
                len: 1,
                text: ScalarOrArray::new_scalar(text.to_string()),
                text_syntax,
                font: ScalarOrArray::new_scalar(family),
                font_size: ScalarOrArray::new_scalar(size),
                font_weight: ScalarOrArray::new_scalar(weight),
                color: ScalarOrArray::new_scalar(color),
                ..Default::default()
            };
            let label = mark.labels().next().expect("a label of one text");
            let bounds = engine.bounds(&label.label).map_err(guide_error)?;
            Ok((mark, bounds))
        };
        let mut lines = vec![line(
            &title.text,
            title.style.as_ref(),
            theme.font_family.clone(),
            theme.font_size,
            theme.font_weight,
            theme.color,
        )?];
        if !title.subtitle.is_empty() {
            lines.push(line(
                &title.subtitle,
                title.subtitle_style.as_ref(),
                theme.font_family.clone(),
                theme.subtitle_font_size,
                theme.subtitle_font_weight,
                theme.subtitle_color,
            )?);
        }
        Ok(Self {
            side,
            align,
            lines,
            gap: theme.subtitle_gap,
            offset: theme.offset,
        })
    }

    /// The thickness of the title's lines and the space between them.
    fn block(&self) -> f32 {
        let lines: f32 = self.lines.iter().map(|(_, bounds)| bounds.height).sum();
        lines + self.gap * (self.lines.len().saturating_sub(1)) as f32
    }

    pub(crate) fn extent(&self, _length: f32) -> PartExtent {
        PartExtent {
            thickness: self.block() + self.offset,
            overhang: [0.0, 0.0],
            holds: RangeInclusive::new(0.0, f32::INFINITY),
        }
    }

    pub(crate) fn origin(&self, slot: [f32; 4]) -> [f32; 2] {
        self.side.slot_origin(slot)
    }

    /// The title's lines in the frame of its slab at `length`: along from the slab's start, and
    /// across from its inner edge, outward.
    pub(crate) fn part(&self, length: f32) -> SceneGroup {
        let outward = match self.side {
            GuideSide::Top | GuideSide::Left => -1.0,
            GuideSide::Bottom | GuideSide::Right => 1.0,
        };
        let thickness = self.block() + self.offset;
        // The first line stands at the outer edge, except at the bottom, where the title reads
        // down from the plot as a caption does.
        let from_outer = self.side != GuideSide::Bottom;
        let mut marks = Vec::new();
        let mut used = 0.0;
        for (mark, bounds) in &self.lines {
            let middle = if from_outer {
                thickness - used - bounds.height / 2.0
            } else {
                self.offset + used + bounds.height / 2.0
            };
            used += bounds.height + self.gap;
            let across = outward * middle;
            let (along, align) = match (self.side, self.align) {
                (GuideSide::Left, TitleAlign::Start) => (length, TextAlign::Left),
                (GuideSide::Left, TitleAlign::End) => (0.0, TextAlign::Right),
                (_, TitleAlign::Start) => (0.0, TextAlign::Left),
                (_, TitleAlign::End) => (length, TextAlign::Right),
                (_, TitleAlign::Center) => (length / 2.0, TextAlign::Center),
            };
            let (x, y, angle) = match self.side {
                GuideSide::Top | GuideSide::Bottom => (along, across, 0.0),
                GuideSide::Left => (across, along, -90.0),
                GuideSide::Right => (across, along, 90.0),
            };
            let mark = SceneTextMark {
                x: ScalarOrArray::new_scalar(x),
                y: ScalarOrArray::new_scalar(y),
                align: ScalarOrArray::new_scalar(align),
                baseline: ScalarOrArray::new_scalar(TextBaseline::Middle),
                angle: ScalarOrArray::new_scalar(angle),
                ..mark.clone()
            };
            marks.push(mark.into());
        }
        SceneGroup {
            marks,
            ..Default::default()
        }
    }
}

/// The parts of an axis by the order of their slabs from the plot.
pub(crate) const AXIS_PARTS: [AxisPart; 3] = [AxisPart::Ticks, AxisPart::Labels, AxisPart::Title];
