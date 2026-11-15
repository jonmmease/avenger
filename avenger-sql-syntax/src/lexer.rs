//! The lexer: the tokens of a text, following GoogleSQL's lexer.
//!
//! The lexer never fails and loses no text: every byte belongs to a token, whitespace and
//! comments are trivia tokens, and a malformed token keeps a kind and adds a diagnostic, so the
//! parser can go on. Tokens keep their raw text: the accessors of [`crate::ast`] decode a
//! literal's value, and the validation pass checks its escapes. Keywords and unquoted names are [`SyntaxKind::WORD`] tokens, which the parser tells apart.
//! As in GoogleSQL, `<>` and `>>` are two tokens each, which the parser reads as one operator
//! when they are adjacent.

use rowan::{TextRange, TextSize};

use crate::{
    diagnostic::{Diagnostic, DiagnosticCode, TextEdit},
    SyntaxKind::{self, *},
};

/// A token: its kind and the range of its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: SyntaxKind,
    pub range: TextRange,
}

/// The tokens of `source`, which cover all of it, and the diagnostics of malformed tokens.
pub fn tokenize(source: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut lexer = Lexer {
        source,
        bytes: source.as_bytes(),
        position: 0,
        tokens: Vec::new(),
        diagnostics: Vec::new(),
        last: None,
        before_last: None,
    };
    lexer.run();
    (lexer.tokens, lexer.diagnostics)
}

struct Lexer<'a> {
    source: &'a str,
    bytes: &'a [u8],
    position: usize,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
    /// The kinds of the last two tokens that are not trivia.
    last: Option<SyntaxKind>,
    before_last: Option<SyntaxKind>,
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn range(start: usize, end: usize) -> TextRange {
    TextRange::new(TextSize::from(start as u32), TextSize::from(end as u32))
}

/// Whether a token of `kind` ends a name or an expression, so that a `.` after it is a field
/// access.
fn ends_name(kind: Option<SyntaxKind>) -> bool {
    matches!(
        kind,
        Some(WORD | QUOTED_IDENT | R_PAREN | R_BRACK | PARAM | POSITIONAL)
    )
}

impl Lexer<'_> {
    fn peek(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.position + offset).copied()
    }

    fn push(&mut self, kind: SyntaxKind, start: usize) {
        self.tokens.push(Token {
            kind,
            range: range(start, self.position),
        });
        if !kind.is_trivia() {
            self.before_last = self.last;
            self.last = Some(kind);
        }
    }

    fn error(&mut self, start: usize, end: usize, message: impl Into<String>) -> &mut Diagnostic {
        self.diagnostics.push(Diagnostic::new(
            DiagnosticCode::InvalidToken,
            range(start, end),
            message,
        ));
        self.diagnostics
            .last_mut()
            .expect("a diagnostic was pushed")
    }

    fn run(&mut self) {
        while let Some(byte) = self.peek(0) {
            let start = self.position;
            if self.whitespace().is_some() {
                while let Some(len) = self.whitespace() {
                    self.position += len;
                }
                self.push(WHITESPACE, start);
                continue;
            }
            match byte {
                b'#' => self.line_comment(),
                b'-' if self.peek(1) == Some(b'-') => self.line_comment(),
                b'/' if self.peek(1) == Some(b'*') => self.block_comment(),
                b'`' => self.quoted(start, 0, QUOTED_IDENT),
                b'\'' | b'"' => self.quoted(start, 0, STRING),
                // As in GoogleSQL, the word after a field's `.` is a name, even when it starts
                // with a digit, as in `t.2023`.
                b'0'..=b'9' if self.last == Some(DOT) && ends_name(self.before_last) => {
                    self.field_name()
                }
                b'0'..=b'9' => self.number(),
                b'.' if self.peek(1).is_some_and(|b| b.is_ascii_digit())
                    && !ends_name(self.last) =>
                {
                    self.number()
                }
                b'@' if self.peek(1) == Some(b'{') => self.hint(),
                b'@' if self.peek(1).is_some_and(|b| b.is_ascii_digit()) => self.number_hint(),
                b'@' => self.param(),
                _ if is_ident_start(byte) => self.word(),
                _ => self.punctuation(byte),
            }
        }
    }

    /// The length of the whitespace at the current position: a space, a tab, a line end, a
    /// backspace, a form feed, a vertical tab, or one of the Unicode spaces that GoogleSQL reads
    /// as whitespace.
    fn whitespace(&self) -> Option<usize> {
        self.whitespace_at(self.position)
    }

    fn whitespace_at(&self, at: usize) -> Option<usize> {
        match *self.bytes.get(at)? {
            b' ' | b'\t' | b'\n' | b'\r' | 0x08 | 0x0b | 0x0c => Some(1),
            byte if byte >= 0x80 => {
                let character = self.source[at..].chars().next()?;
                let space = matches!(
                    character,
                    '\u{a0}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
                );
                space.then_some(character.len_utf8())
            }
            _ => None,
        }
    }

    fn field_name(&mut self) {
        let start = self.position;
        while self.peek(0).is_some_and(is_ident_continue) {
            self.position += 1;
        }
        self.push(WORD, start);
    }

    /// A comment to the end of the line, at `\n` or `\r`.
    fn line_comment(&mut self) {
        let start = self.position;
        while let Some(byte) = self.peek(0) {
            if byte == b'\n' || byte == b'\r' {
                break;
            }
            self.position += 1;
        }
        self.push(COMMENT, start);
    }

    fn block_comment(&mut self) {
        let start = self.position;
        self.position += 2;
        loop {
            match self.peek(0) {
                None => {
                    self.error(start, start + 2, "this comment has no closing `*/`");
                    break;
                }
                Some(b'*') if self.peek(1) == Some(b'/') => {
                    self.position += 2;
                    break;
                }
                Some(_) => self.position += 1,
            }
        }
        self.push(COMMENT, start);
    }

    /// A word, or a string or bytes literal with an `r`, `b`, `rb` or `br` prefix.
    fn word(&mut self) {
        let start = self.position;
        let prefix_len = (0..2)
            .take_while(|&i| {
                self.peek(i)
                    .is_some_and(|b| matches!(b.to_ascii_lowercase(), b'r' | b'b'))
            })
            .count();
        for len in (1..=prefix_len).rev() {
            if !matches!(self.peek(len), Some(b'\'' | b'"')) {
                continue;
            }
            let prefix = self.source[start..start + len].to_ascii_lowercase();
            if prefix == "rr" || prefix == "bb" {
                continue;
            }
            let kind = match prefix.contains('b') {
                true => BYTES,
                false => STRING,
            };
            self.quoted(start, len, kind);
            return;
        }
        while self.peek(0).is_some_and(is_ident_continue) {
            self.position += 1;
        }
        self.push(WORD, start);
    }

    /// A quoted string, bytes literal, or identifier of `kind`, which starts at `start` with a
    /// prefix of `prefix` bytes and then its quote at the current position.
    fn quoted(&mut self, start: usize, prefix: usize, kind: SyntaxKind) {
        self.position = start + prefix;
        let quote = self.bytes[self.position];
        let triple = quote != b'`' && self.peek(1) == Some(quote) && self.peek(2) == Some(quote);
        self.position += if triple { 3 } else { 1 };
        let closed = loop {
            let Some(byte) = self.peek(0) else {
                break false;
            };
            if byte == quote {
                if !triple {
                    self.position += 1;
                    break true;
                }
                if self.peek(1) == Some(quote) && self.peek(2) == Some(quote) {
                    self.position += 3;
                    break true;
                }
            }
            if (byte == b'\n' || byte == b'\r') && !triple {
                break false;
            }
            // A backslash escapes the next byte, in a raw string too, for the end of the text.
            self.position += match byte == b'\\' && self.peek(1).is_some() {
                true => 2,
                false => 1,
            };
        };
        self.push(kind, start);
        if !closed {
            let what = match kind {
                QUOTED_IDENT => "identifier",
                _ => "literal",
            };
            let at_line_end = matches!(self.peek(0), Some(b'\n' | b'\r'));
            let diagnostic = self.error(
                start,
                self.position,
                format!("this quoted {what} has no closing quote"),
            );
            if at_line_end {
                diagnostic.help("triple quotes, ''' or \"\"\", hold line ends");
            }
        }
    }

    fn number(&mut self) {
        let start = self.position;
        if self.peek(0) == Some(b'0') && matches!(self.peek(1), Some(b'x' | b'X')) {
            self.position += 2;
            let digits = self.position;
            while self.peek(0).is_some_and(|b| b.is_ascii_hexdigit()) {
                self.position += 1;
            }
            if self.position == digits {
                // At the `x`, where GoogleSQL finds the alias of `0`.
                self.error(
                    start + 1,
                    self.position,
                    "a hex integer takes digits after 0x",
                );
            }
            self.end_number(start, INTEGER);
            return;
        }
        let mut float = false;
        self.skip_digits();
        if self.peek(0) == Some(b'.') {
            // As in GoogleSQL, `1.` is a float, so `1.x` is a float and an alias.
            float = true;
            self.position += 1;
            self.skip_digits();
        }
        if matches!(self.peek(0), Some(b'e' | b'E')) {
            let sign = usize::from(matches!(self.peek(1), Some(b'+' | b'-')));
            if self.peek(1 + sign).is_some_and(|b| b.is_ascii_digit()) {
                float = true;
                self.position += 1 + sign;
                self.skip_digits();
            }
        }
        let kind = if float { FLOAT } else { INTEGER };
        // As in GoogleSQL, a word can follow `2.` with no space, as in `1.x`, a float and its
        // alias, or `my-project-2.dataset`.
        if self.bytes[self.position - 1] == b'.' {
            self.push(kind, start);
            return;
        }
        self.end_number(start, kind);
    }

    fn skip_digits(&mut self) {
        while self.peek(0).is_some_and(|b| b.is_ascii_digit()) {
            self.position += 1;
        }
    }

    /// The end of a number of `kind`. A number followed by the letters of a name, as in `5abc`,
    /// is a name that starts with a digit: an error, but a name to the parser.
    fn end_number(&mut self, start: usize, kind: SyntaxKind) {
        if !self.peek(0).is_some_and(is_ident_continue) {
            self.push(kind, start);
            return;
        }
        let letters = self.position;
        while self.peek(0).is_some_and(is_ident_continue) {
            self.position += 1;
        }
        self.push(WORD, start);
        let text = self.source[start..self.position].to_string();
        let end = self.position;
        // At the letters, where GoogleSQL finds an alias with no space before it.
        self.error(letters, end, "a name can't start with a digit")
            .help("quote the name in backticks")
            .fix(
                "Quote the name in backticks",
                vec![TextEdit::new(range(start, end), format!("`{text}`"))],
            );
    }

    fn param(&mut self) {
        let start = self.position;
        self.position += 1;
        let system = self.peek(0) == Some(b'@');
        if system {
            self.position += 1;
        }
        // As in GoogleSQL, whitespace can separate `@` from the name.
        let mut after = self.position;
        while let Some(len) = self.whitespace_at(after) {
            after += len;
        }
        match self.bytes.get(after) {
            Some(b'`') => {
                self.position = after;
                self.quoted(start, after - start, PARAM);
            }
            Some(&byte) if is_ident_start(byte) => {
                self.position = after;
                while self.peek(0).is_some_and(is_ident_continue) {
                    self.position += 1;
                }
                self.push(PARAM, start);
            }
            _ => {
                self.push(PARAM, start);
                self.error(start, self.position, "a query parameter is `@` and a name");
                return;
            }
        }
        if system {
            self.error(
                start,
                self.position,
                "avenger-sql has no system variables, `@@name`",
            )
            .code = DiagnosticCode::Unsupported;
        }
    }

    /// A hint of a statement, `@` and a number, as in `@1 BEGIN ... END`, which avenger-sql reads
    /// as trivia and reports.
    fn number_hint(&mut self) {
        let start = self.position;
        self.position += 1;
        while self.peek(0).is_some_and(|b| b.is_ascii_digit()) {
            self.position += 1;
        }
        self.push(HINT, start);
        self.error(start, self.position, "avenger-sql has no hints, `@{...}`")
            .code = DiagnosticCode::Unsupported;
    }

    /// A hint, `@{...}` through its matching `}`, which avenger-sql reads as trivia and reports,
    /// since its planner chooses its own plans.
    fn hint(&mut self) {
        let start = self.position;
        self.position += 1;
        let mut depth = 0;
        while let Some(byte) = self.peek(0) {
            self.position += 1;
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        // A hint can hold any bytes, so it ends at a character's boundary.
        while !self.source.is_char_boundary(self.position) {
            self.position += 1;
        }
        self.push(HINT, start);
        self.error(start, self.position, "avenger-sql has no hints, `@{...}`")
            .code = DiagnosticCode::Unsupported;
    }

    fn punctuation(&mut self, byte: u8) {
        let start = self.position;
        let next = self.peek(1);
        let (kind, len) = match (byte, next) {
            (b'|', Some(b'>')) => (PIPE, 2),
            (b'|', Some(b'|')) => (CONCAT, 2),
            (b'|', _) => (BIT_OR, 1),
            (b'<', Some(b'<')) => (SHL, 2),
            (b'<', Some(b'=')) => (LT_EQ, 2),
            (b'<', _) => (LT, 1),
            (b'>', Some(b'=')) => (GT_EQ, 2),
            (b'>', _) => (GT, 1),
            (b'!', Some(b'=')) => (NEQ, 2),
            (b'=', Some(b'>')) => (FAT_ARROW, 2),
            (b'=', _) => (EQ, 1),
            (b'(', _) => (L_PAREN, 1),
            (b')', _) => (R_PAREN, 1),
            (b'[', _) => (L_BRACK, 1),
            (b']', _) => (R_BRACK, 1),
            (b',', _) => (COMMA, 1),
            (b'.', _) => (DOT, 1),
            (b';', _) => (SEMICOLON, 1),
            (b'+', _) => (PLUS, 1),
            (b'-', _) => (MINUS, 1),
            (b'*', _) => (STAR, 1),
            (b'/', _) => (SLASH, 1),
            (b'&', _) => (AMP, 1),
            (b'^', _) => (CARET, 1),
            (b'~', _) => (TILDE, 1),
            (b'?', _) => (POSITIONAL, 1),
            (b':', _) => (COLON, 1),
            (b'{', _) => (L_BRACE, 1),
            (b'}', _) => (R_BRACE, 1),
            _ => {
                let character = self.source[start..].chars().next().unwrap_or_default();
                self.position += character.len_utf8().max(1);
                self.push(ERROR_TOKEN, start);
                self.error(
                    start,
                    self.position,
                    format!("`{character}` is not part of the grammar"),
                );
                return;
            }
        };
        self.position += len;
        self.push(kind, start);
    }
}
