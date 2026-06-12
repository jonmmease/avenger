//! Edge declarations and solved edge values.
//!
//! Two types span the demand/grant split:
//!
//! - [`EdgeDemand`] is what callers *declare* on a leaf: a layered pair
//!   (interior `inner` chrome plus `outer` content stacking beyond it).
//!   There is no lift and no stored total — the granted total is always
//!   the layer sum, and node-attached space is declared as chrome
//!   instead.
//! - [`EdgeGrant`] is what the solver *produces*: requested/granted region
//!   edges, envelope sides, grid track edge vectors. Grants carry
//!   `(inner, outer, total)` under the lift law `total >= inner + outer`.
//!
//! The lift law is the merge law's companion. Layered grants merge by
//! component-wise max, and a merged side must hold every member's layers
//! at once: merging `(4, 11, 15)` with `(9, 3, 22)` must give
//! `(9, 11, 22)` — the merged total (22) exceeds the merged layer sum
//! (20), so the total is stored, not derived; and where a stored total
//! falls below the layer sum, construction lifts it so independently
//! coordinated layers stay representable.

use crate::geometry::Edges;

/// A caller's declared per-side overflow on a leaf.
///
/// This is the *declaration* type consumed by
/// [`Layout::demand`](crate::build::Layout::demand); the solver's outputs
/// (requested/granted edges, envelopes) are [`EdgeGrant`]. A demand is
/// always layered: interior chrome (`inner`, e.g. guides) plus content
/// stacking beyond it (`outer`, e.g. legends); the granted total is
/// `inner + outer`. Extent-only clearance asks put their value in one
/// stratum and zero in the other. Total-only space exists only as
/// [`EdgeGrant`] values constructed directly (e.g. exported requirement
/// folds) — a solve never produces a total beyond the layer sum.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EdgeDemand {
    pub inner: f32,
    pub outer: f32,
}

impl EdgeDemand {
    /// The residual declaration pattern: a known interior layer within a
    /// measured envelope. `outer` is the non-negative remainder, so the
    /// granted total is `max(envelope, inner)`.
    pub fn from_inner_and_envelope(inner: f32, envelope: f32) -> Self {
        let inner = inner.max(0.0);
        Self {
            inner,
            outer: (envelope - inner).max(0.0),
        }
    }
}

/// A solved per-side edge value: the solver's output space for demands that
/// have been lifted, merged, or allocated (`Region.requested/granted`,
/// envelope edges, grid track edge vectors).
///
/// `inner` is interior chrome between the content rectangle and any outer
/// content; `outer` is content that stacks beyond the inner edge; `total` is
/// the full rendered envelope.
///
/// `new` LIFTS the total to `max(total, inner + outer, 0)`. This is an
/// intentional law, not input validation: when layered grants merge by
/// component-wise max, the lift makes a merged side's total equal
/// `max(inner) + max(outer)` — the space a region occupies once each layer
/// has been coordinated independently. Callers that need raw, unlifted
/// totals should compute envelopes with the geometric tree-envelope kind
/// instead of layering.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EdgeGrant {
    pub inner: f32,
    pub outer: f32,
    pub total: f32,
}

impl EdgeGrant {
    pub fn new(inner: f32, outer: f32, total: f32) -> Self {
        let inner = inner.max(0.0);
        let outer = outer.max(0.0);
        let total = total.max(inner + outer).max(0.0);
        Self {
            inner,
            outer,
            total,
        }
    }

    /// A grant with no layer structure: `total` only.
    pub fn total_only(total: f32) -> Self {
        Self::new(0.0, 0.0, total)
    }

    pub fn max_components(self, other: Self) -> Self {
        Self::new(
            self.inner.max(other.inner),
            self.outer.max(other.outer),
            self.total.max(other.total),
        )
    }
}

impl From<EdgeDemand> for EdgeGrant {
    fn from(demand: EdgeDemand) -> Self {
        Self::new(demand.inner, demand.outer, demand.inner + demand.outer)
    }
}

impl Edges<EdgeGrant> {
    /// Component-wise [`EdgeGrant::max_components`] on every side.
    ///
    /// This is the neutral merge law for per-side layered demand (for
    /// charts: guide overflow as `inner`, legend overflow as `outer`).
    pub fn max_components(self, other: Self) -> Self {
        Edges {
            top: self.top.max_components(other.top),
            right: self.right.max_components(other.right),
            bottom: self.bottom.max_components(other.bottom),
            left: self.left.max_components(other.left),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_grant_new_lifts_total_to_layer_sum() {
        // total 2.0 < inner 4.0 + outer 11.0: the constructor lifts it so
        // independently coordinated layers stay representable.
        assert_eq!(
            EdgeGrant::new(4.0, 11.0, 2.0),
            EdgeGrant {
                inner: 4.0,
                outer: 11.0,
                total: 15.0
            }
        );
    }

    #[test]
    fn edge_grant_merges_structured_components() {
        let left = EdgeGrant::new(4.0, 11.0, 2.0);
        assert_eq!(
            left,
            EdgeGrant {
                inner: 4.0,
                outer: 11.0,
                total: 15.0
            }
        );

        let right = EdgeGrant::new(9.0, 3.0, 22.0);
        assert_eq!(
            left.max_components(right),
            EdgeGrant {
                inner: 9.0,
                outer: 11.0,
                total: 22.0
            }
        );
    }
}
