//! Edge demand layering.

use crate::geometry::Edges;

/// A caller's declared per-side overflow on a leaf.
///
/// This is the *declaration* type consumed by
/// [`Layout::demand`](crate::build::Layout::demand); the solver's outputs
/// (requested/granted edges, envelopes) are [`EdgeGrant`]. A side is either
/// layered (interior chrome plus content stacking beyond it) or a bare
/// total — mixing layered and unlayered space on one side is
/// unrepresentable by design. Node-attached unlayered space is declared as
/// chrome, not as a demand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EdgeDemand {
    /// Interior chrome (`inner`, e.g. guides) plus content stacking beyond
    /// it (`outer`, e.g. legends). The granted total is `inner + outer`.
    Layered { inner: f32, outer: f32 },
    /// A total-only envelope with no layer structure.
    Unlayered(f32),
}

impl EdgeDemand {
    /// The residual declaration pattern: a known interior layer within a
    /// measured envelope. `outer` is the non-negative remainder, so the
    /// granted total is `max(envelope, inner)`.
    pub fn from_inner_and_envelope(inner: f32, envelope: f32) -> Self {
        let inner = inner.max(0.0);
        Self::Layered {
            inner,
            outer: (envelope - inner).max(0.0),
        }
    }
}

impl Default for EdgeDemand {
    fn default() -> Self {
        Self::Unlayered(0.0)
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
        match demand {
            EdgeDemand::Layered { inner, outer } => Self::new(inner, outer, inner + outer),
            EdgeDemand::Unlayered(total) => Self::total_only(total),
        }
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

/// Coordinated edge targets granted to a child region by its parent.
///
/// `inner` is the interior edge between the content rectangle and any outer
/// content. `total` is the full rendered edge envelope. The difference is
/// important for local outer-content anchoring: outer content should start
/// after the coordinated inner edge, while sibling spacing uses the
/// coordinated total edge.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EdgeTargets {
    pub inner: Edges<f32>,
    pub total: Edges<f32>,
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
