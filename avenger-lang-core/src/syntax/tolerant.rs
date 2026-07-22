use sqlparser::tokenizer::Token;

use crate::{
    ByteSpan, Diagnostic, SourceFile, SourceLabel, SourceSpan,
    sql::{LosslessTokenKind, LosslessTokenStream, TokenClass, tokenize_lossless},
};

use super::{ParseError, ParsedFile, SqlIslandContext, parse_file};

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
    let tokens = tokenize_lossless(source);
    let strict_result = parse_file(source);
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
        let mut builder = TolerantTreeBuilder::new(source, &tokens, diagnostics);
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
}

impl<'a> TolerantTreeBuilder<'a> {
    fn new(
        source: &'a SourceFile,
        tokens: &'a LosslessTokenStream,
        diagnostics: Vec<Diagnostic>,
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
                self.delimiters.push(OpenDelimiter {
                    delimiter,
                    owner: parent,
                    span: token.span(),
                });
            } else if let Some(delimiter) = closing_delimiter(token.token()) {
                self.close_delimiter(delimiter, token.span());
            }

            if self.at_statement_boundary(position) {
                if let Some(keyword) = self.word_at(position).filter(|word| is_declaration(word)) {
                    let end_position = self.find_header_end(position);
                    let end_span = self.token_at_significant(end_position).span();
                    let name = self.declaration_name(position, end_position);
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
                            keyword: keyword.to_owned(),
                            name,
                        },
                    );
                    if matches!(
                        end_span_token(self.token_at_significant(end_position)),
                        Some('{')
                    ) {
                        if let Some(open) = self.delimiters.last_mut() {
                            open.owner = Some(id);
                        }
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
        let fallback = if keyword.eq_ignore_ascii_case("define") {
            start + 2
        } else {
            start + 1
        };
        (fallback <= end)
            .then(|| self.word_at(fallback))
            .flatten()
            .map(str::to_owned)
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

fn is_declaration(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "import"
            | "catalog"
            | "schema"
            | "table"
            | "chart"
            | "define"
            | "param"
            | "store"
            | "selection"
            | "tool"
            | "widget"
            | "group"
            | "mark"
            | "transform"
            | "scale"
            | "axis"
            | "legend"
            | "projection"
            | "event"
    )
}

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
}
