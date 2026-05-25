//! Scope identity for measured child-frame containers.
//!
//! These keys are deliberately layout-neutral. Facet cells, concat children,
//! and future coordinate-positioned subplots can all describe their child
//! frames without forcing generic layout code to understand the producer's
//! domain model.

use datafusion::common::ScalarValue;

use avenger_chart_core::{ChildFrameGuideSharingView, CoordinationAxis, FacetAxis};

/// Stable identity for one measured child frame within a container.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ChildFrameScopeKey {
    pub(crate) container_path: Vec<ContainerPathSegment>,
    pub(crate) child_key: ChildFrameKey,
}

impl ChildFrameScopeKey {
    pub(crate) fn new(container_path: Vec<ContainerPathSegment>, child_key: ChildFrameKey) -> Self {
        Self {
            container_path,
            child_key,
        }
    }
}

/// Identity for a child frame relative to its immediate container.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ChildFrameKey {
    /// Explicit child subplot in a concat-like container.
    ConcatChild { index: usize, key: Option<String> },
    /// Child subplot positioned by a parent coordinate system.
    PositionedSubplot {
        mark_index: usize,
        row_index: usize,
        key: Option<String>,
    },
    /// Child subplot positioned from a partitioned parent-coordinate mark.
    PositionedPartition {
        mark_index: usize,
        value: ScalarValue,
        key: Option<String>,
    },
    /// Facet cell value at one row/column facet level.
    FacetValue {
        axis: FacetAxis,
        level: u8,
        value: ScalarValue,
    },
}

/// One ancestor segment in a child-frame container path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ContainerPathSegment {
    /// Explicit child subplot in a concat-like ancestor container.
    ConcatChild { index: usize, key: Option<String> },
    /// Child subplot positioned by a coordinate-system ancestor.
    PositionedSubplot {
        mark_index: usize,
        row_index: usize,
        key: Option<String>,
    },
    /// Child subplot positioned from a partitioned coordinate-system ancestor.
    PositionedPartition {
        mark_index: usize,
        value: ScalarValue,
        key: Option<String>,
    },
    /// Facet cell value at one row/column facet level.
    FacetValue {
        axis: FacetAxis,
        level: u8,
        value: ScalarValue,
    },
}

impl ContainerPathSegment {
    pub(crate) fn concat_child(index: usize, key: Option<&str>) -> Self {
        Self::ConcatChild {
            index,
            key: key.map(ToOwned::to_owned),
        }
    }

    pub(crate) fn positioned_subplot(
        mark_index: usize,
        row_index: usize,
        key: Option<&str>,
    ) -> Self {
        Self::PositionedSubplot {
            mark_index,
            row_index,
            key: key.map(ToOwned::to_owned),
        }
    }

    pub(crate) fn positioned_partition(
        mark_index: usize,
        value: ScalarValue,
        key: Option<&str>,
    ) -> Self {
        Self::PositionedPartition {
            mark_index,
            value,
            key: key.map(ToOwned::to_owned),
        }
    }

    pub(crate) fn facet_value(axis: FacetAxis, level: u8, value: ScalarValue) -> Self {
        Self::FacetValue { axis, level, value }
    }
}

pub(crate) fn container_path_without_facet_segments(
    path: &[ContainerPathSegment],
) -> Vec<ContainerPathSegment> {
    path.iter()
        .filter(|segment| !matches!(segment, ContainerPathSegment::FacetValue { .. }))
        .cloned()
        .collect()
}

/// One child-frame level in a nested sharing path.
///
/// This is the container-neutral equivalent of a facet position index. Concat
/// containers record child index/count metadata here, and future child-frame
/// containers can do the same without teaching guide ownership code about their
/// concrete coordinate system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChildFrameSharingLevel {
    pub(crate) axis: CoordinationAxis,
    pub(crate) index: usize,
    pub(crate) count: usize,
    pub(crate) segment: ContainerPathSegment,
}

impl ChildFrameSharingLevel {
    pub(crate) fn hconcat_child(index: usize, count: usize, key: Option<&str>) -> Self {
        Self {
            axis: CoordinationAxis::Horizontal,
            index,
            count,
            segment: ContainerPathSegment::concat_child(index, key),
        }
    }

    pub(crate) fn vconcat_child(index: usize, count: usize, key: Option<&str>) -> Self {
        Self {
            axis: CoordinationAxis::Vertical,
            index,
            count,
            segment: ContainerPathSegment::concat_child(index, key),
        }
    }

    pub(crate) fn positioned_subplot(
        index: usize,
        count: usize,
        mark_index: usize,
        row_index: usize,
        key: Option<&str>,
    ) -> Self {
        Self {
            axis: CoordinationAxis::Positioned,
            index,
            count,
            segment: ContainerPathSegment::positioned_subplot(mark_index, row_index, key),
        }
    }

    pub(crate) fn positioned_partition(
        index: usize,
        count: usize,
        mark_index: usize,
        value: ScalarValue,
        key: Option<&str>,
    ) -> Self {
        Self {
            axis: CoordinationAxis::Positioned,
            index,
            count,
            segment: ContainerPathSegment::positioned_partition(mark_index, value, key),
        }
    }
}

/// Nested child-frame sharing path for the currently evaluated child plot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ChildFrameSharingPath {
    levels: Vec<ChildFrameSharingLevel>,
}

impl ChildFrameSharingPath {
    pub(crate) fn root() -> Self {
        Self::default()
    }

    pub(crate) fn levels(&self) -> &[ChildFrameSharingLevel] {
        &self.levels
    }

    pub(crate) fn container_path(&self) -> Vec<ContainerPathSegment> {
        self.levels
            .iter()
            .map(|level| level.segment.clone())
            .collect()
    }

    pub(crate) fn appended(&self, level: ChildFrameSharingLevel) -> Self {
        let mut next = self.clone();
        next.levels.push(level);
        next
    }

    pub(crate) fn position_indices(&self) -> Vec<usize> {
        self.levels.iter().map(|level| level.index).collect()
    }

    pub(crate) fn level_counts(&self) -> Vec<usize> {
        self.levels.iter().map(|level| level.count).collect()
    }

    pub(crate) fn level_axes(&self) -> Vec<CoordinationAxis> {
        self.levels.iter().map(|level| level.axis).collect()
    }

    #[cfg(test)]
    pub(crate) fn has_axis(&self, axis: CoordinationAxis) -> bool {
        self.levels.iter().any(|level| level.axis == axis)
    }
}

impl ChildFrameGuideSharingView for ChildFrameSharingPath {
    fn position_indices(&self) -> Vec<usize> {
        self.position_indices()
    }

    fn level_counts(&self) -> Vec<usize> {
        self.level_counts()
    }

    fn level_axes(&self) -> Vec<CoordinationAxis> {
        self.level_axes()
    }

    fn relevant_depth(&self, axis: CoordinationAxis) -> usize {
        self.levels
            .iter()
            .filter(|level| level.axis == axis)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharing_path_root_is_empty() {
        let path = ChildFrameSharingPath::root();
        assert!(path.levels().is_empty());
        assert!(path.container_path().is_empty());
        assert!(path.position_indices().is_empty());
        assert!(path.level_counts().is_empty());
        assert!(path.level_axes().is_empty());
    }

    #[test]
    fn sharing_path_tracks_horizontal_concat_child() {
        let path = ChildFrameSharingPath::root().appended(ChildFrameSharingLevel::hconcat_child(
            1,
            3,
            Some("right"),
        ));

        assert_eq!(path.position_indices(), vec![1]);
        assert_eq!(path.level_counts(), vec![3]);
        assert_eq!(path.level_axes(), vec![CoordinationAxis::Horizontal]);
        assert!(path.has_axis(CoordinationAxis::Horizontal));
        assert!(!path.has_axis(CoordinationAxis::Vertical));
        assert_eq!(
            path.container_path(),
            vec![ContainerPathSegment::concat_child(1, Some("right"))]
        );
    }

    #[test]
    fn sharing_path_tracks_nested_concat_levels_in_order() {
        let path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::hconcat_child(0, 2, Some("left")))
            .appended(ChildFrameSharingLevel::vconcat_child(1, 4, Some("bottom")));

        assert_eq!(path.position_indices(), vec![0, 1]);
        assert_eq!(path.level_counts(), vec![2, 4]);
        assert_eq!(
            path.level_axes(),
            vec![CoordinationAxis::Horizontal, CoordinationAxis::Vertical]
        );
        assert_eq!(
            path.container_path(),
            vec![
                ContainerPathSegment::concat_child(0, Some("left")),
                ContainerPathSegment::concat_child(1, Some("bottom")),
            ]
        );
    }

    #[test]
    fn container_path_without_facet_segments_keeps_concat_identity() {
        let path = vec![
            ContainerPathSegment::concat_child(0, Some("outer")),
            ContainerPathSegment::facet_value(
                avenger_chart_core::FacetAxis::Row,
                0,
                ScalarValue::Utf8(Some("A".to_string())),
            ),
            ContainerPathSegment::concat_child(1, Some("inner")),
        ];

        assert_eq!(
            container_path_without_facet_segments(&path),
            vec![
                ContainerPathSegment::concat_child(0, Some("outer")),
                ContainerPathSegment::concat_child(1, Some("inner")),
            ]
        );
    }
}
