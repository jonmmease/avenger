use std::collections::BTreeMap;

use sha2::{Digest, Sha256};
use sqlparser::tokenizer::Token;

use crate::{
    ByteSpan, Diagnostic, SourceFile, SourceLabel, SourceSpan,
    sql::{LosslessTokenKind, LosslessTokenStream, TokenClass, tokenize_lossless_with_limit},
};

use super::{
    ParseError, ParsedFile, SqlIslandSite, SyntaxLimits, parse_file, parse_file_with_limits,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseMode {
    Strict,
    Tolerant,
}

#[derive(Clone, Debug)]
pub enum ParseModeOutput {
    Strict(Box<ParsedFile>),
    Tolerant(Box<TolerantParsedFile>),
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
    VersionPragma {
        version: Option<u32>,
    },
    Import,
    ImportSpecifier {
        imported: Option<String>,
        local: Option<String>,
    },
    ModuleItem {
        exported: bool,
        keyword: Option<String>,
        kind: Vec<String>,
        name: Option<String>,
        chart_name: Option<String>,
    },
    Recovery {
        context: TolerantRecoveryContext,
    },
    Declaration {
        keyword: String,
        name: Option<String>,
    },
    Property {
        name: String,
    },
    ChannelMode {
        mode: String,
        role: TolerantChannelModeRole,
        expression_span: Option<SourceSpan>,
        configuration_span: Option<SourceSpan>,
    },
    SqlIsland {
        site: SqlIslandSite,
        fingerprint: String,
    },
    Token,
    MissingToken {
        expected: char,
    },
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TolerantChannelModeRole {
    Head,
    WhenBranch,
    OtherwiseBranch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TolerantRecoveryContext {
    Version,
    Import,
    Export,
    ModuleItemHeader,
    QualifiedKind,
    ModuleItemBody,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TolerantModuleSyntax {
    pub version: Option<TolerantVersionSyntax>,
    pub imports: Vec<TolerantImportSyntax>,
    pub items: Vec<TolerantModuleItemSyntax>,
    pub recovery: Vec<TolerantRecoverySyntax>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TolerantVersionSyntax {
    pub span: SourceSpan,
    pub version: Option<u32>,
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TolerantSpannedText {
    pub span: SourceSpan,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TolerantImportClauseSyntax {
    Named {
        span: SourceSpan,
        specifiers: Vec<TolerantImportSpecifierSyntax>,
        complete: bool,
    },
    Namespace {
        span: SourceSpan,
        local: Option<TolerantSpannedText>,
        complete: bool,
    },
    Missing {
        span: SourceSpan,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TolerantImportSpecifierSyntax {
    pub span: SourceSpan,
    pub imported: Option<TolerantSpannedText>,
    pub local: Option<TolerantSpannedText>,
    pub alias_keyword: Option<SourceSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TolerantImportSyntax {
    pub span: SourceSpan,
    pub clause: TolerantImportClauseSyntax,
    pub source: Option<TolerantSpannedText>,
    pub sha256: Option<TolerantSpannedText>,
    pub complete: bool,
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TolerantModuleItemSyntax {
    pub span: SourceSpan,
    pub declaration_span: SourceSpan,
    pub export_keyword: Option<SourceSpan>,
    pub exported: bool,
    pub keyword: Option<TolerantSpannedText>,
    pub kind_segments: Vec<TolerantSpannedText>,
    pub name: Option<TolerantSpannedText>,
    pub chart_name: Option<TolerantSpannedText>,
    pub body_span: Option<SourceSpan>,
    pub complete: bool,
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TolerantRecoverySyntax {
    pub span: SourceSpan,
    pub context: TolerantRecoveryContext,
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
    pub module_syntax: TolerantModuleSyntax,
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
        ParseMode::Strict => parse_file(source)
            .map(Box::new)
            .map(ParseModeOutput::Strict),
        ParseMode::Tolerant => Ok(ParseModeOutput::Tolerant(Box::new(parse_file_tolerant(
            source,
        )))),
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
    let module_syntax = scan_module_syntax(source, &tokens);
    let (nodes, diagnostics) = {
        let mut builder =
            TolerantTreeBuilder::new(source, &tokens, &module_syntax, diagnostics, limits);
        builder.build();
        (builder.nodes, builder.diagnostics)
    };
    TolerantParsedFile {
        strict,
        tokens,
        module_syntax,
        nodes,
        diagnostics,
    }
}

fn scan_module_syntax(source: &SourceFile, tokens: &LosslessTokenStream) -> TolerantModuleSyntax {
    ModuleSyntaxScanner::new(source, tokens).scan()
}

struct ModuleSyntaxScanner<'a> {
    source: &'a SourceFile,
    tokens: &'a LosslessTokenStream,
    significant: Vec<usize>,
    module: TolerantModuleSyntax,
}

impl<'a> ModuleSyntaxScanner<'a> {
    fn new(source: &'a SourceFile, tokens: &'a LosslessTokenStream) -> Self {
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
            module: TolerantModuleSyntax::default(),
        }
    }

    fn scan(mut self) -> TolerantModuleSyntax {
        let mut position = self.scan_version();
        while position < self.significant.len() {
            if self.word_at(position) == Some("import") {
                position = self.scan_import(position);
            } else if self.apparent_module_start(position) {
                position = self.scan_item(position);
            } else {
                let span = self.token(position).span();
                self.module.recovery.push(TolerantRecoverySyntax {
                    span,
                    context: if self.word_at(position) == Some("export") {
                        TolerantRecoveryContext::Export
                    } else {
                        TolerantRecoveryContext::ModuleItemHeader
                    },
                });
                position += 1;
            }
        }
        self.module
    }

    fn scan_version(&mut self) -> usize {
        if self.significant.is_empty() {
            self.module.recovery.push(TolerantRecoverySyntax {
                span: SourceSpan::empty(self.source.id, 0),
                context: TolerantRecoveryContext::Version,
            });
            return 0;
        }
        let start = self.token(0).span().range.start;
        let mut end_position = 0;
        while end_position + 1 < self.significant.len()
            && !matches!(self.token(end_position).token(), Some(Token::SemiColon))
            && !self.starts_line(self.token(end_position + 1).span().range.start)
        {
            end_position += 1;
        }
        let complete = self.word_at(0) == Some("avenger")
            && matches!(
                self.token_opt(1).and_then(|token| token.token()),
                Some(Token::Number(_, false))
            )
            && matches!(
                self.token_opt(2).and_then(|token| token.token()),
                Some(Token::SemiColon)
            );
        if complete {
            end_position = 2;
        }
        let end = self.token(end_position).span().range.end;
        let version = match self.token_opt(1).and_then(|token| token.token()) {
            Some(Token::Number(value, false)) => value.parse().ok(),
            _ => None,
        };
        let span = self.span(start, end);
        self.module.version = Some(TolerantVersionSyntax {
            span,
            version,
            complete,
        });
        if !complete {
            self.module.recovery.push(TolerantRecoverySyntax {
                span,
                context: TolerantRecoveryContext::Version,
            });
        }
        (end_position + 1).min(self.significant.len())
    }

    fn scan_import(&mut self, start: usize) -> usize {
        let mut end_position = start;
        while end_position + 1 < self.significant.len() {
            if matches!(self.token(end_position).token(), Some(Token::SemiColon)) {
                break;
            }
            if end_position > start
                && self.starts_column_zero(self.token(end_position + 1).span().range.start)
                && self.apparent_module_start(end_position + 1)
            {
                break;
            }
            end_position += 1;
        }
        let complete = matches!(self.token(end_position).token(), Some(Token::SemiColon));
        let exclusive_end = end_position + 1;
        let end = self.token(end_position).span().range.end;
        let span = self.span(self.token(start).span().range.start, end);

        let clause_start = start + 1;
        let clause = if matches!(
            self.token_opt(clause_start).and_then(|token| token.token()),
            Some(Token::LBrace)
        ) {
            self.scan_named_import_clause(clause_start, end_position)
        } else if matches!(
            self.token_opt(clause_start).and_then(|token| token.token()),
            Some(Token::Mul)
        ) {
            self.scan_namespace_import_clause(clause_start, end_position)
        } else {
            TolerantImportClauseSyntax::Missing {
                span: self.token_opt(clause_start).map_or_else(
                    || SourceSpan::empty(self.source.id, end),
                    |token| token.span(),
                ),
            }
        };

        let from =
            (clause_start..=end_position).find(|position| self.word_at(*position) == Some("from"));
        let source_value = from
            .and_then(|position| self.token_opt(position + 1))
            .and_then(|token| string_token_value(token.token()).map(|text| (token.span(), text)))
            .map(|(span, text)| TolerantSpannedText { span, text });
        let sha_position = from.and_then(|from| {
            (from + 1..=end_position).find(|position| self.word_at(*position) == Some("sha256"))
        });
        let sha256 = sha_position
            .and_then(|position| self.token_opt(position + 1))
            .and_then(|token| string_token_value(token.token()).map(|text| (token.span(), text)))
            .map(|(span, text)| TolerantSpannedText { span, text });
        let structurally_complete = complete
            && source_value.is_some()
            && !matches!(clause, TolerantImportClauseSyntax::Missing { .. })
            && match &clause {
                TolerantImportClauseSyntax::Named { complete, .. }
                | TolerantImportClauseSyntax::Namespace { complete, .. } => *complete,
                TolerantImportClauseSyntax::Missing { .. } => false,
            };
        self.module.imports.push(TolerantImportSyntax {
            span,
            clause,
            source: source_value,
            sha256,
            complete: structurally_complete,
            fingerprint: syntax_fingerprint(&self.source.text()[span.range.as_range()]),
        });
        if !structurally_complete {
            self.module.recovery.push(TolerantRecoverySyntax {
                span,
                context: TolerantRecoveryContext::Import,
            });
        }
        exclusive_end
    }

    fn scan_named_import_clause(&self, start: usize, end: usize) -> TolerantImportClauseSyntax {
        let close = (start + 1..=end)
            .find(|position| matches!(self.token(*position).token(), Some(Token::RBrace)));
        let clause_end = close.unwrap_or_else(|| {
            (start + 1..=end)
                .find(|position| self.word_at(*position) == Some("from"))
                .unwrap_or(end)
                .saturating_sub(1)
        });
        let mut specifiers = Vec::new();
        let mut position = start + 1;
        while position <= clause_end {
            if matches!(self.token(position).token(), Some(Token::Comma)) {
                position += 1;
                continue;
            }
            let specifier_start = self.token(position).span().range.start;
            let imported = self.word_at(position).map(|text| TolerantSpannedText {
                span: self.token(position).span(),
                text: text.to_owned(),
            });
            if imported.is_some() {
                position += 1;
            }
            let (alias_keyword, local) = if self.word_at(position) == Some("as") {
                let alias = Some(self.token(position).span());
                position += 1;
                let local = self.word_at(position).map(|text| TolerantSpannedText {
                    span: self.token(position).span(),
                    text: text.to_owned(),
                });
                if local.is_some() {
                    position += 1;
                }
                (alias, local)
            } else {
                (None, imported.clone())
            };
            let specifier_end = self
                .token_opt(position.saturating_sub(1))
                .map_or(specifier_start, |token| token.span().range.end);
            specifiers.push(TolerantImportSpecifierSyntax {
                span: self.span(specifier_start, specifier_end),
                imported,
                local,
                alias_keyword,
            });
            while position <= clause_end
                && !matches!(self.token(position).token(), Some(Token::Comma))
            {
                position += 1;
            }
        }
        let span_end = close
            .map(|position| self.token(position).span().range.end)
            .unwrap_or_else(|| self.token(clause_end).span().range.end);
        TolerantImportClauseSyntax::Named {
            span: self.span(self.token(start).span().range.start, span_end),
            complete: close.is_some()
                && !specifiers.is_empty()
                && specifiers
                    .iter()
                    .all(|specifier| specifier.imported.is_some() && specifier.local.is_some()),
            specifiers,
        }
    }

    fn scan_namespace_import_clause(&self, start: usize, end: usize) -> TolerantImportClauseSyntax {
        let alias_position =
            (start + 1..=end).find(|position| self.word_at(*position) == Some("as"));
        let local = alias_position
            .and_then(|position| self.token_opt(position + 1))
            .and_then(|token| match token.token() {
                Some(Token::Word(word)) => Some(TolerantSpannedText {
                    span: token.span(),
                    text: word.value.clone(),
                }),
                _ => None,
            });
        let span_end = local
            .as_ref()
            .map_or(self.token(start).span().range.end, |local| {
                local.span.range.end
            });
        TolerantImportClauseSyntax::Namespace {
            span: self.span(self.token(start).span().range.start, span_end),
            complete: alias_position.is_some() && local.is_some(),
            local,
        }
    }

    fn scan_item(&mut self, start: usize) -> usize {
        let exported = self.word_at(start) == Some("export");
        let declaration_start = start + usize::from(exported);
        let declaration_start_offset = self
            .token_opt(declaration_start)
            .map_or(self.token(start).span().range.start, |token| {
                token.span().range.start
            });
        let keyword = self
            .token_opt(declaration_start)
            .and_then(|token| match token.token() {
                Some(Token::Word(word)) => Some(TolerantSpannedText {
                    span: token.span(),
                    text: word.value.clone(),
                }),
                _ => None,
            });
        let export_keyword = exported.then(|| self.token(start).span());

        let mut header_end = declaration_start;
        while header_end + 1 < self.significant.len() {
            if matches!(
                self.token(header_end).token(),
                Some(Token::LBrace | Token::SemiColon)
            ) {
                break;
            }
            if header_end > declaration_start
                && self.starts_column_zero(self.token(header_end + 1).span().range.start)
                && self.apparent_module_start(header_end + 1)
            {
                break;
            }
            header_end += 1;
        }
        let open = (declaration_start..=header_end)
            .find(|position| matches!(self.token(*position).token(), Some(Token::LBrace)));
        let header_stop = open.unwrap_or(header_end);
        let (kind_segments, name) =
            self.item_header_parts(declaration_start, header_stop, keyword.as_ref());
        let chart_name = keyword
            .as_ref()
            .is_some_and(|keyword| keyword.text == "chart")
            .then(|| name.clone())
            .flatten();

        let mut item_end_position = header_end;
        let mut complete = false;
        let mut body_span = None;
        if let Some(open) = open {
            let mut depth = 0usize;
            let mut position = open;
            while position < self.significant.len() {
                if position > open
                    && depth == 1
                    && self.starts_column_zero(self.token(position).span().range.start)
                    && self.apparent_module_start(position)
                {
                    item_end_position = position.saturating_sub(1);
                    break;
                }
                match self.token(position).token() {
                    Some(Token::LBrace) => depth += 1,
                    Some(Token::RBrace) => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            item_end_position = position;
                            complete = true;
                            break;
                        }
                    }
                    _ => {}
                }
                item_end_position = position;
                position += 1;
            }
            body_span = Some(self.span(
                self.token(open).span().range.start,
                self.token(item_end_position).span().range.end,
            ));
        }

        let span_start = self.token(start).span().range.start;
        let span_end = self.token(item_end_position).span().range.end;
        let span = self.span(span_start, span_end);
        let declaration_span = self.span(declaration_start_offset, span_end);
        let valid_keyword = keyword
            .as_ref()
            .is_some_and(|keyword| is_top_level_item_keyword(&keyword.text));
        let header_complete = valid_keyword
            && !kind_segments.is_empty()
            && match keyword.as_ref().map(|keyword| keyword.text.as_str()) {
                Some("chart") => true,
                Some("define" | "catalog" | "schema" | "table") => name.is_some(),
                _ => false,
            };
        self.module.items.push(TolerantModuleItemSyntax {
            span,
            declaration_span,
            export_keyword,
            exported,
            keyword,
            kind_segments,
            name,
            chart_name,
            body_span,
            complete: complete && header_complete,
            fingerprint: syntax_fingerprint(&self.source.text()[span.range.as_range()]),
        });
        if !header_complete {
            self.module.recovery.push(TolerantRecoverySyntax {
                span: self.span(
                    declaration_span.range.start,
                    self.token(header_stop).span().range.end,
                ),
                context: TolerantRecoveryContext::ModuleItemHeader,
            });
        }
        if open.is_some() && !complete {
            self.module.recovery.push(TolerantRecoverySyntax {
                span,
                context: TolerantRecoveryContext::ModuleItemBody,
            });
        }
        if exported && !valid_keyword {
            self.module.recovery.push(TolerantRecoverySyntax {
                span: export_keyword.unwrap(),
                context: TolerantRecoveryContext::Export,
            });
        }

        (item_end_position + 1).max(start + 1)
    }

    fn item_header_parts(
        &mut self,
        declaration_start: usize,
        header_stop: usize,
        keyword: Option<&TolerantSpannedText>,
    ) -> (Vec<TolerantSpannedText>, Option<TolerantSpannedText>) {
        let mut kind_segments = Vec::new();
        let mut position = declaration_start + 1;
        if position > header_stop {
            return (kind_segments, None);
        }
        while position <= header_stop {
            let Some(word) = self.word_at(position) else {
                break;
            };
            if word == "as" {
                break;
            }
            kind_segments.push(TolerantSpannedText {
                span: self.token(position).span(),
                text: word.to_owned(),
            });
            if !matches!(
                self.token_opt(position + 1).and_then(|token| token.token()),
                Some(Token::Period)
            ) {
                break;
            }
            if self
                .word_at(position + 2)
                .is_none_or(|segment| segment == "as")
            {
                let period = self.token(position + 1).span();
                self.module.recovery.push(TolerantRecoverySyntax {
                    span: period,
                    context: TolerantRecoveryContext::QualifiedKind,
                });
                break;
            }
            position += 2;
        }
        let name = match keyword.map(|keyword| keyword.text.as_str()) {
            Some("define") => self
                .word_at(declaration_start + 2)
                .map(|text| TolerantSpannedText {
                    span: self.token(declaration_start + 2).span(),
                    text: text.to_owned(),
                }),
            Some("chart" | "catalog" | "schema" | "table") => {
                let alias = (declaration_start + 1..header_stop)
                    .find(|position| self.word_at(*position) == Some("as"));
                alias.and_then(|position| {
                    self.word_at(position + 1).map(|text| TolerantSpannedText {
                        span: self.token(position + 1).span(),
                        text: text.to_owned(),
                    })
                })
            }
            _ => None,
        };
        (kind_segments, name)
    }

    fn apparent_module_start(&self, position: usize) -> bool {
        match self.word_at(position) {
            Some("export") => self
                .word_at(position + 1)
                .is_some_and(is_top_level_item_keyword),
            Some(keyword) => is_top_level_item_keyword(keyword),
            None => false,
        }
    }

    fn starts_line(&self, offset: usize) -> bool {
        let line_start = self.source.text()[..offset]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        self.source.text()[line_start..offset].trim().is_empty()
    }

    fn starts_column_zero(&self, offset: usize) -> bool {
        offset == 0 || self.source.text().as_bytes().get(offset - 1) == Some(&b'\n')
    }

    fn word_at(&self, position: usize) -> Option<&str> {
        match self.token_opt(position)?.token()? {
            Token::Word(word) => Some(word.value.as_str()),
            _ => None,
        }
    }

    fn token(&self, position: usize) -> &crate::sql::LosslessToken {
        &self.tokens.tokens()[self.significant[position]]
    }

    fn token_opt(&self, position: usize) -> Option<&crate::sql::LosslessToken> {
        self.significant
            .get(position)
            .map(|index| &self.tokens.tokens()[*index])
    }

    fn span(&self, start: usize, end: usize) -> SourceSpan {
        SourceSpan {
            source: self.source.id,
            range: ByteSpan { start, end },
        }
    }
}

fn is_top_level_item_keyword(keyword: &str) -> bool {
    matches!(keyword, "chart" | "define" | "catalog" | "schema" | "table")
}

fn string_token_value(token: Option<&Token>) -> Option<String> {
    match token? {
        Token::SingleQuotedString(value) => Some(value.clone()),
        _ => None,
    }
}

fn syntax_fingerprint(source: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"avenger-tolerant-syntax-v1");
    hash.update((source.len() as u64).to_le_bytes());
    hash.update(source.as_bytes());
    format!("{:x}", hash.finalize())
}

fn sql_island_fingerprint(site: SqlIslandSite, source: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"avenger-tolerant-sql-island-v1");
    hash.update(site.manifest_name().as_bytes());
    hash.update((source.len() as u64).to_le_bytes());
    hash.update(source.as_bytes());
    format!("{:x}", hash.finalize())
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
    module_syntax: &'a TolerantModuleSyntax,
    significant: Vec<usize>,
    nodes: Vec<TolerantSyntaxNode>,
    diagnostics: Vec<Diagnostic>,
    delimiters: Vec<OpenDelimiter>,
    pending_delimiter_owners: BTreeMap<usize, TolerantSyntaxNodeId>,
    module_item_owners: BTreeMap<usize, TolerantSyntaxNodeId>,
    limits: SyntaxLimits,
    declaration_count: usize,
    reported_declaration_limit: bool,
}

impl<'a> TolerantTreeBuilder<'a> {
    fn new(
        source: &'a SourceFile,
        tokens: &'a LosslessTokenStream,
        module_syntax: &'a TolerantModuleSyntax,
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
            module_syntax,
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
            module_item_owners: BTreeMap::new(),
            limits,
            declaration_count: 0,
            reported_declaration_limit: false,
        }
    }

    fn build(&mut self) {
        self.install_module_nodes();
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
            let module_owner = self
                .module_item_owners
                .get(&token.span().range.start)
                .copied();
            if module_owner.is_some() && !self.delimiters.is_empty() {
                self.recover_at_module_boundary(token.span().range.start);
            }
            let parent = module_owner.or_else(|| self.current_owner());
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
                let has_declaration_prefix = self
                    .word_at(position)
                    .is_some_and(|word| matches!(word, "public" | "private"))
                    || (self.word_at(position).is_some_and(|word| word == "export")
                        && self
                            .token_at_significant_opt(position + 1)
                            .is_some_and(|token| {
                                self.module_item_owners
                                    .contains_key(&token.span().range.start)
                                    || self
                                        .word_at(position + 1)
                                        .is_some_and(is_top_level_item_keyword)
                            }));
                let declaration_position = if has_declaration_prefix {
                    position + 1
                } else {
                    position
                };
                let declaration_parent = self
                    .token_at_significant_opt(declaration_position)
                    .and_then(|token| {
                        self.module_item_owners
                            .get(&token.span().range.start)
                            .copied()
                    })
                    .or(parent);
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
                    let declaration_token = self.token_at_significant(declaration_position);
                    let name = recovered_name
                        .or_else(|| self.declaration_name(declaration_position, end_position));
                    let id = self.push_node(
                        declaration_parent,
                        SourceSpan {
                            source: self.source.id,
                            range: ByteSpan {
                                start: declaration_token.span().range.start,
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
                    if matches!(keyword.as_str(), "output" | "param")
                        && let Some((start, end)) =
                            self.output_island_bounds(declaration_position, end_position)
                    {
                        let site = if keyword == "param" {
                            SqlIslandSite::ParamInitializer
                        } else {
                            SqlIslandSite::OutputSource
                        };
                        self.push_sql_island(Some(id), site, start, end);
                    }
                    if keyword == "set"
                        && let Some((start, end)) =
                            self.action_island_bounds(declaration_position, end_position)
                    {
                        let site = if self
                            .word_at(declaration_position + 1)
                            .is_some_and(|target| target.eq_ignore_ascii_case("cursor"))
                        {
                            SqlIslandSite::CursorActionRhs
                        } else {
                            SqlIslandSite::StateActionRhs
                        };
                        self.push_sql_island(Some(id), site, start, end);
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
                let is_structural_array = self
                    .token_at_significant_opt(value_start)
                    .is_some_and(|token| matches!(token.token(), Some(Token::LBracket)));
                if is_structural_array {
                    self.push_array_element_islands(Some(property), value_start, value_end);
                }
                if matches!(name.as_str(), "encoded" | "direct")
                    && let Some(role) = self.channel_branch_role(parent)
                {
                    let expression_span = if value_start < value_end {
                        let first = self.token_at_significant_opt(value_start);
                        let last = self.token_at_significant_opt(value_end.saturating_sub(1));
                        first.zip(last).map(|(first, last)| SourceSpan {
                            source: self.source.id,
                            range: ByteSpan {
                                start: first.span().range.start,
                                end: last.span().range.end,
                            },
                        })
                    } else {
                        None
                    };
                    self.push_node(
                        Some(property),
                        token.span(),
                        TolerantSyntaxNodeKind::ChannelMode {
                            mode: name.clone(),
                            role,
                            expression_span,
                            configuration_span: None,
                        },
                    );
                }
                let body_position = (value_start..=value_end).find(|position| {
                    self.token_at_significant_opt(*position)
                        .is_some_and(|token| matches!(token.token(), Some(Token::LBrace)))
                });
                let channel_mode = self.word_at(value_start).and_then(|word| {
                    (word.eq_ignore_ascii_case("encoded") || word.eq_ignore_ascii_case("direct"))
                        .then(|| {
                            (
                                self.token_at_significant(value_start).span(),
                                word.to_ascii_lowercase(),
                            )
                        })
                });
                let has_channel_mode = channel_mode.is_some();
                if let Some((span, mode)) = channel_mode {
                    let expression_start = value_start + 1;
                    let expression_end = body_position.unwrap_or(value_end);
                    let expression_span = (expression_start < expression_end)
                        .then(|| {
                            let first = self.token_at_significant_opt(expression_start)?;
                            let last = self.token_at_significant_opt(expression_end - 1)?;
                            Some(SourceSpan {
                                source: self.source.id,
                                range: ByteSpan {
                                    start: first.span().range.start,
                                    end: last.span().range.end,
                                },
                            })
                        })
                        .flatten();
                    let configuration_span = body_position.and_then(|start| {
                        let first = self.token_at_significant_opt(start)?;
                        let last = self.token_at_significant_opt(value_end)?;
                        Some(SourceSpan {
                            source: self.source.id,
                            range: ByteSpan {
                                start: first.span().range.start,
                                end: last.span().range.end,
                            },
                        })
                    });
                    self.push_node(
                        Some(property),
                        span,
                        TolerantSyntaxNodeKind::ChannelMode {
                            mode,
                            role: TolerantChannelModeRole::Head,
                            expression_span,
                            configuration_span,
                        },
                    );
                }
                if let Some(body_position) = body_position {
                    self.pending_delimiter_owners
                        .insert(body_position, property);
                }
                let island_start = if self.word_at(value_start).is_some_and(|word| {
                    word.eq_ignore_ascii_case("encoded") || word.eq_ignore_ascii_case("direct")
                }) {
                    value_start + 1
                } else {
                    value_start
                };
                if !is_structural_array
                    && island_start <= value_end
                    && let (Some(first), Some(last)) = (
                        self.token_at_significant_opt(island_start),
                        self.token_at_significant_opt(value_end),
                    )
                {
                    let island_end = if let Some(body_position) = body_position {
                        self.token_at_significant(body_position).span().range.start
                    } else if matches!(last.token(), Some(Token::SemiColon)) {
                        last.span().range.start
                    } else {
                        last.span().range.end
                    };
                    if first.span().range.start <= island_end {
                        let site = if name.eq_ignore_ascii_case("sql")
                            || name.eq_ignore_ascii_case("query")
                        {
                            SqlIslandSite::QueryProperty
                        } else if name.eq_ignore_ascii_case("expressions")
                            || self.tokens_form_projection(island_start, value_end)
                        {
                            SqlIslandSite::ProjectionProperty
                        } else if has_channel_mode {
                            SqlIslandSite::ChannelModePayload
                        } else {
                            SqlIslandSite::PropertyValue
                        };
                        self.push_sql_island(
                            Some(property),
                            site,
                            first.span().range.start,
                            island_end,
                        );
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

    fn channel_branch_role(
        &self,
        parent: Option<TolerantSyntaxNodeId>,
    ) -> Option<TolerantChannelModeRole> {
        let parent = self.nodes.iter().find(|node| Some(node.id) == parent)?;
        match &parent.kind {
            TolerantSyntaxNodeKind::Declaration { keyword, .. } if keyword == "when" => {
                Some(TolerantChannelModeRole::WhenBranch)
            }
            TolerantSyntaxNodeKind::Property { name } if name == "otherwise" => {
                Some(TolerantChannelModeRole::OtherwiseBranch)
            }
            _ => None,
        }
    }

    fn install_module_nodes(&mut self) {
        let module = self.module_syntax.clone();
        if let Some(version) = module.version {
            self.push_node(
                Some(TolerantSyntaxNodeId(0)),
                version.span,
                TolerantSyntaxNodeKind::VersionPragma {
                    version: version.version,
                },
            );
        }
        for import in module.imports {
            let import_id = self.push_node(
                Some(TolerantSyntaxNodeId(0)),
                import.span,
                TolerantSyntaxNodeKind::Import,
            );
            if let TolerantImportClauseSyntax::Named { specifiers, .. } = import.clause {
                for specifier in specifiers {
                    self.push_node(
                        Some(import_id),
                        specifier.span,
                        TolerantSyntaxNodeKind::ImportSpecifier {
                            imported: specifier.imported.map(|name| name.text),
                            local: specifier.local.map(|name| name.text),
                        },
                    );
                }
            }
        }
        for item in module.items {
            let keyword = item.keyword.as_ref().map(|keyword| keyword.text.clone());
            let name = item.name.as_ref().map(|name| name.text.clone());
            let chart_name = item.chart_name.as_ref().map(|name| name.text.clone());
            let owner = self.push_node(
                Some(TolerantSyntaxNodeId(0)),
                item.span,
                TolerantSyntaxNodeKind::ModuleItem {
                    exported: item.exported,
                    keyword,
                    kind: item
                        .kind_segments
                        .iter()
                        .map(|segment| segment.text.clone())
                        .collect(),
                    name,
                    chart_name,
                },
            );
            self.module_item_owners
                .insert(item.declaration_span.range.start, owner);
            self.module_item_owners.insert(item.span.range.start, owner);
        }
        for recovery in module.recovery {
            self.push_node(
                Some(TolerantSyntaxNodeId(0)),
                recovery.span,
                TolerantSyntaxNodeKind::Recovery {
                    context: recovery.context,
                },
            );
        }
    }

    fn recover_at_module_boundary(&mut self, offset: usize) {
        for open in self.delimiters.drain(..).rev() {
            self.nodes.push(TolerantSyntaxNode {
                id: TolerantSyntaxNodeId(self.nodes.len() as u32),
                parent: open.owner,
                span: SourceSpan::empty(self.source.id, offset),
                kind: TolerantSyntaxNodeKind::MissingToken {
                    expected: matching_close(open.delimiter),
                },
            });
            self.diagnostics.push(Diagnostic::error(
                "AVENGER-PARSE-RECOVER-006",
                format!(
                    "missing `{}` before the next module item",
                    matching_close(open.delimiter)
                ),
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

    fn push_sql_island(
        &mut self,
        parent: Option<TolerantSyntaxNodeId>,
        site: SqlIslandSite,
        start: usize,
        end: usize,
    ) -> TolerantSyntaxNodeId {
        let span = SourceSpan {
            source: self.source.id,
            range: ByteSpan { start, end },
        };
        let text = self.source.text().get(start..end).unwrap_or_default();
        self.push_node(
            parent,
            span,
            TolerantSyntaxNodeKind::SqlIsland {
                site,
                fingerprint: sql_island_fingerprint(site, text),
            },
        )
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
        let mut depth = 0usize;
        for position in start..self.significant.len() {
            match self.token_at_significant(position).token() {
                Some(Token::LParen | Token::LBracket) => depth += 1,
                Some(Token::RParen | Token::RBracket) => depth = depth.saturating_sub(1),
                Some(Token::LBrace | Token::SemiColon) if depth == 0 => return position,
                _ => {}
            }
        }
        self.significant.len() - 1
    }

    fn declaration_name(&self, start: usize, end: usize) -> Option<String> {
        let mut depth = 0usize;
        for position in start..end {
            match self.token_at_significant(position).token() {
                Some(Token::LParen | Token::LBracket | Token::LBrace) => depth += 1,
                Some(Token::RParen | Token::RBracket | Token::RBrace) => {
                    depth = depth.saturating_sub(1)
                }
                Some(Token::Word(word)) if depth == 0 && word.value.eq_ignore_ascii_case("as") => {
                    return self.word_at(position + 1).map(str::to_owned);
                }
                _ => {}
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
        Some(keyword.to_owned())
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
        let last = self.token_at_significant_opt(end)?;
        let mut island_end = if matches!(last.token(), Some(Token::SemiColon))
            || matches!(last.token(), Some(Token::Word(word)) if word.value.eq_ignore_ascii_case("as"))
        {
            last.span().range.start
        } else {
            // Tolerant lexing can collapse an unterminated string through the
            // rest of an aliased declaration into one error token. In that
            // case the token itself is the editable SQL island; there is no
            // independently tokenized outer `as` to exclude.
            last.span().range.end
        };
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
        (first.span().range.start <= island_end).then_some((first.span().range.start, island_end))
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
        // `;` is owned by the action grammar even when the expression has an
        // unmatched inner delimiter. It cannot be a legal token inside an
        // Avenger scalar expression, so lexical recovery must not let the SQL
        // island consume the rest of the event block.
        let island_end = (value_start..=end)
            .find_map(|position| {
                let token = self.token_at_significant_opt(position)?;
                matches!(token.token(), Some(Token::SemiColon)).then_some(token.span().range.start)
            })
            .unwrap_or_else(|| self.token_at_significant(end).span().range.end);
        (first.span().range.start <= island_end).then_some((first.span().range.start, island_end))
    }

    fn push_array_element_islands(
        &mut self,
        parent: Option<TolerantSyntaxNodeId>,
        array_start: usize,
        property_end: usize,
    ) {
        let mut depth = 0usize;
        let mut element_start = array_start + 1;
        let mut closed_array = false;
        let mut last_close_candidate = None;
        let scan_end = property_end.min(self.significant.len().saturating_sub(1));
        for position in array_start + 1..=scan_end {
            let (boundary, closes_array, boundary_start) = {
                let token = self.token_at_significant(position);
                if matches!(token.token(), Some(Token::RBracket)) {
                    last_close_candidate = Some(position);
                }
                let boundary = match token.token() {
                    Some(Token::LParen | Token::LBracket | Token::LBrace) => {
                        depth += 1;
                        false
                    }
                    Some(Token::RParen | Token::RBrace) if depth > 0 => {
                        depth -= 1;
                        false
                    }
                    Some(Token::RBracket) if depth > 0 => {
                        depth -= 1;
                        false
                    }
                    Some(Token::Comma) if depth == 0 => true,
                    Some(Token::RBracket) if depth == 0 => true,
                    _ => false,
                };
                (
                    boundary,
                    matches!(token.token(), Some(Token::RBracket)) && depth == 0,
                    token.span().range.start,
                )
            };
            if !boundary {
                continue;
            }

            if element_start == position {
                self.push_sql_island(
                    parent,
                    SqlIslandSite::ArrayElement,
                    boundary_start,
                    boundary_start,
                );
            } else if element_start < position && !self.array_element_is_structural(element_start) {
                let start = self.token_at_significant(element_start).span().range.start;
                let end = boundary_start;
                if start < end {
                    self.push_sql_island(parent, SqlIslandSite::ArrayElement, start, end);
                }
            }
            element_start = position + 1;
            if closes_array {
                closed_array = true;
                break;
            }
        }
        if !closed_array
            && let Some(close) = last_close_candidate
            && element_start < close
            && !self.array_element_is_structural(element_start)
        {
            let start = self.token_at_significant(element_start).span().range.start;
            let end = self.token_at_significant(close).span().range.start;
            if start < end {
                self.push_sql_island(parent, SqlIslandSite::ArrayElement, start, end);
            }
        } else if !closed_array
            && last_close_candidate.is_none()
            && element_start <= scan_end
            && !self.array_element_is_structural(element_start)
        {
            // An unterminated lexical form (notably a dollar-quoted string)
            // can make sqlparser's tokenizer retain the remainder of the file
            // as one error token. Preserve that residual range as the active
            // array-element island so completion remains available at the
            // cursor even though the outer `]` is no longer tokenized.
            let start = self.token_at_significant(element_start).span().range.start;
            let end = self.token_at_significant(scan_end).span().range.end;
            if start < end {
                self.push_sql_island(parent, SqlIslandSite::ArrayElement, start, end);
            }
        }
    }

    fn array_element_is_structural(&self, start: usize) -> bool {
        matches!(
            self.token_at_significant_opt(start)
                .and_then(|token| token.token()),
            Some(Token::LBrace)
        ) || self
            .word_at(start)
            .is_some_and(|word| word.eq_ignore_ascii_case("none"))
            || (self
                .word_at(start)
                .is_some_and(|word| word.eq_ignore_ascii_case("pattern"))
                && self
                    .token_at_significant_opt(start + 1)
                    .is_some_and(|token| matches!(token.token(), Some(Token::LBrace))))
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

    fn tokens_form_projection(&self, start: usize, end: usize) -> bool {
        let mut depth = 0usize;
        for position in start..=end {
            let Some(token) = self.token_at_significant_opt(position) else {
                continue;
            };
            match token.token() {
                Some(Token::LParen | Token::LBracket | Token::LBrace) => depth += 1,
                Some(Token::RParen | Token::RBracket | Token::RBrace) => {
                    depth = depth.saturating_sub(1);
                }
                Some(Token::Comma) if depth == 0 => return true,
                Some(Token::Word(word)) if depth == 0 && word.value.eq_ignore_ascii_case("as") => {
                    return true;
                }
                _ => {}
            }
        }
        false
    }
}

/// Canonical source starters. Internal semantic declaration keywords such as
/// `dimension` and `channel` do not enter the tolerant source grammar through
/// this list.
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
    "selection",
    "set",
    "slot",
    "splice",
    "store",
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
    use std::collections::BTreeSet;

    use crate::{SourceFile, SourceId, SourceOrigin};

    use super::{
        TolerantImportClauseSyntax, TolerantRecoveryContext, TolerantSyntaxNodeKind,
        parse_file_tolerant,
    };

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
            "avenger 1; chart cartesian as chart { public param 1.0 as width; mark symbol as points {} }",
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
              store as rows {}
              selection as picked {}
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
                    site: super::SqlIslandSite::OutputSource,
                    ..
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
            .filter_map(|node| match &node.kind {
                TolerantSyntaxNodeKind::SqlIsland {
                    site: super::SqlIslandSite::StateActionRhs,
                    ..
                } => Some(&source.text()[node.span.range.as_range()]),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(islands, ["$width + 1"]);
    }

    #[test]
    fn channel_mode_is_outside_the_tolerant_sql_island() {
        let source = SourceFile::new(
            SourceId::new(1),
            SourceOrigin::Memory("channel-mode.avenger".into()),
            "avenger 1; chart cartesian { mark symbol { x: encoded \"amount\" + 1; } }",
        );
        let parsed = parse_file_tolerant(&source);
        let island = parsed
            .nodes
            .iter()
            .find(|node| {
                matches!(
                    node.kind,
                    TolerantSyntaxNodeKind::SqlIsland {
                        site: super::SqlIslandSite::ChannelModePayload,
                        ..
                    }
                )
            })
            .expect("channel expression island");
        assert_eq!(
            &source.text()[island.span.range.as_range()],
            "\"amount\" + 1"
        );
    }

    #[test]
    fn sql_islands_retain_exact_sites_and_array_element_spans() {
        let source = SourceFile::new(
            SourceId::new(7),
            SourceOrigin::Memory("sql-sites.avenger".into()),
            r#"avenger 1;
define transform sample {
  param 1 + 2 as offset;
  output $offset + 1 as adjusted;
  expressions: "value" AS copied;
}
chart cartesian {
  sql: SELECT 1;
  values: [1 + 2, none, { enabled: true; }];
  mark symbol {
    x: encoded "value";
    opacity: $offset;
  }
  on click {
    set cursor = 'crosshair';
    set offset = $offset + 1;
  }
}"#,
        );
        let parsed = parse_file_tolerant(&source);
        let islands = parsed
            .nodes
            .iter()
            .filter_map(|node| match &node.kind {
                TolerantSyntaxNodeKind::SqlIsland { site, fingerprint } => Some((
                    *site,
                    fingerprint,
                    &source.text()[node.span.range.as_range()],
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        let sites = islands
            .iter()
            .map(|(site, _, _)| *site)
            .collect::<BTreeSet<_>>();
        assert_eq!(sites, super::SqlIslandSite::ALL.into_iter().collect());
        assert!(
            islands
                .iter()
                .all(|(_, fingerprint, _)| fingerprint.len() == 64)
        );
        assert!(islands.iter().any(|(site, _, text)| {
            *site == super::SqlIslandSite::ArrayElement && *text == "1 + 2"
        }));
        assert!(!islands.iter().any(|(_, _, text)| text.contains("none")));
    }

    #[test]
    fn empty_editing_positions_retain_every_exact_sql_site() {
        let source = SourceFile::new(
            SourceId::new(8),
            SourceOrigin::Memory("empty-sql-sites.avenger".into()),
            r#"avenger 1;
define transform sample {
  param  as offset;
  output  as adjusted;
  expressions: ;
}
chart cartesian {
  sql: ;
  values: [,];
  mark symbol {
    x: encoded ;
    opacity: ;
  }
  on click {
    set cursor = ;
    set offset = ;
  }
}"#,
        );
        let parsed = parse_file_tolerant(&source);
        let mut sites = BTreeSet::new();
        for node in &parsed.nodes {
            let TolerantSyntaxNodeKind::SqlIsland { site, fingerprint } = &node.kind else {
                continue;
            };
            sites.insert(*site);
            assert!(node.span.range.is_empty(), "{site:?}: {:?}", node.span);
            assert_eq!(fingerprint.len(), 64);
        }
        assert_eq!(sites, super::SqlIslandSite::ALL.into_iter().collect());
    }

    #[test]
    fn malformed_lexical_and_delimiter_states_retain_the_editable_sql_range() {
        for (site, text, cursor_needle, expected_start) in [
            (
                super::SqlIslandSite::ParamInitializer,
                "avenger 1; chart cartesian { param $$raw as width; }",
                "$$raw",
                "$$raw",
            ),
            (
                super::SqlIslandSite::ArrayElement,
                "avenger 1; chart cartesian { table inline { values: [$$raw]; } }",
                "$$raw",
                "$$raw",
            ),
            (
                super::SqlIslandSite::StateActionRhs,
                "avenger 1; chart cartesian { on click { set width = coalesce(1,; } }",
                "coalesce(1,",
                "coalesce(1,",
            ),
        ] {
            let source = SourceFile::new(
                SourceId::new(9),
                SourceOrigin::Memory(format!("malformed-{site:?}.avenger")),
                text,
            );
            let cursor = text.find(cursor_needle).unwrap() + cursor_needle.len();
            let parsed = parse_file_tolerant(&source);
            let island = parsed
                .nodes
                .iter()
                .find(|node| {
                    matches!(
                        node.kind,
                        TolerantSyntaxNodeKind::SqlIsland { site: actual, .. }
                            if actual == site
                    ) && node.span.range.start <= cursor
                        && cursor <= node.span.range.end
                })
                .unwrap_or_else(|| panic!("missing {site:?} island at {cursor}: {parsed:#?}"));
            assert!(
                source.text()[island.span.range.as_range()].starts_with(expected_start),
                "{site:?}: {:?}",
                &source.text()[island.span.range.as_range()]
            );
            if site == super::SqlIslandSite::StateActionRhs {
                assert_eq!(&source.text()[island.span.range.as_range()], "coalesce(1,");
            }
        }
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

    #[test]
    fn module_syntax_retains_partial_import_names_and_source_spans() {
        let source = SourceFile::new(
            SourceId::new(4),
            SourceOrigin::Memory("partial-import".into()),
            "avenger 1;\nimport { chart as local, points as } from './library.avenger'\nchart cartesian as visible {}\n",
        );
        let parsed = parse_file_tolerant(&source);
        assert!(parsed.strict.is_none());
        let [import] = parsed.module_syntax.imports.as_slice() else {
            panic!("one import")
        };
        let TolerantImportClauseSyntax::Named { specifiers, .. } = &import.clause else {
            panic!("named import")
        };
        assert_eq!(specifiers[0].imported.as_ref().unwrap().text, "chart");
        assert_eq!(specifiers[0].local.as_ref().unwrap().text, "local");
        assert_eq!(specifiers[1].imported.as_ref().unwrap().text, "points");
        assert!(specifiers[1].local.is_none());
        assert_eq!(import.source.as_ref().unwrap().text, "./library.avenger");
        assert!(!import.complete);
        assert_eq!(parsed.module_syntax.items.len(), 1);
        assert_eq!(
            parsed.module_syntax.items[0]
                .chart_name
                .as_ref()
                .unwrap()
                .text,
            "visible"
        );
    }

    #[test]
    fn malformed_item_recovers_before_later_siblings() {
        let source = SourceFile::new(
            SourceId::new(5),
            SourceOrigin::Memory("item-recovery".into()),
            r#"avenger 1;
export chart acme. as broken {
  mark symbol {}
chart polar as second {}
export define mark badge { mark symbol {} }
"#,
        );
        let parsed = parse_file_tolerant(&source);
        assert!(parsed.strict.is_none());
        assert_eq!(parsed.module_syntax.items.len(), 3);
        assert!(!parsed.module_syntax.items[0].complete);
        assert_eq!(
            parsed.module_syntax.items[1]
                .chart_name
                .as_ref()
                .unwrap()
                .text,
            "second"
        );
        assert!(parsed.module_syntax.items[1].complete);
        assert!(parsed.module_syntax.items[2].complete);
        let module_nodes = parsed
            .nodes
            .iter()
            .filter(|node| matches!(node.kind, TolerantSyntaxNodeKind::ModuleItem { .. }))
            .count();
        assert_eq!(module_nodes, 3);
        assert!(
            parsed
                .module_syntax
                .recovery
                .iter()
                .any(|recovery| { recovery.context == TolerantRecoveryContext::QualifiedKind })
        );
    }

    #[test]
    fn sibling_fingerprints_survive_an_unrelated_item_edit() {
        let before = parse_file_tolerant(&SourceFile::new(
            SourceId::new(6),
            SourceOrigin::Memory("fingerprints".into()),
            "avenger 1;\nchart cartesian as first { width: 1; }\nchart polar as second {}\n",
        ));
        let after = parse_file_tolerant(&SourceFile::new(
            SourceId::new(6),
            SourceOrigin::Memory("fingerprints".into()),
            "avenger 1;\nchart cartesian as first { width: 200; }\nchart polar as second {}\n",
        ));
        assert_ne!(
            before.module_syntax.items[0].fingerprint,
            after.module_syntax.items[0].fingerprint
        );
        assert_eq!(
            before.module_syntax.items[1].fingerprint,
            after.module_syntax.items[1].fingerprint
        );
    }
}
