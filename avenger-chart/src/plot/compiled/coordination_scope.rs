//! Generic coordination scope identity for child-frame containers.
//!
//! These keys describe why measurements should be coordinated together. They
//! are deliberately separate from traversal/node ids, which only describe where
//! a measurement lives in the current plan walk.

use datafusion::common::ScalarValue;

use super::ContainerPathSegment;

/// Stable key for one coordination or sharing group.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CoordinationScopeKey {
    pub(crate) kind: CoordinationKind,
    pub(crate) container_path: Vec<ContainerPathSegment>,
    pub(crate) group: CoordinationGroup,
    pub(crate) channel: Option<CoordinationChannel>,
}

impl CoordinationScopeKey {
    pub(crate) fn new(
        kind: CoordinationKind,
        container_path: Vec<ContainerPathSegment>,
        group: CoordinationGroup,
    ) -> Self {
        Self {
            kind,
            container_path,
            group,
            channel: None,
        }
    }

    pub(crate) fn with_channel(mut self, channel: impl Into<CoordinationChannel>) -> Self {
        self.channel = Some(channel.into());
        self
    }

    pub(crate) fn with_kind(&self, kind: CoordinationKind) -> CoordinationScopeKey {
        Self {
            kind,
            container_path: self.container_path.clone(),
            group: self.group.clone(),
            channel: self.channel.clone(),
        }
    }

    pub(crate) fn partition_path(
        kind: CoordinationKind,
        path: Vec<ScalarValue>,
    ) -> CoordinationScopeKey {
        Self::new(kind, Vec::new(), CoordinationGroup::PartitionPath(path))
    }

    pub(crate) fn container_group(
        kind: CoordinationKind,
        depth: usize,
        identity: impl Into<String>,
    ) -> CoordinationScopeKey {
        Self::new(
            kind,
            Vec::new(),
            CoordinationGroup::ContainerGroup {
                depth,
                identity: identity.into(),
            },
        )
    }

    pub(crate) fn position_path(kind: CoordinationKind, path: Vec<usize>) -> CoordinationScopeKey {
        Self::new(kind, Vec::new(), CoordinationGroup::PositionPath(path))
    }

    pub(crate) fn container_lane(
        kind: CoordinationKind,
        axis: CoordinationAxis,
        root_path: Vec<usize>,
    ) -> CoordinationScopeKey {
        Self::new(
            kind,
            Vec::new(),
            CoordinationGroup::ContainerLane { axis, root_path },
        )
    }

    pub(crate) fn lane_from_scope(
        kind: CoordinationKind,
        base_scope: &CoordinationScopeKey,
        lane_path: Vec<usize>,
    ) -> CoordinationScopeKey {
        Self {
            kind,
            container_path: base_scope.container_path.clone(),
            group: CoordinationGroup::Lane {
                base: Box::new(base_scope.group.clone()),
                lane_path,
            },
            channel: base_scope.channel.clone(),
        }
    }
}

/// The behavior being coordinated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CoordinationKind {
    ScaleDomain,
    OverflowResidual,
    BoundaryResidual,
    GuideAnchor,
    GuideLane,
    GuideOwnership,
    ChildSize,
}

/// The semantic grouping rule within a coordination kind.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CoordinationGroup {
    PartitionPath(Vec<ScalarValue>),
    PositionPath(Vec<usize>),
    ContainerGroup {
        depth: usize,
        identity: String,
    },
    ContainerLane {
        axis: CoordinationAxis,
        root_path: Vec<usize>,
    },
    Lane {
        base: Box<CoordinationGroup>,
        lane_path: Vec<usize>,
    },
}

/// Coordinate or guide channel associated with a coordination key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CoordinationChannel(String);

impl From<String> for CoordinationChannel {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for CoordinationChannel {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// Physical axis associated with a container coordination key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CoordinationAxis {
    Horizontal,
    Vertical,
}

#[cfg(test)]
mod tests {
    use datafusion::common::ScalarValue;

    use super::*;

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    #[test]
    fn partition_path_keys_group_by_kind_channel_and_path() {
        let key_a = CoordinationScopeKey::partition_path(
            CoordinationKind::ScaleDomain,
            vec![s("A"), s("B")],
        )
        .with_channel("x");
        let key_b = CoordinationScopeKey::partition_path(
            CoordinationKind::ScaleDomain,
            vec![s("A"), s("B")],
        )
        .with_channel("x");
        let different_channel = CoordinationScopeKey::partition_path(
            CoordinationKind::ScaleDomain,
            vec![s("A"), s("B")],
        )
        .with_channel("y");

        assert_eq!(key_a, key_b);
        assert_ne!(key_a, different_channel);
    }

    #[test]
    fn container_group_keys_include_kind_depth_and_identity() {
        let overflow_a =
            CoordinationScopeKey::container_group(CoordinationKind::OverflowResidual, 1, "col:a");
        let overflow_b =
            CoordinationScopeKey::container_group(CoordinationKind::OverflowResidual, 1, "col:a");
        let boundary =
            CoordinationScopeKey::container_group(CoordinationKind::BoundaryResidual, 1, "col:a");
        let different_depth =
            CoordinationScopeKey::container_group(CoordinationKind::OverflowResidual, 2, "col:a");

        assert_eq!(overflow_a, overflow_b);
        assert_ne!(overflow_a, boundary);
        assert_ne!(overflow_a, different_depth);
    }

    #[test]
    fn position_path_keys_group_by_kind_and_indices() {
        let owner_a =
            CoordinationScopeKey::position_path(CoordinationKind::GuideOwnership, vec![0, 2]);
        let owner_b =
            CoordinationScopeKey::position_path(CoordinationKind::GuideOwnership, vec![0, 2]);
        let different_kind =
            CoordinationScopeKey::position_path(CoordinationKind::GuideLane, vec![0, 2]);
        let different_path =
            CoordinationScopeKey::position_path(CoordinationKind::GuideOwnership, vec![0, 3]);

        assert_eq!(owner_a, owner_b);
        assert_ne!(owner_a, different_kind);
        assert_ne!(owner_a, different_path);
    }

    #[test]
    fn lane_keys_derive_from_base_scope_and_lane_path() {
        let base =
            CoordinationScopeKey::container_group(CoordinationKind::ChildSize, 2, "row:department");
        let lane_a =
            CoordinationScopeKey::lane_from_scope(CoordinationKind::GuideAnchor, &base, vec![0, 2]);
        let lane_b =
            CoordinationScopeKey::lane_from_scope(CoordinationKind::GuideAnchor, &base, vec![0, 2]);
        let different_lane =
            CoordinationScopeKey::lane_from_scope(CoordinationKind::GuideAnchor, &base, vec![1, 2]);

        assert_eq!(lane_a, lane_b);
        assert_ne!(lane_a, different_lane);
    }
}
