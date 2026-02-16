//! Explicit layout slab decomposition for facet overflow.
//!
//! Converts coordinated overflow into guide/legend/total slabs so callers can
//! reason about reservation and anchoring using explicit components.

use crate::coords::{CoordinatedOverflow, OverflowSpaceRequirement};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LayoutSlabs {
    pub guide: OverflowSpaceRequirement,
    pub legend: OverflowSpaceRequirement,
}

impl LayoutSlabs {
    pub(crate) fn from_coordinated(overflow: &CoordinatedOverflow) -> Self {
        let legend = OverflowSpaceRequirement {
            top: (overflow.total.top - overflow.guide.top).max(0.0),
            right: (overflow.total.right - overflow.guide.right).max(0.0),
            bottom: (overflow.total.bottom - overflow.guide.bottom).max(0.0),
            left: (overflow.total.left - overflow.guide.left).max(0.0),
        };
        Self {
            guide: overflow.guide.clone(),
            legend,
        }
    }

    #[inline]
    pub(crate) fn guide_anchor(&self, place_at_bottom: bool) -> f32 {
        if place_at_bottom {
            self.guide.bottom
        } else {
            self.guide.top
        }
    }

    #[inline]
    pub(crate) fn legend_vertical(&self) -> (f32, f32) {
        (self.legend.top, self.legend.bottom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::CoordinatedOverflow;

    #[test]
    fn from_coordinated_extracts_non_negative_legend_slab() {
        let overflow = CoordinatedOverflow {
            guide: OverflowSpaceRequirement {
                top: 12.0,
                right: 9.0,
                bottom: 5.0,
                left: 3.0,
            },
            total: OverflowSpaceRequirement {
                top: 20.0,
                right: 10.0,
                bottom: 8.0,
                left: 3.0,
            },
        };
        let slabs = LayoutSlabs::from_coordinated(&overflow);
        assert_eq!(slabs.legend.top, 8.0);
        assert_eq!(slabs.legend.right, 1.0);
        assert_eq!(slabs.legend.bottom, 3.0);
        assert_eq!(slabs.legend.left, 0.0);
    }
}
