//! The styles of a chart: its style tree, whose nodes stand for the parts of the chart, the inline
//! styles that the format's style fields give the nodes, and the choice of each scale's guide.

pub(crate) mod inline;
pub(crate) mod tree;

use avenger_chart::{
    proto::{self, guide, scale_spec, Channel},
    resolve::ScaleGroup,
};
use avenger_style::{ComputedStyle, Property};

/// The computed number of `property`, whose values are numbers, lengths, or angles, so that every
/// node has one.
pub(crate) fn number(style: &ComputedStyle, property: Property) -> f32 {
    (style.number(property)).unwrap_or_else(|| panic!("`{}` computes to a number", property.name()))
}

/// The computed color of `property`, whose values are colors, so that every node has one.
pub(crate) fn color(style: &ComputedStyle, property: Property) -> [f32; 4] {
    (style.color(property)).unwrap_or_else(|| panic!("`{}` computes to a color", property.name()))
}

/// The kind of the guide that a scale draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuideKind {
    Axis,
    Legend,
    Colorbar,
}

/// Whether a scale's type maps a continuous domain: linear, log, pow, symlog, time, and custom
/// scales of a continuous kind.
pub(crate) fn is_continuous(spec: &proto::ScaleSpec) -> bool {
    use scale_spec::Type;
    matches!(
        spec.r#type,
        Some(Type::Linear(_) | Type::Log(_) | Type::Pow(_) | Type::Symlog(_) | Type::Time(_))
    )
}

/// Whether a scale's type maps categories: band, point, nested band, and ordinal scales.
pub(crate) fn is_discrete(spec: &proto::ScaleSpec) -> bool {
    use scale_spec::Type;
    matches!(
        spec.r#type,
        Some(Type::Band(_) | Type::Point(_) | Type::NestedBand(_) | Type::Ordinal(_))
    )
}

/// The guide that a scale group draws, as `ScaleSpec.guide` gives it: its own guide, or without
/// one, an axis for `x` and `y`, a legend for a discrete scale or the bins of a quantize, quantile,
/// or threshold scale, a colorbar for a continuous color scale, and a legend of the ticks of another
/// continuous scale. `theta` and `radius` draw no guide in a Cartesian chart.
pub(crate) fn guide_kind(group: &ScaleGroup) -> Option<GuideKind> {
    match group
        .spec
        .guide
        .as_ref()
        .and_then(|guide| guide.guide.as_ref())
    {
        Some(guide::Guide::Axis(_)) => return Some(GuideKind::Axis),
        Some(guide::Guide::Legend(_)) => return Some(GuideKind::Legend),
        Some(guide::Guide::Colorbar(_)) => return Some(GuideKind::Colorbar),
        Some(guide::Guide::None(_) | guide::Guide::PolarAxis(_)) => return None,
        None => {}
    }
    match group.channel {
        Channel::X | Channel::Y => Some(GuideKind::Axis),
        Channel::Theta | Channel::Radius => None,
        Channel::Color | Channel::Fill | Channel::Stroke if is_continuous(&group.spec) => {
            Some(GuideKind::Colorbar)
        }
        _ => Some(GuideKind::Legend),
    }
}

/// The name of a scale's type, as the `type` attribute of its nodes gives it: `linear`, `band`,
/// and so on, and the name of a custom scale.
pub(crate) fn scale_type(spec: &proto::ScaleSpec) -> String {
    use scale_spec::Type;
    match spec.r#type.as_ref().expect("a valid scale has a type") {
        Type::Linear(_) => "linear",
        Type::Log(_) => "log",
        Type::Pow(_) => "pow",
        Type::Symlog(_) => "symlog",
        Type::Time(_) => "time",
        Type::Band(_) => "band",
        Type::Point(_) => "point",
        Type::NestedBand(_) => "nested_band",
        Type::Ordinal(_) => "ordinal",
        Type::Quantile(_) => "quantile",
        Type::Quantize(_) => "quantize",
        Type::Threshold(_) => "threshold",
        Type::Custom(custom) => return custom.r#type.clone(),
    }
    .to_string()
}
