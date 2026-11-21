//! The analysis of a data script: what each statement defines, with its schema, the plans of
//! the queries of its views and tables, and the reads that leave the script.
//!
//! The statements run in the order of the text. A read of a table that an earlier statement
//! defines, `TEMP` or not, plans a scan of a [`ScriptTable`] of its definition, which a loader
//! replaces with the table that it registers. A read of a table that a later statement defines
//! is an error, and a read of any other table goes to the catalog, as in a script. The table of
//! an external table or a `LOAD DATA` comes from [`Catalog::files`].

use std::sync::Arc;

use avenger_sql_syntax::{
    ast::{self, AstNode, CreateStmt, Scope as CreateScope},
    SyntaxKind, SyntaxNode, TextRange,
};
use datafusion::{
    arrow::datatypes::{Field, SchemaRef},
    logical_expr::{LogicalPlan, TableType},
};

use super::{rows, Options, Pass};
use crate::{
    catalog::{same_path, Catalog, Table},
    data::{
        column_list, file_options, table_options, DataAnalysis, Definition, DefinitionKind,
        FileStatement, Files, LoadMode, ScriptTable,
    },
    library::{form::coercion, Library, Ty},
    types,
};

/// A table that an earlier statement of a data script defines: its path, and its table, or
/// `None` when its statement has an error that leaves its schema unknown, with the names of the
/// columns that its syntax gives.
#[derive(Clone, Debug)]
pub(super) struct DataTable {
    pub names: Vec<String>,
    pub table: Option<Table>,
    pub columns: Vec<String>,
}

/// Analyzes the data script `root`, a [`avenger_sql_syntax::SyntaxKind::SCRIPT`] node that
/// [`avenger_sql_syntax::parse_data`] reads: each table, view and schema that it defines, the
/// plans of the queries of its views and tables, and the reads that leave it. The tables outside
/// the script, and the tables of its files, come from `catalog`, and its functions from
/// `library`.
pub fn analyze_data(
    root: &SyntaxNode,
    catalog: &dyn Catalog,
    library: &Library,
    options: &Options,
) -> DataAnalysis {
    let mut pass = Pass::new(catalog, library, options);
    let mut script = DataScript::default();
    match ast::Script::cast(root.clone()) {
        Some(root) => {
            let statements: Vec<ast::Stmt> = root.stmts().collect();
            script = DataScript::new(&statements);
            for (index, statement) in statements.iter().enumerate() {
                script.later(&mut pass, index);
                script.statement(&mut pass, statement);
            }
        }
        None => pass.error(
            root.text_range(),
            "A data script is a script of statements, not a module",
        ),
    }
    DataAnalysis {
        definitions: script.definitions,
        reads: pass.reads,
        model: pass.model,
        diagnostics: pass.diagnostics,
    }
}

/// The path that `statement` defines, and whether it is a schema.
fn defines(statement: &ast::Stmt) -> Option<(Vec<String>, bool)> {
    let (path, schema) = match statement {
        ast::Stmt::CreateExternalTableStmt(create) => (create.path(), false),
        ast::Stmt::LoadDataStmt(load) => (load.path(), false),
        ast::Stmt::CreateViewStmt(create) => (create.path(), false),
        ast::Stmt::CreateTableStmt(create) => (create.path(), false),
        ast::Stmt::CreateSchemaStmt(create) => (create.path(), true),
        _ => return None,
    };
    Some((path?.names()?, schema))
}

/// The state of the analysis of a data script, beside the pass over its queries.
#[derive(Default)]
pub(super) struct DataScript {
    pub definitions: Vec<Definition>,
    /// What each statement of the script defines, as [`defines`] gives it.
    defined: Vec<Option<(Vec<String>, bool)>>,
    /// The schemas that the later statements create.
    later_schemas: Vec<Vec<String>>,
}

/// What a statement defines: its path, the range of the path, and whether it is `TEMP`.
struct Target {
    names: Vec<String>,
    range: TextRange,
    temp: bool,
}

impl DataScript {
    /// The analysis of the statements of the script `statements`, before its first statement.
    pub fn new(statements: &[ast::Stmt]) -> Self {
        DataScript {
            defined: statements.iter().map(defines).collect(),
            ..DataScript::default()
        }
    }

    /// Sets the tables and the schemas that the statements after the statement at `index`
    /// define, which that statement can't read yet.
    pub fn later(&mut self, pass: &mut Pass, index: usize) {
        let later = self.defined[index + 1..].iter().flatten();
        pass.later_tables = (later.clone())
            .filter(|(_, schema)| !schema)
            .map(|(names, _)| names.clone())
            .collect();
        self.later_schemas = later
            .filter(|(_, schema)| *schema)
            .map(|(names, _)| names.clone())
            .collect();
    }

    /// Analyzes `statement`, a statement of a data script.
    pub fn statement(&mut self, pass: &mut Pass, statement: &ast::Stmt) {
        match statement {
            ast::Stmt::CreateExternalTableStmt(create) => self.external_table(pass, create),
            ast::Stmt::LoadDataStmt(load) => self.load_data(pass, load),
            ast::Stmt::CreateViewStmt(create) => {
                let Some(target) = self.create_target(pass, create, false) else {
                    return;
                };
                let description = table_options(create.options(), "a view", &mut pass.diagnostics);
                let (plan, columns) = (create.query())
                    .map(|query| self.query(pass, &query, None, "a view"))
                    .unwrap_or_default();
                let schema = plan.as_ref().map(plan_schema);
                let kind = DefinitionKind::View { plan };
                let definition = target.definition(create, description, schema, kind);
                self.define(pass, definition, columns);
            }
            ast::Stmt::CreateTableStmt(create) => {
                let Some(target) = self.create_target(pass, create, false) else {
                    return;
                };
                let description = table_options(create.options(), "a table", &mut pass.diagnostics);
                let (plan, columns) = (create.query())
                    .map(|query| self.query(pass, &query, create.column_list(), "a table"))
                    .unwrap_or_default();
                let schema = plan.as_ref().map(plan_schema);
                let kind = DefinitionKind::TableAs { plan };
                let definition = target.definition(create, description, schema, kind);
                self.define(pass, definition, columns);
            }
            ast::Stmt::CreateSchemaStmt(create) => {
                let Some(target) = self.create_target(pass, create, true) else {
                    return;
                };
                let description =
                    table_options(create.options(), "a schema", &mut pass.diagnostics);
                let definition =
                    target.definition(create, description, None, DefinitionKind::Schema);
                self.define(pass, definition, Vec::new());
            }
            ast::Stmt::CreateFunctionStmt(create) => pass.create_function(create, None),
            ast::Stmt::ImportStmt(import) => pass.import(import, None),
            // The statements of a script, whose parse as a data script reports them already.
            ast::Stmt::Query(_)
            | ast::Stmt::DeclareStmt(_)
            | ast::Stmt::SetStmt(_)
            | ast::Stmt::CreateConstantStmt(_) => {}
        }
    }

    fn external_table(&mut self, pass: &mut Pass, create: &ast::CreateExternalTableStmt) {
        let Some(target) = self.create_target(pass, create, false) else {
            return;
        };
        // A statement without its options, which its parse reports, defines a table without a
        // schema.
        let statement = FileStatement::external(create);
        let options = (statement.as_ref())
            .map(|statement| file_options(statement, pass.type_options, &mut pass.diagnostics))
            .unwrap_or_default();
        let at = (options.uris.or(statement.map(|statement| statement.at)))
            .unwrap_or_else(|| create.syntax().text_range());
        let schema = (options.files.as_ref()).and_then(|files| files_schema(pass, files, at));
        let kind = DefinitionKind::ExternalTable {
            files: options.files,
        };
        let description = options.description.map(|(text, _)| text);
        let definition = target.definition(create, description, schema, kind);
        self.define(pass, definition, syntax_columns(create.column_list()));
    }

    fn load_data(&mut self, pass: &mut Pass, load: &ast::LoadDataStmt) {
        let Some(path) = load.path() else {
            return;
        };
        let Some(target) = self.target(pass, &path, false, load.scope().is_some()) else {
            return;
        };
        let statement = FileStatement::load(load);
        let mode = match load.mode().map(|mode| mode.kind()) {
            Some(SyntaxKind::INTO_KW) => LoadMode::Into,
            _ => LoadMode::Overwrite,
        };
        let options = (statement.as_ref())
            .map(|statement| file_options(statement, pass.type_options, &mut pass.diagnostics))
            .unwrap_or_default();
        let at = (options.uris.or(statement.map(|statement| statement.at)))
            .unwrap_or_else(|| load.syntax().text_range());
        let name = target.names.join(".");
        let existing = self.existing(&target.names, false);
        let appends = match (mode, existing) {
            (LoadMode::Overwrite, Some(_)) => {
                pass.error(
                    target.range,
                    format!(
                        "LOAD DATA OVERWRITE of {name}, which an earlier statement of this script defines: a project loads its tables from nothing, so the earlier statement would have no effect"
                    ),
                );
                return;
            }
            (LoadMode::Into, Some(root)) => {
                let defined = &self.definitions[root];
                let what = match defined.kind {
                    DefinitionKind::LoadData { .. } | DefinitionKind::TableAs { .. } => None,
                    DefinitionKind::ExternalTable { .. } => Some("an external table"),
                    DefinitionKind::View { .. } => Some("a view"),
                    DefinitionKind::Schema => Some("a schema"),
                };
                if let Some(what) = what {
                    pass.error(
                        target.range,
                        format!(
                            "{name} is {what}, and LOAD DATA INTO appends only to a table that an earlier LOAD DATA or CREATE TABLE ... AS of this script creates"
                        ),
                    );
                    return;
                }
                if defined.temp != target.temp {
                    let message = match defined.temp {
                        true => format!(
                            "{name} is a temporary table, which LOAD DATA INTO TEMP TABLE appends to"
                        ),
                        false => format!(
                            "{name} is not a temporary table, and LOAD DATA INTO {name} appends to it"
                        ),
                    };
                    pass.error(target.range, message);
                    return;
                }
                Some(root)
            }
            // A temporary table hides a table of the catalog, as a temporary view does.
            (LoadMode::Into, None)
                if !target.temp && pass.catalog.table(&target.names).is_some() =>
            {
                pass.error(
                    target.range,
                    format!(
                        "{name} is a table of another data script or of the host, and LOAD DATA INTO appends only to a table that an earlier LOAD DATA or CREATE TABLE ... AS of this script creates"
                    ),
                );
                return;
            }
            _ => None,
        };
        let files_schema = (options.files.as_ref()).and_then(|files| files_schema(pass, files, at));
        let schema = match appends {
            None => files_schema,
            Some(root) => {
                let schema = self.definitions[root].schema.clone();
                if let (Some(table), Some(files)) = (&schema, &files_schema) {
                    same_columns(pass, &name, table, files, at);
                }
                schema
            }
        };
        let (description, columns) = match appends {
            None => (
                options.description.map(|(text, _)| text),
                syntax_columns(load.column_list()),
            ),
            Some(root) => {
                if let Some((_, at)) = options.description {
                    pass.error(
                        at,
                        format!(
                            "LOAD DATA INTO {name} appends to the table of an earlier statement, which gives the table's description"
                        ),
                    );
                }
                let root = &self.definitions[root];
                // The columns of the table, which its earlier statement gives.
                let columns = (pass.data_tables.iter().rev())
                    .find(|table| same_path(&table.names, &root.names))
                    .map_or_else(Vec::new, |table| table.columns.clone());
                (root.description.clone(), columns)
            }
        };
        let kind = DefinitionKind::LoadData {
            files: options.files,
            mode,
            appends,
        };
        let definition = target.definition(load, description, schema, kind);
        self.define(pass, definition, columns);
    }

    /// The plan of the query of a view or a table, `what`, with the names and the types of
    /// `columns`, or `None` after an error, and the names of its columns.
    fn query(
        &mut self,
        pass: &mut Pass,
        query: &ast::Query,
        columns: Option<ast::ColumnList>,
        what: &str,
    ) -> (Option<LogicalPlan>, Vec<String>) {
        let errors = pass.diagnostics.len();
        let range = query.syntax().text_range();
        let mut rows = pass.query(query);
        if let Some(list) = columns {
            let columns: Vec<_> = column_list(&list, pass.type_options, &mut pass.diagnostics)
                .into_iter()
                .map(|column| column.typed())
                .collect();
            rows = pass.typed_columns(rows, &columns, list.syntax().text_range());
        }
        let names = rows
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect();
        if rows.plan.is_none() {
            return (None, names);
        }
        for (index, column) in rows.columns.iter().enumerate() {
            if column.name.is_empty() {
                pass.error(
                    range,
                    format!(
                        "Column {} of the query has no name, which a column of {what} takes, as in `AS name`",
                        index + 1
                    ),
                );
            } else if (rows.columns[..index].iter())
                .any(|other| other.name.eq_ignore_ascii_case(&column.name))
            {
                pass.error(
                    range,
                    format!(
                        "Column {} is named twice in the output of the query, whose names are the columns of {what}",
                        column.name
                    ),
                );
            }
        }
        if pass.diagnostics.len() != errors {
            return (None, names);
        }
        match rows::finish(&mut pass.qualifiers, rows) {
            Ok(plan) => (plan, names),
            Err(error) => {
                pass.datafusion_error(range, error);
                (None, names)
            }
        }
    }

    /// What a `CREATE` statement defines, or `None` when it defines nothing: after an error, or
    /// with `IF NOT EXISTS` of a name that an earlier statement defines.
    fn create_target(
        &mut self,
        pass: &mut Pass,
        create: &impl CreateStmt,
        schema: bool,
    ) -> Option<Target> {
        let path = create.name_path()?;
        let temp = create.scope_kind() == Some(CreateScope::Temp);
        let target = self.target(pass, &path, schema, temp)?;
        if self.existing(&target.names, schema).is_none() || create.or_replace() {
            return Some(target);
        }
        if !create.if_not_exists() {
            let what = match schema {
                true => "Schema",
                false => "Table",
            };
            pass.error(
                target.range,
                format!(
                    "{what} {} is already defined by an earlier statement of this script, which CREATE OR REPLACE replaces",
                    target.names.join(".")
                ),
            );
        }
        None
    }

    /// The target of the path `path`, which names a schema when `schema` holds: a table's path is
    /// a name, or a schema and a name, whose schema an earlier statement or the catalog holds.
    fn target(
        &mut self,
        pass: &mut Pass,
        path: &ast::Path,
        schema: bool,
        temp: bool,
    ) -> Option<Target> {
        let names = path.names()?;
        let range = path.syntax().text_range();
        match (schema, names.len()) {
            (true, 1) | (false, 1) => {}
            (true, _) => {
                pass.error(range, "A schema has one name");
                return None;
            }
            (false, 2) => {
                let at = path
                    .name_refs()
                    .next()
                    .map_or(range, |name| name.syntax().text_range());
                self.read_schema(pass, &names[..1], at);
            }
            (false, _) => {
                pass.error(
                    range,
                    "A data script names a table by its name, or by its schema and its name",
                );
                return None;
            }
        }
        Some(Target { names, range, temp })
    }

    /// Checks that the schema `names`, at `range`, exists: an earlier statement creates it, or
    /// the catalog holds it.
    fn read_schema(&self, pass: &mut Pass, names: &[String], range: TextRange) {
        if self.existing(names, true).is_some() {
            return;
        }
        let name = names.join(".");
        if (self.later_schemas.iter()).any(|later| same_path(later, names)) {
            pass.error(
                range,
                format!(
                    "Schema {name} is created by a later statement of this script, and a statement reads only the schemas of the statements before it"
                ),
            );
            return;
        }
        if !pass.catalog.has_schema(names) {
            pass.error(
                range,
                format!(
                    "Schema {name} not found: a CREATE SCHEMA creates a schema, which BigQuery calls a dataset, before the tables in it"
                ),
            );
        }
    }

    /// The index of the definition of the script that creates the table, or the schema when
    /// `schema` holds, at the path `names`.
    fn existing(&self, names: &[String], schema: bool) -> Option<usize> {
        let index = (self.definitions.iter()).rposition(|definition| {
            matches!(definition.kind, DefinitionKind::Schema) == schema
                && same_path(&definition.names, names)
        })?;
        Some(self.definitions[index].appends().unwrap_or(index))
    }

    /// Records `definition`, and its table for the statements after it, whose columns are
    /// `columns` when its schema is unknown.
    fn define(&mut self, pass: &mut Pass, definition: Definition, columns: Vec<String>) {
        let index = self.definitions.len();
        if !matches!(definition.kind, DefinitionKind::Schema) {
            let table_type = match definition.kind {
                DefinitionKind::View { .. } => TableType::View,
                _ => TableType::Base,
            };
            let table = definition.schema.clone().map(|schema| Table {
                names: definition.names.clone(),
                provider: Arc::new(ScriptTable::new(
                    definition.names.clone(),
                    index,
                    schema,
                    table_type,
                )),
            });
            pass.data_tables.push(DataTable {
                names: definition.names.clone(),
                table,
                columns,
            });
        }
        self.definitions.push(definition);
    }
}

impl Target {
    /// The definition of the target by `statement`.
    fn definition(
        self,
        statement: &impl AstNode,
        description: Option<String>,
        schema: Option<SchemaRef>,
        kind: DefinitionKind,
    ) -> Definition {
        Definition {
            names: self.names,
            kind,
            temp: self.temp,
            name_range: self.range,
            range: statement.syntax().text_range(),
            description,
            schema,
        }
    }
}

/// The schema of the rows of `plan`, a view's or a table's.
fn plan_schema(plan: &LogicalPlan) -> SchemaRef {
    Arc::new(plan.schema().as_arrow().clone())
}

/// The names of the columns of a column list.
fn syntax_columns(list: Option<ast::ColumnList>) -> Vec<String> {
    (list.iter())
        .flat_map(|list| list.column_defs())
        .filter_map(|column| column.name()?.text())
        .collect()
}

/// The schema of the table of `files`, which the catalog gives, or `None` with an error at `at`.
fn files_schema(pass: &mut Pass, files: &Files, at: TextRange) -> Option<SchemaRef> {
    match pass.catalog.files(files) {
        Some(Ok(provider)) => Some(provider.schema()),
        Some(Err(error)) => {
            pass.error(at, error);
            None
        }
        None => {
            pass.error(
                at,
                "The catalog of this analysis reads no files, so the files have no schema",
            );
            None
        }
    }
}

/// Checks that the files of an append to the table `name` have the table's columns: by name,
/// without regard to case, and of types whose values coerce to the types of the table's, as the
/// append casts them.
fn same_columns(pass: &mut Pass, name: &str, table: &SchemaRef, files: &SchemaRef, at: TextRange) {
    // Whether the column `file` of the files fills the column `column` of the table.
    let fills = |file: &Field, column: &Field| {
        let (from, to) = (file.data_type(), column.data_type());
        file.name().eq_ignore_ascii_case(column.name())
            && match (Ty::of_arrow(from), Ty::of_arrow(to)) {
                (Some(from), Some(to)) => coercion(&from, false, &to).is_some(),
                _ => types::googlesql_name(from) == types::googlesql_name(to),
            }
    };
    // The columns of `schema` that no column of `other` matches by `matches`, each as its name
    // and its type.
    let unmatched =
        |schema: &SchemaRef, other: &SchemaRef, matches: &dyn Fn(&Field, &Field) -> bool| {
            (schema.fields().iter())
                .filter(|field| !(other.fields().iter()).any(|other| matches(field, other)))
                .map(|field| {
                    format!(
                        "{} {}",
                        field.name(),
                        types::googlesql_name(field.data_type())
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
    let in_files = unmatched(files, table, &|file, column| fills(file, column));
    let in_table = unmatched(table, files, &|column, file| fills(file, column));
    let difference = match (in_files.is_empty(), in_table.is_empty()) {
        (true, true) => return,
        (false, true) => format!("The files have the columns {in_files}, which {name} lacks"),
        (true, false) => format!("The files lack the columns {in_table} of {name}"),
        (false, false) => format!(
            "The files have the columns {in_files}, where {name} has the columns {in_table}"
        ),
    };
    pass.error(
        at,
        format!(
            "{difference}: LOAD DATA INTO appends files whose columns match the table's by name, with values that coerce to the types of the table's"
        ),
    );
}
