//! Scale-domain sharing policy for child-frame containers.
//!
//! This module converts child plot scale-sharing declarations into generic
//! child-frame domain requests. Container implementations still decide which
//! child frames exist and what their scope keys are.

use std::collections::HashMap;

use crate::{
    channel::value::strip_trailing_numbers,
    scales::{DomainExtent, ScaleBuilder},
};

use super::{
    ChildFrameDomainRequest, ChildFrameScopeKey, CompiledPlot, CoordinationKind,
    CoordinationScopeKey, SharingLevel, aggregate_domain_requests,
};

/// One local channel domain plus the child-frame sharing level that applies to it.
#[derive(Clone, Debug)]
pub(crate) struct ChildFrameChannelDomainExtent {
    pub(crate) extent: DomainExtent,
    pub(crate) sharing_level: SharingLevel,
}

/// Domain-sharing inputs for one measured child frame.
pub(crate) struct ChildFrameDomainSharingInput<'a> {
    pub(crate) scope_key: &'a ChildFrameScopeKey,
    pub(crate) local_domain_extents: &'a HashMap<String, ChildFrameChannelDomainExtent>,
    pub(crate) channel_domain_sharing_levels: &'a HashMap<String, SharingLevel>,
}

impl<'a> ChildFrameDomainSharingInput<'a> {
    pub(crate) fn new(
        scope_key: &'a ChildFrameScopeKey,
        local_domain_extents: &'a HashMap<String, ChildFrameChannelDomainExtent>,
        channel_domain_sharing_levels: &'a HashMap<String, SharingLevel>,
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
) -> HashMap<String, SharingLevel> {
    let mut sharing_levels = HashMap::new();
    for mark in &plot.marks {
        for (channel, channel_value) in mark.data_context().channels() {
            let Some(sharing) = channel_value.get_share_mode() else {
                continue;
            };
            let channel = strip_trailing_numbers(channel).to_string();
            let sharing_level = SharingLevel::from(sharing);
            sharing_levels
                .entry(channel)
                .and_modify(|existing: &mut SharingLevel| {
                    *existing = (*existing).max(sharing_level);
                })
                .or_insert(sharing_level);
        }
    }
    sharing_levels
}

/// Extract local child-frame domain extents only for channels that request sharing.
pub(crate) fn extract_child_frame_shared_domain_extents(
    scale_builder: &ScaleBuilder,
    sharing_levels: &HashMap<String, SharingLevel>,
) -> HashMap<String, ChildFrameChannelDomainExtent> {
    let shared_channels = sharing_levels
        .iter()
        .filter_map(|(channel, sharing_level)| {
            (!sharing_level.is_free()).then_some(channel.as_str())
        })
        .collect::<Vec<_>>();
    if shared_channels.is_empty() {
        return HashMap::new();
    }

    scale_builder
        .extract_domain_extents(&shared_channels)
        .into_iter()
        .filter_map(|(channel, extent)| {
            sharing_levels.get(&channel).map(|sharing_level| {
                (
                    channel,
                    ChildFrameChannelDomainExtent {
                        extent,
                        sharing_level: *sharing_level,
                    },
                )
            })
        })
        .collect()
}

pub(crate) fn child_frame_domain_scope_key(
    child_scope: &ChildFrameScopeKey,
    channel: &str,
    sharing_level: SharingLevel,
) -> CoordinationScopeKey {
    debug_assert!(
        !sharing_level.is_free(),
        "Free child-frame scale domains should not need a coordination scope"
    );
    CoordinationScopeKey::child_frame_container(CoordinationKind::ScaleDomain, child_scope)
        .with_channel(channel)
}

fn child_frame_domain_request(
    child_scope: &ChildFrameScopeKey,
    channel: &str,
    annotated: &ChildFrameChannelDomainExtent,
) -> Option<ChildFrameDomainRequest> {
    (!annotated.sharing_level.is_free()).then(|| {
        ChildFrameDomainRequest::new(
            child_frame_domain_scope_key(child_scope, channel, annotated.sharing_level),
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
            for (channel, sharing_level) in child.channel_domain_sharing_levels {
                if sharing_level.is_free() {
                    continue;
                }

                let key = child_frame_domain_scope_key(child.scope_key, channel, *sharing_level);
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
            sharing_level,
        }
    }

    #[test]
    fn coordinated_child_frame_domain_extents_group_siblings() {
        let left_scope = child_scope(None, 0, Some("left"));
        let right_scope = child_scope(None, 1, Some("right"));
        let left_extents = HashMap::from([("x".to_string(), extent(2.0, SharingLevel::GLOBAL))]);
        let right_extents = HashMap::from([("x".to_string(), extent(101.0, SharingLevel::GLOBAL))]);
        let sharing_levels = HashMap::from([("x".to_string(), SharingLevel::GLOBAL)]);
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
        let sharing_levels = HashMap::from([("x".to_string(), SharingLevel::FREE)]);
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
        let sharing_levels = HashMap::from([("x".to_string(), SharingLevel::GLOBAL)]);
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
        let left_extents = HashMap::from([("x".to_string(), extent(2.0, SharingLevel::GLOBAL))]);
        let right_extents = HashMap::from([("x".to_string(), extent(101.0, SharingLevel::GLOBAL))]);
        let sharing_levels = HashMap::from([("x".to_string(), SharingLevel::GLOBAL)]);
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
}
