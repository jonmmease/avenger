//! Scripts and modules: statements separated by `;`. A script's statement is a pipe query,
//! `IMPORT MODULE`, `DECLARE`, `SET`, `CREATE TEMP TABLE`, or `CREATE FUNCTION` of a scalar,
//! aggregate or table function. A module, a `.av.sqlm` file, starts with `MODULE name` and holds
//! `IMPORT MODULE` and `CREATE PUBLIC` or `CREATE PRIVATE` statements of functions, table
//! functions and constants, following GoogleSQL's modules. A data script registers the tables of
//! a project with `CREATE EXTERNAL TABLE`, `LOAD DATA`, `CREATE VIEW`, `CREATE TABLE ... AS` and
//! `CREATE SCHEMA`, following GoogleSQL's DDL, and a script reads its own files with `CREATE TEMP
//! EXTERNAL TABLE` and `LOAD DATA ... TEMP TABLE`.
//!
//! The parser reads the statements of every role in every text, and the validation pass checks
//! each statement against the role of its text, so a statement out of its place has an error
//! that names its place.

use super::Parser;
use crate::{
    ast::FunctionKind, diagnostic::DiagnosticCode, Expected, NameRole, Role, SyntaxKind::*,
};

/// The first keywords of GoogleSQL's statements that avenger-sql doesn't run: procedural
/// statements and blocks, DDL, DML, transactions and the rest.
#[rustfmt::skip]
const UNSUPPORTED_STATEMENTS: &[&str] = &[
    // Procedural statements and blocks.
    "BEGIN", "IF", "LOOP", "REPEAT", "WHILE", "FOR", "BREAK", "LEAVE", "CONTINUE", "ITERATE",
    "RETURN", "RAISE", "EXECUTE", "CALL", "CASE",
    // Transactions and batches.
    "START", "COMMIT", "ROLLBACK", "RUN", "ABORT",
    // DDL.
    "ALTER", "DROP", "UNDROP", "RENAME", "TRUNCATE", "GRANT", "REVOKE", "DEFINE", "CLONE",
    // DML.
    "INSERT", "UPDATE", "DELETE", "MERGE",
    // The rest.
    "EXPORT", "LOAD", "ASSERT", "DESCRIBE", "DESC", "SHOW", "EXPLAIN", "ANALYZE", "GRAPH",
];

/// The first keywords of GoogleSQL's blocks, which end at a matching `END`, where they start a
/// statement. A `CASE` expression ends at an `END` too, so a `CASE` anywhere opens one.
const BLOCKS: &[&str] = &["BEGIN", "IF", "LOOP", "WHILE", "FOR", "REPEAT", "CASE"];

/// The keywords after which a statement starts, inside a block.
const STATEMENT_STARTS: &[&str] = &["THEN", "ELSE", "DO", "BEGIN", "LOOP", "REPEAT"];

/// The keywords that start a `LOAD DATA` statement.
const LOAD_DATA: &[&str] = &["LOAD", "DATA"];

/// The kind of object that a `CREATE` statement creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Object {
    Table,
    Function(FunctionKind),
    Constant,
    ExternalTable,
    View,
    Schema,
}

/// The help of a statement that a script doesn't hold.
const SCRIPT_HOLDS: &str =
    "a script holds pipe queries, IMPORT MODULE, DECLARE, SET, CREATE TEMP TABLE, CREATE TEMP FUNCTION, CREATE TEMP EXTERNAL TABLE and LOAD DATA ... TEMP TABLE";

/// The help of a statement that a data script doesn't hold.
const DATA_SCRIPT_HOLDS: &str =
    "a data script holds CREATE EXTERNAL TABLE, LOAD DATA, CREATE VIEW, CREATE TABLE ... AS, CREATE SCHEMA, CREATE TEMP FUNCTION and IMPORT MODULE";

/// An example of the options of a table's files, for the help of a missing `OPTIONS` or `FROM
/// FILES`.
const FILE_OPTIONS: &str = "(format = 'CSV', uris = ['data/files/sales.csv'])";

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
        if !self.eat(SEMICOLON) && !self.at_eof() {
            self.error_expected("`;` or the end of the module");
            self.skip_until(|p| p.current() == SEMICOLON);
            self.eat(SEMICOLON);
        }
        self.statements();
        marker.complete(self, MODULE);
    }

    /// The help of a statement that the text's role doesn't hold.
    fn holds(&self) -> &'static str {
        match self.role {
            Role::Data => DATA_SCRIPT_HOLDS,
            Role::Script | Role::Module => SCRIPT_HOLDS,
        }
    }

    /// The statements up to the end, separated by `;`. A statement starts at the start of the
    /// text or after a `;`.
    fn statements(&mut self) {
        let place = match self.role {
            Role::Module => "module",
            Role::Script | Role::Data => "script",
        };
        loop {
            self.record(Expected::Statement);
            // `LOAD DATA` starts a statement of a data script or a chart script, though the text
            // ends here.
            if self.role != Role::Module {
                self.record(Expected::Phrase(LOAD_DATA));
            }
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
        let in_module = self.role == Role::Module;
        if self.at_keyword("MODULE") {
            let message = match in_module {
                true => "a module has one MODULE statement, its first",
                false => "a MODULE statement starts a module, not a script",
            };
            let range = self.range();
            // GoogleSQL takes a MODULE statement in a script, and avenger reads it in a module.
            let code = match in_module {
                true => DiagnosticCode::InvalidStatement,
                false => DiagnosticCode::Unsupported,
            };
            self.error(code, range, message);
            self.skip_until(|p| p.current() == SEMICOLON);
            return None;
        }
        // A `LOAD DATA` belongs to a data script, or with `TEMP TABLE` to a chart script, and the
        // validation pass reports it elsewhere.
        if self.at_data_keywords(LOAD_DATA, true) {
            return Some(self.load_data());
        }
        if in_module && !self.at_keyword("IMPORT") && !self.at_keyword("CREATE") {
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
            let help = self.holds();
            self.error(
                DiagnosticCode::Unsupported,
                range,
                format!(
                    "avenger-sql's scripts have no {} statement",
                    word.to_ascii_uppercase()
                ),
            )
            .help(help);
            self.skip_statement();
            return None;
        }
        self.query()
    }

    /// Skips a statement into an `ERROR` node, through its `;` or the end, and through the `END`
    /// of each block that it opens, so the statements of a block skip with it.
    fn skip_statement(&mut self) {
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
                if (BLOCKS.iter()).any(|word| *word != "BEGIN" && self.keyword_at(1, word)) {
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
    /// against the role of the text.
    fn create(&mut self) -> super::CompletedMarker {
        let marker = self.start();
        self.bump_keyword();
        self.eat_keywords(&["OR", "REPLACE"]);
        // A script creates temporary objects and a module public or private ones, as the
        // validation pass checks, so the parser expects the scopes of the text that it reads.
        let in_module = self.role == Role::Module;
        let scopes: [(&str, bool); 4] = [
            ("TEMP", !in_module),
            ("TEMPORARY", !in_module),
            ("PUBLIC", in_module),
            ("PRIVATE", in_module),
        ];
        let scope = (scopes.iter())
            .find(|(word, expected)| {
                (*expected && self.at_keyword(word)) || self.keyword_at(0, word)
            })
            .map(|(word, _)| *word);
        if let Some(scope) = scope {
            self.eat_keyword(scope);
        }
        // A chart script's own table of files is `TEMP`.
        let temp = matches!(scope, Some("TEMP" | "TEMPORARY"));
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
        } else if (in_module && self.at_keyword("CONSTANT")) || self.keyword_at(0, "CONSTANT") {
            // A constant belongs to a module, so a script doesn't expect one.
            self.bump_keyword();
            Some(Object::Constant)
        } else if !self.keyword_at(2, "FUNCTION")
            && self.eat_data_keywords(&["EXTERNAL", "TABLE"], temp)
        {
            Some(Object::ExternalTable)
        } else if self.eat_data_keywords(&["VIEW"], false) {
            Some(Object::View)
        } else if (scope.is_none() && self.at_data_keywords(&["SCHEMA"], false))
            || self.keyword_at(0, "SCHEMA")
        {
            // As in GoogleSQL, a schema takes no scope, an error at its keyword.
            let range = self.range();
            self.bump_keyword();
            if let Some(scope) = scope {
                self.error(
                    DiagnosticCode::InvalidStatement,
                    range,
                    format!("a SCHEMA takes no {scope}"),
                )
                .help("a schema groups the tables of a project, which every script reads");
            }
            Some(Object::Schema)
        } else if self.current() == WORD {
            // An object of GoogleSQL's that avenger-sql doesn't create, such as a materialized
            // view.
            let range = self.range();
            let mut words = vec![self.nth_text(0).to_ascii_uppercase()];
            // GoogleSQL's `EXTERNAL TABLE FUNCTION`, which its parser rejects.
            let external_function = self.keyword_at(0, "EXTERNAL")
                && self.keyword_at(1, "TABLE")
                && self.keyword_at(2, "FUNCTION");
            for n in 1..4 {
                let word = self.nth_text(n).to_ascii_uppercase();
                if self.nth(n) != WORD || matches!(word.as_str(), "IF" | "AS") {
                    break;
                }
                // The objects of two words, such as `MATERIALIZED VIEW`.
                if !matches!(
                    words[0].as_str(),
                    "MATERIALIZED"
                        | "EXTERNAL"
                        | "RECURSIVE"
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
                ) || n > 2
                    || (n > 1 && !external_function)
                {
                    break;
                }
                words.push(word);
            }
            let help = self.holds();
            self.error(
                DiagnosticCode::Unsupported,
                range,
                format!("avenger-sql's scripts create no {}", words.join(" ")),
            )
            .help(help);
            self.skip_statement();
            return marker.complete(self, ERROR);
        } else {
            let data = match self.role {
                Role::Data => ", `EXTERNAL TABLE`, `VIEW`, `SCHEMA`",
                Role::Script if temp => ", `EXTERNAL TABLE`",
                Role::Script | Role::Module => "",
            };
            self.error_expected(&format!(
                "`TABLE`, `FUNCTION`, `AGGREGATE FUNCTION`, `TABLE FUNCTION`{data} or `CONSTANT`"
            ));
            None
        };
        self.eat_keywords(&["IF", "NOT", "EXISTS"]);
        let Some(object) = object else {
            self.skip_until(|p| p.current() == SEMICOLON);
            return marker.complete(self, ERROR);
        };
        let role = match object {
            Object::Table | Object::ExternalTable | Object::View => NameRole::Table,
            Object::Function(kind) => NameRole::Function(kind.into()),
            Object::Constant => NameRole::Variable,
            Object::Schema => NameRole::Schema,
        };
        self.record_name(role);
        let named = match object {
            Object::Table | Object::ExternalTable | Object::View => self.table_path(),
            Object::Function(_) | Object::Constant | Object::Schema => self.path(role).is_some(),
        };
        // The statements of a data script without a name skip the rest, whose errors would
        // follow from the missing name.
        let data_kind = match object {
            Object::ExternalTable => Some(CREATE_EXTERNAL_TABLE_STMT),
            Object::View => Some(CREATE_VIEW_STMT),
            Object::Schema => Some(CREATE_SCHEMA_STMT),
            Object::Table | Object::Function(_) | Object::Constant => None,
        };
        if let (false, Some(kind)) = (named, data_kind) {
            self.skip_until(|p| p.current() == SEMICOLON);
            return marker.complete(self, kind);
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
            Object::ExternalTable => {
                self.create_external_table();
                CREATE_EXTERNAL_TABLE_STMT
            }
            Object::View => {
                self.create_view();
                CREATE_VIEW_STMT
            }
            Object::Schema => {
                self.create_schema();
                CREATE_SCHEMA_STMT
            }
        };
        marker.complete(self, kind)
    }

    /// Whether the next tokens are the keywords `keywords` of a statement of a data script. A
    /// data script records them at the cursor, and a chart script when `in_chart` holds, for its
    /// own tables of files, so that a completion offers only the statements that the text's role
    /// holds.
    fn at_data_keywords(&self, keywords: &'static [&'static str], in_chart: bool) -> bool {
        let expected = match self.role {
            Role::Data => true,
            Role::Script => in_chart,
            Role::Module => false,
        };
        match (expected, keywords) {
            (true, [keyword]) => self.at_keyword(keyword),
            (true, _) => self.at_keywords(keywords),
            (false, _) => {
                (keywords.iter().enumerate()).all(|(n, keyword)| self.keyword_at(n, keyword))
            }
        }
    }

    /// Whether the next tokens are the keywords `keywords` of a statement of a data script, as
    /// [`Self::at_data_keywords`] reads them, which it consumes when they are.
    fn eat_data_keywords(&mut self, keywords: &'static [&'static str], in_chart: bool) -> bool {
        let found = self.at_data_keywords(keywords, in_chart);
        if found {
            for _ in keywords {
                self.bump_keyword();
            }
        }
        found
    }

    /// Reports the clause of two keywords at the current token, GoogleSQL's clause of `what`
    /// that avenger-sql doesn't read, when it is one of `clauses`, and skips the tokens before
    /// the next token that `stop` accepts.
    fn unsupported_clause(
        &mut self,
        what: &str,
        clauses: &[[&str; 2]],
        stop: impl Fn(&Self) -> bool,
    ) -> bool {
        let Some(clause) = (clauses.iter())
            .find(|clause| self.keyword_at(0, clause[0]) && self.keyword_at(1, clause[1]))
        else {
            return false;
        };
        let range = self.range().cover(self.nth_range(1));
        self.error(
            DiagnosticCode::Unsupported,
            range,
            format!("avenger-sql's {what} take no {} {}", clause[0], clause[1]),
        );
        self.skip_until(|p| p.current() == SEMICOLON || stop(p));
        true
    }

    /// The rest of `CREATE EXTERNAL TABLE`, after its name.
    fn create_external_table(&mut self) {
        if self.copied_table() {
            return;
        }
        if self.current() == L_PAREN {
            self.column_list();
        }
        if self.copied_table() {
            return;
        }
        self.unsupported_clause("external tables", &[["DEFAULT", "COLLATE"]], |p| {
            p.keyword_at(0, "WITH") || p.keyword_at(0, "OPTIONS")
        });
        self.partition_columns();
        self.with_connection();
        // A `WITH` of another clause, which GoogleSQL reports at the word after it.
        if self.keyword_at(0, "WITH") && !self.keyword_at(1, "PARTITION") {
            let error = self.start();
            self.bump_keyword();
            error.complete(self, ERROR);
            self.error_expected("`PARTITION COLUMNS` or `CONNECTION`");
            self.skip_until(|p| p.current() == SEMICOLON);
            return;
        }
        if !self.at_keyword("OPTIONS") {
            if self.reported_at_end() {
                return;
            }
            self.error_expected("`OPTIONS`").help = Some(format!(
                "an external table names its files in its options, as in `OPTIONS {FILE_OPTIONS}`"
            ));
            self.skip_until(|p| p.current() == SEMICOLON);
            return;
        }
        let options = self.start();
        self.bump_keyword();
        self.option_entries();
        options.complete(self, OPTIONS);
    }

    /// `[WITH PARTITION COLUMNS [(columns)]]`
    fn partition_columns(&mut self) {
        let at = self.at_keywords(&["WITH", "PARTITION", "COLUMNS"])
            || (self.keyword_at(0, "WITH") && self.keyword_at(1, "PARTITION"));
        if !at {
            return;
        }
        let marker = self.start();
        self.bump_keyword();
        self.bump_keyword();
        if self.expect_keyword("COLUMNS") && self.current() == L_PAREN {
            self.column_list();
        }
        marker.complete(self, PARTITION_COLUMNS);
    }

    /// Reports GoogleSQL's `WITH CONNECTION` of a table of files, which avenger-sql reads without
    /// connections, and skips it.
    fn with_connection(&mut self) {
        if !(self.keyword_at(0, "WITH") && self.keyword_at(1, "CONNECTION")) {
            return;
        }
        let range = self.range().cover(self.nth_range(1));
        self.error(
            DiagnosticCode::Unsupported,
            range,
            "avenger-sql reads files without connections, so no WITH CONNECTION",
        );
        self.skip_until(|p| {
            p.current() == SEMICOLON
                || p.keyword_at(0, "OPTIONS")
                || (p.keyword_at(0, "WITH") && p.keyword_at(1, "PARTITION"))
        });
    }

    /// The rest of `CREATE VIEW`, after its name.
    fn create_view(&mut self) {
        if self.current() == L_PAREN {
            let range = self.range();
            self.error(
                DiagnosticCode::Unsupported,
                range,
                "avenger-sql's views take their columns from their queries, so no column list",
            );
            let error = self.start();
            self.skip_balanced();
            error.complete(self, ERROR);
        }
        if self.keyword_at(0, "SQL") && self.keyword_at(1, "SECURITY") {
            let n = if self.nth(2) == WORD { 3 } else { 2 };
            self.unsupported_tokens(n, "avenger-sql's views take no SQL SECURITY");
        }
        self.options();
        if !self.at_keyword("AS") {
            if !self.reported_at_end() {
                self.error_expected("`AS`");
            }
            self.skip_until(|p| p.current() == SEMICOLON);
            return;
        }
        self.bump_keyword();
        self.query();
    }

    /// The rest of `CREATE SCHEMA`, after its name.
    fn create_schema(&mut self) {
        self.unsupported_clause("schemas", &[["DEFAULT", "COLLATE"]], |p| {
            p.keyword_at(0, "OPTIONS")
        });
        self.options();
    }

    /// `LOAD DATA {OVERWRITE | INTO} [TEMP TABLE] name [(columns)] [OPTIONS (...)] FROM FILES
    /// (...) [WITH PARTITION COLUMNS [(columns)]]`
    fn load_data(&mut self) -> super::CompletedMarker {
        let marker = self.start();
        self.bump_keyword();
        self.bump_keyword();
        if !self.expect_keyword_of(&["OVERWRITE", "INTO"]) {
            self.skip_until(|p| p.current() == SEMICOLON);
            return marker.complete(self, LOAD_DATA_STMT);
        }
        // `TEMP` is a scope only before `TABLE`, and otherwise a table's name.
        if self.at_keywords(&["TEMP", "TABLE"]) || self.at_keywords(&["TEMPORARY", "TABLE"]) {
            self.bump_keyword();
            self.bump_keyword();
        }
        self.record_name(NameRole::Table);
        if !self.table_path() {
            self.skip_until(|p| p.current() == SEMICOLON);
            return marker.complete(self, LOAD_DATA_STMT);
        }
        if self.current() == L_PAREN {
            self.column_list();
        }
        // GoogleSQL's clauses of the partitions, the collation and the clusters of the table.
        let stop = |p: &Self| {
            p.keyword_at(0, "OPTIONS") || (p.keyword_at(0, "FROM") && p.keyword_at(1, "FILES"))
        };
        let n = match self.keyword_at(0, "OVERWRITE") && self.keyword_at(1, "PARTITIONS") {
            true => 2,
            false => usize::from(self.keyword_at(0, "PARTITIONS") || self.keyword_at(0, "COLLATE")),
        };
        if n > 0 {
            let range = self.range().cover(self.nth_range(n - 1));
            let clause = (0..n)
                .map(|k| self.nth_text(k).to_ascii_uppercase())
                .collect::<Vec<_>>()
                .join(" ");
            self.error(
                DiagnosticCode::Unsupported,
                range,
                format!("avenger-sql's LOAD DATA statements take no {clause}"),
            );
            self.skip_until(|p| p.current() == SEMICOLON || stop(p));
        }
        self.unsupported_clause(
            "LOAD DATA statements",
            &[["PARTITION", "BY"], ["CLUSTER", "BY"]],
            stop,
        );
        self.options();
        if !self.files() {
            self.skip_until(|p| p.current() == SEMICOLON);
            return marker.complete(self, LOAD_DATA_STMT);
        }
        self.partition_columns();
        self.with_connection();
        marker.complete(self, LOAD_DATA_STMT)
    }

    /// `FROM FILES (name = value, ...)`, and whether it is there.
    fn files(&mut self) -> bool {
        if !(self.at_keywords(&["FROM", "FILES"]) || self.keyword_at(0, "FROM")) {
            if self.reported_at_end() {
                return false;
            }
            self.error_expected("`FROM FILES`").help = Some(format!(
                "LOAD DATA names its files in FROM FILES, as in `FROM FILES {FILE_OPTIONS}`"
            ));
            return false;
        }
        let marker = self.start();
        self.bump_keyword();
        let files = self.expect_keyword("FILES");
        if files {
            self.option_entries();
        }
        marker.complete(self, FROM_FILES);
        files
    }

    /// Whether GoogleSQL's table that copies another's schema or data, with `LIKE`, `CLONE` or
    /// `COPY`, is at the current token, which it reports, skipping the rest of the statement.
    fn copied_table(&mut self) -> bool {
        let Some(word) = (["LIKE", "CLONE", "COPY"].iter()).find(|word| self.keyword_at(0, word))
        else {
            return false;
        };
        let range = self.range();
        self.error(
            DiagnosticCode::Unsupported,
            range,
            format!("avenger-sql's tables take no {word}"),
        );
        self.skip_statement();
        true
    }

    /// The rest of `CREATE TABLE`, after its name.
    fn create_table(&mut self) {
        if self.copied_table() {
            return;
        }
        if self.current() == L_PAREN {
            self.column_list();
        }
        if self.copied_table() {
            return;
        }
        for clause in [
            ["DEFAULT", "COLLATE"],
            ["PARTITION", "BY"],
            ["CLUSTER", "BY"],
            ["WITH", "CONNECTION"],
        ] {
            self.unsupported_clause("tables", &[clause], |p| {
                p.at_keyword("OPTIONS") || p.at_keyword("AS")
            });
        }
        self.options();
        if self.eat_keyword("AS") {
            self.query();
        }
    }

    /// `(name type [NOT NULL], ...)`
    fn column_list(&mut self) {
        let list = self.start();
        self.parens(|p| {
            // Where the text ends after the `(`, the `)` is missing, and a name can start.
            if p.at_eof() {
                p.record_name(NameRole::Column);
            } else {
                p.comma_list_until(Self::column_definition, |p| p.current() == R_PAREN);
            }
        });
        list.complete(self, COLUMN_LIST);
    }

    /// `name type [NOT NULL]`
    fn column_definition(&mut self) {
        let marker = self.start();
        // GoogleSQL's constraints of a table, as `PRIMARY KEY (a)` or `CONSTRAINT c CHECK (a > 0)`,
        // among its columns.
        let constraint = (self.keyword_at(0, "PRIMARY") && self.keyword_at(1, "KEY"))
            || (self.keyword_at(0, "FOREIGN") && self.keyword_at(1, "KEY"))
            || (self.keyword_at(0, "CHECK") && self.nth(1) == L_PAREN)
            || (self.keyword_at(0, "CONSTRAINT")
                && (self.keyword_at(2, "FOREIGN") || self.keyword_at(2, "CHECK")));
        if constraint {
            let range = self.range();
            self.error(
                DiagnosticCode::Unsupported,
                range,
                "avenger-sql's tables take no constraints, such as PRIMARY KEY",
            );
            self.skip_until(|p| matches!(p.current(), COMMA | R_PAREN | SEMICOLON));
            marker.complete(self, ERROR);
            return;
        }
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
        self.option_entries();
        marker.complete(self, OPTIONS);
    }

    /// `(name = value, ...)`, the entries of `OPTIONS` or of `FROM FILES`.
    fn option_entries(&mut self) {
        self.parens(|p| {
            // Where the text ends after the `(`, the `)` is missing, and a name can start.
            if p.at_eof() {
                p.record_name(NameRole::Option);
            } else if p.current() != R_PAREN {
                p.comma_list(Self::option_entry);
            }
        });
    }

    /// `name = value`
    fn option_entry(&mut self) {
        let entry = self.start();
        // As in GoogleSQL, an option's name can also be HASH, PROTO or PARTITION.
        let hint_word = ["HASH", "PROTO", "PARTITION"]
            .iter()
            .any(|word| self.keyword_at(0, word));
        if hint_word {
            let name = self.start();
            self.bump();
            name.complete(self, NAME);
        } else {
            self.name(NameRole::Option);
        }
        // GoogleSQL's `+=` and `-=` of an ALTER's options.
        if matches!(self.current(), PLUS | MINUS) && self.nth(1) == EQ {
            self.unsupported_tokens(1, "avenger-sql's OPTIONS set each option with `=`");
        }
        self.expect(EQ, "`=`");
        self.expr();
        entry.complete(self, OPTION_ENTRY);
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
            // The `ANY` without its kind stays in the tree as an error.
            let any = self.start();
            self.bump_keyword();
            any.complete(self, ERROR);
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
