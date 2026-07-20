use std::{collections::BTreeMap, fmt, sync::Arc};

use avenger_chart::plot::CompiledPlot;
use avenger_chart_core::StateMigrationKey;
use avenger_chart_lang_registry::{NativeRegistry, NativeRegistryProfileId};
use avenger_lang_core::{ProjectFileId, SourceId, SourceMap};
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

string_id!(ProjectChartId);
string_id!(ProjectFingerprint);
string_id!(DependencyFingerprint);

/// Reviewed fingerprint layers used by project analysis and incremental chart
/// compilation. These values describe immutable inputs; they never retain a
/// DataFusion session or provider registry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDependencyFingerprints {
    pub sources: BTreeMap<ProjectFileId, DependencyFingerprint>,
    pub definition_closures: BTreeMap<ProjectChartId, DependencyFingerprint>,
    pub datasets: BTreeMap<crate::ProjectDatasetId, DependencyFingerprint>,
    pub data_catalog: DependencyFingerprint,
    pub compile_environment: DependencyFingerprint,
    pub charts: BTreeMap<ProjectChartId, DependencyFingerprint>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceStateBinding {
    pub runtime_id: String,
    pub migration_key: Option<StateMigrationKey>,
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

    /// Serialize a versioned host artifact envelope followed by the compiled
    /// plot payload. The fixed prefix lets hosts reject incompatible format
    /// majors and native registry profiles before deserializing trait objects.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ArtifactSerializationError> {
        let header = SerializedArtifactHeader {
            id: self.id.clone(),
            name: self.name.clone(),
            source: self.source,
            interface: self.interface.clone(),
            native_registry_profile: self.native_registry_profile.clone(),
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
        if &header.native_registry_profile != registry.profile_id() {
            return Err(ArtifactSerializationError::RegistryProfileMismatch {
                artifact: header.native_registry_profile.as_str().to_string(),
                host: registry.profile_id().as_str().to_string(),
            });
        }
        let compiled = bincode::deserialize(compiled)
            .map_err(|error| ArtifactSerializationError::Decode(error.to_string()))?;
        Ok(Self {
            id: header.id,
            name: header.name,
            source: header.source,
            compiled: Arc::new(compiled),
            interface: header.interface,
            native_registry_profile: header.native_registry_profile,
            dependency_fingerprint: header.dependency_fingerprint,
        })
    }
}

pub const COMPILED_ARTIFACT_FORMAT_MAJOR: u16 = 1;
const ARTIFACT_MAGIC: [u8; 8] = *b"AVNGRART";
const ARTIFACT_PREFIX_LEN: usize = ARTIFACT_MAGIC.len() + 2 + 4;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SerializedArtifactHeader {
    id: ProjectChartId,
    name: Option<String>,
    source: SourceId,
    interface: CompiledChartInterface,
    native_registry_profile: NativeRegistryProfileId,
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
    #[error("artifact native registry profile '{artifact}' does not match host profile '{host}'")]
    RegistryProfileMismatch { artifact: String, host: String },
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
    pub dependency_fingerprints: ProjectDependencyFingerprints,
}

impl CompiledProject {
    pub fn chart(&self, name: &str) -> Option<&CompiledChartArtifact> {
        self.charts.iter().find_map(|(id, chart)| {
            (id.as_str() == name || chart.name.as_deref() == Some(name)).then_some(chart)
        })
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
