//! Scale-domain sharing policy for child-frame containers.
//!
//! This module converts child plot scale-sharing declarations into generic
//! child-frame domain requests. Container implementations still decide which
//! child frames exist and what their scope keys are.

use std::collections::HashMap;

use avenger_chart_core::{
    DomainCoordination, DomainCoordinationGroup, SharingLevel, sharing_group_boundary,
};

use crate::{
    channel::value::strip_trailing_numbers,
    scales::{DomainExtent, ScaleBuilder},
};

use super::{
    ChildFrameDomainRequest, ChildFrameScopeKey, CompiledPlot, CoordinationKind,
    CoordinationScopeKey, aggregate_domain_requests, coordination_scope::CoordinationGroup,
};

/// One local channel domain plus the child-frame domain coordination that applies to it.
#[derive(Clone, Debug)]
pub(crate) struct ChildFrameChannelDomainExtent {
    pub(crate) extent: DomainExtent,
    pub(crate) domain_coordination: DomainCoordination,
}

/// Domain-sharing inputs for one measured child frame.
pub(crate) struct ChildFrameDomainSharingInput<'a> {
    pub(crate) scope_key: &'a ChildFrameScopeKey,
    pub(crate) local_domain_extents: &'a HashMap<String, ChildFrameChannelDomainExtent>,
    pub(crate) channel_domain_sharing_levels: &'a HashMap<String, DomainCoordination>,
}

impl<'a> ChildFrameDomainSharingInput<'a> {
    pub(crate) fn new(
        scope_key: &'a ChildFrameScopeKey,
        local_domain_extents: &'a HashMap<String, ChildFrameChannelDomainExtent>,
        channel_domain_sharing_levels: &'a HashMap<String, DomainCoordination>,
    ) -> Self {
        Self {
            scope_key,
            local_domain_extents,
            channel_domain_sharing_levels,
        }
    }
}

/// Extract the strongest scale-sharing level requested by each child plot channel.
pub(crate) fn child_frame_domain_sharing_levels_for_plot(
    plot: &CompiledPlot,
) -> HashMap<String, DomainCoordination> {
    let mut coordinations = HashMap::new();
    for mark in &plot.marks {
        for (channel, channel_value) in mark.data_context().channels() {
            let Some(coordination) = channel_value.get_domain_coordination() else {
                continue;
            };
            let channel = strip_trailing_numbers(channel).to_string();
            let sharing_level = SharingLevel::from(coordination.scope);
            coordinations
                .entry(channel)
                .and_modify(|existing: &mut DomainCoordination| {
                    if sharing_level > SharingLevel::from(existing.scope) {
                        *existing = coordination.clone();
                    }
                })
                .or_insert_with(|| coordination.clone());
        }
    }
    coordinations
}

/// Extract local child-frame domain extents only for channels that request sharing.
pub(crate) fn extract_child_frame_shared_domain_extents(
    scale_builder: &ScaleBuilder,
    domain_coordinations: &HashMap<String, DomainCoordination>,
) -> HashMap<String, ChildFrameChannelDomainExtent> {
    let shared_channels = domain_coordinations
        .iter()
        .filter_map(|(channel, coordination)| {
            (!SharingLevel::from(coordination.scope).is_free()).then_some(channel.as_str())
        })
        .collect::<Vec<_>>();
    if shared_channels.is_empty() {
        return HashMap::new();
    }

    scale_builder
        .extract_domain_extents(&shared_channels)
        .into_iter()
        .filter_map(|(channel, extent)| {
            domain_coordinations.get(&channel).map(|coordination| {
                (
                    channel,
                    ChildFrameChannelDomainExtent {
                        extent,
                        domain_coordination: coordination.clone(),
                    },
                )
            })
        })
        .collect()
}

pub(crate) fn child_frame_domain_scope_key(
    child_scope: &ChildFrameScopeKey,
    channel: &str,
    coordination: &DomainCoordination,
) -> CoordinationScopeKey {
    let sharing_level = SharingLevel::from(coordination.scope);
    debug_assert!(
        !sharing_level.is_free(),
        "Free child-frame scale domains should not need a coordination scope"
    );
    let depth = u8::try_from(child_scope.container_path.len() + 1).unwrap_or(u8::MAX);
    let boundary = sharing_group_boundary(depth, sharing_level);
    let container_path = child_scope
        .container_path
        .get(..boundary.min(child_scope.container_path.len()))
        .unwrap_or(&child_scope.container_path)
        .to_vec();

    let key = CoordinationScopeKey::new(
        CoordinationKind::ScaleDomain,
        container_path,
        CoordinationGroup::Container,
    );
    apply_domain_group_to_key(key, channel, coordination)
}

pub(crate) fn apply_domain_group_to_key(
    key: CoordinationScopeKey,
    channel: &str,
    coordination: &DomainCoordination,
) -> CoordinationScopeKey {
    match &coordination.group {
        DomainCoordinationGroup::ScaleName => key.with_channel(channel),
        DomainCoordinationGroup::Named(group) => key.with_named_group(group.clone()),
    }
}

fn child_frame_domain_request(
    child_scope: &ChildFrameScopeKey,
    channel: &str,
    annotated: &ChildFrameChannelDomainExtent,
) -> Option<ChildFrameDomainRequest> {
    (!SharingLevel::from(annotated.domain_coordination.scope).is_free()).then(|| {
        ChildFrameDomainRequest::new(
            child_frame_domain_scope_key(child_scope, channel, &annotated.domain_coordination),
            annotated.extent.clone(),
        )
    })
}

/// Resolve the coordinated scale domains each child frame should use.
///
/// Free channels are omitted. Non-free channels are grouped by the current
/// child-frame container. Empty children with no local extent still receive a
/// coordinated extent when another child in the group supplied one.
pub(crate) fn coordinated_child_frame_domain_extents(
    children: &[ChildFrameDomainSharingInput<'_>],
) -> Vec<HashMap<String, DomainExtent>> {
    let unified = aggregate_domain_requests(children.iter().flat_map(|child| {
        child
            .local_domain_extents
            .iter()
            .filter_map(move |(channel, annotated)| {
                child_frame_domain_request(child.scope_key, channel, annotated)
            })
    }));

    children
        .iter()
        .map(|child| {
            let mut coordinated = HashMap::new();
            for (channel, coordination) in child.channel_domain_sharing_levels {
                if SharingLevel::from(coordination.scope).is_free() {
                    continue;
                }

                let key = child_frame_domain_scope_key(child.scope_key, channel, coordination);
                if let Some(unified_extent) = unified.get(&key) {
                    coordinated.insert(channel.clone(), unified_extent.clone());
                }
            }
            coordinated
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{
        container::{ChildFrameKey, ContainerPathSegment},
        scales::domain_extent::DomainExtent,
    };

    use super::*;

    fn child_scope(
        ancestor: Option<&str>,
        child_index: usize,
        child_key: Option<&str>,
    ) -> ChildFrameScopeKey {
        let container_path = ancestor
            .map(|key| vec![ContainerPathSegment::concat_child(0, Some(key))])
            .unwrap_or_default();
        ChildFrameScopeKey::new(
            container_path,
            ChildFrameKey::ConcatChild {
                index: child_index,
                key: child_key.map(ToOwned::to_owned),
            },
        )
    }

    fn extent(max: f64, sharing_level: SharingLevel) -> ChildFrameChannelDomainExtent {
        ChildFrameChannelDomainExtent {
            extent: DomainExtent::numeric(0.0, max),
            domain_coordination: DomainCoordination::scale_name(sharing_level.into()),
        }
    }

    fn named_extent(
        max: f64,
        sharing_level: SharingLevel,
        group: &str,
    ) -> ChildFrameChannelDomainExtent {
        ChildFrameChannelDomainExtent {
            extent: DomainExtent::numeric(0.0, max),
            domain_coordination: DomainCoordination::named(sharing_level.into(), group).unwrap(),
        }
    }

    #[test]
    fn coordinated_child_frame_domain_extents_group_siblings() {
        let left_scope = child_scope(None, 0, Some("left"));
        let right_scope = child_scope(None, 1, Some("right"));
        let left_extents = HashMap::from([("x".to_string(), extent(2.0, SharingLevel::GLOBAL))]);
        let right_extents = HashMap::from([("x".to_string(), extent(101.0, SharingLevel::GLOBAL))]);
        let sharing_levels = HashMap::from([(
            "x".to_string(),
            DomainCoordination::scale_name(SharingLevel::GLOBAL.into()),
        )]);
        let inputs = [
            ChildFrameDomainSharingInput::new(&left_scope, &left_extents, &sharing_levels),
            ChildFrameDomainSharingInput::new(&right_scope, &right_extents, &sharing_levels),
        ];

        let coordinated = coordinated_child_frame_domain_extents(&inputs);

        assert_eq!(coordinated.len(), 2);
        assert_eq!(
            coordinated[0].get("x"),
            Some(&DomainExtent::numeric(0.0, 101.0))
        );
        assert_eq!(coordinated[0].get("x"), coordinated[1].get("x"));
    }

    #[test]
    fn free_child_frame_domain_extents_are_not_coordinated() {
        let scope = child_scope(None, 0, Some("left"));
        let local_extents = HashMap::from([("x".to_string(), extent(2.0, SharingLevel::FREE))]);
        let sharing_levels = HashMap::from([(
            "x".to_string(),
            DomainCoordination::scale_name(SharingLevel::FREE.into()),
        )]);
        let inputs = [ChildFrameDomainSharingInput::new(
            &scope,
            &local_extents,
            &sharing_levels,
        )];

        let coordinated = coordinated_child_frame_domain_extents(&inputs);

        assert_eq!(coordinated, vec![HashMap::new()]);
    }

    #[test]
    fn empty_child_frame_uses_saved_sharing_level() {
        let left_scope = child_scope(None, 0, Some("left"));
        let right_scope = child_scope(None, 1, Some("right"));
        let left_extents = HashMap::from([("x".to_string(), extent(2.0, SharingLevel::GLOBAL))]);
        let empty_extents = HashMap::new();
        let sharing_levels = HashMap::from([(
            "x".to_string(),
            DomainCoordination::scale_name(SharingLevel::GLOBAL.into()),
        )]);
        let inputs = [
            ChildFrameDomainSharingInput::new(&left_scope, &left_extents, &sharing_levels),
            ChildFrameDomainSharingInput::new(&right_scope, &empty_extents, &sharing_levels),
        ];

        let coordinated = coordinated_child_frame_domain_extents(&inputs);

        assert_eq!(
            coordinated[1].get("x"),
            Some(&DomainExtent::numeric(0.0, 2.0))
        );
    }

    #[test]
    fn child_frame_domain_sharing_respects_container_path() {
        let left_scope = child_scope(Some("outer-left"), 0, Some("inner"));
        let right_scope = child_scope(Some("outer-right"), 0, Some("inner"));
        let left_extents =
            HashMap::from([("x".to_string(), extent(2.0, SharingLevel::from_raw(1)))]);
        let right_extents =
            HashMap::from([("x".to_string(), extent(101.0, SharingLevel::from_raw(1)))]);
        let sharing_levels = HashMap::from([(
            "x".to_string(),
            DomainCoordination::scale_name(SharingLevel::from_raw(1).into()),
        )]);
        let inputs = [
            ChildFrameDomainSharingInput::new(&left_scope, &left_extents, &sharing_levels),
            ChildFrameDomainSharingInput::new(&right_scope, &right_extents, &sharing_levels),
        ];

        let coordinated = coordinated_child_frame_domain_extents(&inputs);

        assert_eq!(
            coordinated[0].get("x"),
            Some(&DomainExtent::numeric(0.0, 2.0))
        );
        assert_eq!(
            coordinated[1].get("x"),
            Some(&DomainExtent::numeric(0.0, 101.0))
        );
    }

    #[test]
    fn named_child_frame_domain_extents_can_group_different_channels() {
        let left_scope = child_scope(None, 0, Some("left"));
        let right_scope = child_scope(None, 1, Some("right"));
        let left_extents = HashMap::from([(
            "x".to_string(),
            named_extent(2.0, SharingLevel::GLOBAL, "height"),
        )]);
        let right_extents = HashMap::from([(
            "y".to_string(),
            named_extent(101.0, SharingLevel::GLOBAL, "height"),
        )]);
        let left_sharing = HashMap::from([(
            "x".to_string(),
            DomainCoordination::named(SharingLevel::GLOBAL.into(), "height").unwrap(),
        )]);
        let right_sharing = HashMap::from([(
            "y".to_string(),
            DomainCoordination::named(SharingLevel::GLOBAL.into(), "height").unwrap(),
        )]);
        let inputs = [
            ChildFrameDomainSharingInput::new(&left_scope, &left_extents, &left_sharing),
            ChildFrameDomainSharingInput::new(&right_scope, &right_extents, &right_sharing),
        ];

        let coordinated = coordinated_child_frame_domain_extents(&inputs);

        assert_eq!(
            coordinated[0].get("x"),
            Some(&DomainExtent::numeric(0.0, 101.0))
        );
        assert_eq!(
            coordinated[1].get("y"),
            Some(&DomainExtent::numeric(0.0, 101.0))
        );
    }

    #[test]
    fn child_frame_domain_sharing_level_two_projects_one_ancestor() {
        let left_scope = child_scope(Some("outer-left"), 0, Some("inner"));
        let right_scope = child_scope(Some("outer-right"), 0, Some("inner"));
        let left_extents =
            HashMap::from([("x".to_string(), extent(2.0, SharingLevel::from_raw(2)))]);
        let right_extents =
            HashMap::from([("x".to_string(), extent(101.0, SharingLevel::from_raw(2)))]);
        let sharing_levels = HashMap::from([(
            "x".to_string(),
            DomainCoordination::scale_name(SharingLevel::from_raw(2).into()),
        )]);
        let inputs = [
            ChildFrameDomainSharingInput::new(&left_scope, &left_extents, &sharing_levels),
            ChildFrameDomainSharingInput::new(&right_scope, &right_extents, &sharing_levels),
        ];

        let coordinated = coordinated_child_frame_domain_extents(&inputs);

        assert_eq!(
            coordinated[0].get("x"),
            Some(&DomainExtent::numeric(0.0, 101.0))
        );
        assert_eq!(coordinated[0].get("x"), coordinated[1].get("x"));
    }
}
