//! Dependency-light metadata for Avenger's language-facing native surface.
//!
//! These types describe authoring declarations, not parser nodes and not the
//! compiled chart serialization format. Hosts compose entries explicitly and
//! serialize the resulting schema for documentation, validation, completion,
//! and compatibility profiles.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SchemaVersion {
    pub major: u32,
    pub minor: u32,
}

impl SchemaVersion {
    pub const V1: Self = Self { major: 1, minor: 0 };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeKindNamespace {
    Coordinate,
    Adjust,
    Mark,
    Transform,
    Tool,
    Widget,
    Scale,
    Axis,
    Legend,
    Layout,
    View,
    Resource,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NativeKindKey {
    pub namespace: NativeKindNamespace,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coordinate: Option<String>,
}

/// Family-specific aliases make registry signatures self-documenting while
/// preserving one compact schema representation.
pub type CoordinateSchema = KindSchema;
pub type AdjustSchema = KindSchema;
pub type MarkSchema = KindSchema;
pub type TransformSchema = KindSchema;
pub type ToolSchema = KindSchema;
pub type WidgetSchema = KindSchema;
pub type ScaleSchema = KindSchema;
pub type AxisSchema = KindSchema;
pub type LegendSchema = KindSchema;
pub type LayoutSchema = KindSchema;
pub type ViewSchema = KindSchema;
pub type ResourceSchema = KindSchema;

impl NativeKindKey {
    pub fn new(namespace: NativeKindNamespace, kind: impl Into<String>) -> Self {
        Self {
            namespace,
            kind: kind.into(),
            coordinate: None,
        }
    }

    pub fn mark(coordinate: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            namespace: NativeKindNamespace::Mark,
            kind: kind.into(),
            coordinate: Some(coordinate.into()),
        }
    }
}

/// Exact host-provided native module specifier.
///
/// Native modules are schema/export catalogs already linked into the host.
/// Constructing an ID never resolves, downloads, or loads executable code.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NativeModuleId(String);

impl NativeModuleId {
    pub fn new(value: impl Into<String>) -> Result<Self, SchemaError> {
        let value = value.into();
        if is_native_module_specifier(&value) {
            Ok(Self(value))
        } else {
            Err(SchemaError::InvalidNativeModuleId(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NativeModuleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for NativeModuleId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for NativeModuleId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

fn is_native_module_specifier(value: &str) -> bool {
    value
        .strip_prefix("native:")
        .is_some_and(|body| !body.is_empty() && !body.chars().any(char::is_whitespace))
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NativeModuleSchemaProfileId(String);

impl NativeModuleSchemaProfileId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable host-supplied identity for the executable implementation behind a
/// native module. Rust lowerer closures cannot be content-hashed, so hosts
/// must change this value whenever compile-affecting behavior changes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NativeModuleImplementationProfileId(String);

impl NativeModuleImplementationProfileId {
    pub fn new(value: impl Into<String>) -> Result<Self, SchemaError> {
        let value = value.into();
        if value.trim().is_empty() {
            Err(SchemaError::InvalidNativeModuleImplementationProfile)
        } else {
            Ok(Self(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeModuleExport {
    pub category: NativeKindNamespace,
    pub implementation: NativeKindKey,
    pub docs: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeModuleSchema {
    pub id: NativeModuleId,
    pub docs: String,
    pub exports: BTreeMap<String, NativeModuleExport>,
}

impl NativeModuleSchema {
    pub fn new(id: NativeModuleId, docs: impl Into<String>) -> Self {
        Self {
            id,
            docs: docs.into(),
            exports: BTreeMap::new(),
        }
    }

    pub fn add_export(
        &mut self,
        name: impl Into<String>,
        implementation: NativeKindKey,
        docs: impl Into<String>,
    ) -> Result<(), SchemaError> {
        let name = name.into();
        if self.exports.contains_key(&name) {
            return Err(SchemaError::DuplicateNativeExport {
                module: self.id.clone(),
                name,
            });
        }
        self.exports.insert(
            name,
            NativeModuleExport {
                category: implementation.namespace,
                implementation,
                docs: docs.into(),
            },
        );
        Ok(())
    }

    pub fn validate(
        &self,
        entries: &BTreeMap<NativeKindKey, KindSchema>,
    ) -> Result<(), SchemaError> {
        require_docs(&self.docs, format!("native module '{}'", self.id))?;
        if self.exports.is_empty() {
            return Err(SchemaError::EmptyNativeModule {
                module: self.id.clone(),
            });
        }
        for (name, export) in &self.exports {
            if !is_export_name(name) {
                return Err(SchemaError::InvalidNativeExportName {
                    module: self.id.clone(),
                    name: name.clone(),
                });
            }
            require_docs(
                &export.docs,
                format!("native module '{}' export '{name}'", self.id),
            )?;
            if export.category != export.implementation.namespace {
                return Err(SchemaError::NativeExportCategoryMismatch {
                    module: self.id.clone(),
                    name: name.clone(),
                    category: export.category,
                    implementation: export.implementation.namespace,
                });
            }
            if !entries.contains_key(&export.implementation) {
                return Err(SchemaError::MissingNativeExportImplementation {
                    module: self.id.clone(),
                    name: name.clone(),
                    implementation: export.implementation.clone(),
                });
            }
        }
        Ok(())
    }

    pub fn schema_profile(
        &self,
        entries: &BTreeMap<NativeKindKey, KindSchema>,
    ) -> Result<NativeModuleSchemaProfileId, SchemaError> {
        self.validate(entries)?;
        let implementations = self
            .exports
            .values()
            .map(|export| export.implementation.clone())
            .collect::<BTreeSet<_>>();
        let schemas = implementations
            .iter()
            .map(|key| {
                entries
                    .get(key)
                    .ok_or_else(|| SchemaError::MissingNativeExportImplementation {
                        module: self.id.clone(),
                        name: key.kind.clone(),
                        implementation: key.clone(),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let canonical = serde_json::to_vec(&(self, schemas)).map_err(SchemaError::Serialize)?;
        Ok(NativeModuleSchemaProfileId(format!(
            "avenger-native-module-schema-{:x}",
            Sha256::digest(canonical)
        )))
    }
}

fn is_export_name(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_alphabetic())
        && chars.all(|character| {
            character == '_' || character.is_alphabetic() || character.is_ascii_digit()
        })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", content = "detail", rename_all = "snake_case")]
pub enum ValueShape {
    Boolean,
    Integer,
    Number,
    String,
    /// A stable source identifier authored as a bare atom or string.
    Identifier,
    Atom {
        values: Vec<EnumValueSchema>,
    },
    SqlExpression,
    SqlProjection {
        policy: ProjectionPolicy,
        expression_mode: ProjectionExpressionMode,
    },
    SqlQuery,
    /// A channel configuration block with no authored data head. Native
    /// runtime values supply the data while scale/axis/legend metadata is
    /// authored normally.
    ChannelConfig,
    /// A required value head followed by a schema-owned configuration block.
    /// This models declarations such as facet dimensions, whose expression
    /// and configuration are one authored value but are not encoding channels.
    ConfiguredExpression(BTreeMap<String, PropertySchema>),
    /// A required typed declaration reference followed by schema-owned
    /// configuration, such as `tiles: osm { zindex: -10; }`.
    ConfiguredReference {
        namespaces: BTreeSet<NativeKindNamespace>,
        properties: BTreeMap<String, PropertySchema>,
    },
    /// A literal `pattern { ... }` or a configured expression whose scale
    /// range contains pattern values.
    PatternChannel,
    /// `shared`, `free`, or `level(<non-negative integer>)` transform scope.
    CoordinationScope,
    /// `filtered`, `broadcast`, or `level(<non-negative integer>)` mark data
    /// scope within the current facet tree.
    FacetDataScope,
    RasterDimension,
    /// A raster dimension handle with ordinary channel configuration.
    RasterDimensionChannel,
    ScalarBinding,
    TableBinding,
    /// A typed selection reference retained as a state handle.
    SelectionBinding,
    /// A widget item relation authored as inline `data.values` or another
    /// ordered data source.
    WidgetData,
    /// An ordered block of state mutations triggered by a parameter change.
    ///
    /// This is intentionally a semantic shape rather than an arbitrary
    /// object: the compiler preserves authored `set` order and lowers the
    /// block to one atomic `ChartAction` owned by the native declaration.
    StateActionBlock,
    /// A headless, propertyless block containing one or more mark
    /// declarations. Its declarations use the ordinary mark pipeline but
    /// remain local to the owning property.
    MarkBlock,
    TypedReference {
        namespaces: BTreeSet<NativeKindNamespace>,
    },
    /// Accept one of several explicitly documented shapes.
    Union(Vec<ValueShape>),
    /// Accept either one value of `detail` or an array of those values.
    OneOrMany(Box<ValueShape>),
    Array(Box<ValueShape>),
    /// An object with arbitrary keys and values of one shared shape.
    Map(Box<ValueShape>),
    /// An object whose user-named entries are configured encoding channels.
    ChannelMap,
    Object(BTreeMap<String, PropertySchema>),
    Any,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionPolicy {
    /// Every item must use an explicit `AS <name>` alias.
    Named,
    /// Direct column references may be unaliased; computed expressions must
    /// use an explicit `AS <name>` alias.
    Select,
}

/// SQL function families admitted by a projection-list property.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionExpressionMode {
    Scalar,
    Aggregate,
    Window,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnumValueSchema {
    pub value: String,
    pub docs: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PropertySchema {
    pub shape: ValueShape,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    pub docs: String,
}

impl PropertySchema {
    pub fn required(shape: ValueShape, docs: impl Into<String>) -> Self {
        Self {
            shape,
            required: true,
            default: None,
            docs: docs.into(),
        }
    }

    pub fn optional(shape: ValueShape, docs: impl Into<String>) -> Self {
        Self {
            shape,
            required: false,
            default: None,
            docs: docs.into(),
        }
    }

    pub fn with_default(mut self, default: impl Into<serde_json::Value>) -> Self {
        self.default = Some(default.into());
        self
    }
}

/// Coordinate-independent chart properties installed into every coordinate
/// authoring schema. Keeping this vocabulary in the schema crate lets native
/// and downstream coordinate packs share one documented chart surface.
pub fn chart_core_properties() -> BTreeMap<String, PropertySchema> {
    let time = BTreeMap::from([
        (
            "timezone".to_string(),
            PropertySchema::optional(ValueShape::String, "IANA timezone identifier."),
        ),
        (
            "week_start".to_string(),
            PropertySchema::optional(
                ValueShape::Atom {
                    values: [
                        "sunday",
                        "monday",
                        "tuesday",
                        "wednesday",
                        "thursday",
                        "friday",
                        "saturday",
                    ]
                    .into_iter()
                    .map(|value| EnumValueSchema {
                        value: value.to_string(),
                        docs: format!("Use {value} as the first day of the week."),
                    })
                    .collect(),
                },
                "First day used by weekly temporal operations.",
            ),
        ),
    ]);
    let format = BTreeMap::from([
        (
            "number_locale".to_string(),
            PropertySchema::optional(ValueShape::String, "Default number-format locale."),
        ),
        (
            "datetime_locale".to_string(),
            PropertySchema::optional(ValueShape::String, "Default date/time-format locale."),
        ),
        (
            "datetime_timezone".to_string(),
            PropertySchema::optional(ValueShape::String, "Default date/time display timezone."),
        ),
    ]);
    BTreeMap::from([
        (
            "data".to_string(),
            PropertySchema::optional(ValueShape::Any, "Chart-level data source."),
        ),
        (
            "title".to_string(),
            PropertySchema::optional(ValueShape::SqlExpression, "Chart title expression."),
        ),
        (
            "subtitle".to_string(),
            PropertySchema::optional(ValueShape::SqlExpression, "Chart subtitle expression."),
        ),
        (
            "layout".to_string(),
            PropertySchema::optional(
                ValueShape::Any,
                "Chart canvas, plot-area, and margin layout.",
            ),
        ),
        (
            "guide".to_string(),
            PropertySchema::optional(ValueShape::Any, "Coordinate-independent guide styling."),
        ),
        (
            "time".to_string(),
            PropertySchema::optional(ValueShape::Object(time), "Chart temporal defaults."),
        ),
        (
            "format".to_string(),
            PropertySchema::optional(ValueShape::Object(format), "Chart formatting defaults."),
        ),
    ])
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelSchema {
    pub name: String,
    pub required: bool,
    pub shape: ValueShape,
    /// Exact physical Arrow type exposed by `item.channel.<name>` after mark
    /// evaluation. `None` means the channel is not materialized in the mark's
    /// item frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_type: Option<String>,
    pub docs: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartSchema {
    pub alias: String,
    pub runtime_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_alias: Option<String>,
    pub targetable: bool,
    pub docs: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportSchema {
    pub alias: String,
    pub value_kind: String,
    /// Allocate and publish this generated export only when source actually
    /// references it. Existing-state binding exports are never lazy.
    #[serde(default)]
    pub lazy: bool,
    /// Optional declaration property that binds this export to existing state
    /// instead of asking the compiler to allocate generated state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_property: Option<String>,
    /// Optional declaration property that supplies the generated state's
    /// initial value when no existing-state binding is present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_property: Option<String>,
    pub docs: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyMode {
    /// The declaration accepts properties only.
    #[default]
    Properties,
    /// The declaration may also contain ordered children. Core placement and
    /// `child_rules` determine which child roles are legal.
    Mixed,
}

impl BodyMode {
    fn is_properties(&self) -> bool {
        *self == Self::Properties
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransformOutputSchema {
    pub name: String,
    pub shape: ValueShape,
    /// When set, the handle exists only when this declaration property is
    /// authored. The native lowerer must follow the same condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition_property: Option<String>,
    pub docs: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum DynamicOutputSource {
    /// Each object in an array property contributes the string stored in the
    /// configured field as an output handle.
    ArrayObjectField { property: String, field: String },
    /// Each string or atom in an array property contributes its value as an
    /// output-handle name.
    ArrayValueNames { property: String },
    /// Every explicit alias in a SQL projection-list property contributes an
    /// output handle with that exact name.
    ProjectionAliases { property: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DynamicTransformOutputSchema {
    pub source: DynamicOutputSource,
    pub shape: ValueShape,
    pub docs: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KindSchema {
    pub key: NativeKindKey,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_kind: Option<String>,
    pub docs: String,
    #[serde(default)]
    pub properties: BTreeMap<String, PropertySchema>,
    /// Optional schema for user-named properties not listed in `properties`.
    /// This supports native constructs such as calculate expressions and
    /// named aggregate measures without making other declarations open-ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_properties: Option<PropertySchema>,
    #[serde(default)]
    pub channels: BTreeMap<String, ChannelSchema>,
    #[serde(default)]
    pub parts: BTreeMap<String, PartSchema>,
    #[serde(default)]
    pub exports: BTreeMap<String, ExportSchema>,
    #[serde(default)]
    pub outputs: BTreeMap<String, TransformOutputSchema>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dynamic_outputs: Vec<DynamicTransformOutputSchema>,
    #[serde(default)]
    pub compatible_coordinates: BTreeSet<String>,
    /// Empty means the core language placement table applies without an
    /// additional native-kind restriction.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub allowed_parents: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BodyMode::is_properties")]
    pub body_mode: BodyMode,
    #[serde(default)]
    pub stateless: bool,
    #[serde(default)]
    pub child_rules: Vec<ChildRule>,
}

impl KindSchema {
    pub fn new(key: NativeKindKey, docs: impl Into<String>) -> Self {
        Self {
            key,
            runtime_kind: None,
            docs: docs.into(),
            properties: BTreeMap::new(),
            additional_properties: None,
            channels: BTreeMap::new(),
            parts: BTreeMap::new(),
            exports: BTreeMap::new(),
            outputs: BTreeMap::new(),
            dynamic_outputs: Vec::new(),
            compatible_coordinates: BTreeSet::new(),
            allowed_parents: BTreeSet::new(),
            body_mode: BodyMode::Properties,
            stateless: false,
            child_rules: Vec::new(),
        }
    }

    pub fn property(mut self, name: impl Into<String>, property: PropertySchema) -> Self {
        self.properties.insert(name.into(), property);
        self
    }

    pub fn additional_properties(mut self, property: PropertySchema) -> Self {
        self.additional_properties = Some(property);
        self
    }

    pub fn runtime_kind(mut self, runtime_kind: impl Into<String>) -> Self {
        self.runtime_kind = Some(runtime_kind.into());
        self
    }

    pub fn channel(mut self, channel: ChannelSchema) -> Self {
        self.channels.insert(channel.name.clone(), channel);
        self
    }

    pub fn part(mut self, part: PartSchema) -> Self {
        self.parts.insert(part.alias.clone(), part);
        self
    }

    pub fn export(mut self, export: ExportSchema) -> Self {
        self.exports.insert(export.alias.clone(), export);
        self
    }

    pub fn output(mut self, output: TransformOutputSchema) -> Self {
        self.outputs.insert(output.name.clone(), output);
        self
    }

    pub fn dynamic_output(mut self, output: DynamicTransformOutputSchema) -> Self {
        self.dynamic_outputs.push(output);
        self
    }

    pub fn allowed_parent(mut self, parent: impl Into<String>) -> Self {
        self.allowed_parents.insert(parent.into());
        self
    }

    pub fn body_mode(mut self, mode: BodyMode) -> Self {
        self.body_mode = mode;
        self
    }

    pub fn child_rule(mut self, rule: ChildRule) -> Self {
        self.child_rules.push(rule);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildRule {
    pub role: String,
    pub min: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<usize>,
    pub docs: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeclarationSchema {
    pub name: String,
    pub docs: String,
    pub allowed_children: Vec<NativeKindNamespace>,
    pub ordered_body: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NativeSchemaSnapshot {
    pub version: SchemaVersion,
    pub profile_label: String,
    #[serde(with = "entry_map")]
    pub entries: BTreeMap<NativeKindKey, KindSchema>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub modules: BTreeMap<NativeModuleId, NativeModuleSchema>,
}

mod entry_map {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

    use super::{KindSchema, NativeKindKey};

    pub fn serialize<S>(
        entries: &BTreeMap<NativeKindKey, KindSchema>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        entries.values().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<NativeKindKey, KindSchema>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let schemas = Vec::<KindSchema>::deserialize(deserializer)?;
        let mut entries = BTreeMap::new();
        for schema in schemas {
            let key = schema.key.clone();
            if entries.insert(key.clone(), schema).is_some() {
                return Err(D::Error::custom(format!(
                    "duplicate native schema entry {key:?}"
                )));
            }
        }
        Ok(entries)
    }
}

impl NativeSchemaSnapshot {
    pub fn canonical_json(&self) -> Result<Vec<u8>, SchemaError> {
        serde_json::to_vec(self).map_err(SchemaError::Serialize)
    }

    pub fn validate_docs(&self) -> Result<(), SchemaError> {
        for (key, schema) in &self.entries {
            require_docs(&schema.docs, format!("{key:?}"))?;
            for (name, property) in &schema.properties {
                require_docs(&property.docs, format!("{key:?} property '{name}'"))?;
                validate_shape_docs(&property.shape, format!("{key:?} property '{name}'"))?;
            }
            if let Some(property) = &schema.additional_properties {
                require_docs(&property.docs, format!("{key:?} additional properties"))?;
                validate_shape_docs(&property.shape, format!("{key:?} additional properties"))?;
            }
            for (name, channel) in &schema.channels {
                require_docs(&channel.docs, format!("{key:?} channel '{name}'"))?;
                validate_shape_docs(&channel.shape, format!("{key:?} channel '{name}'"))?;
            }
            for (name, part) in &schema.parts {
                require_docs(&part.docs, format!("{key:?} part '{name}'"))?;
            }
            for (name, export) in &schema.exports {
                require_docs(&export.docs, format!("{key:?} export '{name}'"))?;
            }
            for (name, output) in &schema.outputs {
                require_docs(&output.docs, format!("{key:?} output '{name}'"))?;
                validate_shape_docs(&output.shape, format!("{key:?} output '{name}'"))?;
            }
            for output in &schema.dynamic_outputs {
                require_docs(&output.docs, format!("{key:?} dynamic outputs"))?;
                validate_shape_docs(&output.shape, format!("{key:?} dynamic outputs"))?;
                if let DynamicOutputSource::ProjectionAliases { property } = &output.source
                    && !matches!(
                        schema
                            .properties
                            .get(property)
                            .map(|property| &property.shape),
                        Some(ValueShape::SqlProjection { .. })
                    )
                {
                    return Err(SchemaError::InvalidProjectionOutputSource {
                        key: key.clone(),
                        property: property.clone(),
                    });
                }
            }
            for child in &schema.child_rules {
                require_docs(&child.docs, format!("{key:?} child role '{}'", child.role))?;
            }
        }
        for (id, module) in &self.modules {
            if id != &module.id {
                return Err(SchemaError::NativeModuleKeyMismatch {
                    key: id.clone(),
                    module: module.id.clone(),
                });
            }
            module.validate(&self.entries)?;
        }
        Ok(())
    }

    pub fn native_module(
        &self,
        id: &NativeModuleId,
    ) -> Option<(&NativeModuleSchema, NativeModuleSchemaProfileId)> {
        let module = self.modules.get(id)?;
        let profile = module.schema_profile(&self.entries).ok()?;
        Some((module, profile))
    }

    /// Deterministic documentation data generated from the same entries used
    /// for validation and registry lowering.
    pub fn markdown_reference(&self) -> String {
        let mut output = format!(
            "# Avenger native schema: {}\n\nLanguage schema {}.{}.\n",
            self.profile_label, self.version.major, self.version.minor
        );
        if !self.modules.is_empty() {
            output.push_str("\n## Native modules\n");
            for (id, module) in &self.modules {
                output.push_str(&format!("\n### `{id}`\n\n{}\n", module.docs));
                output.push_str(
                    "\n| Export | Category | Implementation | Description |\n\
                     |---|---|---|---|\n",
                );
                for (name, export) in &module.exports {
                    output.push_str(&format!(
                        "| `{name}` | `{:?}` | `{:?}.{}{}` | {} |\n",
                        export.category,
                        export.implementation.namespace,
                        export
                            .implementation
                            .coordinate
                            .as_ref()
                            .map(|coordinate| format!("{coordinate}."))
                            .unwrap_or_default(),
                        export.implementation.kind,
                        export.docs
                    ));
                }
            }
        }
        for (key, schema) in &self.entries {
            output.push_str(&format!(
                "\n## `{:?}.{}{}`\n\n{}\n",
                key.namespace,
                key.coordinate
                    .as_ref()
                    .map(|coordinate| format!("{coordinate}."))
                    .unwrap_or_default(),
                key.kind,
                schema.docs
            ));
            if !schema.properties.is_empty()
                || schema.additional_properties.is_some()
                || !schema.channels.is_empty()
            {
                output.push_str("\n| Name | Role | Required | Description |\n|---|---|---:|---|\n");
                for (name, property) in &schema.properties {
                    output.push_str(&format!(
                        "| `{name}` | property | {} | {} |\n",
                        property.required, property.docs
                    ));
                }
                if let Some(property) = &schema.additional_properties {
                    output.push_str(&format!(
                        "| `*` | user-named property | {} | {} |\n",
                        property.required, property.docs
                    ));
                }
                for (name, channel) in &schema.channels {
                    output.push_str(&format!(
                        "| `{name}` | channel | {} | {} |\n",
                        channel.required, channel.docs
                    ));
                }
            }
            if !schema.exports.is_empty() {
                output.push_str("\nExports:\n");
                for export in schema.exports.values() {
                    output.push_str(&format!(
                        "\n- `{}` (`{}`): {}\n",
                        export.alias, export.value_kind, export.docs
                    ));
                }
            }
            if !schema.dynamic_outputs.is_empty() {
                output.push_str("\nDynamic transform outputs:\n");
                for dynamic in &schema.dynamic_outputs {
                    output.push_str(&format!("\n- {:?}: {}\n", dynamic.source, dynamic.docs));
                }
            }
        }
        output
    }
}

fn validate_shape_docs(shape: &ValueShape, context: String) -> Result<(), SchemaError> {
    match shape {
        ValueShape::Atom { values } => {
            for value in values {
                require_docs(&value.docs, format!("{context} enum '{}'", value.value))?;
            }
        }
        ValueShape::OneOrMany(inner) | ValueShape::Array(inner) | ValueShape::Map(inner) => {
            validate_shape_docs(inner, context)?
        }
        ValueShape::Union(shapes) => {
            for shape in shapes {
                validate_shape_docs(shape, context.clone())?;
            }
        }
        ValueShape::Object(properties) => {
            for (name, property) in properties {
                require_docs(&property.docs, format!("{context} field '{name}'"))?;
                validate_shape_docs(&property.shape, format!("{context} field '{name}'"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn require_docs(docs: &str, context: String) -> Result<(), SchemaError> {
    if docs.trim().is_empty() {
        Err(SchemaError::MissingDocs { context })
    } else {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("language schema is missing documentation for {context}")]
    MissingDocs { context: String },
    #[error("invalid native module ID '{0}'; expected an exact nonempty native: specifier")]
    InvalidNativeModuleId(String),
    #[error("native module implementation profile must not be empty")]
    InvalidNativeModuleImplementationProfile,
    #[error("native module '{module}' has no exports")]
    EmptyNativeModule { module: NativeModuleId },
    #[error("native module '{module}' has invalid export name '{name}'")]
    InvalidNativeExportName {
        module: NativeModuleId,
        name: String,
    },
    #[error("native module '{module}' has duplicate export '{name}'")]
    DuplicateNativeExport {
        module: NativeModuleId,
        name: String,
    },
    #[error(
        "native module '{module}' export '{name}' declares {category:?} but points to {implementation:?}"
    )]
    NativeExportCategoryMismatch {
        module: NativeModuleId,
        name: String,
        category: NativeKindNamespace,
        implementation: NativeKindNamespace,
    },
    #[error(
        "native module '{module}' export '{name}' points to missing implementation {implementation:?}"
    )]
    MissingNativeExportImplementation {
        module: NativeModuleId,
        name: String,
        implementation: NativeKindKey,
    },
    #[error(
        "native schema {key:?} derives projection aliases from non-projection property '{property}'"
    )]
    InvalidProjectionOutputSource {
        key: NativeKindKey,
        property: String,
    },
    #[error("native module map key '{key}' does not match embedded module ID '{module}'")]
    NativeModuleKeyMismatch {
        key: NativeModuleId,
        module: NativeModuleId,
    },
    #[error("failed to serialize language schema: {0}")]
    Serialize(serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_snapshot_order_is_independent_of_registration_order() {
        let left = KindSchema::new(
            NativeKindKey::new(NativeKindNamespace::Coordinate, "left"),
            "Left coordinate.",
        );
        let right = KindSchema::new(
            NativeKindKey::new(NativeKindNamespace::Coordinate, "right"),
            "Right coordinate.",
        );
        let snapshot = |entries: Vec<KindSchema>| NativeSchemaSnapshot {
            version: SchemaVersion::V1,
            profile_label: "test".to_string(),
            entries: entries
                .into_iter()
                .map(|entry| (entry.key.clone(), entry))
                .collect(),
            modules: BTreeMap::new(),
        };
        assert_eq!(
            snapshot(vec![left.clone(), right.clone()])
                .canonical_json()
                .unwrap(),
            snapshot(vec![right, left]).canonical_json().unwrap()
        );
    }

    #[test]
    fn documentation_lint_reaches_enum_values() {
        let schema = KindSchema::new(
            NativeKindKey::new(NativeKindNamespace::Layout, "flow"),
            "Flow layout.",
        )
        .property(
            "direction",
            PropertySchema::required(
                ValueShape::Atom {
                    values: vec![EnumValueSchema {
                        value: "row".to_string(),
                        docs: String::new(),
                    }],
                },
                "Flow direction.",
            ),
        );
        let snapshot = NativeSchemaSnapshot {
            version: SchemaVersion::V1,
            profile_label: "test".to_string(),
            entries: [(schema.key.clone(), schema)].into_iter().collect(),
            modules: BTreeMap::new(),
        };
        assert!(matches!(
            snapshot.validate_docs(),
            Err(SchemaError::MissingDocs { .. })
        ));
    }

    #[test]
    fn additional_properties_are_documented_and_canonical() {
        let schema = KindSchema::new(
            NativeKindKey::new(NativeKindNamespace::Transform, "calculate"),
            "Add user-named expressions.",
        )
        .additional_properties(PropertySchema::optional(
            ValueShape::SqlExpression,
            "A user-named output expression.",
        ));
        let snapshot = NativeSchemaSnapshot {
            version: SchemaVersion::V1,
            profile_label: "test".to_string(),
            entries: [(schema.key.clone(), schema)].into_iter().collect(),
            modules: BTreeMap::new(),
        };
        snapshot.validate_docs().unwrap();
        assert!(snapshot.markdown_reference().contains("`*`"));
        assert!(
            String::from_utf8(snapshot.canonical_json().unwrap())
                .unwrap()
                .contains("additional_properties")
        );
    }

    #[test]
    fn native_module_ids_are_exact_nonempty_specifiers() {
        assert!(NativeModuleId::new("native:com.acme.visuals@1").is_ok());
        for invalid in ["", "com.acme.visuals@1", "native:", "native:has space"] {
            assert!(
                NativeModuleId::new(invalid).is_err(),
                "accepted invalid native module ID {invalid:?}"
            );
        }
    }

    #[test]
    fn native_module_schema_profile_ignores_unrelated_entries() {
        let implementation = NativeKindKey::mark("cartesian", "acme_hexbin");
        let mut module = NativeModuleSchema::new(
            NativeModuleId::new("native:com.acme.visuals@1").unwrap(),
            "Acme visual extensions.",
        );
        module
            .add_export(
                "hexbin",
                implementation.clone(),
                "Aggregate points into hexagonal bins.",
            )
            .unwrap();
        let mut entries = BTreeMap::from([(
            implementation.clone(),
            KindSchema::new(implementation, "Acme hexbin mark."),
        )]);
        let first = module.schema_profile(&entries).unwrap();
        let unrelated = NativeKindKey::new(NativeKindNamespace::Tool, "unrelated");
        entries.insert(
            unrelated.clone(),
            KindSchema::new(unrelated, "Unrelated tool."),
        );
        let second = module.schema_profile(&entries).unwrap();
        assert_eq!(first, second);

        let snapshot = NativeSchemaSnapshot {
            version: SchemaVersion::V1,
            profile_label: "test".to_string(),
            entries,
            modules: [(module.id.clone(), module)].into_iter().collect(),
        };
        let reference = snapshot.markdown_reference();
        assert!(reference.contains("## Native modules"));
        assert!(reference.contains("`native:com.acme.visuals@1`"));
        assert!(reference.contains("`hexbin`"));
    }

    #[test]
    fn native_module_exports_validate_category_docs_and_implementation() {
        let implementation = NativeKindKey::mark("cartesian", "acme_hexbin");
        let entries = BTreeMap::from([(
            implementation.clone(),
            KindSchema::new(implementation.clone(), "Acme hexbin mark."),
        )]);
        let id = NativeModuleId::new("native:com.acme.visuals@1").unwrap();

        let mut valid = NativeModuleSchema::new(id.clone(), "Acme visuals.");
        valid
            .add_export("hexbin", implementation.clone(), "Hexbin mark.")
            .unwrap();
        valid.validate(&entries).unwrap();
        assert!(matches!(
            valid.add_export("hexbin", implementation.clone(), "Duplicate."),
            Err(SchemaError::DuplicateNativeExport { .. })
        ));

        let mut wrong_category = valid.clone();
        wrong_category.exports.get_mut("hexbin").unwrap().category = NativeKindNamespace::Transform;
        assert!(matches!(
            wrong_category.validate(&entries),
            Err(SchemaError::NativeExportCategoryMismatch { .. })
        ));

        let mut missing = NativeModuleSchema::new(id, "Acme visuals.");
        missing
            .add_export(
                "missing",
                NativeKindKey::new(NativeKindNamespace::Tool, "missing"),
                "Missing tool.",
            )
            .unwrap();
        assert!(matches!(
            missing.validate(&entries),
            Err(SchemaError::MissingNativeExportImplementation { .. })
        ));
    }
}
