//! Facet-band access shims for the coordination driver: typed ref/mut
//! views over the band coord measurement, shared by the snapshot,
//! install, and adopt walks.

use crate::{
    facet::coord::{
        FacetBandCoordMeasurement, facet_band_mut as facet_band_mut_from_coord,
        facet_band_ref as facet_band_ref_from_coord,
    },
    plot::compiled::ComponentsMeasurement,
};

#[derive(Clone, Copy)]
pub(crate) struct FacetBandRef<'a> {
    base: &'a FacetBandCoordMeasurement,
}

impl<'a> FacetBandRef<'a> {
    pub(crate) fn new(base: &'a FacetBandCoordMeasurement) -> Self {
        Self { base }
    }

    pub(crate) fn base(self) -> &'a FacetBandCoordMeasurement {
        self.base
    }
}

pub(crate) struct FacetBandMut<'a> {
    base: &'a mut FacetBandCoordMeasurement,
}

impl<'a> FacetBandMut<'a> {
    pub(crate) fn new(base: &'a mut FacetBandCoordMeasurement) -> Self {
        Self { base }
    }

    pub(crate) fn base_mut(&mut self) -> &mut FacetBandCoordMeasurement {
        self.base
    }
}

pub(crate) struct FacetCoordinationPolicy;

impl FacetCoordinationPolicy {
    pub(crate) const LABEL: &'static str = "facet-coordination-policy";

    pub(crate) fn facet_band_ref(measurement: &ComponentsMeasurement) -> Option<FacetBandRef<'_>> {
        facet_band_ref_from_coord(measurement.coord_measurement.as_ref()).map(FacetBandRef::new)
    }

    pub(crate) fn facet_band_mut(
        measurement: &mut ComponentsMeasurement,
    ) -> Option<FacetBandMut<'_>> {
        facet_band_mut_from_coord(measurement.coord_measurement.as_mut()).map(FacetBandMut::new)
    }
}
