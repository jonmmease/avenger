use super::{invalid, FileFormat, FileSource};
use crate::{Result, TableSnapshot};
use async_trait::async_trait;
use datafusion::object_store::ObjectStoreExt;
use datafusion::{
    arrow::datatypes::SchemaRef,
    catalog::{Session, TableProvider},
    common::Result as DFResult,
    datasource::{
        file_format::{
            arrow::ArrowFormat, csv::CsvFormat, parquet::ParquetFormat, FileFormat as DFFileFormat,
        },
        listing::{ListingOptions, ListingTable, ListingTableConfig, ListingTableUrl},
    },
    execution::context::SessionContext,
    logical_expr::{Expr, TableType},
    physical_plan::ExecutionPlan,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Host-owned immutable assets. Reusing an entry preserves its snapshot identity.
pub type AssetBindings = HashMap<String, TableSnapshot>;

/// Resolves source metadata during loading; returned providers must defer reads until scan.
#[async_trait]
pub trait SourceResolver: Send + Sync {
    async fn infer_schema(&self, source: &FileSource) -> Result<SchemaRef>;
    fn file_provider(
        &self,
        source: &FileSource,
        schema: SchemaRef,
    ) -> Result<Arc<dyn TableProvider>>;
    fn asset(&self, name: &str) -> Result<TableSnapshot>;
}
/// Local file resolver with an explicit base directory and optional Arrow assets.
#[derive(Clone, Debug)]
pub struct FileSourceResolver {
    base: PathBuf,
    assets: AssetBindings,
}
impl FileSourceResolver {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self {
            base: base.into(),
            assets: AssetBindings::new(),
        }
    }
    pub fn with_assets(mut self, assets: AssetBindings) -> Self {
        self.assets = assets;
        self
    }
    fn location(&self, source: &FileSource) -> Result<ListingTableUrl> {
        let path = Path::new(&source.url);
        // Make paths absolute without canonicalizing (which would touch the source).
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.base.join(path)
        };
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir().map_err(invalid)?.join(path)
        };
        let url = if source.url.ends_with(std::path::MAIN_SEPARATOR) {
            url::Url::from_directory_path(&path)
        } else {
            url::Url::from_file_path(&path)
        }
        .map_err(|_| invalid("invalid local file path"))?;
        // Passing an actual URL avoids DataFusion's path parser, which probes the filesystem.
        Ok(ListingTableUrl::try_new(
            url::Url::parse(url.as_str()).map_err(invalid)?,
            None,
        )?)
    }
}
fn options(source: &FileSource) -> Result<ListingOptions> {
    let format: Arc<dyn DFFileFormat> = match source.format {
        FileFormat::Csv => {
            let delimiter = source
                .options
                .delimiter
                .as_deref()
                .unwrap_or(",")
                .as_bytes();
            if delimiter.len() != 1 || !delimiter[0].is_ascii() {
                return Err(invalid("CSV delimiter must be one ASCII byte"));
            }
            if source.options.schema_infer_max_records == Some(0) {
                return Err(invalid("schema inference limit must be positive"));
            }
            Arc::new(
                CsvFormat::default()
                    .with_has_header(source.options.has_header.unwrap_or(true))
                    .with_delimiter(delimiter[0])
                    .with_schema_infer_max_rec(
                        source.options.schema_infer_max_records.unwrap_or(1000),
                    ),
            )
        }
        FileFormat::Parquet | FileFormat::Arrow => {
            if source.options.has_header.is_some()
                || source.options.delimiter.is_some()
                || source.options.schema_infer_max_records.is_some()
            {
                return Err(invalid("CSV options require CSV format"));
            }
            match source.format {
                FileFormat::Parquet => Arc::new(ParquetFormat::default()),
                _ => Arc::new(ArrowFormat),
            }
        }
    };
    Ok(ListingOptions::new(format).with_file_extension(""))
}
#[async_trait]
impl SourceResolver for FileSourceResolver {
    async fn infer_schema(&self, source: &FileSource) -> Result<SchemaRef> {
        Ok(options(source)?
            .infer_schema(&SessionContext::new().state(), &self.location(source)?)
            .await?)
    }
    fn file_provider(
        &self,
        source: &FileSource,
        schema: SchemaRef,
    ) -> Result<Arc<dyn TableProvider>> {
        Ok(Arc::new(DeferredFile {
            descriptor: source.clone(),
            schema,
            location: self.location(source)?,
            options: options(source)?,
        }))
    }
    fn asset(&self, name: &str) -> Result<TableSnapshot> {
        self.assets
            .get(name)
            .cloned()
            .ok_or_else(|| invalid(format!("unknown asset {name}")))
    }
}
#[derive(Debug)]
struct DeferredFile {
    descriptor: FileSource,
    schema: SchemaRef,
    location: ListingTableUrl,
    options: ListingOptions,
}
#[async_trait]
impl TableProvider for DeferredFile {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }
    fn table_type(&self) -> TableType {
        TableType::Base
    }
    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DFResult<Arc<dyn ExecutionPlan>> {
        if self.location.get_glob().is_none() && !self.location.is_collection() {
            let store = state
                .runtime_env()
                .object_store(self.location.object_store())?;
            store.head(self.location.prefix()).await.map_err(|e| {
                datafusion::common::DataFusionError::Execution(format!(
                    "source {}: {e}",
                    self.descriptor.url
                ))
            })?;
        }
        let table = ListingTable::try_new(
            ListingTableConfig::new(self.location.clone())
                .with_listing_options(self.options.clone())
                .with_schema(self.schema.clone()),
        )?;
        table.scan(state, projection, filters, limit).await
    }
}
