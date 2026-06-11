//! The durable artifact of one facet coordination run.
//!
//! Each coordination round produces one [`CoordinationSolution`]: the
//! post-adjustment per-node channel values (the free-slot, lane-gap, and
//! global-edge adjustments are solution properties applied at
//! construction) plus the per-key aggregates and round deltas the driver
//! logs. Bands hold an `Arc` handle to the round they last participated
//! in and read their coordinated values as views into it, falling back to
//! local values pre-coordination — replacing the four per-band mutable
//! stores ("same stores, new producer" was the previous campaign's
//! contract; this artifact dissolves the stores themselves).
//!
//! A band rebuilt during retargeting briefly has no handle until the next
//! round installs one; every read path keeps the local fallback, exactly
//! as the stores' `Option` semantics worked.

use std::collections::HashMap;

use avenger_chart_core::{CoordinatedLayout, CoordinatedOverflow};

use crate::facet::coordination_plans::CoordinationNodeKey;

/// Solved coordination values for one round, keyed by traversal node.
#[derive(Debug, Clone, Default)]
pub(crate) struct CoordinationSolution {
    pub(crate) layout_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
    pub(crate) overflow_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    pub(crate) guide_anchor_overflow_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    pub(crate) boundary_overflow_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
}

impl CoordinationSolution {
    /// Build the solution from one requirement pass (transitional: the
    /// pass structs collapse into this artifact when the stores dissolve).
    pub(crate) fn from_pass(
        pass: &crate::facet::coordination_plans::RequirementPass,
    ) -> CoordinationSolution {
        CoordinationSolution {
            layout_by_node: pass.distribution.layout_patches_by_node.clone(),
            overflow_by_node: pass.distribution.overflow_patches_by_node.clone(),
            guide_anchor_overflow_by_node: pass
                .distribution
                .guide_anchor_overflow_patches_by_node
                .clone(),
            boundary_overflow_by_node: pass.distribution.boundary_overflow_patches_by_node.clone(),
        }
    }

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
