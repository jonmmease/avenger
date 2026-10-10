use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, AsArray, Float64Array},
    compute::cast,
    datatypes::{DataType, Float32Type},
};
use avenger_color::ColorOrGradient;
use avenger_common::types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode};
use avenger_common::value::ScalarOrArray;
use avenger_format::{FormattedNumber, NumberTypesetting, PreparedFormatter, TickSpacing};
use avenger_geometry::{marks::TextGeometryUtils, rtree::EnvelopeUtils};
use avenger_scales::{
    array::tick_array,
    scales::{time, ConfiguredScale},
};
use avenger_scenegraph::marks::{
    group::SceneGroup,
    rule::SceneRuleMark,
    text::{text_label, text_style, SceneTextMark},
};
use avenger_typst_label::{escape_text, LabelEngine, LabelOptions};
use rstar::AABB;

use super::{
    opts::{AxisConfig, AxisOrientation, AxisTickInterval},
    tick_labels, tick_numbers,
};
use crate::error::AvengerGuidesError;

const TEXT_MARGIN: f32 = 3.0;
const TITLE_MARGIN: f32 = 2.0;
const PIXEL_OFFSET: f32 = 0.5;

const DEFAULT_TICK_LENGTH: f32 = 5.0;
const DEFAULT_TITLE_FONT_SIZE: f32 = 12.0;
const DEFAULT_TICK_FONT_SIZE: f32 = 12.0;
const DEFAULT_MAX_TICK_COUNT: f32 = 10.0;
const HORIZONTAL_MIN_TICK_SPACING_PX: f32 = 25.0;
const VERTICAL_TICK_SPACING_FONT_HEIGHT_FACTOR: f32 = 1.6;
const MAX_START_STEP_TICKS: usize = 10_000;

/// Axis marks for a scale with a numeric or temporal domain, such as a linear, log, or time
/// scale, with tick labels from `config.format`.
pub fn make_continuous_axis_marks(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
    text_engine: &LabelEngine,
) -> Result<SceneGroup, AvengerGuidesError> {
    let config = &resolve_tick_count(config, text_engine);
    // For scales with a band option, make sure ticks end up centered in the band.
    // Other scales reject unknown options.
    let has_band = scale
        .scale_impl
        .option_definitions()
        .iter()
        .any(|option| option.name == "band");
    let scale = if has_band {
        scale.clone().with_option("band", 0.5)
    } else {
        scale.clone()
    };

    // Build main group with origin [0, 0] so we can work in local coordinate.
    // Then set final original at the end
    let mut main_group = SceneGroup {
        origin: [0.0, 0.0],
        ..Default::default()
    };

    let (ticks, label_ticks) = axis_ticks(&scale, config)?;

    // Get range bounds considering orientation
    let range = scale.numeric_interval_range()?;

    let (start, end) = match config.orientation {
        AxisOrientation::Left | AxisOrientation::Right => {
            let upper = f32::min(range.1, range.0) - PIXEL_OFFSET;
            let lower = f32::max(range.0, range.1) + PIXEL_OFFSET;
            (lower, upper)
        }
        AxisOrientation::Top | AxisOrientation::Bottom => {
            let left = f32::min(range.0, range.1) - PIXEL_OFFSET;
            let right = f32::max(range.0, range.1) + PIXEL_OFFSET;
            (left, right)
        }
    };

    // Add axis line
    let is_vertical = matches!(
        config.orientation,
        AxisOrientation::Left | AxisOrientation::Right
    );

    // Calculate axis position based on orientation and plot dimensions
    let offset = match config.orientation {
        AxisOrientation::Left => 0.0,
        AxisOrientation::Right => config.dimensions[0],
        AxisOrientation::Top => 0.0,
        AxisOrientation::Bottom => config.dimensions[1],
    };

    // Add tick grid if enabled
    if config.grid {
        let grid_group = make_tick_grid_marks(
            &ticks,
            &scale,
            &config.orientation,
            &config.dimensions,
            config.style.grid_color,
            config.style.grid_width,
        )?;
        main_group.marks.push(grid_group.into());
    }

    // Create a group for axis elements that should render above data marks
    let mut axis_elements_group = SceneGroup {
        origin: [0.0, 0.0],
        zindex: Some(1), // Axis elements above data marks
        ..Default::default()
    };

    // Add axis line
    axis_elements_group
        .marks
        .push(make_axis_line(start, end, is_vertical, offset, config.style.domain_color).into());

    // Add tick marks
    axis_elements_group.marks.push(
        make_tick_marks(
            &ticks,
            &scale,
            &config.orientation,
            &config.dimensions,
            config.style.tick_length,
            config.style.tick_color,
        )?
        .into(),
    );

    // Add tick labels (if visible)
    if config.style.labels_visible.unwrap_or(true) {
        axis_elements_group
            .marks
            .push(make_tick_labels(&label_ticks, &scale, config)?.into());
    }

    // Add title if visible and non-empty
    if config.style.title_visible.unwrap_or(true) && !title.is_empty() {
        axis_elements_group.marks.push(
            make_title(
                title,
                &scale,
                &axis_elements_group.bounding_box(text_engine),
                config,
                text_engine,
            )?
            .into(),
        );
    }

    // Add the axis elements group to the main group
    main_group.marks.push(axis_elements_group.into());

    // Measure the overall bounds to create a clip rect
    let bbox = main_group.bounding_box(text_engine);

    // Add clip rect to define bounds
    // Use the actual bounding box coordinates, not assuming 0,0
    let padding = 2.0;
    main_group.clip = avenger_scenegraph::marks::group::Clip::Rect {
        x: bbox.lower()[0] - padding,
        y: bbox.lower()[1] - padding,
        width: bbox.width() + 2.0 * padding,
        height: bbox.height() + 2.0 * padding,
    };

    // Now set the actual origin
    main_group.origin = origin;

    Ok(main_group)
}

/// The configuration with its explicit tick count, or one adapted to the axis length.
fn resolve_tick_count(config: &AxisConfig, text_engine: &LabelEngine) -> AxisConfig {
    let mut resolved = config.clone();
    resolved.style.tick_count = config
        .style
        .tick_count
        .or_else(|| adaptive_tick_count(config, text_engine));
    resolved
}

/// The ticks an axis draws, and the subset it labels.
fn axis_ticks(
    scale: &ConfiguredScale,
    config: &AxisConfig,
) -> Result<(ArrayRef, ArrayRef), AvengerGuidesError> {
    if let Some(tick_spacing) = config.style.tick_start_step {
        let ticks = match tick_spacing {
            AxisTickInterval::Numeric { start, step } => start_step_ticks(scale, start, step)?,
            AxisTickInterval::Temporal {
                start_millis,
                months,
                days,
                nanos,
            } => scale.temporal_start_step_ticks(start_millis, months, days, nanos)?,
        };
        return Ok((ticks.clone(), ticks));
    }
    let tick_count = config.style.tick_count;
    let ticks = scale.ticks(tick_count)?;
    let label_ticks = log_label_ticks(&ticks, scale, tick_count.unwrap_or(DEFAULT_MAX_TICK_COUNT));
    Ok((ticks, label_ticks))
}

/// The labels an axis with this configuration shows, in tick order, before they are placed.
pub(crate) fn axis_tick_labels(
    scale: &ConfiguredScale,
    config: &AxisConfig,
    text_engine: &LabelEngine,
) -> Result<(Vec<String>, TextSyntaxMode), AvengerGuidesError> {
    let config = resolve_tick_count(config, text_engine);
    let (_, ticks) = axis_ticks(scale, &config)?;
    let labels = make_tick_label_text(&ticks, scale, &config)?;
    Ok((labels.text.as_vec(ticks.len(), None), labels.syntax_mode))
}

/// Log scales generate minor ticks for visual context. Label only the subset
/// that fits the requested density, following the D3 log tick-format policy.
fn log_label_ticks(ticks: &ArrayRef, scale: &ConfiguredScale, count: f32) -> ArrayRef {
    if scale.scale_impl.scale_type() != "log" {
        return ticks.clone();
    }
    let Some(values) = ticks.as_any().downcast_ref::<Float64Array>() else {
        return ticks.clone();
    };
    let base = f64::from(scale.option_f32("base", 10.0));
    if !base.is_finite() || base <= 0.0 || base == 1.0 || values.is_empty() {
        return ticks.clone();
    }
    let threshold = (base * f64::from(count) / values.len() as f64).max(1.0);
    Arc::new(Float64Array::from_iter_values(
        values.values().iter().copied().filter(|value| {
            let power = base.powf(value.log(base).round());
            let mut coefficient = value / power;
            if coefficient * base < base - 0.5 {
                coefficient *= base;
            }
            coefficient <= threshold
        }),
    ))
}

fn start_step_ticks(
    scale: &ConfiguredScale,
    start: f32,
    step: f32,
) -> Result<ArrayRef, AvengerGuidesError> {
    if !start.is_finite() {
        return Err(AvengerGuidesError::InvalidAxisTicks(
            "tick start must be finite".to_string(),
        ));
    }
    if !step.is_finite() || step <= 0.0 {
        return Err(AvengerGuidesError::InvalidAxisTicks(
            "tick step must be finite and greater than zero".to_string(),
        ));
    }

    let domain = scale.normalized_domain()?;
    if domain.len() != 2 {
        return Err(AvengerGuidesError::InvalidAxisTicks(
            "start-step ticks require a two-value numeric scale domain".to_string(),
        ));
    }
    let domain = cast(domain.as_ref(), &DataType::Float32)
        .map_err(|err| AvengerGuidesError::InvalidAxisTicks(err.to_string()))?;
    let domain = domain.as_primitive::<Float32Type>();
    let domain_start = domain.value(0);
    let domain_end = domain.value(1);
    let lo = domain_start.min(domain_end);
    let hi = domain_start.max(domain_end);

    if !lo.is_finite() || !hi.is_finite() {
        return Err(AvengerGuidesError::InvalidAxisTicks(
            "scale domain must be finite for start-step ticks".to_string(),
        ));
    }

    let first_index = ((lo - start) / step).ceil();
    if !first_index.is_finite() {
        return Err(AvengerGuidesError::InvalidAxisTicks(
            "failed to compute first tick index".to_string(),
        ));
    }

    let epsilon = (step.abs() * 1e-4).max(f32::EPSILON);
    let mut value = start + first_index * step;
    let mut ticks = Vec::new();
    while value <= hi + epsilon {
        if value >= lo - epsilon {
            if ticks.len() >= MAX_START_STEP_TICKS {
                return Err(AvengerGuidesError::InvalidAxisTicks(format!(
                    "start-step ticks would generate more than {MAX_START_STEP_TICKS} ticks"
                )));
            }
            ticks.push(if value.abs() <= epsilon { 0.0 } else { value });
        }
        value += step;
    }

    Ok(tick_array(ticks))
}

fn adaptive_tick_count(config: &AxisConfig, text_engine: &LabelEngine) -> Option<f32> {
    // Estimate reasonable tick count from available axis length.
    // For vertical axes (Left/Right), use height; for horizontal (Top/Bottom), use width.
    let axis_length_px = match config.orientation {
        AxisOrientation::Left | AxisOrientation::Right => config.dimensions[1],
        AxisOrientation::Top | AxisOrientation::Bottom => config.dimensions[0],
    };

    let min_tick_spacing = match config.orientation {
        AxisOrientation::Left | AxisOrientation::Right => {
            vertical_min_tick_spacing_px(config, text_engine)
        }
        AxisOrientation::Top | AxisOrientation::Bottom => HORIZONTAL_MIN_TICK_SPACING_PX,
    };

    let adaptive_count = (axis_length_px / min_tick_spacing).max(2.0);

    // Only override the default scale tick count if the axis is small enough to need fewer ticks.
    if adaptive_count < DEFAULT_MAX_TICK_COUNT {
        Some(adaptive_count)
    } else {
        None
    }
}

fn vertical_min_tick_spacing_px(config: &AxisConfig, text_engine: &LabelEngine) -> f32 {
    let font_size = config
        .style
        .label_font_size
        .unwrap_or(DEFAULT_TICK_FONT_SIZE);
    let font_weight = FontWeight::from(config.style.label_font_weight.unwrap_or(400.0));
    let font_family = config
        .style
        .label_font_family
        .as_deref()
        .unwrap_or("sans-serif");
    let font_height = text_engine
        .font_metrics(&text_style(
            font_family,
            font_size,
            font_weight,
            FontStyle::Normal,
            [0.0, 0.0, 0.0, 1.0],
        ))
        .map_or(font_size, |metrics| metrics.ascent + metrics.descent);

    (font_height * VERTICAL_TICK_SPACING_FONT_HEIGHT_FACTOR).max(1.0)
}

fn make_axis_line(
    start: f32,
    end: f32,
    is_vertical: bool,
    offset: f32,
    color: Option<[f32; 4]>,
) -> SceneRuleMark {
    let (x0, x1, y0, y1) = if is_vertical {
        (offset, offset, start, end)
    } else {
        (start, end, offset, offset)
    };

    SceneRuleMark {
        x: x0.into(),
        x2: x1.into(),
        y: y0.into(),
        y2: y1.into(),
        stroke: ColorOrGradient::Color(color.unwrap_or([0.0, 0.0, 0.0, 1.0])).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    }
}

fn make_tick_marks(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
    tick_length: Option<f32>,
    color: Option<[f32; 4]>,
) -> Result<SceneRuleMark, AvengerGuidesError> {
    let scaled_values = scale.scale_to_numeric(ticks)?;
    let tick_len = tick_length.unwrap_or(DEFAULT_TICK_LENGTH);

    let (x0, x1, y0, y1) = match orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(-tick_len),
            scaled_values.clone(),
            scaled_values.clone(),
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(dimensions[0]),
            ScalarOrArray::new_scalar(dimensions[0] + tick_len),
            scaled_values.clone(),
            scaled_values.clone(),
        ),
        AxisOrientation::Top => (
            scaled_values.clone(),
            scaled_values.clone(),
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(-tick_len),
        ),
        AxisOrientation::Bottom => (
            scaled_values.clone(),
            scaled_values.clone(),
            ScalarOrArray::new_scalar(dimensions[1]),
            ScalarOrArray::new_scalar(dimensions[1] + tick_len),
        ),
    };

    Ok(SceneRuleMark {
        len: ticks.len() as u32,
        clip: false,
        x: x0,
        x2: x1,
        y: y0,
        y2: y1,
        stroke: ColorOrGradient::Color(color.unwrap_or([0.0, 0.0, 0.0, 1.0])).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    })
}

/// Construct grid lines separately from foreground axis marks.
pub fn make_tick_grid_marks(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
    color: Option<[f32; 4]>,
    width: Option<f32>,
) -> Result<SceneGroup, AvengerGuidesError> {
    let scaled_values = scale.scale_to_numeric(ticks)?;

    let (x0, x1, y0, y1) = match orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(dimensions[0]),
            scaled_values.clone(),
            scaled_values.clone(),
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(dimensions[0]),
            scaled_values.clone(),
            scaled_values.clone(),
        ),
        AxisOrientation::Top => (
            scaled_values.clone(),
            scaled_values.clone(),
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(dimensions[1]),
        ),
        AxisOrientation::Bottom => (
            scaled_values.clone(),
            scaled_values.clone(),
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(dimensions[1]),
        ),
    };

    let grid_mark = SceneRuleMark {
        interactive: false,
        len: ticks.len() as u32,
        clip: false,
        x: x0,
        x2: x1,
        y: y0,
        y2: y1,
        stroke: ColorOrGradient::Color(color.unwrap_or([0.878, 0.878, 0.878, 0.5])).into(), // Default: #E0E0E0 with opacity 0.5
        stroke_width: width.unwrap_or(0.5).into(),
        ..Default::default()
    };

    // Grid lines are positioned relative to the axis group origin
    let grid_origin = [0.0, 0.0];

    Ok(SceneGroup {
        interactive: false,
        origin: grid_origin,
        zindex: Some(-1), // Grid lines behind data marks
        marks: vec![grid_mark.into()],
        ..Default::default()
    })
}

fn make_tick_labels(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    config: &AxisConfig,
) -> Result<SceneTextMark, AvengerGuidesError> {
    let tick_labels = make_tick_label_text(ticks, scale, config)?;
    let scaled_values = scale.scale_to_numeric(ticks)?;

    // Adjust y position slightly for font metrics
    // Numbers don't use full descent, so shift up by ~10% of font size for better visual centering
    let tick_font_size = config
        .style
        .label_font_size
        .unwrap_or(DEFAULT_TICK_FONT_SIZE);
    let label_angle = config.style.label_angle.unwrap_or(0.0);
    let tick_length = config.style.tick_length.unwrap_or(DEFAULT_TICK_LENGTH);
    let font_adjustment = tick_font_size * 0.10;
    let adjusted_values_left_right = scaled_values
        .as_vec(ticks.len(), None)
        .into_iter()
        .map(|v| v - font_adjustment)
        .collect::<Vec<_>>();

    let (x, y, align, baseline, angle) = match config.orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(-tick_length - TEXT_MARGIN),
            ScalarOrArray::new_array(adjusted_values_left_right.clone()),
            TextAlign::Right,
            TextBaseline::Middle,
            label_angle,
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(config.dimensions[0] + tick_length + TEXT_MARGIN),
            ScalarOrArray::new_array(adjusted_values_left_right),
            TextAlign::Left,
            TextBaseline::Middle,
            label_angle,
        ),
        AxisOrientation::Top => (
            scaled_values,
            ScalarOrArray::new_scalar(-tick_length),
            TextAlign::Center,
            TextBaseline::Bottom,
            label_angle,
        ),
        AxisOrientation::Bottom => (
            scaled_values,
            ScalarOrArray::new_scalar(config.dimensions[1] + tick_length + TEXT_MARGIN),
            TextAlign::Center,
            TextBaseline::Top,
            label_angle,
        ),
    };

    Ok(SceneTextMark {
        clip: false,
        len: ticks.len() as u32,
        text: tick_labels.text,
        x,
        y,
        align: align.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        color: ColorOrGradient::Color(config.style.label_color.unwrap_or([0.0, 0.0, 0.0, 1.0]))
            .into(), // Default: black
        font_size: tick_font_size.into(),
        font_weight: FontWeight::from(config.style.label_font_weight.unwrap_or(400.0)).into(), // Default: normal weight for tick labels
        font: config
            .style
            .label_font_family
            .clone()
            .unwrap_or_else(|| "sans-serif".to_string())
            .into(),
        text_syntax: tick_labels.syntax_mode,
        ..Default::default()
    })
}

struct TickLabelText {
    text: ScalarOrArray<String>,
    syntax_mode: TextSyntaxMode,
}

/// Tick labels from the axis formatter. Numeric labels with an exponent are typeset as Typst
/// math, and the axis's label template, if any, wraps each label.
fn make_tick_label_text(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    config: &AxisConfig,
) -> Result<TickLabelText, AvengerGuidesError> {
    check_timezone(ticks, scale, &config.format)?;
    let spacing = tick_spacing(scale);
    let template = config.style.tick_label.as_deref();
    if let (PreparedFormatter::Number(format), true) =
        (&config.format, ticks.data_type().is_numeric())
    {
        let labels = format.format_ticks(&tick_numbers(ticks)?, spacing);
        let plain = labels
            .iter()
            .all(|label| matches!(label.typesetting, NumberTypesetting::Plain));
        if template.is_none() && plain {
            return Ok(TickLabelText {
                text: ScalarOrArray::new_array(
                    labels.into_iter().map(|label| label.text).collect(),
                ),
                syntax_mode: TextSyntaxMode::Plain,
            });
        }
        return Ok(typst_labels(
            labels.iter().map(formatted_number_to_typst),
            template,
        ));
    }
    let labels = tick_labels(ticks, &config.format, spacing)?;
    Ok(match template {
        Some(template) => typst_labels(
            labels
                .as_vec(ticks.len(), None)
                .iter()
                .map(|label| escape_text(label)),
            Some(template),
        ),
        None => TickLabelText {
            text: labels,
            syntax_mode: TextSyntaxMode::Plain,
        },
    })
}

/// Typst labels, each wrapped in the template when there is one.
fn typst_labels(labels: impl Iterator<Item = String>, template: Option<&str>) -> TickLabelText {
    TickLabelText {
        text: ScalarOrArray::new_array(
            labels
                .map(|label| match template {
                    Some(template) => fill_label_template(template, &label),
                    None => label,
                })
                .collect(),
        ),
        syntax_mode: TextSyntaxMode::TypstMarkup,
    }
}

/// Zoned labels must use the timezone that the scale places ticks in.
fn check_timezone(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    format: &PreparedFormatter,
) -> Result<(), AvengerGuidesError> {
    if let (DataType::Timestamp(_, Some(_)), PreparedFormatter::ZonedDateTime(zoned)) =
        (ticks.data_type(), format)
    {
        let scale_timezone = time::timezone(&scale.config)?;
        if zoned.timezone() != scale_timezone {
            return Err(AvengerGuidesError::TimezoneMismatch {
                scale: scale_timezone,
                formatter: zoned.timezone(),
            });
        }
    }
    Ok(())
}

/// Log, threshold, and quantile ticks span magnitudes or arbitrary breaks. Other numeric
/// scales space their ticks evenly.
fn tick_spacing(scale: &ConfiguredScale) -> TickSpacing {
    match scale.scale_impl.scale_type() {
        "log" | "threshold" | "quantile" => TickSpacing::Varying,
        _ => TickSpacing::Uniform,
    }
}

/// Replace each `#label` in a template with a tick's Typst label.
fn fill_label_template(template: &str, label: &str) -> String {
    let mut output = String::with_capacity(template.len() + label.len());
    let mut rest = template;
    while let Some(index) = rest.find("#label") {
        output.push_str(&rest[..index]);
        let after = &rest[index + "#label".len()..];
        if after.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '-') {
            output.push_str("#label");
        } else {
            output.push_str(label);
        }
        rest = after;
    }
    output.push_str(rest);
    output
}

/// A formatted number as Typst markup. Scientific notation is math, as `#numfmt` sets it: the
/// mantissa is a string, which math sets as text, so that a localized mantissa such as "1,2"
/// keeps its punctuation, and its sign is a minus.
fn formatted_number_to_typst(formatted: &FormattedNumber) -> String {
    match &formatted.typesetting {
        NumberTypesetting::Plain => escape_text(&formatted.text),
        NumberTypesetting::Exponent { mantissa, exponent } => {
            let (sign, digits) = match mantissa.strip_prefix(['-', '\u{2212}']) {
                Some(digits) => ("-", digits),
                None => ("", mantissa.as_str()),
            };
            let digits = digits.replace('\\', "\\\\").replace('"', "\\\"");
            format!("${sign}#\"{digits}\" times 10^({exponent})$")
        }
    }
}

fn make_title(
    title: &str,
    scale: &ConfiguredScale,
    envelope: &AABB<[f32; 2]>,
    config: &AxisConfig,
    text_engine: &avenger_typst_label::LabelEngine,
) -> Result<SceneTextMark, AvengerGuidesError> {
    let range = scale.numeric_interval_range()?;
    let mid = (range.0 + range.1) / 2.0;
    let title_font_size = config
        .style
        .title_font_size
        .unwrap_or(DEFAULT_TITLE_FONT_SIZE);
    let title_font_weight = FontWeight::from(config.style.title_font_weight.unwrap_or(400.0));
    let title_font_family = config
        .style
        .title_font_family
        .clone()
        .unwrap_or_else(|| "sans-serif".to_string());
    text_engine.bounds(&text_label(
        title,
        config.style.title_syntax_mode,
        LabelOptions {
            text: text_style(
                &title_font_family,
                title_font_size,
                title_font_weight,
                FontStyle::Normal,
                [0.0, 0.0, 0.0, 1.0],
            ),
            ..Default::default()
        },
    ))?;

    // Now the envelope is in the group's local coordinate system (origin = [0, 0])
    // For left/top axes, labels extend into negative coordinates
    // For right/bottom axes, labels extend beyond the axis dimensions

    let (x, y, align, baseline, angle) = match config.orientation {
        AxisOrientation::Left => {
            // Labels are right-aligned and extend left from the axis
            // envelope.lower()[0] gives us the leftmost edge of the labels
            (
                (envelope.lower()[0] - TITLE_MARGIN).into(),
                mid.into(),
                TextAlign::Center,
                TextBaseline::LineBottom,
                -90.0,
            )
        }
        AxisOrientation::Right => {
            // Labels are left-aligned and extend right from the axis
            // envelope.upper()[0] gives us the rightmost edge of the labels
            (
                (envelope.upper()[0] + TITLE_MARGIN).into(),
                mid.into(),
                TextAlign::Center,
                TextBaseline::LineBottom,
                90.0,
            )
        }
        AxisOrientation::Top => {
            // Labels are bottom-aligned and extend up from the axis
            // envelope.lower()[1] gives us the topmost edge of the labels
            (
                mid.into(),
                (envelope.lower()[1] - TITLE_MARGIN).into(),
                TextAlign::Center,
                TextBaseline::Bottom,
                0.0,
            )
        }
        AxisOrientation::Bottom => {
            // Labels are top-aligned and extend down from the axis
            // envelope.upper()[1] gives us the bottommost edge of the labels
            (
                mid.into(),
                (envelope.upper()[1] + TITLE_MARGIN).into(),
                TextAlign::Center,
                TextBaseline::Top,
                0.0,
            )
        }
    };

    Ok(SceneTextMark {
        clip: false,
        len: 1,
        text: title.to_string().into(),
        x,
        y,
        align: align.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        color: ColorOrGradient::Color(config.style.title_color.unwrap_or([0.0, 0.0, 0.0, 1.0]))
            .into(), // Default: black
        font_size: title_font_size.into(),
        font_weight: title_font_weight.into(),
        font: title_font_family.into(),
        text_syntax: config.style.title_syntax_mode,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::super::opts::AxisStyle;
    use super::*;
    use arrow::array::{Date32Array, Float64Array};
    use arrow::datatypes::Float64Type;
    use avenger_format::{DateTimeFormatProvider, NumberFormatProvider};
    use avenger_format_datetime_d3::D3DateTimeFormatProvider;
    use avenger_format_number_d3::D3NumberFormatProvider;
    use avenger_geometry::rtree::SceneGraphRTree;
    use avenger_scales::scales::linear::LinearScale;
    use avenger_scales::scales::log::LogScale;
    use avenger_scales::scales::time::TimeScale;
    use avenger_scenegraph::{marks::symbol::SceneSymbolMark, scene_graph::SceneGraph};
    use avenger_typst_label::{bundled_font_options, EngineOptions, FontOptions};

    fn config(pattern: &str, style: AxisStyle) -> AxisConfig {
        AxisConfig {
            orientation: AxisOrientation::Bottom,
            dimensions: [100.0, 100.0],
            grid: false,
            format: D3NumberFormatProvider::new()
                .prepare(pattern)
                .unwrap()
                .into(),
            style,
        }
    }

    fn values(array: &ArrayRef) -> Vec<f64> {
        array.as_primitive::<Float64Type>().values().to_vec()
    }

    #[test]
    fn tick_labels_keep_their_clearance_when_tick_length_changes() {
        let scale = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![0.0, 5.0, 10.0])) as ArrayRef;
        for orientation in [
            AxisOrientation::Left,
            AxisOrientation::Right,
            AxisOrientation::Top,
            AxisOrientation::Bottom,
        ] {
            for tick_length in [None, Some(25.0)] {
                let config = AxisConfig {
                    orientation,
                    ..config(
                        ",",
                        AxisStyle {
                            tick_length,
                            ..Default::default()
                        },
                    )
                };
                let marks = make_tick_marks(
                    &ticks,
                    &scale,
                    &orientation,
                    &config.dimensions,
                    tick_length,
                    None,
                )
                .unwrap();
                let labels = make_tick_labels(&ticks, &scale, &config).unwrap();
                let (clearance, expected) = match orientation {
                    AxisOrientation::Left => (
                        marks.x2.as_vec(3, None)[0] - labels.x.as_vec(3, None)[0],
                        TEXT_MARGIN,
                    ),
                    AxisOrientation::Right => (
                        labels.x.as_vec(3, None)[0] - marks.x2.as_vec(3, None)[0],
                        TEXT_MARGIN,
                    ),
                    AxisOrientation::Top => (
                        marks.y2.as_vec(3, None)[0] - labels.y.as_vec(3, None)[0],
                        0.0,
                    ),
                    AxisOrientation::Bottom => (
                        labels.y.as_vec(3, None)[0] - marks.y2.as_vec(3, None)[0],
                        TEXT_MARGIN,
                    ),
                };
                assert_eq!(clearance, expected, "{orientation:?}, {tick_length:?}");
            }
        }
    }

    #[test]
    fn start_step_ticks_clip_to_domain() {
        let scale = LinearScale::configured((1.2, 8.8), (0.0, 100.0));
        let ticks = start_step_ticks(&scale, 0.0, 2.0).expect("ticks");
        assert_eq!(values(&ticks), vec![2.0, 4.0, 6.0, 8.0]);
    }

    #[test]
    fn start_step_ticks_support_reversed_domain() {
        let scale = LinearScale::configured((8.8, 1.2), (0.0, 100.0));
        let ticks = start_step_ticks(&scale, 0.0, 2.0).expect("ticks");
        assert_eq!(values(&ticks), vec![2.0, 4.0, 6.0, 8.0]);
    }

    #[test]
    fn start_step_ticks_reject_invalid_step() {
        let scale = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
        let err = start_step_ticks(&scale, 0.0, 0.0).expect_err("invalid step");
        assert!(matches!(err, AvengerGuidesError::InvalidAxisTicks(_)));
    }

    #[test]
    fn start_step_ticks_are_labeled_as_a_set() {
        let scale = LinearScale::configured((0.0, 0.035), (0.0, 100.0));
        let ticks = start_step_ticks(&scale, 0.0, 0.01).expect("ticks");
        assert_eq!(values(&ticks), vec![0.0, 0.01, 0.02, 0.03]);
        let labels =
            make_tick_label_text(&ticks, &scale, &config(",", Default::default())).unwrap();
        assert_eq!(labels.text.as_vec(4, None), ["0", "0.01", "0.02", "0.03"]);
    }

    #[test]
    fn temporal_start_step_ticks_stay_civil() {
        let start = Arc::new(Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(Date32Array::from(vec![19800])) as ArrayRef;
        let scale = TimeScale::configured((start, end), (0.0, 100.0))
            .with_option("timezone", "America/New_York");
        let ticks = scale
            .temporal_start_step_ticks(1_704_067_200_000, 1, 0, 0)
            .expect("ticks");
        let format = D3DateTimeFormatProvider::new()
            .prepare_date("%b %d")
            .unwrap();
        let config = AxisConfig {
            format: format.into(),
            ..config(",", Default::default())
        };
        let labels = make_tick_label_text(&ticks, &scale, &config).unwrap();
        assert_eq!(labels.text.as_vec(3, None), ["Jan 01", "Feb 01", "Mar 01"]);
    }

    #[test]
    fn log_tick_labels_suppress_dense_minor_values() {
        let scale = LogScale::configured((0.1, 100.0), (0.0, 100.0)).with_option("base", 10.0);
        let ticks = scale.ticks(Some(10.0)).expect("ticks");
        let labels = log_label_ticks(&ticks, &scale, 10.0);

        assert_eq!(
            values(&labels),
            vec![0.1, 0.2, 0.3, 1.0, 2.0, 3.0, 10.0, 20.0, 30.0, 100.0]
        );
    }

    #[test]
    fn continuous_axis_tick_labels_honor_configured_angle() {
        let scale = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![0.0, 5.0, 10.0])) as ArrayRef;
        let style = AxisStyle {
            label_angle: Some(-45.0),
            ..Default::default()
        };
        let labels = make_tick_labels(&ticks, &scale, &config(",", style)).expect("tick labels");

        assert_eq!(
            labels.angle.as_vec(labels.len as usize, None),
            vec![-45.0, -45.0, -45.0]
        );
    }

    #[test]
    fn exponent_labels_use_typst_math() {
        let scale = LinearScale::configured((0.0, 2000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1200.0])) as ArrayRef;
        let labels = make_tick_label_text(&ticks, &scale, &config(".1e", Default::default()))
            .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(
            labels.text.as_vec(1, None),
            vec![r#"$#"1.2" times 10^(3)$"#]
        );
    }

    #[test]
    fn exponent_mantissas_are_text() {
        let label = |mantissa: &str, exponent| {
            formatted_number_to_typst(&FormattedNumber {
                text: String::new(),
                typesetting: NumberTypesetting::Exponent {
                    mantissa: mantissa.into(),
                    exponent,
                },
            })
        };
        assert_eq!(label("1,2", 3), r#"$#"1,2" times 10^(3)$"#);
        assert_eq!(label("\u{2212}1,2", -3), r#"$-#"1,2" times 10^(-3)$"#);

        // A comma in the mantissa is text, not math punctuation.
        let engine = LabelEngine::new(EngineOptions {
            fonts: FontOptions {
                load_system_fonts: false,
                ..bundled_font_options()
            },
        });
        let measure = |text: &str| {
            engine
                .bounds(&text_label(
                    text,
                    TextSyntaxMode::TypstMarkup,
                    LabelOptions::default(),
                ))
                .unwrap()
                .width
        };
        assert!(measure(&label("1,2", 3)) < measure("$1,2 times 10^(3)$"));
    }

    #[test]
    fn templates_wrap_formatted_labels() {
        let scale = LinearScale::configured((0.0, 2000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1000.0, 2000.0])) as ArrayRef;
        let style = AxisStyle {
            tick_label: Some("#label m/s, #labels".into()),
            ..Default::default()
        };
        let labels = make_tick_label_text(&ticks, &scale, &config(",.0f", style)).unwrap();
        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(
            labels.text.as_vec(2, None),
            ["1,000 m/s, #labels", "2,000 m/s, #labels"]
        );

        // Temporal labels are escaped as Typst text.
        let start = Arc::new(Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(Date32Array::from(vec![19754])) as ArrayRef;
        let scale = TimeScale::configured((start.clone(), end), (0.0, 100.0));
        let format = D3DateTimeFormatProvider::new()
            .prepare_date("#%d*")
            .unwrap();
        let config = AxisConfig {
            format: format.into(),
            ..config(
                ",",
                AxisStyle {
                    tick_label: Some("[#label]".into()),
                    ..Default::default()
                },
            )
        };
        let labels = make_tick_label_text(&start, &scale, &config).unwrap();
        assert_eq!(labels.text.as_vec(1, None), [r"[\#01\*]"]);
    }

    #[test]
    fn grid_behind_a_symbol_cannot_replace_it_as_the_top_hit() {
        let scale = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![5.0])) as ArrayRef;
        let grid = make_tick_grid_marks(
            &ticks,
            &scale,
            &AxisOrientation::Left,
            &[100.0, 100.0],
            None,
            None,
        )
        .expect("numeric grid");
        assert!(!grid.interactive);
        assert!(grid.marks.iter().all(|mark| !mark.interactive()));

        let point = SceneSymbolMark {
            name: "point".into(),
            x: 50.0.into(),
            y: 50.0.into(),
            size: 64.0.into(),
            ..Default::default()
        };
        let scene = SceneGraph {
            // Keep the guide later in document order to reproduce the prior
            // conflict between its scene path and its negative visual z-index.
            marks: vec![point.into(), grid.into()],
            width: 100.0,
            height: 100.0,
            origin: [0.0, 0.0],
        };
        let rtree =
            SceneGraphRTree::from_scene_graph(&scene, &avenger_typst_label::bundled_label_engine());

        assert_eq!(
            rtree
                .pick_top_mark_at_point(&[50.0, 50.0])
                .expect("point at grid intersection")
                .name,
            "point"
        );
    }
}
