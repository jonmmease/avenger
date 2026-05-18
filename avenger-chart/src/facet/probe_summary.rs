use crate::{coords::OverflowSpaceRequirement, facet::overflow_projection::FacetBoundaryProfiles};

#[derive(Clone, Debug)]
pub(crate) struct FacetCellProbeSummary {
    pub(crate) guide_overflow: OverflowSpaceRequirement,
    pub(crate) total_overflow: OverflowSpaceRequirement,
    pub(crate) boundary_profiles: FacetBoundaryProfiles,
    pub(crate) max_child_padding: f32,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct FacetBandProbeLayout {
    pub(crate) cell_probe_summary: FacetCellProbeSummary,
}
