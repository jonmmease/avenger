//! Queries, the items of `FROM`, and the pipe operators.

use super::{CompletedMarker, Parser};
use crate::{
    diagnostic::DiagnosticCode,
    Expected, NameRole,
    SyntaxKind::{self, *},
};

/// The help for a query that doesn't start with `FROM`.
const FROM_FIRST: &str = "a query starts with FROM, as in `FROM t |> SELECT a`";

/// The pipe operators of GoogleSQL that avenger-sql does not read.
const UNSUPPORTED_OPERATORS: &[&str] = &[
    "RECURSIVE",
    "WINDOW",
    "MATCH_RECOGNIZE",
    "FINISH",
    "DESCRIBE",
    "STATIC_DESCRIBE",
    "ASSERT",
    "LOG",
    "IF",
    "FORK",
    "TEE",
    "EXPORT",
    "CREATE",
    "INSERT",
    "ALIGN",
];

impl Parser<'_> {
    /// `FROM from_clause pipe_operator*`, or `(query) [AS alias] pipe_operator*`.
    pub(super) fn query(&mut self) -> Option<CompletedMarker> {
        if !self.enter() {
            return None;
        }
        let marker = self.start();
        if self.at(L_PAREN) {
            self.parenthesized_query();
            if self.at_keyword("AS") {
                let alias = self.start();
                self.bump_keyword();
                self.name(NameRole::Alias);
                alias.complete(self, ALIAS);
            }
        } else if self.at_keyword("FROM") {
            let from = self.start();
            self.bump_keyword();
            self.sources();
            from.complete(self, FROM_CLAUSE);
        } else {
            let select = self.at_keyword("SELECT");
            let with = self.at_keyword("WITH");
            let diagnostic = self.error_expected("`FROM`");
            if select {
                diagnostic.code = DiagnosticCode::NotPipeSyntax;
                diagnostic.help = Some(format!("avenger-sql reads pipe queries: {FROM_FIRST}"));
            } else if with {
                diagnostic.code = DiagnosticCode::NotPipeSyntax;
                diagnostic.help = Some(
                    "a pipe WITH names queries, as in `FROM t |> WITH u AS (FROM v) |> JOIN u USING (k)`"
                        .to_string(),
                );
            }
            self.skip_until(|p| p.at_sync());
        }
        while self.current() == PIPE {
            self.pipe_operator();
        }
        self.leave();
        Some(marker.complete(self, QUERY))
    }

    /// `(query)`
    pub(super) fn parenthesized_query(&mut self) -> CompletedMarker {
        let marker = self.start();
        let opened = self.open_paren();
        if opened {
            self.query();
        }
        self.close_paren(opened);
        marker.complete(self, PAREN_QUERY)
    }

    /// The sources of `FROM` and their joins. Commas and joins associate to the left, as in
    /// GoogleSQL, so `a, b JOIN c` joins `c` to the cross join of `a` and `b`.
    fn sources(&mut self) {
        let Some(mut item) = self.source() else {
            return;
        };
        let mut after_comma = false;
        loop {
            if self.current() == COMMA {
                let marker = item.precede(self);
                let op = self.start();
                self.bump();
                op.complete(self, JOIN_OP);
                self.source();
                item = marker.complete(self, JOIN);
                after_comma = true;
                continue;
            }
            if !self.at_join() {
                return;
            }
            let marker = item.precede(self);
            let join_start = self.range();
            let kind = self.join_op();
            if after_comma && matches!(kind, Some(RIGHT_KW | FULL_KW)) {
                let range = self.since(join_start);
                self.error(
                    DiagnosticCode::Expected,
                    range,
                    "a RIGHT or FULL JOIN after a comma join needs parentheses",
                )
                .help = Some(
                    "put the comma join in parentheses, as in `FROM (a, b) RIGHT JOIN c ON ...`"
                        .to_string(),
                );
            }
            self.source();
            self.join_condition(kind == Some(CROSS_KW));
            item = marker.complete(self, JOIN);
        }
    }

    /// Whether a join starts at the current token.
    fn at_join(&self) -> bool {
        let offset = usize::from(self.at_keyword("NATURAL"));
        ["JOIN", "INNER", "CROSS", "LEFT", "RIGHT", "FULL"]
            .iter()
            .any(|word| self.keyword_at(offset, word))
            || offset == 1
    }

    /// `[NATURAL] [INNER | CROSS | {LEFT | RIGHT | FULL} [OUTER]] JOIN`, and the kind of its
    /// first keyword after `NATURAL`.
    fn join_op(&mut self) -> Option<SyntaxKind> {
        let marker = self.start();
        let natural = self.eat_keyword("NATURAL");
        let kind = SyntaxKind::keyword(self.nth_text(0));
        match kind {
            Some(JOIN_KW) => {}
            Some(INNER_KW | CROSS_KW) => {
                self.bump_keyword();
            }
            Some(LEFT_KW | RIGHT_KW | FULL_KW) => {
                self.bump_keyword();
                self.eat_keyword("OUTER");
            }
            _ if natural => {
                self.error_expected("a join").help =
                    Some("NATURAL comes before a join, as in `NATURAL JOIN u`".to_string());
            }
            _ => {}
        }
        self.expect_keyword("JOIN");
        marker.complete(self, JOIN_OP);
        kind
    }

    /// `ON expr` or `USING (columns)`, which a cross join doesn't take.
    fn join_condition(&mut self, cross: bool) {
        let marker = self.start();
        let kind = if self.eat_keyword("ON") {
            self.expr();
            ON_CONDITION
        } else if self.eat_keyword("USING") {
            self.name_list(NameRole::Column);
            USING_CONDITION
        } else {
            marker.abandon(self);
            return;
        };
        let node = marker.complete(self, kind);
        if cross {
            let range = self.node_range(node);
            self.error(
                DiagnosticCode::Expected,
                range,
                "a CROSS JOIN takes no condition",
            )
            .help = Some("an INNER JOIN takes ON or USING".to_string());
        }
    }

    /// A source of `FROM` or of a join, with its `TABLESAMPLE` clauses.
    fn source(&mut self) -> Option<CompletedMarker> {
        let mut item = self.source_primary()?;
        while self.at_keyword("TABLESAMPLE") {
            let marker = item.precede(self);
            self.sample();
            item = marker.complete(self, SAMPLE_SOURCE);
        }
        Some(item)
    }

    /// A table, a query in parentheses, `UNNEST`, a table function call, or a join in
    /// parentheses. A table, a query, `UNNEST` and a call take an alias and then one `PIVOT`
    /// or `UNPIVOT`.
    fn source_primary(&mut self) -> Option<CompletedMarker> {
        let marker = self.start();
        let lateral = self.keyword_range("LATERAL");
        let kind = if self.current() == L_PAREN {
            if self.keyword_at(1, "FROM") {
                self.parenthesized_query();
                self.table_alias();
                SUBQUERY_SOURCE
            } else {
                match self.attempt(|p| {
                    p.parenthesized_query();
                }) {
                    Ok(()) => {
                        self.table_alias();
                        SUBQUERY_SOURCE
                    }
                    Err(query_error) if lateral.is_none() => {
                        let checkpoint = self.checkpoint();
                        self.parenthesized_join();
                        let join_error = self
                            .diagnostics
                            .get(checkpoint.diagnostics)
                            .map(|error| error.range.start());
                        match join_error {
                            Some(join_error) if join_error < query_error => {
                                self.rewind(&checkpoint);
                                self.parenthesized_query();
                                self.table_alias();
                                SUBQUERY_SOURCE
                            }
                            _ => {
                                let node = marker.complete(self, PAREN_JOIN);
                                return Some(node);
                            }
                        }
                    }
                    Err(_) => {
                        self.parenthesized_query();
                        self.table_alias();
                        SUBQUERY_SOURCE
                    }
                }
            }
        } else if lateral.is_none() && self.at_keyword("UNNEST") {
            self.bump_keyword();
            self.unnest_arguments();
            self.table_alias();
            self.with_offset();
            UNNEST_SOURCE
        } else {
            self.record_name(NameRole::Table);
            let Some((path, dashed)) = self.table_path() else {
                marker.abandon(self);
                self.error_recover_source();
                return None;
            };
            if self.current() == L_PAREN {
                if dashed {
                    let range = self.node_range(path);
                    self.error(
                        DiagnosticCode::Expected,
                        range,
                        "a table function's name has no dashes",
                    )
                    .help = Some("quote the name in backticks".to_string());
                }
                self.table_arguments();
                self.table_alias();
                TVF_SOURCE
            } else {
                if let Some(lateral) = lateral {
                    self.error(
                        DiagnosticCode::Expected,
                        lateral,
                        "LATERAL comes before a query in parentheses or a table function call",
                    );
                }
                self.table_alias();
                self.with_offset();
                if self.at_keyword("FOR") && self.keyword_at(1, "SYSTEM_TIME") {
                    let range = self.range();
                    self.error(
                        DiagnosticCode::Unsupported,
                        range,
                        "avenger-sql reads no FOR SYSTEM_TIME AS OF",
                    );
                    self.skip_until(|p| p.at_sync() || p.at_join() || p.current() == COMMA);
                }
                TABLE_SOURCE
            }
        };
        let item = marker.complete(self, kind);
        Some(self.pivot_or_unpivot(item))
    }

    /// Reports that no source is at the current token, and skips it when it is not a token
    /// that ends a skip.
    fn error_recover_source(&mut self) {
        if self.at_sync() || self.current() == COMMA {
            return;
        }
        let marker = self.start();
        self.skip_balanced();
        marker.complete(self, ERROR);
    }

    /// `(join)`, a join in parentheses, which holds a join and no commas.
    fn parenthesized_join(&mut self) {
        let opened = self.open_paren();
        if opened {
            if let Some(mut item) = self.source() {
                let mut joined = false;
                while self.at_join() {
                    let marker = item.precede(self);
                    let kind = self.join_op();
                    self.source();
                    self.join_condition(kind == Some(CROSS_KW));
                    item = marker.complete(self, JOIN);
                    joined = true;
                }
                if !joined {
                    self.error_expected("a JOIN").help = Some(
                        "parentheses in FROM hold a join or a query, as in `(a JOIN b USING (k))`"
                            .to_string(),
                    );
                }
            }
        }
        self.close_paren(opened);
    }

    /// `(array [[AS] alias], ... [, mode => value])` of `UNNEST`.
    fn unnest_arguments(&mut self) {
        let opened = self.open_paren();
        if opened {
            loop {
                if self.at_ident() && self.nth(1) == FAT_ARROW {
                    let marker = self.start();
                    if !self.nth_text(0).eq_ignore_ascii_case("mode") {
                        let range = self.range();
                        self.error(
                            DiagnosticCode::Expected,
                            range,
                            "UNNEST takes one named argument, `mode`",
                        );
                    }
                    self.name_ref(NameRole::Parameter);
                    self.bump();
                    self.expr();
                    marker.complete(self, NAMED_ARG);
                    break;
                }
                self.aliased_expr();
                if !self.eat(COMMA) {
                    break;
                }
            }
        }
        self.close_paren(opened);
    }

    /// `[WITH OFFSET [[AS] alias]]`
    fn with_offset(&mut self) {
        if !(self.at_keyword("WITH") && self.keyword_at(1, "OFFSET")) {
            return;
        }
        let marker = self.start();
        self.eat_keywords(&["WITH", "OFFSET"]);
        self.alias(NameRole::Alias);
        marker.complete(self, WITH_OFFSET);
    }

    /// `item` and the `PIVOT` or `UNPIVOT` after it, when it has one.
    fn pivot_or_unpivot(&mut self, item: CompletedMarker) -> CompletedMarker {
        let kind = if self.pivot_ahead() {
            let marker = item.precede(self);
            self.pivot();
            (marker, PIVOT_SOURCE)
        } else if self.unpivot_ahead() {
            let marker = item.precede(self);
            self.unpivot();
            (marker, UNPIVOT_SOURCE)
        } else {
            return item;
        };
        kind.0.complete(self, kind.1)
    }

    /// A table name, whose first part can hold single dashes, as in `my-project.dataset.table`,
    /// when no space separates the dashes from the words, and whether it has dashes. As in
    /// GoogleSQL, a part after a dash is a name that isn't reserved, or a number, and
    /// `my-project-2.dataset` holds the float `2.`.
    pub(super) fn table_path(&mut self) -> Option<(CompletedMarker, bool)> {
        if !self.at_ident() {
            return self.path(NameRole::Table).map(|(path, _)| (path, false));
        }
        let path = self.start();
        let first = self.start();
        let quoted = self.current() == QUOTED_IDENT;
        self.bump();
        let mut dashed = false;
        if !quoted {
            while self.current() == MINUS && self.touches_previous() && self.adjacent(0) {
                let next_kind = self.nth(1);
                let next_text = self.nth_text(1);
                let ok = match next_kind {
                    WORD => !super::is_reserved(next_text),
                    INTEGER => true,
                    FLOAT => {
                        next_text.ends_with('.')
                            && next_text[..next_text.len() - 1]
                                .bytes()
                                .all(|b| b.is_ascii_digit())
                    }
                    _ => false,
                };
                if !ok {
                    break;
                }
                dashed = true;
                self.bump();
                self.bump();
                if next_kind == FLOAT {
                    // `2.` ends the first part, and the word after it starts the second.
                    first.complete(self, NAME_REF);
                    if self.current() == WORD && self.touches_previous() {
                        let second = self.start();
                        self.bump();
                        second.complete(self, NAME_REF);
                    }
                    let (path, _) = self.path_rest(path, 2);
                    return Some((path, dashed));
                }
            }
        }
        first.complete(self, NAME_REF);
        let (path, _) = self.path_rest(path, 1);
        Some((path, dashed))
    }

    /// The alias of a source. Without `AS`, `PIVOT` and `UNPIVOT` start operators, not aliases.
    fn table_alias(&mut self) {
        if self.pivot_ahead() || self.unpivot_ahead() {
            return;
        }
        self.alias(NameRole::Alias);
    }

    fn pivot_ahead(&self) -> bool {
        self.at_keyword("PIVOT") && self.nth(1) == L_PAREN
    }

    fn unpivot_ahead(&self) -> bool {
        self.at_keyword("UNPIVOT")
            && (self.nth(1) == L_PAREN
                || self.keyword_at(1, "INCLUDE")
                || self.keyword_at(1, "EXCLUDE"))
    }

    /// `|> operator`
    fn pipe_operator(&mut self) {
        let marker = self.start();
        self.bump();
        self.record(Expected::PipeOperator);
        let word = match self.current() {
            WORD => self.nth_text(0).to_ascii_uppercase(),
            _ => String::new(),
        };
        let kind = self.operator(&word);
        match kind {
            Some(kind) => {
                marker.complete(self, kind);
            }
            None => {
                marker.complete(self, ERROR);
            }
        }
        if !self.at_operator_end() {
            self.error_expected("`|>`, `;` or the end of the query");
            self.skip_until(|p| p.at_sync());
        }
    }

    /// The operator after `|>`, and the kind of its node.
    fn operator(&mut self, word: &str) -> Option<SyntaxKind> {
        let kind = match word {
            "SELECT" => {
                self.bump_keyword();
                if self.at_keyword("AS") {
                    let range = self.range();
                    self.error(
                        DiagnosticCode::Unsupported,
                        range,
                        "avenger-sql has no value tables, so no SELECT AS STRUCT or AS VALUE",
                    );
                    self.skip_until(|p| p.at_sync());
                    return Some(PIPE_SELECT);
                }
                if !self.eat_keyword("DISTINCT") {
                    self.eat_keyword("ALL");
                }
                self.select_list();
                self.window_clause();
                PIPE_SELECT
            }
            "EXTEND" => {
                self.bump_keyword();
                if self.current() == STAR {
                    let range = self.range();
                    self.error(
                        DiagnosticCode::Expected,
                        range,
                        "a pipe EXTEND adds columns, so it takes no `*`",
                    );
                }
                self.select_list();
                self.window_clause();
                PIPE_EXTEND
            }
            "SET" => {
                self.bump_keyword();
                self.comma_list(
                    |p| {
                        let marker = p.start();
                        p.name_ref(NameRole::Column);
                        p.expect(EQ, "`=`");
                        p.expr();
                        marker.complete(p, ASSIGNMENT);
                    },
                    Self::at_operator_end,
                );
                PIPE_SET
            }
            "DROP" => {
                self.bump_keyword();
                self.comma_list(
                    |p| {
                        p.name_ref(NameRole::Column);
                    },
                    Self::at_operator_end,
                );
                PIPE_DROP
            }
            "RENAME" => {
                self.bump_keyword();
                self.comma_list(
                    |p| {
                        let marker = p.start();
                        p.name_ref(NameRole::Column);
                        p.eat_keyword("AS");
                        p.name(NameRole::Alias);
                        marker.complete(p, RENAME_ITEM);
                    },
                    Self::at_operator_end,
                );
                PIPE_RENAME
            }
            "AS" => {
                self.bump_keyword();
                self.name(NameRole::Alias);
                PIPE_AS
            }
            "WHERE" => {
                self.bump_keyword();
                self.expr();
                PIPE_WHERE
            }
            "AGGREGATE" => {
                self.aggregate();
                PIPE_AGGREGATE
            }
            "DISTINCT" => {
                self.bump_keyword();
                PIPE_DISTINCT
            }
            "ORDER" => {
                self.order_by(Self::at_operator_end);
                PIPE_ORDER_BY
            }
            "LIMIT" => {
                self.bump_keyword();
                self.expr();
                if self.at_keyword("OFFSET") {
                    let marker = self.start();
                    self.bump_keyword();
                    self.expr();
                    marker.complete(self, LIMIT_OFFSET);
                }
                PIPE_LIMIT
            }
            "UNION" | "INTERSECT" | "EXCEPT" => {
                self.set_operation(None);
                PIPE_SET_OPERATION
            }
            "INNER" | "LEFT" | "FULL" | "OUTER"
                if !(self.keyword_at(1, "JOIN")
                    || self.keyword_at(1, "OUTER") && self.keyword_at(2, "JOIN")) =>
            {
                let mode = self.start();
                self.bump_keyword();
                if word != "INNER" && word != "OUTER" {
                    self.eat_keyword("OUTER");
                }
                let mode = mode.complete(self, SET_OP_MODE);
                self.set_operation(Some(mode));
                PIPE_SET_OPERATION
            }
            "JOIN" | "INNER" | "LEFT" | "RIGHT" | "FULL" | "CROSS" | "NATURAL" => {
                let kind = self.join_op();
                self.source();
                self.join_condition(kind == Some(CROSS_KW));
                PIPE_JOIN
            }
            "CALL" => {
                self.bump_keyword();
                self.path(NameRole::Function);
                self.table_arguments();
                self.alias(NameRole::Alias);
                PIPE_CALL
            }
            "TABLESAMPLE" => {
                self.sample();
                PIPE_SAMPLE
            }
            "WITH" => {
                self.bump_keyword();
                if self.at_keyword("RECURSIVE") {
                    let range = self.range();
                    self.error(
                        DiagnosticCode::Unsupported,
                        range,
                        "avenger-sql has no recursive pipe WITH",
                    );
                    self.skip_until(|p| p.at_sync());
                    return Some(PIPE_WITH);
                }
                self.comma_list(
                    |p| {
                        let marker = p.start();
                        p.name(NameRole::Query);
                        p.expect_keyword("AS");
                        p.parenthesized_query();
                        marker.complete(p, NAMED_QUERY);
                    },
                    Self::at_operator_end,
                );
                PIPE_WITH
            }
            "PIVOT" => {
                self.pivot();
                PIPE_PIVOT
            }
            "UNPIVOT" => {
                self.unpivot();
                PIPE_UNPIVOT
            }
            "GROUP" => {
                let range = self.range();
                self.error(
                    DiagnosticCode::Expected,
                    range,
                    "GROUP BY goes in a pipe AGGREGATE",
                )
                .help = Some("write `|> AGGREGATE COUNT(*) GROUP BY k`".to_string());
                self.skip_until(|p| p.at_sync());
                return None;
            }
            _ if UNSUPPORTED_OPERATORS.contains(&word) => {
                let range = self.range();
                self.error(
                    DiagnosticCode::Unsupported,
                    range,
                    format!("avenger-sql has no pipe {word}"),
                );
                self.skip_until(|p| p.at_sync());
                return None;
            }
            _ => {
                self.error_expected("a pipe operator");
                self.skip_until(|p| p.at_sync());
                return None;
            }
        };
        Some(kind)
    }

    /// The items of a pipe `SELECT` or `EXTEND`.
    fn select_list(&mut self) {
        self.comma_list(Self::select_item, |p| {
            p.at_operator_end() || p.at_keyword("WINDOW")
        });
    }

    fn select_item(&mut self) {
        let marker = self.start();
        if self.current() != STAR && self.qualified_star_ahead().is_none() {
            self.expr();
            self.alias(NameRole::Alias);
            marker.complete(self, SELECT_ITEM);
            return;
        }
        if self.current() != STAR {
            self.path(NameRole::Table);
            self.expect(DOT, "`.`");
        }
        self.expect(STAR, "`*`");
        if self.at_keyword("EXCEPT") {
            let except = self.start();
            self.bump_keyword();
            self.name_list(NameRole::Column);
            except.complete(self, EXCEPT_LIST);
        }
        if self.at_keyword("REPLACE") && self.nth(1) == L_PAREN {
            let replace = self.start();
            self.bump_keyword();
            let opened = self.open_paren();
            if opened {
                self.comma_list(
                    |p| {
                        let item = p.start();
                        p.expr();
                        p.expect_keyword("AS");
                        p.name(NameRole::Alias);
                        item.complete(p, REPLACE_ITEM);
                    },
                    |_| false,
                );
            }
            self.close_paren(opened);
            replace.complete(self, REPLACE_LIST);
        }
        marker.complete(self, WILDCARD);
    }

    /// The number of names before `.*` when the current tokens are `name(.name)*.*`.
    fn qualified_star_ahead(&self) -> Option<usize> {
        if !self.at_ident() {
            return None;
        }
        let mut offset = 1;
        loop {
            if self.nth(offset) != DOT {
                return None;
            }
            match self.nth(offset + 1) {
                STAR => return Some(offset.div_ceil(2)),
                WORD | QUOTED_IDENT => offset += 2,
                _ => return None,
            }
        }
    }

    /// `[WINDOW name AS window, ...]`
    fn window_clause(&mut self) {
        if !self.at_keyword("WINDOW") {
            return;
        }
        let marker = self.start();
        self.bump_keyword();
        self.comma_list(
            |p| {
                let window = p.start();
                p.name(NameRole::Window);
                p.expect_keyword("AS");
                p.window_ref();
                window.complete(p, NAMED_WINDOW);
            },
            Self::at_operator_end,
        );
        marker.complete(self, WINDOW_CLAUSE);
    }

    /// `AGGREGATE [items] [GROUP [AND ORDER] BY items]`, where either list can be empty.
    fn aggregate(&mut self) {
        self.bump_keyword();
        let at_group = |p: &Self| p.at_keyword("GROUP") || p.at_operator_end();
        if !at_group(self) {
            self.comma_list(
                |p| {
                    let marker = p.start();
                    p.expr();
                    p.alias(NameRole::Alias);
                    p.order_suffix(false);
                    marker.complete(p, AGGREGATE_ITEM);
                },
                at_group,
            );
        }
        if self.at_keyword("GROUP") {
            let marker = self.start();
            self.bump_keyword();
            if self.at_keyword("AND") && self.keyword_at(1, "ORDER") {
                self.bump_keyword();
                self.bump_keyword();
            }
            self.expect_keyword("BY");
            self.comma_list(Self::grouping_item, Self::at_operator_end);
            marker.complete(self, GROUP_BY);
        }
    }

    /// An item of the `GROUP BY` of a pipe `AGGREGATE`: `()`, `ROLLUP`, `CUBE`, `GROUPING SETS`,
    /// or an expression with an alias and an order suffix.
    fn grouping_item(&mut self) {
        if self.grouping_base() {
            return;
        }
        if self.at_keyword("GROUPING") && self.keyword_at(1, "SETS") {
            let marker = self.start();
            self.eat_keywords(&["GROUPING", "SETS"]);
            let opened = self.open_paren();
            if opened {
                self.comma_list(
                    |p| {
                        if !p.grouping_base() {
                            p.expr();
                        }
                    },
                    |_| false,
                );
            }
            self.close_paren(opened);
            marker.complete(self, GROUPING_SETS);
            return;
        }
        let marker = self.start();
        self.expr();
        self.alias(NameRole::Alias);
        self.order_suffix(true);
        marker.complete(self, GROUP_ITEM);
    }

    /// `()`, `ROLLUP (exprs)` or `CUBE (exprs)`, when one is at the current token.
    fn grouping_base(&mut self) -> bool {
        if self.current() == L_PAREN && self.nth(1) == R_PAREN {
            let marker = self.start();
            self.bump();
            self.bump();
            marker.complete(self, GROUPING_EMPTY);
            return true;
        }
        let rollup = self.at_keyword("ROLLUP");
        if !rollup && !self.at_keyword("CUBE") {
            return false;
        }
        let marker = self.start();
        self.bump_keyword();
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
        marker.complete(self, if rollup { ROLLUP } else { CUBE });
        true
    }

    /// `{ASC | DESC} [NULLS {FIRST | LAST}]` after an item of a pipe `AGGREGATE`. A group item
    /// can also take `NULLS {FIRST | LAST}` alone, when `nulls_alone`.
    fn order_suffix(&mut self, nulls_alone: bool) {
        let direction = self.at_keyword("ASC") || self.at_keyword("DESC");
        if !(direction || nulls_alone && self.at_keyword("NULLS")) {
            return;
        }
        let marker = self.start();
        if direction {
            self.bump_keyword();
        }
        self.nulls();
        marker.complete(self, ORDER_SUFFIX);
    }

    /// `set_operator [BY NAME | CORRESPONDING] (query), ...`, after the mode.
    fn set_operation(&mut self, mode: Option<CompletedMarker>) {
        let operator = self.nth_text(0).to_ascii_uppercase();
        if !matches!(operator.as_str(), "UNION" | "INTERSECT" | "EXCEPT") {
            self.error_expected("`UNION`, `INTERSECT` or `EXCEPT`");
        } else {
            self.bump_keyword();
        }
        if self.at_keyword("ALL") || self.at_keyword("DISTINCT") {
            self.bump_keyword();
        } else {
            self.error_expected("`ALL` or `DISTINCT`").help = Some(format!(
                "GoogleSQL writes `{operator} ALL` or `{operator} DISTINCT`"
            ));
        }
        if self.at_keyword("BY") && self.keyword_at(1, "NAME") {
            let marker = self.start();
            self.eat_keywords(&["BY", "NAME"]);
            if self.eat_keyword("ON") {
                self.name_list(NameRole::Column);
            }
            marker.complete(self, BY_NAME);
        } else if self.at_keyword("STRICT") || self.at_keyword("CORRESPONDING") {
            let marker = self.start();
            if let Some(strict) = self.keyword_range("STRICT") {
                if mode.is_some() {
                    self.error(
                        DiagnosticCode::Expected,
                        strict,
                        "STRICT can't go with the mode of a set operation",
                    );
                }
            }
            self.expect_keyword("CORRESPONDING");
            if self.eat_keyword("BY") {
                self.name_list(NameRole::Column);
            }
            marker.complete(self, CORRESPONDING);
        } else if let Some(mode) = mode {
            let range = self.node_range(mode);
            self.error(
                DiagnosticCode::Expected,
                range,
                "the mode of a set operation goes with BY NAME or CORRESPONDING",
            );
        }
        self.comma_list(
            |p| {
                p.parenthesized_query();
            },
            Self::at_operator_end,
        );
    }

    /// `TABLESAMPLE method (size {ROWS | PERCENT} [PARTITION BY exprs]) [REPEATABLE (seed)]
    /// [WITH WEIGHT [[AS] alias] [REPEATABLE (seed)]]`
    fn sample(&mut self) {
        let marker = self.start();
        self.bump_keyword();
        self.name_ref(NameRole::Other);
        let opened = self.open_paren();
        if opened {
            let size = self.start();
            self.sample_number(true);
            match self.at_keyword("ROWS") || self.at_keyword("PERCENT") {
                true => {
                    self.bump_keyword();
                }
                false => {
                    self.error_expected("`ROWS` or `PERCENT`");
                }
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
            size.complete(self, SAMPLE_SIZE);
        }
        self.close_paren(opened);
        let repeatable = self.repeatable();
        if !repeatable && self.at_keyword("WITH") && self.keyword_at(1, "WEIGHT") {
            let weight = self.start();
            self.eat_keywords(&["WITH", "WEIGHT"]);
            if self.at_keyword("AS") || (self.at_ident() && !self.at_keyword("REPEATABLE")) {
                self.alias(NameRole::Alias);
            }
            weight.complete(self, WEIGHT);
            self.repeatable();
        }
        marker.complete(self, SAMPLE_CLAUSE);
    }

    /// `[REPEATABLE (seed)]`
    fn repeatable(&mut self) -> bool {
        if !(self.at_keyword("REPEATABLE") && self.nth(1) == L_PAREN) {
            return false;
        }
        let marker = self.start();
        self.bump_keyword();
        let opened = self.open_paren();
        if opened {
            self.sample_number(false);
        }
        self.close_paren(opened);
        marker.complete(self, REPEATABLE);
        true
    }

    /// A number of `TABLESAMPLE`: an integer, or a float when `float`, a query parameter, or a
    /// `CAST` of one of them.
    fn sample_number(&mut self, float: bool) {
        let fits = match self.current() {
            INTEGER | PARAM | POSITIONAL => true,
            FLOAT => float,
            _ => self.at_keyword("CAST") && self.nth(1) == L_PAREN,
        };
        if !fits {
            let what = match float {
                true => "a number, a query parameter, or a CAST of one",
                false => "an integer, a query parameter, or a CAST of one",
            };
            self.error_recover(what);
            return;
        }
        self.primary();
    }

    /// `PIVOT (aggregates FOR column IN (values)) [alias]`
    fn pivot(&mut self) {
        let marker = self.start();
        self.bump_keyword();
        let opened = self.open_paren();
        if opened {
            self.comma_list(Self::aliased_expr, |_| false);
            self.expect_keyword("FOR");
            self.pivot_column();
            self.expect_keyword("IN");
            let values = self.open_paren();
            if values {
                self.comma_list(Self::aliased_expr, |_| false);
            }
            self.close_paren(values);
        }
        self.close_paren(opened);
        self.table_alias();
        marker.complete(self, PIVOT_CLAUSE);
    }

    /// An expression and its alias.
    pub(super) fn aliased_expr(&mut self) {
        let marker = self.start();
        self.expr();
        self.alias(NameRole::Alias);
        marker.complete(self, ALIASED_EXPR);
    }

    /// `UNPIVOT [{INCLUDE | EXCLUDE} NULLS] (values FOR name IN (columns)) [alias]`
    fn unpivot(&mut self) {
        let marker = self.start();
        self.bump_keyword();
        if !self.eat_keywords(&["INCLUDE", "NULLS"]) {
            self.eat_keywords(&["EXCLUDE", "NULLS"]);
        }
        let opened = self.open_paren();
        if opened {
            self.path_or_paths();
            self.expect_keyword("FOR");
            self.path(NameRole::Column);
            self.expect_keyword("IN");
            let columns = self.open_paren();
            if columns {
                self.comma_list(
                    |p| {
                        let item = p.start();
                        p.path_or_paths();
                        let as_range = p.range();
                        let has_as = p.eat_keyword("AS");
                        match p.current() {
                            STRING | INTEGER => {
                                p.primary();
                            }
                            _ if has_as => {
                                let range = as_range.cover(p.range());
                                p.error(
                                    DiagnosticCode::Expected,
                                    range,
                                    "an UNPIVOT label is a string or an integer",
                                );
                                if !p.at_sync() && !matches!(p.current(), COMMA | R_PAREN) {
                                    p.bump();
                                }
                            }
                            _ => {}
                        }
                        item.complete(p, UNPIVOT_COLUMNS);
                    },
                    |_| false,
                );
            }
            self.close_paren(columns);
        }
        self.close_paren(opened);
        self.table_alias();
        marker.complete(self, UNPIVOT_CLAUSE);
    }

    /// A path, or paths in parentheses.
    fn path_or_paths(&mut self) {
        if self.current() != L_PAREN {
            self.path(NameRole::Column);
            return;
        }
        let marker = self.start();
        let opened = self.open_paren();
        if opened {
            self.comma_list(
                |p| {
                    p.path(NameRole::Column);
                },
                |_| false,
            );
        }
        self.close_paren(opened);
        marker.complete(self, NAME_LIST);
    }
}
