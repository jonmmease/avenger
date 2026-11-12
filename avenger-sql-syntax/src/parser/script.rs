//! Scripts and modules: statements separated by `;`. A script's statement is a pipe query,
//! `IMPORT MODULE`, `DECLARE`, `SET`, `CREATE TEMP TABLE`, or `CREATE FUNCTION` of a scalar,
//! aggregate or table function. A module, a `.sqlm` file, starts with `MODULE name` and holds
//! `IMPORT MODULE` and `CREATE PUBLIC` or `CREATE PRIVATE` statements of functions, table
//! functions and constants, following GoogleSQL's modules.

use rowan::TextRange;

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

    /// The statements up to the end, separated by `;`. As in GoogleSQL, every `DECLARE` of a
    /// script comes before its other statements.
    fn statements(&mut self) {
        let place = match self.in_module {
            true => "module",
            false => "script",
        };
        let mut after_others = false;
        loop {
            self.record(Expected::Statement);
            if self.at_eof() {
                break;
            }
            let start = self.position;
            let statement = self.statement();
            if let Some(statement) = statement {
                if statement.kind() == DECLARE_STMT && after_others {
                    let range = self.node_range(statement);
                    self.error(
                        DiagnosticCode::InvalidStatement,
                        range,
                        "a DECLARE comes before the other statements of a script",
                    );
                }
                after_others |= statement.kind() != DECLARE_STMT;
            }
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
            .help = Some(
                "functions, table functions and constants go in a module, and queries in a script"
                    .to_string(),
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
            .help = Some(
                "a script holds queries, IMPORT MODULE, DECLARE, SET, CREATE TEMP TABLE and CREATE FUNCTION"
                    .to_string(),
            );
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
            self.bump();
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
            self.bump_keyword();
            self.name(NameRole::Alias);
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
        self.comma_list(
            |p| {
                p.name(NameRole::Variable);
            },
            |_| false,
        );
        let at_end = matches!(self.current(), SEMICOLON | EOF);
        let typed = !self.at_keyword("DEFAULT") && !at_end;
        if typed {
            self.ty();
        }
        let defaulted = self.eat_keyword("DEFAULT");
        if defaulted {
            self.expr();
        }
        let node = marker.complete(self, DECLARE_STMT);
        if !typed && !defaulted {
            let range = self.node_range(node);
            self.error(
                DiagnosticCode::InvalidStatement,
                range,
                "a DECLARE takes a type, a DEFAULT, or both",
            );
        }
        node
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
            self.name_ref(NameRole::Variable);
        }
        self.expect(EQ, "`=`");
        self.expr();
        marker.complete(self, SET_STMT)
    }

    /// A `CREATE` statement.
    fn create(&mut self) -> super::CompletedMarker {
        let marker = self.start();
        self.bump_keyword();
        let mode = self.start();
        let or_replace = self.keywords_range(&["OR", "REPLACE"]);
        if let (Some(range), true) = (or_replace, self.in_module) {
            self.error(
                DiagnosticCode::InvalidStatement,
                range,
                "a module creates each object once, so no OR REPLACE",
            );
        }
        let scope_range = self.range();
        let scope = ["TEMP", "TEMPORARY", "PUBLIC", "PRIVATE"]
            .iter()
            .find(|word| self.at_keyword(word))
            .map(|word| word.to_string());
        if scope.is_some() {
            self.bump_keyword();
        }
        let public = matches!(scope.as_deref(), Some("PUBLIC" | "PRIVATE"));
        if self.in_module && !public {
            let range = match scope {
                None => self.range(),
                Some(_) => scope_range,
            };
            self.error(
                DiagnosticCode::InvalidStatement,
                range,
                "a CREATE in a module is PUBLIC or PRIVATE",
            )
            .help = Some(
                "PUBLIC objects are visible to the scripts and modules that import the module"
                    .to_string(),
            );
        }
        if !self.in_module && public {
            self.error(
                DiagnosticCode::InvalidStatement,
                scope_range,
                "PUBLIC and PRIVATE belong to the CREATE statements of a module",
            );
        }
        let object_start = self.range();
        let object = if self.eat_keywords(&["AGGREGATE", "FUNCTION"]) {
            Some(Object::Function(FunctionKind::Aggregate))
        } else if self.at_keyword("TABLE")
            && self.keyword_at(1, "FUNCTION")
            && (self.keyword_at(2, "IF") || self.ident_at(2))
        {
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
        let object_range = self.since(object_start);
        match (object, self.in_module) {
            (Some(Object::Table), true) => {
                self.error(
                    DiagnosticCode::InvalidStatement,
                    object_range,
                    "a module creates functions, table functions and constants, not tables",
                );
            }
            (Some(Object::Constant), false) => {
                self.error(
                    DiagnosticCode::InvalidStatement,
                    object_range,
                    "a CONSTANT belongs to a module",
                )
                .help =
                    Some("a script declares a variable, as in `DECLARE x DEFAULT 1`".to_string());
            }
            _ => {}
        }
        let if_not_exists = self.keywords_range(&["IF", "NOT", "EXISTS"]);
        if let Some(range) = if_not_exists {
            if or_replace.is_some() || self.in_module {
                let message = match self.in_module {
                    true => "a module creates each object once, so no IF NOT EXISTS",
                    false => "OR REPLACE and IF NOT EXISTS don't go together",
                };
                self.error(DiagnosticCode::InvalidStatement, range, message);
            }
        }
        mode.complete(self, CREATE_MODE);
        let Some(object) = object else {
            self.skip_until(|p| p.current() == SEMICOLON);
            return marker.complete(self, CREATE_TABLE_STMT);
        };
        self.record_name(match object {
            Object::Table => NameRole::Table,
            Object::Function(_) => NameRole::Function,
            Object::Constant => NameRole::Variable,
        });
        let name = match object {
            Object::Table => self.table_name(),
            _ => self.definition_path(),
        };
        if let (Some((range, parts)), true) = (name, self.in_module) {
            if parts > 1 {
                self.error(
                    DiagnosticCode::InvalidStatement,
                    range,
                    "an object of a module has a one-part name",
                );
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

    /// The name of a table that a `CREATE TABLE` defines, which can hold dashes, and the range
    /// and the number of its names.
    fn table_name(&mut self) -> Option<(TextRange, usize)> {
        let (path, _) = self.table_path()?;
        let range = self.node_range(path);
        Some((range, self.path_parts(range)))
    }

    /// The path of a function or a constant that a `CREATE` defines.
    fn definition_path(&mut self) -> Option<(TextRange, usize)> {
        let (path, parts) = self.path(NameRole::Function)?;
        Some((self.node_range(path), parts))
    }

    /// The number of names in the path at `range`: its dots that the lexer read as tokens, plus
    /// one.
    fn path_parts(&self, range: TextRange) -> usize {
        (self.tokens.iter())
            .filter(|token| token.kind == DOT && range.contains_range(token.range))
            .count()
            + 1
    }

    /// The rest of `CREATE TABLE`, after its name.
    fn create_table(&mut self) {
        let start = self.range();
        let mut columns = false;
        if self.current() == L_PAREN {
            columns = true;
            let list = self.start();
            let opened = self.open_paren();
            if opened {
                self.comma_list(Self::column_definition, |p| p.current() == R_PAREN);
            }
            self.close_paren(opened);
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
        let query = self.eat_keyword("AS");
        if query {
            self.query();
        }
        if !columns && !query {
            let range = self.since(start);
            self.error(
                DiagnosticCode::InvalidStatement,
                range,
                "a CREATE TABLE takes columns, AS and a query, or both",
            );
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
        let opened = self.open_paren();
        if opened && self.current() != R_PAREN {
            self.comma_list(
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
                |_| false,
            );
        }
        self.close_paren(opened);
        marker.complete(self, OPTIONS);
    }

    /// The rest of `CREATE [AGGREGATE | TABLE] FUNCTION`, after its name.
    fn create_function(&mut self, kind: FunctionKind) {
        let list = self.start();
        let opened = self.open_paren();
        if opened && self.current() != R_PAREN {
            self.comma_list(|p| p.parameter(kind), |_| false);
        }
        self.close_paren(opened);
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
                let opened = self.open_paren();
                if opened {
                    self.expr();
                }
                self.close_paren(opened);
                body.complete(self, FUNCTION_BODY);
                if !before {
                    self.options();
                }
            }
        }
    }

    /// `name type`, `name ANY TYPE`, or for a table function `name TABLE<columns>` or `name ANY
    /// TABLE`, or for an aggregate function `name type NOT AGGREGATE`.
    fn parameter(&mut self, kind: FunctionKind) {
        let marker = self.start();
        self.name(NameRole::Parameter);
        let type_start = self.range();
        let table = if self.at_keyword("ANY") && self.keyword_at(1, "TYPE") {
            let any = self.start();
            self.eat_keywords(&["ANY", "TYPE"]);
            any.complete(self, ANY_TYPE);
            false
        } else if self.at_keyword("ANY") && self.keyword_at(1, "TABLE") {
            let any = self.start();
            self.eat_keywords(&["ANY", "TABLE"]);
            any.complete(self, ANY_TABLE);
            true
        } else if self.keyword_at(0, "TABLE") && self.nth(1) == LT {
            let table = self.start();
            self.bump_keyword();
            self.bump();
            self.table_columns();
            table.complete(self, TABLE_TYPE);
            true
        } else {
            self.ty();
            false
        };
        if table && kind != FunctionKind::Table {
            let range = self.since(type_start);
            self.error(
                DiagnosticCode::InvalidStatement,
                range,
                "a table parameter belongs to a TABLE FUNCTION",
            );
        }
        if let Some(range) = self.keywords_range(&["NOT", "AGGREGATE"]) {
            if kind != FunctionKind::Aggregate {
                self.error(
                    DiagnosticCode::InvalidStatement,
                    range,
                    "NOT AGGREGATE belongs to a parameter of an AGGREGATE FUNCTION",
                );
            }
        }
        marker.complete(self, PARAMETER);
    }

    /// The columns of `TABLE<...>`, after `<` and through `>`, each a name and a type.
    fn table_columns(&mut self) {
        let start = self.previous_range();
        let mut unnamed = self.current() == GT;
        if !self.eat(GT) {
            loop {
                let named = self.at_ident() && !matches!(self.nth(1), COMMA | GT | L_PAREN | LT);
                let field = self.start();
                match named {
                    true => {
                        self.name(NameRole::Column);
                    }
                    false => unnamed = true,
                }
                self.ty();
                field.complete(self, STRUCT_FIELD);
                if !self.eat(COMMA) {
                    break;
                }
            }
            self.expect(GT, "`>`");
        }
        if unnamed {
            let range = self.since(start);
            self.error(
                DiagnosticCode::InvalidStatement,
                range,
                "the columns of a table each take a name and a type",
            );
        }
    }
}
