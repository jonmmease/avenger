use std::{collections::BTreeMap, fmt, sync::Arc};

use avenger_chart::plot::CompiledPlot;
use avenger_chart_core::StateMigrationKey;
use avenger_chart_lang_registry::{
    NativeBuiltinProfileId, NativeModuleId, NativeModuleImplementationProfileId,
    NativeModuleSchemaProfileId, NativeRegistry,
};
use avenger_lang_core::{ChartEntrypointId, ChartSelector, SourceId, SourceMap, SourceModuleId};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($name:ident) => {
        #[derive(
            Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
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

string_id!(ModuleFingerprint);
string_id!(DependencyFingerprint);

/// Reviewed fingerprint layers used by module analysis and incremental chart
/// compilation. These values describe immutable inputs; they never retain a
/// DataFusion session or provider registry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleDependencyFingerprints {
    pub sources: BTreeMap<SourceModuleId, DependencyFingerprint>,
    pub item_closures: BTreeMap<ChartEntrypointId, DependencyFingerprint>,
    pub datasets: BTreeMap<crate::DatasetStageId, DependencyFingerprint>,
    pub data_catalog: DependencyFingerprint,
    pub compile_environment: DependencyFingerprint,
    pub charts: BTreeMap<ChartEntrypointId, DependencyFingerprint>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceStateBinding {
    pub runtime_id: String,
    pub migration_key: Option<StateMigrationKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeModuleRequirement {
    pub schema_profile: NativeModuleSchemaProfileId,
    pub implementation_profile: NativeModuleImplementationProfileId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeRequirementSet {
    pub builtin_profile: NativeBuiltinProfileId,
    pub modules: BTreeMap<NativeModuleId, NativeModuleRequirement>,
}

impl NativeRequirementSet {
    pub fn builtin_only(registry: &NativeRegistry) -> Self {
        Self {
            builtin_profile: registry.builtin_profile_id().clone(),
            modules: BTreeMap::new(),
        }
    }

    pub fn from_modules(
        registry: &NativeRegistry,
        modules: impl IntoIterator<Item = NativeModuleId>,
    ) -> Result<Self, ArtifactSerializationError> {
        let mut requirements = Self::builtin_only(registry);
        for id in modules {
            let (_, registered) = registry.native_module(&id).ok_or_else(|| {
                ArtifactSerializationError::MissingNativeModule {
                    module: id.as_str().to_owned(),
                }
            })?;
            requirements.modules.insert(
                id,
                NativeModuleRequirement {
                    schema_profile: registered.schema_profile.clone(),
                    implementation_profile: registered.implementation_profile.clone(),
                },
            );
        }
        Ok(requirements)
    }

    pub fn validate(&self, registry: &NativeRegistry) -> Result<(), ArtifactSerializationError> {
        if &self.builtin_profile != registry.builtin_profile_id() {
            return Err(ArtifactSerializationError::BuiltinProfileMismatch {
                artifact: self.builtin_profile.as_str().to_owned(),
                host: registry.builtin_profile_id().as_str().to_owned(),
            });
        }
        for (id, requirement) in &self.modules {
            let (_, installed) = registry.native_module(id).ok_or_else(|| {
                ArtifactSerializationError::MissingNativeModule {
                    module: id.as_str().to_owned(),
                }
            })?;
            if installed.schema_profile != requirement.schema_profile {
                return Err(ArtifactSerializationError::NativeModuleSchemaMismatch {
                    module: id.as_str().to_owned(),
                    artifact: requirement.schema_profile.as_str().to_owned(),
                    host: installed.schema_profile.as_str().to_owned(),
                });
            }
            if installed.implementation_profile != requirement.implementation_profile {
                return Err(
                    ArtifactSerializationError::NativeModuleImplementationMismatch {
                        module: id.as_str().to_owned(),
                        artifact: requirement.implementation_profile.as_str().to_owned(),
                        host: installed.implementation_profile.as_str().to_owned(),
                    },
                );
            }
        }
        Ok(())
    }

    pub fn fingerprint(&self) -> String {
        use sha2::{Digest, Sha256};
        let bytes = serde_json::to_vec(self).expect("native requirements serialize");
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    pub fn union<'a>(sets: impl IntoIterator<Item = &'a Self>) -> Option<Self> {
        let mut sets = sets.into_iter();
        let first = sets.next()?.clone();
        let mut result = first;
        for set in sets {
            debug_assert_eq!(result.builtin_profile, set.builtin_profile);
            result.modules.extend(set.modules.clone());
        }
        Some(result)
    }
}

/// Public host-binding surface retained alongside the compiled plot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledChartInterface {
    pub params: BTreeMap<String, InterfaceStateBinding>,
    pub stores: BTreeMap<String, InterfaceStateBinding>,
    pub selections: BTreeMap<String, InterfaceStateBinding>,
    pub widget_exports: BTreeMap<String, String>,
    pub public_targets: BTreeMap<String, String>,
}

impl CompiledChartInterface {
    pub fn from_compiled(compiled: &CompiledPlot) -> Self {
        let mut interface = Self::default();
        for (name, spec) in compiled.param_specs() {
            interface.params.insert(
                name.clone(),
                InterfaceStateBinding {
                    runtime_id: spec.runtime_id.as_opaque_str().to_string(),
                    migration_key: spec.migration_key.clone(),
                },
            );
        }
        for (name, spec) in compiled.store_specs() {
            interface.stores.insert(
                name.clone(),
                InterfaceStateBinding {
                    runtime_id: spec.runtime_id.as_opaque_str().to_string(),
                    migration_key: spec.migration_key.clone(),
                },
            );
        }
        for (name, spec) in compiled.selection_specs() {
            interface.selections.insert(
                name.clone(),
                InterfaceStateBinding {
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
    pub id: ChartEntrypointId,
    pub name: Option<String>,
    pub source: SourceId,
    pub compiled: Arc<CompiledPlot>,
    pub interface: CompiledChartInterface,
    pub native_requirements: NativeRequirementSet,
    pub dependency_fingerprint: DependencyFingerprint,
}

impl CompiledChartArtifact {
    pub fn new(
        id: ChartEntrypointId,
        name: Option<String>,
        source: SourceId,
        compiled: Arc<CompiledPlot>,
        native_requirements: NativeRequirementSet,
        dependency_fingerprint: DependencyFingerprint,
    ) -> Self {
        let interface = CompiledChartInterface::from_compiled(&compiled);
        Self {
            id,
            name,
            source,
            compiled,
            interface,
            native_requirements,
            dependency_fingerprint,
        }
    }

    pub fn compiled_plot(&self) -> &CompiledPlot {
        &self.compiled
    }

    /// Serialize a versioned host artifact envelope followed by the compiled
    /// plot payload. The fixed prefix lets hosts reject incompatible format
    /// majors and native registry profiles before deserializing trait objects.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ArtifactSerializationError> {
        let header = SerializedArtifactHeader {
            id: self.id.clone(),
            name: self.name.clone(),
            source: self.source,
            interface: self.interface.clone(),
            native_requirements: self.native_requirements.clone(),
            dependency_fingerprint: self.dependency_fingerprint.clone(),
        };
        let header = bincode::serialize(&header)
            .map_err(|error| ArtifactSerializationError::Encode(error.to_string()))?;
        let compiled = bincode::serialize(self.compiled.as_ref())
            .map_err(|error| ArtifactSerializationError::Encode(error.to_string()))?;
        let header_len = u32::try_from(header.len()).map_err(|_| {
            ArtifactSerializationError::Encode("artifact header exceeds u32 length".to_string())
        })?;
        let mut output = Vec::with_capacity(ARTIFACT_PREFIX_LEN + header.len() + compiled.len());
        output.extend_from_slice(&ARTIFACT_MAGIC);
        output.extend_from_slice(&COMPILED_ARTIFACT_FORMAT_MAJOR.to_le_bytes());
        output.extend_from_slice(&header_len.to_le_bytes());
        output.extend_from_slice(&header);
        output.extend_from_slice(&compiled);
        Ok(output)
    }

    /// Deserialize an artifact only after verifying both the format major and
    /// the exact composed native-registry profile installed in this host.
    pub fn from_bytes(
        bytes: &[u8],
        registry: &NativeRegistry,
    ) -> Result<Self, ArtifactSerializationError> {
        let (header, compiled) = decode_artifact_parts(bytes)?;
        header.native_requirements.validate(registry)?;
        let compiled = bincode::deserialize(compiled)
            .map_err(|error| ArtifactSerializationError::Decode(error.to_string()))?;
        Ok(Self {
            id: header.id,
            name: header.name,
            source: header.source,
            compiled: Arc::new(compiled),
            interface: header.interface,
            native_requirements: header.native_requirements,
            dependency_fingerprint: header.dependency_fingerprint,
        })
    }
}

pub const COMPILED_ARTIFACT_FORMAT_MAJOR: u16 = 2;
const ARTIFACT_MAGIC: [u8; 8] = *b"AVNGRART";
const ARTIFACT_PREFIX_LEN: usize = ARTIFACT_MAGIC.len() + 2 + 4;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SerializedArtifactHeader {
    id: ChartEntrypointId,
    name: Option<String>,
    source: SourceId,
    interface: CompiledChartInterface,
    native_requirements: NativeRequirementSet,
    dependency_fingerprint: DependencyFingerprint,
}

fn decode_artifact_parts(
    bytes: &[u8],
) -> Result<(SerializedArtifactHeader, &[u8]), ArtifactSerializationError> {
    if bytes.len() < ARTIFACT_PREFIX_LEN {
        return Err(ArtifactSerializationError::Truncated);
    }
    if bytes[..ARTIFACT_MAGIC.len()] != ARTIFACT_MAGIC {
        return Err(ArtifactSerializationError::InvalidMagic);
    }
    let major = u16::from_le_bytes(
        bytes[ARTIFACT_MAGIC.len()..ARTIFACT_MAGIC.len() + 2]
            .try_into()
            .expect("prefix length checked"),
    );
    if major != COMPILED_ARTIFACT_FORMAT_MAJOR {
        return Err(ArtifactSerializationError::UnsupportedFormatMajor {
            found: major,
            supported: COMPILED_ARTIFACT_FORMAT_MAJOR,
        });
    }
    let length_start = ARTIFACT_MAGIC.len() + 2;
    let header_len = u32::from_le_bytes(
        bytes[length_start..length_start + 4]
            .try_into()
            .expect("prefix length checked"),
    ) as usize;
    let header_end = ARTIFACT_PREFIX_LEN
        .checked_add(header_len)
        .filter(|end| *end <= bytes.len())
        .ok_or(ArtifactSerializationError::Truncated)?;
    let header = bincode::deserialize(&bytes[ARTIFACT_PREFIX_LEN..header_end])
        .map_err(|error| ArtifactSerializationError::Decode(error.to_string()))?;
    Ok((header, &bytes[header_end..]))
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ArtifactSerializationError {
    #[error("artifact is truncated")]
    Truncated,
    #[error("artifact magic header is invalid")]
    InvalidMagic,
    #[error("unsupported compiled artifact format major {found}; host supports {supported}")]
    UnsupportedFormatMajor { found: u16, supported: u16 },
    #[error("artifact built-in profile '{artifact}' does not match host profile '{host}'")]
    BuiltinProfileMismatch { artifact: String, host: String },
    #[error("artifact requires unavailable native module '{module}'")]
    MissingNativeModule { module: String },
    #[error(
        "artifact native module '{module}' schema profile '{artifact}' does not match host '{host}'"
    )]
    NativeModuleSchemaMismatch {
        module: String,
        artifact: String,
        host: String,
    },
    #[error(
        "artifact native module '{module}' implementation profile '{artifact}' does not match host '{host}'"
    )]
    NativeModuleImplementationMismatch {
        module: String,
        artifact: String,
        host: String,
    },
    #[error("failed to encode compiled artifact: {0}")]
    Encode(String),
    #[error("failed to decode compiled artifact: {0}")]
    Decode(String),
}

impl fmt::Debug for CompiledChartArtifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompiledChartArtifact")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("source", &self.source)
            .field("interface", &self.interface)
            .field("native_requirements", &self.native_requirements)
            .field("dependency_fingerprint", &self.dependency_fingerprint)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct CompiledModule {
    pub charts: IndexMap<ChartEntrypointId, CompiledChartArtifact>,
    pub sources: SourceMap,
    pub native_requirements: NativeRequirementSet,
    pub module_fingerprint: ModuleFingerprint,
    pub dependency_fingerprints: ModuleDependencyFingerprints,
}

impl CompiledModule {
    pub fn chart(&self, name: &str) -> Option<&CompiledChartArtifact> {
        self.charts.iter().find_map(|(id, chart)| {
            (matches!(&id.selector, ChartSelector::Named(selector) if selector == name)
                || chart.name.as_deref() == Some(name))
            .then_some(chart)
        })
    }
}

/// Cache identity always includes the immutable native-registry profile.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ArtifactCacheKey {
    pub native_requirements: String,
    pub dependency_fingerprint: DependencyFingerprint,
}

impl ArtifactCacheKey {
    pub fn new(
        native_requirements: &NativeRequirementSet,
        dependency_fingerprint: DependencyFingerprint,
    ) -> Self {
        Self {
            native_requirements: native_requirements.fingerprint(),
            dependency_fingerprint,
        }
    }
}
