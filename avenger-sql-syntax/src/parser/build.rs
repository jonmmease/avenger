//! The tree of the parser's events.
//!
//! Trivia between two significant tokens goes in the innermost node that is open at the second
//! token, so a node starts and ends at significant tokens. A comment directly above a statement
//! or a pipe operator, with no blank line between, starts that node instead, so the node holds
//! its documentation.

use std::mem;

use rowan::{GreenNode, GreenNodeBuilder};

use super::Event;
use crate::{
    lexer::Token,
    SyntaxKind::{self, *},
};

/// The kinds of nodes that hold the comments directly above them.
fn holds_comments(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        MODULE_STMT
            | IMPORT_STMT
            | DECLARE_STMT
            | SET_STMT
            | CREATE_TABLE_STMT
            | CREATE_FUNCTION_STMT
            | CREATE_CONSTANT_STMT
            | QUERY
            | PIPE_SELECT
            | PIPE_EXTEND
            | PIPE_SET
            | PIPE_DROP
            | PIPE_RENAME
            | PIPE_AS
            | PIPE_WHERE
            | PIPE_AGGREGATE
            | PIPE_DISTINCT
            | PIPE_ORDER_BY
            | PIPE_LIMIT
            | PIPE_SET_OPERATION
            | PIPE_JOIN
            | PIPE_CALL
            | PIPE_SAMPLE
            | PIPE_WITH
            | PIPE_PIVOT
            | PIPE_UNPIVOT
    )
}

pub(super) fn build(source: &str, tokens: &[Token], mut events: Vec<Event>) -> GreenNode {
    let mut builder = Builder {
        source,
        tokens,
        next: 0,
        depth: 0,
        inner: GreenNodeBuilder::new(),
    };
    let mut parents = Vec::new();
    for index in 0..events.len() {
        match mem::replace(&mut events[index], Event::Tombstone) {
            Event::Start {
                kind,
                forward_parent,
            } => {
                // A node that `precede` wrapped starts after its parents, outermost first.
                parents.push(kind);
                let mut parent = forward_parent;
                let mut at = index;
                while let Some(distance) = parent {
                    at += distance as usize;
                    match mem::replace(&mut events[at], Event::Tombstone) {
                        Event::Start {
                            kind,
                            forward_parent,
                        } => {
                            parents.push(kind);
                            parent = forward_parent;
                        }
                        _ => unreachable!("a forward parent is a start event"),
                    }
                }
                for kind in parents.drain(..).rev() {
                    builder.start(kind);
                }
            }
            Event::Finish => builder.finish(),
            Event::Token { kind } => builder.token(kind),
            Event::Tombstone => {}
        }
    }
    builder.finish_all()
}

struct Builder<'a> {
    source: &'a str,
    tokens: &'a [Token],
    /// The index of the next token to emit.
    next: usize,
    depth: usize,
    inner: GreenNodeBuilder<'static>,
}

impl Builder<'_> {
    fn emit(&mut self, index: usize, kind: SyntaxKind) {
        let token = self.tokens[index];
        self.inner
            .token(rowan::SyntaxKind(kind as u16), &self.source[token.range]);
    }

    /// The number of trivia tokens before the next significant token.
    fn pending_trivia(&self) -> usize {
        self.tokens[self.next..]
            .iter()
            .take_while(|token| token.kind.is_trivia())
            .count()
    }

    /// The number of trailing trivia tokens of the pending trivia that a node starting at the
    /// next significant token holds: the comments directly above it and the whitespace between
    /// them, with no blank line.
    fn attached_trivia(&self, pending: usize) -> usize {
        let trivia = &self.tokens[self.next..self.next + pending];
        let mut attached = 0;
        let mut count = 0;
        for token in trivia.iter().rev() {
            count += 1;
            match token.kind {
                COMMENT => attached = count,
                WHITESPACE => {
                    let text = &self.source[token.range];
                    if text.matches('\n').count() > 1 {
                        break;
                    }
                }
                _ => break,
            }
        }
        attached
    }

    fn start(&mut self, kind: SyntaxKind) {
        if self.depth > 0 {
            let pending = self.pending_trivia();
            let attached = match holds_comments(kind) {
                true => self.attached_trivia(pending),
                false => 0,
            };
            for _ in 0..pending - attached {
                self.emit(self.next, self.tokens[self.next].kind);
                self.next += 1;
            }
        }
        self.inner.start_node(rowan::SyntaxKind(kind as u16));
        self.depth += 1;
    }

    fn token(&mut self, kind: SyntaxKind) {
        while self.tokens[self.next].kind.is_trivia() {
            self.emit(self.next, self.tokens[self.next].kind);
            self.next += 1;
        }
        self.emit(self.next, kind);
        self.next += 1;
    }

    fn finish(&mut self) {
        self.depth -= 1;
        if self.depth == 0 {
            while self.next < self.tokens.len() {
                self.emit(self.next, self.tokens[self.next].kind);
                self.next += 1;
            }
        }
        self.inner.finish_node();
    }

    fn finish_all(self) -> GreenNode {
        debug_assert_eq!(self.depth, 0, "every node finishes");
        self.inner.finish()
    }
}
