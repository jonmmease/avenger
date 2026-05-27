use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use datafusion::common::ScalarValue;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;

use crate::{
    facet::{
        coord::{FacetBandCoordMeasurement, facet_band_ref},
        evaluated_facet_tree::EvaluatedFacetTree,
    },
    plot::compiled::{
        CompiledPlot, ComponentsMeasurement, PlotComponents,
        session::plot_dependency_param_fingerprint,
    },
};

#[derive(Clone)]
pub(crate) struct LayoutProfileSnapshot {
    pub(crate) measurement: ComponentsMeasurement,
    pub(crate) physical_facet_tree_structure: Option<Vec<String>>,
    pub(crate) logical_facet_tree_structure: Option<Vec<String>>,
    pub(crate) facet_cell_measurements: FacetCellMeasurementProfileIndex,
    pub(crate) facet_cell_rendered_components: FacetCellRenderedComponentsProfileIndex,
    pub(crate) rendered_components: Option<PlotComponents>,
}

impl LayoutProfileSnapshot {
    pub(crate) fn new_with_components(
        measurement: ComponentsMeasurement,
        facet_tree: Option<&EvaluatedFacetTree>,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        rendered_components: Option<PlotComponents>,
        facet_cell_rendered_components: FacetCellRenderedComponentsProfileIndex,
    ) -> Self {
        let facet_cell_measurements = facet_tree
            .map(|tree| {
                FacetCellMeasurementProfileIndex::from_measurement(&measurement, tree, ctx, params)
            })
            .unwrap_or_default();
        Self {
            measurement,
            physical_facet_tree_structure: facet_tree.map(EvaluatedFacetTree::structure_cache_key),
            logical_facet_tree_structure: facet_tree
                .map(EvaluatedFacetTree::logical_structure_cache_key),
            facet_cell_measurements,
            facet_cell_rendered_components,
            rendered_components,
        }
    }

    pub(crate) fn physical_structure_matches(&self, facet_tree: &EvaluatedFacetTree) -> bool {
        self.physical_facet_tree_structure
            .as_deref()
            .is_some_and(|expected| expected == facet_tree.structure_cache_key().as_slice())
    }

    pub(crate) fn logical_structure_matches(&self, facet_tree: &EvaluatedFacetTree) -> bool {
        self.logical_facet_tree_structure
            .as_deref()
            .is_some_and(|expected| expected == facet_tree.logical_structure_cache_key().as_slice())
    }

    pub(crate) fn facet_cell_measurement(
        &self,
        facet_tree: &EvaluatedFacetTree,
        full_path: &[ScalarValue],
        compiled_subplot: &CompiledPlot,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Option<ComponentsMeasurement> {
        let logical_cell_path = facet_tree.logical_cell_key_for_path(full_path)?;
        let dependency_params = plot_dependency_param_fingerprint(compiled_subplot, ctx, params);
        let key = FacetCellMeasurementProfileKey::new(
            logical_cell_path,
            compiled_subplot as *const _ as usize,
            dependency_params,
        );
        self.facet_cell_measurements.get(&key)
    }

    pub(crate) fn facet_cell_profile_count(&self) -> usize {
        self.facet_cell_measurements.len()
    }

    pub(crate) fn facet_cell_rendered_components(
        &self,
        facet_tree: &EvaluatedFacetTree,
        full_path: &[ScalarValue],
        compiled_subplot: &CompiledPlot,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Option<PlotComponents> {
        let key = facet_cell_profile_key(facet_tree, full_path, compiled_subplot, ctx, params)?;
        self.facet_cell_rendered_components.get(&key)
    }
}

pub(crate) type FacetCellRenderedComponentsProfileCapture =
    Arc<Mutex<FacetCellRenderedComponentsProfileIndex>>;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct FacetCellMeasurementProfileKey {
    logical_cell_path: Vec<String>,
    compiled_subplot_ptr: usize,
    dependency_params: Vec<(String, String)>,
}

impl FacetCellMeasurementProfileKey {
    pub(crate) fn new(
        logical_cell_path: Vec<String>,
        compiled_subplot_ptr: usize,
        dependency_params: Vec<(String, String)>,
    ) -> Self {
        Self {
            logical_cell_path,
            compiled_subplot_ptr,
            dependency_params,
        }
    }
}

fn facet_cell_profile_key(
    facet_tree: &EvaluatedFacetTree,
    full_path: &[ScalarValue],
    compiled_subplot: &CompiledPlot,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Option<FacetCellMeasurementProfileKey> {
    let logical_cell_path = facet_tree.logical_cell_key_for_path(full_path)?;
    let dependency_params = plot_dependency_param_fingerprint(compiled_subplot, ctx, params);
    Some(FacetCellMeasurementProfileKey::new(
        logical_cell_path,
        compiled_subplot as *const _ as usize,
        dependency_params,
    ))
}

#[derive(Clone, Default)]
pub(crate) struct FacetCellMeasurementProfileIndex {
    measurements: HashMap<FacetCellMeasurementProfileKey, ComponentsMeasurement>,
}

impl FacetCellMeasurementProfileIndex {
    pub(crate) fn get(
        &self,
        key: &FacetCellMeasurementProfileKey,
    ) -> Option<ComponentsMeasurement> {
        self.measurements.get(key).cloned()
    }

    pub(crate) fn len(&self) -> usize {
        self.measurements.len()
    }

    fn from_measurement(
        measurement: &ComponentsMeasurement,
        facet_tree: &EvaluatedFacetTree,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Self {
        let mut index = Self::default();
        Self::collect_from_measurement(measurement, facet_tree, ctx, params, &mut index);
        index
    }

    fn collect_from_measurement(
        measurement: &ComponentsMeasurement,
        facet_tree: &EvaluatedFacetTree,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        index: &mut Self,
    ) {
        let Some(facet_band) = facet_band_ref(measurement.coord_measurement.as_ref()) else {
            return;
        };
        Self::collect_from_facet_band(facet_band, facet_tree, ctx, params, index);
    }

    fn collect_from_facet_band(
        facet_band: &FacetBandCoordMeasurement,
        facet_tree: &EvaluatedFacetTree,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        index: &mut Self,
    ) {
        let compiled_subplot_ptr = facet_band.compiled_subplot.as_ref() as *const _ as usize;
        let dependency_params =
            plot_dependency_param_fingerprint(facet_band.compiled_subplot.as_ref(), ctx, params);
        for cell in &facet_band.cells {
            if facet_band_ref(cell.measurement.coord_measurement.as_ref()).is_some() {
                Self::collect_from_measurement(&cell.measurement, facet_tree, ctx, params, index);
                continue;
            }
            let Some(logical_cell_path) =
                facet_tree.logical_cell_key_for_path(&cell.plan.full_path)
            else {
                continue;
            };
            let key = FacetCellMeasurementProfileKey::new(
                logical_cell_path,
                compiled_subplot_ptr,
                dependency_params.clone(),
            );
            index.measurements.insert(key, cell.measurement.clone());
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct FacetCellRenderedComponentsProfileIndex {
    components: HashMap<FacetCellMeasurementProfileKey, PlotComponents>,
}

impl FacetCellRenderedComponentsProfileIndex {
    pub(crate) fn get(&self, key: &FacetCellMeasurementProfileKey) -> Option<PlotComponents> {
        self.components.get(key).cloned()
    }

    pub(crate) fn insert_for_cell(
        &mut self,
        facet_tree: &EvaluatedFacetTree,
        full_path: &[ScalarValue],
        compiled_subplot: &CompiledPlot,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        components: PlotComponents,
    ) {
        if let Some(key) =
            facet_cell_profile_key(facet_tree, full_path, compiled_subplot, ctx, params)
        {
            self.components.insert(key, components);
        }
    }
}
