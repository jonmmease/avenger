//! Stable, schema-free semantic tree for Avenger language version 1.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    hash::{Hash, Hasher},
    ops::ControlFlow,
};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sqlparser::ast::{
    AccessExpr, ExcludeSelectItem, Expr, Ident, ObjectName, ObjectNamePart, Query,
    RenameSelectItem, Select, SelectItem, Visit, VisitMut, Visitor, VisitorMut,
    WildcardAdditionalOptions,
};

use crate::{
    SourceFile, SourceId, SourceOrigin, SourceSpan,
    sql::{
        BindingOccurrence, BindingVersion, ParsedSqlIsland, is_unquoted_identifier,
        parse_sql_expression, parse_sql_projection, parse_sql_query, tokenize,
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
    is_unquoted_identifier(value)
}

/// Closed authored verb set for ordered state actions.
///
/// These words are contextual at an action-statement head. Target resolution
/// determines whether a particular verb is valid for a scalar param, store,
/// selection, or the reserved cursor effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StateActionVerb {
    Set,
    Clear,
    Insert,
    Replace,
    Upsert,
    Patch,
    Delete,
    Toggle,
}

impl StateActionVerb {
    pub const ALL: [Self; 8] = [
        Self::Set,
        Self::Clear,
        Self::Insert,
        Self::Replace,
        Self::Upsert,
        Self::Patch,
        Self::Delete,
        Self::Toggle,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Set => "set",
            Self::Clear => "clear",
            Self::Insert => "insert",
            Self::Replace => "replace",
            Self::Upsert => "upsert",
            Self::Patch => "patch",
            Self::Delete => "delete",
            Self::Toggle => "toggle",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|verb| verb.as_str() == value)
    }
}

pub fn is_state_action_keyword(value: &str) -> bool {
    StateActionVerb::parse(value).is_some()
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<Import>,
    pub items: Vec<ModuleItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleItem {
    pub exported: bool,
    pub declaration: Decl,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Import {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub clause: ImportClause,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportClause {
    Named(Vec<ImportSpecifier>),
    Namespace(Name),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportSpecifier {
    pub imported: Name,
    pub local: Name,
}

/// A non-empty dotted path used only in declaration-kind positions.
///
/// The canonical spelling is retained alongside validated segments so
/// resolvers can distinguish unqualified and namespaced kinds without
/// reparsing or allocating a joined string.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QualifiedName {
    segments: Vec<Name>,
    canonical: String,
}

impl QualifiedName {
    pub fn new(segments: Vec<Name>) -> Result<Self, AstError> {
        if segments.is_empty() {
            return Err(AstError::InvalidQualifiedName(
                "qualified name must contain at least one segment".to_string(),
            ));
        }
        let canonical = segments
            .iter()
            .map(Name::as_str)
            .collect::<Vec<_>>()
            .join(".");
        Ok(Self {
            segments,
            canonical,
        })
    }

    pub fn parse(value: &str) -> Result<Self, AstError> {
        if value.is_empty() {
            return Err(AstError::InvalidQualifiedName(value.to_string()));
        }
        value
            .split('.')
            .map(Name::new)
            .collect::<Result<Vec<_>, _>>()
            .and_then(Self::new)
    }

    pub fn segments(&self) -> &[Name] {
        &self.segments
    }

    pub fn as_str(&self) -> &str {
        &self.canonical
    }

    pub fn simple(&self) -> Option<&Name> {
        let [name] = self.segments.as_slice() else {
            return None;
        };
        Some(name)
    }
}

impl From<Name> for QualifiedName {
    fn from(name: Name) -> Self {
        let canonical = name.as_str().to_string();
        Self {
            segments: vec![name],
            canonical,
        }
    }
}

impl fmt::Display for QualifiedName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for QualifiedName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for QualifiedName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(&String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decl {
    #[serde(rename = "decl")]
    pub keyword: Name,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<QualifiedName>,
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
    Projection(Box<SqlProjection>),
    Query(Box<SqlQuery>),
    /// A statically resolved catalog relation path, authored without quotes.
    Relation(Vec<Name>),
    Binding {
        kind: BindingKind,
        path: Vec<Name>,
        time: BindingTime,
    },
    Ref {
        kind: RefKind,
        path: Vec<Name>,
    },
    Channel {
        mode: ChannelMode,
        expression: Box<Value>,
    },
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
pub enum ChannelMode {
    Encoded,
    Direct,
}

impl ChannelMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Encoded => "encoded",
            Self::Direct => "direct",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingKind {
    Param,
    Store,
    /// A resolved scalar binding whose value is the selection predicate for
    /// the current data row. The parser initially classifies `$name` as a
    /// parameter binding; resolution refines it to this kind when `name`
    /// denotes a selection.
    Selection,
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
        let mut ast = parsed.ast;
        normalize_contextual_namespaces(&mut ast);
        Ok(Self {
            ast,
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

    pub(crate) fn rewrite_identifiers(&mut self, replacement: impl FnMut(&str) -> Option<String>) {
        let _ = VisitMut::visit(&mut self.ast, &mut IdentifierRewriter { replacement });
    }

    /// Replace contextual `datum."field"` references with ordinary quoted
    /// identifiers. Compiler lowering uses this to let DataFusion plan the
    /// surrounding SQL before restoring the runtime event expression.
    pub fn rewrite_datum_fields(&mut self, replacement: impl FnMut(&str) -> Option<String>) {
        let _ = VisitMut::visit(&mut self.ast, &mut DatumFieldRewriter { replacement });
    }

    /// Replace complete SQL expression nodes selected by their canonical SQL
    /// spelling. Contextual language accesses use this to become synthetic
    /// columns before DataFusion planning without touching strings, comments,
    /// or similarly spelled identifier fragments.
    pub fn rewrite_expression_nodes(&mut self, replacement: impl FnMut(&Expr) -> Option<String>) {
        let _ = VisitMut::visit(&mut self.ast, &mut ExpressionNodeRewriter { replacement });
    }

    /// SQL column references and output/selector aliases, excluding relation
    /// names, string literals, and unrelated SQL binders.
    pub fn column_identifier_values(&self) -> BTreeSet<String> {
        column_identifier_values(&self.ast)
    }

    pub(crate) fn column_identifier_occurrences(&self, value: &str) -> usize {
        column_identifier_occurrences(&self.ast, value)
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

/// A SQL projection list, equivalent to the comma-separated expressions
/// between `SELECT` and `FROM` but without either clause.
#[derive(Clone, Debug)]
pub struct SqlProjection {
    items: Vec<SelectItem>,
    bindings: Vec<SqlBinding>,
}

impl SqlProjection {
    pub fn parse(source: &str) -> Result<Self, AstError> {
        let source_file = SourceFile::new(
            SourceId::new(0),
            SourceOrigin::Memory("<sql-projection>".into()),
            source,
        );
        let stream = tokenize(&source_file).map_err(|error| AstError::Sql(error.to_string()))?;
        let parsed =
            parse_sql_projection(&stream, 0).map_err(|error| AstError::Sql(error.to_string()))?;
        ensure_sql_consumed(&stream, parsed.next_token)?;
        Self::from_parsed(parsed)
    }

    pub(crate) fn from_parsed(parsed: ParsedSqlIsland<Vec<SelectItem>>) -> Result<Self, AstError> {
        let relation_names = relation_names(&parsed.ast);
        Ok(Self {
            items: parsed.ast,
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

    pub fn items(&self) -> &[SelectItem] {
        &self.items
    }

    pub fn bindings(&self) -> &[SqlBinding] {
        &self.bindings
    }

    pub fn canonical_sql(&self) -> String {
        self.canonical_items().join(", ")
    }

    pub fn canonical_items(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|item| restore_bindings(item.to_string(), &self.bindings))
            .collect()
    }

    pub(crate) fn rewrite_identifiers(&mut self, replacement: impl FnMut(&str) -> Option<String>) {
        let _ = VisitMut::visit(&mut self.items, &mut IdentifierRewriter { replacement });
    }

    pub fn column_identifier_values(&self) -> BTreeSet<String> {
        column_identifier_values(&self.items)
    }

    pub(crate) fn column_identifier_occurrences(&self, value: &str) -> usize {
        column_identifier_occurrences(&self.items, value)
    }
}

impl PartialEq for SqlProjection {
    fn eq(&self, other: &Self) -> bool {
        self.canonical_sql() == other.canonical_sql()
    }
}

impl Eq for SqlProjection {}

impl Hash for SqlProjection {
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

    pub(crate) fn rewrite_identifiers(&mut self, replacement: impl FnMut(&str) -> Option<String>) {
        let _ = VisitMut::visit(&mut self.ast, &mut IdentifierRewriter { replacement });
    }

    /// SQL column references and output/selector aliases, excluding relation
    /// names, string literals, and unrelated SQL binders.
    pub fn column_identifier_values(&self) -> BTreeSet<String> {
        column_identifier_values(self.ast.as_ref())
    }

    pub(crate) fn column_identifier_occurrences(&self, value: &str) -> usize {
        column_identifier_occurrences(self.ast.as_ref(), value)
    }
}

struct IdentifierRewriter<F> {
    replacement: F,
}

impl<F> IdentifierRewriter<F>
where
    F: FnMut(&str) -> Option<String>,
{
    fn rewrite(&mut self, identifier: &mut Ident) {
        if let Some(replacement) = (self.replacement)(&identifier.value) {
            identifier.value = replacement;
        }
    }

    fn rewrite_object_name(&mut self, name: &mut ObjectName) {
        for part in &mut name.0 {
            if let ObjectNamePart::Identifier(identifier) = part {
                self.rewrite(identifier);
            }
        }
    }

    fn rewrite_wildcard(&mut self, options: &mut WildcardAdditionalOptions) {
        match &mut options.opt_exclude {
            Some(ExcludeSelectItem::Single(name)) => self.rewrite_object_name(name),
            Some(ExcludeSelectItem::Multiple(names)) => {
                for name in names {
                    self.rewrite_object_name(name);
                }
            }
            None => {}
        }
        if let Some(except) = &mut options.opt_except {
            self.rewrite(&mut except.first_element);
            for identifier in &mut except.additional_elements {
                self.rewrite(identifier);
            }
        }
        if let Some(replace) = &mut options.opt_replace {
            for item in &mut replace.items {
                self.rewrite(&mut item.column_name);
            }
        }
        match &mut options.opt_rename {
            Some(RenameSelectItem::Single(item)) => {
                self.rewrite(&mut item.ident);
                self.rewrite(&mut item.alias);
            }
            Some(RenameSelectItem::Multiple(items)) => {
                for item in items {
                    self.rewrite(&mut item.ident);
                    self.rewrite(&mut item.alias);
                }
            }
            None => {}
        }
        if let Some(alias) = &mut options.opt_alias {
            self.rewrite(alias);
        }
    }
}

impl<F> VisitorMut for IdentifierRewriter<F>
where
    F: FnMut(&str) -> Option<String>,
{
    type Break = ();

    fn post_visit_expr(&mut self, expression: &mut Expr) -> ControlFlow<Self::Break> {
        match expression {
            Expr::Identifier(identifier) => self.rewrite(identifier),
            Expr::CompoundIdentifier(identifiers) => {
                for identifier in identifiers {
                    self.rewrite(identifier);
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }

    fn post_visit_select(&mut self, select: &mut Select) -> ControlFlow<Self::Break> {
        for item in &mut select.projection {
            match item {
                SelectItem::ExprWithAlias { alias, .. } => self.rewrite(alias),
                SelectItem::ExprWithAliases { aliases, .. } => {
                    for alias in aliases {
                        self.rewrite(alias);
                    }
                }
                SelectItem::QualifiedWildcard(_, options) | SelectItem::Wildcard(options) => {
                    self.rewrite_wildcard(options);
                }
                SelectItem::UnnamedExpr(_) => {}
            }
        }
        ControlFlow::Continue(())
    }
}

fn normalize_contextual_namespaces(expression: &mut Expr) {
    #[derive(Default)]
    struct Normalizer;

    impl VisitorMut for Normalizer {
        type Break = ();

        fn post_visit_expr(&mut self, expression: &mut Expr) -> ControlFlow<Self::Break> {
            if let Expr::CompoundIdentifier(identifiers) = expression {
                normalize_contextual_identifier_path(identifiers);
            } else if let Expr::CompoundFieldAccess { root, access_chain } = expression {
                if let Expr::CompoundIdentifier(identifiers) = root.as_mut() {
                    normalize_contextual_identifier_path(identifiers);
                } else if let Expr::Identifier(identifier) = root.as_mut()
                    && identifier.quote_style.is_none()
                    && identifier.value.eq_ignore_ascii_case("event")
                    && let Some(AccessExpr::Dot(Expr::Identifier(member))) =
                        access_chain.first_mut()
                    && member.quote_style.is_none()
                    && member.value.eq_ignore_ascii_case("facet")
                {
                    identifier.value = "event".to_owned();
                    member.value = "facet".to_owned();
                }
            }
            ControlFlow::Continue(())
        }
    }

    let _ = VisitMut::visit(expression, &mut Normalizer);
}

fn normalize_contextual_identifier_path(identifiers: &mut [Ident]) {
    let Some(root) = identifiers.first_mut() else {
        return;
    };
    if root.quote_style.is_some() {
        return;
    }
    let root_name = root.value.to_ascii_lowercase();
    match root_name.as_str() {
        "datum" | "channel" => {
            root.value = root_name;
            return;
        }
        "event" => {
            root.value = root_name;
            let channel_index = identifiers.get(1).and_then(|member| {
                if member.value.eq_ignore_ascii_case("coord")
                    || member.value.eq_ignore_ascii_case("domain")
                {
                    Some(2)
                } else if member.value.eq_ignore_ascii_case("start") {
                    Some(3)
                } else {
                    None
                }
            });
            for (index, identifier) in identifiers.iter_mut().enumerate().skip(1) {
                if identifier.quote_style.is_none() && channel_index != Some(index) {
                    identifier.value.make_ascii_lowercase();
                }
            }
            return;
        }
        "item" => {
            root.value = root_name;
            if let Some(namespace) = identifiers.get_mut(1)
                && namespace.quote_style.is_none()
            {
                namespace.value.make_ascii_lowercase();
            }
            if identifiers
                .get(1)
                .is_some_and(|namespace| namespace.value == "bbox")
                && let Some(edge) = identifiers.get_mut(2)
                && edge.quote_style.is_none()
            {
                edge.value.make_ascii_lowercase();
            }
            return;
        }
        _ => {}
    }

    // Inline view binders are user-defined, but their fixed member suffixes
    // are language-owned and canonicalized case-insensitively.
    let suffix = identifiers
        .iter()
        .skip(1)
        .filter(|identifier| identifier.quote_style.is_none())
        .map(|identifier| identifier.value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if matches!(
        suffix.as_slice(),
        [axis, field]
            if matches!(axis.as_str(), "x" | "y") && field == "pixels"
    ) || matches!(
        suffix.as_slice(),
        [axis, domain, boundary]
            if matches!(axis.as_str(), "x" | "y")
                && domain == "domain"
                && matches!(boundary.as_str(), "start" | "end")
    ) {
        for identifier in identifiers.iter_mut().skip(1) {
            identifier.value.make_ascii_lowercase();
        }
    }
}

struct DatumFieldRewriter<F> {
    replacement: F,
}

impl<F> VisitorMut for DatumFieldRewriter<F>
where
    F: FnMut(&str) -> Option<String>,
{
    type Break = ();

    fn post_visit_expr(&mut self, expression: &mut Expr) -> ControlFlow<Self::Break> {
        let Expr::CompoundIdentifier(identifiers) = expression else {
            return ControlFlow::Continue(());
        };
        let [namespace, field] = identifiers.as_slice() else {
            return ControlFlow::Continue(());
        };
        if namespace.quote_style.is_none()
            && namespace.value.eq_ignore_ascii_case("datum")
            && field.quote_style == Some('"')
            && let Some(replacement) = (self.replacement)(&field.value)
        {
            *expression = Expr::Identifier(Ident::with_quote('"', replacement));
        }
        ControlFlow::Continue(())
    }
}

struct ExpressionNodeRewriter<F> {
    replacement: F,
}

impl<F> VisitorMut for ExpressionNodeRewriter<F>
where
    F: FnMut(&Expr) -> Option<String>,
{
    type Break = ();

    fn post_visit_expr(&mut self, expression: &mut Expr) -> ControlFlow<Self::Break> {
        if let Some(replacement) = (self.replacement)(expression) {
            *expression = Expr::Identifier(Ident::with_quote('"', replacement));
        }
        ControlFlow::Continue(())
    }
}

fn column_identifier_values<T>(ast: &T) -> BTreeSet<String>
where
    T: Visit,
{
    struct IdentifierCollector {
        values: BTreeSet<String>,
    }

    impl Visitor for IdentifierCollector {
        type Break = ();

        fn pre_visit_expr(&mut self, expression: &Expr) -> ControlFlow<Self::Break> {
            match expression {
                Expr::Identifier(identifier) => {
                    self.values.insert(identifier.value.clone());
                }
                Expr::CompoundIdentifier(identifiers) => {
                    self.values.extend(
                        identifiers
                            .iter()
                            .map(|identifier| identifier.value.clone()),
                    );
                }
                _ => {}
            }
            ControlFlow::Continue(())
        }

        fn pre_visit_select(&mut self, select: &Select) -> ControlFlow<Self::Break> {
            for item in &select.projection {
                match item {
                    SelectItem::ExprWithAlias { alias, .. } => {
                        self.values.insert(alias.value.clone());
                    }
                    SelectItem::ExprWithAliases { aliases, .. } => {
                        self.values
                            .extend(aliases.iter().map(|alias| alias.value.clone()));
                    }
                    SelectItem::QualifiedWildcard(_, options) | SelectItem::Wildcard(options) => {
                        collect_wildcard_identifier_values(options, &mut self.values);
                    }
                    SelectItem::UnnamedExpr(_) => {}
                }
            }
            ControlFlow::Continue(())
        }
    }

    let mut collector = IdentifierCollector {
        values: BTreeSet::new(),
    };
    let _ = ast.visit(&mut collector);
    collector.values
}

fn column_identifier_occurrences<T>(ast: &T, expected: &str) -> usize
where
    T: Visit,
{
    struct IdentifierCounter<'a> {
        expected: &'a str,
        count: usize,
    }

    impl Visitor for IdentifierCounter<'_> {
        type Break = ();

        fn pre_visit_expr(&mut self, expression: &Expr) -> ControlFlow<Self::Break> {
            match expression {
                Expr::Identifier(identifier) => {
                    self.count += usize::from(identifier.value == self.expected);
                }
                Expr::CompoundIdentifier(identifiers) => {
                    self.count += identifiers
                        .iter()
                        .filter(|identifier| identifier.value == self.expected)
                        .count();
                }
                _ => {}
            }
            ControlFlow::Continue(())
        }
    }

    let mut counter = IdentifierCounter { expected, count: 0 };
    let _ = Visit::visit(ast, &mut counter);
    counter.count
}

fn collect_wildcard_identifier_values(
    options: &WildcardAdditionalOptions,
    output: &mut BTreeSet<String>,
) {
    let mut collect_name = |name: &ObjectName| {
        output.extend(name.0.iter().filter_map(|part| {
            let ObjectNamePart::Identifier(identifier) = part else {
                return None;
            };
            Some(identifier.value.clone())
        }));
    };
    match &options.opt_exclude {
        Some(ExcludeSelectItem::Single(name)) => collect_name(name),
        Some(ExcludeSelectItem::Multiple(names)) => {
            for name in names {
                collect_name(name);
            }
        }
        None => {}
    }
    if let Some(except) = &options.opt_except {
        output.insert(except.first_element.value.clone());
        output.extend(
            except
                .additional_elements
                .iter()
                .map(|identifier| identifier.value.clone()),
        );
    }
    if let Some(replace) = &options.opt_replace {
        output.extend(
            replace
                .items
                .iter()
                .map(|item| item.column_name.value.clone()),
        );
    }
    match &options.opt_rename {
        Some(RenameSelectItem::Single(item)) => {
            output.insert(item.ident.value.clone());
            output.insert(item.alias.value.clone());
        }
        Some(RenameSelectItem::Multiple(items)) => {
            for item in items {
                output.insert(item.ident.value.clone());
                output.insert(item.alias.value.clone());
            }
        }
        None => {}
    }
    if let Some(alias) = &options.opt_alias {
        output.insert(alias.value.clone());
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
    DeclarationKindSegment { name: Name, index: usize },
    DeclarationBinder(Name),
    Import,
    ImportSource,
    ImportImportedName(Name),
    ImportLocalName(Name),
    ImportNamespaceAlias(Name),
    ModuleItem,
    ExportKeyword,
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
    #[error("invalid qualified Avenger name `{0}`")]
    InvalidQualifiedName(String),
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

pub(crate) fn restore_bindings(mut sql: String, bindings: &[SqlBinding]) -> String {
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
    use super::{BindingKind, NumericLiteral, SqlExpression, SqlProjection, SqlQuery};

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
    fn datum_field_references_are_canonical_and_ast_rewritable() {
        let mut expression = SqlExpression::parse(r#"DATUM."a""b" + datum."value""#).unwrap();
        assert_eq!(
            expression.canonical_sql(),
            r#"datum."a""b" + datum."value""#
        );
        expression.rewrite_datum_fields(|field| Some(format!("event_{field}")));
        assert_eq!(
            expression.canonical_sql(),
            r#""event_a""b" + "event_value""#
        );

        let query = SqlQuery::parse(r#"SELECT datum."id" FROM input AS datum"#).unwrap();
        assert_eq!(
            query.canonical_sql(),
            r#"SELECT datum."id" FROM input AS datum"#
        );
    }

    #[test]
    fn contextual_property_accesses_are_canonical_and_ast_rewritable() {
        let mut expression = SqlExpression::parse(
            "EVENT.START.COORD.Horizontal + event.domain.Horizontal.END + event.facet[1]",
        )
        .unwrap();
        assert_eq!(
            expression.canonical_sql(),
            "event.start.coord.Horizontal + event.domain.Horizontal.end + event.facet[1]"
        );
        expression.rewrite_expression_nodes(|candidate| {
            (candidate.to_string() == "event.facet[1]").then(|| "facet_one".to_owned())
        });
        assert_eq!(
            expression.canonical_sql(),
            r#"event.start.coord.Horizontal + event.domain.Horizontal.end + "facet_one""#
        );

        assert_eq!(
            SqlExpression::parse("Viewport.X.DOMAIN.START + Viewport.Y.PIXELS")
                .unwrap()
                .canonical_sql(),
            "Viewport.x.domain.start + Viewport.y.pixels"
        );
        assert_eq!(
            SqlExpression::parse(r#"ITEM.DATA."Display Name" || item.BBOX.TOP"#)
                .unwrap()
                .canonical_sql(),
            r#"item.data."Display Name" || item.bbox.top"#
        );
    }

    #[test]
    fn projection_lists_round_trip_canonically() {
        let projection =
            SqlProjection::parse("sum(value) as total, $width + 1 AS adjusted").unwrap();
        assert_eq!(
            projection.canonical_sql(),
            "sum(value) AS total, $width + 1 AS adjusted"
        );
        assert_eq!(projection.bindings().len(), 1);

        let nested = SqlProjection::parse(
            "coalesce(struct(1, 2), struct(3, 4)) AS nested, CAST(value AS DOUBLE) AS numeric,",
        )
        .unwrap();
        assert_eq!(
            nested.canonical_sql(),
            "coalesce(STRUCT(1, 2), STRUCT(3, 4)) AS nested, CAST(value AS DOUBLE) AS numeric"
        );
        assert_eq!(
            SqlProjection::parse(&nested.canonical_sql()).unwrap(),
            nested
        );

        let commented =
            SqlProjection::parse("sum(value) /* measure */ AS total, -- next\n value AS raw")
                .unwrap();
        assert_eq!(
            commented.canonical_sql(),
            "sum(value) AS total, value AS raw"
        );
        assert_eq!(
            SqlProjection::parse(&commented.canonical_sql()).unwrap(),
            commented
        );
    }

    #[test]
    fn projection_lists_reject_implicit_aliases_without_confusing_nested_as() {
        assert!(SqlProjection::parse("sum(value) total").is_err());
        let projection = SqlProjection::parse("CAST(value AS DOUBLE) AS converted").unwrap();
        assert_eq!(
            projection.canonical_sql(),
            "CAST(value AS DOUBLE) AS converted"
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

    #[test]
    fn ast_identifier_rewrite_changes_references_and_aliases_but_not_literals() {
        let mut query = SqlQuery::parse(
            "SELECT '__private_value' AS literal, \"__private_value\" AS \"__private_output\" FROM input",
        )
        .unwrap();
        query.rewrite_identifiers(|identifier| {
            identifier
                .strip_prefix("__private_")
                .map(|suffix| format!("__av_col_deadbeef0000_{suffix}"))
        });
        let sql = query.canonical_sql();
        assert!(sql.contains("'__private_value'"), "{sql}");
        assert!(sql.contains("\"__av_col_deadbeef0000_value\""), "{sql}");
        assert!(sql.contains("AS \"__av_col_deadbeef0000_output\""), "{sql}");
        assert!(
            query
                .column_identifier_values()
                .contains("__av_col_deadbeef0000_output")
        );

        let mut wildcard =
            SqlQuery::parse("SELECT * EXCLUDE (\"__private_value\") FROM input").unwrap();
        wildcard.rewrite_identifiers(|identifier| {
            identifier
                .strip_prefix("__private_")
                .map(|suffix| format!("__av_col_deadbeef0000_{suffix}"))
        });
        assert!(
            wildcard
                .canonical_sql()
                .contains("\"__av_col_deadbeef0000_value\"")
        );
    }
}
