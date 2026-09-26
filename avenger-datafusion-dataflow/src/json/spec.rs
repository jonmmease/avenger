use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

// BTreeMap's default deserializer overwrites duplicate names. Definition and binding
// maps must reject them, including row objects.
fn unique<'de, D, T>(d: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Visitor<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "an object with unique names")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut out = BTreeMap::new();
            while let Some((k, v)) = map.next_entry::<String, T>()? {
                if out.insert(k.clone(), v).is_some() {
                    return Err(serde::de::Error::custom(format!("duplicate name {k}")));
                }
            }
            Ok(out)
        }
    }
    d.deserialize_map(Visitor(std::marker::PhantomData))
}

/// Version 1 authoring document. Parsing it performs no source access.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DataflowSpec {
    #[schemars(range(min = 1, max = 1))]
    pub version: u32,
    #[schemars(regex(pattern = "^datafusion$"))]
    pub dialect: String,
    #[serde(default, deserialize_with = "unique")]
    pub inputs: BTreeMap<String, InputSpec>,
    #[serde(default, deserialize_with = "unique")]
    pub sources: BTreeMap<String, SourceSpec>,
    #[serde(default, deserialize_with = "unique")]
    pub tables: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "unique")]
    pub scalars: BTreeMap<String, String>,
    #[serde(default)]
    pub outputs: OutputSpec,
    #[serde(default, deserialize_with = "unique")]
    pub scopes: BTreeMap<String, ScopeSpec>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScopeSpec {
    pub partition: PartitionSpec,
    pub rows: String,
    #[serde(default, deserialize_with = "unique")]
    pub inputs: BTreeMap<String, InputSpec>,
    #[serde(default, deserialize_with = "unique")]
    pub sources: BTreeMap<String, SourceSpec>,
    #[serde(default, deserialize_with = "unique")]
    pub tables: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "unique")]
    pub scalars: BTreeMap<String, String>,
    #[serde(default)]
    pub outputs: OutputSpec,
    #[serde(default, deserialize_with = "unique")]
    pub scopes: BTreeMap<String, ScopeSpec>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartitionSpec {
    pub source: String,
    pub keys: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputSpec {
    Expr {
        #[serde(rename = "type")]
        data_type: TypeSpec,
    },
    Scalar {
        #[serde(rename = "type")]
        data_type: TypeSpec,
    },
    Table {
        schema: Vec<FieldSpec>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FieldSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub data_type: TypeSpec,
    pub nullable: bool,
}
/// Arrow primitive types supported by the version 1 JSON adapter.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TypeSpec {
    Null,
    Boolean,
    Int8,
    Int16,
    Int32,
    Int64,
    Uint8,
    Uint16,
    Uint32,
    Uint64,
    Float32,
    Float64,
    Utf8,
    LargeUtf8,
    Date32,
    Date64,
    Timestamp(TimestampSpec),
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimestampSpec {
    pub unit: TimeUnitSpec,
    pub timezone: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TimeUnitSpec {
    S,
    Ms,
    Us,
    Ns,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Row(#[serde(deserialize_with = "unique")] pub BTreeMap<String, Value>);
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SourceSpec {
    Inline(InlineSource),
    File(FileSource),
    Asset(AssetSource),
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InlineSource {
    pub schema: Vec<FieldSpec>,
    pub values: Vec<Row>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileSource {
    pub url: String,
    pub format: FileFormat,
    pub schema: Option<Vec<FieldSpec>>,
    #[serde(default)]
    pub options: FileOptions,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileFormat {
    Csv,
    Parquet,
    Arrow,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileOptions {
    pub has_header: Option<bool>,
    pub delimiter: Option<String>,
    pub schema_infer_max_records: Option<usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetSource {
    pub asset: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputSpec {
    #[serde(default, deserialize_with = "unique")]
    pub tables: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "unique")]
    pub scalars: BTreeMap<String, String>,
}
/// Bindings and output selection for one independent query.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryRequest {
    #[serde(default)]
    pub bindings: RequestBindings,
    #[serde(default)]
    pub outputs: OutputSelection,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestBindings {
    #[serde(default, deserialize_with = "unique")]
    pub scalars: BTreeMap<String, Value>,
    #[serde(default, deserialize_with = "unique")]
    pub exprs: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "unique")]
    pub tables: BTreeMap<String, TableBinding>,
    #[serde(default)]
    pub scope_defaults: Vec<ScopeBindings>,
    #[serde(default)]
    pub overrides: Vec<InstanceBindings>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum TableBinding {
    Inline(InlineBinding),
    Asset(AssetSource),
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InlineBinding {
    pub values: Vec<Row>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScopeBindings {
    pub scope: Vec<String>,
    #[serde(default, deserialize_with = "unique")]
    pub scalars: BTreeMap<String, Value>,
    #[serde(default, deserialize_with = "unique")]
    pub exprs: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "unique")]
    pub tables: BTreeMap<String, TableBinding>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstanceBindings {
    pub path: Vec<InstanceAddress>,
    #[serde(default, deserialize_with = "unique")]
    pub scalars: BTreeMap<String, Value>,
    #[serde(default, deserialize_with = "unique")]
    pub exprs: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "unique")]
    pub tables: BTreeMap<String, TableBinding>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstanceAddress {
    pub scope: String,
    pub key: Vec<Value>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputSelection {
    #[serde(default)]
    pub tables: Vec<OutputAddress>,
    #[serde(default)]
    pub scalars: Vec<OutputAddress>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputAddress {
    #[serde(default)]
    pub scope: Vec<String>,
    pub output: String,
}
