//! Scripts and modules: statements separated by `;`. A script's statement is a pipe query,
//! `IMPORT MODULE`, `DECLARE`, `SET`, `CREATE TEMP TABLE`, or `CREATE FUNCTION` of a scalar,
//! aggregate or table function. A module, a `.sqlm` file, starts with `MODULE name` and holds
//! `IMPORT MODULE` and `CREATE PUBLIC` or `CREATE PRIVATE` statements of functions, table
//! functions and constants, following GoogleSQL's modules.

use super::Parser;
use crate::{diagnostic::DiagnosticCode, Expected, FunctionRole, NameRole, SyntaxKind::*};

/// The first keywords of GoogleSQL's statements that avenger-sql doesn't run: procedural
/// statements and blocks, DDL, DML, transactions and the rest.
const UNSUPPORTED_STATEMENTS: &[&str] = &[
    // Procedural statements and blocks.
    "BEGIN", "IF", "LOOP", "REPEAT", "WHILE", "FOR", "BREAK", "LEAVE", "CONTINUE", "ITERATE",
    "RETURN", "RAISE", "EXECUTE", "CALL", "CASE", // Transactions and batches.
    "START", "COMMIT", "ROLLBACK", "RUN", "ABORT", // DDL.
    "ALTER", "DROP", "UNDROP", "RENAME", "TRUNCATE", "GRANT", "REVOKE", "DEFINE", "CLONE",
    // DML.
    "INSERT", "UPDATE", "DELETE", "MERGE", // The rest.
    "EXPORT", "LOAD", "ASSERT", "DESCRIBE", "DESC", "SHOW", "EXPLAIN", "ANALYZE", "GRAPH",
];

/// The first keywords of GoogleSQL's blocks, which end at a matching `END`, where they start a
/// statement. A `CASE` expression ends at an `END` too, so a `CASE` anywhere opens one.
const BLOCKS: &[&str] = &["BEGIN", "IF", "LOOP", "WHILE", "FOR", "REPEAT", "CASE"];

/// The keywords after which a statement starts, inside a block.
const STATEMENT_STARTS: &[&str] = &["THEN", "ELSE", "DO", "BEGIN", "LOOP", "REPEAT"];

/// The kind of object that a `CREATE` statement creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Object {
    Table,
    Function(FunctionKind),
    Constant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FunctionKind {
    Scalar,
    Aggregate,
    Table,
}

/// The help of a statement that a script doesn't hold.
const SCRIPT_HOLDS: &str =
    "a script holds pipe queries, IMPORT MODULE, DECLARE, SET, CREATE TEMP TABLE and CREATE TEMP FUNCTION";

impl Parser<'_> {
    /// A script, which, as in GoogleSQL, can hold no statements.
    pub(super) fn script(&mut self) {
        let marker = self.start();
        self.statements();
        marker.complete(self, SCRIPT);
    }

    /// A module: `MODULE name [OPTIONS (...)]` and the statements after it.
    pub(super) fn module(&mut self) {
        let marker = self.start();
        let statement = self.start();
        if self.eat_keyword("MODULE") {
            self.path(NameRole::Module);
            self.options();
        } else {
            self.error_expected("`MODULE`").help =
                Some("a module starts with its name, as in `MODULE charts.scales;`".to_string());
        }
        statement.complete(self, MODULE_STMT);
        self.in_module = true;
        if !self.eat(SEMICOLON) && !self.at_eof() {
            self.error_expected("`;` or the end of the module");
            self.skip_until(|p| p.current() == SEMICOLON);
            self.eat(SEMICOLON);
        }
        self.statements();
        marker.complete(self, MODULE);
    }

    /// The statements up to the end, separated by `;`. A statement starts at the start of the
    /// text or after a `;`.
    fn statements(&mut self) {
        let place = match self.in_module {
            true => "module",
            false => "script",
        };
        loop {
            self.record(Expected::Statement);
            if self.at_eof() {
                break;
            }
            let start = self.position;
            let statement = self.statement();
            let query = matches!(statement.map(|s| s.kind()), Some(QUERY));
            if self.eat(SEMICOLON) {
                continue;
            }
            if self.at_eof() {
                break;
            }
            let expected = match query {
                true => format!("`|>`, `;` or the end of the {place}"),
                false => format!("`;` or the end of the {place}"),
            };
            self.error_expected(&expected);
            let saved = self.paren_depth;
            self.paren_depth = 0;
            self.skip_until(|p| p.current() == SEMICOLON);
            self.paren_depth = saved;
            debug_assert!(self.position > start || self.at_eof());
            if !self.eat(SEMICOLON) {
                break;
            }
        }
    }

    fn statement(&mut self) -> Option<super::CompletedMarker> {
        if self.at_keyword("MODULE") {
            let message = match self.in_module {
                true => "a module has one MODULE statement, its first",
                false => "a MODULE statement starts a module, not a script",
            };
            let range = self.range();
            // GoogleSQL takes a MODULE statement in a script, and avenger reads it in a module.
            let code = match self.in_module {
                true => DiagnosticCode::InvalidStatement,
                false => DiagnosticCode::Unsupported,
            };
            self.error(code, range, message);
            self.skip_until(|p| p.current() == SEMICOLON);
            return None;
        }
        if self.in_module && !self.at_keyword("IMPORT") && !self.at_keyword("CREATE") {
            let range = self.range();
            self.error(
                DiagnosticCode::InvalidStatement,
                range,
                "a module holds IMPORT MODULE and CREATE PUBLIC or PRIVATE statements",
            )
            .help(
                "functions, table functions and constants go in a module, and queries in a script",
            );
            self.skip_until(|p| p.current() == SEMICOLON);
            return None;
        }
        if self.at_keyword("IMPORT") {
            return Some(self.import());
        }
        if self.at_keyword("DECLARE") {
            return Some(self.declare());
        }
        if self.at_keyword("SET") {
            return Some(self.set_variable());
        }
        if self.at_keyword("CREATE") {
            return Some(self.create());
        }
        // A statement of GoogleSQL's that avenger-sql doesn't run, with or without a label.
        let labeled = self.nth(0) == WORD && self.nth(1) == COLON;
        let first = if labeled { 2 } else { 0 };
        if let Some(word) =
            (UNSUPPORTED_STATEMENTS.iter()).find(|word| self.keyword_at(first, word))
        {
            let range = self.range();
            self.error(
                DiagnosticCode::Unsupported,
                range,
                format!(
                    "avenger-sql's scripts have no {} statement",
                    word.to_ascii_uppercase()
                ),
            )
            .help(SCRIPT_HOLDS);
            self.skip_statement();
            return None;
        }
        self.query()
    }

    /// Skips a statement into an `ERROR` node, through its `;` or the end, and through the `END`
    /// of each block that it opens, so the statements of a block skip with it.
    pub(super) fn skip_statement(&mut self) {
        let marker = self.start();
        let saved = self.paren_depth;
        self.paren_depth = 0;
        let mut depth = 0usize;
        let mut statement_start = true;
        let labeled = self.nth(0) == WORD && self.nth(1) == COLON;
        if labeled {
            self.bump();
            self.bump();
        }
        while !self.at_eof() {
            if self.current() == SEMICOLON && depth == 0 {
                break;
            }
            let opens = match statement_start {
                // `BEGIN TRANSACTION` and a lone `BEGIN` start a transaction, not a block.
                true if self.keyword_at(0, "BEGIN") => {
                    !(self.keyword_at(1, "TRANSACTION") || matches!(self.nth(1), SEMICOLON | EOF))
                }
                true => BLOCKS.iter().any(|word| self.keyword_at(0, word)),
                false => self.keyword_at(0, "CASE"),
            };
            if opens {
                depth += 1;
            } else if self.keyword_at(0, "END") && depth > 0 {
                depth -= 1;
                // The keyword of an `END IF` or `END LOOP` is part of the `END`.
                if (["IF", "LOOP", "WHILE", "FOR", "REPEAT", "CASE"].iter())
                    .any(|word| self.keyword_at(1, word))
                {
                    self.bump();
                }
            }
            statement_start = self.current() == SEMICOLON
                || STATEMENT_STARTS.iter().any(|word| self.keyword_at(0, word));
            self.skip_balanced();
            // A label of a statement in a block.
            if statement_start && self.nth(0) == WORD && self.nth(1) == COLON {
                self.bump();
                self.bump();
            }
        }
        self.paren_depth = saved;
        marker.complete(self, ERROR);
    }

    /// `IMPORT MODULE path [AS alias] [OPTIONS (...)]`
    fn import(&mut self) -> super::CompletedMarker {
        let marker = self.start();
        self.bump_keyword();
        if self.at_keyword("PROTO") {
            let range = self.range();
            self.error(
                DiagnosticCode::Unsupported,
                range,
                "avenger-sql has no protocol buffers, so no IMPORT PROTO",
            );
            self.skip_until(|p| p.current() == SEMICOLON);
            return marker.complete(self, IMPORT_STMT);
        }
        self.expect_keyword("MODULE");
        if self.current() == STRING {
            let range = self.range();
            self.error(
                DiagnosticCode::InvalidStatement,
                range,
                "an IMPORT MODULE names its module by a path, as in `charts.scales`",
            );
            let error = self.start();
            self.bump();
            error.complete(self, ERROR);
        } else {
            self.path(NameRole::Module);
        }
        if self.at_keyword("INTO") {
            let range = self.range();
            self.error(
                DiagnosticCode::InvalidStatement,
                range,
                "an IMPORT MODULE names its alias with AS, not INTO",
            );
            let error = self.start();
            self.bump_keyword();
            self.name(NameRole::Alias);
            error.complete(self, ERROR);
        } else if self.at_keyword("AS") {
            let alias = self.start();
            self.bump_keyword();
            self.name(NameRole::Alias);
            alias.complete(self, ALIAS);
        }
        self.options();
        marker.complete(self, IMPORT_STMT)
    }

    /// `DECLARE names [type] [DEFAULT value]`
    fn declare(&mut self) -> super::CompletedMarker {
        let marker = self.start();
        self.bump_keyword();
        self.comma_list(|p| p.name(NameRole::Variable));
        let at_end = matches!(self.current(), SEMICOLON | EOF);
        // The type is optional, so it can start where the text ends.
        self.record(Expected::Type);
        if !self.at_keyword("DEFAULT") && !at_end {
            self.ty();
        }
        if self.eat_keyword("DEFAULT") {
            self.expr();
        }
        marker.complete(self, DECLARE_STMT)
    }

    /// `SET name = value` or `SET (names) = value`
    fn set_variable(&mut self) -> super::CompletedMarker {
        let marker = self.start();
        self.bump_keyword();
        if self.current() == L_PAREN {
            self.name_list(NameRole::Variable);
        } else if self.current() == PARAM {
            let range = self.range();
            self.error(
                DiagnosticCode::Unsupported,
                range,
                "SET assigns a variable of the script, not a query parameter",
            );
            self.bump();
        } else if self.keyword_at(0, "TRANSACTION") {
            let range = self.range();
            self.error(
                DiagnosticCode::Unsupported,
                range,
                "avenger-sql's scripts have no transactions",
            );
            self.skip_statement();
            return marker.complete(self, ERROR);
        } else {
            let start = self.range();
            self.name_ref(NameRole::Variable);
            // Several names take parentheses. As GoogleSQL does, the error is at the names, once
            // the `=` after them shows an assignment.
            if self.current() == COMMA {
                let error = self.start();
                while self.eat(COMMA) {
                    self.name_ref(NameRole::Variable);
                }
                error.complete(self, ERROR);
                if self.current() == EQ {
                    let range = start.cover(self.previous_range());
                    self.error(
                        DiagnosticCode::InvalidStatement,
                        range,
                        "a SET of several variables takes them in parentheses, as in `SET (a, b) = (1, 2)`",
                    );
                }
            }
        }
        self.expect(EQ, "`=`");
        self.expr();
        marker.complete(self, SET_STMT)
    }

    /// A `CREATE` statement. The validation pass checks its modifiers against its object and
    /// against the scripts and modules.
    fn create(&mut self) -> super::CompletedMarker {
        let marker = self.start();
        self.bump_keyword();
        self.eat_keywords(&["OR", "REPLACE"]);
        // A script creates temporary objects and a module public or private ones, as the
        // validation pass checks, so the parser expects the scopes of the text that it reads.
        let scopes: [(&str, bool); 4] = [
            ("TEMP", !self.in_module),
            ("TEMPORARY", !self.in_module),
            ("PUBLIC", self.in_module),
            ("PRIVATE", self.in_module),
        ];
        if let Some((scope, _)) = (scopes.iter()).find(|(word, expected)| {
            (*expected && self.at_keyword(word)) || self.keyword_at(0, word)
        }) {
            self.eat_keyword(scope);
        }
        let object = if self.eat_keywords(&["AGGREGATE", "FUNCTION"]) {
            Some(Object::Function(FunctionKind::Aggregate))
        } else if self.at_keywords(&["TABLE", "FUNCTION"]) {
            // As in GoogleSQL, `TABLE FUNCTION` is a table function, and a table named `function`
            // takes backticks.
            self.eat_keywords(&["TABLE", "FUNCTION"]);
            Some(Object::Function(FunctionKind::Table))
        } else if self.eat_keyword("FUNCTION") {
            Some(Object::Function(FunctionKind::Scalar))
        } else if self.eat_keyword("TABLE") {
            Some(Object::Table)
        } else if (self.in_module && self.at_keyword("CONSTANT")) || self.keyword_at(0, "CONSTANT")
        {
            // A constant belongs to a module, so a script doesn't expect one.
            self.bump_keyword();
            Some(Object::Constant)
        } else if self.current() == WORD {
            // An object of GoogleSQL's that avenger-sql doesn't create, such as a view.
            let range = self.range();
            let mut words = vec![self.nth_text(0).to_ascii_uppercase()];
            for n in 1..3 {
                let word = self.nth_text(n).to_ascii_uppercase();
                if self.nth(n) != WORD || matches!(word.as_str(), "IF" | "AS") {
                    break;
                }
                // The objects of two words, such as `MATERIALIZED VIEW`.
                if !matches!(
                    words[0].as_str(),
                    "MATERIALIZED"
                        | "EXTERNAL"
                        | "ROW"
                        | "SEARCH"
                        | "VECTOR"
                        | "AI"
                        | "UNIQUE"
                        | "APPROX"
                        | "PROPERTY"
                        | "SNAPSHOT"
                        | "PRIVILEGE"
                        | "LOCALITY"
                ) || n > 1
                {
                    break;
                }
                words.push(word);
            }
            self.error(
                DiagnosticCode::Unsupported,
                range,
                format!("avenger-sql's scripts create no {}", words.join(" ")),
            )
            .help(SCRIPT_HOLDS);
            self.skip_statement();
            return marker.complete(self, ERROR);
        } else {
            self.error_expected(
                "`TABLE`, `FUNCTION`, `AGGREGATE FUNCTION`, `TABLE FUNCTION` or `CONSTANT`",
            );
            None
        };
        self.eat_keywords(&["IF", "NOT", "EXISTS"]);
        let Some(object) = object else {
            self.skip_until(|p| p.current() == SEMICOLON);
            return marker.complete(self, ERROR);
        };
        self.record_name(match object {
            Object::Table => NameRole::Table,
            Object::Function(FunctionKind::Scalar) => NameRole::Function(FunctionRole::Scalar),
            Object::Function(FunctionKind::Aggregate) => {
                NameRole::Function(FunctionRole::Aggregate)
            }
            Object::Function(FunctionKind::Table) => NameRole::Function(FunctionRole::Table),
            Object::Constant => NameRole::Variable,
        });
        match object {
            Object::Table => {
                self.table_path();
            }
            Object::Function(kind) => {
                let role = match kind {
                    FunctionKind::Scalar => FunctionRole::Scalar,
                    FunctionKind::Aggregate => FunctionRole::Aggregate,
                    FunctionKind::Table => FunctionRole::Table,
                };
                self.path(NameRole::Function(role));
            }
            Object::Constant => {
                self.path(NameRole::Variable);
            }
        }
        let kind = match object {
            Object::Table => {
                self.create_table();
                CREATE_TABLE_STMT
            }
            Object::Function(kind) => {
                self.create_function(kind);
                CREATE_FUNCTION_STMT
            }
            Object::Constant => {
                self.expect(EQ, "`=`");
                self.expr();
                CREATE_CONSTANT_STMT
            }
        };
        marker.complete(self, kind)
    }

    /// The rest of `CREATE TABLE`, after its name.
    fn create_table(&mut self) {
        // GoogleSQL's tables that copy another's schema or data.
        if let Some(word) = ["LIKE", "CLONE", "COPY"]
            .iter()
            .find(|word| self.keyword_at(0, word))
        {
            let range = self.range();
            self.error(
                DiagnosticCode::Unsupported,
                range,
                format!("avenger-sql's tables take no {word}"),
            );
            self.skip_statement();
            return;
        }
        if self.current() == L_PAREN {
            let list = self.start();
            self.parens(|p| {
                p.comma_list_until(Self::column_definition, |p| p.current() == R_PAREN);
            });
            list.complete(self, COLUMN_LIST);
        }
        if let Some(word) = ["LIKE", "CLONE", "COPY"]
            .iter()
            .find(|word| self.keyword_at(0, word))
        {
            let range = self.range();
            self.error(
                DiagnosticCode::Unsupported,
                range,
                format!("avenger-sql's tables take no {word}"),
            );
            self.skip_statement();
            return;
        }
        for clause in [
            ["DEFAULT", "COLLATE"],
            ["PARTITION", "BY"],
            ["CLUSTER", "BY"],
            ["WITH", "CONNECTION"],
        ] {
            if self.keyword_at(0, clause[0]) && self.keyword_at(1, clause[1]) {
                let range = self.range().cover(self.nth_range(1));
                self.error(
                    DiagnosticCode::Unsupported,
                    range,
                    format!("avenger-sql's tables take no {} {}", clause[0], clause[1]),
                );
                self.skip_until(|p| {
                    p.current() == SEMICOLON || p.at_keyword("OPTIONS") || p.at_keyword("AS")
                });
            }
        }
        self.options();
        if self.eat_keyword("AS") {
            self.query();
        }
    }

    /// `name type [NOT NULL]`
    fn column_definition(&mut self) {
        let marker = self.start();
        self.name(NameRole::Column);
        self.ty();
        self.eat_keywords(&["NOT", "NULL"]);
        if ["DEFAULT", "OPTIONS", "PRIMARY", "REFERENCES", "GENERATED"]
            .iter()
            .any(|word| self.keyword_at(0, word))
        {
            let range = self.range();
            self.error(
                DiagnosticCode::Unsupported,
                range,
                "a column of avenger-sql's tables takes a name, a type and NOT NULL",
            );
            self.skip_until(|p| matches!(p.current(), COMMA | R_PAREN | SEMICOLON));
        }
        marker.complete(self, COLUMN_DEF);
    }

    /// `[OPTIONS (name = value, ...)]`
    pub(super) fn options(&mut self) {
        if !(self.at_keyword("OPTIONS") && self.nth(1) == L_PAREN) {
            return;
        }
        let marker = self.start();
        self.bump_keyword();
        self.paren_list(
            |p| {
                let entry = p.start();
                // As in GoogleSQL, an option's name can also be HASH, PROTO or PARTITION.
                let hint_word = ["HASH", "PROTO", "PARTITION"]
                    .iter()
                    .any(|word| p.keyword_at(0, word));
                if hint_word {
                    let name = p.start();
                    p.bump();
                    name.complete(p, NAME);
                } else {
                    p.name(NameRole::Option);
                }
                // GoogleSQL's `+=` and `-=` of an ALTER's options.
                if matches!(p.current(), PLUS | MINUS) && p.nth(1) == EQ {
                    let range = p.range();
                    p.error(
                        DiagnosticCode::Unsupported,
                        range,
                        "avenger-sql's OPTIONS set each option with `=`",
                    );
                    let error = p.start();
                    p.bump();
                    error.complete(p, ERROR);
                }
                p.expect(EQ, "`=`");
                p.expr();
                entry.complete(p, OPTION_ENTRY);
            },
            true,
        );
        marker.complete(self, OPTIONS);
    }

    /// The rest of `CREATE [AGGREGATE | TABLE] FUNCTION`, after its name.
    fn create_function(&mut self, kind: FunctionKind) {
        let list = self.start();
        let table = kind == FunctionKind::Table;
        self.paren_list(|p| p.parameter(table), true);
        list.complete(self, PARAM_LIST);
        match kind {
            FunctionKind::Table => {
                if self.at_keyword("RETURNS") {
                    let returns = self.start();
                    self.bump_keyword();
                    let table = self.start();
                    self.expect_keyword("TABLE");
                    if self.expect(LT, "`<`") {
                        self.table_columns();
                    }
                    table.complete(self, TABLE_TYPE);
                    returns.complete(self, RETURNS_CLAUSE);
                }
                if self.language() {
                    return;
                }
                self.options();
                if self.no_body() {
                    return;
                }
                let body = self.start();
                self.expect_keyword("AS");
                self.query();
                body.complete(self, FUNCTION_BODY);
            }
            FunctionKind::Scalar | FunctionKind::Aggregate => {
                if self.at_keyword("RETURNS") {
                    let returns = self.start();
                    self.bump_keyword();
                    self.ty();
                    returns.complete(self, RETURNS_CLAUSE);
                }
                if self.language() {
                    return;
                }
                // GoogleSQL's attributes of a function, as `DETERMINISTIC`.
                let attribute = [
                    "DETERMINISTIC",
                    "IMMUTABLE",
                    "STABLE",
                    "VOLATILE",
                    "SQL",
                    "REMOTE",
                ]
                .iter()
                .any(|word| self.keyword_at(0, word))
                    || (self.keyword_at(0, "NOT") && self.keyword_at(1, "DETERMINISTIC"))
                    || (self.keyword_at(0, "WITH") && self.keyword_at(1, "CONNECTION"));
                if attribute {
                    let range = self.range();
                    self.error(
                        DiagnosticCode::Unsupported,
                        range,
                        "avenger-sql's functions take no attributes, such as DETERMINISTIC",
                    );
                    self.skip_statement();
                    return;
                }
                // As in GoogleSQL, OPTIONS goes before the body, or after it.
                let before = self.at_keyword("OPTIONS");
                self.options();
                if self.no_body() {
                    return;
                }
                let body = self.start();
                self.expect_keyword("AS");
                self.parens(|p| {
                    p.expr();
                });
                body.complete(self, FUNCTION_BODY);
                if !before {
                    self.options();
                }
            }
        }
    }

    /// Whether a function names a LANGUAGE, which it reports, skipping the rest of the statement.
    fn language(&mut self) -> bool {
        if !self.keyword_at(0, "LANGUAGE") {
            return false;
        }
        let range = self.range();
        self.error(
            DiagnosticCode::Unsupported,
            range,
            "avenger-sql's functions are SQL, so they take no LANGUAGE",
        );
        self.skip_until(|p| p.current() == SEMICOLON);
        true
    }

    /// Whether a function ends without a body, as a declaration of GoogleSQL's does, which it
    /// reports.
    fn no_body(&mut self) -> bool {
        // The body, where the text ends.
        self.record(Expected::Keyword(AS_KW));
        if !matches!(self.current(), SEMICOLON | EOF) {
            return false;
        }
        let range = self.previous_range();
        self.error(
            DiagnosticCode::Unsupported,
            range,
            "avenger-sql's functions have bodies, as in `AS (x + 1)`",
        );
        true
    }

    /// Reports a form of a parameter that GoogleSQL has and avenger-sql doesn't, and skips the
    /// parameter.
    fn unsupported_parameter(&mut self, marker: super::Marker, message: &str) {
        let range = self.range();
        self.error(DiagnosticCode::Unsupported, range, message);
        self.skip_until(|p| matches!(p.current(), COMMA | R_PAREN | SEMICOLON));
        marker.complete(self, PARAMETER);
    }

    /// `name type`, `name ANY TYPE`, `name TABLE<columns>`, or `name ANY TABLE`, and `NOT
    /// AGGREGATE`. The validation pass checks them against the kind of the function.
    fn parameter(&mut self, table: bool) {
        let marker = self.start();
        // GoogleSQL's parameters without names, such as `(STRING, INT64)`, and its templated
        // parameters without names, such as `(ANY TYPE)`.
        let unnamed = matches!(self.nth(1), COMMA | R_PAREN)
            || self.keyword_at(1, "NOT")
            || self.keyword_at(1, "AS")
            || (self.keyword_at(0, "ANY") && self.nth(1) == WORD)
            || (self.nth(1) == LT && !self.keyword_at(0, "TABLE"));
        if unnamed && !self.at(R_PAREN) {
            return self.unsupported_parameter(marker, "a parameter of avenger-sql has a name");
        }
        if self.current() == WORD && self.nth(1) == DOT {
            return self.unsupported_parameter(marker, "a parameter's name is one name");
        }
        self.name(NameRole::Parameter);
        // `ANY` starts a templated type, and GoogleSQL rejects any other word after it.
        if self.at_keyword("ANY")
            && !["TYPE", "TABLE", "PROTO", "ENUM", "STRUCT", "ARRAY"]
                .iter()
                .any(|word| self.keyword_at(1, word))
        {
            self.bump_keyword();
            self.error_expected("`TYPE`, `TABLE`, `PROTO`, `ENUM`, `STRUCT` or `ARRAY`");
            self.skip_until(|p| matches!(p.current(), COMMA | R_PAREN | SEMICOLON));
            marker.complete(self, PARAMETER);
            return;
        }
        if self.at_keyword("ANY")
            && ["PROTO", "ENUM", "STRUCT", "ARRAY"]
                .iter()
                .any(|word| self.keyword_at(1, word))
        {
            return self.unsupported_parameter(
                marker,
                "avenger-sql's templated parameters are ANY TYPE and ANY TABLE",
            );
        }
        if self.at_keywords(&["ANY", "TYPE"]) {
            let any = self.start();
            self.eat_keywords(&["ANY", "TYPE"]);
            any.complete(self, ANY_TYPE);
        } else if (table && self.at_keywords(&["ANY", "TABLE"]))
            || (self.keyword_at(0, "ANY") && self.keyword_at(1, "TABLE"))
        {
            // A table parameter belongs to a table function, as the validation pass checks, so
            // only a table function's parameters expect one.
            let any = self.start();
            self.eat_keywords(&["ANY", "TABLE"]);
            any.complete(self, ANY_TABLE);
        } else if self.keyword_at(0, "TABLE") && self.nth(1) == LT {
            let table = self.start();
            self.bump_keyword();
            self.bump();
            self.table_columns();
            table.complete(self, TABLE_TYPE);
        } else {
            self.ty();
        }
        if self.at_keyword("AS") {
            return self
                .unsupported_parameter(marker, "avenger-sql's parameters have no type aliases");
        }
        if self.at_keyword("DEFAULT") {
            return self.unsupported_parameter(marker, "avenger-sql's parameters have no defaults");
        }
        self.eat_keywords(&["NOT", "AGGREGATE"]);
        marker.complete(self, PARAMETER);
    }

    /// The columns of `TABLE<...>`, after `<` and through `>`. The validation pass checks that
    /// each has a name.
    fn table_columns(&mut self) {
        if self.eat(GT) {
            return;
        }
        loop {
            let named = self.at_ident() && !matches!(self.nth(1), COMMA | GT | L_PAREN | LT);
            let field = self.start();
            if named {
                self.name(NameRole::Column);
            }
            self.ty();
            field.complete(self, STRUCT_FIELD);
            if !self.eat(COMMA) || self.at_list_close() {
                break;
            }
        }
        self.expect(GT, "`>`");
    }
}
