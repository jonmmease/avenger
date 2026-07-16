//! Versioned JSON interchange encoding for the stable semantic AST.

use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor},
    ser::SerializeMap,
};

use crate::ast::{
    AstError, BindingKind, BindingTime, Body, Decl, Name, NumericLiteral, PropertyMap, RefKind,
    Root, SqlExpression, SqlQuery, Value,
};

impl Serialize for Root {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Chart(decl) | Self::Define(decl) => decl.serialize(serializer),
            Self::Data(declarations) => declarations.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Root {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RootVisitor;

        impl<'de> Visitor<'de> for RootVisitor {
            type Value = Root;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("one root declaration or a non-empty data declaration array")
            }

            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let decl = Decl::deserialize(de::value::MapAccessDeserializer::new(map))?;
                if decl.keyword.as_str() == "chart" {
                    Ok(Root::Chart(decl))
                } else {
                    Ok(Root::Define(decl))
                }
            }

            fn visit_seq<A>(self, sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let declarations =
                    Vec::<Decl>::deserialize(de::value::SeqAccessDeserializer::new(sequence))?;
                if declarations.is_empty() {
                    return Err(de::Error::invalid_length(0, &"at least one declaration"));
                }
                Ok(Root::Data(declarations))
            }
        }

        deserializer.deserialize_any(RootVisitor)
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
            Self::Query(value) => tagged(serializer, "query", &value.canonical_sql()),
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
            Self::Visual(value) => tagged(serializer, "value", value),
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
                    "num" => Value::Num(
                        NumericLiteral::new(&map.next_value::<String>()?)
                            .map_err(de::Error::custom)?,
                    ),
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
                    "query" => Value::Query(Box::new(
                        SqlQuery::parse(&map.next_value::<String>()?).map_err(de::Error::custom)?,
                    )),
                    "value" => Value::Visual(Box::new(map.next_value()?)),
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
    "num", "col", "atom", "binding", "expr", "query", "value", "dim", "ref", "pattern", "env",
    "none", "block", "call",
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
    let path = value
        .split('.')
        .map(Name::new)
        .collect::<Result<Vec<_>, _>>()?;
    if path.len() != 2 {
        return Err(AstError::InvalidInterchange(
            "dimension path must contain exactly two names".into(),
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

impl BindingTime {
    const fn is_current(&self) -> bool {
        matches!(self, Self::Current)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::ast::{BindingKind, BindingTime, Name, Value};

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
    fn ast_interchange_rejects_duplicate_properties_and_tags() {
        assert!(serde_json::from_str::<Value>(r#"{"num":"1","atom":"x"}"#).is_err());
        assert!(
            serde_json::from_str::<crate::ast::PropertyMap>(r#"{"x":true,"x":false}"#).is_err()
        );
    }
}
