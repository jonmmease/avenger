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
        session::{SelectionRevisionFingerprint, plot_profile_dependency_param_fingerprint},
    },
};

#[derive(Clone)]
pub(crate) struct LayoutProfileSnapshot {
    pub(crate) measurement: ComponentsMeasurement,
    pub(crate) facet_tree: Option<Arc<EvaluatedFacetTree>>,
    pub(crate) physical_facet_tree_structure: Option<Vec<String>>,
    pub(crate) logical_facet_tree_structure: Option<Vec<String>>,
    pub(crate) profile_dependency_params: Vec<(String, String)>,
    pub(crate) selection_revision_fingerprint: SelectionRevisionFingerprint,
    pub(crate) facet_cell_profiles: FacetCellProfileIndex,
    pub(crate) rendered_components: Option<PlotComponents>,
}

impl LayoutProfileSnapshot {
    pub(crate) fn new_with_components(
        compiled_plot: &CompiledPlot,
        measurement: ComponentsMeasurement,
        facet_tree: Option<Arc<EvaluatedFacetTree>>,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        selection_revision_fingerprint: SelectionRevisionFingerprint,
        rendered_components: Option<PlotComponents>,
        mut facet_cell_profiles: FacetCellProfileIndex,
    ) -> Self {
        if let Some(tree) = facet_tree.as_ref() {
            facet_cell_profiles.collect_measurements_from_measurement(
                &measurement,
                tree,
                ctx,
                params,
            );
        }
        Self {
            measurement,
            facet_tree: facet_tree.clone(),
            physical_facet_tree_structure: facet_tree
                .as_ref()
                .map(|tree| tree.structure_cache_key()),
            logical_facet_tree_structure: facet_tree
                .as_ref()
                .map(|tree| tree.logical_structure_cache_key()),
            profile_dependency_params: plot_profile_dependency_param_fingerprint(
                compiled_plot,
                ctx,
                params,
            ),
            selection_revision_fingerprint,
            facet_cell_profiles,
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

    pub(crate) fn profile_dependencies_match(
        &self,
        params: &IndexMap<String, ScalarValue>,
    ) -> bool {
        self.profile_dependency_params.iter().all(|(name, value)| {
            let current = params
                .get(name)
                .or_else(|| params.get(&format!("${name}")))
                .map(|value| format!("{value:?}"))
                .unwrap_or_else(|| "<missing>".to_string());
            &current == value
        })
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
        let dependency_params =
            plot_profile_dependency_param_fingerprint(compiled_subplot, ctx, params);
        let key = FacetCellProfileKey::measurement(
            logical_cell_path,
            compiled_subplot as *const _ as usize,
            dependency_params,
        );
        self.facet_cell_profiles.measurement(&key)
    }

    pub(crate) fn facet_cell_profile_count(&self) -> usize {
        self.facet_cell_profiles.measurement_count()
    }

    pub(crate) fn facet_cell_rendered_components(
        &self,
        facet_tree: &EvaluatedFacetTree,
        full_path: &[ScalarValue],
        compiled_subplot: &CompiledPlot,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        selection_revision_fingerprint: SelectionRevisionFingerprint,
    ) -> Option<PlotComponents> {
        let key = facet_cell_rendered_components_profile_key(
            facet_tree,
            full_path,
            compiled_subplot,
            ctx,
            params,
            selection_revision_fingerprint,
        )?;
        self.facet_cell_profiles.rendered_components(&key)
    }
}

pub(crate) type FacetCellRenderedComponentsProfileCapture = Arc<Mutex<FacetCellProfileIndex>>;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct FacetCellProfileKey {
    logical_cell_path: Vec<String>,
    compiled_subplot_ptr: usize,
    dependency_params: Vec<(String, String)>,
    selection_revision_fingerprint: SelectionRevisionFingerprint,
}

impl FacetCellProfileKey {
    pub(crate) fn measurement(
        logical_cell_path: Vec<String>,
        compiled_subplot_ptr: usize,
        dependency_params: Vec<(String, String)>,
    ) -> Self {
        Self {
            logical_cell_path,
            compiled_subplot_ptr,
            dependency_params,
            selection_revision_fingerprint: Vec::new(),
        }
    }

    pub(crate) fn rendered_components(
        logical_cell_path: Vec<String>,
        compiled_subplot_ptr: usize,
        dependency_params: Vec<(String, String)>,
        selection_revision_fingerprint: SelectionRevisionFingerprint,
    ) -> Self {
        Self {
            logical_cell_path,
            compiled_subplot_ptr,
            dependency_params,
            selection_revision_fingerprint,
        }
    }
}

fn facet_cell_rendered_components_profile_key(
    facet_tree: &EvaluatedFacetTree,
    full_path: &[ScalarValue],
    compiled_subplot: &CompiledPlot,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    selection_revision_fingerprint: SelectionRevisionFingerprint,
) -> Option<FacetCellProfileKey> {
    let logical_cell_path = facet_tree.logical_cell_key_for_path(full_path)?;
    let dependency_params =
        plot_profile_dependency_param_fingerprint(compiled_subplot, ctx, params);
    Some(FacetCellProfileKey::rendered_components(
        logical_cell_path,
        compiled_subplot as *const _ as usize,
        dependency_params,
        selection_revision_fingerprint,
    ))
}

#[derive(Clone, Default)]
struct FacetCellProfile {
    measurement: Option<ComponentsMeasurement>,
    rendered_components: Option<PlotComponents>,
}

#[derive(Clone, Default)]
pub(crate) struct FacetCellProfileIndex {
    profiles: HashMap<FacetCellProfileKey, FacetCellProfile>,
}

impl FacetCellProfileIndex {
    pub(crate) fn measurement(&self, key: &FacetCellProfileKey) -> Option<ComponentsMeasurement> {
        self.profiles
            .get(key)
            .and_then(|profile| profile.measurement.clone())
    }

    pub(crate) fn rendered_components(&self, key: &FacetCellProfileKey) -> Option<PlotComponents> {
        self.profiles
            .get(key)
            .and_then(|profile| profile.rendered_components.clone())
    }

    pub(crate) fn measurement_count(&self) -> usize {
        self.profiles
            .values()
            .filter(|profile| profile.measurement.is_some())
            .count()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    fn collect_measurements_from_measurement(
        &mut self,
        measurement: &ComponentsMeasurement,
        facet_tree: &EvaluatedFacetTree,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) {
        let Some(facet_band) = facet_band_ref(measurement.coord_measurement.as_ref()) else {
            return;
        };
        self.collect_measurements_from_facet_band(facet_band, facet_tree, ctx, params);
    }

    fn collect_measurements_from_facet_band(
        &mut self,
        facet_band: &FacetBandCoordMeasurement,
        facet_tree: &EvaluatedFacetTree,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) {
        let compiled_subplot_ptr = facet_band.compiled_subplot.as_ref() as *const _ as usize;
        let dependency_params = plot_profile_dependency_param_fingerprint(
            facet_band.compiled_subplot.as_ref(),
            ctx,
            params,
        );
        for cell in &facet_band.cells {
            if facet_band_ref(cell.measurement.coord_measurement.as_ref()).is_some() {
                self.collect_measurements_from_measurement(
                    &cell.measurement,
                    facet_tree,
                    ctx,
                    params,
                );
                continue;
            }
            let Some(logical_cell_path) =
                facet_tree.logical_cell_key_for_path(&cell.plan.full_path)
            else {
                continue;
            };
            let key = FacetCellProfileKey::measurement(
                logical_cell_path,
                compiled_subplot_ptr,
                dependency_params.clone(),
            );
            self.profiles.entry(key).or_default().measurement = Some(cell.measurement.clone());
        }
    }

    pub(crate) fn insert_for_cell(
        &mut self,
        facet_tree: &EvaluatedFacetTree,
        full_path: &[ScalarValue],
        compiled_subplot: &CompiledPlot,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        selection_revision_fingerprint: SelectionRevisionFingerprint,
        components: PlotComponents,
    ) {
        if let Some(key) = facet_cell_rendered_components_profile_key(
            facet_tree,
            full_path,
            compiled_subplot,
            ctx,
            params,
            selection_revision_fingerprint,
        ) {
            self.profiles.entry(key).or_default().rendered_components = Some(components);
        }
    }
}
