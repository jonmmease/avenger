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
