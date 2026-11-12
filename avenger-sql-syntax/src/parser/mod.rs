//! The parser: event-based recursive descent over the tokens, with precedence climbing for
//! expressions ([`expr`]). Scripts and their statements are in [`script`], and queries, sources
//! and pipe operators in [`query`].
//!
//! A grammar function opens a [`Marker`], consumes tokens, and completes the marker as a node of
//! a kind. [`CompletedMarker::precede`] wraps a completed node in a new one, which builds binary
//! expressions and postfix forms without left recursion. The events build the tree in
//! [`build`]. The parser never stops at an error: it reports a diagnostic, skips what it cannot
//! read into an `ERROR` node, and goes on, so every text has a tree.

mod build;
mod expr;
mod query;
mod script;

use std::cell::RefCell;

use rowan::{GreenNode, TextRange, TextSize};

use crate::{
    diagnostic::{Diagnostic, DiagnosticCode},
    lexer::{tokenize, Token},
    Expected, NameRole,
    SyntaxKind::{self, *},
};

/// The deepest nesting of expressions and queries that the parser reads.
const MAX_DEPTH: u32 = 256;

/// Whether `word` is a reserved keyword, which is a name only in backticks.
pub(crate) fn is_reserved(word: &str) -> bool {
    SyntaxKind::keyword(word).is_some_and(SyntaxKind::is_reserved)
}

/// What the parser reads: a script or a module.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    Script,
    Module,
}

/// The tree of `source`, the diagnostics of its tokens and its grammar, and, with a cursor, what
/// the parser expected at the cursor.
pub(crate) fn parse(
    source: &str,
    entry: Entry,
    cursor: Option<TextSize>,
) -> (GreenNode, Vec<Diagnostic>, Vec<Expected>) {
    let (tokens, mut diagnostics) = tokenize(source);
    let mut parser = Parser::new(source, &tokens, cursor);
    match entry {
        Entry::Script => parser.script(),
        Entry::Module => parser.module(),
    }
    let Parser {
        events,
        diagnostics: grammar,
        expected,
        ..
    } = parser;
    diagnostics.extend(grammar);
    diagnostics.sort_by_key(|diagnostic| diagnostic.range.start());
    let green = build::build(source, &tokens, events);
    (green, diagnostics, expected.into_inner())
}

#[derive(Clone, Debug)]
pub(crate) enum Event {
    /// The start of a node. A node that `precede` wrapped in a parent names the parent's start
    /// event by its distance.
    Start {
        kind: SyntaxKind,
        forward_parent: Option<u32>,
    },
    Finish,
    /// The next token that is not trivia, with the kind that the tree gives it.
    Token {
        kind: SyntaxKind,
    },
    /// A start that the parser abandoned.
    Tombstone,
}

pub(crate) struct Parser<'a> {
    source: &'a str,
    tokens: &'a [Token],
    /// The indices of the tokens that are not trivia.
    significant: Vec<usize>,
    position: usize,
    events: Vec<Event>,
    diagnostics: Vec<Diagnostic>,
    /// The depth of the parentheses that the parser is inside, where `)` ends a skip.
    paren_depth: u32,
    depth: u32,
    in_module: bool,
    cursor: Option<TextSize>,
    expected: RefCell<Vec<Expected>>,
}

/// The start of a node, which the parser completes or abandons.
#[must_use]
pub(crate) struct Marker {
    event: u32,
    position: usize,
}

/// A completed node: its start event, its kind, and the significant tokens it covers.
#[derive(Clone, Copy)]
pub(crate) struct CompletedMarker {
    event: u32,
    kind: SyntaxKind,
    start: usize,
    end: usize,
}

/// The state of the parser, to which it can rewind.
pub(crate) struct Checkpoint {
    events: usize,
    position: usize,
    diagnostics: usize,
    paren_depth: u32,
    depth: u32,
}

impl Marker {
    pub(crate) fn complete(self, p: &mut Parser, kind: SyntaxKind) -> CompletedMarker {
        match &mut p.events[self.event as usize] {
            Event::Start { kind: slot, .. } => *slot = kind,
            _ => unreachable!("a marker points at a start event"),
        }
        p.events.push(Event::Finish);
        CompletedMarker {
            event: self.event,
            kind,
            start: self.position,
            end: p.position,
        }
    }

    pub(crate) fn abandon(self, p: &mut Parser) {
        let index = self.event as usize;
        if index == p.events.len() - 1 {
            p.events.pop();
        } else {
            p.events[index] = Event::Tombstone;
        }
    }
}

impl CompletedMarker {
    pub(crate) fn kind(&self) -> SyntaxKind {
        self.kind
    }

    /// A new marker that starts before this node, so the node becomes its child.
    pub(crate) fn precede(self, p: &mut Parser) -> Marker {
        let marker = p.start();
        match &mut p.events[self.event as usize] {
            Event::Start { forward_parent, .. } => {
                *forward_parent = Some(marker.event - self.event);
            }
            _ => unreachable!("a completed marker points at a start event"),
        }
        Marker {
            event: marker.event,
            position: self.start,
        }
    }
}

impl<'a> Parser<'a> {
    fn new(source: &'a str, tokens: &'a [Token], cursor: Option<TextSize>) -> Self {
        let significant = (tokens.iter().enumerate())
            .filter(|(_, token)| !token.kind.is_trivia())
            .map(|(index, _)| index)
            .collect();
        Self {
            source,
            tokens,
            significant,
            position: 0,
            events: Vec::new(),
            diagnostics: Vec::new(),
            paren_depth: 0,
            depth: 0,
            in_module: false,
            cursor,
            expected: RefCell::new(Vec::new()),
        }
    }

    // --- Tokens ---

    fn token_at(&self, position: usize) -> Option<&Token> {
        self.significant
            .get(position)
            .map(|&index| &self.tokens[index])
    }

    /// The kind of the significant token `n` places ahead, or [`EOF`].
    pub(crate) fn nth(&self, n: usize) -> SyntaxKind {
        self.token_at(self.position + n)
            .map_or(EOF, |token| token.kind)
    }

    pub(crate) fn current(&self) -> SyntaxKind {
        self.nth(0)
    }

    /// The text of the significant token `n` places ahead, or `""` at the end.
    pub(crate) fn nth_text(&self, n: usize) -> &'a str {
        self.token_at(self.position + n)
            .map_or("", |token| &self.source[token.range])
    }

    /// The range of the significant token `n` places ahead, or the empty range at the end.
    pub(crate) fn nth_range(&self, n: usize) -> TextRange {
        match self.token_at(self.position + n) {
            Some(token) => token.range,
            None => TextRange::empty(TextSize::of(self.source)),
        }
    }

    pub(crate) fn range(&self) -> TextRange {
        self.nth_range(0)
    }

    /// The range of the previous significant token.
    pub(crate) fn previous_range(&self) -> TextRange {
        match self.position {
            0 => TextRange::empty(0.into()),
            position => self.token_at(position - 1).expect("a token").range,
        }
    }

    /// The kind that the lexer gave the previous significant token.
    pub(crate) fn previous_kind(&self) -> Option<SyntaxKind> {
        self.position
            .checked_sub(1)
            .and_then(|position| self.token_at(position))
            .map(|token| token.kind)
    }

    /// The range of the tokens of a completed node.
    pub(crate) fn node_range(&self, node: CompletedMarker) -> TextRange {
        if node.start == node.end {
            return TextRange::empty(self.nth_range(0).start());
        }
        let first = self.token_at(node.start).expect("a token").range;
        let last = self.token_at(node.end - 1).expect("a token").range;
        first.cover(last)
    }

    /// Whether the significant token `n` places ahead touches the next one, with no trivia
    /// between.
    pub(crate) fn adjacent(&self, n: usize) -> bool {
        match (
            self.token_at(self.position + n),
            self.token_at(self.position + n + 1),
        ) {
            (Some(a), Some(b)) => a.range.end() == b.range.start(),
            _ => false,
        }
    }

    /// Whether the previous significant token touches the current one.
    pub(crate) fn touches_previous(&self) -> bool {
        self.position > 0
            && self.token_at(self.position).is_some()
            && self.previous_range().end() == self.range().start()
    }

    pub(crate) fn at(&self, kind: SyntaxKind) -> bool {
        self.record(Expected::Token(kind));
        self.current() == kind
    }

    pub(crate) fn at_eof(&self) -> bool {
        self.current() == EOF
    }

    /// Whether the token `n` places ahead is the keyword `keyword`.
    pub(crate) fn keyword_at(&self, n: usize, keyword: &str) -> bool {
        self.nth(n) == WORD && self.nth_text(n).eq_ignore_ascii_case(keyword)
    }

    pub(crate) fn at_keyword(&self, keyword: &str) -> bool {
        if let Some(kind) = SyntaxKind::keyword(keyword) {
            self.record(Expected::Keyword(kind));
        }
        self.keyword_at(0, keyword)
    }

    /// Whether the token `n` places ahead is a name: a word that isn't reserved, or a quoted
    /// identifier.
    pub(crate) fn ident_at(&self, n: usize) -> bool {
        match self.nth(n) {
            WORD => !is_reserved(self.nth_text(n)),
            QUOTED_IDENT => true,
            _ => false,
        }
    }

    pub(crate) fn at_ident(&self) -> bool {
        self.ident_at(0)
    }

    // --- Events ---

    pub(crate) fn start(&mut self) -> Marker {
        let event = self.events.len() as u32;
        self.events.push(Event::Start {
            kind: ERROR,
            forward_parent: None,
        });
        Marker {
            event,
            position: self.position,
        }
    }

    /// Consumes the current token with the kind `kind`.
    fn bump_with(&mut self, kind: SyntaxKind) -> TextRange {
        let range = self.range();
        if self.at_eof() {
            return range;
        }
        self.events.push(Event::Token { kind });
        self.position += 1;
        range
    }

    /// Consumes the current token as it is.
    pub(crate) fn bump(&mut self) -> TextRange {
        self.bump_with(self.current())
    }

    /// Consumes the current word as a keyword, with the keyword's kind, or another token as it
    /// is.
    pub(crate) fn bump_keyword(&mut self) -> TextRange {
        let kind = match self.current() {
            WORD => SyntaxKind::keyword(self.nth_text(0)).unwrap_or(WORD),
            kind => kind,
        };
        self.bump_with(kind)
    }

    pub(crate) fn eat(&mut self, kind: SyntaxKind) -> bool {
        let found = self.at(kind);
        if found {
            self.bump();
        }
        found
    }

    pub(crate) fn eat_keyword(&mut self, keyword: &str) -> bool {
        let found = self.at_keyword(keyword);
        if found {
            self.bump_keyword();
        }
        found
    }

    /// Whether the current token is one of `keywords`, which it consumes when it is.
    pub(crate) fn eat_keyword_of(&mut self, keywords: &[&str]) -> bool {
        let found = keywords.iter().any(|keyword| self.at_keyword(keyword));
        if found {
            self.bump_keyword();
        }
        found
    }

    /// Whether the next tokens are the keywords `keywords`, which it consumes when they are.
    pub(crate) fn eat_keywords(&mut self, keywords: &[&str]) -> bool {
        if let Some(kind) = keywords.first().and_then(|word| SyntaxKind::keyword(word)) {
            self.record(Expected::Keyword(kind));
        }
        let found =
            (keywords.iter().enumerate()).all(|(offset, keyword)| self.keyword_at(offset, keyword));
        if found {
            for _ in keywords {
                self.bump_keyword();
            }
        }
        found
    }

    /// Consumes `kind`, or reports that it is missing.
    pub(crate) fn expect(&mut self, kind: SyntaxKind, what: &str) -> bool {
        if self.eat(kind) {
            return true;
        }
        self.error_expected(what);
        false
    }

    /// Consumes the keyword `keyword`, or reports that it is missing.
    pub(crate) fn expect_keyword(&mut self, keyword: &str) -> bool {
        if self.eat_keyword(keyword) {
            return true;
        }
        self.error_expected(&format!("`{keyword}`"));
        false
    }

    /// Consumes one of `keywords`, or reports that none is there.
    pub(crate) fn expect_keyword_of(&mut self, keywords: &[&str]) -> bool {
        if self.eat_keyword_of(keywords) {
            return true;
        }
        let names: Vec<String> = keywords
            .iter()
            .map(|keyword| format!("`{keyword}`"))
            .collect();
        self.error_expected(&names.join(" or "));
        false
    }

    /// Consumes `(` and enters its parentheses, where `)` ends a skip.
    pub(crate) fn open_paren(&mut self) -> bool {
        let found = self.expect(L_PAREN, "`(`");
        if found {
            self.paren_depth += 1;
        }
        found
    }

    /// Consumes the `)` of an [`open_paren`](Self::open_paren) that `opened`, after skipping
    /// what the parser did not read, and leaves the parentheses.
    pub(crate) fn close_paren(&mut self, opened: bool) {
        if !opened {
            return;
        }
        if !self.at(R_PAREN) && !self.at_eof() {
            self.error_expected("`)`");
            self.skip_until(|p| matches!(p.current(), R_PAREN | SEMICOLON | PIPE));
        }
        self.paren_depth -= 1;
        self.eat(R_PAREN);
    }

    // --- Errors and recovery ---

    pub(crate) fn error(
        &mut self,
        code: DiagnosticCode,
        range: TextRange,
        message: impl Into<String>,
    ) -> &mut Diagnostic {
        self.diagnostics.push(Diagnostic::new(code, range, message));
        self.diagnostics
            .last_mut()
            .expect("a diagnostic was pushed")
    }

    /// The description of the current token, for an error.
    fn found(&self) -> String {
        match self.current() {
            EOF => "the end of the text".to_string(),
            STRING => "a string".to_string(),
            BYTES => "a bytes literal".to_string(),
            _ => format!("`{}`", self.nth_text(0)),
        }
    }

    /// Reports that the current token is not `expected`.
    pub(crate) fn error_expected(&mut self, expected: &str) -> &mut Diagnostic {
        let message = format!("expected {expected}, found {}", self.found());
        let range = self.range();
        self.error(DiagnosticCode::Expected, range, message)
    }

    /// Whether the current token ends what the parser skips: the end, `;`, `|>`, or a `)` of
    /// the parentheses that the parser is inside.
    pub(crate) fn at_sync(&self) -> bool {
        matches!(self.current(), EOF | SEMICOLON | PIPE)
            || (self.current() == R_PAREN && self.paren_depth > 0)
    }

    /// Skips tokens into an `ERROR` node until `stop` accepts the current token or the text
    /// ends. A `(` and its tokens through the matching `)` are skipped together.
    pub(crate) fn skip_until(&mut self, stop: impl Fn(&Self) -> bool) {
        if stop(self) || self.at_eof() {
            return;
        }
        let marker = self.start();
        while !self.at_eof() && !stop(self) {
            self.skip_balanced();
        }
        marker.complete(self, ERROR);
    }

    /// Skips one token, or a `(` or `[` and its tokens through the matching one.
    fn skip_balanced(&mut self) {
        let open = self.current();
        let close = match open {
            L_PAREN => R_PAREN,
            L_BRACK => R_BRACK,
            _ => {
                self.bump();
                return;
            }
        };
        self.bump();
        let mut depth = 1;
        while !self.at_eof() && depth > 0 {
            match self.current() {
                kind if kind == open => depth += 1,
                kind if kind == close => depth -= 1,
                SEMICOLON => return,
                _ => {}
            }
            self.bump();
        }
    }

    /// Reports that the current token is not `expected`, and skips it into an `ERROR` node
    /// when it does not end a skip or a list.
    pub(crate) fn error_recover(&mut self, expected: &str) {
        self.error_expected(expected);
        if !self.at_sync() && !matches!(self.current(), COMMA | R_BRACK | R_PAREN) {
            let marker = self.start();
            self.skip_balanced();
            marker.complete(self, ERROR);
        }
    }

    // --- Nesting ---

    /// Enters a nested form, or reports that the nesting is too deep and skips to the end of
    /// the statement.
    pub(crate) fn enter(&mut self) -> bool {
        self.depth += 1;
        if self.depth <= MAX_DEPTH {
            return true;
        }
        self.depth -= 1;
        if self.depth == MAX_DEPTH {
            let range = self.range();
            self.error(
                DiagnosticCode::TooDeep,
                range,
                "this form is nested too deeply to parse",
            );
        }
        let saved = self.paren_depth;
        self.paren_depth = 0;
        self.skip_until(|p| p.current() == SEMICOLON);
        self.paren_depth = saved;
        false
    }

    pub(crate) fn leave(&mut self) {
        self.depth -= 1;
    }

    // --- Checkpoints ---

    pub(crate) fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            events: self.events.len(),
            position: self.position,
            diagnostics: self.diagnostics.len(),
            paren_depth: self.paren_depth,
            depth: self.depth,
        }
    }

    pub(crate) fn rewind(&mut self, checkpoint: &Checkpoint) {
        self.events.truncate(checkpoint.events);
        self.position = checkpoint.position;
        self.diagnostics.truncate(checkpoint.diagnostics);
        self.paren_depth = checkpoint.paren_depth;
        self.depth = checkpoint.depth;
    }

    /// Runs `parse`. When it reports an error, rewinds and returns the start of its first
    /// error, so the caller can try another form.
    pub(crate) fn attempt(&mut self, parse: impl FnOnce(&mut Self)) -> Result<(), TextSize> {
        let checkpoint = self.checkpoint();
        parse(self);
        match self.diagnostics.get(checkpoint.diagnostics) {
            None => Ok(()),
            Some(error) => {
                let at = error.range.start();
                self.rewind(&checkpoint);
                Err(at)
            }
        }
    }

    /// Runs `parse`, and keeps what it read unless it reports an error before `limit`.
    pub(crate) fn attempt_past(&mut self, limit: TextSize, parse: impl FnOnce(&mut Self)) -> bool {
        let checkpoint = self.checkpoint();
        parse(self);
        let early = (self.diagnostics.get(checkpoint.diagnostics))
            .is_some_and(|error| error.range.start() < limit);
        if early {
            self.rewind(&checkpoint);
        }
        !early
    }

    // --- Completion ---

    /// Records `expected` when the parser is at the cursor.
    pub(crate) fn record(&self, expected: Expected) {
        let Some(cursor) = self.cursor else {
            return;
        };
        if self.range().start() != cursor {
            return;
        }
        let mut list = self.expected.borrow_mut();
        if !list.contains(&expected) {
            list.push(expected);
        }
    }

    /// Records that a name in `role` can stand at the current token.
    pub(crate) fn record_name(&self, role: NameRole) {
        self.record(Expected::Name(role));
    }

    // --- Lists ---

    /// Items that `item` reads, separated by commas.
    pub(crate) fn comma_list<T>(&mut self, item: impl FnMut(&mut Self) -> T) {
        self.comma_list_until(item, |_| false);
    }

    /// Items that `item` reads, separated by commas, with a trailing comma before a token that
    /// `ends` accepts.
    pub(crate) fn comma_list_until<T>(
        &mut self,
        mut item: impl FnMut(&mut Self) -> T,
        ends: impl Fn(&Self) -> bool,
    ) {
        item(self);
        while self.eat(COMMA) {
            if ends(self) {
                break;
            }
            item(self);
        }
    }

    /// `(`, what `inside` reads when the `(` is there, and `)`.
    pub(crate) fn parens(&mut self, inside: impl FnOnce(&mut Self)) {
        let opened = self.open_paren();
        if opened {
            inside(self);
        }
        self.close_paren(opened);
    }

    /// `(item, ...)`, which is empty only when `empty` holds.
    pub(crate) fn paren_list<T>(&mut self, item: impl FnMut(&mut Self) -> T, empty: bool) {
        self.parens(|p| {
            if !(empty && p.current() == R_PAREN) {
                p.comma_list(item);
            }
        });
    }

    /// Whether the current token ends a pipe operator: `|>`, `)`, `;` or the end.
    pub(crate) fn at_operator_end(&self) -> bool {
        matches!(self.current(), PIPE | R_PAREN | SEMICOLON | EOF)
    }
}
