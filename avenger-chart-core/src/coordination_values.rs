use crate::OverflowSpaceRequirement;

/// Coordinated overflow values aggregated across all facets at the same nesting level.
///
/// This enables facet labels at the same nesting depth to be horizontally aligned
/// regardless of their parent facet's individual overflow requirements.
///
/// Contains both guide-only overflow (for label positioning) and total overflow
/// (including legends, for consistent spacing across facet cells).
#[derive(Default, Clone, Debug)]
pub struct CoordinatedOverflow {
    /// Overflow from guides only (axes, labels, ticks).
    /// Used for facet label positioning.
    pub guide: OverflowSpaceRequirement,

    /// Total overflow including legends.
    /// Used for consistent legend spacing across facet cells.
    pub total: OverflowSpaceRequirement,
}

impl CoordinatedOverflow {
    /// Merge with another overflow context, keeping the maximum of each field.
    pub fn merge(&mut self, other: &Self) {
        let legend_top = (self.total.top - self.guide.top)
            .max(0.0)
            .max((other.total.top - other.guide.top).max(0.0));
        let legend_right = (self.total.right - self.guide.right)
            .max(0.0)
            .max((other.total.right - other.guide.right).max(0.0));
        let legend_bottom = (self.total.bottom - self.guide.bottom)
            .max(0.0)
            .max((other.total.bottom - other.guide.bottom).max(0.0));
        let legend_left = (self.total.left - self.guide.left)
            .max(0.0)
            .max((other.total.left - other.guide.left).max(0.0));

        self.guide.top = self.guide.top.max(other.guide.top);
        self.guide.bottom = self.guide.bottom.max(other.guide.bottom);
        self.guide.left = self.guide.left.max(other.guide.left);
        self.guide.right = self.guide.right.max(other.guide.right);

        self.total.top = self.guide.top + legend_top;
        self.total.right = self.guide.right + legend_right;
        self.total.bottom = self.guide.bottom + legend_bottom;
        self.total.left = self.guide.left + legend_left;
    }
}

/// Layout parameters coordinated across all facet-band nodes at the same depth.
///
/// Ensures subplot widths and gaps are consistent across all branches at each
/// nesting depth, even when branches have different overflow patterns or cell counts.
#[derive(Default, Clone, Debug)]
pub struct CoordinatedLayout {
    /// Physical gap between adjacent subplot plot areas.
    ///
    /// This value is written to band scales and explicit plot-area placement.
    pub padding_inner_px: f32,
    /// Virtual same-axis guide slot gap used for facet-guide alignment.
    ///
    /// This may be larger than `padding_inner_px` in deeply nested same-axis
    /// facet chains, where labels and guide titles need a common slot model but
    /// subplot plot areas should not inherit that extra guide-only space.
    pub guide_slot_gap_px: f32,
    pub outer_start: f32,
    pub outer_end: f32,
    pub n: usize,
}

impl CoordinatedLayout {
    pub fn merge(&mut self, other: &CoordinatedLayout) {
        self.padding_inner_px = self.padding_inner_px.max(other.padding_inner_px);
        self.guide_slot_gap_px = self.guide_slot_gap_px.max(other.guide_slot_gap_px);
        self.outer_start = self.outer_start.max(other.outer_start);
        self.outer_end = self.outer_end.max(other.outer_end);
        self.n = self.n.max(other.n);
    }
}
