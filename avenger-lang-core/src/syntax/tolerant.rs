use std::collections::BTreeMap;

use sqlparser::tokenizer::Token;

use crate::{
    ByteSpan, Diagnostic, SourceFile, SourceLabel, SourceSpan,
    sql::{LosslessTokenKind, LosslessTokenStream, TokenClass, tokenize_lossless_with_limit},
};

use super::{
    ParseError, ParsedFile, SqlIslandContext, SyntaxLimits, parse_file, parse_file_with_limits,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseMode {
    Strict,
    Tolerant,
}

#[derive(Clone, Debug)]
pub enum ParseModeOutput {
    Strict(ParsedFile),
    Tolerant(TolerantParsedFile),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TolerantSyntaxNodeId(u32);

impl TolerantSyntaxNodeId {
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TolerantSyntaxNodeKind {
    Root,
    Declaration {
        keyword: String,
        name: Option<String>,
    },
    Property {
        name: String,
    },
    SqlIsland {
        context: SqlIslandContext,
    },
    Token,
    MissingToken {
        expected: char,
    },
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TolerantSyntaxNode {
    pub id: TolerantSyntaxNodeId,
    pub parent: Option<TolerantSyntaxNodeId>,
    pub span: SourceSpan,
    pub kind: TolerantSyntaxNodeKind,
}

#[derive(Clone, Debug)]
pub struct TolerantParsedFile {
    pub strict: Option<ParsedFile>,
    pub tokens: LosslessTokenStream,
    pub nodes: Vec<TolerantSyntaxNode>,
    pub diagnostics: Vec<Diagnostic>,
}

impl TolerantParsedFile {
    pub fn enclosing_node(&self, offset: usize) -> Option<&TolerantSyntaxNode> {
        self.nodes
            .iter()
            .filter(|node| {
                node.span.range.start <= offset
                    && offset <= node.span.range.end
                    && !matches!(node.kind, TolerantSyntaxNodeKind::Token)
            })
            .min_by_key(|node| node.span.range.len())
    }
}

pub fn parse_file_with_mode(
    source: &SourceFile,
    mode: ParseMode,
) -> Result<ParseModeOutput, ParseError> {
    match mode {
        ParseMode::Strict => parse_file(source).map(ParseModeOutput::Strict),
        ParseMode::Tolerant => Ok(ParseModeOutput::Tolerant(parse_file_tolerant(source))),
    }
}

pub fn parse_file_tolerant(source: &SourceFile) -> TolerantParsedFile {
    parse_file_tolerant_with_limits(source, SyntaxLimits::default())
}

pub fn parse_file_tolerant_with_limits(
    source: &SourceFile,
    limits: SyntaxLimits,
) -> TolerantParsedFile {
    let tokens = tokenize_lossless_with_limit(source, limits.max_tokens);
    let strict_result = parse_file_with_limits(source, limits);
    let mut diagnostics = tokens.diagnostics().to_vec();
    let strict = match strict_result {
        Ok(parsed) => Some(parsed),
        Err(error) => {
            if diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != error.diagnostic().code)
            {
                diagnostics.push(error.into_diagnostic());
            }
            None
        }
    };
    let (nodes, diagnostics) = {
        let mut builder = TolerantTreeBuilder::new(source, &tokens, diagnostics, limits);
        builder.build();
        (builder.nodes, builder.diagnostics)
    };
    TolerantParsedFile {
        strict,
        tokens,
        nodes,
        diagnostics,
    }
}

#[derive(Clone, Copy)]
struct OpenDelimiter {
    delimiter: char,
    owner: Option<TolerantSyntaxNodeId>,
    span: SourceSpan,
}

struct TolerantTreeBuilder<'a> {
    source: &'a SourceFile,
    tokens: &'a LosslessTokenStream,
    significant: Vec<usize>,
    nodes: Vec<TolerantSyntaxNode>,
    diagnostics: Vec<Diagnostic>,
    delimiters: Vec<OpenDelimiter>,
    pending_delimiter_owners: BTreeMap<usize, TolerantSyntaxNodeId>,
    limits: SyntaxLimits,
    declaration_count: usize,
    reported_declaration_limit: bool,
}

impl<'a> TolerantTreeBuilder<'a> {
    fn new(
        source: &'a SourceFile,
        tokens: &'a LosslessTokenStream,
        diagnostics: Vec<Diagnostic>,
        limits: SyntaxLimits,
    ) -> Self {
        let significant = tokens
            .tokens()
            .iter()
            .enumerate()
            .filter_map(|(index, token)| match token.kind() {
                LosslessTokenKind::Token(TokenClass::Whitespace(_) | TokenClass::Comment(_))
                | LosslessTokenKind::Eof => None,
                _ => Some(index),
            })
            .collect();
        Self {
            source,
            tokens,
            significant,
            nodes: vec![TolerantSyntaxNode {
                id: TolerantSyntaxNodeId(0),
                parent: None,
                span: SourceSpan {
                    source: source.id,
                    range: ByteSpan {
                        start: 0,
                        end: source.text().len(),
                    },
                },
                kind: TolerantSyntaxNodeKind::Root,
            }],
            diagnostics,
            delimiters: Vec::new(),
            pending_delimiter_owners: BTreeMap::new(),
            limits,
            declaration_count: 0,
            reported_declaration_limit: false,
        }
    }

    fn build(&mut self) {
        let token_nodes = self
            .tokens
            .tokens()
            .iter()
            .filter(|token| token.kind() != LosslessTokenKind::Eof)
            .map(|token| {
                let kind = if token.kind() == LosslessTokenKind::Error {
                    TolerantSyntaxNodeKind::Error
                } else {
                    TolerantSyntaxNodeKind::Token
                };
                (token.span(), kind)
            })
            .collect::<Vec<_>>();
        for (span, kind) in token_nodes {
            self.push_node(Some(TolerantSyntaxNodeId(0)), span, kind);
        }

        let mut position = 0;
        while position < self.significant.len() {
            let token_index = self.significant[position];
            let token = &self.tokens.tokens()[token_index];
            let parent = self.current_owner();
            if token.kind() == LosslessTokenKind::Error {
                position += 1;
                continue;
            }

            if let Some(delimiter) = opening_delimiter(token.token()) {
                if self.delimiters.len() < self.limits.max_nesting_depth {
                    self.delimiters.push(OpenDelimiter {
                        delimiter,
                        owner: self.pending_delimiter_owners.remove(&position).or(parent),
                        span: token.span(),
                    });
                } else {
                    self.diagnostics.push(Diagnostic::error(
                        "AVENGER-PARSE-RECOVER-004",
                        "tolerant syntax nesting limit exceeded",
                        SourceLabel::new(
                            token.span(),
                            format!(
                                "configured nesting limit is {}",
                                self.limits.max_nesting_depth
                            ),
                        ),
                    ));
                }
            } else if let Some(delimiter) = closing_delimiter(token.token()) {
                self.close_delimiter(delimiter, token.span());
            }

            if self.at_statement_boundary(position) {
                let declaration_position = if self
                    .word_at(position)
                    .is_some_and(|word| matches!(word, "public" | "private"))
                {
                    position + 1
                } else {
                    position
                };
                let declaration = self
                    .declaration_keyword(declaration_position)
                    .map(|keyword| (keyword, None))
                    .or_else(|| {
                        self.is_predicate_owner(parent)
                            .then(|| {
                                self.word_at(declaration_position)
                                    .filter(|_| {
                                        self.token_at_significant_opt(declaration_position + 1)
                                            .is_some_and(|token| {
                                                matches!(token.token(), Some(Token::LBrace))
                                            })
                                    })
                                    .map(|name| ("dimension".to_owned(), Some(name.to_owned())))
                            })
                            .flatten()
                    });
                if let Some((keyword, recovered_name)) = declaration {
                    if self.declaration_count >= self.limits.max_declarations {
                        if !self.reported_declaration_limit {
                            self.diagnostics.push(Diagnostic::error(
                                "AVENGER-PARSE-RECOVER-005",
                                "tolerant declaration limit exceeded",
                                SourceLabel::new(
                                    token.span(),
                                    format!(
                                        "configured declaration limit is {}",
                                        self.limits.max_declarations
                                    ),
                                ),
                            ));
                            self.reported_declaration_limit = true;
                        }
                        position += 1;
                        continue;
                    }
                    self.declaration_count += 1;
                    let end_position = self.find_header_end(declaration_position);
                    let end_span = self.token_at_significant(end_position).span();
                    let name = recovered_name
                        .or_else(|| self.declaration_name(declaration_position, end_position));
                    let id = self.push_node(
                        parent,
                        SourceSpan {
                            source: self.source.id,
                            range: ByteSpan {
                                start: token.span().range.start,
                                end: end_span.range.end,
                            },
                        },
                        TolerantSyntaxNodeKind::Declaration {
                            keyword: keyword.clone(),
                            name,
                        },
                    );
                    if matches!(
                        end_span_token(self.token_at_significant(end_position)),
                        Some('{')
                    ) {
                        self.pending_delimiter_owners.insert(end_position, id);
                    }
                    if keyword == "output"
                        && let Some((start, end)) =
                            self.output_island_bounds(declaration_position, end_position)
                    {
                        self.push_node(
                            Some(id),
                            SourceSpan {
                                source: self.source.id,
                                range: ByteSpan { start, end },
                            },
                            TolerantSyntaxNodeKind::SqlIsland {
                                context: SqlIslandContext::AliasedExpression,
                            },
                        );
                    }
                    if keyword == "set"
                        && let Some((start, end)) =
                            self.action_island_bounds(declaration_position, end_position)
                    {
                        self.push_node(
                            Some(id),
                            SourceSpan {
                                source: self.source.id,
                                range: ByteSpan { start, end },
                            },
                            TolerantSyntaxNodeKind::SqlIsland {
                                context: SqlIslandContext::TerminatedExpression,
                            },
                        );
                    }
                }
            }

            if self.word_at(position).is_some()
                && self
                    .token_at_significant_opt(position + 1)
                    .is_some_and(|next| matches!(next.token(), Some(Token::Colon)))
            {
                let name = self.word_at(position).unwrap().to_owned();
                let value_start = position + 2;
                let value_end = self.find_property_end(value_start);
                let end = self
                    .token_at_significant_opt(value_end)
                    .map_or(token.span().range.end, |value| value.span().range.end);
                let property = self.push_node(
                    parent,
                    SourceSpan {
                        source: self.source.id,
                        range: ByteSpan {
                            start: token.span().range.start,
                            end,
                        },
                    },
                    TolerantSyntaxNodeKind::Property { name: name.clone() },
                );
                if let Some(body_position) = (value_start..=value_end).find(|position| {
                    self.token_at_significant_opt(*position)
                        .is_some_and(|token| matches!(token.token(), Some(Token::LBrace)))
                }) {
                    self.pending_delimiter_owners
                        .insert(body_position, property);
                }
                if value_start <= value_end {
                    if let (Some(first), Some(last)) = (
                        self.token_at_significant_opt(value_start),
                        self.token_at_significant_opt(value_end),
                    ) {
                        let island_end = if matches!(last.token(), Some(Token::SemiColon)) {
                            last.span().range.start
                        } else {
                            last.span().range.end
                        };
                        if first.span().range.start <= island_end {
                            self.push_node(
                                Some(property),
                                SourceSpan {
                                    source: self.source.id,
                                    range: ByteSpan {
                                        start: first.span().range.start,
                                        end: island_end,
                                    },
                                },
                                TolerantSyntaxNodeKind::SqlIsland {
                                    context: if name.eq_ignore_ascii_case("sql") {
                                        SqlIslandContext::QueryProperty
                                    } else {
                                        SqlIslandContext::PropertyExpression
                                    },
                                },
                            );
                        }
                    }
                }
                if !self
                    .token_at_significant_opt(value_end)
                    .is_some_and(|last| matches!(last.token(), Some(Token::SemiColon)))
                {
                    self.missing(';', end, Some(property));
                    let span = SourceSpan::empty(self.source.id, end);
                    self.diagnostics.push(Diagnostic::error(
                        "AVENGER-PARSE-RECOVER-003",
                        "missing `;`",
                        SourceLabel::new(span, "insert `;` here"),
                    ));
                }
            }
            position += 1;
        }

        let eof = self.source.text().len();
        for open in self.delimiters.clone().into_iter().rev() {
            self.missing(matching_close(open.delimiter), eof, open.owner);
            self.diagnostics.push(Diagnostic::error(
                "AVENGER-PARSE-RECOVER-001",
                format!("missing `{}`", matching_close(open.delimiter)),
                SourceLabel::new(open.span, "opening delimiter is not closed"),
            ));
        }
    }

    fn push_node(
        &mut self,
        parent: Option<TolerantSyntaxNodeId>,
        span: SourceSpan,
        kind: TolerantSyntaxNodeKind,
    ) -> TolerantSyntaxNodeId {
        let id = TolerantSyntaxNodeId(self.nodes.len() as u32);
        self.nodes.push(TolerantSyntaxNode {
            id,
            parent,
            span,
            kind,
        });
        id
    }

    fn missing(&mut self, expected: char, offset: usize, parent: Option<TolerantSyntaxNodeId>) {
        self.push_node(
            parent,
            SourceSpan::empty(self.source.id, offset),
            TolerantSyntaxNodeKind::MissingToken { expected },
        );
    }

    fn current_owner(&self) -> Option<TolerantSyntaxNodeId> {
        self.delimiters
            .iter()
            .rev()
            .find_map(|delimiter| delimiter.owner)
            .or(Some(TolerantSyntaxNodeId(0)))
    }

    fn close_delimiter(&mut self, close: char, span: SourceSpan) {
        let expected_open = matching_open(close);
        if self
            .delimiters
            .last()
            .is_some_and(|open| open.delimiter == expected_open)
        {
            let open = self.delimiters.pop().unwrap();
            if let Some(owner) = open.owner {
                self.nodes[owner.0 as usize].span.range.end = span.range.end;
            }
        } else {
            self.push_node(self.current_owner(), span, TolerantSyntaxNodeKind::Error);
            self.diagnostics.push(Diagnostic::error(
                "AVENGER-PARSE-RECOVER-002",
                format!("unmatched `{close}`"),
                SourceLabel::new(span, "no matching opening delimiter"),
            ));
        }
    }

    fn at_statement_boundary(&self, position: usize) -> bool {
        position == 0
            || self
                .token_at_significant_opt(position - 1)
                .and_then(|token| token.token())
                .is_some_and(|token| {
                    matches!(token, Token::SemiColon | Token::LBrace | Token::RBrace)
                })
    }

    fn find_header_end(&self, start: usize) -> usize {
        (start..self.significant.len())
            .find(|position| {
                matches!(
                    self.token_at_significant(*position).token(),
                    Some(Token::LBrace | Token::SemiColon)
                )
            })
            .unwrap_or(self.significant.len() - 1)
    }

    fn declaration_name(&self, start: usize, end: usize) -> Option<String> {
        for position in start..end {
            if self
                .word_at(position)
                .is_some_and(|word| word.eq_ignore_ascii_case("as"))
            {
                return self.word_at(position + 1).map(str::to_owned);
            }
        }
        let keyword = self.word_at(start)?;
        let fallback = match keyword {
            "define" | "slot" | "variable" => Some(start + 2),
            "field" => self.name_after_physical_type(start + 1, end),
            "output" | "export" => Some(start + 1),
            _ => None,
        };
        fallback
            .filter(|fallback| *fallback <= end)
            .and_then(|fallback| self.word_at(fallback))
            .map(str::to_owned)
    }

    fn name_after_physical_type(&self, type_start: usize, end: usize) -> Option<usize> {
        let mut next = type_start + 1;
        if matches!(
            self.token_at_significant_opt(next)
                .and_then(|token| token.token()),
            Some(Token::LParen)
        ) {
            let mut depth = 0usize;
            while next < end {
                match self.token_at_significant(next).token() {
                    Some(Token::LParen) => depth += 1,
                    Some(Token::RParen) => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            next += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                next += 1;
            }
        }
        (next < end).then_some(next)
    }

    fn declaration_keyword(&self, position: usize) -> Option<String> {
        let keyword = self.word_at(position)?;
        if !SOURCE_DECLARATION_KEYWORDS.contains(&keyword) {
            return None;
        }
        let semantic = match (keyword, self.word_at(position + 1)) {
            ("param", Some("store")) => "store",
            ("param", Some("selection")) => "selection",
            _ => keyword,
        };
        Some(semantic.to_owned())
    }

    fn is_predicate_owner(&self, parent: Option<TolerantSyntaxNodeId>) -> bool {
        parent.is_some_and(|parent| {
            matches!(
                &self.nodes[parent.get() as usize].kind,
                TolerantSyntaxNodeKind::Declaration { keyword, .. }
                    if matches!(keyword.as_str(), "equality" | "interval")
            )
        })
    }

    fn output_island_bounds(&self, start: usize, end: usize) -> Option<(usize, usize)> {
        let first = self.token_at_significant_opt(start + 1)?;
        let mut nesting = 0usize;
        let mut island_end = self.token_at_significant_opt(end)?.span().range.start;
        for position in start + 1..end {
            let token = self.token_at_significant(position);
            match token.token() {
                Some(Token::LParen | Token::LBracket | Token::LBrace) => nesting += 1,
                Some(Token::RParen | Token::RBracket | Token::RBrace) if nesting > 0 => {
                    nesting -= 1
                }
                Some(Token::Word(word))
                    if nesting == 0 && word.value.eq_ignore_ascii_case("as") =>
                {
                    island_end = token.span().range.start;
                    break;
                }
                _ => {}
            }
        }
        (first.span().range.start < island_end).then_some((first.span().range.start, island_end))
    }

    fn action_island_bounds(&self, start: usize, end: usize) -> Option<(usize, usize)> {
        let equals = (start + 1..=end).find(|position| {
            matches!(
                self.token_at_significant_opt(*position)
                    .and_then(|token| token.token()),
                Some(Token::Eq)
            )
        })?;
        let value_start = equals + 1;
        let first = self.token_at_significant_opt(value_start)?;
        if self
            .token_at_significant_opt(value_start + 1)
            .is_some_and(|next| matches!(next.token(), Some(Token::LBrace)))
        {
            return None;
        }
        let last = self.token_at_significant_opt(end)?;
        let island_end = if matches!(last.token(), Some(Token::SemiColon)) {
            last.span().range.start
        } else {
            last.span().range.end
        };
        (first.span().range.start < island_end).then_some((first.span().range.start, island_end))
    }

    fn find_property_end(&self, start: usize) -> usize {
        if start >= self.significant.len() {
            return start;
        }
        let mut nesting = 0usize;
        for position in start..self.significant.len() {
            match self.token_at_significant(position).token() {
                Some(Token::LParen | Token::LBracket) => nesting += 1,
                Some(Token::RParen | Token::RBracket) if nesting > 0 => nesting -= 1,
                Some(Token::SemiColon) if nesting == 0 => return position,
                Some(Token::RBrace) if nesting == 0 => return position.saturating_sub(1),
                _ => {}
            }
        }
        self.significant.len() - 1
    }

    fn word_at(&self, position: usize) -> Option<&str> {
        match self.token_at_significant_opt(position)?.token()? {
            Token::Word(word) => Some(word.value.as_str()),
            _ => None,
        }
    }

    fn token_at_significant(&self, position: usize) -> &crate::sql::LosslessToken {
        &self.tokens.tokens()[self.significant[position]]
    }

    fn token_at_significant_opt(&self, position: usize) -> Option<&crate::sql::LosslessToken> {
        self.significant
            .get(position)
            .map(|index| &self.tokens.tokens()[*index])
    }
}

/// Canonical source starters. Internal semantic declaration keywords such as
/// `store`, `selection`, `dimension`, and `channel` do not
/// enter the tolerant source grammar through this list.
const SOURCE_DECLARATION_KEYWORDS: &[&str] = &[
    "adjust",
    "axis",
    "catalog",
    "cell",
    "chart",
    "define",
    "derive",
    "equality",
    "export",
    "field",
    "fields",
    "frame",
    "key",
    "layer",
    "layout",
    "legend",
    "level",
    "interval",
    "mark",
    "match",
    "on",
    "output",
    "param",
    "part",
    "plot",
    "resource",
    "row",
    "scale_edit",
    "scale_hint",
    "schema",
    "set",
    "slot",
    "splice",
    "table",
    "theme",
    "tool",
    "transform",
    "variable",
    "view",
    "when",
    "widget",
];

fn opening_delimiter(token: Option<&Token>) -> Option<char> {
    match token {
        Some(Token::LBrace) => Some('{'),
        Some(Token::LBracket) => Some('['),
        Some(Token::LParen) => Some('('),
        _ => None,
    }
}

fn closing_delimiter(token: Option<&Token>) -> Option<char> {
    match token {
        Some(Token::RBrace) => Some('}'),
        Some(Token::RBracket) => Some(']'),
        Some(Token::RParen) => Some(')'),
        _ => None,
    }
}

fn matching_close(open: char) -> char {
    match open {
        '{' => '}',
        '[' => ']',
        '(' => ')',
        _ => open,
    }
}

fn matching_open(close: char) -> char {
    match close {
        '}' => '{',
        ']' => '[',
        ')' => '(',
        _ => close,
    }
}

fn end_span_token(token: &crate::sql::LosslessToken) -> Option<char> {
    opening_delimiter(token.token()).or_else(|| closing_delimiter(token.token()))
}

#[cfg(test)]
mod tests {
    use crate::{SourceFile, SourceId, SourceOrigin};

    use super::{TolerantSyntaxNodeKind, parse_file_tolerant};

    #[test]
    fn incomplete_document_keeps_declarations_properties_and_missing_tokens() {
        let source = SourceFile::new(
            SourceId::new(1),
            SourceOrigin::Memory("editing".into()),
            "avenger 1; chart cartesian as chart { mark symbol as points { x: 1; y: ",
        );
        let parsed = parse_file_tolerant(&source);
        assert!(parsed.strict.is_none());
        assert!(parsed.nodes.iter().any(|node| {
            matches!(
                &node.kind,
                TolerantSyntaxNodeKind::Declaration { keyword, name }
                    if keyword == "chart" && name.as_deref() == Some("chart")
            )
        }));
        assert!(parsed.nodes.iter().any(|node| {
            matches!(&node.kind, TolerantSyntaxNodeKind::Property { name } if name == "x")
        }));
        assert!(parsed.nodes.iter().any(|node| {
            matches!(
                node.kind,
                TolerantSyntaxNodeKind::MissingToken { expected: '}' }
            )
        }));
    }

    #[test]
    fn valid_document_retains_strict_authoritative_parse() {
        let source = SourceFile::new(
            SourceId::new(1),
            SourceOrigin::Memory("valid".into()),
            "avenger 1; chart cartesian as chart {}",
        );
        let parsed = parse_file_tolerant(&source);
        assert!(parsed.strict.is_some());
        assert!(parsed.diagnostics.is_empty());
    }

    #[test]
    fn declaration_hierarchy_and_visibility_prefix_survive_recovery() {
        let source = SourceFile::new(
            SourceId::new(1),
            SourceOrigin::Memory("hierarchy".into()),
            "avenger 1; chart cartesian as chart { public param float64 as width { value: 1.0; } mark symbol as points {} }",
        );
        let parsed = parse_file_tolerant(&source);
        let chart = parsed
            .nodes
            .iter()
            .find(|node| {
                matches!(&node.kind, TolerantSyntaxNodeKind::Declaration { keyword, .. } if keyword == "chart")
            })
            .unwrap();
        let children = parsed
            .nodes
            .iter()
            .filter(|node| {
                node.parent == Some(chart.id)
                    && matches!(node.kind, TolerantSyntaxNodeKind::Declaration { .. })
            })
            .collect::<Vec<_>>();
        assert_eq!(children.len(), 2);
        assert!(children.iter().any(|node| {
            matches!(&node.kind, TolerantSyntaxNodeKind::Declaration { keyword, name } if keyword == "param" && name.as_deref() == Some("width"))
        }));
        assert!(children.iter().any(|node| {
            matches!(&node.kind, TolerantSyntaxNodeKind::Declaration { keyword, name } if keyword == "mark" && name.as_deref() == Some("points"))
        }));
    }

    #[test]
    fn unified_headers_recover_semantic_categories_and_direct_names() {
        let source = SourceFile::new(
            SourceId::new(1),
            SourceOrigin::Memory("headers".into()),
            r#"avenger 1; define mark sample {
              slot channel x;
              slot expr amount;
              output amount;
              output amount + 1 as next;
              param store as rows {}
              param selection as picked {}
              mark group as layer {}
              variable row mpg {}
              field float64 value;
              field struct(field(float64, 'x')) position;
              equality { id { field: "id"; value: 1; } }
            }"#,
        );
        let parsed = parse_file_tolerant(&source);
        let declarations = parsed
            .nodes
            .iter()
            .filter_map(|node| match &node.kind {
                TolerantSyntaxNodeKind::Declaration { keyword, name } => {
                    Some((keyword.as_str(), name.as_deref()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for expected in [
            ("slot", Some("x")),
            ("slot", Some("amount")),
            ("output", Some("amount")),
            ("output", Some("next")),
            ("store", Some("rows")),
            ("selection", Some("picked")),
            ("mark", Some("layer")),
            ("variable", Some("mpg")),
            ("field", Some("value")),
            ("field", Some("position")),
            ("dimension", Some("id")),
        ] {
            assert!(declarations.contains(&expected), "{declarations:?}");
        }
        assert!(parsed.nodes.iter().any(|node| {
            matches!(
                node.kind,
                TolerantSyntaxNodeKind::SqlIsland {
                    context: super::SqlIslandContext::AliasedExpression
                }
            )
        }));
    }

    #[test]
    fn action_expression_islands_exclude_structural_update_blocks() {
        let source = SourceFile::new(
            SourceId::new(2),
            SourceOrigin::Memory("actions".into()),
            "avenger 1; chart cartesian { on click { set width = $width + 1; set rows = insert_rows { row { id: 1; } } } }",
        );
        let parsed = parse_file_tolerant(&source);
        let islands = parsed
            .nodes
            .iter()
            .filter_map(|node| match node.kind {
                TolerantSyntaxNodeKind::SqlIsland {
                    context: super::SqlIslandContext::TerminatedExpression,
                } => Some(&source.text()[node.span.range.as_range()]),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(islands, ["$width + 1"]);
    }

    #[test]
    fn object_properties_own_nested_recovery_scopes() {
        let source = SourceFile::new(
            SourceId::new(3),
            SourceOrigin::Memory("nested-properties".into()),
            "avenger 1; chart parallel { dimensions: { mpg: { axis: { title: 'MPG'; } } } }",
        );
        let parsed = parse_file_tolerant(&source);
        let property = |name: &str| {
            parsed
                .nodes
                .iter()
                .find(|node| {
                    matches!(
                        &node.kind,
                        TolerantSyntaxNodeKind::Property { name: candidate }
                            if candidate == name
                    )
                })
                .unwrap()
        };
        assert_eq!(property("mpg").parent, Some(property("dimensions").id));
        assert_eq!(property("axis").parent, Some(property("mpg").id));
        assert_eq!(property("title").parent, Some(property("axis").id));
    }
}
