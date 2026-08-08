//! Dependency-direction-safe contracts shared by native language lowerers.
//!
//! Native chart crates own their language schemas and lowerers. The registry
//! depends on those crates to compose a language profile, so owner-facing
//! lowering values cannot live in the registry without creating dependency
//! cycles. This crate deliberately contains only resolved values and native
//! lowering results; it has no parser, registry, or facade dependencies.

use std::{any::Any, collections::BTreeMap, fmt, sync::Arc};

use avenger_chart_core::{
    ChannelExpr, ChannelValue, ChartAction, ChartTool, CompiledDataTransform,
    CompiledMarkAdjustmentTransform, CoordinateSystem, DataTransformCompileContext,
    DataTransformStage, MarkAdjustmentCompileContext, Param, PatternChannelValue, PlotMark,
    PrimitiveMarkEffects, RasterDim, Selection, WidgetAttachment, WidgetItems,
};
use avenger_chart_schema::KindSchema;
use datafusion::{
    common::ScalarValue,
    dataframe::DataFrame,
    logical_expr::{Expr, lit},
};
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
    Projection(Vec<ResolvedProjectionItem>),
    /// A configured encoding that preserves both its data expression and
    /// channel metadata. Compound marks consume the expression while ordinary
    /// primitive marks consume the `ChannelValue` from the same handle.
    Channel(Box<ChannelExpr>),
    Query(String),
    Array(Vec<ResolvedValue>),
    Object(IndexMap<String, ResolvedValue>),
    /// A value-bearing block whose head and schema-owned configuration must
    /// remain distinct for its native owner lowerer.
    Configured {
        head: Box<ResolvedValue>,
        properties: IndexMap<String, ResolvedValue>,
    },
    Call {
        function: String,
        args: Vec<ResolvedValue>,
    },
    DataFrame(Box<DataFrame>),
    Param(Param),
    Selection(Selection),
    WidgetItems(WidgetItems),
    Pattern(PatternChannelValue),
    RasterDimensionChannel {
        dimension: RasterDim,
        channel: Box<ChannelValue>,
    },
    Output(NativeOutputValue),
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
            Self::Projection(value) => f.debug_tuple("Projection").field(value).finish(),
            Self::Channel(value) => f.debug_tuple("Channel").field(value).finish(),
            Self::Query(value) => f.debug_tuple("Query").field(value).finish(),
            Self::Array(value) => f.debug_tuple("Array").field(value).finish(),
            Self::Object(value) => f.debug_tuple("Object").field(value).finish(),
            Self::Configured { head, properties } => f
                .debug_struct("Configured")
                .field("head", head)
                .field("properties", properties)
                .finish(),
            Self::Call { function, args } => f
                .debug_struct("Call")
                .field("function", function)
                .field("args", args)
                .finish(),
            Self::DataFrame(_) => f.write_str("DataFrame(..)"),
            Self::Param(value) => f.debug_tuple("Param").field(&value.name).finish(),
            Self::Selection(value) => f.debug_tuple("Selection").field(&value.id).finish(),
            Self::WidgetItems(_) => f.write_str("WidgetItems(..)"),
            Self::Pattern(value) => f.debug_tuple("Pattern").field(value).finish(),
            Self::RasterDimensionChannel { dimension, .. } => f
                .debug_tuple("RasterDimensionChannel")
                .field(dimension)
                .finish(),
            Self::Output(value) => f.debug_tuple("Output").field(value).finish(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedProjectionItem {
    pub expr: Expr,
    pub alias: Option<String>,
    pub direct_column: bool,
}

/// A schema-validated declaration ready for a native owner lowerer.
#[derive(Clone, Debug)]
pub struct ResolvedDeclaration {
    pub kind: String,
    /// Optional source-level declaration variant following the structural
    /// role, such as `row` in `variable row mpg { ... }`.
    pub variant: Option<String>,
    /// Stable source-level name, separate from schema properties so lowerers
    /// can preserve identity without inventing a kind-specific `id` field.
    pub source_name: Option<String>,
    /// Whether the structural source name is also an implicit public target.
    /// Private DSL structure retains its source name for diagnostics and
    /// component identity while suppressing that automatic target path.
    pub publish_source_name: bool,
    /// Additional public target aliases, relative to the containing public
    /// group. Component exports use these without exposing private ancestry.
    pub public_aliases: Vec<String>,
    /// Component export alias to retain as theme part provenance. This is
    /// deliberately independent of both the private source id and public
    /// target aliases.
    pub component_part_alias: Option<String>,
    /// Ordered render-stage adjustments and derived primitive marks lowered
    /// from core `adjust`/`derive` children.
    pub mark_effects: PrimitiveMarkEffects,
    /// Ordered state action owned by a native declaration, such as the action
    /// attached to a Button activation count.
    pub state_action: Option<ChartAction>,
    pub properties: IndexMap<String, ResolvedValue>,
    /// Ordered schema-owned child declarations. Core containers remain in the
    /// compiler IR; native owners receive only children declared by their
    /// `KindSchema` child rules.
    pub children: Vec<ResolvedDeclaration>,
    /// Schema exports retained after semantic liveness analysis. Native
    /// owners use this for opt-in runtime state such as TextInput cursor data.
    pub live_exports: std::collections::BTreeSet<String>,
}

impl ResolvedDeclaration {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            variant: None,
            source_name: None,
            publish_source_name: true,
            public_aliases: Vec::new(),
            component_part_alias: None,
            mark_effects: PrimitiveMarkEffects::default(),
            state_action: None,
            properties: IndexMap::new(),
            children: Vec::new(),
            live_exports: std::collections::BTreeSet::new(),
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

    pub fn child(mut self, child: ResolvedDeclaration) -> Self {
        self.children.push(child);
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
    pub outputs: BTreeMap<String, NativeOutputValue>,
}

/// Coordinate-independent result from a registered mark-adjustment lowerer.
pub struct LoweredAdjustment {
    pub transform: Box<dyn CompiledMarkAdjustmentTransform>,
    pub outputs: BTreeMap<String, Expr>,
}

/// A typed native output handle preserved until its authoring use site.
#[derive(Clone)]
// Boxing `ChannelExpr` would change the public typed-output API.
#[allow(clippy::large_enum_variant)]
pub enum NativeOutputValue {
    Expr(Expr),
    Channel(ChannelExpr),
    RasterDim(RasterDim),
    Opaque(Arc<dyn Any + Send + Sync>),
}

impl fmt::Debug for NativeOutputValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expr(expr) => f.debug_tuple("Expr").field(expr).finish(),
            Self::Channel(channel) => f.debug_tuple("Channel").field(channel).finish(),
            Self::RasterDim(dimension) => f.debug_tuple("RasterDim").field(dimension).finish(),
            Self::Opaque(_) => f.write_str("Opaque(..)"),
        }
    }
}

impl From<Expr> for NativeOutputValue {
    fn from(value: Expr) -> Self {
        Self::Expr(value)
    }
}

impl From<ChannelExpr> for NativeOutputValue {
    fn from(value: ChannelExpr) -> Self {
        Self::Channel(value)
    }
}

impl From<RasterDim> for NativeOutputValue {
    fn from(value: RasterDim) -> Self {
        Self::RasterDim(value)
    }
}

impl NativeOutputValue {
    pub fn opaque<T: Any + Send + Sync>(value: T) -> Self {
        Self::Opaque(Arc::new(value))
    }

    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        match self {
            Self::Opaque(value) => value.downcast_ref(),
            _ => None,
        }
    }
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
pub type CoordinateMarkLowerer<C> =
    fn(&C, &ResolvedDeclaration) -> Result<Vec<PlotMark<C>>, NativeLoweringError>;
pub type ToolLowerer<C> =
    fn(&ResolvedDeclaration) -> Result<Arc<dyn ChartTool<C>>, NativeLoweringError>;
pub type TransformLowerer = fn(
    &ResolvedDeclaration,
    DataTransformCompileContext,
) -> Result<LoweredTransform, NativeLoweringError>;
pub type TransformPipelineLowerer = fn(
    &ResolvedDeclaration,
    Vec<DataTransformStage>,
    IndexMap<String, Expr>,
    DataTransformCompileContext,
) -> Result<LoweredTransform, NativeLoweringError>;
pub type AdjustmentLowerer = fn(
    &ResolvedDeclaration,
    MarkAdjustmentCompileContext,
) -> Result<LoweredAdjustment, NativeLoweringError>;
pub type WidgetLowerer = fn(&ResolvedDeclaration) -> Result<WidgetAttachment, NativeLoweringError>;
pub type ObjectLowerer =
    fn(&ResolvedDeclaration) -> Result<Box<dyn Any + Send + Sync>, NativeLoweringError>;

/// Owner-provided mark schema and its type-preserving native lowerer.
pub struct MarkLanguageDefinition<C: CoordinateSystem> {
    pub kind: &'static str,
    pub schema: KindSchema,
    pub lowerer: MarkLanguageLowerer<C>,
}

/// Native mark lowerers normally need only the resolved declaration. Marks
/// whose authoring expressions depend on the concrete coordinate instance can
/// opt into coordinate-aware lowering without imposing an unused parameter on
/// every downstream extension.
pub enum MarkLanguageLowerer<C: CoordinateSystem> {
    Declaration(MarkLowerer<C>),
    Coordinate(CoordinateMarkLowerer<C>),
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
            lowerer: MarkLanguageLowerer::Declaration(lowerer),
        });
        self
    }

    pub fn mark_with_coordinate(
        mut self,
        kind: &'static str,
        schema: KindSchema,
        lowerer: CoordinateMarkLowerer<C>,
    ) -> Self {
        self.marks.push(MarkLanguageDefinition {
            kind,
            schema,
            lowerer: MarkLanguageLowerer::Coordinate(lowerer),
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

/// Owner-provided mixed-body transform container and its native assembler.
///
/// The compiler lowers child stages and public output expressions generically;
/// this paired lowerer preserves the owner's native parent-stage boundary.
pub struct TransformPipelineLanguageDefinition {
    pub schema: KindSchema,
    pub lowerer: TransformPipelineLowerer,
}

/// Owner-provided mark-adjustment schema and native lowerer.
pub struct AdjustmentLanguageDefinition {
    pub schema: KindSchema,
    pub lowerer: AdjustmentLowerer,
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
    resolved_expr(declaration.get(name)?).ok_or_else(|| NativeLoweringError::InvalidPropertyType {
        property: name.to_string(),
        expected: "SQL expression".to_string(),
    })
}

pub fn resolved_expr(value: &ResolvedValue) -> Option<Expr> {
    match value {
        ResolvedValue::Boolean(value) => Some(lit(*value)),
        ResolvedValue::Integer(value) => Some(lit(*value)),
        ResolvedValue::Number(value) => Some(lit(*value)),
        ResolvedValue::String(value) => Some(lit(value.clone())),
        ResolvedValue::Scalar(value) => Some(lit(value.clone())),
        ResolvedValue::Expr(expr) => Some(expr.clone()),
        _ => None,
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
