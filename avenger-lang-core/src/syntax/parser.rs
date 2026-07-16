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
        Import, Name, NumericLiteral, PropertyMap, RefKind, Root, SqlExpression, SqlQuery, Value,
        Visibility,
    },
    sql::{
        ParsedSqlIsland, SqlFrontendError, TokenClass, TokenStream, parse_sql_expression,
        parse_sql_query, tokenize,
    },
};

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
    pub concrete: ConcreteFile,
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
    let tokens = tokenize(source).map_err(|error| ParseError {
        diagnostic: Box::new(error.into_diagnostic()),
    })?;
    let mut parser = Parser::new(tokens.clone());
    let ast = parser.file()?;
    let source_map = parser.source_map;
    let nodes = concrete_nodes(&tokens, &source_map);
    Ok(ParsedFile {
        ast,
        source_map,
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
}

impl Parser {
    fn new(stream: TokenStream) -> Self {
        Self {
            stream,
            index: 0,
            next_node_id: 0,
            source_map: AstSourceMap::default(),
        }
    }

    fn file(&mut self) -> Result<File, ParseError> {
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
        let mut imports = Vec::new();
        while self.word_is("import") {
            imports.push(self.import()?);
        }
        let _ = self.leading_doc();
        let root = if self.word_is("chart") {
            Root::Chart(self.chart()?)
        } else if self.word_is("define") {
            Root::Define(self.definition()?)
        } else if matches!(self.word(), Some("catalog" | "schema" | "table")) {
            let mut declarations = Vec::new();
            while matches!(self.word(), Some("catalog" | "schema" | "table")) {
                declarations.push(self.declaration(None)?);
                let _ = self.leading_doc();
            }
            Root::Data(declarations)
        } else {
            return Err(self.error(
                "AVENGER-PARSE-004",
                "expected one chart, definition, or data root",
            ));
        };
        self.trivia();
        if !self.eof() {
            return Err(self.error("AVENGER-PARSE-005", "unexpected tokens after file root"));
        }
        Ok(File {
            version,
            name: None,
            imports,
            root,
        })
    }

    fn import(&mut self) -> Result<Import, ParseError> {
        self.expect_word("import")?;
        let source = self.string()?;
        let sha256 = self
            .consume_word("sha256")
            .then(|| self.string())
            .transpose()?;
        let alias = self.consume_word("as").then(|| self.name()).transpose()?;
        self.expect(Token::SemiColon, "`;` after import")?;
        Ok(Import {
            source,
            sha256,
            alias,
        })
    }

    fn chart(&mut self) -> Result<Decl, ParseError> {
        let start = self.start();
        self.expect_word("chart")?;
        let kind = self.name()?;
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
        let binder = self.name()?;
        let body = self.body()?;
        let mut saw_body_item = false;
        for child in &body.children {
            if matches!(child.keyword.as_str(), "slot" | "channel") {
                if saw_body_item {
                    return Err(self.error(
                        "AVENGER-PARSE-020",
                        "slot and channel interfaces must precede definition body items",
                    ));
                }
            } else if !matches!(child.keyword.as_str(), "output" | "export") {
                saw_body_item = true;
            }
        }
        self.finish(
            start,
            from_body(n("define"), Some(kind), Some(binder), body),
        )
    }

    fn body(&mut self) -> Result<Body, ParseError> {
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
            } else {
                body.children.push(self.declaration(doc)?);
            }
        }
    }

    fn property_value(&mut self, property: &str) -> Result<Value, ParseError> {
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
        if self.word_is("value") {
            self.expect_word("value")?;
            let inner = self.expression(BindingKind::Param)?;
            return self
                .terminated(inner)
                .map(|value| Value::Visual(Box::new(value)));
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
        if property == "sql" {
            let start = self.sig();
            let parsed = parse_sql_query(&self.stream, start)?;
            self.index = parsed.next_token;
            self.expect(Token::SemiColon, "`;` after SQL query")?;
            return SqlQuery::from_parsed(parsed)
                .map(|query| Value::Query(Box::new(query)))
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
        let kind = if property == "data" {
            BindingKind::Store
        } else {
            BindingKind::Param
        };
        let value = self.expression(kind)?;
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
            } else if self.word_is("value") {
                self.expect_word("value")?;
                Value::Visual(Box::new(self.expression(BindingKind::Param)?))
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
                self.expression(BindingKind::Param)?
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

    fn expression(&mut self, binding_kind: BindingKind) -> Result<Value, ParseError> {
        let start = self.sig();
        let parsed = parse_sql_expression(&self.stream, start)?;
        self.index = parsed.next_token;
        expression_value(parsed, binding_kind).map_err(|error| self.ast_error(error))
    }

    // Declaration shapes and cursor helpers continue below.
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
            | "resource" | "variable" | "derive" => self.kind_bind_body()?,
            "tool" => self.tool()?,
            "param" | "store" | "selection" | "dimension" => self.bind_body()?,
            "group" | "overlay" => self.optional_bind_body()?,
            "on" => self.event()?,
            "cell" => self.cell()?,
            "plot" => self.kind_body()?,
            "part" | "layer" => self.named_body()?,
            "level" => self.level()?,
            "adjust" => self.adjust()?,
            "row" | "when" | "key" | "fields" | "scale_edit" | "scale_hint" => self.plain_body()?,
            "field" => self.field()?,
            "slot" => self.slot()?,
            "channel" => self.channel()?,
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
        let kind = self.name()?;
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
        let kind = self.name()?;
        let binder = self.optional_binder()?;
        if self.consume(Token::SemiColon) {
            Ok(from_body(keyword, Some(kind), binder, Body::default()))
        } else {
            let body = self.body()?;
            Ok(from_body(keyword, Some(kind), binder, body))
        }
    }

    fn bind_body(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        self.expect_word("as")?;
        let binder = self.name()?;
        let body = self.body()?;
        Ok(from_body(keyword, None, Some(binder), body))
    }

    fn optional_bind_body(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let binder = self.optional_binder()?;
        let body = self.body()?;
        Ok(from_body(keyword, None, binder, body))
    }

    fn event(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let kind = self.name()?;
        let binder = self.optional_binder()?;
        let body = self.body()?;
        Ok(from_body(keyword, Some(kind), binder, body))
    }

    fn cell(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let kind = self.name()?;
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
        let kind = self.name()?;
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
        let (kind, binder) = if self.is(&Token::LBrace) {
            (None, None)
        } else {
            (Some(self.name()?), self.optional_binder()?)
        };
        let body = self.body()?;
        Ok(from_body(keyword, kind, binder, body))
    }

    fn plain_body(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let body = self.body()?;
        Ok(from_body(keyword, None, None, body))
    }

    fn field(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let binder = self.name()?;
        self.expect(Token::Colon, "`:` after field name")?;
        let value = self.expression(BindingKind::Param)?;
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
        if !matches!(
            kind.as_str(),
            "expr"
                | "expr_list"
                | "literal"
                | "number"
                | "string"
                | "boolean"
                | "enum"
                | "function"
                | "ref"
                | "block"
        ) {
            return Err(self.error("AVENGER-PARSE-021", "unknown definition slot shape"));
        }
        self.expect_word("as")?;
        let binder = self.name()?;
        let body = if self.consume(Token::SemiColon) {
            Body::default()
        } else {
            self.body()?
        };
        Ok(from_body(keyword, Some(kind), Some(binder), body))
    }

    fn channel(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let binder = self.name()?;
        let kind = self
            .consume(Token::Colon)
            .then(|| self.name())
            .transpose()?;
        self.expect(Token::SemiColon, "`;` after channel")?;
        Ok(Decl {
            keyword,
            kind,
            name: Some(binder),
            ..Decl::new(n("channel"))
        })
    }

    fn output(&mut self) -> Result<Decl, ParseError> {
        let keyword = self.name()?;
        let binder = self.name()?;
        let mut props = PropertyMap::default();
        if self.consume(Token::Colon) {
            let value = self.expression(BindingKind::Param)?;
            props.insert(n("value"), value).expect("new property");
        }
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
        let kind = self.name()?;
        let mut props = PropertyMap::default();
        if kind.as_str() == "cursor" {
            self.expect(Token::Eq, "`=` in cursor action")?;
            let value = self.expression(BindingKind::Param)?;
            props.insert(n("value"), value).expect("new property");
            self.expect(Token::SemiColon, "`;` after cursor action")?;
        } else {
            if !matches!(kind.as_str(), "param" | "store" | "selection") {
                return Err(self.error(
                    "AVENGER-PARSE-013",
                    "set target must be param, store, selection, or cursor",
                ));
            }
            let target = self.qual()?;
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
                let value = self.expression(BindingKind::Param)?;
                self.expect(Token::SemiColon, "`;` after set action")?;
                value
            };
            props.insert(n("value"), value).expect("new property");
        }
        Ok(Decl {
            keyword,
            kind: Some(kind),
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
            kind: Some(n("css")),
            props,
            ..Decl::new(n("theme"))
        })
    }
}

impl Parser {
    fn optional_binder(&mut self) -> Result<Option<Name>, ParseError> {
        self.consume_word("as").then(|| self.name()).transpose()
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
            "group" => Some(RefKind::Group),
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
                } else if !raw.trim().is_empty() {
                    lines.clear();
                }
            }
            self.index += 1;
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
    if let Some(value) = literal_value(expression.ast())? {
        return Ok(value);
    }
    Ok(Value::Expr(Box::new(expression)))
}

fn literal_value(expression: &Expr) -> Result<Option<Value>, AstError> {
    match expression {
        Expr::Value(value) => match &value.value {
            SqlValue::Number(value, false) => NumericLiteral::new(value).map(Value::Num).map(Some),
            SqlValue::SingleQuotedString(value) => Ok(Some(Value::Str(value.clone()))),
            SqlValue::Boolean(value) => Ok(Some(Value::Bool(*value))),
            SqlValue::Null => Ok(Some(Value::Null)),
            _ => Ok(None),
        },
        Expr::Identifier(identifier) if identifier.quote_style == Some('"') => {
            Ok(Some(Value::Column(identifier.value.clone())))
        }
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
                let Some(value) = literal_value(expression)? else {
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
        Expr::Struct { values, fields } if fields.is_empty() => {
            let mut args = Vec::with_capacity(values.len());
            for expression in values {
                let Some(value) = literal_value(expression)? else {
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

fn from_body(keyword: Name, kind: Option<Name>, name: Option<Name>, body: Body) -> Decl {
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
    use crate::{SourceFile, SourceId, SourceOrigin, ast::Root};

    use super::parse_file;

    fn parse(source: &str) -> super::ParsedFile {
        parse_file(&SourceFile::new(
            SourceId::new(1),
            SourceOrigin::Memory("test.avenger".into()),
            source,
        ))
        .unwrap()
    }

    #[test]
    fn parse_chart_root_with_mixed_body_and_sql() {
        let parsed = parse(
            r#"
avenger 1;
import './data.data.avenger' as data;
chart cartesian as example {
  height: 400;
  data: $rows;
  -- | Visible points.
  mark symbol as points {
    x: "Horsepower";
    size: $radius@start * 2;
  }
  on pointermove as drag {
    set param radius at start = $radius + 1;
  }
}
"#,
        );
        let Root::Chart(chart) = parsed.ast.root else {
            panic!("expected chart root")
        };
        assert_eq!(chart.children.len(), 2);
        assert_eq!(chart.children[0].doc.as_deref(), Some("Visible points."));
        assert!(!parsed.source_map.is_empty());
    }

    #[test]
    fn parse_definition_and_data_roots() {
        let definition = parse(
            "avenger 1; define mark badge { slot number as radius; channel x; mark symbol {} }",
        );
        assert!(matches!(definition.ast.root, Root::Define(_)));

        let data = parse(
            "avenger 1; catalog memory as local { schema tables as vega { table inline as movies { values: []; } } }",
        );
        assert!(matches!(data.ast.root, Root::Data(_)));
    }

    #[test]
    fn parse_rejects_multiple_roots_and_duplicate_properties() {
        let source = SourceFile::new(
            SourceId::new(1),
            SourceOrigin::Memory("bad.avenger".into()),
            "avenger 1; chart cartesian {} chart polar {}",
        );
        assert!(parse_file(&source).is_err());

        let source = SourceFile::new(
            SourceId::new(2),
            SourceOrigin::Memory("bad.avenger".into()),
            "avenger 1; chart cartesian { width: 1; width: 2; }",
        );
        assert!(parse_file(&source).is_err());
    }
}
