//! Scripts and modules: statements separated by `;`. A script's statement is a pipe query,
//! `IMPORT MODULE`, `DECLARE`, `SET`, `CREATE TEMP TABLE`, or `CREATE FUNCTION` of a scalar,
//! aggregate or table function. A module, a `.sqlm` file, starts with `MODULE name` and holds
//! `IMPORT MODULE` and `CREATE PUBLIC` or `CREATE PRIVATE` statements of functions, table
//! functions and constants, following GoogleSQL's modules.

use super::Parser;
use crate::{diagnostic::DiagnosticCode, Expected, NameRole, SyntaxKind::*};

/// The procedural statements of GoogleSQL's scripts, which avenger-sql doesn't read.
const PROCEDURAL: &[&str] = &[
    "BEGIN", "IF", "LOOP", "REPEAT", "WHILE", "FOR", "BREAK", "LEAVE", "CONTINUE", "ITERATE",
    "RETURN", "RAISE", "EXECUTE", "CALL", "COMMIT", "ROLLBACK", "CASE",
];

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

    /// The statements up to the end, separated by `;`.
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
            if self.eat(SEMICOLON) || self.at_eof() {
                continue;
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
            self.eat(SEMICOLON);
            debug_assert!(self.position > start || self.at_eof());
        }
    }

    fn statement(&mut self) -> Option<super::CompletedMarker> {
        if self.at_keyword("MODULE") {
            let message = match self.in_module {
                true => "a module has one MODULE statement, its first",
                false => "a MODULE statement starts a module, not a script",
            };
            let range = self.range();
            self.error(DiagnosticCode::InvalidStatement, range, message);
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
        if let Some(word) = PROCEDURAL.iter().find(|word| self.keyword_at(0, word)) {
            let range = self.range();
            self.error(
                DiagnosticCode::Unsupported,
                range,
                format!("avenger-sql's scripts have no {word} statement"),
            )
            .help("a script holds queries, IMPORT MODULE, DECLARE, SET, CREATE TEMP TABLE and CREATE FUNCTION");
            self.skip_until(|p| p.current() == SEMICOLON);
            return None;
        }
        self.query()
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
                DiagnosticCode::InvalidStatement,
                range,
                "SET assigns a variable of the script, not a query parameter",
            );
            self.bump();
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
        if let Some(scope) = ["TEMP", "TEMPORARY", "PUBLIC", "PRIVATE"]
            .iter()
            .find(|word| self.at_keyword(word))
        {
            self.eat_keyword(scope);
        }
        let object = if self.eat_keywords(&["AGGREGATE", "FUNCTION"]) {
            Some(Object::Function(FunctionKind::Aggregate))
        } else if self.at_keyword("TABLE") && self.keyword_at(1, "FUNCTION") {
            // As in GoogleSQL, `TABLE FUNCTION` is a table function, and a table named `function`
            // takes backticks.
            self.eat_keywords(&["TABLE", "FUNCTION"]);
            Some(Object::Function(FunctionKind::Table))
        } else if self.eat_keyword("FUNCTION") {
            Some(Object::Function(FunctionKind::Scalar))
        } else if self.eat_keyword("TABLE") {
            Some(Object::Table)
        } else if self.eat_keyword("CONSTANT") {
            Some(Object::Constant)
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
            Object::Function(_) => NameRole::Function,
            Object::Constant => NameRole::Variable,
        });
        match object {
            Object::Table => {
                self.table_path();
            }
            _ => {
                self.path(NameRole::Function);
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
        if self.current() == L_PAREN {
            let list = self.start();
            self.parens(|p| {
                p.comma_list_until(Self::column_definition, |p| p.current() == R_PAREN);
            });
            list.complete(self, COLUMN_LIST);
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
        self.paren_list(Self::parameter, true);
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
                self.options();
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
                if self.at_keyword("LANGUAGE") {
                    let range = self.range();
                    self.error(
                        DiagnosticCode::Unsupported,
                        range,
                        "avenger-sql's functions are SQL, so they take no LANGUAGE",
                    );
                    self.skip_until(|p| p.current() == SEMICOLON);
                    return;
                }
                // As in GoogleSQL, OPTIONS goes before the body, or after it.
                let before = self.at_keyword("OPTIONS");
                self.options();
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

    /// `name type`, `name ANY TYPE`, `name TABLE<columns>`, or `name ANY TABLE`, and `NOT
    /// AGGREGATE`. The validation pass checks them against the kind of the function.
    fn parameter(&mut self) {
        let marker = self.start();
        self.name(NameRole::Parameter);
        if self.at_keyword("ANY") && self.keyword_at(1, "TYPE") {
            let any = self.start();
            self.eat_keywords(&["ANY", "TYPE"]);
            any.complete(self, ANY_TYPE);
        } else if self.at_keyword("ANY") && self.keyword_at(1, "TABLE") {
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
            if !self.eat(COMMA) {
                break;
            }
        }
        self.expect(GT, "`>`");
    }
}
