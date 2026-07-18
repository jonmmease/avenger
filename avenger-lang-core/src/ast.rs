//! Stable, schema-free semantic tree for Avenger language version 1.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    hash::{Hash, Hasher},
};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sqlparser::ast::{Expr, ObjectName, Query, Visit, Visitor};

use crate::{
    SourceFile, SourceId, SourceOrigin, SourceSpan,
    sql::{
        BindingOccurrence, BindingVersion, ParsedSqlIsland, parse_sql_expression, parse_sql_query,
        tokenize,
    },
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(String);

impl Name {
    pub fn new(value: impl Into<String>) -> Result<Self, AstError> {
        let value = value.into();
        if is_name(&value) {
            Ok(Self(value))
        } else {
            Err(AstError::InvalidName(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for Name {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

pub fn is_name(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// Exact canonical spelling of one SQL numeric literal.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NumericLiteral(String);

impl NumericLiteral {
    pub fn new(source: &str) -> Result<Self, AstError> {
        canonicalize_number(source).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NumericLiteral {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct File {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<Name>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<Import>,
    pub root: Root,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Import {
    #[serde(rename = "import")]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<Name>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Root {
    Chart(Decl),
    Define(Decl),
    Data(Vec<Decl>),
}

impl Root {
    pub fn declarations(&self) -> &[Decl] {
        match self {
            Self::Chart(decl) | Self::Define(decl) => std::slice::from_ref(decl),
            Self::Data(declarations) => declarations,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decl {
    #[serde(rename = "decl")]
    pub keyword: Name,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Name>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<Name>,
    #[serde(default, skip_serializing_if = "Visibility::is_default")]
    pub visibility: Visibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    #[serde(default, skip_serializing_if = "PropertyMap::is_empty")]
    pub props: PropertyMap,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Decl>,
}

impl Decl {
    pub fn new(keyword: Name) -> Self {
        Self {
            keyword,
            kind: None,
            name: None,
            visibility: Visibility::Default,
            doc: None,
            props: PropertyMap::default(),
            children: Vec::new(),
        }
    }

    pub fn body(&self) -> Body {
        Body {
            props: self.props.clone(),
            children: self.children.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Default,
    Private,
    Public,
}

impl Visibility {
    pub const fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Body {
    #[serde(default, skip_serializing_if = "PropertyMap::is_empty")]
    pub props: PropertyMap,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Decl>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct PropertyMap(BTreeMap<Name, Value>);

impl PropertyMap {
    pub fn insert(&mut self, name: Name, value: Value) -> Result<(), AstError> {
        if self.0.contains_key(&name) {
            return Err(AstError::DuplicateProperty(name.to_string()));
        }
        self.0.insert(name, value);
        Ok(())
    }

    pub(crate) fn set(&mut self, name: Name, value: Value) {
        self.0.insert(name, value);
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.iter().find_map(|(key, value)| {
            if key.as_str() == name {
                Some(value)
            } else {
                None
            }
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Name, &Value)> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    Str(String),
    Num(NumericLiteral),
    Bool(bool),
    Null,
    Column(String),
    Atom(Name),
    Expr(Box<SqlExpression>),
    Query(Box<SqlQuery>),
    Binding {
        kind: BindingKind,
        path: Vec<Name>,
        time: BindingTime,
    },
    Ref {
        kind: RefKind,
        path: Vec<Name>,
    },
    Visual(Box<Value>),
    Dim(Vec<Name>),
    Pattern(Box<Value>),
    Env(String),
    None,
    Array(Vec<Value>),
    Block {
        head: Option<Box<Value>>,
        body: Body,
    },
    Call {
        function: Name,
        args: Vec<Value>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingKind {
    Param,
    Store,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingTime {
    #[default]
    Current,
    Start,
    Previous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    Mark,
    Group,
    Selection,
    Tool,
    Widget,
    Resource,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SqlBinding {
    pub synthetic_identifier: String,
    pub kind: BindingKind,
    pub path: Vec<Name>,
    pub time: BindingTime,
}

#[derive(Clone, Debug)]
pub struct SqlExpression {
    ast: Expr,
    bindings: Vec<SqlBinding>,
}

impl SqlExpression {
    pub fn parse(source: &str) -> Result<Self, AstError> {
        let source_file = SourceFile::new(
            SourceId::new(0),
            SourceOrigin::Memory("<sql-expression>".into()),
            source,
        );
        let stream = tokenize(&source_file).map_err(|error| AstError::Sql(error.to_string()))?;
        let parsed =
            parse_sql_expression(&stream, 0).map_err(|error| AstError::Sql(error.to_string()))?;
        ensure_sql_consumed(&stream, parsed.next_token)?;
        Self::from_parsed(parsed)
    }

    pub(crate) fn from_parsed(parsed: ParsedSqlIsland<Expr>) -> Result<Self, AstError> {
        let relation_names = relation_names(&parsed.ast);
        Ok(Self {
            ast: parsed.ast,
            bindings: parsed
                .bindings
                .iter()
                .map(|binding| {
                    let quoted = format!("\"{}\"", binding.synthetic_identifier);
                    let kind = if relation_names.contains(&quoted) {
                        BindingKind::Store
                    } else {
                        BindingKind::Param
                    };
                    sql_binding(binding, kind)
                })
                .collect::<Result<_, _>>()?,
        })
    }

    pub fn ast(&self) -> &Expr {
        &self.ast
    }

    pub fn bindings(&self) -> &[SqlBinding] {
        &self.bindings
    }

    pub fn canonical_sql(&self) -> String {
        restore_bindings(self.ast.to_string(), &self.bindings)
    }
}

impl PartialEq for SqlExpression {
    fn eq(&self, other: &Self) -> bool {
        self.canonical_sql() == other.canonical_sql()
    }
}

impl Eq for SqlExpression {}

impl Hash for SqlExpression {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.canonical_sql().hash(state);
    }
}

#[derive(Clone, Debug)]
pub struct SqlQuery {
    ast: Box<Query>,
    bindings: Vec<SqlBinding>,
}

impl SqlQuery {
    pub fn parse(source: &str) -> Result<Self, AstError> {
        let source_file = SourceFile::new(
            SourceId::new(0),
            SourceOrigin::Memory("<sql-query>".into()),
            source,
        );
        let stream = tokenize(&source_file).map_err(|error| AstError::Sql(error.to_string()))?;
        let parsed =
            parse_sql_query(&stream, 0).map_err(|error| AstError::Sql(error.to_string()))?;
        ensure_sql_consumed(&stream, parsed.next_token)?;
        Self::from_parsed(parsed)
    }

    pub(crate) fn from_parsed(parsed: ParsedSqlIsland<Box<Query>>) -> Result<Self, AstError> {
        let relation_names = relation_names(&parsed.ast);
        Ok(Self {
            ast: parsed.ast,
            bindings: parsed
                .bindings
                .iter()
                .map(|binding| {
                    let quoted = format!("\"{}\"", binding.synthetic_identifier);
                    let kind = if relation_names.contains(&quoted) {
                        BindingKind::Store
                    } else {
                        BindingKind::Param
                    };
                    sql_binding(binding, kind)
                })
                .collect::<Result<_, _>>()?,
        })
    }

    pub fn ast(&self) -> &Query {
        &self.ast
    }

    pub fn bindings(&self) -> &[SqlBinding] {
        &self.bindings
    }

    pub fn canonical_sql(&self) -> String {
        restore_bindings(self.ast.to_string(), &self.bindings)
    }
}

impl PartialEq for SqlQuery {
    fn eq(&self, other: &Self) -> bool {
        self.canonical_sql() == other.canonical_sql()
    }
}

impl Eq for SqlQuery {}

impl Hash for SqlQuery {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.canonical_sql().hash(state);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AstNodeId(u32);

impl AstNodeId {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Debug, Default)]
pub struct AstSourceMap {
    spans: BTreeMap<AstNodeId, SourceSpan>,
    roles: BTreeMap<AstNodeId, AstNodeRole>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AstNodeRole {
    Declaration(Name),
    PropertyValue(Name),
}

impl AstSourceMap {
    pub fn insert(&mut self, id: AstNodeId, span: SourceSpan) {
        self.spans.insert(id, span);
    }

    pub fn insert_with_role(&mut self, id: AstNodeId, span: SourceSpan, role: AstNodeRole) {
        self.spans.insert(id, span);
        self.roles.insert(id, role);
    }

    pub fn get(&self, id: AstNodeId) -> Option<SourceSpan> {
        self.spans.get(&id).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (AstNodeId, SourceSpan)> + '_ {
        self.spans.iter().map(|(id, span)| (*id, *span))
    }

    pub fn role(&self, id: AstNodeId) -> Option<&AstNodeRole> {
        self.roles.get(&id)
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AstError {
    #[error("invalid Avenger name `{0}`")]
    InvalidName(String),
    #[error("invalid exact numeric literal `{0}`")]
    InvalidNumber(String),
    #[error("duplicate property `{0}`")]
    DuplicateProperty(String),
    #[error("invalid SQL: {0}")]
    Sql(String),
    #[error("invalid AST interchange value: {0}")]
    InvalidInterchange(String),
    #[error("SQL parser left unconsumed tokens")]
    TrailingSql,
}

fn canonicalize_number(source: &str) -> Result<String, AstError> {
    let (negative, unsigned) = source
        .strip_prefix('-')
        .map_or((false, source), |value| (true, value));
    let (mantissa, exponent) = unsigned
        .split_once(['e', 'E'])
        .map_or((unsigned, None), |(left, right)| (left, Some(right)));
    let (integer, fraction) = mantissa
        .split_once('.')
        .map_or((mantissa, None), |(left, right)| (left, Some(right)));
    if integer.is_empty()
        || !integer.chars().all(|character| character.is_ascii_digit())
        || fraction.is_some_and(|value| {
            value.is_empty() || !value.chars().all(|character| character.is_ascii_digit())
        })
    {
        return Err(AstError::InvalidNumber(source.to_owned()));
    }
    let integer = integer.trim_start_matches('0');
    let integer = if integer.is_empty() { "0" } else { integer };
    let exponent = exponent
        .map(|value| {
            let (sign, digits) = if let Some(rest) = value.strip_prefix('-') {
                ("-", rest)
            } else {
                ("", value.strip_prefix('+').unwrap_or(value))
            };
            if digits.is_empty() || !digits.chars().all(|character| character.is_ascii_digit()) {
                return Err(AstError::InvalidNumber(source.to_owned()));
            }
            let digits = digits.trim_start_matches('0');
            Ok(format!(
                "{sign}{}",
                if digits.is_empty() { "0" } else { digits }
            ))
        })
        .transpose()?;
    let mut canonical = String::new();
    if negative {
        canonical.push('-');
    }
    canonical.push_str(integer);
    if let Some(fraction) = fraction {
        canonical.push('.');
        canonical.push_str(fraction);
    }
    if let Some(exponent) = exponent {
        canonical.push('e');
        canonical.push_str(&exponent);
    }
    Ok(canonical)
}

fn ensure_sql_consumed(stream: &crate::sql::TokenStream, mut index: usize) -> Result<(), AstError> {
    while matches!(
        stream.token(index).map(|token| token.token()),
        Some(sqlparser::tokenizer::Token::Whitespace(_))
    ) {
        index += 1;
    }
    if stream
        .token(index)
        .is_some_and(|token| matches!(token.token(), sqlparser::tokenizer::Token::EOF))
    {
        Ok(())
    } else {
        Err(AstError::TrailingSql)
    }
}

fn sql_binding(occurrence: &BindingOccurrence, kind: BindingKind) -> Result<SqlBinding, AstError> {
    Ok(SqlBinding {
        synthetic_identifier: occurrence.synthetic_identifier.clone(),
        kind,
        path: occurrence
            .path
            .iter()
            .map(|segment| Name::new(segment.clone()))
            .collect::<Result<_, _>>()?,
        time: match occurrence.version {
            BindingVersion::Current => BindingTime::Current,
            BindingVersion::Start => BindingTime::Start,
            BindingVersion::Previous => BindingTime::Previous,
        },
    })
}

fn binding_surface(binding: &SqlBinding) -> String {
    let mut output = String::from("$");
    for (index, segment) in binding.path.iter().enumerate() {
        if index > 0 {
            output.push('.');
        }
        output.push_str(segment.as_str());
    }
    match binding.time {
        BindingTime::Current => {}
        BindingTime::Start => output.push_str("@start"),
        BindingTime::Previous => output.push_str("@previous"),
    }
    output
}

fn restore_bindings(mut sql: String, bindings: &[SqlBinding]) -> String {
    for binding in bindings {
        sql = sql.replace(
            &format!("\"{}\"", binding.synthetic_identifier),
            &binding_surface(binding),
        );
    }
    sql
}

fn relation_names(node: &impl Visit) -> BTreeSet<String> {
    #[derive(Default)]
    struct Relations(BTreeSet<String>);

    impl Visitor for Relations {
        type Break = ();

        fn pre_visit_relation(&mut self, relation: &ObjectName) -> std::ops::ControlFlow<()> {
            self.0.insert(relation.to_string());
            std::ops::ControlFlow::Continue(())
        }
    }

    let mut relations = Relations::default();
    let _ = node.visit(&mut relations);
    relations.0
}

#[cfg(test)]
mod tests {
    use super::{BindingKind, NumericLiteral, SqlExpression, SqlQuery};

    #[test]
    fn ast_numeric_literals_are_exact_and_canonical() {
        assert_eq!(
            NumericLiteral::new("9007199254740993").unwrap().as_str(),
            "9007199254740993"
        );
        assert_eq!(NumericLiteral::new("-0").unwrap().as_str(), "-0");
        assert_eq!(
            NumericLiteral::new("001.20E+003").unwrap().as_str(),
            "1.20e3"
        );
    }

    #[test]
    fn ast_sql_wrappers_restore_binding_spelling_and_roles() {
        let expression = SqlExpression::parse("$width@start + 1").unwrap();
        assert_eq!(expression.canonical_sql(), "$width@start + 1");
        assert_eq!(expression.bindings()[0].kind, BindingKind::Param);

        let expression = SqlExpression::parse("(SELECT count(*) FROM $rows)").unwrap();
        assert_eq!(expression.bindings()[0].kind, BindingKind::Store);

        let query = SqlQuery::parse("FROM $rows SELECT * WHERE \"x\" > $minimum").unwrap();
        assert_eq!(query.bindings()[0].kind, BindingKind::Store);
        assert_eq!(query.bindings()[1].kind, BindingKind::Param);
        assert!(query.canonical_sql().contains("FROM $rows"));
    }
}
