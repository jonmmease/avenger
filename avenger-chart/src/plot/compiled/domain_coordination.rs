//! Generic domain coordination for child-frame containers.
//!
//! Container-specific code decides which children belong to the same
//! coordination scope. This module only knows how to merge domain extents for
//! those scope keys.

use std::collections::HashMap;

use avenger_chart_core::scalar_total_cmp;

use crate::scales::domain_extent::{DomainBounds, DomainExtent, RadiusPadding};

use super::CoordinationScopeKey;

/// One local scale-domain extent that participates in child-frame coordination.
#[derive(Clone, Debug)]
pub(crate) struct ChildFrameDomainRequest {
    pub(crate) scope_key: CoordinationScopeKey,
    pub(crate) extent: DomainExtent,
}

impl ChildFrameDomainRequest {
    pub(crate) fn new(scope_key: CoordinationScopeKey, extent: DomainExtent) -> Self {
        Self { scope_key, extent }
    }
}

pub(crate) fn aggregate_domain_requests(
    requests: impl IntoIterator<Item = ChildFrameDomainRequest>,
) -> HashMap<CoordinationScopeKey, DomainExtent> {
    let mut groups: HashMap<CoordinationScopeKey, Vec<DomainExtent>> = HashMap::new();

    for request in requests {
        groups
            .entry(request.scope_key)
            .or_default()
            .push(request.extent);
    }

    groups
        .into_iter()
        .map(|(key, extents)| {
            let mut extents = extents.into_iter();
            let first = extents
                .next()
                .expect("Domain coordination invariant violated: non-empty group expected");
            let unified = extents.fold(first, |acc, extent| union_domain_extents(&acc, &extent));
            (key, unified)
        })
        .collect()
}

/// Union two domain extents.
///
/// Combines the bounds of two extents to form a single extent that covers both.
/// For radius padding, takes the maximum of each direction.
pub(crate) fn union_domain_extents(a: &DomainExtent, b: &DomainExtent) -> DomainExtent {
    match (&a.bounds, &b.bounds) {
        (
            DomainBounds::Numeric {
                min: a_min,
                max: a_max,
            },
            DomainBounds::Numeric {
                min: b_min,
                max: b_max,
            },
        ) => DomainExtent {
            bounds: DomainBounds::Numeric {
                min: a_min.min(*b_min),
                max: a_max.max(*b_max),
            },
            radius: union_radius_padding(&a.radius, &b.radius),
        },
        (
            DomainBounds::Temporal {
                min: a_min,
                max: a_max,
            },
            DomainBounds::Temporal {
                min: b_min,
                max: b_max,
            },
        ) => DomainExtent {
            bounds: DomainBounds::Temporal {
                min: (*a_min).min(*b_min),
                max: (*a_max).max(*b_max),
            },
            radius: union_radius_padding(&a.radius, &b.radius),
        },
        (DomainBounds::Discrete(a_vals), DomainBounds::Discrete(b_vals)) => {
            let mut combined = a_vals.clone();
            for val in b_vals {
                if !combined.contains(val) {
                    combined.push(val.clone());
                }
            }
            combined.sort_by(|a, b| scalar_total_cmp(&a.to_scalar(), &b.to_scalar()));
            DomainExtent {
                bounds: DomainBounds::Discrete(combined),
                radius: None,
            }
        }
        _ => a.clone(), // Type mismatch: keep first.
    }
}

/// Union two optional radius padding values.
///
/// Takes the maximum of each direction.
fn union_radius_padding(
    a: &Option<RadiusPadding>,
    b: &Option<RadiusPadding>,
) -> Option<RadiusPadding> {
    match (a, b) {
        (Some(a), Some(b)) => Some(RadiusPadding {
            max_lower: a.max_lower.max(b.max_lower),
            max_upper: a.max_upper.max(b.max_upper),
        }),
        (Some(r), None) | (None, Some(r)) => Some(r.clone()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use datafusion::common::ScalarValue;

    use crate::{
        plot::compiled::{
            ChildFrameKey, ChildFrameScopeKey, ContainerPathSegment, CoordinationKind,
            CoordinationScopeKey,
        },
        scales::domain_extent::SerializableDomainValue,
    };

    use super::*;

    fn request(channel: &str, path: &str, max: f64) -> ChildFrameDomainRequest {
        let key = CoordinationScopeKey::partition_path(
            CoordinationKind::ScaleDomain,
            vec![ScalarValue::Utf8(Some(path.to_string()))],
        )
        .with_channel(channel);
        ChildFrameDomainRequest::new(key, DomainExtent::numeric(0.0, max))
    }

    fn child_frame_request(
        channel: &str,
        ancestor: &str,
        child_index: usize,
        max: f64,
    ) -> ChildFrameDomainRequest {
        let child_scope = ChildFrameScopeKey::new(
            vec![ContainerPathSegment::concat_child(0, Some(ancestor))],
            ChildFrameKey::ConcatChild {
                index: child_index,
                key: None,
            },
        );
        let key = CoordinationScopeKey::child_frame_container(
            CoordinationKind::ScaleDomain,
            &child_scope,
        )
        .with_channel(channel);
        ChildFrameDomainRequest::new(key, DomainExtent::numeric(0.0, max))
    }

    #[test]
    fn aggregate_domain_requests_groups_by_scope_key() {
        let aggregated = aggregate_domain_requests([
            request("x", "a", 1.0),
            request("x", "a", 2.0),
            request("x", "b", 4.0),
        ]);

        assert_eq!(aggregated.len(), 2);
        assert!(
            aggregated
                .values()
                .any(|extent| extent.numeric_bounds() == Some((0.0, 2.0)))
        );
        assert!(
            aggregated
                .values()
                .any(|extent| extent.numeric_bounds() == Some((0.0, 4.0)))
        );
    }

    #[test]
    fn aggregate_domain_requests_groups_child_frames_by_container_path() {
        let aggregated = aggregate_domain_requests([
            child_frame_request("x", "outer-a", 0, 1.0),
            child_frame_request("x", "outer-a", 1, 2.0),
            child_frame_request("x", "outer-b", 0, 4.0),
        ]);

        assert_eq!(aggregated.len(), 2);
        assert!(
            aggregated
                .values()
                .any(|extent| extent.numeric_bounds() == Some((0.0, 2.0)))
        );
        assert!(
            aggregated
                .values()
                .any(|extent| extent.numeric_bounds() == Some((0.0, 4.0)))
        );
    }

    #[test]
    fn discrete_domain_union_is_deterministic() {
        let a = DomainExtent::discrete(vec![
            SerializableDomainValue::String("D".to_string()),
            SerializableDomainValue::String("A".to_string()),
        ]);
        let b = DomainExtent::discrete(vec![
            SerializableDomainValue::String("C".to_string()),
            SerializableDomainValue::String("B".to_string()),
        ]);

        let unified = union_domain_extents(&a, &b);
        assert_eq!(
            unified.discrete_values(),
            Some(
                [
                    SerializableDomainValue::String("A".to_string()),
                    SerializableDomainValue::String("B".to_string()),
                    SerializableDomainValue::String("C".to_string()),
                    SerializableDomainValue::String("D".to_string()),
                ]
                .as_slice()
            )
        );
    }
}
