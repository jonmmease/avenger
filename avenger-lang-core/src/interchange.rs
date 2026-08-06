//! Versioned JSON interchange encoding for the stable semantic AST.

use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor},
    ser::SerializeMap,
};

use crate::ast::{
    AstError, BindingKind, BindingTime, Body, Decl, File, Import, ImportClause, ModuleItem, Name,
    NumericLiteral, PropertyMap, RefKind, SqlExpression, SqlQuery, Value, Visibility,
};

pub const CORE_SCHEMA_V1: &str = include_str!("../schemas/ast-core-1.json");

impl<'de> Deserialize<'de> for File {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Payload {
            version: u32,
            #[serde(default)]
            imports: Vec<Import>,
            items: Vec<ModuleItem>,
        }

        let payload = Payload::deserialize(deserializer)?;
        if payload.version != crate::LANGUAGE_MAJOR {
            return Err(de::Error::custom(format!(
                "unsupported Avenger language major {}; expected {}",
                payload.version,
                crate::LANGUAGE_MAJOR
            )));
        }
        if payload.items.is_empty() {
            return Err(de::Error::custom(
                "Avenger modules must contain at least one module item",
            ));
        }
        for item in &payload.items {
            validate_module_item(item).map_err(de::Error::custom)?;
        }
        Ok(Self {
            version: payload.version,
            imports: payload.imports,
            items: payload.items,
        })
    }
}

pub fn decode_json(source: &str) -> Result<File, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(source);
    let file = File::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(file)
}

pub fn encode_json(file: &File) -> Result<String, serde_json::Error> {
    serde_json::to_string(file)
}

pub fn canonical_json(file: &File) -> Result<String, serde_json::Error> {
    let value = serde_json::to_value(file)?;
    let mut output = String::new();
    write_canonical(&value, &mut output)?;
    Ok(output)
}

fn write_canonical(
    value: &serde_json::Value,
    output: &mut String,
) -> Result<(), serde_json::Error> {
    match value {
        serde_json::Value::Null => output.push_str("null"),
        serde_json::Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        serde_json::Value::Number(value) => output.push_str(&value.to_string()),
        serde_json::Value::String(value) => output.push_str(&serde_json::to_string(value)?),
        serde_json::Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical(value, output)?;
            }
            output.push(']');
        }
        serde_json::Value::Object(values) => {
            output.push('{');
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.encode_utf16().cmp(right.encode_utf16()));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key)?);
                output.push(':');
                write_canonical(value, output)?;
            }
            output.push('}');
        }
    }
    Ok(())
}

impl<'de> Deserialize<'de> for Import {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Payload {
            source: String,
            #[serde(default)]
            sha256: Option<String>,
            clause: ImportClause,
        }

        let payload = Payload::deserialize(deserializer)?;
        if payload.source.is_empty() {
            return Err(de::Error::custom("import specifier must not be empty"));
        }
        if payload.sha256.as_deref().is_some_and(|hash| {
            hash.len() != 64
                || !hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }) {
            return Err(de::Error::custom(
                "import sha256 must contain exactly 64 lowercase hexadecimal digits",
            ));
        }
        if matches!(&payload.clause, ImportClause::Named(specifiers) if specifiers.is_empty()) {
            return Err(de::Error::custom(
                "named import clauses must contain at least one specifier",
            ));
        }
        Ok(Self {
            source: payload.source,
            sha256: payload.sha256,
            clause: payload.clause,
        })
    }
}

fn validate_module_item(item: &ModuleItem) -> Result<(), AstError> {
    let declaration = &item.declaration;
    if declaration.visibility != Visibility::Default {
        return Err(AstError::InvalidInterchange(
            "top-level module items cannot carry component visibility".to_string(),
        ));
    }
    let valid = match declaration.keyword.as_str() {
        "chart" => declaration.kind.is_some(),
        "define" => {
            declaration.name.is_some()
                && matches!(
                    declaration
                        .kind
                        .as_ref()
                        .and_then(|kind| kind.simple())
                        .map(Name::as_str),
                    Some("mark" | "tool" | "transform")
                )
        }
        "table" | "schema" | "catalog" => declaration.kind.is_some() && declaration.name.is_some(),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(AstError::InvalidInterchange(format!(
            "declaration `{}` is not a valid top-level module item",
            declaration.keyword
        )))
    }
}

impl<'de> Deserialize<'de> for Visibility {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match String::deserialize(deserializer)?.as_str() {
            "private" => Ok(Self::Private),
            "public" => Ok(Self::Public),
            value => Err(de::Error::unknown_variant(value, &["private", "public"])),
        }
    }
}

impl<'de> Deserialize<'de> for BindingTime {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match String::deserialize(deserializer)?.as_str() {
            "start" => Ok(Self::Start),
            "previous" => Ok(Self::Previous),
            value => Err(de::Error::unknown_variant(value, &["start", "previous"])),
        }
    }
}

impl<'de> Deserialize<'de> for PropertyMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PropertiesVisitor;

        impl<'de> Visitor<'de> for PropertiesVisitor {
            type Value = PropertyMap;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object with unique Avenger property names")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut properties = PropertyMap::default();
                while let Some((name, value)) = map.next_entry::<Name, Value>()? {
                    properties.insert(name, value).map_err(de::Error::custom)?;
                }
                Ok(properties)
            }
        }

        deserializer.deserialize_map(PropertiesVisitor)
    }
}

impl Serialize for Value {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Str(value) => serializer.serialize_str(value),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Null => serializer.serialize_none(),
            Self::Array(values) => values.serialize(serializer),
            Self::Num(value) => tagged(serializer, "num", value.as_str()),
            Self::Column(value) => tagged(serializer, "col", value),
            Self::Atom(value) => tagged(serializer, "atom", value),
            Self::Expr(value) => tagged(serializer, "expr", &value.canonical_sql()),
            Self::Projection(value) => tagged(serializer, "projection", &value.canonical_sql()),
            Self::Query(value) => tagged(serializer, "query", &value.canonical_sql()),
            Self::Relation(path) => tagged(serializer, "relation", &dotted_path(path)),
            Self::Binding { kind, path, time } => tagged(
                serializer,
                "binding",
                &BindingPayloadRef {
                    kind: *kind,
                    path,
                    time: *time,
                },
            ),
            Self::Ref { kind, path } => {
                tagged(serializer, "ref", &RefPayloadRef { kind: *kind, path })
            }
            Self::Channel { mode, expression } => tagged(serializer, mode.as_str(), expression),
            Self::Dim(path) => tagged(serializer, "dim", &dotted_path(path)),
            Self::Pattern(value) => tagged(serializer, "pattern", value),
            Self::Env(value) => tagged(serializer, "env", value),
            Self::None => tagged(serializer, "none", &true),
            Self::Block { head, body } => tagged(
                serializer,
                "block",
                &BlockPayloadRef {
                    head: head.as_deref(),
                    body,
                },
            ),
            Self::Call { function, args } => {
                tagged(serializer, "call", &CallPayloadRef { function, args })
            }
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ValueVisitor;

        impl<'de> Visitor<'de> for ValueVisitor {
            type Value = Value;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(
                    "a string, boolean, null, value array, or single-key tagged value object",
                )
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Value::Str(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Value::Str(value))
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Value::Bool(value))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Value::Null)
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Value::Null)
            }

            fn visit_seq<A>(self, sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                Vec::<Value>::deserialize(de::value::SeqAccessDeserializer::new(sequence))
                    .map(Value::Array)
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let tag = map
                    .next_key::<String>()?
                    .ok_or_else(|| de::Error::custom("tagged value object must not be empty"))?;
                let value = match tag.as_str() {
                    "num" => {
                        let value = map.next_value::<String>()?;
                        if !is_interchange_number(&value) {
                            return Err(de::Error::custom("invalid tagged numeric spelling"));
                        }
                        Value::Num(NumericLiteral::new(&value).map_err(de::Error::custom)?)
                    }
                    "col" => {
                        let value = map.next_value::<String>()?;
                        require_nonempty("col", &value).map_err(de::Error::custom)?;
                        Value::Column(value)
                    }
                    "atom" => Value::Atom(map.next_value()?),
                    "binding" => map
                        .next_value::<BindingPayload>()?
                        .try_into()
                        .map_err(de::Error::custom)?,
                    "expr" => Value::Expr(Box::new(
                        SqlExpression::parse(&map.next_value::<String>()?)
                            .map_err(de::Error::custom)?,
                    )),
                    "projection" => Value::Projection(Box::new(
                        crate::ast::SqlProjection::parse(&map.next_value::<String>()?)
                            .map_err(de::Error::custom)?,
                    )),
                    "query" => Value::Query(Box::new(
                        SqlQuery::parse(&map.next_value::<String>()?).map_err(de::Error::custom)?,
                    )),
                    "relation" => Value::Relation(
                        parse_dotted_path(&map.next_value::<String>()?)
                            .map_err(de::Error::custom)?,
                    ),
                    "encoded" => Value::Channel {
                        mode: crate::ast::ChannelMode::Encoded,
                        expression: Box::new(map.next_value()?),
                    },
                    "direct" => Value::Channel {
                        mode: crate::ast::ChannelMode::Direct,
                        expression: Box::new(map.next_value()?),
                    },
                    "dim" => Value::Dim(
                        parse_dotted_dim(&map.next_value::<String>()?)
                            .map_err(de::Error::custom)?,
                    ),
                    "ref" => map
                        .next_value::<RefPayload>()?
                        .try_into()
                        .map_err(de::Error::custom)?,
                    "pattern" => Value::Pattern(Box::new(map.next_value()?)),
                    "env" => {
                        let value = map.next_value::<String>()?;
                        require_nonempty("env", &value).map_err(de::Error::custom)?;
                        Value::Env(value)
                    }
                    "none" => {
                        if !map.next_value::<bool>()? {
                            return Err(de::Error::custom("`none` must be exactly true"));
                        }
                        Value::None
                    }
                    "block" => map.next_value::<BlockPayload>()?.into_value(),
                    "call" => map.next_value::<CallPayload>()?.into_value(),
                    _ => {
                        let _ = map.next_value::<IgnoredAny>()?;
                        return Err(de::Error::unknown_field(&tag, VALUE_TAGS));
                    }
                };
                if map.next_key::<IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom(
                        "tagged value object must contain exactly one member",
                    ));
                }
                Ok(value)
            }

            fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Err(E::custom(
                    "plain JSON numbers are not Avenger values; use `num`",
                ))
            }

            fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Err(E::custom(
                    "plain JSON numbers are not Avenger values; use `num`",
                ))
            }

            fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Err(E::custom(
                    "plain JSON numbers are not Avenger values; use `num`",
                ))
            }
        }

        deserializer.deserialize_any(ValueVisitor)
    }
}

const VALUE_TAGS: &[&str] = &[
    "num",
    "col",
    "atom",
    "binding",
    "expr",
    "projection",
    "query",
    "relation",
    "encoded",
    "direct",
    "dim",
    "ref",
    "pattern",
    "env",
    "none",
    "block",
    "call",
];

fn tagged<S, T>(serializer: S, tag: &'static str, payload: &T) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: ?Sized + Serialize,
{
    let mut map = serializer.serialize_map(Some(1))?;
    map.serialize_entry(tag, payload)?;
    map.end()
}

#[derive(Serialize)]
struct BindingPayloadRef<'a> {
    kind: BindingKind,
    #[serde(serialize_with = "serialize_binding_path")]
    path: &'a [Name],
    #[serde(default, skip_serializing_if = "BindingTime::is_current")]
    time: BindingTime,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BindingPayload {
    kind: BindingKind,
    path: BindingPath,
    #[serde(default)]
    time: BindingTime,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BindingPath {
    Compact(Name),
    Qualified(Vec<Name>),
}

impl TryFrom<BindingPayload> for Value {
    type Error = AstError;

    fn try_from(payload: BindingPayload) -> Result<Self, Self::Error> {
        let path = match payload.path {
            BindingPath::Compact(name) => vec![name],
            BindingPath::Qualified(path) if path.len() >= 2 => path,
            BindingPath::Qualified(_) => {
                return Err(AstError::InvalidInterchange(
                    "qualified binding path must contain at least two names".into(),
                ));
            }
        };
        if payload.kind == BindingKind::Store && payload.time != BindingTime::Current {
            return Err(AstError::InvalidInterchange(
                "store bindings cannot use temporal versions".into(),
            ));
        }
        Ok(Self::Binding {
            kind: payload.kind,
            path,
            time: payload.time,
        })
    }
}

fn serialize_binding_path<S>(path: &&[Name], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let [name] = *path {
        name.serialize(serializer)
    } else {
        path.serialize(serializer)
    }
}

#[derive(Serialize)]
struct RefPayloadRef<'a> {
    kind: RefKind,
    path: &'a [Name],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RefPayload {
    kind: RefKind,
    path: Vec<Name>,
}

impl TryFrom<RefPayload> for Value {
    type Error = AstError;

    fn try_from(payload: RefPayload) -> Result<Self, Self::Error> {
        if payload.path.is_empty() {
            return Err(AstError::InvalidInterchange(
                "reference path must contain at least one name".into(),
            ));
        }
        Ok(Self::Ref {
            kind: payload.kind,
            path: payload.path,
        })
    }
}

#[derive(Serialize)]
struct BlockPayloadRef<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    head: Option<&'a Value>,
    #[serde(flatten)]
    body: &'a Body,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockPayload {
    #[serde(default)]
    head: Option<Box<Value>>,
    #[serde(default)]
    props: PropertyMap,
    #[serde(default)]
    children: Vec<Decl>,
}

impl BlockPayload {
    fn into_value(self) -> Value {
        Value::Block {
            head: self.head,
            body: Body {
                props: self.props,
                children: self.children,
            },
        }
    }
}

#[derive(Serialize)]
struct CallPayloadRef<'a> {
    #[serde(rename = "fn")]
    function: &'a Name,
    #[serde(skip_serializing_if = "<[Value]>::is_empty")]
    args: &'a [Value],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallPayload {
    #[serde(rename = "fn")]
    function: Name,
    #[serde(default)]
    args: Vec<Value>,
}

impl CallPayload {
    fn into_value(self) -> Value {
        Value::Call {
            function: self.function,
            args: self.args,
        }
    }
}

fn dotted_path(path: &[Name]) -> String {
    path.iter().map(Name::as_str).collect::<Vec<_>>().join(".")
}

fn parse_dotted_dim(value: &str) -> Result<Vec<Name>, AstError> {
    let path = parse_dotted_path(value)?;
    if path.len() != 2 {
        return Err(AstError::InvalidInterchange(
            "dimension path must contain exactly two names".into(),
        ));
    }
    Ok(path)
}

fn parse_dotted_path(value: &str) -> Result<Vec<Name>, AstError> {
    let path = value
        .split('.')
        .map(Name::new)
        .collect::<Result<Vec<_>, _>>()?;
    if path.is_empty() {
        return Err(AstError::InvalidInterchange(
            "relation path must contain at least one name".into(),
        ));
    }
    Ok(path)
}

fn require_nonempty(tag: &str, value: &str) -> Result<(), AstError> {
    if value.is_empty() {
        Err(AstError::InvalidInterchange(format!(
            "`{tag}` payload must not be empty"
        )))
    } else {
        Ok(())
    }
}

fn is_interchange_number(value: &str) -> bool {
    let value = value.strip_prefix('-').unwrap_or(value);
    let exponent = value.find(['e', 'E']);
    let (mantissa, exponent) = exponent.map_or((value, None), |index| {
        (&value[..index], Some(&value[index + 1..]))
    });
    let (integer, fraction) = mantissa
        .split_once('.')
        .map_or((mantissa, None), |(integer, fraction)| {
            (integer, Some(fraction))
        });
    let valid_integer = integer == "0"
        || integer
            .strip_prefix(|character: char| ('1'..='9').contains(&character))
            .is_some_and(|rest| rest.chars().all(|character| character.is_ascii_digit()));
    let valid_fraction = fraction.is_none_or(|fraction| {
        !fraction.is_empty() && fraction.chars().all(|character| character.is_ascii_digit())
    });
    let valid_exponent = exponent.is_none_or(|exponent| {
        let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit())
    });
    valid_integer && valid_fraction && valid_exponent
}

impl BindingTime {
    const fn is_current(&self) -> bool {
        matches!(self, Self::Current)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::ast::{BindingKind, BindingTime, Name, SqlProjection, Value};

    use super::{CORE_SCHEMA_V1, canonical_json, decode_json};

    #[test]
    fn ast_interchange_uses_exact_tagged_numbers() {
        let value: Value = serde_json::from_value(json!({ "num": "9007199254740993" })).unwrap();
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            json!({ "num": "9007199254740993" })
        );
        assert!(serde_json::from_str::<Value>("9007199254740993").is_err());
    }

    #[test]
    fn ast_interchange_binding_paths_have_canonical_shapes() {
        let compact = Value::Binding {
            kind: BindingKind::Param,
            path: vec![Name::new("width").unwrap()],
            time: BindingTime::Start,
        };
        assert_eq!(
            serde_json::to_value(compact).unwrap(),
            json!({ "binding": { "kind": "param", "path": "width", "time": "start" } })
        );
        assert!(
            serde_json::from_str::<Value>(r#"{"binding":{"kind":"param","path":["width"]}}"#)
                .is_err()
        );
    }

    #[test]
    fn ast_interchange_projection_lists_round_trip_canonically() {
        let value = Value::Projection(Box::new(
            SqlProjection::parse("sum(\"amount\") as total, $width + 1 AS adjusted").unwrap(),
        ));
        let encoded = serde_json::to_value(&value).unwrap();
        assert_eq!(
            encoded,
            json!({
                "projection": "sum(\"amount\") AS total, $width + 1 AS adjusted"
            })
        );
        assert_eq!(serde_json::from_value::<Value>(encoded).unwrap(), value);
    }

    #[test]
    fn ast_interchange_relation_paths_round_trip_canonically() {
        let value = Value::Relation(vec![
            Name::new("samples").unwrap(),
            Name::new("movies").unwrap(),
        ]);
        let encoded = serde_json::to_value(&value).unwrap();
        assert_eq!(encoded, json!({ "relation": "samples.movies" }));
        assert_eq!(serde_json::from_value::<Value>(encoded).unwrap(), value);
    }

    #[test]
    fn ast_interchange_rejects_duplicate_properties_and_tags() {
        assert!(serde_json::from_str::<Value>(r#"{"num":"1","atom":"x"}"#).is_err());
        assert!(
            serde_json::from_str::<crate::ast::PropertyMap>(r#"{"x":true,"x":false}"#).is_err()
        );
    }

    #[test]
    fn ast_interchange_schema_is_frozen_valid_json() {
        let schema: serde_json::Value = serde_json::from_str(CORE_SCHEMA_V1).unwrap();
        assert_eq!(schema["$id"], "https://avenger.dev/schemas/ast-core-1.json");
        assert_eq!(
            schema["$defs"]["tagged"]["properties"]
                .as_object()
                .unwrap()
                .len(),
            17
        );
    }

    #[test]
    fn ast_interchange_canonical_json_sorts_every_object() {
        let file = decode_json(
            r#"{"items":[{"exported":false,"declaration":{"props":{"z":{"num":"2"},"a":{"num":"1"}},"decl":"chart","kind":"cartesian"}}],"version":1}"#,
        )
        .unwrap();
        assert_eq!(
            canonical_json(&file).unwrap(),
            r#"{"items":[{"declaration":{"decl":"chart","kind":"cartesian","props":{"a":{"num":"1"},"z":{"num":"2"}}},"exported":false}],"version":1}"#
        );
    }

    #[test]
    fn ast_interchange_rejects_invalid_top_level_and_imports() {
        assert!(decode_json(r#"{"version":2,"root":{"decl":"chart"}}"#).is_err());
        assert!(decode_json(r#"{"version":1,"name":"data","root":[{"decl":"table"}]}"#).is_err());
        assert!(
            decode_json(
                r#"{"version":1,"imports":[{"import":"","sha256":"ABC"}],"root":{"decl":"chart"}}"#
            )
            .is_err()
        );
    }
}
