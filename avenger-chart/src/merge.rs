//! The merge of the scale specs of a shared scale by the agreement rule of `Scale`, and of the
//! axes and the legends of its encodings by the rule of `Encoding.axis`: the specs and the guides
//! of the layers that share a scale must agree, and the merged one takes each field, at every
//! depth, that one of them sets.
//!
//! build.rs generates the field-by-field merge of the axes, the legends, the options of the scale
//! types, the pins of the fits, and the widgets from the descriptors of the format, so a field that
//! the format adds merges by the rule of its kind.

use crate::proto::{
    self,
    scale::{self, Domain, Range, Sort},
};

/// Why two specs disagree, as the end of a sentence about them, such as "set `range` to
/// different values".
pub(crate) type Disagreement = String;

pub(crate) type Merged = std::result::Result<(), Disagreement>;

/// Whether the values that two `ValueRef`s refer to are equal.
pub(crate) type SameValue<'a> = dyn Fn(proto::ValueRef, proto::ValueRef) -> bool + 'a;

/// A message whose fields merge one by one. build.rs implements it for each message that the
/// specs of a shared scale merge field by field.
pub(crate) trait MergeFields {
    /// Merges `other` into `self`, where `path` names `self` in a disagreement.
    fn merge_fields(&mut self, other: &Self, same: &SameValue, path: &str) -> Merged;
}

fn differ(field: &str) -> Disagreement {
    format!(
        "set `{}` to different values",
        field.trim_start_matches('.')
    )
}

/// An `optional` field or a message that does not merge field by field, set when it is `Some`.
fn optional<T: PartialEq + Clone>(
    merged: &mut Option<T>,
    other: &Option<T>,
    field: &str,
) -> Merged {
    match (merged.as_ref(), other) {
        (_, None) => Ok(()),
        (None, Some(other)) => {
            *merged = Some(other.clone());
            Ok(())
        }
        (Some(value), Some(other)) if value == other => Ok(()),
        _ => Err(differ(field)),
    }
}

/// A string field, set when it is not empty.
fn text(merged: &mut String, other: &str, field: &str) -> Merged {
    match (merged.is_empty(), other.is_empty()) {
        (_, true) => Ok(()),
        (true, false) => {
            *merged = other.to_string();
            Ok(())
        }
        (false, false) if merged == other => Ok(()),
        _ => Err(differ(field)),
    }
}

/// An enum field, set when it is not its zero value.
fn enumeration(merged: &mut i32, other: &i32, field: &str) -> Merged {
    match (*merged, *other) {
        (_, 0) => Ok(()),
        (0, other) => {
            *merged = other;
            Ok(())
        }
        (value, other) if value == other => Ok(()),
        _ => Err(differ(field)),
    }
}

/// A scalar field without a state of being not set, whose values must be equal.
fn plain<T: PartialEq>(merged: &mut T, other: &T, field: &str) -> Merged {
    match merged == other {
        true => Ok(()),
        false => Err(differ(field)),
    }
}

/// The classes of an axis or a legend, which join: the merged one has each class of either, in
/// order.
fn classes(merged: &mut Vec<String>, other: &[String], _field: &str) -> Merged {
    for class in other {
        if !merged.contains(class) {
            merged.push(class.clone());
        }
    }
    Ok(())
}

/// A repeated field, set when it is not empty.
fn repeated<T: PartialEq + Clone>(merged: &mut Vec<T>, other: &Vec<T>, field: &str) -> Merged {
    match (merged.is_empty(), other.is_empty()) {
        (_, true) => Ok(()),
        (true, false) => {
            *merged = other.clone();
            Ok(())
        }
        (false, false) if merged == other => Ok(()),
        _ => Err(differ(field)),
    }
}

/// A reference to a value, set when it is `Some`, which agrees with another reference to an equal
/// value.
fn reference(
    merged: &mut Option<proto::ValueRef>,
    other: &Option<proto::ValueRef>,
    same: &SameValue,
    field: &str,
) -> Merged {
    match (*merged, *other) {
        (_, None) => Ok(()),
        (None, Some(other)) => {
            *merged = Some(other);
            Ok(())
        }
        (Some(value), Some(other)) if same(value, other) => Ok(()),
        _ => Err(differ(field)),
    }
}

include!(concat!(env!("OUT_DIR"), "/merge.rs"));

/// Merges `other` into `merged`. Their types must be equal, the custom scales of one name, or a
/// band and a point scale share a band scale. Summary and data domains merge, with their fit fields
/// field by field into `merged`, and explicit and selection domains must be equal, selection
/// domains with their fit fields. The range and the navigation that one spec sets is the merged
/// value, and those that both set must be equal, and a reversed spec reverses the merged one. The
/// options of the type merge field by field.
pub(crate) fn merge(merged: &mut proto::Scale, other: &proto::Scale, same: &SameValue) -> Merged {
    merge_type(&mut merged.r#type, &other.r#type, same)?;
    merge_domain(&mut merged.domain, other.domain.as_ref(), same)?;
    match (&merged.domain, &other.domain) {
        (Some(Domain::Selection(_)), Some(Domain::Selection(_)))
            if !fit_equal(merged, other, same) =>
        {
            return Err(differ("domain"))
        }
        _ => merge_fit(merged, other, same)?,
    }
    optional(&mut merged.range, &other.range, "range").or_else(|reason| {
        // Ranges of values are equal when their values are.
        match (&merged.range, &other.range) {
            (Some(Range::Values(a)), Some(Range::Values(b))) if same(*a, *b) => Ok(()),
            _ => Err(reason),
        }
    })?;
    merged.reverse |= other.reverse;
    optional(&mut merged.navigation, &other.navigation, "navigation")
}

/// Merges the axis and the legend of `other`, an encoding of a shared scale, into `axis` and
/// `legend` by the rule of `Encoding.axis`: field by field, where a setting that one sets is the
/// merged one, settings that both set must be equal, and the classes join.
pub(crate) fn merge_guides(
    axis: &mut Option<proto::Axis>,
    legend: &mut Option<proto::Legend>,
    other: &proto::Encoding,
    same: &SameValue,
) -> Merged {
    optional_merged(axis, &other.axis, same, "axis")?;
    optional_merged(legend, &other.legend, same, "legend")
}

/// Merges the fit fields of `other` into `merged`, field by field.
fn merge_fit(merged: &mut proto::Scale, other: &proto::Scale, same: &SameValue) -> Merged {
    optional(&mut merged.zero, &other.zero, "zero")?;
    optional(&mut merged.nice, &other.nice, "nice")?;
    optional_merged(&mut merged.pin, &other.pin, same, "pin")?;
    reference(
        &mut merged.domain_empty,
        &other.domain_empty,
        same,
        "domain_empty",
    )?;
    optional(&mut merged.sort, &other.sort, "sort").or_else(|reason| {
        // Preferred orders are equal when their values are.
        match (&merged.sort, &other.sort) {
            (Some(Sort::Preferred(a)), Some(Sort::Preferred(b))) if same(*a, *b) => Ok(()),
            _ => Err(reason),
        }
    })
}

/// The fit fields of `scale`, with no other field.
fn fit_fields(scale: &proto::Scale) -> proto::Scale {
    proto::Scale {
        zero: scale.zero,
        nice: scale.nice,
        pin: scale.pin,
        domain_empty: scale.domain_empty,
        sort: scale.sort,
        ..Default::default()
    }
}

/// Whether two specs set the same fit fields to the same values.
fn fit_equal(a: &proto::Scale, b: &proto::Scale, same: &SameValue) -> bool {
    let (a, b) = (fit_fields(a), fit_fields(b));
    let (mut ab, mut ba) = (a.clone(), b.clone());
    // A field that one sets and the other does not changes the merge of the other.
    merge_fit(&mut ab, &b, same).is_ok()
        && merge_fit(&mut ba, &a, same).is_ok()
        && ab == a
        && ba == b
}

/// Merges an optional message field by field: an absent side takes the other, and two present
/// sides merge their fields.
fn optional_merged<T: MergeFields + Clone>(
    merged: &mut Option<T>,
    other: &Option<T>,
    same: &SameValue,
    name: &str,
) -> Merged {
    match (merged.as_mut(), other) {
        (_, None) => Ok(()),
        (None, Some(other)) => {
            *merged = Some(other.clone());
            Ok(())
        }
        (Some(merged), Some(other)) => merged.merge_fields(other, same, name),
    }
}

/// The type of both specs, with its options merged.
fn merge_type(
    merged: &mut Option<scale::Type>,
    other: &Option<scale::Type>,
    same: &SameValue,
) -> Merged {
    use scale::Type;
    let (Some(merged_type), Some(other)) = (merged.as_mut(), other.as_ref()) else {
        return Ok(());
    };
    // A point scale's options as the options of the band scale that it shares.
    let as_band = |point: &proto::PointScale| -> Result<proto::BandScale, Disagreement> {
        if point.padding.is_some() {
            return Err("set the `padding` of a point scale, which is invalid on the band scale that they share".to_string());
        }
        Ok(proto::BandScale {
            align: point.align,
            round: point.round,
            clip_padding_lower: point.clip_padding_lower,
            clip_padding_upper: point.clip_padding_upper,
            ..Default::default()
        })
    };
    match (merged_type, other) {
        (Type::Custom(merged), Type::Custom(other)) => {
            if merged.name != other.name {
                return Err(differ("custom.name"));
            }
            for option in &other.options {
                match merged
                    .options
                    .iter()
                    .find(|known| known.name == option.name)
                {
                    Some(known) if known.value == option.value => {}
                    Some(_) => return Err(differ(&format!("custom.options.{}", option.name))),
                    None => merged.options.push(option.clone()),
                }
            }
            Ok(())
        }
        (Type::Band(band), Type::Point(point)) => band.merge_fields(&as_band(point)?, same, "band"),
        (merged_type @ Type::Point(_), Type::Band(other_band)) => {
            let Type::Point(point) = merged_type else {
                unreachable!("the type is a point scale")
            };
            let mut band = as_band(point)?;
            band.merge_fields(other_band, same, "band")?;
            *merged_type = Type::Band(band);
            Ok(())
        }
        (Type::Linear(a), Type::Linear(b)) => a.merge_fields(b, same, "linear"),
        (Type::Log(a), Type::Log(b)) => a.merge_fields(b, same, "log"),
        (Type::Pow(a), Type::Pow(b)) => a.merge_fields(b, same, "pow"),
        (Type::Symlog(a), Type::Symlog(b)) => a.merge_fields(b, same, "symlog"),
        (Type::Time(a), Type::Time(b)) => a.merge_fields(b, same, "time"),
        (Type::Band(a), Type::Band(b)) => a.merge_fields(b, same, "band"),
        (Type::Point(a), Type::Point(b)) => a.merge_fields(b, same, "point"),
        (Type::NestedBand(a), Type::NestedBand(b)) => a.merge_fields(b, same, "nested_band"),
        (Type::Ordinal(a), Type::Ordinal(b)) => a.merge_fields(b, same, "ordinal"),
        (Type::Quantile(a), Type::Quantile(b)) => a.merge_fields(b, same, "quantile"),
        (Type::Quantize(a), Type::Quantize(b)) => a.merge_fields(b, same, "quantize"),
        (Type::Threshold(a), Type::Threshold(b)) => a.merge_fields(b, same, "threshold"),
        _ => Err(differ("type")),
    }
}

/// Whether a domain is inferred: the producer's summary or, when unset, the layer's data.
pub(crate) fn inferred(domain: Option<&Domain>) -> bool {
    matches!(domain, None | Some(Domain::Summary(_)))
}

fn domain_equal(a: Option<&Domain>, b: Option<&Domain>, same: &SameValue) -> bool {
    use proto::selection_domain::Fallback;
    match (a, b) {
        (Some(Domain::Explicit(a)), Some(Domain::Explicit(b))) => same(*a, *b),
        (Some(Domain::Selection(a)), Some(Domain::Selection(b))) => {
            a.selection == b.selection
                && a.channel == b.channel
                && match (&a.fallback, &b.fallback) {
                    (Some(Fallback::Explicit(x)), Some(Fallback::Explicit(y)))
                    | (Some(Fallback::Summary(x)), Some(Fallback::Summary(y))) => same(*x, *y),
                    (None, None) => true,
                    _ => false,
                }
        }
        (a, b) => a == b,
    }
}

/// Merges the domain of `other` into `merged`: summary and data domains merge, and `merged` keeps
/// its own, and explicit and selection domains must be equal.
fn merge_domain(merged: &mut Option<Domain>, other: Option<&Domain>, same: &SameValue) -> Merged {
    match (inferred(merged.as_ref()), inferred(other)) {
        (true, true) => Ok(()),
        (false, false) if domain_equal(merged.as_ref(), other, same) => Ok(()),
        (false, false) => Err(differ("domain")),
        _ => Err("set an explicit or a selection domain and a summary or a data domain, which is a conflict".to_string()),
    }
}

/// Merges the copy `other` of a widget into `merged`, by the rule of `Widget`: their kinds are of
/// one kind, an option that one sets is the merged value and those that both set must be equal, and
/// their classes join. Summary and data domains merge, with their fit fields field by field into
/// the domain of `merged`, and explicit domains must be equal. The columns of the copies can differ
/// by name, so `merged` keeps its own.
pub(crate) fn merge_widget(
    merged: &mut proto::Widget,
    other: &proto::Widget,
    same: &SameValue,
) -> Merged {
    use proto::widget::Kind;
    if (merged.kind.as_ref().map(std::mem::discriminant))
        != (other.kind.as_ref().map(std::mem::discriminant))
    {
        return Err("are of two kinds".to_string());
    }
    // The domains merge by their own rule, and the columns not at all, so the fields of the rest
    // merge one by one.
    let domain = |widget: &mut proto::Widget| -> Option<proto::WidgetDomain> {
        match widget.kind.as_mut()? {
            Kind::Slider(slider) => slider.domain.take(),
            Kind::RangeSlider(slider) => slider.domain.take(),
            Kind::Radio(radio) => radio.domain.take(),
            Kind::Checkboxes(checkboxes) => checkboxes.domain.take(),
            Kind::Checkbox(_) | Kind::Mode(_) => None,
        }
    };
    let restore =
        |widget: &mut proto::Widget, value: Option<proto::WidgetDomain>| match widget.kind.as_mut()
        {
            Some(Kind::Slider(slider)) => slider.domain = value,
            Some(Kind::RangeSlider(slider)) => slider.domain = value,
            Some(Kind::Radio(radio)) => radio.domain = value,
            Some(Kind::Checkboxes(checkboxes)) => checkboxes.domain = value,
            Some(Kind::Checkbox(_) | Kind::Mode(_)) | None => {}
        };
    let mut rest = other.clone();
    let column = merged.column().to_string();
    if let Some(target) = target_mut(&mut rest) {
        target.column = column;
    }
    let other_domain = domain(&mut rest);
    let mut merged_domain = domain(merged);
    let fields = merged.merge_fields(&rest, same, "widget");
    let domains = merge_widget_domain(&mut merged_domain, other_domain.as_ref(), same);
    restore(merged, merged_domain);
    fields.and(domains)
}

/// The target of a widget's kind, to change.
fn target_mut(widget: &mut proto::Widget) -> Option<&mut proto::WidgetTarget> {
    use proto::widget::Kind;
    match widget.kind.as_mut()? {
        Kind::Slider(slider) => slider.target.as_mut(),
        Kind::RangeSlider(slider) => slider.target.as_mut(),
        Kind::Radio(radio) => radio.target.as_mut(),
        Kind::Checkboxes(checkboxes) => checkboxes.target.as_mut(),
        Kind::Checkbox(checkbox) => checkbox.target.as_mut(),
        Kind::Mode(_) => None,
    }
}

/// Merges the domain of a widget's copy into `merged`, as `merge_domain` merges a scale's.
fn merge_widget_domain(
    merged: &mut Option<proto::WidgetDomain>,
    other: Option<&proto::WidgetDomain>,
    same: &SameValue,
) -> Merged {
    use proto::widget_domain::Domain;
    fn kind(domain: Option<&proto::WidgetDomain>) -> Option<&proto::widget_domain::Domain> {
        domain.and_then(|domain| domain.domain.as_ref())
    }
    match (kind(merged.as_ref()).cloned(), kind(other)) {
        (Some(Domain::Explicit(a)), Some(Domain::Explicit(b))) => match same(a, *b) {
            true => Ok(()),
            false => Err(differ("domain")),
        },
        (Some(Domain::Explicit(_)), Some(_)) | (Some(_), Some(Domain::Explicit(_))) => Err(
            "set an explicit domain and a summary or a data domain, which is a conflict"
                .to_string(),
        ),
        (Some(_), Some(other_domain)) => {
            let other_fit = match other_domain {
                Domain::Summary(summary) => summary_fit(summary),
                Domain::Data(data) => *data,
                Domain::Explicit(_) => unreachable!("explicit domains merge above"),
            };
            match merged.as_mut().and_then(|domain| domain.domain.as_mut()) {
                Some(Domain::Summary(summary)) => {
                    let mut fit = summary_fit(summary);
                    fit.merge_fields(&other_fit, same, "domain")?;
                    set_summary_fit(summary, fit);
                    Ok(())
                }
                Some(Domain::Data(data)) => data.merge_fields(&other_fit, same, "domain"),
                _ => unreachable!("the merged domain is a summary or a data domain"),
            }
        }
        // A check box has no domain, and a valid widget of another kind has one.
        _ => Ok(()),
    }
}

/// The fit fields of a widget's summary domain, as a data domain holds them.
fn summary_fit(summary: &proto::WidgetSummaryDomain) -> proto::WidgetDataDomain {
    use proto::{widget_data_domain as data, widget_summary_domain as summary_domain};
    proto::WidgetDataDomain {
        zero: summary.zero,
        nice: summary.nice.map(|nice| match nice {
            summary_domain::Nice::Enabled(enabled) => data::Nice::Enabled(enabled),
            summary_domain::Nice::TickCount(count) => data::Nice::TickCount(count),
        }),
        pin: summary.pin,
        domain_empty: summary.domain_empty,
        sort: summary.sort.map(|sort| match sort {
            summary_domain::Sort::Direction(direction) => data::Sort::Direction(direction),
            summary_domain::Sort::Preferred(preferred) => data::Sort::Preferred(preferred),
        }),
    }
}

/// Sets the fit fields of a widget's summary domain to those of `fit`.
fn set_summary_fit(summary: &mut proto::WidgetSummaryDomain, fit: proto::WidgetDataDomain) {
    use proto::{widget_data_domain as data, widget_summary_domain as summary_domain};
    summary.zero = fit.zero;
    summary.nice = fit.nice.map(|nice| match nice {
        data::Nice::Enabled(enabled) => summary_domain::Nice::Enabled(enabled),
        data::Nice::TickCount(count) => summary_domain::Nice::TickCount(count),
    });
    summary.pin = fit.pin;
    summary.domain_empty = fit.domain_empty;
    summary.sort = fit.sort.map(|sort| match sort {
        data::Sort::Direction(direction) => summary_domain::Sort::Direction(direction),
        data::Sort::Preferred(preferred) => summary_domain::Sort::Preferred(preferred),
    });
}
