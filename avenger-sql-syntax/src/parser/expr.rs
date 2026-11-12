//! Names, expressions, types, calls and windows. Expressions use precedence climbing over the
//! operator precedence of GoogleSQL's grammar.

use super::{CompletedMarker, Marker, Parser};
use crate::{
    diagnostic::{DiagnosticCode, Fix, TextEdit},
    parser::is_reserved,
    Expected, NameRole,
    SyntaxKind::{self, *},
};

/// The precedence levels of GoogleSQL's grammar, from the loosest binding.
const OR: u8 = 1;
const AND: u8 = 2;
const NOT: u8 = 3;
const COMPARISON: u8 = 4;
pub(super) const BIT_OR_LEVEL: u8 = 5;
const BIT_XOR: u8 = 6;
const BIT_AND: u8 = 7;
const SHIFT: u8 = 8;
const ADDITIVE: u8 = 9;
const CONCAT_LEVEL: u8 = 10;
const MULTIPLICATIVE: u8 = 11;
const UNARY: u8 = 12;

/// The reserved keywords that name functions when `(` follows them.
const RESERVED_FUNCTIONS: &[&str] = &["IF", "LEFT", "RIGHT", "GROUPING", "COLLATE", "RANGE"];

/// The functions that GoogleSQL calls without parentheses, as in `CURRENT_DATE`.
const BARE_CALLS: &[&str] = &[
    "CURRENT_DATE",
    "CURRENT_TIME",
    "CURRENT_DATETIME",
    "CURRENT_TIMESTAMP",
];

/// The keywords of typed literals, such as `DATE '2024-01-01'`.
const TYPED_LITERALS: &[&str] = &[
    "DATE",
    "TIME",
    "DATETIME",
    "TIMESTAMP",
    "NUMERIC",
    "DECIMAL",
    "BIGNUMERIC",
    "BIGDECIMAL",
    "JSON",
];

impl Parser<'_> {
    // --- Names ---

    /// Reports that the current token is not a name, with a fix that quotes a reserved word.
    /// A reserved word before the end of a list or an operator becomes the name, so the parse
    /// goes on.
    fn error_name(&mut self, kind: SyntaxKind) -> bool {
        let reserved = self.current() == WORD;
        let word = self.nth_text(0).to_string();
        let range = self.range();
        let ends = reserved
            && matches!(
                self.nth(1),
                COMMA | R_PAREN | PIPE | SEMICOLON | EOF | DOT | EQ
            );
        let diagnostic = self.error_expected("a name");
        if !reserved {
            return false;
        }
        diagnostic.code = DiagnosticCode::ReservedName;
        diagnostic.help = Some(format!(
            "`{word}` is reserved; write it in backticks to use it as a name"
        ));
        diagnostic.fixes.push(Fix {
            label: "Quote the name in backticks".to_string(),
            edits: vec![TextEdit {
                range,
                replacement: format!("`{word}`"),
            }],
        });
        if ends {
            let marker = self.start();
            self.bump();
            marker.complete(self, kind);
        }
        ends
    }

    /// A name that the node defines, such as an alias.
    pub(super) fn name(&mut self, role: NameRole) -> bool {
        self.record_name(role);
        if !self.at_ident() {
            return self.error_name(NAME);
        }
        let marker = self.start();
        self.bump();
        marker.complete(self, NAME);
        true
    }

    /// A name that refers to a definition, such as a column.
    pub(super) fn name_ref(&mut self, role: NameRole) -> bool {
        self.record_name(role);
        if !self.at_ident() {
            return self.error_name(NAME_REF);
        }
        let marker = self.start();
        self.bump();
        marker.complete(self, NAME_REF);
        true
    }

    /// A name after a `.`, which can be a reserved keyword, as in `t.group`.
    fn field_name_ref(&mut self) -> bool {
        self.record_name(NameRole::Field);
        if !matches!(self.current(), WORD | QUOTED_IDENT) {
            self.error_expected("a name");
            return false;
        }
        let marker = self.start();
        self.bump();
        marker.complete(self, NAME_REF);
        true
    }

    /// `name (. name)*`, a path whose first name is in `role`, and the number of its names.
    pub(super) fn path(&mut self, role: NameRole) -> Option<(CompletedMarker, usize)> {
        if !self.at_ident() {
            self.record_name(role);
            if !self.error_name(NAME_REF) {
                return None;
            }
            let marker = self.start();
            return Some((marker.complete(self, PATH), 1));
        }
        let marker = self.start();
        self.name_ref(role);
        Some(self.path_rest(marker, 1))
    }

    /// The rest of a path after its first `parts` names.
    pub(super) fn path_rest(
        &mut self,
        marker: Marker,
        mut parts: usize,
    ) -> (CompletedMarker, usize) {
        while self.current() == DOT && matches!(self.nth(1), WORD | QUOTED_IDENT) {
            self.bump();
            self.field_name_ref();
            parts += 1;
        }
        (marker.complete(self, PATH), parts)
    }

    /// `(name, ...)` of names in `role`.
    pub(super) fn name_list(&mut self, role: NameRole) {
        let marker = self.start();
        let opened = self.open_paren();
        if opened {
            self.comma_list(
                |p| {
                    p.name_ref(role);
                },
                |_| false,
            );
        }
        self.close_paren(opened);
        marker.complete(self, NAME_LIST);
    }

    /// `[AS] alias`, where an alias without `AS` is a name that isn't reserved.
    pub(super) fn alias(&mut self, role: NameRole) -> bool {
        if self.at_keyword("AS") {
            let marker = self.start();
            self.bump_keyword();
            self.name(role);
            marker.complete(self, ALIAS);
            return true;
        }
        self.record_name(role);
        if !self.at_ident() {
            return false;
        }
        let marker = self.start();
        self.name(role);
        marker.complete(self, ALIAS);
        true
    }

    // --- Expressions ---

    pub(crate) fn expr(&mut self) -> Option<CompletedMarker> {
        self.expr_at(0)
    }

    /// An expression whose operators bind at least as tightly as `level`.
    pub(super) fn expr_at(&mut self, level: u8) -> Option<CompletedMarker> {
        if !self.enter() {
            return None;
        }
        let result = self.expr_at_inner(level);
        self.leave();
        result
    }

    fn expr_at_inner(&mut self, level: u8) -> Option<CompletedMarker> {
        let mut left = self.prefix(level)?;
        let mut compared = false;
        while let Some(op_level) = self.infix_level() {
            if op_level < level {
                break;
            }
            if op_level == COMPARISON {
                if compared {
                    let range = self.range();
                    self.error(
                        DiagnosticCode::ChainedComparison,
                        range,
                        "comparisons don't chain",
                    )
                    .help = Some("use parentheses or AND, as in `a < b AND b < c`".to_string());
                }
                compared = true;
                left = self.comparison(left);
                continue;
            }
            let marker = left.precede(self);
            let shift = self.current() == GT;
            match self.current() {
                WORD => {
                    self.bump_keyword();
                }
                _ => {
                    self.bump();
                }
            }
            if shift {
                self.bump();
            }
            self.expr_at(op_level + 1);
            left = marker.complete(self, BIN_EXPR);
        }
        Some(left)
    }

    /// The level of the infix operator at the current token.
    fn infix_level(&self) -> Option<u8> {
        let level = match self.current() {
            GT if self.adjacent_gt() => SHIFT,
            EQ | NEQ | LT | LT_EQ | GT | GT_EQ => COMPARISON,
            BIT_OR => BIT_OR_LEVEL,
            CARET => BIT_XOR,
            AMP => BIT_AND,
            SHL => SHIFT,
            PLUS | MINUS => ADDITIVE,
            CONCAT => CONCAT_LEVEL,
            STAR | SLASH => MULTIPLICATIVE,
            WORD => {
                if self.at_keyword("OR") {
                    OR
                } else if self.at_keyword("AND") {
                    AND
                } else if self.at_keyword("IS")
                    || self.at_keyword("LIKE")
                    || self.at_keyword("BETWEEN")
                    || self.at_keyword("IN")
                    || (self.at_keyword("NOT")
                        && (self.keyword_at(1, "LIKE")
                            || self.keyword_at(1, "BETWEEN")
                            || self.keyword_at(1, "IN")))
                {
                    COMPARISON
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        Some(level)
    }

    /// The comparison after `left`: an operator such as `<`, or `IS`, `LIKE`, `BETWEEN` or `IN`.
    fn comparison(&mut self, left: CompletedMarker) -> CompletedMarker {
        let marker = left.precede(self);
        if matches!(self.current(), EQ | NEQ | LT | LT_EQ | GT | GT_EQ) {
            let not_equal = self.adjacent_lt_gt();
            self.bump();
            if not_equal {
                self.bump();
            }
            self.expr_at(BIT_OR_LEVEL);
            return marker.complete(self, BIN_EXPR);
        }
        if self.eat_keyword("IS") {
            self.eat_keyword("NOT");
            if self.eat_keywords(&["DISTINCT", "FROM"]) {
                self.expr_at(BIT_OR_LEVEL);
                return marker.complete(self, IS_DISTINCT_EXPR);
            }
            let test = ["NULL", "TRUE", "FALSE", "UNKNOWN"]
                .iter()
                .any(|word| self.at_keyword(word));
            match test {
                true => {
                    self.bump_keyword();
                }
                false => {
                    self.error_expected("`NULL`, `TRUE`, `FALSE`, `UNKNOWN` or `DISTINCT FROM`");
                }
            }
            return marker.complete(self, IS_EXPR);
        }
        self.eat_keyword("NOT");
        if self.eat_keyword("LIKE") {
            if ["ANY", "SOME", "ALL"]
                .iter()
                .any(|word| self.at_keyword(word))
            {
                let patterns = self.start();
                self.bump_keyword();
                self.in_set(true);
                patterns.complete(self, QUANTIFIED_PATTERNS);
            } else {
                self.expr_at(BIT_OR_LEVEL);
            }
            return marker.complete(self, LIKE_EXPR);
        }
        if self.eat_keyword("BETWEEN") {
            self.expr_at(BIT_OR_LEVEL);
            self.expect_keyword("AND");
            self.expr_at(BIT_OR_LEVEL);
            return marker.complete(self, BETWEEN_EXPR);
        }
        self.expect_keyword("IN");
        self.in_set(true);
        marker.complete(self, IN_EXPR)
    }

    /// `UNNEST(array)`, or values in parentheses, or a query in parentheses when `query`.
    fn in_set(&mut self, query: bool) {
        let marker = self.start();
        if self.eat_keyword("UNNEST") {
            let opened = self.open_paren();
            self.expr();
            self.close_paren(opened);
            marker.complete(self, IN_UNNEST);
            return;
        }
        if query && self.at(L_PAREN) && self.keyword_at(1, "FROM") {
            self.parenthesized_query();
            marker.complete(self, IN_QUERY);
            return;
        }
        let opened = self.open_paren();
        if opened {
            self.comma_list(
                |p| {
                    p.expr();
                },
                |_| false,
            );
        }
        self.close_paren(opened);
        marker.complete(self, IN_LIST);
    }

    /// A prefix operator and its operand, or a primary and its field accesses and subscripts,
    /// in an expression at `level`. As in GoogleSQL, a `NOT` in the operand of a tighter
    /// operator needs parentheses.
    fn prefix(&mut self, level: u8) -> Option<CompletedMarker> {
        self.record(Expected::Expression);
        let operand_level = match self.current() {
            PLUS | MINUS | TILDE => Some(UNARY),
            WORD if self.at_keyword("NOT") => Some(NOT),
            _ => None,
        };
        if let Some(operand_level) = operand_level {
            let misplaced = operand_level == NOT && level > NOT;
            let diagnostic = match misplaced {
                true => {
                    let range = self.range();
                    self.error(
                        DiagnosticCode::UnparenthesizedNot,
                        range,
                        "this NOT needs parentheses",
                    )
                    .help = Some("write the NOT in parentheses, as in `a = (NOT b)`".to_string());
                    Some(self.diagnostics.len() - 1)
                }
                false => None,
            };
            let marker = self.start();
            match self.current() {
                WORD => {
                    self.bump_keyword();
                }
                _ => {
                    self.bump();
                }
            }
            self.expr_at(operand_level);
            let node = marker.complete(self, PREFIX_EXPR);
            if let Some(index) = diagnostic {
                let range = self.node_range(node);
                self.diagnostics[index].fixes.push(Fix {
                    label: "Put the NOT in parentheses".to_string(),
                    edits: vec![
                        TextEdit {
                            range: rowan::TextRange::empty(range.start()),
                            replacement: "(".to_string(),
                        },
                        TextEdit {
                            range: rowan::TextRange::empty(range.end()),
                            replacement: ")".to_string(),
                        },
                    ],
                });
            }
            return Some(node);
        }
        let mut expr = self.primary()?;
        loop {
            if self.current() == DOT {
                // A reserved word follows a `.` only after a name, as in `t.select`.
                let after_name = matches!(self.previous_kind(), Some(WORD | QUOTED_IDENT));
                let marker = expr.precede(self);
                self.bump();
                match after_name {
                    true => {
                        self.field_name_ref();
                    }
                    false => self.field_after_expr(),
                }
                expr = marker.complete(self, FIELD_EXPR);
            } else if self.current() == L_BRACK {
                let marker = expr.precede(self);
                self.bump();
                self.expr();
                self.expect(R_BRACK, "`]`");
                expr = marker.complete(self, SUBSCRIPT_EXPR);
            } else {
                return Some(expr);
            }
        }
    }

    /// The name of a field after `.` and a base that isn't a path, which, as in GoogleSQL,
    /// isn't a reserved keyword.
    fn field_after_expr(&mut self) {
        let reserved = self.current() == WORD
            && is_reserved(self.nth_text(0))
            && !self.nth_text(0).as_bytes()[0].is_ascii_digit();
        if reserved {
            self.error_expected("a name");
            return;
        }
        self.field_name_ref();
    }

    /// A literal, a name, a call, or a form in parentheses or brackets.
    pub(super) fn primary(&mut self) -> Option<CompletedMarker> {
        match self.current() {
            INTEGER | FLOAT => {
                let marker = self.start();
                self.bump();
                Some(marker.complete(self, LITERAL))
            }
            STRING | BYTES => {
                let marker = self.start();
                self.string_literal();
                Some(marker.complete(self, LITERAL))
            }
            PARAM | POSITIONAL => {
                let marker = self.start();
                self.bump();
                Some(marker.complete(self, PARAM_EXPR))
            }
            L_PAREN => Some(self.parenthesized()),
            L_BRACK => {
                let marker = self.start();
                self.array_elements();
                Some(marker.complete(self, ARRAY_EXPR))
            }
            QUOTED_IDENT => self.name_or_call(),
            WORD => self.word_expr(),
            _ => {
                self.error_recover("an expression");
                None
            }
        }
    }

    /// Adjacent string literals, or adjacent bytes literals, as one literal. As in GoogleSQL,
    /// whitespace or a comment separates them.
    fn string_literal(&mut self) {
        let kind = self.current();
        self.bump();
        while matches!(self.current(), STRING | BYTES) {
            if self.touches_previous() {
                let range = self.range();
                let at = range.start();
                self.error(
                    DiagnosticCode::AdjacentLiterals,
                    range,
                    "concatenated string literals need whitespace or a comment between them",
                )
                .fixes
                .push(Fix {
                    label: "Add a space between the literals".to_string(),
                    edits: vec![TextEdit {
                        range: rowan::TextRange::empty(at),
                        replacement: " ".to_string(),
                    }],
                });
            } else if self.current() != kind {
                let range = self.range();
                self.error(
                    DiagnosticCode::InvalidToken,
                    range,
                    "a string literal and a bytes literal don't concatenate",
                );
            }
            self.bump();
        }
    }

    /// A form that starts with a word: a keyword form, a typed literal, a name or a call.
    fn word_expr(&mut self) -> Option<CompletedMarker> {
        let word = self.nth_text(0).to_ascii_uppercase();
        let next = self.nth(1);
        let marker = self.start();
        let kind = match word.as_str() {
            "NULL" | "TRUE" | "FALSE" => {
                self.bump_keyword();
                LITERAL
            }
            "CASE" => {
                self.case();
                CASE_EXPR
            }
            "CAST" | "SAFE_CAST" if next == L_PAREN => {
                self.cast();
                CAST_EXPR
            }
            "EXTRACT" => {
                self.extract();
                EXTRACT_EXPR
            }
            "INTERVAL" => {
                self.bump_keyword();
                self.expr();
                self.name_ref(NameRole::DatePart);
                if self.eat_keyword("TO") {
                    self.name_ref(NameRole::DatePart);
                }
                INTERVAL_EXPR
            }
            "EXISTS" => {
                self.bump_keyword();
                self.parenthesized_query();
                EXISTS_EXPR
            }
            "ARRAY" if next == L_PAREN => {
                self.bump_keyword();
                self.parenthesized_query();
                ARRAY_SUBQUERY_EXPR
            }
            "ARRAY" => {
                self.bump_keyword();
                if self.eat(LT) {
                    self.ty();
                    self.expect(GT, "`>`");
                }
                self.array_elements();
                ARRAY_EXPR
            }
            "STRUCT" => {
                self.struct_literal();
                STRUCT_EXPR
            }
            "RANGE" if next == LT => {
                self.bump_keyword();
                self.bump();
                self.ty();
                self.expect(GT, "`>`");
                self.typed_value();
                TYPED_LITERAL
            }
            _ if TYPED_LITERALS.contains(&word.as_str()) && next == STRING => {
                self.bump_keyword();
                self.typed_value();
                TYPED_LITERAL
            }
            _ if TYPED_LITERALS.contains(&word.as_str()) && next == BYTES => {
                self.bump_keyword();
                let range = self.range();
                self.error(
                    DiagnosticCode::Expected,
                    range,
                    "a typed literal takes a string, not bytes",
                );
                self.string_literal();
                TYPED_LITERAL
            }
            _ if RESERVED_FUNCTIONS.contains(&word.as_str()) && next == L_PAREN => {
                let path = self.start();
                let name = self.start();
                self.bump();
                name.complete(self, NAME_REF);
                path.complete(self, PATH);
                self.call_arguments();
                CALL_EXPR
            }
            "SELECT" => {
                marker.abandon(self);
                self.error_expected("an expression").help =
                    Some("a query starts with FROM, as in `(FROM t |> SELECT a)`".to_string());
                self.skip_until(|p| p.at_sync() || p.current() == COMMA);
                return None;
            }
            _ => {
                marker.abandon(self);
                return self.name_or_call();
            }
        };
        Some(marker.complete(self, kind))
    }

    /// The string of a typed literal.
    fn typed_value(&mut self) {
        self.string_literal();
    }

    /// A path, or a call when `(` follows it.
    fn name_or_call(&mut self) -> Option<CompletedMarker> {
        let bare = self.current() == WORD
            && BARE_CALLS
                .iter()
                .any(|call| call.eq_ignore_ascii_case(self.nth_text(0)))
            && self.nth(1) != DOT;
        let (path, _) = self.path(NameRole::Column)?;
        let marker = path.precede(self);
        if self.current() == L_PAREN {
            self.call_arguments();
            return Some(marker.complete(self, CALL_EXPR));
        }
        Some(marker.complete(self, if bare { CALL_EXPR } else { PATH_EXPR }))
    }

    /// The arguments and clauses of a call, from `(`, and its `OVER`.
    fn call_arguments(&mut self) {
        let list = self.start();
        let opened = self.open_paren();
        if opened {
            self.eat_keyword("DISTINCT");
            if self.current() == STAR {
                let star = self.start();
                let range = self.bump();
                star.complete(self, STAR_ARG);
                if self.current() == COMMA {
                    self.error(
                        DiagnosticCode::Expected,
                        range,
                        "`*` is a call's only argument, as in `COUNT(*)`",
                    );
                    while self.eat(COMMA) {
                        self.argument();
                    }
                }
            } else if self.current() != R_PAREN {
                self.comma_list(Self::argument, |_| false);
            }
            if self.at_keyword("IGNORE") || self.at_keyword("RESPECT") {
                let treatment = self.start();
                self.bump_keyword();
                self.expect_keyword("NULLS");
                treatment.complete(self, NULL_TREATMENT);
            }
            if self.at_keyword("HAVING") {
                let having = self.start();
                self.bump_keyword();
                match self.at_keyword("MAX") || self.at_keyword("MIN") {
                    true => {
                        self.bump_keyword();
                    }
                    false => {
                        self.error_expected("`MAX` or `MIN`");
                    }
                }
                self.expr();
                having.complete(self, HAVING_CLAUSE);
            }
            if self.at_keyword("GROUP") || self.at_keyword("WHERE") {
                let range = self.range();
                self.error(
                    DiagnosticCode::Unsupported,
                    range,
                    "avenger-sql doesn't support WHERE or GROUP BY in an aggregate call",
                );
                self.skip_until(|p| {
                    p.current() == R_PAREN || p.at_keyword("ORDER") || p.at_keyword("LIMIT")
                });
            }
            if self.at_keyword("ORDER") {
                self.order_by(|_| false);
            }
            if self.at_keyword("LIMIT") {
                let limit = self.start();
                self.bump_keyword();
                self.expr();
                limit.complete(self, CALL_LIMIT);
            }
        }
        self.close_paren(opened);
        list.complete(self, ARG_LIST);
        if self.at_keyword("OVER") {
            let over = self.start();
            self.bump_keyword();
            self.window_ref();
            over.complete(self, OVER_CLAUSE);
        }
    }

    /// An argument of a function call: a value, or `name => value`.
    fn argument(&mut self) {
        self.record_name(NameRole::Parameter);
        let marker = self.start();
        if self.at_ident() && self.nth(1) == FAT_ARROW {
            self.name_ref(NameRole::Parameter);
            self.bump();
            self.expr();
            marker.complete(self, NAMED_ARG);
            return;
        }
        self.expr();
        marker.complete(self, ARG);
    }

    /// An argument of a table function call: a value, `TABLE name`, or either after `name =>`.
    fn table_argument(&mut self) {
        if self.current() == STAR {
            let range = self.range();
            self.error(
                DiagnosticCode::Expected,
                range,
                "a table function takes no `*` argument",
            );
            let marker = self.start();
            self.bump();
            marker.complete(self, ERROR);
            return;
        }
        let marker = self.start();
        if self.at_ident()
            && self.nth(1) == FAT_ARROW
            && self.keyword_at(2, "TABLE")
            && self.ident_at(3)
        {
            self.name_ref(NameRole::Parameter);
            self.bump();
            self.bump_keyword();
            self.path(NameRole::Table);
            marker.complete(self, NAMED_TABLE_ARG);
            return;
        }
        if self.keyword_at(0, "TABLE") && self.ident_at(1) {
            self.bump_keyword();
            self.path(NameRole::Table);
            marker.complete(self, TABLE_ARG);
            return;
        }
        marker.abandon(self);
        self.argument();
    }

    /// The arguments of a table function call, in parentheses.
    pub(super) fn table_arguments(&mut self) {
        let marker = self.start();
        let opened = self.open_paren();
        if opened && self.current() != R_PAREN {
            self.comma_list(Self::table_argument, |_| false);
        }
        self.close_paren(opened);
        marker.complete(self, ARG_LIST);
    }

    /// `(expr)`, a tuple of two values or more, or a query in parentheses.
    fn parenthesized(&mut self) -> CompletedMarker {
        let marker = self.start();
        if self.keyword_at(1, "FROM") {
            self.parenthesized_query();
            return marker.complete(self, SUBQUERY_EXPR);
        }
        // `((FROM t) |> WHERE x)` is a query, and `((FROM t) + 1)` an expression.
        if self.nth(1) == L_PAREN
            && self
                .attempt(|p| {
                    p.parenthesized_query();
                })
                .is_ok()
        {
            return marker.complete(self, SUBQUERY_EXPR);
        }
        let opened = self.open_paren();
        self.expr();
        let mut tuple = false;
        while self.eat(COMMA) {
            tuple = true;
            self.expr();
        }
        self.close_paren(opened);
        marker.complete(self, if tuple { TUPLE_EXPR } else { PAREN_EXPR })
    }

    /// `[elements]`, after `ARRAY` and its type, when the array has them.
    fn array_elements(&mut self) {
        if !self.expect(L_BRACK, "`[`") {
            return;
        }
        if self.current() != R_BRACK {
            self.comma_list(
                |p| {
                    p.expr();
                },
                |_| false,
            );
        }
        self.expect(R_BRACK, "`]`");
    }

    /// `STRUCT[<fields>](values)`
    fn struct_literal(&mut self) {
        self.bump_keyword();
        if self.eat(LT) {
            self.struct_fields();
        }
        let opened = self.open_paren();
        if opened && self.current() != R_PAREN {
            self.comma_list(
                |p| {
                    let marker = p.start();
                    p.expr();
                    if p.at_keyword("AS") {
                        let alias = p.start();
                        p.bump_keyword();
                        p.name(NameRole::Field);
                        alias.complete(p, ALIAS);
                    }
                    marker.complete(p, ALIASED_EXPR);
                },
                |_| false,
            );
        }
        self.close_paren(opened);
    }

    fn case(&mut self) {
        self.bump_keyword();
        if !self.at_keyword("WHEN") {
            self.expr();
        }
        let mut whens = 0;
        while self.at_keyword("WHEN") {
            let marker = self.start();
            self.bump_keyword();
            self.expr();
            self.expect_keyword("THEN");
            self.expr();
            marker.complete(self, WHEN_CLAUSE);
            whens += 1;
        }
        if whens == 0 {
            self.error_expected("`WHEN`");
        }
        if self.at_keyword("ELSE") {
            let marker = self.start();
            self.bump_keyword();
            self.expr();
            marker.complete(self, ELSE_CLAUSE);
        }
        self.expect_keyword("END");
    }

    /// `CAST(expr AS type [FORMAT format [AT TIME ZONE zone]])`, or `SAFE_CAST`.
    fn cast(&mut self) {
        self.bump_keyword();
        let opened = self.open_paren();
        if opened {
            self.expr();
            self.expect_keyword("AS");
            self.ty();
            if self.at_keyword("FORMAT") {
                let format = self.start();
                self.bump_keyword();
                self.expr();
                self.time_zone();
                format.complete(self, FORMAT_CLAUSE);
            }
        }
        self.close_paren(opened);
    }

    /// `[AT TIME ZONE zone]`
    fn time_zone(&mut self) {
        let at_zone =
            self.at_keyword("AT") && self.keyword_at(1, "TIME") && self.keyword_at(2, "ZONE");
        if !at_zone {
            return;
        }
        let marker = self.start();
        self.eat_keywords(&["AT", "TIME", "ZONE"]);
        self.expr();
        marker.complete(self, TIME_ZONE);
    }

    /// `EXTRACT(part FROM expr [AT TIME ZONE zone])`
    fn extract(&mut self) {
        self.bump_keyword();
        let opened = self.open_paren();
        if opened {
            self.record_name(NameRole::DatePart);
            self.name_or_call();
            self.expect_keyword("FROM");
            self.expr();
            self.time_zone();
        }
        self.close_paren(opened);
    }

    /// `ORDER BY` keys, up to a token that `ends` accepts after a trailing comma.
    pub(super) fn order_by(&mut self, ends: impl Fn(&Self) -> bool) {
        let marker = self.start();
        self.bump_keyword();
        self.expect_keyword("BY");
        self.comma_list(
            |p| {
                let key = p.start();
                p.expr();
                if p.at_keyword("ASC") || p.at_keyword("DESC") {
                    p.bump_keyword();
                }
                p.nulls();
                key.complete(p, ORDER_KEY);
            },
            ends,
        );
        marker.complete(self, ORDER_BY);
    }

    /// `[NULLS {FIRST | LAST}]`
    pub(super) fn nulls(&mut self) {
        if !self.eat_keyword("NULLS") {
            return;
        }
        match self.at_keyword("FIRST") || self.at_keyword("LAST") {
            true => {
                self.bump_keyword();
            }
            false => {
                self.error_expected("`FIRST` or `LAST`");
            }
        }
    }

    /// The window of `OVER` or of `WINDOW name AS`.
    pub(super) fn window_ref(&mut self) {
        if self.current() != L_PAREN {
            self.name_ref(NameRole::Window);
            return;
        }
        let marker = self.start();
        let opened = self.open_paren();
        if opened {
            if self.at_ident() {
                self.name_ref(NameRole::Window);
            }
            if self.at_keyword("PARTITION") {
                let partition = self.start();
                self.bump_keyword();
                self.expect_keyword("BY");
                self.comma_list(
                    |p| {
                        p.expr();
                    },
                    |_| false,
                );
                partition.complete(self, PARTITION_BY);
            }
            if self.at_keyword("ORDER") {
                self.order_by(|_| false);
            }
            if self.at_keyword("ROWS") || self.at_keyword("RANGE") {
                self.frame();
            }
        }
        self.close_paren(opened);
        marker.complete(self, WINDOW_SPEC);
    }

    fn frame(&mut self) {
        let marker = self.start();
        self.bump_keyword();
        if self.eat_keyword("BETWEEN") {
            self.frame_bound();
            self.expect_keyword("AND");
            self.frame_bound();
        } else {
            self.frame_bound();
        }
        marker.complete(self, FRAME);
    }

    fn frame_bound(&mut self) {
        let marker = self.start();
        if self.eat_keyword("UNBOUNDED") {
            match self.at_keyword("PRECEDING") || self.at_keyword("FOLLOWING") {
                true => {
                    self.bump_keyword();
                }
                false => {
                    self.error_expected("`PRECEDING` or `FOLLOWING`");
                }
            }
        } else if !self.eat_keywords(&["CURRENT", "ROW"]) {
            self.expr();
            match self.at_keyword("PRECEDING") || self.at_keyword("FOLLOWING") {
                true => {
                    self.bump_keyword();
                }
                false => {
                    self.error_expected("`PRECEDING` or `FOLLOWING`");
                }
            }
        }
        marker.complete(self, FRAME_BOUND);
    }

    // --- Types ---

    /// A type, such as `INT64`, `NUMERIC(10, 2)`, `ARRAY<STRING>` or `STRUCT<x INT64>`.
    pub(super) fn ty(&mut self) {
        self.record(Expected::Type);
        let marker = self.start();
        let kind = if self.eat_keyword("ARRAY") {
            self.expect(LT, "`<`");
            self.ty();
            self.expect(GT, "`>`");
            ARRAY_TYPE
        } else if self.eat_keyword("STRUCT") {
            if self.expect(LT, "`<`") {
                self.struct_fields();
            }
            STRUCT_TYPE
        } else if self.eat_keyword("RANGE") {
            self.expect(LT, "`<`");
            self.ty();
            self.expect(GT, "`>`");
            RANGE_TYPE
        } else if self.eat_keyword("INTERVAL") {
            NAMED_TYPE
        } else {
            self.name_ref(NameRole::Type);
            if self.current() == L_PAREN {
                let params = self.start();
                let opened = self.open_paren();
                if opened {
                    self.comma_list(Self::type_parameter, |_| false);
                }
                self.close_paren(opened);
                params.complete(self, TYPE_PARAMS);
            }
            NAMED_TYPE
        };
        marker.complete(self, kind);
    }

    /// A parameter of a type, such as `10` in `STRING(10)`: a literal or `MAX`.
    fn type_parameter(&mut self) {
        if self.eat_keyword("MAX") {
            return;
        }
        let literal = matches!(self.current(), INTEGER | FLOAT | STRING | BYTES)
            || self.at_keyword("TRUE")
            || self.at_keyword("FALSE");
        if !literal {
            self.error_recover("a literal or `MAX`");
            return;
        }
        self.primary();
    }

    /// The fields of `STRUCT<...>`, after `<` and through `>`. A field is a type, or a name
    /// and a type.
    pub(super) fn struct_fields(&mut self) {
        if self.eat(GT) {
            return;
        }
        loop {
            let named = self.at_ident() && !matches!(self.nth(1), COMMA | GT | L_PAREN | LT);
            let marker = self.start();
            if named {
                self.name(NameRole::Field);
            }
            self.ty();
            marker.complete(self, STRUCT_FIELD);
            if !self.eat(COMMA) {
                break;
            }
        }
        self.expect(GT, "`>`");
    }

    /// Whether the current `>` and the next one are adjacent, which makes `>>`.
    fn adjacent_gt(&self) -> bool {
        self.current() == GT && self.nth(1) == GT && self.adjacent(0)
    }

    /// Whether the current `<` and a `>` after it are adjacent, which makes `<>`.
    fn adjacent_lt_gt(&self) -> bool {
        self.current() == LT && self.nth(1) == GT && self.adjacent(0)
    }

    /// The column of a `PIVOT`'s `FOR`, which binds tighter than its `IN`.
    pub(super) fn pivot_column(&mut self) {
        self.expr_at(BIT_OR_LEVEL);
    }
}
