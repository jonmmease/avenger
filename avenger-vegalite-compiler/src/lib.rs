#![doc = include_str!("../README.md")]
mod compile;
mod source;

pub use avenger_chart::{Chart, ChartOptions};
pub use avenger_chart_definition::ChartDefinition;
pub use avenger_datafusion_dataflow::TableSnapshot;
pub use avenger_vegalite_spec as spec;
use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
};

/// A compilation failure associated with a specification property.
#[derive(Debug, thiserror::Error)]
#[error("{path}: {cause}")]
pub struct CompileError {
    path: String,
    #[source]
    cause: Box<dyn std::error::Error + Send + Sync>,
}
impl CompileError {
    /// Specification property or transform that failed.
    pub fn path(&self) -> &str {
        &self.path
    }
    /// Description of the underlying failure.
    pub fn message(&self) -> String {
        self.cause.to_string()
    }
    fn at(path: impl Into<String>, cause: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            path: path.into(),
            cause: Box::new(cause),
        }
    }
}
fn error(path: impl Into<String>, message: impl Into<String>) -> CompileError {
    CompileError::at(path, std::io::Error::other(message.into()))
}
type Result<T> = std::result::Result<T, CompileError>;

/// Source resolution and destination chart resources.
pub struct VegaLiteOptions {
    /// Directory used to resolve relative local data URLs.
    pub base_dir: PathBuf,
    /// Resources passed to native chart preparation.
    pub chart: ChartOptions,
}
impl Default for VegaLiteOptions {
    fn default() -> Self {
        Self {
            base_dir: PathBuf::from("."),
            chart: ChartOptions::default(),
        }
    }
}

/// Compile a parsed spec without preparing the dataflow or initializing rendering resources.
/// Local sources are captured once. Named bindings take precedence over root datasets.
pub async fn compile_vegalite(
    spec: &spec::UnitSpec,
    datasets: &BTreeMap<String, TableSnapshot>,
    base_dir: &Path,
) -> Result<ChartDefinition> {
    spec.validate()
        .map_err(|e| CompileError::at(e.path().to_string(), e))?;
    let source = source::load(spec, datasets, base_dir).await?;
    compile::compile(spec, source)
}

/// Compile a named dataset as a required, replaceable dataflow table input.
///
/// The spec must use `data.name`. That name identifies the root table input in
/// the returned definition. The supplied schema is the raw input schema, with
/// no compiler-generated columns. Bind a matching `TableSnapshot` through
/// `Chart::inputs` for each render. Row order follows snapshot batch order.
///
/// No data is loaded or embedded, including any matching root `datasets` entry.
/// The input declaration and row-order handling survive native serialization.
pub fn compile_vegalite_with_input(
    spec: &spec::UnitSpec,
    schema: avenger_datafusion_dataflow::arrow::datatypes::SchemaRef,
) -> Result<ChartDefinition> {
    spec.validate()
        .map_err(|e| CompileError::at(e.path().to_string(), e))?;
    compile::compile(spec, source::input(spec, schema)?)
}

/// Construct a prepared chart through the Vega-Lite frontend.
pub trait FromVegaLite: Sized {
    /// Compile a spec and prepare its native definition using the supplied resources.
    fn from_vegalite(
        spec: &spec::UnitSpec,
        datasets: &BTreeMap<String, TableSnapshot>,
        options: VegaLiteOptions,
    ) -> impl Future<Output = Result<Self>> + Send;
}
impl FromVegaLite for Chart {
    async fn from_vegalite(
        spec: &spec::UnitSpec,
        datasets: &BTreeMap<String, TableSnapshot>,
        options: VegaLiteOptions,
    ) -> Result<Self> {
        let definition = compile_vegalite(spec, datasets, &options.base_dir).await?;
        Chart::prepare(definition, options.chart)
            .await
            .map_err(|e| CompileError::at("$", e))
    }
}
