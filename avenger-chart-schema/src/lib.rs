//! Dependency-light metadata for Avenger's language-facing native surface.
//!
//! These types describe authoring declarations, not parser nodes and not the
//! compiled chart serialization format. Hosts compose entries explicitly and
//! serialize the resulting schema for documentation, validation, completion,
//! and compatibility profiles.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

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
    Mark,
    Transform,
    Tool,
    Widget,
    Scale,
    Axis,
    Legend,
    Layout,
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
pub type MarkSchema = KindSchema;
pub type TransformSchema = KindSchema;
pub type ToolSchema = KindSchema;
pub type WidgetSchema = KindSchema;
pub type ScaleSchema = KindSchema;
pub type AxisSchema = KindSchema;
pub type LegendSchema = KindSchema;
pub type LayoutSchema = KindSchema;

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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", content = "detail", rename_all = "snake_case")]
pub enum ValueShape {
    Boolean,
    Integer,
    Number,
    String,
    Atom {
        values: Vec<EnumValueSchema>,
    },
    SqlExpression,
    SqlQuery,
    RasterDimension,
    ScalarBinding,
    TableBinding,
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
    Object(BTreeMap<String, PropertySchema>),
    Any,
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelSchema {
    pub name: String,
    pub required: bool,
    pub shape: ValueShape,
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
    /// Every user-named property except the listed configuration properties
    /// contributes an output handle with the same name.
    PropertyNames {
        #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
        exclude: BTreeSet<String>,
    },
    /// Each object in an array property contributes the string stored in the
    /// configured field as an output handle.
    ArrayObjectField { property: String, field: String },
    /// Each string or atom in an array property contributes its value as an
    /// output-handle name.
    ArrayValueNames { property: String },
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
            }
            for child in &schema.child_rules {
                require_docs(&child.docs, format!("{key:?} child role '{}'", child.role))?;
            }
        }
        Ok(())
    }

    /// Deterministic documentation data generated from the same entries used
    /// for validation and registry lowering.
    pub fn markdown_reference(&self) -> String {
        let mut output = format!(
            "# Avenger native schema: {}\n\nLanguage schema {}.{}.\n",
            self.profile_label, self.version.major, self.version.minor
        );
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
        };
        snapshot.validate_docs().unwrap();
        assert!(snapshot.markdown_reference().contains("`*`"));
        assert!(
            String::from_utf8(snapshot.canonical_json().unwrap())
                .unwrap()
                .contains("additional_properties")
        );
    }
}
