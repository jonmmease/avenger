use std::collections::BTreeMap;
use std::sync::Arc;

use arrow::datatypes::{DataType, SchemaRef};
use avenger_chart_lang_registry::NativeRegistryProfileId;
use avenger_lang_core::{DeclarationId, SourceMap, SourceSpan};
use serde::{Deserialize, Serialize};

use crate::{ModuleDependencyFingerprints, ModuleFingerprint};

/// Stable compiler identity for one project dataset declaration.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModuleDatasetId(String);

impl ModuleDatasetId {
    pub fn new(stable_identity: impl Into<String>) -> Self {
        Self(stable_identity.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable identity for the source or a transform stage of a dataset.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DatasetStageId {
    pub dataset: ModuleDatasetId,
    pub ordinal: u32,
}

impl DatasetStageId {
    pub fn new(dataset: ModuleDatasetId, ordinal: u32) -> Self {
        Self { dataset, ordinal }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DatasetStageKind {
    CatalogTable,
    SqlView,
    DatasetSource,
    Transform {
        native_kind: String,
    },
    /// Final effective relation visible to one primitive mark after inherited
    /// data and all enclosing/local transforms have been applied.
    MarkInput,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetProvenance {
    pub declaration_span: SourceSpan,
    pub stage_span: SourceSpan,
    pub stage_kind: DatasetStageKind,
}

#[derive(Clone)]
pub struct AnalyzedDataset {
    pub id: ModuleDatasetId,
    pub stage: DatasetStageId,
    pub provenance: DatasetProvenance,
    /// SQL-visible relation or stage name when one exists.
    pub qualified_name: Option<String>,
    /// Exact catalog/schema/table components without lossy dot splitting.
    pub qualified_path: Option<Vec<String>>,
    /// DataFusion's resolved qualification, Arrow type, and nullability for
    /// each output column. `schema` remains the convenient Arrow view.
    pub columns: Vec<AnalyzedColumn>,
    pub schema: SchemaRef,
    /// Stable fingerprint of the execution-free DataFusion logical plan when
    /// this stage has one. Schema-only custom stages may leave this absent.
    pub logical_plan_fingerprint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnalyzedColumn {
    pub name: String,
    pub qualifier: Option<String>,
    pub data_type: DataType,
    pub nullable: bool,
}

/// Exact authored expression type for one configured primitive-mark channel.
///
/// These types come from the same DataFusion planning path used by compiler
/// lowering, including dependencies such as `channel.x`. They are kept
/// separate from the mark's logical input relation because channels are
/// evaluated outputs rather than source columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnalyzedMarkChannel {
    pub name: String,
    pub data_type: DataType,
    pub nullable: bool,
}

impl std::fmt::Debug for AnalyzedDataset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalyzedDataset")
            .field("id", &self.id)
            .field("stage", &self.stage)
            .field("provenance", &self.provenance)
            .field("qualified_name", &self.qualified_name)
            .field("qualified_path", &self.qualified_path)
            .field("columns", &self.columns)
            .field("schema", &self.schema)
            .field("logical_plan_fingerprint", &self.logical_plan_fingerprint)
            .finish()
    }
}

/// Exact Arrow schemas for every dataset stage.
#[derive(Clone, Debug, Default)]
pub struct DatasetSchemaIndex {
    stages: BTreeMap<DatasetStageId, AnalyzedDataset>,
}

impl DatasetSchemaIndex {
    pub fn insert(&mut self, dataset: AnalyzedDataset) -> Result<(), AnalysisIndexError> {
        if dataset.stage.dataset != dataset.id {
            return Err(AnalysisIndexError::DatasetStageMismatch);
        }
        let id = dataset.stage.clone();
        if self.stages.contains_key(&id) {
            return Err(AnalysisIndexError::DuplicateStage(id));
        }
        self.stages.insert(id, dataset);
        Ok(())
    }

    pub fn get(&self, stage: &DatasetStageId) -> Option<&AnalyzedDataset> {
        self.stages.get(stage)
    }

    pub fn stages_for(&self, dataset: &ModuleDatasetId) -> impl Iterator<Item = &AnalyzedDataset> {
        self.stages
            .values()
            .filter(move |stage| &stage.id == dataset)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&DatasetStageId, &AnalyzedDataset)> {
        self.stages.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnLineage {
    pub output_column: String,
    pub inputs: Vec<(DatasetStageId, String)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetLineage {
    pub upstream_stages: Vec<DatasetStageId>,
    pub columns: Vec<ColumnLineage>,
}

#[derive(Clone, Debug, Default)]
pub struct DatasetLineageIndex {
    stages: BTreeMap<DatasetStageId, DatasetLineage>,
}

impl DatasetLineageIndex {
    pub fn insert(
        &mut self,
        stage: DatasetStageId,
        lineage: DatasetLineage,
    ) -> Result<(), AnalysisIndexError> {
        if self.stages.contains_key(&stage) {
            return Err(AnalysisIndexError::DuplicateStage(stage));
        }
        self.stages.insert(stage, lineage);
        Ok(())
    }

    pub fn get(&self, stage: &DatasetStageId) -> Option<&DatasetLineage> {
        self.stages.get(stage)
    }
}

/// Immutable, execution-free project analysis. It deliberately contains no
/// mutable DataFusion `SessionContext`.
#[derive(Clone, Debug)]
pub struct ModuleAnalysis {
    pub sources: SourceMap,
    pub datasets: DatasetSchemaIndex,
    pub lineage: DatasetLineageIndex,
    pub native_registry_profile: NativeRegistryProfileId,
    pub module_fingerprint: ModuleFingerprint,
    pub dependency_fingerprints: ModuleDependencyFingerprints,
    pub functions: FunctionInventory,
    pub physical_type_constructors: Vec<String>,
    /// Exact planned types for configured channels on each primitive mark.
    pub mark_channels: BTreeMap<DeclarationId, Vec<AnalyzedMarkChannel>>,
    /// The immutable semantic model that produced this analysis.
    ///
    /// Editor hosts use this to build symbol, scope, and reference indexes
    /// without reimplementing resolver semantics. It contains no DataFusion
    /// session or executable plan and is shared cheaply with cached analyses.
    pub resolved_module_graph: Option<Arc<avenger_lang_core::ResolvedModuleGraph>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FunctionInventory {
    pub scalar: Vec<String>,
    pub aggregate: Vec<String>,
    pub window: Vec<String>,
}

impl ModuleAnalysis {
    pub fn empty(
        sources: SourceMap,
        native_registry_profile: NativeRegistryProfileId,
        module_fingerprint: ModuleFingerprint,
    ) -> Self {
        Self {
            sources,
            datasets: DatasetSchemaIndex::default(),
            lineage: DatasetLineageIndex::default(),
            native_registry_profile,
            module_fingerprint,
            dependency_fingerprints: ModuleDependencyFingerprints::default(),
            functions: FunctionInventory::default(),
            physical_type_constructors: physical_type_constructors(),
            mark_channels: BTreeMap::new(),
            resolved_module_graph: None,
        }
    }
}

fn physical_type_constructors() -> Vec<String> {
    avenger_lang_core::PhysicalType::CONSTRUCTORS
        .iter()
        .copied()
        .map(str::to_owned)
        .collect()
}

/// Convert the language's required physical type into its exact Arrow type.
///
/// Compiler lowering and editor analysis share this conversion so nested
/// struct/list/map completion cannot drift from runtime parameter and store
/// schemas.
pub fn physical_type_to_arrow(value: &avenger_lang_core::PhysicalType) -> DataType {
    crate::lowering::physical_data_type(value)
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum AnalysisIndexError {
    #[error("dataset stage identity does not belong to the analyzed dataset")]
    DatasetStageMismatch,
    #[error("duplicate dataset stage {0:?}")]
    DuplicateStage(DatasetStageId),
}
