//! Scope identity for measured child-frame containers.
//!
//! These keys are deliberately layout-neutral. Facet cells, concat children,
//! and future coordinate-positioned subplots can all describe their child
//! frames without forcing generic layout code to understand the producer's
//! domain model.

use datafusion::common::ScalarValue;

use crate::coords::FacetAxis;

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

    pub(crate) fn facet_value(axis: FacetAxis, level: u8, value: ScalarValue) -> Self {
        Self::FacetValue { axis, level, value }
    }
}
