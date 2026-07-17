//! Dependency-direction-safe contracts shared by native language lowerers.
//!
//! Native chart crates own their language schemas and lowerers. The registry
//! depends on those crates to compose a language profile, so owner-facing
//! lowering values cannot live in the registry without creating dependency
//! cycles. This crate deliberately contains only resolved values and native
//! lowering results; it has no parser, registry, or facade dependencies.

use std::{any::Any, collections::BTreeMap, fmt, sync::Arc};

use avenger_chart_core::{
    ChannelValue, ChartTool, CompiledDataTransform, CoordinateSystem, DataTransformCompileContext,
    Param, PlotMark, WidgetAttachment,
};
use avenger_chart_schema::KindSchema;
use datafusion::{common::ScalarValue, dataframe::DataFrame, logical_expr::Expr};
use indexmap::IndexMap;

/// Schema-validated value independent of parser AST and registry types.
#[derive(Clone)]
pub enum ResolvedValue {
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(String),
    Scalar(ScalarValue),
    Expr(Expr),
    Channel(Box<ChannelValue>),
    Query(String),
    Array(Vec<ResolvedValue>),
    Object(IndexMap<String, ResolvedValue>),
    DataFrame(Box<DataFrame>),
    Param(Param),
}

impl fmt::Debug for ResolvedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boolean(value) => f.debug_tuple("Boolean").field(value).finish(),
            Self::Integer(value) => f.debug_tuple("Integer").field(value).finish(),
            Self::Number(value) => f.debug_tuple("Number").field(value).finish(),
            Self::String(value) => f.debug_tuple("String").field(value).finish(),
            Self::Scalar(value) => f.debug_tuple("Scalar").field(value).finish(),
            Self::Expr(_) => f.write_str("Expr(..)"),
            Self::Channel(value) => f.debug_tuple("Channel").field(value).finish(),
            Self::Query(value) => f.debug_tuple("Query").field(value).finish(),
            Self::Array(value) => f.debug_tuple("Array").field(value).finish(),
            Self::Object(value) => f.debug_tuple("Object").field(value).finish(),
            Self::DataFrame(_) => f.write_str("DataFrame(..)"),
            Self::Param(value) => f.debug_tuple("Param").field(&value.name).finish(),
        }
    }
}

/// A schema-validated declaration ready for a native owner lowerer.
#[derive(Clone, Debug)]
pub struct ResolvedDeclaration {
    pub kind: String,
    /// Stable source-level name, separate from schema properties so lowerers
    /// can preserve identity without inventing a kind-specific `id` field.
    pub source_name: Option<String>,
    pub properties: IndexMap<String, ResolvedValue>,
}

impl ResolvedDeclaration {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            source_name: None,
            properties: IndexMap::new(),
        }
    }

    pub fn source_name(mut self, source_name: impl Into<String>) -> Self {
        self.source_name = Some(source_name.into());
        self
    }

    pub fn property(mut self, name: impl Into<String>, value: ResolvedValue) -> Self {
        self.properties.insert(name.into(), value);
        self
    }

    pub fn get(&self, name: &str) -> Result<&ResolvedValue, NativeLoweringError> {
        self.properties
            .get(name)
            .ok_or_else(|| NativeLoweringError::Lowering {
                kind: self.kind.clone(),
                message: format!("missing resolved property '{name}'"),
            })
    }
}

/// Coordinate-independent result from a native transform lowerer.
pub struct LoweredTransform {
    pub transform: Box<dyn CompiledDataTransform>,
    pub outputs: BTreeMap<String, Expr>,
}

/// Errors produced inside an owner crate after registry schema validation.
#[derive(Debug, thiserror::Error)]
pub enum NativeLoweringError {
    #[error(transparent)]
    Chart(#[from] avenger_chart_core::AvengerChartError),
    #[error("property '{property}' expected {expected}")]
    InvalidPropertyType { property: String, expected: String },
    #[error("failed to lower '{kind}': {message}")]
    Lowering { kind: String, message: String },
}

pub type CoordinateLowerer<C> = fn(&ResolvedDeclaration) -> Result<C, NativeLoweringError>;
pub type MarkLowerer<C> = fn(&ResolvedDeclaration) -> Result<Vec<PlotMark<C>>, NativeLoweringError>;
pub type ToolLowerer<C> =
    fn(&ResolvedDeclaration) -> Result<Arc<dyn ChartTool<C>>, NativeLoweringError>;
pub type TransformLowerer = fn(
    &ResolvedDeclaration,
    DataTransformCompileContext,
) -> Result<LoweredTransform, NativeLoweringError>;
pub type WidgetLowerer = fn(&ResolvedDeclaration) -> Result<WidgetAttachment, NativeLoweringError>;
pub type ObjectLowerer =
    fn(&ResolvedDeclaration) -> Result<Box<dyn Any + Send + Sync>, NativeLoweringError>;

/// Owner-provided mark schema and its type-preserving native lowerer.
pub struct MarkLanguageDefinition<C: CoordinateSystem> {
    pub kind: &'static str,
    pub schema: KindSchema,
    pub lowerer: MarkLowerer<C>,
}

/// Owner-provided tool schema and its type-preserving native lowerer.
pub struct ToolLanguageDefinition<C: CoordinateSystem> {
    pub kind: &'static str,
    pub schema: KindSchema,
    pub lowerer: ToolLowerer<C>,
}

/// A coordinate owner's language surface before registry composition.
pub struct CoordinateLanguageDefinition<C: CoordinateSystem> {
    pub kind: &'static str,
    pub schema: KindSchema,
    pub lowerer: CoordinateLowerer<C>,
    pub marks: Vec<MarkLanguageDefinition<C>>,
    pub tools: Vec<ToolLanguageDefinition<C>>,
}

impl<C: CoordinateSystem> CoordinateLanguageDefinition<C> {
    pub fn new(kind: &'static str, schema: KindSchema, lowerer: CoordinateLowerer<C>) -> Self {
        Self {
            kind,
            schema,
            lowerer,
            marks: Vec::new(),
            tools: Vec::new(),
        }
    }

    pub fn mark(mut self, kind: &'static str, schema: KindSchema, lowerer: MarkLowerer<C>) -> Self {
        self.marks.push(MarkLanguageDefinition {
            kind,
            schema,
            lowerer,
        });
        self
    }

    pub fn tool(mut self, kind: &'static str, schema: KindSchema, lowerer: ToolLowerer<C>) -> Self {
        self.tools.push(ToolLanguageDefinition {
            kind,
            schema,
            lowerer,
        });
        self
    }
}

pub struct TransformLanguageDefinition {
    pub schema: KindSchema,
    pub lowerer: TransformLowerer,
}

pub struct WidgetLanguageDefinition {
    pub schema: KindSchema,
    pub lowerer: WidgetLowerer,
}

pub struct ObjectLanguageDefinition {
    pub schema: KindSchema,
    pub lowerer: ObjectLowerer,
}

pub fn expr_property(
    declaration: &ResolvedDeclaration,
    name: &str,
) -> Result<Expr, NativeLoweringError> {
    match declaration.get(name)? {
        ResolvedValue::Expr(expr) => Ok(expr.clone()),
        _ => Err(NativeLoweringError::InvalidPropertyType {
            property: name.to_string(),
            expected: "SQL expression".to_string(),
        }),
    }
}

pub fn string_property(
    declaration: &ResolvedDeclaration,
    name: &str,
) -> Result<String, NativeLoweringError> {
    match declaration.get(name)? {
        ResolvedValue::String(value) | ResolvedValue::Query(value) => Ok(value.clone()),
        _ => Err(NativeLoweringError::InvalidPropertyType {
            property: name.to_string(),
            expected: "string".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_declaration_reports_missing_properties_without_registry_types() {
        let error = ResolvedDeclaration::new("example")
            .get("value")
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "failed to lower 'example': missing resolved property 'value'"
        );
    }
}
