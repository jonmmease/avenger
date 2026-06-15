//! The durable artifact of one facet coordination run.
//!
//! Each coordination round produces one [`CoordinationSolution`]: the
//! per-node channel values, with the lane-gap fold and global-edge outer
//! reversion applied at construction (slot counts arrive pre-folded —
//! the free-slot law lives in the pre-solve fold, `compute_band_folds`).
//! Bands hold an `Arc` handle to the round they last participated in and
//! read their coordinated values as views into it, falling back to local
//! values pre-coordination.
//!
//! A freshly (re)built band briefly has no handle until the next round
//! installs one; every read path keeps the local fallback.

use std::collections::HashMap;

use avenger_chart_core::{CoordinatedLayout, CoordinatedOverflow};

use crate::facet::coordination_plans::CoordinationNodeKey;

/// Solved coordination channel values for one round, keyed by traversal node.
#[derive(Debug, Clone, Default)]
pub(crate) struct CoordinationSolution {
    pub(crate) layout_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
    pub(crate) overflow_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    pub(crate) guide_anchor_overflow_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    pub(crate) boundary_overflow_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
}

impl CoordinationSolution {
    pub(crate) fn layout(&self, node: &CoordinationNodeKey) -> Option<&CoordinatedLayout> {
        self.layout_by_node.get(node)
    }

    pub(crate) fn overflow(&self, node: &CoordinationNodeKey) -> Option<&CoordinatedOverflow> {
        self.overflow_by_node.get(node)
    }

    pub(crate) fn guide_anchor_overflow(
        &self,
        node: &CoordinationNodeKey,
    ) -> Option<&CoordinatedOverflow> {
        self.guide_anchor_overflow_by_node.get(node)
    }

    pub(crate) fn boundary_overflow(
        &self,
        node: &CoordinationNodeKey,
    ) -> Option<&CoordinatedOverflow> {
        self.boundary_overflow_by_node.get(node)
    }
}
