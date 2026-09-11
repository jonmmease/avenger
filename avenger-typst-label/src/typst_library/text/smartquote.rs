//! Retained Typst smartquote state machine.
//!
//! This mirrors the core of upstream
//! `typst-library/src/text/smartquote.rs`, reduced to Typst's default English
//! quote pairs and static label text.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SmartQuote {
    pub(crate) double: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct SmartQuoter {
    depth: u8,
    kinds: u32,
}

impl SmartQuoter {
    pub(crate) fn new() -> Self {
        Self { depth: 0, kinds: 0 }
    }

    pub(crate) fn quote(&mut self, before: Option<char>, double: bool) -> &'static str {
        let opened = self.top();
        let before = before.unwrap_or(' ');

        if before.is_numeric() && opened != Some(double) {
            return if double { "″" } else { "′" };
        }

        if !double && opened != Some(false) && (before.is_alphabetic() || before == '\u{FFFC}') {
            return "’";
        }

        if opened == Some(double)
            && !before.is_whitespace()
            && !crate::typst_syntax::is_newline(before)
            && !is_opening_bracket(before)
        {
            self.pop();
            return if double { "”" } else { "’" };
        }

        self.push(double);
        if double { "“" } else { "‘" }
    }

    fn top(&self) -> Option<bool> {
        self.depth
            .checked_sub(1)
            .map(|i| (self.kinds >> i) & 1 == 1)
    }

    fn push(&mut self, double: bool) {
        if self.depth < 32 {
            self.kinds |= (double as u32) << self.depth;
            self.depth += 1;
        }
    }

    fn pop(&mut self) {
        self.depth -= 1;
        self.kinds &= (1 << self.depth) - 1;
    }
}

impl Default for SmartQuoter {
    fn default() -> Self {
        Self::new()
    }
}

fn is_opening_bracket(c: char) -> bool {
    matches!(c, '(' | '{' | '[')
}

pub(crate) fn is_default_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{E0100}'..='\u{E01EF}'
    )
}
