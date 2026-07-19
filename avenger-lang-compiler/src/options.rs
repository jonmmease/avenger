use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use avenger_chart_lang_registry::NativeRegistry;
use avenger_lang_core::{DataCapabilities, EnvironmentProvider, ImportCapabilities, SourceLoader};
use datafusion::{catalog::CatalogProvider, datasource::TableProvider, prelude::SessionContext};

#[derive(Clone)]
pub struct CompileEnvironment {
    session_context: Arc<SessionContext>,
}

impl CompileEnvironment {
    pub fn new(session_context: SessionContext) -> Self {
        Self {
            session_context: Arc::new(session_context),
        }
    }

    pub fn session_context(&self) -> &SessionContext {
        &self.session_context
    }

    pub fn session_context_arc(&self) -> Arc<SessionContext> {
        Arc::clone(&self.session_context)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileEnvironmentRequest {
    pub generation: u64,
    pub native_registry_profile: String,
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
