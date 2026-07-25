use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use avenger_chart_lang_registry::NativeRegistry;
use avenger_lang_core::{
    DataCapabilities, EnvironmentProvider, ExpansionLimits, ImportCapabilities,
    ModuleGraphLoadLimits, SourceLoader,
};
use datafusion::{catalog::CatalogProvider, datasource::TableProvider, prelude::SessionContext};

#[derive(Clone)]
pub struct CompileEnvironment {
    session_context: Arc<SessionContext>,
    dependency_fingerprint: String,
}

impl CompileEnvironment {
    pub fn new(session_context: SessionContext) -> Self {
        Self {
            session_context: Arc::new(session_context),
            dependency_fingerprint: "avenger-default-compile-environment-v1".to_owned(),
        }
    }

    /// Attach an immutable host snapshot identity (for example the installed
    /// object-store configuration and shared physical-plan-cache version).
    pub fn with_dependency_fingerprint(mut self, fingerprint: impl Into<String>) -> Self {
        self.dependency_fingerprint = fingerprint.into();
        self
    }

    pub fn dependency_fingerprint(&self) -> &str {
        &self.dependency_fingerprint
    }

    pub fn session_context(&self) -> &SessionContext {
        &self.session_context
    }

    pub fn session_context_arc(&self) -> Arc<SessionContext> {
        Arc::clone(&self.session_context)
    }

    /// Fork one chart-local session from the immutable project generation.
    /// Catalog providers and runtime/cache configuration are cloned from the
    /// base state, while chart-local temporary registrations remain isolated.
    pub fn fork(&self) -> Self {
        Self {
            session_context: Arc::new(SessionContext::new_with_state(self.session_context.state())),
            dependency_fingerprint: self.dependency_fingerprint.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileEnvironmentRequest {
    pub generation: u64,
    pub native_registry_profile: String,
    /// Immutable local-resource content identities discovered before this
    /// generation's DataFusion context is constructed.
    pub local_resource_versions: Vec<CompileEnvironmentResourceVersion>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileEnvironmentResourceVersion {
    /// Exact file path, directory root, or static root preceding a glob.
    pub path: PathBuf,
    /// Whether the content identity covers descendants of `path`.
    pub recursive: bool,
    pub content_version: String,
}

pub trait CompileEnvironmentFactory: Send + Sync {
    fn create(
        &self,
        request: &CompileEnvironmentRequest,
    ) -> Result<CompileEnvironment, CompileEnvironmentError>;
}

#[derive(Clone, Debug, Default)]
pub struct DefaultCompileEnvironmentFactory;

impl CompileEnvironmentFactory for DefaultCompileEnvironmentFactory {
    fn create(
        &self,
        _request: &CompileEnvironmentRequest,
    ) -> Result<CompileEnvironment, CompileEnvironmentError> {
        Ok(CompileEnvironment::new(SessionContext::new()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CompileEnvironmentError {
    #[error("failed to create compile environment: {0}")]
    Message(String),
}

#[async_trait]
pub trait CatalogFactory: Send + Sync {
    async fn create(
        &self,
        options: &serde_json::Value,
        environment: &CompileEnvironment,
    ) -> Result<Arc<dyn CatalogProvider>, CatalogFactoryError>;

    /// Immutable snapshot identity (for example an Iceberg snapshot id) to
    /// fold into project/artifact fingerprints. Configuration text is already
    /// covered by the source fingerprint.
    async fn dependency_fingerprint(
        &self,
        _options: &serde_json::Value,
        _environment: &CompileEnvironment,
    ) -> Result<Option<String>, CatalogFactoryError> {
        Ok(None)
    }
}

#[derive(Clone, Default)]
pub struct CatalogFactoryRegistry {
    factories: BTreeMap<String, Arc<dyn CatalogFactory>>,
}

impl CatalogFactoryRegistry {
    pub fn register(
        &mut self,
        kind: impl Into<String>,
        factory: Arc<dyn CatalogFactory>,
    ) -> Result<(), CatalogFactoryError> {
        let kind = kind.into();
        if self.factories.contains_key(&kind) {
            return Err(CatalogFactoryError::DuplicateKind(kind));
        }
        self.factories.insert(kind, factory);
        Ok(())
    }

    pub fn get(&self, kind: &str) -> Option<&Arc<dyn CatalogFactory>> {
        self.factories.get(kind)
    }

    pub fn kinds(&self) -> impl Iterator<Item = &str> {
        self.factories.keys().map(String::as_str)
    }
}

impl std::fmt::Debug for CatalogFactoryRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogFactoryRegistry")
            .field("kinds", &self.factories.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum CatalogFactoryError {
    #[error("duplicate catalog factory kind '{0}'")]
    DuplicateKind(String),
    #[error("catalog factory failed: {0}")]
    Message(String),
}

/// Host extension point for table formats such as Delta Lake. Built-in file,
/// SQL, and inline tables bypass this registry.
#[async_trait]
pub trait TableFactory: Send + Sync {
    async fn create(
        &self,
        options: &serde_json::Value,
        environment: &CompileEnvironment,
    ) -> Result<Arc<dyn TableProvider>, TableFactoryError>;

    async fn dependency_fingerprint(
        &self,
        _options: &serde_json::Value,
        _environment: &CompileEnvironment,
    ) -> Result<Option<String>, TableFactoryError> {
        Ok(None)
    }
}

#[derive(Clone, Default)]
pub struct TableFactoryRegistry {
    factories: BTreeMap<String, Arc<dyn TableFactory>>,
}

impl TableFactoryRegistry {
    pub fn register(
        &mut self,
        kind: impl Into<String>,
        factory: Arc<dyn TableFactory>,
    ) -> Result<(), TableFactoryError> {
        let kind = kind.into();
        if self.factories.contains_key(&kind) {
            return Err(TableFactoryError::DuplicateKind(kind));
        }
        self.factories.insert(kind, factory);
        Ok(())
    }

    pub fn get(&self, kind: &str) -> Option<&Arc<dyn TableFactory>> {
        self.factories.get(kind)
    }

    pub fn kinds(&self) -> impl Iterator<Item = &str> {
        self.factories.keys().map(String::as_str)
    }
}

impl std::fmt::Debug for TableFactoryRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableFactoryRegistry")
            .field("kinds", &self.factories.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum TableFactoryError {
    #[error("duplicate table factory kind '{0}'")]
    DuplicateKind(String),
    #[error("table factory failed: {0}")]
    Message(String),
}

#[derive(Clone)]
pub struct CompilerOptions {
    pub project_root: PathBuf,
    pub import_capabilities: ImportCapabilities,
    pub data_capabilities: DataCapabilities,
    pub environment: Arc<dyn EnvironmentProvider>,
    pub native_registry: Arc<NativeRegistry>,
    pub source_loader: Arc<dyn SourceLoader>,
    pub catalog_factories: CatalogFactoryRegistry,
    pub table_factories: TableFactoryRegistry,
    pub environment_factory: Arc<dyn CompileEnvironmentFactory>,
    pub limits: CompilerLimits,
}

/// Bounds used by project loading and local-resource fingerprinting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompilerLimits {
    pub module_graph: ModuleGraphLoadLimits,
    pub resources: LocalResourceLimits,
    pub expansion: ExpansionLimits,
}

/// Bounds for files, directories, and globs consulted as local data resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalResourceLimits {
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_files: usize,
    pub max_directory_depth: usize,
    pub max_directory_entries: usize,
}

impl Default for LocalResourceLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 512 * 1024 * 1024,
            max_total_bytes: 2 * 1024 * 1024 * 1024,
            max_files: 100_000,
            max_directory_depth: 64,
            max_directory_entries: 100_000,
        }
    }
}

impl std::fmt::Debug for CompilerOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompilerOptions")
            .field("project_root", &self.project_root)
            .field("import_capabilities", &self.import_capabilities)
            .field("data_capabilities", &self.data_capabilities)
            .field(
                "native_registry_profile",
                &self.native_registry.profile_id(),
            )
            .field("catalog_factories", &self.catalog_factories)
            .field("table_factories", &self.table_factories)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingFactory(&'static str);

    #[async_trait]
    impl CatalogFactory for FailingFactory {
        async fn create(
            &self,
            _options: &serde_json::Value,
            _environment: &CompileEnvironment,
        ) -> Result<Arc<dyn CatalogProvider>, CatalogFactoryError> {
            Err(CatalogFactoryError::Message(self.0.to_string()))
        }
    }

    #[test]
    fn duplicate_catalog_factory_does_not_replace_the_original() {
        let first: Arc<dyn CatalogFactory> = Arc::new(FailingFactory("first"));
        let second: Arc<dyn CatalogFactory> = Arc::new(FailingFactory("second"));
        let mut factories = CatalogFactoryRegistry::default();
        factories.register("fixture", first.clone()).unwrap();
        assert!(matches!(
            factories.register("fixture", second),
            Err(CatalogFactoryError::DuplicateKind(kind)) if kind == "fixture"
        ));
        assert!(Arc::ptr_eq(factories.get("fixture").unwrap(), &first));
    }
}
