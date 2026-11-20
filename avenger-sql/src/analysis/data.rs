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

/// The name that a statement of a data script defines, as its syntax writes it, which the
/// dependency graph of a project's data scripts reads without analyzing them.
#[derive(Clone, Debug)]
pub(crate) struct StatementTarget {
    pub names: Vec<String>,
    /// The path of the name.
    pub path: ast::Path,
    /// Whether it names a schema, not a table or a view.
    pub schema: bool,
    pub temp: bool,
    /// Whether the statement replaces an earlier definition of its name, as `CREATE OR REPLACE`
    /// does. A `LOAD DATA` replaces none: `INTO` appends to it, and `OVERWRITE` is an error.
    pub replaces: bool,
}

impl StatementTarget {
    /// What `statement` defines, or `None` for a statement that defines no table, view, or
    /// schema, or whose path the tree lacks.
    pub fn of(statement: &ast::Stmt) -> Option<StatementTarget> {
        match statement {
            ast::Stmt::CreateExternalTableStmt(statement) => Self::create(statement, false),
            ast::Stmt::CreateViewStmt(statement) => Self::create(statement, false),
            ast::Stmt::CreateTableStmt(statement) => Self::create(statement, false),
            ast::Stmt::CreateSchemaStmt(statement) => Self::create(statement, true),
            ast::Stmt::LoadDataStmt(load) => Self::load(load),
            _ => None,
        }
    }

    /// What the `CREATE` statement `create` defines, a schema when `schema` holds.
    fn create(create: &impl CreateStmt, schema: bool) -> Option<StatementTarget> {
        let path = create.name_path()?;
        Some(StatementTarget {
            names: path.names()?,
            path,
            schema,
            temp: create.scope_kind() == Some(CreateScope::Temp),
            replaces: create.or_replace(),
        })
    }

    /// What the `LOAD DATA` statement `load` defines.
    fn load(load: &ast::LoadDataStmt) -> Option<StatementTarget> {
        let path = load.path()?;
        Some(StatementTarget {
            names: path.names()?,
            path,
            schema: false,
            temp: load.scope().is_some(),
            replaces: false,
        })
    }

    /// The error of a path whose length the target does not take, or `None`: a schema has one
    /// name, and a table has one name, or a schema and a name.
    fn path_error(&self) -> Option<&'static str> {
        match (self.schema, self.names.len()) {
            (true, 1) | (false, 1) | (false, 2) => None,
            (true, _) => Some("A schema has one name"),
            (false, _) => {
                Some("A data script names a table by its name, or by its schema and its name")
            }
        }
    }

    /// The schema of a table in a schema, which the statement reads: its path, and the range of
    /// its name.
    pub fn read_schema(&self) -> Option<(Vec<String>, TextRange)> {
        let [schema, _] = self.names.as_slice() else {
            return None;
        };
        if self.schema {
            return None;
        }
        let range = (self.path.name_refs().next())
            .map_or(self.path.syntax().text_range(), |name| {
                name.syntax().text_range()
            });
        Some((vec![schema.clone()], range))
    }

    /// Whether `other` names what the target names: a schema or a table at the same path.
    fn same_name(&self, other: &StatementTarget) -> bool {
        self.schema == other.schema && same_path(&self.names, &other.names)
    }
}

/// What a statement of a data script defines among the statements of its script, by their
/// syntax alone, as the analysis of the script decides it when the statements have no errors.
#[derive(Clone, Debug)]
pub(crate) struct ScriptTarget {
    pub target: StatementTarget,
    /// Whether the statement defines its name: its path has a length that the target takes, and
    /// no earlier statement of the script defines the name, or the statement replaces it. An
    /// append of `LOAD DATA INTO`, and a repeat without `OR REPLACE`, define nothing.
    pub defines: bool,
    /// Whether a later statement of the script defines the name again, which replaces the
    /// statement's definition.
    pub replaced: bool,
}

impl ScriptTarget {
    /// Whether the statement defines its name for the other scripts of a project: it defines
    /// the name without `TEMP`, and no later statement replaces it, as
    /// [`DataAnalysis::exported`] gives the definitions of an analysis.
    pub fn exported(&self) -> bool {
        self.defines && !self.target.temp && !self.replaced
    }
}

/// What each of the statements `statements` of a data script defines, in the order of the text,
/// or `None` for a statement that defines no table, view, or schema.
pub(crate) fn script_targets(statements: &[ast::Stmt]) -> Vec<Option<ScriptTarget>> {
    let mut targets: Vec<Option<ScriptTarget>> = Vec::with_capacity(statements.len());
    for statement in statements {
        let Some(target) = StatementTarget::of(statement) else {
            targets.push(None);
            continue;
        };
        let earlier = (targets.iter().flatten())
            .any(|earlier| earlier.defines && earlier.target.same_name(&target));
        let defines = target.path_error().is_none() && (!earlier || target.replaces);
        targets.push(Some(ScriptTarget {
            target,
            defines,
            replaced: false,
        }));
    }
    for index in 0..targets.len() {
        let replaced = targets[index].as_ref().is_some_and(|defined| {
            defined.defines
                && (targets[index + 1..].iter().flatten())
                    .any(|later| later.defines && later.target.same_name(&defined.target))
        });
        if let Some(defined) = &mut targets[index] {
            defined.replaced = replaced;
        }
    }
    targets
}

/// The state of the analysis of a data script, or of the tables of files of a chart script,
/// beside the pass over its queries.
#[derive(Default)]
pub(super) struct DataScript {
    pub definitions: Vec<Definition>,
    /// What each statement of the script defines, as [`script_targets`] gives it.
    defined: Vec<Option<ScriptTarget>>,
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
            defined: script_targets(statements),
            ..DataScript::default()
        }
    }

    /// Sets the tables and the schemas that the statements after the statement at `index`
    /// define, which that statement can't read yet.
    pub fn later(&mut self, pass: &mut Pass, index: usize) {
        let later = (self.defined[index + 1..].iter().flatten()).map(|later| &later.target);
        pass.later_tables = (later.clone())
            .filter(|target| !target.schema)
            .map(|target| target.names.clone())
            .collect();
        self.later_schemas = later
            .filter(|target| target.schema)
            .map(|target| target.names.clone())
            .collect();
    }

    /// Analyzes `statement`, a statement of a data script, or a statement of files of a chart
    /// script: `CREATE EXTERNAL TABLE` or `LOAD DATA`.
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
        let Some(target) =
            (StatementTarget::load(load)).and_then(|target| self.target(pass, target))
        else {
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
        let statement = StatementTarget::create(create, schema)?;
        let replaces = statement.replaces;
        let target = self.target(pass, statement)?;
        if self.existing(&target.names, schema).is_none() || replaces {
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

    /// The target of `statement`: a table's path is a name, or a schema and a name, whose schema
    /// an earlier statement or the catalog holds.
    fn target(&mut self, pass: &mut Pass, statement: StatementTarget) -> Option<Target> {
        let range = statement.path.syntax().text_range();
        // A chart script's temporary table of a query, which the pass inlines, has its name.
        let name = statement.names.join(".");
        if (pass.temp_tables.iter()).any(|table| table.name.eq_ignore_ascii_case(&name)) {
            pass.error(range, format!("Table {name} already exists"));
            return None;
        }
        if let Some(error) = statement.path_error() {
            pass.error(range, error);
            return None;
        }
        if let Some((schema, at)) = statement.read_schema() {
            self.read_schema(pass, &schema, at);
        }
        Some(Target {
            names: statement.names,
            range,
            temp: statement.temp,
        })
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
