//! The validation pass: the checks that GoogleSQL makes after its parser, in its resolver or
//! module catalog, when they are about syntax, and the statements that each role of a text
//! holds. [`crate::parse`], [`crate::parse_module`] and [`crate::parse_data`] run it on every
//! tree.
//!
//! The pass reads the typed tree of [`crate::ast`] and checks:
//!
//! - the rules of modules: a `CREATE` in a module is `PUBLIC` or `PRIVATE`, takes no `OR
//!   REPLACE` or `IF NOT EXISTS`, names its object by one name, and creates no table, and a
//!   script's `CREATE` is not `PUBLIC` or `PRIVATE` and creates no constant;
//! - the rules of data scripts: a data script holds `CREATE EXTERNAL TABLE`, `LOAD DATA`,
//!   `CREATE VIEW`, `CREATE TABLE ... AS`, `CREATE SCHEMA`, `CREATE TEMP FUNCTION` and `IMPORT
//!   MODULE`, and no query, `DECLARE` or `SET`, while a module holds none of the statements of
//!   data scripts, and a script holds of them only `CREATE TEMP EXTERNAL TABLE` and `LOAD DATA
//!   ... TEMP TABLE`, since a script's tables are `TEMP`;
//! - `OR REPLACE` with `IF NOT EXISTS`;
//! - the order of `DECLARE`, which comes before a script's other statements, and that a
//!   `DECLARE` takes a type or a `DEFAULT`;
//! - that a `CREATE TABLE` takes columns or a query, and in a data script a query;
//! - the parameters of functions: a table parameter belongs to a table function, `NOT AGGREGATE`
//!   to an aggregate function, and the columns of `TABLE<...>` each take a name;
//! - sources: a table function's name has no dashes, `LATERAL` goes before a query or a table
//!   function call, `UNNEST` takes only the named argument `mode`, a cross join takes no
//!   condition, and a `RIGHT` or `FULL` join after a comma join needs parentheses;
//! - set operations: a mode goes with `BY NAME` or `CORRESPONDING`, and not with `STRICT`;
//! - calls: a `*` is a call's only argument;
//! - literals: a typed literal takes a string, a string and a bytes literal don't concatenate,
//!   and the escapes of strings, bytes literals and quoted names are valid, the value of a
//!   string is UTF-8, and a quoted name isn't empty.

use rowan::{TextRange, TextSize};

use crate::{
    ast::{self, tokens, AstNode, CreateStmt, FunctionKind, Scope},
    diagnostic::{Diagnostic, DiagnosticCode},
    escape, Role,
    SyntaxKind::{self, *},
    SyntaxNode, SyntaxToken,
};

/// The help of a statement of a data script in a script or a module.
const IN_DATA_SCRIPT: &str =
    "a project's data scripts register the tables, views and schemas that its scripts read";

/// The help of a script's `CREATE EXTERNAL TABLE` without `TEMP`.
const EXTERNAL_TEMP: &str = "a chart script reads its own files with `CREATE TEMP EXTERNAL TABLE t OPTIONS (...)`, and a project's data scripts register the tables that its scripts read";

/// The help of a script's `LOAD DATA` without `TEMP TABLE`.
const LOAD_TEMP: &str = "a chart script loads its own files with `LOAD DATA OVERWRITE TEMP TABLE t FROM FILES (...)`, and a project's data scripts register the tables that its scripts read";

/// The help of a statement of a script in a data script.
const IN_SCRIPT: &str =
    "a data script only registers tables, and queries, DECLARE and SET go in a chart script";

/// The diagnostics of the checks of the validation pass on the tree `root` of a text of `role`,
/// a script of cells that start at `cells`, or none.
pub(crate) fn validate(root: &SyntaxNode, role: Role, cells: &[TextSize]) -> Vec<Diagnostic> {
    let mut validator = Validator {
        in_module: role == Role::Module,
        in_data: role == Role::Data,
        diagnostics: Vec::new(),
    };
    if let (Some(script), Role::Script) = (ast::Script::cast(root.clone()), role) {
        validator.declare_order(script.stmts(), cells);
    }
    for node in root.descendants() {
        validator.node(&node);
    }
    for token in root
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
    {
        validator.quoted(&token);
    }
    validator.diagnostics
}

struct Validator {
    in_module: bool,
    in_data: bool,
    diagnostics: Vec<Diagnostic>,
}

/// `node` as the typed node of its kind.
fn cast<N: AstNode>(node: &SyntaxNode) -> N {
    N::cast(node.clone()).expect("a node of its kind")
}

/// The range from the first token of `node` whose kind is in `first` through the first token
/// of `last` from there.
fn tokens_range(node: &SyntaxNode, first: &[SyntaxKind], last: SyntaxKind) -> Option<TextRange> {
    let mut parts = tokens(node).skip_while(|token| !first.contains(&token.kind()));
    let start = parts.next()?;
    let end = match start.kind() == last {
        true => start.clone(),
        false => parts.find(|token| token.kind() == last)?,
    };
    Some(start.text_range().cover(end.text_range()))
}

/// The start of the first token after `node` that isn't trivia, or the end of the text.
fn next_token_start(node: &SyntaxNode) -> TextSize {
    let mut token = node.last_token();
    while let Some(next) = token.and_then(|token| token.next_token()) {
        if !next.kind().is_trivia() {
            return next.text_range().start();
        }
        token = Some(next);
    }
    node.ancestors()
        .last()
        .map_or(node.text_range().end(), |root| root.text_range().end())
}

impl Validator {
    fn error(&mut self, code: DiagnosticCode, range: TextRange, message: &str) -> &mut Diagnostic {
        self.diagnostics.push(Diagnostic::new(code, range, message));
        self.diagnostics
            .last_mut()
            .expect("a diagnostic was pushed")
    }

    fn node(&mut self, node: &SyntaxNode) {
        match node.kind() {
            DECLARE_STMT if self.in_data => {
                self.in_script(node, DECLARE_KW, "a data script declares no variables")
            }
            DECLARE_STMT => self.declare(node),
            SET_STMT if self.in_data => {
                self.in_script(node, SET_KW, "a data script sets no variables")
            }
            QUERY if self.in_data && node.parent().is_some_and(|p| p.kind() == SCRIPT) => {
                self.error(
                    DiagnosticCode::InvalidStatement,
                    cast::<ast::Query>(node).range(),
                    "a data script runs no queries",
                )
                .help(IN_SCRIPT);
            }
            CREATE_TABLE_STMT => {
                let statement = cast::<ast::CreateTableStmt>(node);
                if self.permanent_table(&statement) {
                    return;
                }
                self.create(&statement, &[TABLE_KW]);
                self.create_table(&statement);
            }
            CREATE_FUNCTION_STMT => {
                let statement = cast::<ast::CreateFunctionStmt>(node);
                let object = [AGGREGATE_KW, TABLE_KW, FUNCTION_KW];
                self.create(&statement, &object);
                if let (true, None) = (self.in_data, statement.scope_token()) {
                    if let Some(range) = tokens_range(node, &object, FUNCTION_KW) {
                        self.error(
                            DiagnosticCode::InvalidStatement,
                            range,
                            "a function of a data script is TEMP",
                        )
                        .help(
                            "a data script's functions are its own, and the functions that scripts share go in a module",
                        );
                    }
                }
            }
            CREATE_EXTERNAL_TABLE_STMT => {
                let statement = cast::<ast::CreateExternalTableStmt>(node);
                if self.in_chart() && statement.scope_kind() == Some(Scope::Temp) {
                    self.create(&statement, &[EXTERNAL_KW, TABLE_KW]);
                    return;
                }
                self.data_statement(&statement, &[EXTERNAL_KW, TABLE_KW], "an EXTERNAL TABLE");
            }
            CREATE_VIEW_STMT => {
                let statement = cast::<ast::CreateViewStmt>(node);
                self.data_statement(&statement, &[VIEW_KW], "a VIEW");
            }
            CREATE_SCHEMA_STMT => {
                let statement = cast::<ast::CreateSchemaStmt>(node);
                self.data_statement(&statement, &[SCHEMA_KW], "a SCHEMA");
            }
            LOAD_DATA_STMT if !self.in_data => {
                let load = cast::<ast::LoadDataStmt>(node);
                let Some(range) = tokens_range(node, &[LOAD_KW], DATA_KW) else {
                    return;
                };
                match (self.in_module, load.scope().is_some()) {
                    (false, true) => {}
                    // At the table's name, where `TEMP TABLE` goes.
                    (false, false) => {
                        if let Some(path) = load.path() {
                            self.error(
                                DiagnosticCode::Unsupported,
                                path.syntax().text_range(),
                                "LOAD DATA without TEMP TABLE belongs to a data script",
                            )
                            .help(LOAD_TEMP);
                        }
                    }
                    (true, _) => {
                        self.error(
                            DiagnosticCode::Unsupported,
                            range,
                            "LOAD DATA belongs to a data script",
                        )
                        .help(IN_DATA_SCRIPT);
                    }
                }
            }
            CREATE_CONSTANT_STMT => {
                let statement = cast::<ast::CreateConstantStmt>(node);
                self.create(&statement, &[CONSTANT_KW]);
            }
            PARAMETER => self.parameter(cast::<ast::Parameter>(node)),
            TABLE_TYPE => self.table_type(node),
            TVF_SOURCE => self.tvf_source(cast::<ast::TvfSource>(node)),
            TABLE_SOURCE => {
                if let Some(lateral) = cast::<ast::TableSource>(node).lateral_kw() {
                    self.error(
                        DiagnosticCode::Expected,
                        lateral.text_range(),
                        "LATERAL comes before a query in parentheses or a table function call",
                    );
                }
            }
            UNNEST_SOURCE => self.unnest(cast::<ast::UnnestSource>(node)),
            JOIN => self.join(cast::<ast::Join>(node)),
            PIPE_JOIN => {
                let join = cast::<ast::PipeJoin>(node);
                self.join_condition(join.join_op(), join.join_condition());
            }
            PIPE_SET_OPERATION => self.set_operation(cast::<ast::PipeSetOperation>(node)),
            ARG_LIST => self.arguments(cast::<ast::ArgList>(node)),
            LITERAL | TYPED_LITERAL => self.literal(node),
            _ => {}
        }
    }

    /// As in GoogleSQL, every `DECLARE` of a script comes before its other statements, and in a
    /// script of cells, which start at `cells`, before the other statements of its cell.
    fn declare_order(&mut self, statements: impl Iterator<Item = ast::Stmt>, cells: &[TextSize]) {
        let mut after_others = false;
        let mut cell = 0;
        for statement in statements {
            let start = statement.range().start();
            while cell < cells.len() && cells[cell] <= start {
                cell += 1;
                after_others = false;
            }
            let declare = matches!(statement, ast::Stmt::DeclareStmt(_));
            if declare && after_others {
                let message = match cells.is_empty() {
                    true => "a DECLARE comes before the other statements of a script",
                    false => "a DECLARE comes before the other statements of its cell",
                };
                self.error(DiagnosticCode::DeclareOrder, statement.range(), message);
            }
            after_others |= !declare;
        }
    }

    /// Reports a statement of a script, whose first keyword is `keyword`, in a data script.
    fn in_script(&mut self, node: &SyntaxNode, keyword: SyntaxKind, message: &str) {
        if let Some(token) = tokens(node).find(|token| token.kind() == keyword) {
            self.error(
                DiagnosticCode::InvalidStatement,
                token.text_range(),
                message,
            )
            .help(IN_SCRIPT);
        }
    }

    /// Whether the text is a chart script.
    fn in_chart(&self) -> bool {
        !self.in_module && !self.in_data
    }

    /// Checks a statement of a data script, `name` of the object of the keywords `object`: in
    /// a data script, the modifiers of its `CREATE`, and in a script or a module, the statement
    /// itself, with one error. GoogleSQL takes these statements in a script, and avenger reads
    /// them in a data script. A script's external table without `TEMP` has the help of the
    /// script's own form.
    fn data_statement(&mut self, statement: &impl CreateStmt, object: &[SyntaxKind], name: &str) {
        if self.in_data {
            self.create(statement, object);
            return;
        }
        let last = *object.last().expect("an object's keywords");
        let Some(range) = tokens_range(statement.syntax(), object, last) else {
            return;
        };
        match (self.in_chart(), statement.syntax().kind()) {
            (true, CREATE_EXTERNAL_TABLE_STMT) => {
                self.error(
                    DiagnosticCode::Unsupported,
                    range,
                    &format!("{name} without TEMP belongs to a data script"),
                )
                .help(EXTERNAL_TEMP);
            }
            _ => {
                self.error(
                    DiagnosticCode::Unsupported,
                    range,
                    &format!("{name} belongs to a data script"),
                )
                .help(IN_DATA_SCRIPT);
            }
        }
    }

    /// Whether `statement` is a script's `CREATE TABLE` without a scope, which it reports: a
    /// table that the charts read belongs to a data script, and a script's own tables are
    /// `TEMP`.
    fn permanent_table(&mut self, statement: &ast::CreateTableStmt) -> bool {
        if self.in_module || self.in_data || statement.scope_token().is_some() {
            return false;
        }
        let Some(table) = statement.table_kw() else {
            return false;
        };
        self.error(
            DiagnosticCode::Unsupported,
            table.text_range(),
            "a table without TEMP belongs to a data script",
        )
        .help("a script creates temporary tables, as in `CREATE TEMP TABLE t AS (FROM u)`, and a project's data scripts the tables that its scripts read");
        true
    }

    fn declare(&mut self, node: &SyntaxNode) {
        let declare = cast::<ast::DeclareStmt>(node);
        if declare.ty().is_none() && declare.default_kw().is_none() {
            // At the end of the names, where GoogleSQL expects the type.
            self.error(
                DiagnosticCode::InvalidStatement,
                TextRange::empty(declare.range().end()),
                "a DECLARE takes a type, a DEFAULT, or both",
            );
        }
    }

    /// The modifiers of a `CREATE` whose object's keywords are `object`, against the rules of
    /// scripts and modules.
    fn create(&mut self, statement: &impl CreateStmt, object: &[SyntaxKind]) {
        let node = statement.syntax();
        let or_replace = tokens_range(node, &[OR_KW], REPLACE_KW);
        if let (Some(range), true) = (or_replace, self.in_module) {
            self.error(
                DiagnosticCode::InvalidStatement,
                range,
                "a module creates each object once, so no OR REPLACE",
            );
        }
        let scope_token = statement.scope_token();
        let object_range = tokens_range(node, object, *object.last().unwrap());
        let public = matches!(statement.scope_kind(), Some(Scope::Public | Scope::Private));
        if self.in_module && !public {
            // The range of the scope, or without one, of the object's first keyword.
            let range = scope_token
                .as_ref()
                .or(tokens(node)
                    .find(|token| object.contains(&token.kind()))
                    .as_ref())
                .map(|token| token.text_range());
            if let Some(range) = range {
                self.error(
                    DiagnosticCode::InvalidStatement,
                    range,
                    "a CREATE in a module is PUBLIC or PRIVATE",
                )
                .help(
                    "PUBLIC objects are visible to the scripts and modules that import the module",
                );
            }
        }
        // GoogleSQL takes them in a script, and avenger reads them in a module.
        if let (false, true, Some(token)) = (self.in_module, public, &scope_token) {
            self.error(
                DiagnosticCode::Unsupported,
                token.text_range(),
                "PUBLIC and PRIVATE belong to the CREATE statements of a module",
            );
        }
        if let Some(range) = object_range {
            match (node.kind(), self.in_module) {
                (CREATE_TABLE_STMT, true) => {
                    self.error(
                        DiagnosticCode::InvalidStatement,
                        range,
                        "a module creates functions, table functions and constants, not tables",
                    )
                    .help(IN_DATA_SCRIPT);
                }
                (CREATE_CONSTANT_STMT, false) => {
                    let in_data = self.in_data;
                    let error = self.error(
                        DiagnosticCode::Unsupported,
                        range,
                        "a CONSTANT belongs to a module",
                    );
                    if !in_data {
                        error.help("a script declares a variable, as in `DECLARE x DEFAULT 1`");
                    }
                }
                _ => {}
            }
        }
        if let Some(range) = tokens_range(node, &[IF_KW], EXISTS_KW) {
            if or_replace.is_some() || self.in_module {
                let message = match self.in_module {
                    true => "a module creates each object once, so no IF NOT EXISTS",
                    false => "OR REPLACE and IF NOT EXISTS don't go together",
                };
                self.error(DiagnosticCode::InvalidStatement, range, message);
            }
        }
        if let (true, Some(path)) = (self.in_module, statement.name_path()) {
            if path.name_refs().nth(1).is_some() {
                self.error(
                    DiagnosticCode::InvalidStatement,
                    path.range(),
                    "an object of a module has a one-part name",
                );
            }
        }
    }

    /// A `CREATE TABLE` takes columns or a query, and in a data script, which gives a table its
    /// rows, a query.
    fn create_table(&mut self, statement: &ast::CreateTableStmt) {
        let columns = statement.column_list().is_some() && !self.in_data;
        if columns || statement.as_kw().is_some() {
            return;
        }
        let range = match statement.options() {
            Some(options) => options.range(),
            None => TextRange::empty(next_token_start(statement.syntax())),
        };
        if self.in_data {
            self.error(
                DiagnosticCode::InvalidStatement,
                range,
                "a table of a data script takes AS and a query",
            )
            .help("a data script gives a table its rows with AS or LOAD DATA, and a table of columns alone goes in a chart script as a CREATE TEMP TABLE");
            return;
        }
        // GoogleSQL declares a table without either.
        self.error(
            DiagnosticCode::Unsupported,
            range,
            "avenger-sql's CREATE TABLE takes columns, AS and a query, or both",
        );
    }

    fn parameter(&mut self, parameter: ast::Parameter) {
        let Some(function) = parameter
            .syntax()
            .ancestors()
            .find_map(ast::CreateFunctionStmt::cast)
        else {
            return;
        };
        let kind = function.function_kind();
        if let Some(ty) = parameter.param_type() {
            let table = matches!(
                ty,
                ast::ParamType::AnyTable(_) | ast::ParamType::TableType(_)
            );
            if table && kind != FunctionKind::Table {
                self.error(
                    DiagnosticCode::InvalidStatement,
                    ty.range(),
                    "a table parameter belongs to a TABLE FUNCTION",
                );
            }
        }
        if let Some(range) = tokens_range(parameter.syntax(), &[NOT_KW], AGGREGATE_KW) {
            if kind != FunctionKind::Aggregate {
                self.error(
                    DiagnosticCode::InvalidStatement,
                    range,
                    "NOT AGGREGATE belongs to a parameter of an AGGREGATE FUNCTION",
                );
            }
        }
    }

    /// The columns of a table each take a name and a type.
    fn table_type(&mut self, node: &SyntaxNode) {
        let table = cast::<ast::TableType>(node);
        let fields: Vec<_> = table.struct_fields().collect();
        if !fields.is_empty() && fields.iter().all(|field| field.name().is_some()) {
            return;
        }
        let Some(lt) = table.lt() else { return };
        let range = TextRange::new(lt.text_range().start(), table.range().end());
        self.error(
            DiagnosticCode::InvalidStatement,
            range,
            "the columns of a table each take a name and a type",
        );
    }

    fn tvf_source(&mut self, source: ast::TvfSource) {
        let Some(path) = source.path() else { return };
        let dashed = path
            .name_refs()
            .next()
            .is_some_and(|name| tokens(name.syntax()).any(|token| token.kind() == MINUS));
        if dashed {
            self.error(
                DiagnosticCode::Expected,
                path.range(),
                "a table function's name has no dashes",
            )
            .help("quote the name in backticks");
        }
    }

    fn unnest(&mut self, unnest: ast::UnnestSource) {
        let Some(name) = unnest.named_arg().and_then(|arg| arg.name_ref()) else {
            return;
        };
        if !name
            .syntax()
            .text()
            .to_string()
            .eq_ignore_ascii_case("mode")
        {
            self.error(
                DiagnosticCode::Expected,
                name.range(),
                "UNNEST takes one named argument, `mode`",
            );
        }
    }

    fn join(&mut self, join: ast::Join) {
        self.join_condition(join.join_op(), join.join_condition());
        let Some(op) = join.join_op() else { return };
        if op.right_kw().is_none() && op.full_kw().is_none() {
            return;
        }
        // As in GoogleSQL, a RIGHT or FULL join after a comma join in the same FROM needs
        // parentheses. The comma joins of the left operand are its leftmost joins.
        let mut left = join.lhs();
        while let Some(ast::Source::Join(inner)) = left {
            if inner.join_op().is_some_and(|op| op.comma().is_some()) {
                self.error(
                    DiagnosticCode::Expected,
                    op.range(),
                    "a RIGHT or FULL JOIN after a comma join needs parentheses",
                )
                .help("put the comma join in parentheses, as in `FROM (a, b) RIGHT JOIN c ON ...`");
                return;
            }
            left = inner.lhs();
        }
    }

    /// A cross join takes no condition.
    fn join_condition(&mut self, op: Option<ast::JoinOp>, condition: Option<ast::JoinCondition>) {
        let (Some(op), Some(condition)) = (op, condition) else {
            return;
        };
        if op.cross_kw().is_some() {
            self.error(
                DiagnosticCode::Expected,
                condition.range(),
                "a CROSS JOIN takes no condition",
            )
            .help("an INNER JOIN takes ON or USING");
        }
    }

    fn set_operation(&mut self, operation: ast::PipeSetOperation) {
        let Some(mode) = operation.set_op_mode() else {
            return;
        };
        match (operation.by_name(), operation.corresponding()) {
            (None, None) => {
                self.error(
                    DiagnosticCode::Expected,
                    mode.range(),
                    "the mode of a set operation goes with BY NAME or CORRESPONDING",
                );
            }
            (_, Some(corresponding)) => {
                if let Some(strict) = corresponding.strict_kw() {
                    self.error(
                        DiagnosticCode::Expected,
                        strict.text_range(),
                        "STRICT can't go with the mode of a set operation",
                    );
                }
            }
            _ => {}
        }
    }

    fn arguments(&mut self, arguments: ast::ArgList) {
        let Some(star) = arguments.star_arg() else {
            return;
        };
        if arguments.arguments().next().is_some() {
            self.error(
                DiagnosticCode::Expected,
                star.range(),
                "`*` is a call's only argument, as in `COUNT(*)`",
            );
        }
    }

    /// The strings and bytes literals of a literal, which don't mix, and of a typed literal,
    /// which takes strings, when every part is closed.
    fn literal(&mut self, node: &SyntaxNode) {
        let parts: Vec<SyntaxToken> = tokens(node)
            .filter(|token| matches!(token.kind(), STRING | BYTES))
            .collect();
        let Some(first) = parts.first() else { return };
        // A part whose closing quote is missing has the lexer's error, and the parts after it
        // are often text that the open quote took for a literal, so they get no others.
        if parts
            .iter()
            .any(|part| escape::split(part.text()).is_none())
        {
            return;
        }
        if node.kind() == TYPED_LITERAL && first.kind() == BYTES {
            self.error(
                DiagnosticCode::Expected,
                first.text_range(),
                "a typed literal takes a string, not bytes",
            );
        }
        for pair in parts.windows(2) {
            let adjacent = pair[0].text_range().end() == pair[1].text_range().start();
            if !adjacent && pair[1].kind() != first.kind() {
                self.error(
                    DiagnosticCode::InvalidToken,
                    pair[1].text_range(),
                    "a string literal and a bytes literal don't concatenate",
                );
            }
        }
    }

    /// The escapes of a string, a bytes literal or a quoted name, its value as UTF-8 text, and
    /// for a name, that it isn't empty. A token whose closing quote is missing has the lexer's
    /// error instead.
    fn quoted(&mut self, token: &SyntaxToken) {
        let kind = token.kind();
        if !matches!(kind, STRING | BYTES | QUOTED_IDENT) {
            return;
        }
        let Some(quoted) = escape::split(token.text()) else {
            return;
        };
        let range = token.text_range();
        match escape::unescape(quoted.body, quoted.raw, kind == BYTES) {
            Ok(value) => {
                if kind == QUOTED_IDENT && value.is_empty() {
                    self.error(
                        DiagnosticCode::InvalidToken,
                        range,
                        "a quoted identifier can't be empty",
                    );
                } else if kind != BYTES && String::from_utf8(value).is_err() {
                    self.error(
                        DiagnosticCode::InvalidToken,
                        range,
                        "this string's escapes don't form UTF-8 text",
                    );
                }
            }
            Err(error) => {
                let at = range.start() + TextSize::from((quoted.body_offset + error.offset) as u32);
                self.error(
                    DiagnosticCode::InvalidToken,
                    TextRange::at(at, TextSize::from(error.len as u32)),
                    &error.message,
                );
            }
        }
    }
}
