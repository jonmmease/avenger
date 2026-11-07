//! The styles of a chart: its style tree, whose nodes stand for the parts of the chart, the inline
//! styles that the format's style fields give the nodes, and the choice of each scale's axis or
//! legend.

pub(crate) mod inline;
pub(crate) mod tree;

use avenger_chart::{
    proto::{self, scale, Channel, LegendForm},
    resolve::ScaleGroup,
    FitKind,
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

/// Whether a scale of the kind of fit `kind` maps a continuous domain to a continuous range: an
/// interval scale other than a quantize scale, whose range is bins.
pub(crate) fn is_continuous(spec: &proto::Scale, kind: FitKind) -> bool {
    kind == FitKind::Interval && !matches!(spec.r#type, Some(scale::Type::Quantize(_)))
}

/// The axis or the legend that a scale group draws: none when its spec's axis or legend has `show:
/// false`, an axis for `x` and `y`, and for another channel the legend of its spec's form, or
/// without one, a colorbar for a continuous color scale and a symbol legend for another scale: the
/// categories of a discrete scale, the bins of a quantize, quantile, or threshold scale, or the
/// ticks of another continuous scale. `theta` and `radius` draw no axis in a Cartesian chart.
pub(crate) fn guide_kind(group: &ScaleGroup, kind: FitKind) -> Option<GuideKind> {
    let spec = &group.spec;
    match group.channel {
        Channel::X | Channel::Y => match &spec.axis {
            Some(axis) if axis.show == Some(false) => None,
            _ => Some(GuideKind::Axis),
        },
        Channel::Theta | Channel::Radius => None,
        channel => {
            let legend = spec.legend.as_ref();
            if legend.is_some_and(|legend| legend.show == Some(false)) {
                return None;
            }
            let form = legend.map_or(LegendForm::Unspecified, |legend| {
                LegendForm::try_from(legend.form).unwrap_or_default()
            });
            let color = matches!(channel, Channel::Color | Channel::Fill | Channel::Stroke);
            match form {
                LegendForm::Colorbar => Some(GuideKind::Colorbar),
                LegendForm::Symbol => Some(GuideKind::Legend),
                LegendForm::Unspecified if color && is_continuous(spec, kind) => {
                    Some(GuideKind::Colorbar)
                }
                LegendForm::Unspecified => Some(GuideKind::Legend),
            }
        }
    }
}

/// The name of a scale's type, as the `type` attribute of its nodes gives it: `linear`, `band`,
/// and so on, and the name of a custom scale.
pub(crate) fn scale_type(spec: &proto::Scale) -> String {
    use scale::Type;
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
