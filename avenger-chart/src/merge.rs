//! The merge of the scale specs of a shared scale by the agreement rule of `ScaleSpec`: the specs
//! of the layers that share a scale must agree, and the merged spec takes each field, at every
//! depth, that one of them sets.
//!
//! build.rs generates the field-by-field merge of the guides, the options of the scale types, and
//! the fits from the descriptors of the format, so a field that the format adds merges by the rule
//! of its kind.

use crate::proto::{self, domain, scale_spec};

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

/// The classes of a guide, which join: the merged guide has each class of either, in order.
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

/// Merges `other` into `merged`. Their types must be equal, the custom scales of one type name,
/// or a band and a point scale share a band scale. Summary and data domains merge, with their fits
/// field by field into the domain of `merged`, and explicit and selection domains must be equal.
/// The range and the navigation that one spec sets is the merged value, and those that both set
/// must be equal. The options of the type and the guide merge field by field.
pub(crate) fn merge(
    merged: &mut proto::ScaleSpec,
    other: &proto::ScaleSpec,
    same: &SameValue,
) -> Merged {
    merge_type(&mut merged.r#type, &other.r#type, same)?;
    merge_domain(&mut merged.domain, other.domain.as_ref(), same)?;
    optional(&mut merged.range, &other.range, "range").or_else(|reason| {
        // Ranges of values are equal when their values are.
        use proto::range::Range;
        match (&merged.range, &other.range) {
            (Some(a), Some(b))
                if a.reverse == b.reverse
                    && matches!((&a.range, &b.range), (Some(Range::Values(x)), Some(Range::Values(y))) if same(*x, *y)) =>
            {
                Ok(())
            }
            _ => Err(reason),
        }
    })?;
    optional(
        &mut merged.navigation_limits,
        &other.navigation_limits,
        "navigation_limits",
    )?;
    match (&mut merged.guide, &other.guide) {
        (_, None) => Ok(()),
        (guide @ None, Some(other)) => {
            *guide = Some(other.clone());
            Ok(())
        }
        (Some(guide), Some(other)) => match (&guide.guide, &other.guide) {
            (Some(a), Some(b)) if std::mem::discriminant(a) != std::mem::discriminant(b) => {
                Err("set guides of two kinds".to_string())
            }
            _ => guide.merge_fields(other, same, "guide"),
        },
    }
}

/// The type of both specs, with its options merged.
fn merge_type(
    merged: &mut Option<scale_spec::Type>,
    other: &Option<scale_spec::Type>,
    same: &SameValue,
) -> Merged {
    use scale_spec::Type;
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
            if merged.r#type != other.r#type {
                return Err(differ("type"));
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

/// Whether a domain is inferred: the producer's summary or the layer's data.
pub(crate) fn inferred(domain: Option<&proto::Domain>) -> bool {
    matches!(
        domain.and_then(|domain| domain.domain.as_ref()),
        None | Some(domain::Domain::Summary(_) | domain::Domain::Data(_))
    )
}

fn domain_equal(a: Option<&proto::Domain>, b: Option<&proto::Domain>, same: &SameValue) -> bool {
    use domain::Domain;
    use proto::selection_domain::Fallback;
    match (
        a.and_then(|domain| domain.domain.as_ref()),
        b.and_then(|domain| domain.domain.as_ref()),
    ) {
        (Some(Domain::Explicit(a)), Some(Domain::Explicit(b))) => same(*a, *b),
        (Some(Domain::Selection(a)), Some(Domain::Selection(b))) => {
            a.selection == b.selection
                && a.channel == b.channel
                && match (&a.fallback, &b.fallback) {
                    (Some(Fallback::Explicit(x)), Some(Fallback::Explicit(y))) => same(*x, *y),
                    (Some(Fallback::Summary(x)), Some(Fallback::Summary(y))) => {
                        (x.value.zip(y.value)).is_some_and(|(x, y)| same(x, y))
                            && fit_equal(x.fit.as_ref(), y.fit.as_ref(), same)
                    }
                    (Some(Fallback::Data(x)), Some(Fallback::Data(y))) => {
                        fit_equal(x.fit.as_ref(), y.fit.as_ref(), same)
                    }
                    _ => false,
                }
        }
        (a, b) => a == b,
    }
}

/// The fit of a summary or a data domain, to change.
fn fit_mut(domain: &mut Option<proto::Domain>) -> Option<&mut Option<proto::DomainFit>> {
    match domain.as_mut()?.domain.as_mut()? {
        domain::Domain::Summary(summary) => Some(&mut summary.fit),
        domain::Domain::Data(data) => Some(&mut data.fit),
        _ => None,
    }
}

/// The fit of a summary or a data domain.
fn fit(domain: Option<&proto::Domain>) -> Option<&proto::DomainFit> {
    match domain?.domain.as_ref()? {
        domain::Domain::Summary(summary) => summary.fit.as_ref(),
        domain::Domain::Data(data) => data.fit.as_ref(),
        _ => None,
    }
}

/// Whether two fits set the same fields to the same values.
fn fit_equal(a: Option<&proto::DomainFit>, b: Option<&proto::DomainFit>, same: &SameValue) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            let (mut ab, mut ba) = (*a, *b);
            // A field that one sets and the other does not changes the merge of the other.
            ab.merge_fields(b, same, "fit").is_ok()
                && ba.merge_fields(a, same, "fit").is_ok()
                && ab == *a
                && ba == *b
        }
        (a, b) => a.is_none() && b.is_none(),
    }
}

/// Merges the domain of `other` into `merged`: summary and data domains merge, with their fits
/// field by field into the fit of `merged`, and explicit and selection domains must be equal.
fn merge_domain(
    merged: &mut Option<proto::Domain>,
    other: Option<&proto::Domain>,
    same: &SameValue,
) -> Merged {
    match (inferred(merged.as_ref()), inferred(other)) {
        (true, true) => match (fit_mut(merged), fit(other)) {
            (_, None) | (None, _) => Ok(()),
            (Some(slot @ None), Some(other)) => {
                *slot = Some(*other);
                Ok(())
            }
            (Some(Some(fit)), Some(other)) => fit.merge_fields(other, same, "fit"),
        },
        (false, false) if domain_equal(merged.as_ref(), other, same) => Ok(()),
        (false, false) => Err(differ("domain")),
        _ => Err("set an explicit or a selection domain and a summary or a data domain, which is a conflict".to_string()),
    }
}
