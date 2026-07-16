use std::{collections::BTreeMap, fmt, sync::Arc};

use avenger_chart::plot::CompiledPlot;
use avenger_chart_core::StateMigrationKey;
use avenger_chart_lang_registry::NativeRegistryProfileId;
use avenger_lang_core::{SourceId, SourceMap};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

string_id!(ProjectChartId);
string_id!(ProjectFingerprint);
string_id!(DependencyFingerprint);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceStateKind {
    Param,
    Store,
    Selection,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceStateBinding {
    pub kind: InterfaceStateKind,
    pub runtime_id: String,
    pub migration_key: Option<StateMigrationKey>,
}

/// Public host-binding surface retained alongside the compiled plot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledChartInterface {
    pub states: BTreeMap<String, InterfaceStateBinding>,
    pub widget_exports: BTreeMap<String, String>,
    pub public_targets: BTreeMap<String, String>,
}

impl CompiledChartInterface {
    pub fn from_compiled(compiled: &CompiledPlot) -> Self {
        let mut interface = Self::default();
        for (name, spec) in compiled.param_specs() {
            interface.states.insert(
                name.clone(),
                InterfaceStateBinding {
                    kind: InterfaceStateKind::Param,
                    runtime_id: spec.runtime_id.as_opaque_str().to_string(),
                    migration_key: spec.migration_key.clone(),
                },
            );
        }
        for (name, spec) in compiled.store_specs() {
            interface.states.insert(
                name.clone(),
                InterfaceStateBinding {
                    kind: InterfaceStateKind::Store,
                    runtime_id: spec.runtime_id.as_opaque_str().to_string(),
                    migration_key: spec.migration_key.clone(),
                },
            );
        }
        for (name, spec) in compiled.selection_specs() {
            interface.states.insert(
                name.clone(),
                InterfaceStateBinding {
                    kind: InterfaceStateKind::Selection,
                    runtime_id: spec.runtime_id.as_opaque_str().to_string(),
                    migration_key: spec.migration_key.clone(),
                },
            );
        }
        interface
    }
}

#[derive(Clone)]
pub struct CompiledChartArtifact {
    pub id: ProjectChartId,
    pub name: Option<String>,
    pub source: SourceId,
    pub compiled: Arc<CompiledPlot>,
    pub interface: CompiledChartInterface,
    pub native_registry_profile: NativeRegistryProfileId,
    pub dependency_fingerprint: DependencyFingerprint,
}

impl CompiledChartArtifact {
    pub fn new(
        id: ProjectChartId,
        name: Option<String>,
        source: SourceId,
        compiled: Arc<CompiledPlot>,
        native_registry_profile: NativeRegistryProfileId,
        dependency_fingerprint: DependencyFingerprint,
    ) -> Self {
        let interface = CompiledChartInterface::from_compiled(&compiled);
        Self {
            id,
            name,
            source,
            compiled,
            interface,
            native_registry_profile,
            dependency_fingerprint,
        }
    }

    pub fn compiled_plot(&self) -> &CompiledPlot {
        &self.compiled
    }
}

impl fmt::Debug for CompiledChartArtifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompiledChartArtifact")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("source", &self.source)
            .field("interface", &self.interface)
            .field("native_registry_profile", &self.native_registry_profile)
            .field("dependency_fingerprint", &self.dependency_fingerprint)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct CompiledProject {
    pub charts: IndexMap<ProjectChartId, CompiledChartArtifact>,
    pub sources: SourceMap,
    pub native_registry_profile: NativeRegistryProfileId,
    pub project_fingerprint: ProjectFingerprint,
}

impl CompiledProject {
    pub fn chart(&self, name: &str) -> Option<&CompiledChartArtifact> {
        self.charts
            .iter()
            .find_map(|(id, chart)| (id.as_str() == name).then_some(chart))
    }
}

/// Cache identity always includes the immutable native-registry profile.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ArtifactCacheKey {
    pub native_registry_profile: String,
    pub dependency_fingerprint: DependencyFingerprint,
}

impl ArtifactCacheKey {
    pub fn new(
        native_registry_profile: &NativeRegistryProfileId,
        dependency_fingerprint: DependencyFingerprint,
    ) -> Self {
        Self {
            native_registry_profile: native_registry_profile.as_str().to_string(),
            dependency_fingerprint,
        }
    }
}
