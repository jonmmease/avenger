//! Completion: what the grammar takes at a cursor, filled with the names that the analysis and
//! the catalog know there.

use std::collections::HashSet;

use avenger_sql::{
    analysis::{
        match_recognize_part, Column, Definition, OperatorScope, Place, PlaceKind, RangeVariable,
    },
    library::{entry, Accept, Arity, Family, FormArg, Function, FunctionKind, Need, Over},
    types::googlesql_name,
};
use avenger_sql_syntax::{
    ast::{self, AstNode, CreateStmt},
    expected_at, Expected, FunctionRole, NameRole, SyntaxKind, SyntaxNode, SyntaxToken, TextRange,
};
use datafusion::arrow::datatypes::DataType;

use crate::{
    escape,
    hover::{callee, create_header},
    quote, statements, token_before, Document, Offset,
};

/// The items of a completion, and the ranges of the text that an item replaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completions {
    /// The text that an item replaces when an editor inserts it at the cursor: the part of the
    /// name before the cursor, without the opening backtick of a quoted name, or the string
    /// literal that holds the cursor.
    pub range: TextRange,
    /// The text that an item replaces when an editor replaces the whole name at the cursor: the
    /// name through its end, or a quoted name up to its closing backtick. It starts where `range`
    /// starts.
    pub replace: TextRange,
    /// The items, in the order of their sort keys.
    pub items: Vec<CompletionItem>,
}

/// An item of a completion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    /// A type, or the summary of a function.
    pub detail: Option<String>,
    /// Markdown on a function: the text of its hover.
    pub documentation: Option<String>,
    /// The text that replaces the completion's range.
    pub insert: String,
    /// The key that orders the items: columns, then variables, then functions, then keywords.
    pub sort: String,
}

/// What an item of a completion is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompletionKind {
    Keyword,
    Column,
    RangeVariable,
    Variable,
    /// A field of a struct.
    Field,
    /// A table of the catalog, or a table parameter of a table function in its body.
    Table,
    /// A pipe `WITH` name or a temporary table.
    Query,
    /// A function, by what a call of it computes.
    Function(FunctionRole),
    /// The name of a parameter, as `name => ` of a named argument.
    Parameter,
    /// A value of a set that a parameter takes, as a string literal.
    EnumValue,
    Type,
    /// A named window of a `WINDOW` clause.
    Window,
}

/// The rank of each kind of item, the first part of its sort key.
mod rank {
    pub const ARGUMENT: u8 = 0;
    pub const COLUMN: u8 = 1;
    pub const OUTER_COLUMN: u8 = 2;
    pub const VARIABLE: u8 = 3;
    pub const FUNCTION: u8 = 4;
    pub const KEYWORD: u8 = 5;
}

/// The keywords that start a statement of a script.
const STATEMENTS: &[&str] = &[
    "FROM",
    "DECLARE",
    "SET",
    "CREATE TEMP TABLE",
    "CREATE TEMP FUNCTION",
    "IMPORT MODULE",
];

/// The types of avenger-sql.
const TYPES: &[&str] = &[
    "INT64",
    "FLOAT64",
    "NUMERIC",
    "BIGNUMERIC",
    "BOOL",
    "STRING",
    "BYTES",
    "DATE",
    "TIME",
    "DATETIME",
    "TIMESTAMP",
    "INTERVAL",
    "ARRAY",
    "STRUCT",
];

/// The items of a completion, without repeats of a label and a kind.
struct Items(Vec<CompletionItem>, HashSet<(String, CompletionKind)>);

impl Items {
    fn push(&mut self, item: CompletionItem) {
        if self.1.insert((item.label.clone(), item.kind)) {
            self.0.push(item);
        }
    }

    fn simple(&mut self, label: String, kind: CompletionKind, detail: Option<String>, rank: u8) {
        self.push(CompletionItem {
            sort: sort(rank, &label),
            insert: label.clone(),
            label,
            kind,
            detail,
            documentation: None,
        });
    }
}

fn sort(rank: u8, label: &str) -> String {
    format!("{rank}{}", label.to_ascii_lowercase())
}

impl Document {
    /// The items that the text takes at `offset`, and the range of the text that they replace.
    pub fn completions(&self, offset: Offset) -> Completions {
        let offset = self.clamp(offset);
        let root = self.root();
        let mut items = Items(Vec::new(), HashSet::new());

        // In a string literal, the values of the set that its parameter takes, if any.
        if let Some(string) = (root.token_at_offset(offset).left_biased()).filter(|token| {
            token.kind() == SyntaxKind::STRING && token.text_range().start() < offset
        }) {
            self.enum_values(&string, &mut items);
            return finish(string.text_range(), string.text_range(), items, None);
        }

        // In a quoted name, the name after its opening backtick. An unclosed quoted name runs to
        // the end of its line, so the name ends where its word ends.
        let quoted = (root.token_at_offset(offset).left_biased()).and_then(|token| {
            let range = token.text_range();
            let closed = token.text().len() > 1 && token.text().ends_with('`');
            let inside = token.kind() == SyntaxKind::QUOTED_IDENT
                && range.start() < offset
                && (offset < range.end() || !closed);
            inside.then_some((range, closed))
        });
        let (start, range, replace) = match quoted {
            Some((token, closed)) => {
                let open = token.start() + Offset::from(1);
                let end = match closed {
                    true => token.end() - Offset::from(1),
                    false => word_end(&self.text, offset),
                };
                (
                    token.start(),
                    TextRange::new(open, offset),
                    TextRange::new(open, end),
                )
            }
            None => {
                let start = word_start(&self.text, offset);
                let end = word_end(&self.text, offset);
                (
                    start,
                    TextRange::new(start, offset),
                    TextRange::new(start, end),
                )
            }
        };
        let quote = quoted.map(|(_, closed)| closed);
        let before = token_before(&root, start);
        let scope = before.as_ref().and_then(|token| self.scope_at(token));

        if let Some(dot) = before
            .as_ref()
            .filter(|token| token.kind() == SyntaxKind::DOT)
        {
            self.fields(dot, scope, &mut items);
            return finish(range, replace, items, quote);
        }

        // After a named argument, or past the positional arguments of every form, an argument
        // takes only a name.
        if let Some(before) = &before {
            if self.only_named(before) {
                self.parameters(before, &mut items);
                return finish(range, replace, items, quote);
            }
        }

        let place = before
            .as_ref()
            .and_then(|token| self.analysis.model.place_at(token.text_range().start()));
        let mut expression = false;
        let mut names = false;
        let mut tables = false;
        let mut table_functions = false;
        for expected in expected_at(&self.text, start) {
            match expected {
                Expected::Keyword(kind) => {
                    if before
                        .as_ref()
                        .is_none_or(|before| self.keyword_fits(kind, before))
                    {
                        items.simple(keyword(kind), CompletionKind::Keyword, None, rank::KEYWORD)
                    }
                }
                // A `PIVOT` takes the call of an aggregate, which no word of an expression starts,
                // and an item of `|> AGGREGATE` computes from an aggregate, which no constant does.
                Expected::Words(_) if place.is_some_and(|p| p.kind == PlaceKind::Pivot) => {}
                Expected::Words(words)
                    if words == avenger_sql_syntax::LITERAL_WORDS
                        && place.is_some_and(|p| p.kind == PlaceKind::AggregateItem) => {}
                Expected::Words(words) => {
                    for word in words {
                        items.simple(
                            word.to_string(),
                            CompletionKind::Keyword,
                            None,
                            rank::KEYWORD,
                        );
                    }
                }
                Expected::Phrase(phrase) => {
                    items.simple(
                        phrase.join(" "),
                        CompletionKind::Keyword,
                        None,
                        rank::KEYWORD,
                    );
                }
                Expected::Name(NameRole::Window) => {
                    if let Some(before) = &before {
                        windows(before, &mut items);
                    }
                }
                Expected::Expression => expression = true,
                // A name of a column, which no expression can take the place of, as in a list of
                // the columns of `UNPIVOT`.
                Expected::Name(NameRole::Column) => names = true,
                Expected::Name(NameRole::Table) => tables = true,
                Expected::Name(NameRole::Function(FunctionRole::Table)) => table_functions = true,
                Expected::Name(NameRole::Variable) => self.variables(&root, start, &mut items),
                Expected::Name(NameRole::Parameter) => {
                    if let Some(before) = &before {
                        self.parameters(before, &mut items);
                    }
                }
                Expected::Type => {
                    for ty in TYPES {
                        items.simple(ty.to_string(), CompletionKind::Type, None, rank::KEYWORD);
                    }
                }
                Expected::Statement => {
                    // A `DECLARE` comes before the other statements of a script.
                    let declares = statements(&root)
                        .filter(|stmt| stmt.syntax().text_range().end() <= start)
                        .all(|stmt| matches!(stmt, ast::Stmt::DeclareStmt(_)));
                    for keyword in STATEMENTS {
                        if *keyword != "DECLARE" || declares {
                            items.simple(
                                keyword.to_string(),
                                CompletionKind::Keyword,
                                None,
                                rank::KEYWORD,
                            );
                        }
                    }
                }
                // The parser records the words of each pipe operator.
                Expected::PipeOperator | Expected::Token(_) | Expected::Name(_) => {}
            }
        }
        if tables {
            self.tables(&root, start, scope, &mut items);
        }
        if tables || table_functions {
            self.table_functions(&root, start, &mut items);
        }
        if names && !expression {
            if let Some(scope) = scope.filter(|_| place.is_none_or(Place::takes_columns)) {
                self.columns(scope, None, false, &mut items);
            }
        }
        if expression {
            let calls = place.is_none_or(Place::takes_calls);
            if let Some(scope) = scope.filter(|_| place.is_none_or(Place::takes_columns)) {
                let expected = place.and_then(|place| place.expected.as_ref());
                let range_variables = place.is_none_or(Place::takes_calls);
                self.columns(scope, expected, range_variables, &mut items);
            }
            if calls {
                self.variables(&root, start, &mut items);
                let mark = before.as_ref().is_some_and(|before| self.in_mark(before));
                self.functions(&root, start, place, mark, &mut items);
            }
            if let Some(before) = &before {
                self.argument_values(before, &mut items);
            }
        }
        finish(range, replace, items, quote)
    }

    /// The scope of the innermost operator that holds `token`, by its start: a token that ends
    /// where an operator starts, as `(` before a subquery, is outside it.
    fn scope_at(&self, token: &SyntaxToken) -> Option<&OperatorScope> {
        self.analysis.model.operator_at(token.text_range().start())
    }

    /// The columns of `scope`'s input, its range variables, and the columns and the range
    /// variables of the enclosing queries, after those of the scope.
    fn columns(
        &self,
        scope: &OperatorScope,
        expected: Option<&DataType>,
        range_variables: bool,
        items: &mut Items,
    ) {
        // The scope of a `FROM` clause has no input: its expressions, as the conditions of its
        // joins, read its output.
        let own = match scope.input.is_empty() {
            true => &scope.output,
            false => &scope.input,
        };
        let mut names = HashSet::new();
        let mut variables = HashSet::new();
        let named: Vec<&Column> = own
            .iter()
            .filter(|column| !column.name.is_empty())
            .collect();
        for column in &named {
            // A name that several columns have is ambiguous by itself, so each column goes with
            // its range variable, as `u.s` and `w.s`.
            let same = (named.iter())
                .filter(|other| other.name.eq_ignore_ascii_case(&column.name))
                .count();
            match (same > 1, column.qualifiers.first()) {
                (true, Some(qualifier)) => {
                    let mut item = column_item(column, rank::COLUMN, expected);
                    item.label = format!("{}.{}", qualifier, column.name);
                    item.insert = format!("{}.{}", quote(qualifier), quote(&column.name));
                    items.push(item);
                }
                (true, None) => {}
                (false, _) => {
                    if names.insert(column.name.to_ascii_lowercase()) {
                        items.push(column_item(column, rank::COLUMN, expected));
                    }
                }
            }
        }
        for range_variable in &scope.range_variables {
            // The rows of a value table, whose range variable has no name, hold the fields of
            // its struct as columns.
            if range_variable.name.is_empty() {
                for column in range_variable.columns.iter().filter(|c| !c.name.is_empty()) {
                    if names.insert(column.name.to_ascii_lowercase()) {
                        items.push(column_item(column, rank::COLUMN, expected));
                    }
                }
                continue;
            }
            if range_variables && variables.insert(range_variable.name.to_ascii_lowercase()) {
                items.push(range_variable_item(range_variable));
            }
        }
        // The columns and the range variables of the enclosing queries, which a correlated
        // subquery reads.
        for column in scope.outer.iter().filter(|column| !column.name.is_empty()) {
            if names.insert(column.name.to_ascii_lowercase()) {
                items.push(column_item(column, rank::OUTER_COLUMN, expected));
            }
        }
        for range_variable in scope
            .outer_range_variables
            .iter()
            .filter(|r| !r.name.is_empty())
        {
            if range_variables && variables.insert(range_variable.name.to_ascii_lowercase()) {
                items.push(range_variable_item(range_variable));
            }
        }
    }

    /// After `name.`, the columns of the range variable `name`, or the fields of the struct
    /// column `name`.
    fn fields(&self, dot: &SyntaxToken, scope: Option<&OperatorScope>, items: &mut Items) {
        let Some(scope) = scope else {
            if let Some(fields) = (self.analysis.model.field_base_at(dot.text_range().start()))
                .and_then(struct_fields)
            {
                push_fields(fields, items);
            }
            return;
        };
        let Some(qualifier) = dot.prev_token().filter(|token| !token.kind().is_trivia()) else {
            return;
        };
        let qualifier = unquote(qualifier.text());
        // The range variable of the operator, or else of an enclosing query.
        if let Some(range_variable) = (scope.range_variables.iter())
            .chain(&scope.outer_range_variables)
            .find(|range_variable| range_variable.name.eq_ignore_ascii_case(&qualifier))
        {
            for column in range_variable
                .columns
                .iter()
                .filter(|column| !column.name.is_empty())
            {
                items.push(column_item(column, rank::COLUMN, None));
            }
            return;
        }
        let own = match scope.input.is_empty() {
            true => &scope.output,
            false => &scope.input,
        };
        // The struct of a column, or of the expression before the `.`.
        let ty = (own.iter())
            .find(|column| column.name.eq_ignore_ascii_case(&qualifier))
            .and_then(|column| column.ty.as_ref())
            .or_else(|| self.analysis.model.field_base_at(dot.text_range().start()));
        if let Some(fields) = ty.and_then(struct_fields) {
            push_fields(fields, items);
        }
    }

    /// The variables of the `DECLARE`s before `start`.
    fn variables(&self, root: &SyntaxNode, start: Offset, items: &mut Items) {
        for stmt in statements(root).filter(|stmt| before_statement(&self.text, stmt, start)) {
            let ast::Stmt::DeclareStmt(declare) = stmt else {
                continue;
            };
            for name in declare.names() {
                let Some(text) = name.text() else {
                    continue;
                };
                let ty = (declare.ty().map(|ty| ty.syntax().to_string())).or_else(|| {
                    (self.analysis.model.names.iter()).find_map(|use_| match &use_.definition {
                        Definition::Variable {
                            definition: Some(definition),
                            ty: Some(ty),
                            ..
                        } if *definition == name.syntax().text_range() => Some(googlesql_name(ty)),
                        _ => None,
                    })
                });
                items.push(CompletionItem {
                    sort: sort(rank::VARIABLE, &text),
                    insert: quote(&text),
                    label: text,
                    kind: CompletionKind::Variable,
                    detail: ty,
                    documentation: None,
                });
            }
        }
    }

    /// The tables of the catalog, the pipe `WITH` names and temporary tables of `scope`, and in
    /// the body of a table function, its table parameters.
    fn tables(
        &self,
        root: &SyntaxNode,
        start: Offset,
        scope: Option<&OperatorScope>,
        items: &mut Items,
    ) {
        for table in self.catalog.tables() {
            let label = (table.names.iter())
                .map(|name| quote(name))
                .collect::<Vec<_>>()
                .join(".");
            items.simple(
                label,
                CompletionKind::Table,
                Some(format!("{} columns", table.schema().fields().len())),
                rank::COLUMN,
            );
        }
        // The pipe `WITH` names and the temporary tables in scope, as the analysis knows them.
        for name in scope.iter().flat_map(|scope| &scope.queries) {
            items.simple(quote(name), CompletionKind::Query, None, rank::COLUMN);
        }
        // In the body of a table function, its table parameters.
        let body = (root.token_at_offset(start).left_biased()).and_then(|token| {
            token
                .parent_ancestors()
                .find_map(ast::CreateFunctionStmt::cast)
        });
        for parameter in (body.iter())
            .flat_map(|create| create.param_list())
            .flat_map(|list| list.parameters())
        {
            let table = matches!(
                parameter.param_type(),
                Some(ast::ParamType::AnyTable(_) | ast::ParamType::TableType(_))
            );
            if let (true, Some(name)) = (table, parameter.name().and_then(|name| name.text())) {
                items.simple(name, CompletionKind::Table, None, rank::COLUMN);
            }
        }
    }

    /// The table functions of the library and the temporary table functions before `start`.
    fn table_functions(&self, root: &SyntaxNode, start: Offset, items: &mut Items) {
        for function in (self.library.functions()).filter(|f| f.kind == FunctionKind::Table) {
            items.push(self.function_item(function));
        }
        for create in temporary_functions(&self.text, root, start) {
            if create.function_kind() == ast::FunctionKind::Table {
                items.extend_temporary(&create, FunctionRole::Table);
            }
        }
    }

    /// The functions that `place` takes, by the model's rules, of the library and the temporary
    /// functions before `start`. Without a place, an expression takes scalar functions. In the
    /// arguments of a mark, the constructors come first.
    fn functions(
        &self,
        root: &SyntaxNode,
        start: Offset,
        place: Option<&Place>,
        mark: bool,
        items: &mut Items,
    ) {
        for function in self.library.functions() {
            let takes = match place {
                Some(place) => place.takes(function),
                None => {
                    function.kind == FunctionKind::Scalar
                        && match_recognize_part(&function.name).is_none()
                }
            };
            if takes {
                let mut item = self.function_item(function);
                if mark && function.family == Family::Constructor {
                    item.sort = sort(rank::VARIABLE, &function.name);
                }
                items.push(item);
            }
        }
        let aggregates = place.is_some_and(|place| place.aggregates || place.windows);
        for create in temporary_functions(&self.text, root, start) {
            let (takes, role) = match create.function_kind() {
                ast::FunctionKind::Scalar => (true, FunctionRole::Scalar),
                ast::FunctionKind::Aggregate => (aggregates, FunctionRole::Aggregate),
                ast::FunctionKind::Table => (false, FunctionRole::Table),
            };
            if takes {
                items.extend_temporary(&create, role);
            }
        }
    }

    fn function_item(&self, function: &Function) -> CompletionItem {
        CompletionItem {
            sort: sort(rank::FUNCTION, &function.name),
            insert: function.name.clone(),
            label: function.name.clone(),
            kind: CompletionKind::Function(function.role()),
            detail: Some(function.summary.clone()),
            documentation: Some(entry(function)),
        }
    }

    /// Whether the keyword `kind`, which the grammar takes after `before`, fits the function
    /// that it would apply to: `OVER` after a call of a function that takes one, in an item of
    /// `|> SELECT`, `|> EXTEND`, or `|> SET`, and in a window's specification, `ORDER BY` and a
    /// frame when the window function takes them.
    fn keyword_fits(&self, kind: SyntaxKind, before: &SyntaxToken) -> bool {
        match kind {
            // A `PIVOT` takes the call of an aggregate, which `NOT` doesn't start.
            SyntaxKind::NOT_KW => (self.analysis.model.place_at(before.text_range().end()))
                .is_none_or(|place| place.kind != PlaceKind::Pivot),
            SyntaxKind::OVER_KW => {
                let Some(call) = (before.kind() == SyntaxKind::R_PAREN)
                    .then(|| before.parent()?.parent())
                    .flatten()
                    .filter(|call| call.kind() == SyntaxKind::CALL_EXPR)
                else {
                    return false;
                };
                let windowed = (callee(&call).and_then(|name| self.library.get(&name)))
                    .is_some_and(|function| function.clauses.over != Over::Forbidden);
                let place = self.analysis.model.place_at(call.text_range().start());
                windowed && place.is_some_and(|place| place.windows)
            }
            SyntaxKind::ORDER_KW | SyntaxKind::ROWS_KW | SyntaxKind::RANGE_KW => {
                let function = (before.parent_ancestors())
                    .find(|node| node.kind() == SyntaxKind::WINDOW_SPEC)
                    .and_then(|spec| spec.parent())
                    .filter(|over| over.kind() == SyntaxKind::OVER_CLAUSE)
                    .and_then(|over| over.parent())
                    .and_then(|call| callee(&call))
                    .and_then(|name| self.library.get(&name));
                match function.map(|function| function.clauses.over) {
                    Some(Over::Required { order_by, .. }) if kind == SyntaxKind::ORDER_KW => {
                        order_by != Need::Forbidden
                    }
                    Some(Over::Required { frame, .. }) if kind != SyntaxKind::ORDER_KW => frame,
                    _ => true,
                }
            }
            _ => true,
        }
    }

    /// At the start of an argument of a call of the library, `name => ` for each parameter that
    /// a call can name and that the call has not named.
    fn parameters(&self, before: &SyntaxToken, items: &mut Items) {
        let Some((list, function)) = self.argument_list(before) else {
            return;
        };
        if !matches!(before.kind(), SyntaxKind::L_PAREN | SyntaxKind::COMMA) {
            return;
        }
        // The argument after `before` is the one at the cursor, whose name a completion replaces.
        let end = before.text_range().end();
        let named: HashSet<String> = (list.arguments())
            .filter(|argument| argument.syntax().text_range().start() < end)
            .filter_map(|argument| match argument {
                ast::Argument::NamedArg(named) => named.name_ref()?.text(),
                ast::Argument::NamedTableArg(named) => named.name_ref()?.text(),
                _ => None,
            })
            .map(|name| name.to_ascii_lowercase())
            .collect();
        for param in &function.params {
            let nameable = function.family != Family::GoogleSql
                || (function.forms.iter())
                    .flat_map(|form| &form.signature.args)
                    .any(|arg| arg.named_only && arg.param.eq_ignore_ascii_case(&param.name));
            if !nameable || named.contains(&param.name.to_ascii_lowercase()) {
                continue;
            }
            items.push(CompletionItem {
                sort: sort(rank::ARGUMENT, &param.name),
                insert: format!("{} => ", param.name),
                label: param.name.clone(),
                kind: CompletionKind::Parameter,
                detail: param.doc.lines().next().map(str::to_string),
                documentation: Some(param.doc.clone()),
            });
        }
    }

    /// Whether the argument after `before`, at the start of an argument of a call of the
    /// library, takes only a name: after a named argument, or past the positional arguments of
    /// every form of the function.
    fn only_named(&self, before: &SyntaxToken) -> bool {
        if !matches!(before.kind(), SyntaxKind::COMMA) {
            return false;
        }
        let Some((list, function)) = self.argument_list(before) else {
            return false;
        };
        let end = before.text_range().end();
        let earlier: Vec<ast::Argument> = (list.arguments())
            .filter(|argument| argument.syntax().text_range().end() <= end)
            .collect();
        if (earlier.iter()).any(|argument| matches!(argument, ast::Argument::NamedArg(_))) {
            return true;
        }
        let positional = earlier.len();
        (function.forms.iter()).all(|form| {
            let args: Vec<&FormArg> = (form.signature.args.iter())
                .filter(|arg| !arg.named_only)
                .collect();
            args.len() <= positional && args.last().is_none_or(|arg| arg.arity != Arity::Repeated)
        })
    }

    /// Whether `before` is in the arguments of a mark.
    fn in_mark(&self, before: &SyntaxToken) -> bool {
        self.argument_list(before)
            .is_some_and(|(_, function)| function.family == Family::Mark)
    }

    /// At the start of an argument whose parameter takes a value of a set, its values.
    fn argument_values(&self, before: &SyntaxToken, items: &mut Items) {
        if !matches!(
            before.kind(),
            SyntaxKind::L_PAREN | SyntaxKind::COMMA | SyntaxKind::FAT_ARROW
        ) {
            return;
        }
        let Some((list, function)) = self.argument_list(before) else {
            return;
        };
        let param = match before.kind() {
            SyntaxKind::FAT_ARROW => (before.parent())
                .and_then(ast::NamedArg::cast)
                .and_then(|named| named.name_ref()?.text()),
            _ => {
                let end = before.text_range().end();
                let position = (list.arguments())
                    .filter(|argument| matches!(argument, ast::Argument::Arg(_)))
                    .filter(|argument| argument.syntax().text_range().end() <= end)
                    .count();
                positional_param(function, position)
            }
        };
        if let Some(param) = param {
            push_values(function, &param, items);
        }
    }

    /// In the string literal `string`, an argument of a call of the library, the values of the
    /// set that its parameter takes.
    fn enum_values(&self, string: &SyntaxToken, items: &mut Items) {
        let Some(argument) = string
            .parent_ancestors()
            .find(|node| matches!(node.kind(), SyntaxKind::ARG | SyntaxKind::NAMED_ARG))
        else {
            return;
        };
        let Some(list) = argument.parent().and_then(ast::ArgList::cast) else {
            return;
        };
        let Some(function) = (list.syntax().parent())
            .and_then(|call| callee(&call))
            .and_then(|name| self.library.get(&name))
        else {
            return;
        };
        let param = match ast::NamedArg::cast(argument.clone()) {
            Some(named) => named.name_ref().and_then(|name| name.text()),
            None => {
                let position = (list.arguments())
                    .filter(|argument| matches!(argument, ast::Argument::Arg(_)))
                    .take_while(|other| other.syntax() != &argument)
                    .count();
                positional_param(function, position)
            }
        };
        if let Some(param) = param {
            push_values(function, &param, items);
        }
    }

    /// The argument list that `before` is in, of a call of a function of the library.
    fn argument_list(&self, before: &SyntaxToken) -> Option<(ast::ArgList, &Function)> {
        let list = before.parent_ancestors().find_map(ast::ArgList::cast)?;
        let function = self.library.get(&callee(&list.syntax().parent()?)?)?;
        Some((list, function))
    }
}

impl Items {
    fn extend_temporary(&mut self, create: &ast::CreateFunctionStmt, role: FunctionRole) {
        let Some(name) = create.name_path().map(|path| path.syntax().to_string()) else {
            return;
        };
        self.push(CompletionItem {
            sort: sort(rank::FUNCTION, &name),
            insert: name.clone(),
            label: name,
            kind: CompletionKind::Function(role),
            detail: Some(create_header(create)),
            documentation: None,
        });
    }
}

/// The parameter of the positional argument at `position` in some form of `function` that takes
/// a value of a set there.
fn positional_param(function: &Function, position: usize) -> Option<String> {
    (function.forms.iter()).find_map(|form| {
        let arg = (form.signature.args.iter())
            .filter(|arg| !arg.named_only)
            .nth(position)?;
        matches!(arg.accept, Accept::OneOf(_)).then(|| arg.param.clone())
    })
}

/// The values of the sets that `param` of `function` takes, as string literals.
fn push_values(function: &Function, param: &str, items: &mut Items) {
    let doc = function.param_named(param).map(|param| param.doc.clone());
    for arg in (function.forms.iter()).flat_map(|form| &form.signature.args) {
        let (true, Accept::OneOf(values)) = (arg.param.eq_ignore_ascii_case(param), &arg.accept)
        else {
            continue;
        };
        for value in values {
            let literal = format!("'{value}'");
            items.push(CompletionItem {
                sort: sort(rank::ARGUMENT, value),
                insert: literal.clone(),
                label: literal,
                kind: CompletionKind::EnumValue,
                detail: Some(format!("a value of `{}`", arg.param)),
                documentation: doc.clone(),
            });
        }
    }
}

/// The `CREATE FUNCTION`s of the script before `start`.
fn temporary_functions(
    text: &str,
    root: &SyntaxNode,
    start: Offset,
) -> Vec<ast::CreateFunctionStmt> {
    statements(root)
        .filter(|stmt| before_statement(text, stmt, start))
        .filter_map(|stmt| match stmt {
            ast::Stmt::CreateFunctionStmt(create) => Some(create),
            _ => None,
        })
        .collect()
}

/// Whether `stmt` of `text` is a statement before the one at `start`, which a `;` ends before
/// it, so that its definitions are in scope there. A statement's own name is not in scope in the
/// statement.
fn before_statement(text: &str, stmt: &ast::Stmt, start: Offset) -> bool {
    let end = stmt.syntax().text_range().end();
    end <= start && text[usize::from(end)..usize::from(start)].contains(';')
}

/// The fields of a struct, as items.
fn push_fields(fields: Vec<(String, String)>, items: &mut Items) {
    for (name, ty) in fields {
        items.push(CompletionItem {
            sort: sort(rank::COLUMN, &name),
            insert: quote(&name),
            label: name,
            kind: CompletionKind::Field,
            detail: Some(ty),
            documentation: None,
        });
    }
}

/// The item of `column`, whose rank is `rank`, after the columns of the type `expected`.
fn column_item(column: &Column, rank: u8, expected: Option<&DataType>) -> CompletionItem {
    let unexpected = match (expected, &column.ty) {
        (Some(expected), Some(ty)) => googlesql_name(expected) != googlesql_name(ty),
        _ => false,
    };
    CompletionItem {
        sort: format!(
            "{rank}{}{}",
            u8::from(unexpected),
            column.name.to_ascii_lowercase()
        ),
        insert: quote(&column.name),
        label: column.name.clone(),
        kind: CompletionKind::Column,
        detail: column.ty.as_ref().map(googlesql_name),
        documentation: None,
    }
}

/// The fields of a struct type, with their types as GoogleSQL writes them.
fn struct_fields(ty: &DataType) -> Option<Vec<(String, String)>> {
    match ty {
        DataType::Struct(fields) => Some(
            (fields.iter())
                .map(|field| (field.name().clone(), googlesql_name(field.data_type())))
                .collect(),
        ),
        _ => None,
    }
}

/// The text of the keyword of `kind`, such as `BY` for `BY_KW`, with the `BY` that always follows
/// `GROUP`, `ORDER`, and `PARTITION`.
fn keyword(kind: SyntaxKind) -> String {
    match kind {
        SyntaxKind::GROUP_KW => "GROUP BY".to_string(),
        SyntaxKind::ORDER_KW => "ORDER BY".to_string(),
        SyntaxKind::PARTITION_KW => "PARTITION BY".to_string(),
        _ => {
            let name = format!("{kind:?}");
            name.strip_suffix("_KW").unwrap_or(&name).to_string()
        }
    }
}

/// The named windows of the `WINDOW` clause of the operator that holds `before`.
fn windows(before: &SyntaxToken, items: &mut Items) {
    let Some(clause) = (before.parent_ancestors())
        .find(|node| {
            matches!(
                node.kind(),
                SyntaxKind::PIPE_SELECT | SyntaxKind::PIPE_EXTEND
            )
        })
        .and_then(|operator| operator.children().find_map(ast::WindowClause::cast))
    else {
        return;
    };
    for window in clause.named_windows() {
        if let Some(name) = window.name().and_then(|name| name.text()) {
            let detail = window
                .window_spec()
                .map(|spec| collapse(&spec.syntax().to_string()));
            items.simple(name, CompletionKind::Window, detail, rank::VARIABLE);
        }
    }
}

/// `text` with each run of whitespace as one space.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A name without its backticks.
fn unquote(text: &str) -> String {
    text.strip_prefix('`')
        .and_then(|text| text.strip_suffix('`'))
        .unwrap_or(text)
        .to_string()
}

/// The start of the word of letters, digits, and `_` that ends at `offset`, as
/// [`expected_at`] takes it.
fn word_start(text: &str, offset: Offset) -> Offset {
    let end = usize::from(offset);
    let len = text[..end]
        .bytes()
        .rev()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    Offset::from((end - len) as u32)
}

/// The end of the word of letters, digits, and `_` that starts at `offset`.
fn word_end(text: &str, offset: Offset) -> Offset {
    let start = usize::from(offset);
    let len = text[start..]
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    Offset::from((start + len) as u32)
}

/// The items in the order of their sort keys.
///
/// In a quoted name, `quote` says whether the name has its closing backtick. There the items are
/// names, which insert without their backticks, and with the closing backtick that the name
/// lacks.
fn finish(range: TextRange, replace: TextRange, items: Items, quote: Option<bool>) -> Completions {
    let mut items = items.0;
    if let Some(closed) = quote {
        items.retain(|item| {
            !matches!(
                item.kind,
                CompletionKind::Keyword
                    | CompletionKind::Type
                    | CompletionKind::Parameter
                    | CompletionKind::EnumValue
            )
        });
        for item in &mut items {
            // A table's label quotes the names of its path that need it.
            let name = match item.kind {
                CompletionKind::Table => item.label.replace('`', ""),
                _ => item.label.clone(),
            };
            let escaped = escape(&name);
            item.insert = match closed {
                true => escaped,
                false => format!("{escaped}`"),
            };
        }
    }
    items.sort_by(|a, b| a.sort.cmp(&b.sort));
    Completions {
        range,
        replace,
        items,
    }
}

/// The item of a range variable.
fn range_variable_item(range_variable: &RangeVariable) -> CompletionItem {
    CompletionItem {
        sort: sort(rank::VARIABLE, &range_variable.name),
        insert: quote(&range_variable.name),
        label: range_variable.name.clone(),
        kind: CompletionKind::RangeVariable,
        detail: Some(format!("{} columns", range_variable.columns.len())),
        documentation: None,
    }
}
