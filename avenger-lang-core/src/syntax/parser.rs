use std::fmt;

use sqlparser::{
    ast::{
        Expr, FunctionArg, FunctionArgExpr, FunctionArguments, UnaryOperator, Value as SqlValue,
    },
    tokenizer::Token,
};

use crate::{
    Diagnostic, SourceFile, SourceLabel, SourceSpan,
    ast::{
        AstError, AstNodeId, AstNodeRole, AstSourceMap, BindingKind, BindingTime, Body, Decl, File,
        Import, ImportClause, ImportSpecifier, ModuleItem, Name, NumericLiteral, PropertyMap,
        QualifiedName, RefKind, SqlExpression, SqlProjection, SqlQuery, Value, Visibility,
    },
    physical_type::PhysicalType,
    sql::{
        ParsedSqlIsland, SqlFrontendError, SqlParseLimits, TokenClass, TokenStream,
        parse_sql_expression_with_limits, parse_sql_projection_with_limits,
        parse_sql_query_with_limits, tokenize,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyntaxLimits {
    pub max_tokens: usize,
    pub max_nesting_depth: usize,
    pub max_declarations: usize,
    pub sql: SqlParseLimits,
}

/// The six structural contexts that can own an embedded SQL island.
///
/// This is intentionally a closed compiler/editor contract. Adding a context
/// requires updating the checked Tree-sitter boundary manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SqlIslandContext {
    QueryProperty,
    ProjectionProperty,
    PropertyExpression,
    TerminatedExpression,
    ArrayExpression,
    AliasedExpression,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlIslandRoot {
    Query,
    Projection,
    Expression,
}

/// Every strict-parser call site that delegates one source range to the SQL
/// frontend. Keeping this inventory closed prevents a new island-bearing DSL
/// form from bypassing the editor-boundary contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SqlIslandSite {
    QueryProperty,
    ProjectionProperty,
    ChannelModePayload,
    PropertyValue,
    ArrayElement,
    ParamInitializer,
    OutputSource,
    CursorActionRhs,
    StateActionRhs,
}

impl SqlIslandContext {
    pub const ALL: [Self; 6] = [
        Self::QueryProperty,
        Self::ProjectionProperty,
        Self::PropertyExpression,
        Self::TerminatedExpression,
        Self::ArrayExpression,
        Self::AliasedExpression,
    ];

    pub const fn manifest_name(self) -> &'static str {
        match self {
            Self::QueryProperty => "query_property",
            Self::ProjectionProperty => "projection_property",
            Self::PropertyExpression => "property_expression",
            Self::TerminatedExpression => "terminated_expression",
            Self::ArrayExpression => "array_expression",
            Self::AliasedExpression => "aliased_expression",
        }
    }

    pub const fn root(self) -> SqlIslandRoot {
        match self {
            Self::QueryProperty => SqlIslandRoot::Query,
            Self::ProjectionProperty => SqlIslandRoot::Projection,
            Self::PropertyExpression
            | Self::TerminatedExpression
            | Self::ArrayExpression
            | Self::AliasedExpression => SqlIslandRoot::Expression,
        }
    }
}

impl SqlIslandSite {
    pub const ALL: [Self; 9] = [
        Self::QueryProperty,
        Self::ProjectionProperty,
        Self::ChannelModePayload,
        Self::PropertyValue,
        Self::ArrayElement,
        Self::ParamInitializer,
        Self::OutputSource,
        Self::CursorActionRhs,
        Self::StateActionRhs,
    ];

    pub const fn manifest_name(self) -> &'static str {
        match self {
            Self::QueryProperty => "query_property",
            Self::ProjectionProperty => "projection_property",
            Self::ChannelModePayload => "channel_mode_payload",
            Self::PropertyValue => "property_value",
            Self::ArrayElement => "array_element",
            Self::ParamInitializer => "param_initializer",
            Self::OutputSource => "output_source",
            Self::CursorActionRhs => "cursor_action_rhs",
            Self::StateActionRhs => "state_action_rhs",
        }
    }

    pub const fn context(self) -> SqlIslandContext {
        match self {
            Self::QueryProperty => SqlIslandContext::QueryProperty,
            Self::ProjectionProperty => SqlIslandContext::ProjectionProperty,
            Self::ChannelModePayload | Self::PropertyValue => SqlIslandContext::PropertyExpression,
            Self::ArrayElement => SqlIslandContext::ArrayExpression,
            Self::CursorActionRhs | Self::StateActionRhs => SqlIslandContext::TerminatedExpression,
            Self::ParamInitializer | Self::OutputSource => SqlIslandContext::AliasedExpression,
        }
    }

    /// DSL-owned tokens that may immediately follow this exact SQL island.
    ///
    /// Delimiter ownership is site-specific. In particular, param
    /// initializers end at `as`, while definition outputs may either bind a
    /// computed value with `as` or use the identity-output `;` form.
    pub const fn outer_delimiters(self) -> &'static [&'static str] {
        match self {
            Self::QueryProperty
            | Self::ProjectionProperty
            | Self::CursorActionRhs
            | Self::StateActionRhs => &[";"],
            Self::ChannelModePayload | Self::PropertyValue => &[";", "{"],
            Self::ArrayElement => &[",", "]"],
            Self::ParamInitializer => &["as"],
            Self::OutputSource => &["as", ";"],
        }
    }
}

impl Default for SyntaxLimits {
    fn default() -> Self {
        Self {
            max_tokens: 500_000,
            max_nesting_depth: 128,
            max_declarations: 100_000,
            sql: SqlParseLimits::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConcreteFile {
    source: SourceFile,
    tokens: TokenStream,
    nodes: Vec<ConcreteNode>,
}

impl ConcreteFile {
    pub fn source(&self) -> &SourceFile {
        &self.source
    }

    pub fn tokens(&self) -> &TokenStream {
        &self.tokens
    }

    pub fn nodes(&self) -> &[ConcreteNode] {
        &self.nodes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SyntaxNodeId(u32);

impl SyntaxNodeId {
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConcreteNodeKind {
    Semantic,
    Token,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConcreteNode {
    pub id: SyntaxNodeId,
    pub span: SourceSpan,
    pub kind: ConcreteNodeKind,
}

#[derive(Clone, Debug)]
pub struct ParsedFile {
    pub ast: File,
    pub source_map: AstSourceMap,
    pub module_syntax: ModuleSyntaxMap,
    pub concrete: ConcreteFile,
}

#[derive(Clone, Debug, Default)]
pub struct ModuleSyntaxMap {
    pub version: Option<SourceSpan>,
    pub imports: Vec<ImportSyntax>,
    pub items: Vec<ModuleItemSyntax>,
    pub qualified_kinds: Vec<QualifiedNameSyntax>,
}

#[derive(Clone, Debug)]
pub struct ImportSyntax {
    pub span: SourceSpan,
    pub source: SourceSpan,
    pub sha256: Option<SourceSpan>,
    pub clause: ImportClauseSyntax,
}

#[derive(Clone, Debug)]
pub enum ImportClauseSyntax {
    Named {
        span: SourceSpan,
        specifiers: Vec<ImportSpecifierSyntax>,
    },
    Namespace {
        span: SourceSpan,
        local: SourceSpan,
    },
}

#[derive(Clone, Debug)]
pub struct ImportSpecifierSyntax {
    pub span: SourceSpan,
    pub imported: SourceSpan,
    pub local: SourceSpan,
    pub alias_keyword: Option<SourceSpan>,
}

#[derive(Clone, Debug)]
pub struct ModuleItemSyntax {
    pub span: SourceSpan,
    pub export_keyword: Option<SourceSpan>,
    pub declaration: SourceSpan,
}

#[derive(Clone, Debug)]
pub struct QualifiedNameSyntax {
    pub span: SourceSpan,
    pub segments: Vec<SourceSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    diagnostic: Box<Diagnostic>,
}

impl ParseError {
    pub fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }

    pub fn into_diagnostic(self) -> Diagnostic {
        *self.diagnostic
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.diagnostic.message)
    }
}

impl std::error::Error for ParseError {}

impl From<SqlFrontendError> for ParseError {
    fn from(error: SqlFrontendError) -> Self {
        Self {
            diagnostic: Box::new(error.into_diagnostic()),
        }
    }
}

pub fn parse_file(source: &SourceFile) -> Result<ParsedFile, ParseError> {
    parse_file_with_limits(source, SyntaxLimits::default())
}

pub fn parse_file_with_limits(
    source: &SourceFile,
    limits: SyntaxLimits,
) -> Result<ParsedFile, ParseError> {
    let tokens = tokenize(source).map_err(|error| ParseError {
        diagnostic: Box::new(error.into_diagnostic()),
    })?;
    if tokens.tokens().len() > limits.max_tokens {
        return Err(ParseError {
            diagnostic: Box::new(Diagnostic::error(
                "AVENGER-PARSE-022",
                "source token limit exceeded",
                SourceLabel::new(
                    SourceSpan::empty(source.id, source.text().len()),
                    format!(
                        "source has {} tokens; limit is {}",
                        tokens.tokens().len(),
                        limits.max_tokens
                    ),
                ),
            )),
        });
    }
    let mut parser = Parser::new(tokens.clone(), limits);
    let ast = parser.file()?;
    let source_map = parser.source_map;
    let module_syntax = parser.module_syntax;
    let nodes = concrete_nodes(&tokens, &source_map);
    Ok(ParsedFile {
        ast,
        source_map,
        module_syntax,
        concrete: ConcreteFile {
            source: source.clone(),
            tokens,
            nodes,
        },
    })
}

struct Parser {
    stream: TokenStream,
    index: usize,
    next_node_id: u32,
    source_map: AstSourceMap,
    module_syntax: ModuleSyntaxMap,
    limits: SyntaxLimits,
    nesting_depth: usize,
    declaration_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BodyContext {
    Ordinary,
    Predicate,
}

impl Parser {
    fn new(stream: TokenStream, limits: SyntaxLimits) -> Self {
        Self {
            stream,
            index: 0,
            next_node_id: 0,
            source_map: AstSourceMap::default(),
            module_syntax: ModuleSyntaxMap::default(),
            limits,
            nesting_depth: 0,
            declaration_count: 0,
        }
    }

    fn file(&mut self) -> Result<File, ParseError> {
        let version_start = self.start();
        self.expect_word("avenger")?;
        let version = self.number()?.parse::<u32>().map_err(|_| {
            self.error(
                "AVENGER-PARSE-002",
                "language version must be an unsigned integer",
            )
        })?;
        if version != crate::LANGUAGE_MAJOR {
            return Err(self.error(
                "AVENGER-PARSE-003",
                format!("unsupported language major {version}"),
            ));
        }
        self.expect(Token::SemiColon, "`;` after the version")?;
        self.module_syntax.version = Some(self.span(version_start, self.end()));
        let mut imports = Vec::new();
        while self.word_is("import") {
            let (import, syntax) = self.import()?;
            imports.push(import);
            self.module_syntax.imports.push(syntax);
        }
        let mut items = Vec::new();
        loop {
            let doc = self.leading_doc();
            self.trivia();
            if self.eof() {
                if doc.is_some() {
                    return Err(self.error(
                        "AVENGER-PARSE-043",
                        "doc comment must precede a module item",
                    ));
                }
                break;
            }
            if self.word_is("import") {
                return Err(self.error(
                    "AVENGER-PARSE-040",
                    "imports must precede every module item",
                ));
            }
            items.push(self.module_item(doc)?);
        }
        if items.is_empty() {
            return Err(self.error(
                "AVENGER-PARSE-041",
                "module must contain at least one chart, definition, table, schema, or catalog",
            ));
        }
        Ok(File {
            version,
            imports,
            items,
        })
    }

    fn import(&mut self) -> Result<(Import, ImportSyntax), ParseError> {
        let start = self.start();
        self.expect_word("import")?;
        let clause_start = self.start();
        let (clause, clause_syntax) = if self.consume(Token::LBrace) {
            if self.consume(Token::RBrace) {
                return Err(self.error(
                    "AVENGER-PARSE-042",
                    "named import lists must contain at least one specifier",
                ));
            }
            let mut specifiers = Vec::new();
            let mut syntax = Vec::new();
            loop {
                let specifier_start = self.start();
                let (imported, imported_span) = self.name_with_span()?;
                self.record(
                    imported_span.range.start,
                    imported_span.range.end,
                    AstNodeRole::ImportImportedName(imported.clone()),
                );
                let (local, local_span, alias_keyword) = if self.word_is("as") {
                    let alias_start = self.start();
                    self.expect_word("as")?;
                    let alias_keyword = self.span(alias_start, self.end());
                    let (local, local_span) = self.name_with_span()?;
                    (local, local_span, Some(alias_keyword))
                } else {
                    (imported.clone(), imported_span, None)
                };
                self.record(
                    local_span.range.start,
                    local_span.range.end,
                    AstNodeRole::ImportLocalName(local.clone()),
                );
                specifiers.push(ImportSpecifier { imported, local });
                syntax.push(ImportSpecifierSyntax {
                    span: self.span(specifier_start, self.end()),
                    imported: imported_span,
                    local: local_span,
                    alias_keyword,
                });
                if self.consume(Token::Comma) {
                    if self.consume(Token::RBrace) {
                        break;
                    }
                } else {
                    self.expect(Token::RBrace, "`}` after named imports")?;
                    break;
                }
            }
            (
                ImportClause::Named(specifiers),
                ImportClauseSyntax::Named {
                    span: self.span(clause_start, self.end()),
                    specifiers: syntax,
                },
            )
        } else if self.consume(Token::Mul) {
            self.expect_word("as")?;
            let (local, local_span) = self.name_with_span()?;
            self.record(
                local_span.range.start,
                local_span.range.end,
                AstNodeRole::ImportNamespaceAlias(local.clone()),
            );
            (
                ImportClause::Namespace(local),
                ImportClauseSyntax::Namespace {
                    span: self.span(clause_start, self.end()),
                    local: local_span,
                },
            )
        } else {
            return Err(self.error(
                "AVENGER-PARSE-042",
                "expected a named import list or `* as <namespace>`",
            ));
        };
        self.expect_word("from")?;
        let (source, source_span) = self.string_with_span()?;
        self.record(
            source_span.range.start,
            source_span.range.end,
            AstNodeRole::ImportSource,
        );
        let (sha256, sha256_span) = if self.consume_word("sha256") {
            let (hash, span) = self.string_with_span()?;
            (Some(hash), Some(span))
        } else {
            (None, None)
        };
        self.expect(Token::SemiColon, "`;` after import")?;
        let span = self.span(start, self.end());
        self.record(start, self.end(), AstNodeRole::Import);
        Ok((
            Import {
                source,
                sha256,
                clause,
            },
            ImportSyntax {
                span,
                source: source_span,
                sha256: sha256_span,
                clause: clause_syntax,
            },
        ))
    }

    fn module_item(&mut self, doc: Option<String>) -> Result<ModuleItem, ParseError> {
        let start = self.start();
        if matches!(self.word(), Some("private" | "public")) {
            return Err(self.error(
                "AVENGER-PARSE-044",
                "top-level module items use `export`, not component visibility modifiers",
            ));
        }
        let export_keyword = if self.word_is("export") {
            let export_start = self.start();
            self.expect_word("export")?;
            let span = self.span(export_start, self.end());
            self.record(export_start, self.end(), AstNodeRole::ExportKeyword);
            Some(span)
        } else {
            None
        };
        let declaration_start = self.start();
        let mut declaration = match self.word() {
            Some("chart") => self.chart()?,
            Some("define") => self.definition()?,
            Some("catalog" | "schema" | "table") => self.declaration(None)?,
            _ => {
                return Err(self.error(
                    "AVENGER-PARSE-045",
                    "expected a chart, define mark/tool/transform, table, schema, or catalog module item",
                ));
            }
        };
        declaration.doc = doc;
        let declaration_span = self.span(declaration_start, self.end());
        let span = self.span(start, self.end());
        self.record(start, self.end(), AstNodeRole::ModuleItem);
        self.module_syntax.items.push(ModuleItemSyntax {
            span,
            export_keyword,
            declaration: declaration_span,
        });
        Ok(ModuleItem {
            exported: export_keyword.is_some(),
            declaration,
        })
    }

    fn chart(&mut self) -> Result<Decl, ParseError> {
        let start = self.start();
        self.expect_word("chart")?;
        let kind = self.qualified_kind()?;
        let binder = self.optional_binder()?;
        let body = self.body()?;
        self.finish(start, from_body(n("chart"), Some(kind), binder, body))
    }

    fn definition(&mut self) -> Result<Decl, ParseError> {
        let start = self.start();
        self.expect_word("define")?;
        let kind = self.name()?;
        if !matches!(kind.as_str(), "mark" | "tool" | "transform") {
            return Err(self.error(
                "AVENGER-PARSE-006",
                "`define` supports only mark, tool, and transform",
            ));
        }
        let binder = self.declaration_name()?;
        let body = self.body()?;
        let mut saw_body_item = false;
        for child in &body.children {
            if child.keyword.as_str() == "slot" {
                if saw_body_item {
                    return Err(self.error(
                        "AVENGER-PARSE-020",
                        "slot interfaces must precede definition body items",
                    ));
                }
            } else if !matches!(child.keyword.as_str(), "output" | "export") {
                saw_body_item = true;
            }
        }
        self.finish(
            start,
            from_body(n("define"), Some(kind.into()), Some(binder), body),
        )
    }

    fn body(&mut self) -> Result<Body, ParseError> {
        self.body_with_context(BodyContext::Ordinary)
    }

    fn body_with_context(&mut self, context: BodyContext) -> Result<Body, ParseError> {
        self.enter_nesting()?;
        let result = self.body_inner(context);
        self.nesting_depth -= 1;
        result
    }

    fn body_inner(&mut self, context: BodyContext) -> Result<Body, ParseError> {
        self.expect(Token::LBrace, "`{` to start a body")?;
        let mut body = Body::default();
        loop {
            let doc = self.leading_doc();
            if self.consume(Token::RBrace) {
                if doc.is_some() {
                    return Err(self.error(
                        "AVENGER-PARSE-007",
                        "doc comment must precede a declaration",
                    ));
                }
                return Ok(body);
            }
            if self.eof() {
                return Err(self.error("AVENGER-PARSE-008", "unclosed body"));
            }
            if self.nth_is(1, &Token::Colon) {
                if doc.is_some() {
                    return Err(self.error(
                        "AVENGER-PARSE-007",
                        "doc comment cannot attach to a property",
                    ));
                }
                let key = self.name()?;
                self.expect(Token::Colon, "`:` after property name")?;
                let start = self.start();
                let value = self.property_value(key.as_str())?;
                self.record(start, self.end(), AstNodeRole::PropertyValue(key.clone()));
                body.props
                    .insert(key, value)
                    .map_err(|error| self.ast_error(error))?;
            } else if context == BodyContext::Predicate
                && self.word().is_some()
                && self.nth_is(1, &Token::LBrace)
            {
                let start = self.start();
                let name = self.name()?;
                let mut child = from_body(n("dimension"), None, Some(name), self.body()?);
                child.doc = doc;
                body.children.push(self.finish(start, child)?);
            } else {
                body.children.push(self.declaration(doc)?);
            }
        }
    }

    fn property_value(&mut self, property: &str) -> Result<Value, ParseError> {
        if property == "table" {
            self.trivia();
            if matches!(
                self.stream.token(self.index).map(|token| token.token()),
                Some(Token::SingleQuotedString(_))
            ) {
                return Err(self.error(
                    "AVENGER-PARSE-047",
                    "table relations are paths, not strings; remove the quotes",
                ));
            }
            let path = self.qual()?;
            self.expect(Token::SemiColon, "`;` after table relation")?;
            return Ok(Value::Relation(path));
        }
        if property == "target" && self.word_is("marks") {
            self.expect_word("marks")?;
            let paths = self.qual_list()?;
            self.expect(Token::SemiColon, "`;` after event targets")?;
            return Ok(Value::Array(
                paths
                    .into_iter()
                    .map(|path| Value::Ref {
                        kind: RefKind::Mark,
                        path,
                    })
                    .collect(),
            ));
        }
        if property == "scope" {
            if self.word_is("subplots") {
                self.expect_word("subplots")?;
                let paths = self.qual_list()?;
                self.expect(Token::SemiColon, "`;` after event scopes")?;
                return Ok(Value::Array(
                    paths
                        .into_iter()
                        .map(|path| path_call("subplot", path))
                        .collect(),
                ));
            }
            if self.word_is("subplot") {
                self.expect_word("subplot")?;
                let value = path_call("subplot", self.qual()?);
                self.expect(Token::SemiColon, "`;` after event scope")?;
                return Ok(value);
            }
        }
        if property == "surface" && self.word_is("legend") {
            self.expect_word("legend")?;
            let value = Value::Call {
                function: n("legend"),
                args: vec![Value::Atom(self.name()?)],
            };
            self.expect(Token::SemiColon, "`;` after event surface")?;
            return Ok(value);
        }
        if self.is(&Token::LBrace) {
            return Ok(Value::Block {
                head: None,
                body: self.body()?,
            });
        }
        if self.is(&Token::LBracket) {
            let value = Value::Array(self.array()?);
            self.expect(Token::SemiColon, "`;` after array")?;
            return Ok(value);
        }
        if self.legacy_value_qualifier() {
            return Err(self.error(
                "AVENGER-PARSE-046",
                "the `value` channel qualifier was removed; use `direct`",
            ));
        }
        if self.word_is("encoded") || self.word_is("direct") {
            let mode = if self.word_is("encoded") {
                crate::ast::ChannelMode::Encoded
            } else {
                crate::ast::ChannelMode::Direct
            };
            self.bump();
            let expression =
                self.expression(BindingKind::Param, SqlIslandSite::ChannelModePayload)?;
            return self.terminated(Value::Channel {
                mode,
                expression: Box::new(expression),
            });
        }
        if self.word_is("dim") {
            self.expect_word("dim")?;
            let path = self.qual()?;
            if path.len() != 2 {
                return Err(self.error(
                    "AVENGER-PARSE-009",
                    "dimension path must contain exactly two names",
                ));
            }
            let value = Value::Dim(path);
            return if self.is(&Token::LBrace) {
                Ok(Value::Block {
                    head: Some(Box::new(value)),
                    body: self.body()?,
                })
            } else {
                self.expect(Token::SemiColon, "`;` after dimension handle")?;
                Ok(value)
            };
        }
        if self.word_is("pattern") {
            self.expect_word("pattern")?;
            return Ok(Value::Pattern(Box::new(Value::Block {
                head: None,
                body: self.body()?,
            })));
        }
        if self.word_is("env") {
            self.expect_word("env")?;
            let value = self.string()?;
            self.expect(Token::SemiColon, "`;` after env value")?;
            return Ok(Value::Env(value));
        }
        if self.word_is("none") {
            self.expect_word("none")?;
            self.expect(Token::SemiColon, "`;` after none")?;
            return Ok(Value::None);
        }
        if self.word_is("group") {
            let checkpoint = self.index;
            self.bump();
            if self.word().is_some() {
                let _ = self.qual()?;
                if self.is(&Token::SemiColon) {
                    return Err(self.error(
                        "AVENGER-PARSE-028",
                        "group references were removed; use `mark <path>`",
                    ));
                }
            }
            self.index = checkpoint;
        }
        if let Some(kind) = self.ref_kind() {
            let checkpoint = self.index;
            self.bump();
            if self.word().is_some() {
                let path = self.qual()?;
                if self.consume(Token::SemiColon) {
                    return Ok(Value::Ref { kind, path });
                }
            }
            self.index = checkpoint;
        }
        if matches!(property, "sql" | "query") {
            let parsed = self.query(SqlIslandSite::QueryProperty)?;
            self.index = parsed.next_token;
            self.expect(Token::SemiColon, "`;` after SQL query")?;
            return SqlQuery::from_parsed(parsed)
                .map(|query| Value::Query(Box::new(query)))
                .map_err(|error| self.ast_error(error));
        }
        if property == "expressions" || self.looks_like_projection() {
            let parsed = self.projection(SqlIslandSite::ProjectionProperty)?;
            self.index = parsed.next_token;
            self.expect(Token::SemiColon, "`;` after SQL projection list")?;
            return SqlProjection::from_parsed(parsed)
                .map(|projection| Value::Projection(Box::new(projection)))
                .map_err(|error| self.ast_error(error));
        }
        if self.word().is_some() && self.nth_is(1, &Token::LBrace) {
            let head = Value::Atom(self.name()?);
            return Ok(Value::Block {
                head: Some(Box::new(head)),
                body: self.body()?,
            });
        }
        if self.word().is_some()
            && !matches!(self.word(), Some("true" | "false" | "null"))
            && self.nth_is(1, &Token::SemiColon)
        {
            let value = Value::Atom(self.name()?);
            self.expect(Token::SemiColon, "`;` after atom")?;
            return Ok(value);
        }
        // Generic DSL calls overlap SQL function syntax, but some valid DSL
        // calls are SQL keywords (`interval(...)`, `struct(...)`) or have an
        // empty argument list that sqlparser assigns different semantics to or
        // rejects. Recognize the literal/atom/call subset first and fall back
        // to the SQL expression island for all richer expressions.
        let checkpoint = self.index;
        if let Some(value) = self.try_generic_call()? {
            if self.is(&Token::SemiColon) || self.is(&Token::LBrace) {
                return self.terminated(value);
            }
            self.index = checkpoint;
        }
        let kind = if property == "data" {
            BindingKind::Store
        } else {
            BindingKind::Param
        };
        let value = self.expression(kind, SqlIslandSite::PropertyValue)?;
        self.terminated(value)
    }

    fn terminated(&mut self, value: Value) -> Result<Value, ParseError> {
        if self.consume(Token::SemiColon) {
            Ok(value)
        } else if self.is(&Token::LBrace) {
            Ok(Value::Block {
                head: Some(Box::new(value)),
                body: self.body()?,
            })
        } else {
            Err(self.error("AVENGER-PARSE-010", "expected `;` or a configuration body"))
        }
    }

    fn array(&mut self) -> Result<Vec<Value>, ParseError> {
        self.enter_nesting()?;
        let result = self.array_inner();
        self.nesting_depth -= 1;
        result
    }

    fn array_inner(&mut self) -> Result<Vec<Value>, ParseError> {
        self.expect(Token::LBracket, "`[` to start array")?;
        let mut values = Vec::new();
        if self.consume(Token::RBracket) {
            return Ok(values);
        }
        loop {
            let value = if self.is(&Token::LBrace) {
                Value::Block {
                    head: None,
                    body: self.body()?,
                }
            } else if self.word_is("pattern") {
                self.expect_word("pattern")?;
                Value::Pattern(Box::new(Value::Block {
                    head: None,
                    body: self.body()?,
                }))
            } else if self.word_is("none") {
                self.expect_word("none")?;
                Value::None
            } else {
                self.expression(BindingKind::Param, SqlIslandSite::ArrayElement)?
            };
            values.push(value);
            if self.consume(Token::Comma) {
                if self.consume(Token::RBracket) {
                    return Ok(values);
                }
            } else {
                self.expect(Token::RBracket, "`]` after array")?;
                return Ok(values);
            }
        }
    }

    fn query(
        &mut self,
        site: SqlIslandSite,
    ) -> Result<ParsedSqlIsland<Box<sqlparser::ast::Query>>, ParseError> {
        debug_assert_eq!(site.context().root(), SqlIslandRoot::Query);
        let start = self.sig();
        parse_sql_query_with_limits(&self.stream, start, self.limits.sql).map_err(Into::into)
    }

    fn projection(
        &mut self,
        site: SqlIslandSite,
    ) -> Result<ParsedSqlIsland<Vec<sqlparser::ast::SelectItem>>, ParseError> {
        debug_assert_eq!(site.context().root(), SqlIslandRoot::Projection);
        let start = self.sig();
        parse_sql_projection_with_limits(&self.stream, start, self.limits.sql).map_err(Into::into)
    }

    fn expression(
        &mut self,
        binding_kind: BindingKind,
        site: SqlIslandSite,
    ) -> Result<Value, ParseError> {
        debug_assert_eq!(site.context().root(), SqlIslandRoot::Expression);
        let start = self.sig();
        let parsed = parse_sql_expression_with_limits(&self.stream, start, self.limits.sql)?;
        self.index = parsed.next_token;
        expression_value(parsed, binding_kind).map_err(|error| self.ast_error(error))
    }

    // Declaration shapes and cursor helpers continue below.

    fn looks_like_projection(&mut self) -> bool {
        let start = self.sig();
        let mut index = start;
        let mut depth = 0usize;
        while let Some(token) = self.stream.token(index) {
            match token.token() {
                Token::LBrace if depth == 0 && index != start => return false,
                Token::LParen | Token::LBracket | Token::LBrace => depth += 1,
                Token::RParen | Token::RBracket | Token::RBrace => {
                    depth = depth.saturating_sub(1);
                }
                Token::Comma if depth == 0 => return true,
                Token::Word(word) if depth == 0 && word.value.eq_ignore_ascii_case("as") => {
                    return true;
                }
                Token::SemiColon if depth == 0 => return false,
                Token::EOF => return false,
                _ => {}
            }
            index += 1;
        }
        false
    }
}

impl Parser {
    fn declaration(&mut self, doc: Option<String>) -> Result<Decl, ParseError> {
        let visibility = if self.consume_word("private") {
            Visibility::Private
        } else if self.consume_word("public") {
            Visibility::Public
        } else {
            Visibility::Default
        };
        let start = self.start();
        let Some(keyword) = self.word().map(str::to_owned) else {
            return Err(self.error("AVENGER-PARSE-011", "expected a declaration"));
        };
        let mut decl = match keyword.as_str() {
            "catalog" | "schema" | "table" | "mark" | "transform" | "view" | "widget"
            | "resource" | "derive" => self.kind_bind_body()?,
            "variable" => self.variable()?,
            "tool" => self.tool()?,
            "param" => self.param()?,
            "container" => {
                return Err(self.error(
                    "AVENGER-PARSE-028",
                    "`container` was removed; use `mark group` or a legend `overlay:` property",
                ));
            }
            "store" => {
                return Err(self.error(
                    "AVENGER-PARSE-026",
                    "store declarations use `param store as <name>`",
                ));
            }
            "selection" => {
                return Err(self.error(
                    "AVENGER-PARSE-027",
                    "selection declarations use `param selection as <name>`",
                ));
            }
            "group" | "overlay" => {
                return Err(self.error(
                    "AVENGER-PARSE-028",
                    "standalone `group` and `overlay` declarations were removed; use `mark group` or a legend `overlay:` property",
                ));
            }
            "dimension" => {
                return Err(self.error(
                    "AVENGER-PARSE-029",
                    "predicate dimensions are keyed members and parallel frame dimensions use the `dimensions:` map",
                ));
            }
            "on" => self.event()?,
            "cell" => self.cell()?,
            "plot" => self.kind_body()?,
            "part" | "layer" => self.named_body()?,
            "level" => self.level()?,
            "adjust" => self.adjust()?,
            "row" | "when" | "key" | "fields" | "scale_edit" | "scale_hint" | "clause"
            | "equality" | "interval" => self.plain_body()?,
            "id" if self.nth_is(1, &Token::LBrace) => {
                return Err(
                    self.error("AVENGER-PARSE-025", "`id` is a property, not a declaration")
                );
            }
            "field" => self.field()?,
            "slot" => self.slot()?,
            "channel" => {
                return Err(self.error(
                    "AVENGER-PARSE-030",
                    "definition channels use `slot channel <name>`",
                ));
            }
            "output" => self.output()?,
            "export" => self.export()?,
            "match" => self.match_block()?,
            "set" => self.action()?,
            "theme" => self.theme()?,
            _ => self.splice()?,
        };
        decl.visibility = visibility;
        decl.doc = doc;
        self.finish(start, decl)
    }

    fn kind_bind_body(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let kind = self.qualified_kind()?;
        let binder = self.optional_binder()?;
        if matches!(
            keyword.as_str(),
            "catalog" | "schema" | "table" | "widget" | "resource"
        ) && binder.is_none()
        {
            return Err(self.error(
                "AVENGER-PARSE-012",
                format!("{} declarations require an `as` binder", keyword),
            ));
        }
        let body = self.body()?;
        Ok(from_body(keyword, Some(kind), binder, body))
    }

    fn tool(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let kind = self.qualified_kind()?;
        let binder = self.optional_binder()?;
        if self.consume(Token::SemiColon) {
            Ok(from_body(keyword, Some(kind), binder, Body::default()))
        } else {
            let body = self.body()?;
            Ok(from_body(keyword, Some(kind), binder, body))
        }
    }

    fn variable(&mut self) -> Result<Decl, ParseError> {
        self.expect_word("variable")?;
        let role = self.name()?;
        if !matches!(role.as_str(), "row" | "column" | "item") {
            return Err(self.error(
                "AVENGER-PARSE-035",
                "repeat variable role must be `row`, `column`, or `item`",
            ));
        }
        if self.consume_word("as") {
            return Err(self.error(
                "AVENGER-PARSE-035",
                "repeat variables use `variable <role> <id>` without `as`",
            ));
        }
        let name = self.name()?;
        let body = self.body()?;
        Ok(from_body(
            n("variable"),
            Some(role.into()),
            Some(name),
            body,
        ))
    }

    fn param(&mut self) -> Result<Decl, ParseError> {
        self.expect_word("param")?;
        if self.word_is("as") {
            return Err(self.error(
                "AVENGER-PARSE-031",
                "scalar param declarations require an initializer before `as`",
            ));
        }

        if self.word_is("store") && self.nth_word_is(1, "as") {
            self.expect_word("store")?;
            self.expect_word("as")?;
            let binder = self.name()?;
            let body = self.body()?;
            return Ok(from_body(n("store"), None, Some(binder), body));
        }
        if self.word_is("selection") && self.nth_word_is(1, "as") {
            self.expect_word("selection")?;
            self.expect_word("as")?;
            let binder = self.name()?;
            let body = self.body()?;
            return Ok(from_body(n("selection"), None, Some(binder), body));
        }

        let checkpoint = self.index;
        let legacy_type = self
            .physical_type_value()
            .ok()
            .filter(|value| PhysicalType::parse(value).is_ok())
            .is_some_and(|_| self.word_is("as"));
        self.index = checkpoint;
        if legacy_type {
            return Err(self.error(
                "AVENGER-PARSE-032",
                "scalar params infer their Arrow type; put a SQL initializer before `as` and use `CAST` when an exact type is required",
            ));
        }

        let initializer = self.expression(BindingKind::Param, SqlIslandSite::ParamInitializer)?;
        if !self.consume_word("as") {
            return Err(self.error(
                "AVENGER-PARSE-031",
                "scalar param declarations require `as <name>` after the initializer",
            ));
        }
        let binder = self.name()?;
        let mut body = if self.consume(Token::SemiColon) {
            Body::default()
        } else {
            self.body()?
        };
        if body.props.get("type").is_some()
            || body.props.get("default").is_some()
            || body.props.get("value").is_some()
            || body.props.get("kind").is_some()
        {
            return Err(self.error(
                "AVENGER-PARSE-033",
                "scalar params put the initializer before `as`; `type:`, `default:`, and `value:` are invalid body properties",
            ));
        }
        body.props
            .insert(n("value"), initializer)
            .expect("param body cannot author `value`");
        Ok(from_body(n("param"), None, Some(binder), body))
    }

    fn event(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let kind = self.name()?;
        let binder = self.optional_binder()?;
        let body = self.body()?;
        Ok(from_body(keyword, Some(kind.into()), binder, body))
    }

    fn cell(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let kind = self.qualified_kind()?;
        let binder = self.optional_binder()?;
        let at = if self.consume_word("at") {
            Some(self.body()?)
        } else {
            None
        };
        let mut body = self.body()?;
        if let Some(at) = at {
            body.props
                .insert(
                    n("at"),
                    Value::Block {
                        head: None,
                        body: at,
                    },
                )
                .map_err(|error| self.ast_error(error))?;
        }
        Ok(from_body(keyword, Some(kind), binder, body))
    }

    fn kind_body(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let kind = self.qualified_kind()?;
        let body = self.body()?;
        Ok(from_body(keyword, Some(kind), None, body))
    }

    fn named_body(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let binder = self.name()?;
        let body = self.body()?;
        Ok(from_body(keyword, None, Some(binder), body))
    }

    fn level(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let value = NumericLiteral::new(&self.number()?).map_err(|error| self.ast_error(error))?;
        let mut body = self.body()?;
        body.props
            .insert(n("index"), Value::Num(value))
            .expect("new property");
        Ok(from_body(keyword, None, None, body))
    }

    fn adjust(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        if self.is(&Token::LBrace) {
            return Err(self.error(
                "AVENGER-PARSE-036",
                "expression adjustments use `adjust expr { ... }`",
            ));
        }
        let authored_kind = self.name()?;
        let (kind, binder) = if authored_kind.as_str() == "expr" {
            if self.word_is("as") {
                return Err(self.error(
                    "AVENGER-PARSE-036",
                    "`adjust expr` cannot have an `as` binder",
                ));
            }
            (None, None)
        } else {
            (Some(authored_kind.into()), self.optional_binder()?)
        };
        let body = self.body()?;
        Ok(from_body(keyword, kind, binder, body))
    }

    fn plain_body(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let body = if matches!(keyword.as_str(), "equality" | "interval") {
            self.body_with_context(BodyContext::Predicate)?
        } else {
            self.body()?
        };
        Ok(from_body(keyword, None, None, body))
    }

    fn field(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        if self.nth_is(1, &Token::Colon) {
            return Err(self.error(
                "AVENGER-PARSE-034",
                "store fields use `field <type> <name> [nullable];`",
            ));
        }
        let value = self.physical_type_value()?;
        PhysicalType::parse(&value).map_err(|error| {
            self.error(
                "AVENGER-PARSE-034",
                format!("invalid field physical Arrow type: {error}"),
            )
        })?;
        let binder = self.name()?;
        let mut props = PropertyMap::default();
        props.insert(n("type"), value).expect("new property");
        if self.consume_word("nullable") {
            props
                .insert(n("nullable"), Value::Bool(true))
                .expect("new property");
        }
        self.expect(Token::SemiColon, "`;` after field")?;
        Ok(Decl {
            keyword,
            name: Some(binder),
            props,
            ..Decl::new(n("field"))
        })
    }

    fn slot(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let kind = self.name()?;
        if kind.as_str() == "function" {
            return Err(self.error(
                "AVENGER-PARSE-021",
                "function slots are not supported; pass a complete `expr` instead",
            ));
        }
        if !matches!(
            kind.as_str(),
            "expr"
                | "expr_list"
                | "literal"
                | "number"
                | "string"
                | "boolean"
                | "enum"
                | "ref"
                | "block"
                | "outputs"
                | "channel"
        ) {
            return Err(self.error("AVENGER-PARSE-021", "unknown definition slot shape"));
        }
        if self.consume_word("as") {
            return Err(self.error(
                "AVENGER-PARSE-037",
                "definition slots use `slot <shape> <name>` without `as`",
            ));
        }
        let binder = self.name()?;
        let body = if self.consume(Token::SemiColon) {
            Body::default()
        } else {
            self.body()?
        };
        Ok(from_body(keyword, Some(kind.into()), Some(binder), body))
    }

    fn output(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let value = self.expression(BindingKind::Param, SqlIslandSite::OutputSource)?;
        let mut props = PropertyMap::default();
        let binder = if self.consume_word("as") {
            props.insert(n("value"), value).expect("new property");
            self.name()?
        } else if self.is(&Token::Colon) {
            return Err(self.error(
                "AVENGER-PARSE-038",
                "transform outputs use `output <expression> as <public-name>`",
            ));
        } else if let Value::Atom(name) = value {
            name
        } else {
            return Err(self.error(
                "AVENGER-PARSE-038",
                "computed and qualified outputs require `as <public-name>`",
            ));
        };
        self.expect(Token::SemiColon, "`;` after output")?;
        Ok(Decl {
            keyword,
            name: Some(binder),
            props,
            ..Decl::new(n("output"))
        })
    }

    fn export(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let path = self.qual()?;
        let binder = self.consume_word("as").then(|| self.name()).transpose()?;
        self.expect(Token::SemiColon, "`;` after export")?;
        let mut props = PropertyMap::default();
        props
            .insert(n("source"), path_value(path))
            .expect("new property");
        Ok(Decl {
            keyword,
            name: binder,
            props,
            ..Decl::new(n("export"))
        })
    }

    fn match_block(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let binder = self.name()?;
        self.expect(Token::LBrace, "`{` after match target")?;
        let mut children = Vec::new();
        loop {
            let doc = self.leading_doc();
            if self.consume(Token::RBrace) {
                break;
            }
            let start = self.start();
            let arm_name = self.name()?;
            let body = self.body()?;
            let mut arm = from_body(n("arm"), None, Some(arm_name), body);
            arm.doc = doc;
            children.push(self.finish(start, arm)?);
        }
        Ok(Decl {
            keyword,
            name: Some(binder),
            children,
            ..Decl::new(n("match"))
        })
    }

    fn splice(&mut self) -> Result<Decl, ParseError> {
        let binder = self.name()?;
        self.expect(Token::SemiColon, "`;` after block-slot splice")?;
        Ok(Decl {
            keyword: n("splice"),
            name: Some(binder),
            ..Decl::new(n("splice"))
        })
    }

    fn action(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let target = self.qual()?;
        let is_cursor = target.len() == 1 && target[0].as_str() == "cursor";
        let mut props = PropertyMap::default();
        if is_cursor {
            self.expect(Token::Eq, "`=` in cursor action")?;
            let value = self.expression(BindingKind::Param, SqlIslandSite::CursorActionRhs)?;
            props.insert(n("value"), value).expect("new property");
            self.expect(Token::SemiColon, "`;` after cursor action")?;
        } else {
            if target.len() == 1
                && matches!(target[0].as_str(), "param" | "store" | "selection")
                && !self.is(&Token::Eq)
            {
                return Err(self.error(
                    "AVENGER-PARSE-013",
                    "set actions use `set <target> = ...` without a target-kind prefix",
                ));
            }
            props
                .insert(n("target"), path_value(target))
                .expect("new property");
            if self.consume_word("at") {
                let at = Value::Atom(self.name()?);
                props.insert(n("at"), at).expect("new property");
            }
            if self.consume_word("replacing") {
                self.expect_word("scopes")?;
                props
                    .insert(n("replacing_scopes"), Value::Bool(true))
                    .expect("new property");
            }
            self.expect(Token::Eq, "`=` in set action")?;
            let value = if self.word().is_some() && self.nth_is(1, &Token::LBrace) {
                let head = Value::Atom(self.name()?);
                Value::Block {
                    head: Some(Box::new(head)),
                    body: self.body()?,
                }
            } else {
                let value = self.expression(BindingKind::Param, SqlIslandSite::StateActionRhs)?;
                self.expect(Token::SemiColon, "`;` after set action")?;
                value
            };
            props.insert(n("value"), value).expect("new property");
        }
        Ok(Decl {
            keyword,
            kind: is_cursor.then(|| n("cursor").into()),
            props,
            ..Decl::new(n("set"))
        })
    }

    fn theme(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        self.expect_word("css")?;
        let mut props = PropertyMap::default();
        if self.consume_word("from") {
            props
                .insert(n("from"), Value::Str(self.string()?))
                .expect("new property");
            if self.consume_word("sha256") {
                props
                    .insert(n("sha256"), Value::Str(self.string()?))
                    .expect("new property");
            }
        } else {
            self.expect(Token::Colon, "`:` before inline CSS")?;
            props
                .insert(n("css"), Value::Str(self.string()?))
                .expect("new property");
        }
        self.expect(Token::SemiColon, "`;` after theme")?;
        Ok(Decl {
            keyword,
            kind: Some(n("css").into()),
            props,
            ..Decl::new(n("theme"))
        })
    }
}

impl Parser {
    fn optional_binder(&mut self) -> Result<Option<Name>, ParseError> {
        if self.consume_word("as") {
            self.declaration_name().map(Some)
        } else {
            Ok(None)
        }
    }

    fn declaration_name(&mut self) -> Result<Name, ParseError> {
        let (name, span) = self.name_with_span()?;
        self.record(
            span.range.start,
            span.range.end,
            AstNodeRole::DeclarationBinder(name.clone()),
        );
        Ok(name)
    }

    fn qualified_kind(&mut self) -> Result<QualifiedName, ParseError> {
        let start = self.start();
        let mut names = Vec::new();
        let mut spans = Vec::new();
        loop {
            let (name, span) = self.name_with_span()?;
            let index = names.len();
            self.record(
                span.range.start,
                span.range.end,
                AstNodeRole::DeclarationKindSegment {
                    name: name.clone(),
                    index,
                },
            );
            names.push(name);
            spans.push(span);
            if !self.consume(Token::Period) {
                break;
            }
        }
        let span = self.span(start, self.end());
        self.module_syntax
            .qualified_kinds
            .push(QualifiedNameSyntax {
                span,
                segments: spans,
            });
        QualifiedName::new(names).map_err(|error| self.ast_error(error))
    }

    fn qual(&mut self) -> Result<Vec<Name>, ParseError> {
        let mut path = vec![self.name()?];
        while self.consume(Token::Period) {
            path.push(self.name()?);
        }
        Ok(path)
    }

    fn qual_list(&mut self) -> Result<Vec<Vec<Name>>, ParseError> {
        self.expect(Token::LBracket, "`[` to start a qualified-name list")?;
        let mut paths = Vec::new();
        loop {
            paths.push(self.qual()?);
            if self.consume(Token::Comma) {
                if self.consume(Token::RBracket) {
                    return Ok(paths);
                }
            } else {
                self.expect(Token::RBracket, "`]` after qualified-name list")?;
                return Ok(paths);
            }
        }
    }

    fn finish(&mut self, start: usize, decl: Decl) -> Result<Decl, ParseError> {
        self.declaration_count = self.declaration_count.checked_add(1).ok_or_else(|| {
            self.error("AVENGER-PARSE-024", "source declaration count overflowed")
        })?;
        if self.declaration_count > self.limits.max_declarations {
            return Err(self.error(
                "AVENGER-PARSE-024",
                format!(
                    "source declaration count exceeds {}",
                    self.limits.max_declarations
                ),
            ));
        }
        self.record(
            start,
            self.end(),
            AstNodeRole::Declaration(decl.keyword.clone()),
        );
        Ok(decl)
    }

    fn record(&mut self, start: usize, end: usize, role: AstNodeRole) -> AstNodeId {
        let id = AstNodeId::new(self.next_node_id);
        self.next_node_id += 1;
        self.source_map.insert_with_role(
            id,
            SourceSpan::new(self.stream.source(), start, end)
                .expect("parser token spans are ordered"),
            role,
        );
        id
    }

    fn name(&mut self) -> Result<Name, ParseError> {
        self.trivia();
        let Some(Token::Word(word)) = self.stream.token(self.index).map(|token| token.token())
        else {
            return Err(self.error("AVENGER-PARSE-014", "expected an unquoted name"));
        };
        if word.quote_style.is_some() {
            return Err(self.error("AVENGER-PARSE-014", "DSL names cannot be quoted"));
        }
        let value = Name::new(word.value.clone()).map_err(|error| self.ast_error(error))?;
        self.index += 1;
        Ok(value)
    }

    fn name_with_span(&mut self) -> Result<(Name, SourceSpan), ParseError> {
        let start = self.start();
        let name = self.name()?;
        Ok((name, self.span(start, self.end())))
    }

    fn number(&mut self) -> Result<String, ParseError> {
        self.trivia();
        let Some(Token::Number(value, false)) = self.stream.token(self.index).map(|t| t.token())
        else {
            return Err(self.error("AVENGER-PARSE-015", "expected an unsigned number"));
        };
        let value = value.clone();
        self.index += 1;
        Ok(value)
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.trivia();
        let Some(Token::SingleQuotedString(value)) =
            self.stream.token(self.index).map(|token| token.token())
        else {
            return Err(self.error("AVENGER-PARSE-016", "expected a single-quoted string"));
        };
        let value = value.clone();
        self.index += 1;
        Ok(value)
    }

    fn string_with_span(&mut self) -> Result<(String, SourceSpan), ParseError> {
        let start = self.start();
        let value = self.string()?;
        Ok((value, self.span(start, self.end())))
    }

    /// Parse the unambiguous literal/atom subset of a generic DSL call.
    ///
    /// Failure is non-consuming so callers can delegate the same token range
    /// to the SQL expression parser.
    fn try_generic_call(&mut self) -> Result<Option<Value>, ParseError> {
        let checkpoint = self.index;
        let Some(function) = self.try_name() else {
            return Ok(None);
        };
        if !self.consume(Token::LParen) {
            self.index = checkpoint;
            return Ok(None);
        }

        self.enter_nesting()?;
        let result = self.finish_generic_call(checkpoint, function);
        self.nesting_depth -= 1;
        result
    }

    /// Parse one complete physical Arrow type without delegating to SQL.
    ///
    /// Atomic types end at the next identifier; constructor types are balanced
    /// by `try_generic_call`, so both forms leave a following declaration name
    /// untouched.
    fn physical_type_value(&mut self) -> Result<Value, ParseError> {
        if self.word().is_none() {
            return Err(self.error("AVENGER-PARSE-032", "expected a physical Arrow type"));
        }
        if self.nth_is(1, &Token::LParen) {
            return self.try_generic_call()?.ok_or_else(|| {
                self.error(
                    "AVENGER-PARSE-032",
                    "invalid or unbalanced physical Arrow type constructor",
                )
            });
        }
        self.name().map(Value::Atom)
    }

    fn finish_generic_call(
        &mut self,
        checkpoint: usize,
        function: Name,
    ) -> Result<Option<Value>, ParseError> {
        let mut args = Vec::new();
        if self.consume(Token::RParen) {
            return Ok(Some(Value::Call { function, args }));
        }
        loop {
            let Some(value) = self.try_generic_call_arg()? else {
                self.index = checkpoint;
                return Ok(None);
            };
            args.push(value);
            if self.consume(Token::Comma) {
                continue;
            }
            if self.consume(Token::RParen) {
                return Ok(Some(Value::Call { function, args }));
            }
            self.index = checkpoint;
            return Ok(None);
        }
    }

    fn enter_nesting(&mut self) -> Result<(), ParseError> {
        if self.nesting_depth >= self.limits.max_nesting_depth {
            return Err(self.error(
                "AVENGER-PARSE-023",
                format!(
                    "source nesting exceeds {} levels",
                    self.limits.max_nesting_depth
                ),
            ));
        }
        self.nesting_depth += 1;
        Ok(())
    }

    fn try_generic_call_arg(&mut self) -> Result<Option<Value>, ParseError> {
        self.trivia();
        match self.stream.token(self.index).map(|token| token.token()) {
            Some(Token::SingleQuotedString(value)) => {
                let value = value.clone();
                self.index += 1;
                Ok(Some(Value::Str(value)))
            }
            Some(Token::Number(value, false)) => {
                let value = NumericLiteral::new(value).map_err(|error| self.ast_error(error))?;
                self.index += 1;
                Ok(Some(Value::Num(value)))
            }
            Some(Token::Minus) => {
                let checkpoint = self.index;
                self.index += 1;
                let Some(Token::Number(value, false)) =
                    self.stream.token(self.index).map(|token| token.token())
                else {
                    self.index = checkpoint;
                    return Ok(None);
                };
                let value = NumericLiteral::new(&format!("-{value}"))
                    .map_err(|error| self.ast_error(error))?;
                self.index += 1;
                Ok(Some(Value::Num(value)))
            }
            Some(Token::Word(word)) if word.quote_style.is_none() => {
                let word = word.value.clone();
                if self.nth_is(1, &Token::LParen) {
                    return self.try_generic_call();
                }
                if self.nth_is(1, &Token::Period) {
                    let start = self.sig();
                    let parsed =
                        parse_sql_expression_with_limits(&self.stream, start, self.limits.sql)?;
                    self.index = parsed.next_token;
                    return expression_value(parsed, BindingKind::Param)
                        .map(Some)
                        .map_err(|error| self.ast_error(error));
                }
                self.index += 1;
                match word.as_str() {
                    "true" => Ok(Some(Value::Bool(true))),
                    "false" => Ok(Some(Value::Bool(false))),
                    "null" => Ok(Some(Value::Null)),
                    _ => Name::new(word)
                        .map(Value::Atom)
                        .map(Some)
                        .map_err(|error| self.ast_error(error)),
                }
            }
            _ => Ok(None),
        }
    }

    fn try_name(&mut self) -> Option<Name> {
        self.trivia();
        let Token::Word(word) = self.stream.token(self.index)?.token() else {
            return None;
        };
        if word.quote_style.is_some() {
            return None;
        }
        let value = Name::new(word.value.clone()).ok()?;
        self.index += 1;
        Some(value)
    }

    fn expect_word(&mut self, expected: &str) -> Result<(), ParseError> {
        if self.consume_word(expected) {
            Ok(())
        } else {
            Err(self.error("AVENGER-PARSE-017", format!("expected `{expected}`")))
        }
    }

    fn consume_word(&mut self, expected: &str) -> bool {
        self.trivia();
        let Some(Token::Word(word)) = self.stream.token(self.index).map(|token| token.token())
        else {
            return false;
        };
        if word.quote_style.is_none() && word.value == expected {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn word_is(&mut self, expected: &str) -> bool {
        self.word() == Some(expected)
    }

    fn legacy_value_qualifier(&mut self) -> bool {
        if !self.word_is("value") {
            return false;
        }
        self.stream.tokens()[self.index..]
            .iter()
            .filter(|token| !is_trivia(token.class()))
            .nth(1)
            .is_some_and(|token| {
                matches!(
                    token.token(),
                    Token::Word(_)
                        | Token::Number(_, _)
                        | Token::SingleQuotedString(_)
                        | Token::DoubleQuotedString(_)
                        | Token::TripleSingleQuotedString(_)
                        | Token::TripleDoubleQuotedString(_)
                        | Token::DollarQuotedString(_)
                        | Token::NationalStringLiteral(_)
                        | Token::EscapedStringLiteral(_)
                        | Token::UnicodeStringLiteral(_)
                        | Token::HexStringLiteral(_)
                        | Token::Placeholder(_)
                        | Token::LParen
                )
            })
    }

    fn word(&mut self) -> Option<&str> {
        self.trivia();
        match self.stream.token(self.index)?.token() {
            Token::Word(word) if word.quote_style.is_none() => Some(&word.value),
            _ => None,
        }
    }

    fn ref_kind(&mut self) -> Option<RefKind> {
        match self.word()? {
            "mark" => Some(RefKind::Mark),
            "selection" => Some(RefKind::Selection),
            "tool" => Some(RefKind::Tool),
            "widget" => Some(RefKind::Widget),
            "resource" => Some(RefKind::Resource),
            _ => None,
        }
    }

    fn expect(&mut self, expected: Token, label: &str) -> Result<(), ParseError> {
        if self.consume(expected) {
            Ok(())
        } else {
            Err(self.error("AVENGER-PARSE-018", format!("expected {label}")))
        }
    }

    fn consume(&mut self, expected: Token) -> bool {
        self.trivia();
        if self
            .stream
            .token(self.index)
            .is_some_and(|token| same_token(token.token(), &expected))
        {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn is(&mut self, expected: &Token) -> bool {
        self.trivia();
        self.stream
            .token(self.index)
            .is_some_and(|token| same_token(token.token(), expected))
    }

    fn nth_is(&self, offset: usize, expected: &Token) -> bool {
        self.stream.tokens()[self.index..]
            .iter()
            .filter(|token| !is_trivia(token.class()))
            .nth(offset)
            .is_some_and(|token| same_token(token.token(), expected))
    }

    fn nth_word_is(&self, offset: usize, expected: &str) -> bool {
        self.stream.tokens()[self.index..]
            .iter()
            .filter(|token| !is_trivia(token.class()))
            .nth(offset)
            .is_some_and(|token| {
                matches!(
                    token.token(),
                    Token::Word(word)
                        if word.quote_style.is_none() && word.value == expected
                )
            })
    }

    fn bump(&mut self) {
        self.trivia();
        self.index += 1;
    }

    fn sig(&mut self) -> usize {
        self.trivia();
        self.index
    }

    fn trivia(&mut self) {
        while self
            .stream
            .token(self.index)
            .is_some_and(|token| is_trivia(token.class()))
        {
            self.index += 1;
        }
    }

    fn leading_doc(&mut self) -> Option<String> {
        let mut lines = Vec::new();
        let mut last_doc_end = None;
        while let Some(token) = self.stream.token(self.index) {
            if !is_trivia(token.class()) {
                break;
            }
            if matches!(token.class(), TokenClass::Comment(_)) {
                let raw = self.stream.raw(token);
                if let Some(value) = raw.strip_prefix("-- |") {
                    lines.push(
                        value
                            .strip_prefix(' ')
                            .unwrap_or(value)
                            .trim_end()
                            .to_owned(),
                    );
                    last_doc_end = Some(token.span().range.end);
                } else if !raw.trim().is_empty() {
                    lines.clear();
                    last_doc_end = None;
                }
            }
            self.index += 1;
        }
        if last_doc_end.is_some_and(|end| {
            let next = self
                .stream
                .token(self.index)
                .map_or(self.stream.text().len(), |token| token.span().range.start);
            self.stream.text()[end..next].contains('\n')
        }) {
            lines.clear();
        }
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    fn eof(&self) -> bool {
        self.stream
            .token(self.index)
            .is_none_or(|token| token.class() == TokenClass::Eof)
    }

    fn start(&mut self) -> usize {
        self.trivia();
        self.stream
            .token(self.index)
            .map_or(self.stream.text().len(), |token| token.span().range.start)
    }

    fn end(&self) -> usize {
        self.index
            .checked_sub(1)
            .and_then(|index| self.stream.token(index))
            .map_or(0, |token| token.span().range.end)
    }

    fn span(&self, start: usize, end: usize) -> SourceSpan {
        SourceSpan::new(self.stream.source(), start, end).expect("parser token spans are ordered")
    }

    fn error(&self, code: &'static str, message: impl Into<String>) -> ParseError {
        let message = message.into();
        let span = self.stream.token(self.index).map_or(
            SourceSpan::empty(self.stream.source(), self.stream.text().len()),
            |token| token.span(),
        );
        ParseError {
            diagnostic: Box::new(Diagnostic::error(
                code,
                message.clone(),
                SourceLabel::new(span, message),
            )),
        }
    }

    fn ast_error(&self, error: AstError) -> ParseError {
        self.error("AVENGER-PARSE-019", error.to_string())
    }
}

fn expression_value(
    parsed: ParsedSqlIsland<Expr>,
    binding_kind: BindingKind,
) -> Result<Value, AstError> {
    let expression = SqlExpression::from_parsed(parsed)?;
    if let Some(binding) = expression.bindings().first()
        && expression.bindings().len() == 1
        && expression.canonical_sql() == binding_text(binding)
    {
        return Ok(Value::Binding {
            kind: binding_kind,
            path: binding.path.clone(),
            time: binding.time,
        });
    }
    if let Some(value) = literal_value(expression.ast(), expression.bindings())? {
        return Ok(value);
    }
    Ok(Value::Expr(Box::new(expression)))
}

fn literal_value(
    expression: &Expr,
    bindings: &[crate::ast::SqlBinding],
) -> Result<Option<Value>, AstError> {
    match expression {
        Expr::Value(value) => match &value.value {
            SqlValue::Number(value, false) => NumericLiteral::new(value).map(Value::Num).map(Some),
            SqlValue::SingleQuotedString(value) => Ok(Some(Value::Str(value.clone()))),
            SqlValue::Boolean(value) => Ok(Some(Value::Bool(*value))),
            SqlValue::Null => Ok(Some(Value::Null)),
            _ => Ok(None),
        },
        Expr::Identifier(identifier) if identifier.quote_style == Some('"') => bindings
            .iter()
            .find(|binding| binding.synthetic_identifier == identifier.value)
            .map_or_else(
                || Ok(Some(Value::Column(identifier.value.clone()))),
                |binding| {
                    Ok(Some(Value::Binding {
                        kind: binding.kind,
                        path: binding.path.clone(),
                        time: binding.time,
                    }))
                },
            ),
        Expr::Identifier(identifier) if identifier.quote_style.is_none() => {
            Name::new(identifier.value.clone())
                .map(Value::Atom)
                .map(Some)
        }
        Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr,
        } => {
            let Expr::Value(value) = expr.as_ref() else {
                return Ok(None);
            };
            let SqlValue::Number(value, false) = &value.value else {
                return Ok(None);
            };
            NumericLiteral::new(&format!("-{value}"))
                .map(Value::Num)
                .map(Some)
        }
        Expr::Function(function)
            if !function.uses_odbc_syntax
                && function.parameters == FunctionArguments::None
                && function.filter.is_none()
                && function.null_treatment.is_none()
                && function.over.is_none()
                && function.within_group.is_empty()
                && function.name.0.len() == 1 =>
        {
            let FunctionArguments::List(arguments) = &function.args else {
                return Ok(None);
            };
            if arguments.duplicate_treatment.is_some() || !arguments.clauses.is_empty() {
                return Ok(None);
            }
            let mut values = Vec::with_capacity(arguments.args.len());
            for argument in &arguments.args {
                let FunctionArg::Unnamed(FunctionArgExpr::Expr(expression)) = argument else {
                    return Ok(None);
                };
                let Some(value) = literal_value(expression, bindings)? else {
                    return Ok(None);
                };
                values.push(value);
            }
            let Some(identifier) = function.name.0[0].as_ident() else {
                return Ok(None);
            };
            Ok(Some(Value::Call {
                function: Name::new(identifier.value.clone())?,
                args: values,
            }))
        }
        // `INTERVAL(...)` is parsed by sqlparser as SQL interval syntax rather
        // than as a function call because `INTERVAL` is a keyword. With no SQL
        // interval qualifier, however, this is the DSL's ordinary generic-call
        // shape (notably `interval(month_day_nano)` in the Arrow type algebra).
        Expr::Interval(interval)
            if interval.leading_field.is_none()
                && interval.leading_precision.is_none()
                && interval.last_field.is_none()
                && interval.fractional_seconds_precision.is_none() =>
        {
            let expression = match interval.value.as_ref() {
                Expr::Nested(expression) => expression.as_ref(),
                expression => expression,
            };
            let Some(value) = literal_value(expression, bindings)? else {
                return Ok(None);
            };
            Ok(Some(Value::Call {
                function: n("interval"),
                args: vec![value],
            }))
        }
        Expr::Struct { values, fields } if fields.is_empty() => {
            let mut args = Vec::with_capacity(values.len());
            for expression in values {
                let Some(value) = literal_value(expression, bindings)? else {
                    return Ok(None);
                };
                args.push(value);
            }
            Ok(Some(Value::Call {
                function: n("struct"),
                args,
            }))
        }
        _ => Ok(None),
    }
}

fn binding_text(binding: &crate::ast::SqlBinding) -> String {
    let mut value = format!(
        "${}",
        binding
            .path
            .iter()
            .map(Name::as_str)
            .collect::<Vec<_>>()
            .join(".")
    );
    match binding.time {
        BindingTime::Current => {}
        BindingTime::Start => value.push_str("@start"),
        BindingTime::Previous => value.push_str("@previous"),
    }
    value
}

fn from_body(keyword: Name, kind: Option<QualifiedName>, name: Option<Name>, body: Body) -> Decl {
    Decl {
        keyword,
        kind,
        name,
        visibility: Visibility::Default,
        doc: None,
        props: body.props,
        children: body.children,
    }
}

fn path_value(path: Vec<Name>) -> Value {
    Value::Array(path.into_iter().map(Value::Atom).collect())
}

fn path_call(function: &str, path: Vec<Name>) -> Value {
    Value::Call {
        function: n(function),
        args: path.into_iter().map(Value::Atom).collect(),
    }
}

fn is_trivia(class: TokenClass) -> bool {
    matches!(class, TokenClass::Whitespace(_) | TokenClass::Comment(_))
}

fn same_token(left: &Token, right: &Token) -> bool {
    std::mem::discriminant(left) == std::mem::discriminant(right)
}

fn n(value: &str) -> Name {
    Name::new(value).expect("static parser names are valid")
}

fn concrete_nodes(stream: &TokenStream, source_map: &AstSourceMap) -> Vec<ConcreteNode> {
    let mut spans = source_map
        .iter()
        .map(|(_, span)| (span, ConcreteNodeKind::Semantic))
        .chain(
            stream
                .tokens()
                .iter()
                .map(|token| (token.span(), ConcreteNodeKind::Token)),
        )
        .collect::<Vec<_>>();
    spans.sort_by_key(|(span, kind)| {
        (
            span.range.start,
            std::cmp::Reverse(span.range.end),
            matches!(kind, ConcreteNodeKind::Token),
        )
    });
    spans
        .into_iter()
        .enumerate()
        .map(|(index, (span, kind))| ConcreteNode {
            id: SyntaxNodeId(u32::try_from(index).expect("syntax node count fits u32")),
            span,
            kind,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{
        SourceFile, SourceId, SourceOrigin,
        ast::{ImportClause, ModuleItem, Value},
    };

    use super::parse_file;

    fn parse(source: &str) -> super::ParsedFile {
        parse_file(&SourceFile::new(
            SourceId::new(1),
            SourceOrigin::Memory("test.avenger".into()),
            source,
        ))
        .unwrap()
    }

    fn only_item(parsed: &super::ParsedFile) -> &ModuleItem {
        let [item] = parsed.ast.items.as_slice() else {
            panic!("expected exactly one module item")
        };
        item
    }

    #[test]
    fn parse_chart_module_item_with_mixed_body_and_sql() {
        let parsed = parse(
            r#"
avenger 1;
import { rows as data } from './data.avenger';
chart cartesian as example {
  height: 400;
  data: $rows;
  -- | Visible points.
  mark symbol as points {
    x: "Horsepower";
    size: $radius@start * 2;
  }
  on pointermove as drag {
    set radius at start = $radius + 1;
  }
}
"#,
        );
        let chart = &only_item(&parsed).declaration;
        assert_eq!(chart.children.len(), 2);
        assert_eq!(chart.children[0].doc.as_deref(), Some("Visible points."));
        assert!(matches!(
            &parsed.ast.imports[0].clause,
            ImportClause::Named(specifiers)
                if specifiers[0].imported.as_str() == "rows"
                    && specifiers[0].local.as_str() == "data"
        ));
        assert!(!parsed.source_map.is_empty());
    }

    #[test]
    fn table_source_is_a_relation_path_and_rejects_strings() {
        let parsed = parse("avenger 1; chart cartesian { data: { table: samples.movies; } }");
        let Some(Value::Block { body, .. }) = only_item(&parsed).declaration.props.get("data")
        else {
            panic!("expected a data source block")
        };
        assert!(matches!(
            body.props.get("table"),
            Some(Value::Relation(path))
                if path.iter().map(|part| part.as_str()).collect::<Vec<_>>()
                    == ["samples", "movies"]
        ));

        let source = SourceFile::new(
            SourceId::new(2),
            SourceOrigin::Memory("quoted-table.avenger".into()),
            "avenger 1; chart cartesian { data: { table: 'samples.movies'; } }",
        );
        let error = parse_file(&source).unwrap_err();
        assert_eq!(error.diagnostic().code.as_str(), "AVENGER-PARSE-047");
        assert!(error.diagnostic().message.contains("remove the quotes"));
    }

    #[test]
    fn projection_lookahead_stops_at_configured_expression_blocks() {
        let parsed = parse(
            r#"
avenger 1;
chart cartesian as example {
  mark symbol {
    x: "x" { scale: linear { domain: [0.0, 3.0]; } }
    y: "y";
  }
  public mark text as label {}
}
"#,
        );
        assert_eq!(only_item(&parsed).declaration.children.len(), 2);
    }

    #[test]
    fn parse_mixed_exported_and_private_module_items() {
        let parsed = parse(
            "avenger 1;\
             export define mark badge { slot number radius; slot channel x; mark symbol {} }\
             table inline as movies { values: []; }\
             export chart cartesian as example {}",
        );
        assert_eq!(parsed.ast.items.len(), 3);
        assert!(parsed.ast.items[0].exported);
        assert!(!parsed.ast.items[1].exported);
        assert!(parsed.ast.items[2].exported);
        assert_eq!(parsed.ast.items[0].declaration.keyword.as_str(), "define");
        assert_eq!(parsed.ast.items[1].declaration.keyword.as_str(), "table");
        assert_eq!(parsed.ast.items[2].declaration.keyword.as_str(), "chart");
    }

    #[test]
    fn parse_named_and_namespace_imports_with_metadata() {
        let parsed = parse(
            "avenger 1;\
             import { chart, points as marks, } from './library.avenger' sha256 'abc123';\
             import * as native from 'native:acme';\
             chart native.cartesian as example {}",
        );
        assert_eq!(parsed.ast.imports.len(), 2);
        assert_eq!(
            parsed.ast.items[0]
                .declaration
                .kind
                .as_ref()
                .unwrap()
                .segments()
                .len(),
            2
        );
        assert_eq!(parsed.module_syntax.qualified_kinds[0].segments.len(), 2);
        assert_eq!(parsed.module_syntax.imports.len(), 2);
        assert!(parsed.module_syntax.imports[0].sha256.is_some());
        assert!(matches!(
            &parsed.ast.imports[1].clause,
            ImportClause::Namespace(name) if name.as_str() == "native"
        ));
    }

    #[test]
    fn parse_accepts_multiple_items_and_rejects_duplicate_properties() {
        let parsed = parse("avenger 1; chart cartesian as first {} chart polar as second {}");
        assert_eq!(parsed.ast.items.len(), 2);
        let source = SourceFile::new(
            SourceId::new(2),
            SourceOrigin::Memory("bad.avenger".into()),
            "avenger 1; chart cartesian { width: 1; width: 2; }",
        );
        assert!(parse_file(&source).is_err());
    }

    #[test]
    fn parse_rejects_nested_definitions() {
        let source = SourceFile::new(
            SourceId::new(1),
            SourceOrigin::Memory("nested.avenger".into()),
            "avenger 1; define mark outer { define mark inner { mark symbol {} } }",
        );
        assert!(parse_file(&source).is_err());
    }

    #[test]
    fn parse_doc_comments_require_adjacency() {
        let parsed = parse(
            "avenger 1; define mark docs { -- | attached\n slot number first; -- | detached\n\n slot number second; }",
        );
        let definition = &only_item(&parsed).declaration;
        assert_eq!(definition.children[0].doc.as_deref(), Some("attached"));
        assert_eq!(definition.children[1].doc, None);
    }

    #[test]
    fn parse_unified_declaration_headers_normalize_to_semantic_ast() {
        let parsed = parse(
            r#"avenger 1;
chart cartesian as chart {
  param CAST(NULL AS DOUBLE) as point;
  param store as rows { field float64 x nullable; }
  param selection as picked {}
  mark group as layer {}
  variable row mpg {}
  adjust expr { x: "x" + 1; }
  equality { id { field: "id"; value: datum."id"; } }
  on click { set point = NULL; set picked = clear; set cursor = 'crosshair'; }
}"#,
        );
        let chart = &only_item(&parsed).declaration;
        let keywords = chart
            .children
            .iter()
            .map(|child| child.keyword.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            keywords,
            [
                "param",
                "store",
                "selection",
                "mark",
                "variable",
                "adjust",
                "equality",
                "on"
            ]
        );
        assert_eq!(chart.children[6].kind, None);
        assert_eq!(chart.children[6].children[0].keyword.as_str(), "dimension");
        assert_eq!(chart.children[7].children[0].kind, None);
        assert_eq!(
            chart.children[7].children[2]
                .kind
                .as_ref()
                .unwrap()
                .as_str(),
            "cursor"
        );

        let definition = parse(
            "avenger 1; define transform sample { slot channel x; slot expr amount; output amount; output CAST(amount AS float64) + 1 as next; }",
        );
        let definition = &only_item(&definition).declaration;
        assert_eq!(
            definition.children[0].kind.as_ref().unwrap().as_str(),
            "channel"
        );
        assert_eq!(
            definition.children[2].name.as_ref().unwrap().as_str(),
            "amount"
        );
        assert_eq!(
            definition.children[3].name.as_ref().unwrap().as_str(),
            "next"
        );
    }

    #[test]
    fn parse_rejects_every_removed_declaration_form_with_a_focused_code() {
        let cases = [
            (
                "param as value { type: float64; default: 1; }",
                "AVENGER-PARSE-031",
            ),
            ("store as rows {}", "AVENGER-PARSE-026"),
            ("selection as picked {}", "AVENGER-PARSE-027"),
            ("group as layer {}", "AVENGER-PARSE-028"),
            ("overlay {}", "AVENGER-PARSE-028"),
            ("container group {}", "AVENGER-PARSE-028"),
            ("container overlay {}", "AVENGER-PARSE-028"),
            ("dimension as x {}", "AVENGER-PARSE-029"),
            ("channel x;", "AVENGER-PARSE-030"),
            ("adjust {}", "AVENGER-PARSE-036"),
            ("slot expr as value;", "AVENGER-PARSE-037"),
            ("variable row as mpg {}", "AVENGER-PARSE-035"),
            ("field x: float64;", "AVENGER-PARSE-034"),
            ("output total: values.total;", "AVENGER-PARSE-038"),
            ("set param value = 1;", "AVENGER-PARSE-013"),
        ];
        for (declaration, code) in cases {
            let source = SourceFile::new(
                SourceId::new(9),
                SourceOrigin::Memory("legacy.avenger".into()),
                format!("avenger 1; chart cartesian {{ {declaration} }}"),
            );
            let error = parse_file(&source).expect_err(declaration);
            assert_eq!(error.diagnostic().code.as_str(), code, "{declaration}");
        }

        let nested = SourceFile::new(
            SourceId::new(10),
            SourceOrigin::Memory("legacy-type.avenger".into()),
            "avenger 1; chart cartesian { param struct(field('x', float64)) as value { value: NULL; } }",
        );
        let error = parse_file(&nested).unwrap_err();
        assert_eq!(error.diagnostic().code.as_str(), "AVENGER-PARSE-033");
    }

    #[test]
    fn parse_rejects_removed_value_channel_qualifier() {
        for expression in ["42", "$width"] {
            let source = SourceFile::new(
                SourceId::new(1),
                SourceOrigin::Memory("legacy-channel-value.avenger".into()),
                format!(
                    "avenger 1; chart cartesian {{ mark symbol {{ x: value {expression}; }} }}"
                ),
            );
            let error = parse_file(&source).unwrap_err();
            assert_eq!(error.diagnostic().code.as_str(), "AVENGER-PARSE-046");
            assert!(error.diagnostic().message.contains("use `direct`"));
        }
    }

    #[test]
    fn scalar_params_reject_removed_typed_headers() {
        for body in ["{}", "{ type: float64; value: 1; }", "{ default: 1; }"] {
            let source = SourceFile::new(
                SourceId::new(11),
                SourceOrigin::Memory("param.avenger".into()),
                format!("avenger 1; chart cartesian {{ param float64 as value {body} }}"),
            );
            let error = parse_file(&source).unwrap_err();
            assert_eq!(error.diagnostic().code.as_str(), "AVENGER-PARSE-032");
        }
    }

    #[test]
    fn scalar_params_reject_removed_body_initializer_and_type_properties() {
        for property in [
            "value: 2;",
            "type: float64;",
            "default: 2;",
            "kind: scalar;",
        ] {
            let source = SourceFile::new(
                SourceId::new(11),
                SourceOrigin::Memory("param-body.avenger".into()),
                format!("avenger 1; chart cartesian {{ param 1 as value {{ {property} }} }}"),
            );
            let error = parse_file(&source).unwrap_err();
            assert_eq!(error.diagnostic().code.as_str(), "AVENGER-PARSE-033");
        }
    }

    #[test]
    fn parse_rejects_removed_module_forms_with_focused_codes() {
        let cases = [
            (
                "avenger 1; import './thing.avenger' as thing; chart cartesian {}",
                "AVENGER-PARSE-042",
            ),
            (
                "avenger 1; import {} from './thing.avenger'; chart cartesian {}",
                "AVENGER-PARSE-042",
            ),
            (
                "avenger 1; chart cartesian {} import * as thing from './thing.avenger';",
                "AVENGER-PARSE-040",
            ),
            ("avenger 1;", "AVENGER-PARSE-041"),
            (
                "avenger 1; import * as thing from './thing.avenger';",
                "AVENGER-PARSE-041",
            ),
            (
                "avenger 1; public chart cartesian as chart {}",
                "AVENGER-PARSE-044",
            ),
            ("avenger 1; param 1 as value;", "AVENGER-PARSE-045"),
        ];
        for (source, code) in cases {
            let source = SourceFile::new(
                SourceId::new(12),
                SourceOrigin::Memory("module-error.avenger".into()),
                source,
            );
            let error = parse_file(&source).expect_err(source.text());
            assert_eq!(error.diagnostic().code.as_str(), code, "{}", source.text());
        }
    }

    #[test]
    fn module_keywords_do_not_disturb_sql_islands() {
        let parsed = parse(
            r#"avenger 1;
export chart cartesian as grouped {
  table sql as summary {
    sql: SELECT "group", count(*) AS total
         FROM input
         GROUP BY "group";
  }
}"#,
        );
        let table = &only_item(&parsed).declaration.children[0];
        let crate::ast::Value::Query(query) = table.props.get("sql").unwrap() else {
            panic!("expected a query")
        };
        assert!(query.canonical_sql().contains("GROUP BY"));
    }
}
