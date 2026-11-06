//! The configured scales of the format's scale specs: their types and options, and the ranges of
//! non-position scales from the computed `scheme` or `range` of their `scale` nodes.

use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::{new_empty_array, ArrayRef, Float64Array, ListArray, StringArray},
    datatypes::{DataType, Float64Type},
};
use avenger_chart::{
    proto::{self, option_value, scale_spec},
    FitKind,
};
use avenger_color::ColorSpace;
use avenger_scales::{
    color_interpolator::SpaceColorInterpolator,
    scalar::Scalar,
    scales::{
        band::BandScale, domain_summary::SummaryKind, linear::LinearScale, log::LogScale,
        nested_band::NestedBandScale, ordinal::OrdinalScale, point::PointScale, pow::PowScale,
        quantile::QuantileScale, quantize::QuantizeScale, symlog::SymlogScale,
        threshold::ThresholdScale, time::TimeScale, ConfiguredScale, ScaleConfig, ScaleImpl,
    },
};
use avenger_style::{Computed, ComputedStyle, ComputedStyles, Property};

use crate::{EvalError, Registry};

type Result<T> = std::result::Result<T, EvalError>;

/// The kind of fit of a spec's scale type, as its implementation states it: a scale of
/// avenger-scales, or a custom scale of the registry, which fits the kind of its summary.
pub(crate) fn fit_kind(spec: &proto::ScaleSpec, registry: &Registry) -> Result<FitKind> {
    Ok(match scale_impl(spec, registry)?.summary_kind() {
        Some(SummaryKind::Interval) => FitKind::Interval,
        Some(SummaryKind::Categories) => FitKind::Categories,
        Some(SummaryKind::Sample) => FitKind::Sample,
        None => FitKind::Explicit,
    })
}

/// The implementation of a spec's scale type: a scale of avenger-scales, or a custom scale of the
/// registry.
pub(crate) fn scale_impl(
    spec: &proto::ScaleSpec,
    registry: &Registry,
) -> Result<Arc<dyn ScaleImpl>> {
    use scale_spec::Type;
    Ok(
        match spec.r#type.as_ref().expect("a valid scale has a type") {
            Type::Linear(_) => Arc::new(LinearScale),
            Type::Log(_) => Arc::new(LogScale),
            Type::Pow(_) => Arc::new(PowScale),
            Type::Symlog(_) => Arc::new(SymlogScale),
            Type::Time(_) => Arc::new(TimeScale),
            Type::Band(_) => Arc::new(BandScale),
            Type::Point(_) => Arc::new(PointScale),
            Type::NestedBand(_) => Arc::new(NestedBandScale),
            Type::Ordinal(_) => Arc::new(OrdinalScale),
            Type::Quantile(_) => Arc::new(QuantileScale),
            Type::Quantize(_) => Arc::new(QuantizeScale),
            Type::Threshold(_) => Arc::new(ThresholdScale),
            Type::Custom(custom) => registry
                .scale(&custom.r#type)
                .cloned()
                .ok_or_else(|| EvalError::UnknownScale(custom.r#type.clone()))?,
        },
    )
}

/// The options of a spec's type as avenger-scales reads them, with the paddings of a band or a
/// point scale from the computed `padding-inner` and `padding-outer` of its `scale` node.
pub(crate) fn options(spec: &proto::ScaleSpec, style: &ComputedStyle) -> HashMap<String, Scalar> {
    use scale_spec::Type;
    let mut options: HashMap<String, Scalar> = HashMap::new();
    let mut set = |name: &str, value: Option<Scalar>| {
        if let Some(value) = value {
            options.insert(name.to_string(), value);
        }
    };
    let number = |value: Option<f64>| value.map(Scalar::from);
    let flag = |value: Option<bool>| value.map(Scalar::from);
    let padding = |property: Property| {
        style
            .number(property)
            .map(|value| Scalar::from(value as f64))
    };
    match spec.r#type.as_ref().expect("a valid scale has a type") {
        Type::Linear(linear) => {
            set("clamp", flag(linear.clamp));
            set("round", flag(linear.round));
        }
        Type::Log(log) => {
            set("base", number(log.base));
            set("clamp", flag(log.clamp));
            set("round", flag(log.round));
        }
        Type::Pow(pow) => {
            set("exponent", number(pow.exponent));
            set("clamp", flag(pow.clamp));
            set("round", flag(pow.round));
        }
        Type::Symlog(symlog) => {
            set("constant", number(symlog.constant));
            set("clamp", flag(symlog.clamp));
            set("round", flag(symlog.round));
        }
        Type::Time(time) => {
            if !time.timezone.is_empty() {
                set("timezone", Some(Scalar::from(time.timezone.as_str())));
            }
        }
        Type::Band(band) => {
            set("align", number(band.align));
            set("padding_inner", padding(Property::PaddingInner));
            set("padding_outer", padding(Property::PaddingOuter));
            set("round", flag(band.round));
            set("clip_padding_lower", number(band.clip_padding_lower));
            set("clip_padding_upper", number(band.clip_padding_upper));
        }
        Type::Point(point) => {
            set("align", number(point.align));
            set("padding", padding(Property::PaddingOuter));
            set("round", flag(point.round));
            set("clip_padding_lower", number(point.clip_padding_lower));
            set("clip_padding_upper", number(point.clip_padding_upper));
        }
        Type::NestedBand(nested) => {
            set("align", number(nested.align));
            if !nested.nest_scopes.is_empty() {
                let scopes: Vec<&str> = (nested.nest_scopes.iter())
                    .map(|scope| match proto::NestScope::try_from(*scope) {
                        Ok(proto::NestScope::Shared) => "shared",
                        _ => "free",
                    })
                    .collect();
                set("nest_scopes", Some(Scalar::from(scopes)));
            }
            // The paddings of a nested band scale are content, one for each level or one for all.
            if !nested.padding_inner.is_empty() {
                set(
                    "padding_inner",
                    Some(Scalar::from(nested.padding_inner.clone())),
                );
            }
            if !nested.padding_outer.is_empty() {
                set(
                    "padding_outer",
                    Some(Scalar::from(nested.padding_outer.clone())),
                );
            }
            set("round", flag(nested.round));
            set("clip_padding_lower", number(nested.clip_padding_lower));
            set("clip_padding_upper", number(nested.clip_padding_upper));
        }
        Type::Ordinal(_) | Type::Quantile(_) | Type::Quantize(_) | Type::Threshold(_) => {}
        Type::Custom(custom) => {
            for option in &custom.options {
                let value = option.value.as_ref().and_then(|value| value.value.as_ref());
                let value = match value {
                    Some(option_value::Value::Boolean(value)) => Scalar::from(*value),
                    Some(option_value::Value::Float(value)) => Scalar::from(*value),
                    Some(option_value::Value::Integer(value)) => Scalar::from(*value as f64),
                    Some(option_value::Value::String(value)) => Scalar::from(value.as_str()),
                    Some(option_value::Value::Floats(list)) => Scalar::from(list.values.clone()),
                    Some(option_value::Value::Strings(list)) => {
                        Scalar::from(list.values.iter().map(String::as_str).collect::<Vec<_>>())
                    }
                    None => continue,
                };
                set(&option.name, Some(value));
            }
        }
    }
    options
}

/// A scale of `spec` without its domain and its range: its type and its options, an empty domain
/// of `value_type`, and the range from 0 to 1.
pub(crate) fn unfitted(
    spec: &proto::ScaleSpec,
    style: &ComputedStyle,
    registry: &Registry,
    value_type: &DataType,
) -> Result<ConfiguredScale> {
    Ok(ConfiguredScale {
        scale_impl: scale_impl(spec, registry)?,
        config: ScaleConfig {
            domain: new_empty_array(value_type),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: options(spec, style),
            context: Default::default(),
        },
    })
}

/// The range of a non-position scale, from the computed styles of its `scale` node.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ScaleRange {
    /// Colors: one for each category or bin, or the stops of a continuous scale, which
    /// interpolates between them in `space`.
    Colors {
        colors: Vec<[f32; 4]>,
        space: Option<ColorSpace>,
    },
    /// Other values, such as sizes, shapes, or dashes, in the order that the styles list them.
    Values(ArrayRef),
}

impl ScaleRange {
    /// `scale` with this range.
    pub(crate) fn apply(&self, scale: ConfiguredScale) -> Result<ConfiguredScale> {
        Ok(match self {
            ScaleRange::Colors { colors, space } => {
                let scale = scale.with_range_colors(colors.clone())?;
                match space {
                    Some(space) => {
                        scale.with_color_interpolator(Arc::new(SpaceColorInterpolator::new(*space)))
                    }
                    None => scale,
                }
            }
            ScaleRange::Values(values) => scale.with_range(values.clone()),
        })
    }
}

/// How many values a non-position scale maps to: one for each category of a discrete scale, a
/// threshold scale's thresholds and one more, and the bins of a quantize or a quantile scale, which
/// the scheme's count gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RangeShape {
    /// The stops of a continuous scale.
    Continuous,
    /// `count` values, for the categories of a discrete scale or the bins of a threshold scale.
    Discrete(usize),
    /// The bins of a quantize or a quantile scale.
    Bins,
}

/// The shape of the range of `spec` over a domain of `domain_len` values.
pub(crate) fn range_shape(spec: &proto::ScaleSpec, domain_len: usize) -> RangeShape {
    use scale_spec::Type;
    match spec.r#type.as_ref().expect("a valid scale has a type") {
        Type::Band(_) | Type::Point(_) | Type::NestedBand(_) | Type::Ordinal(_) => {
            RangeShape::Discrete(domain_len.max(1))
        }
        Type::Threshold(_) => RangeShape::Discrete(domain_len + 1),
        Type::Quantize(_) | Type::Quantile(_) => RangeShape::Bins,
        Type::Linear(_)
        | Type::Log(_)
        | Type::Pow(_)
        | Type::Symlog(_)
        | Type::Time(_)
        | Type::Custom(_) => RangeShape::Continuous,
    }
}

/// The range of a non-position scale of `shape` from the computed styles of its `scale` node:
/// its `range` as the styles list it, or else the colors of its `scheme`. A palette's colors sample
/// for each category of a discrete scale, are the stops of a continuous scale, which interpolates
/// in the palette's space, and fill the bins of a quantize or a quantile scale at even steps, as
/// many as the scheme's count, or as the palette's colors without one. `None` when the styles give
/// neither.
pub(crate) fn range(
    style: &ComputedStyle,
    styles: &ComputedStyles,
    shape: RangeShape,
) -> Option<ScaleRange> {
    if let Computed::List(items) = style.get(Property::Range) {
        return range_values(items);
    }
    let Computed::Sequence(parts) = style.get(Property::Scheme) else {
        return None;
    };
    let name = match parts.first() {
        Some(Computed::String(name)) => name.to_string(),
        _ => return None,
    };
    let count = parts.iter().find_map(|part| match part {
        Computed::Integer(count) => Some(*count as usize),
        _ => None,
    });
    let reverse = parts
        .iter()
        .any(|part| matches!(part, Computed::Keyword("reverse")));
    let palette = styles.palette(&name)?;
    let (mut colors, space) = match shape {
        RangeShape::Continuous => (palette.colors.clone(), Some(palette.space())),
        RangeShape::Discrete(count) => (palette.sample(count), None),
        RangeShape::Bins => {
            let count = count.unwrap_or(palette.colors.len()).max(1);
            let positions: Vec<f32> = (0..count)
                .map(|step| match count {
                    1 => 1.0,
                    _ => step as f32 / (count - 1) as f32,
                })
                .collect();
            (palette.interpolate(&positions), None)
        }
    };
    if reverse {
        colors.reverse();
    }
    Some(ScaleRange::Colors { colors, space })
}

/// The items of a computed `range` as a range: colors, numbers, names such as shapes, or dashes,
/// by the kind of the first item.
fn range_values(items: &[Computed]) -> Option<ScaleRange> {
    let color = |item: &Computed| match item {
        Computed::Color(color) | Computed::CurrentColor { color, .. } => Some(*color),
        _ => None,
    };
    let number = |item: &Computed| match item {
        Computed::Number(value) | Computed::Percentage(value) | Computed::Ratio(value) => {
            Some(*value as f64)
        }
        Computed::Integer(value) => Some(*value as f64),
        _ => None,
    };
    let first = items.first()?;
    if color(first).is_some() {
        let colors = items.iter().filter_map(color).collect();
        return Some(ScaleRange::Colors {
            colors,
            space: None,
        });
    }
    if number(first).is_some() {
        let values: Vec<f64> = items.iter().filter_map(number).collect();
        return Some(ScaleRange::Values(Arc::new(Float64Array::from(values))));
    }
    if matches!(first, Computed::Sequence(_)) {
        let dashes = items.iter().map(|item| match item {
            Computed::Sequence(parts) => Some(parts.iter().map(number).collect::<Vec<_>>()),
            other => Some(vec![number(other)]),
        });
        let list: ListArray = ListArray::from_iter_primitive::<Float64Type, _, _>(dashes);
        return Some(ScaleRange::Values(Arc::new(list) as ArrayRef));
    }
    let names: Vec<String> = items
        .iter()
        .filter_map(|item| match item {
            Computed::String(name) => Some(name.to_string()),
            Computed::Keyword(name) => Some(name.to_string()),
            _ => None,
        })
        .collect();
    Some(ScaleRange::Values(Arc::new(StringArray::from(names))))
}
