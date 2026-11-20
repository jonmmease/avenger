//! The pass over a script's syntax tree, which plans each query as a DataFusion plan and records
//! its semantic model: what each operator reads and writes, and what each name refers to.
//!
//! The pass resolves names by GoogleSQL's rules, and each construct against forms: a call through
//! the function [`Library`], and an operator through the library's operators, so GoogleSQL's
//! coercion, not DataFusion's, decides each type. It tolerates errors: an operator with an error
//! reports a [`Diagnostic`] at its text, and the operators after it see the columns that its
//! syntax gives, without types and without a plan, so the model still holds their names.
//!
//! [`analyze`] runs the pass over the queries of a script, and [`analyze_data`] over the
//! statements of a data script. The tables of a script come from a [`Catalog`], which
//! [`crate::catalog::references`] tells what to load.

mod build;
mod data;
mod expr;
mod highlight;
mod lifting;
mod match_recognize;
mod model;
mod nested;
mod print;
mod query;
mod reshape;
mod rows;
mod script;
mod set;
mod transforms;

use avenger_sql_syntax::{ast, ast::AstNode, SyntaxNode, TextRange};
use datafusion::{
    common::{DFSchema, DataFusionError, ExprSchema},
    logical_expr::{Expr, LogicalPlan},
};

pub use data::analyze_data;
pub(crate) use data::{script_targets, StatementTarget};
pub use model::{
    match_recognize_part, CallForm, Column, ConstructorUse, Definition, EnumValue, FieldBase,
    MatchRecognizePart, Model, NameUse, OperatorScope, Place, PlaceKind, RangeVariable,
};
pub use print::{Lifted, LiftedOperator};

use crate::{catalog::Catalog, library::Library, types::TypeOptions};
use data::DataScript;
use rows::{Col, Qualifiers, RangeVar, Rows};

/// The options of an analysis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// The default time zone, of the functions that take one and of `TIMESTAMP` literals without
    /// one.
    pub time_zone: String,
    /// The default locale of the format and parse functions, the full name of a bundled locale,
    /// such as `fr-FR`. Without one, the functions use GoogleSQL's names.
    pub locale: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            time_zone: crate::types::UTC.to_string(),
            locale: None,
        }
    }
}

/// An error of the analysis of a script, at the range of its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub range: TextRange,
    pub message: String,
    /// The kind of the error, for a host that handles errors of a kind itself, or `None` for an
    /// error of no such kind.
    pub code: Option<DiagnosticCode>,
}

/// A kind of error of an analysis that a host can tell apart from the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticCode {
    /// The files of a statement whose catalog reads no files, so that the files have no schema.
    NoFiles,
}

/// The result of an analysis: the plans of the script's statements, the plan of its last query,
/// its semantic model, and its diagnostics, in the order in which the pass found them. The plan
/// is `None` when the last query has an error or the script has no query.
#[derive(Clone, Debug)]
pub struct Analysis {
    pub statements: Vec<Statement>,
    pub plan: Option<LogicalPlan>,
    pub model: Model,
    /// The reads of tables of the catalog, in the order of the text, whether the catalog holds
    /// them or not: not those of pipe `WITH` names and temporary tables. A read in the body of a
    /// function is at the call that plans it, and for a function of a module, at the import that
    /// checks it too.
    pub reads: Vec<crate::catalog::Reference>,
    pub diagnostics: Vec<Diagnostic>,
    /// The pipe `AGGREGATE`s that hold lifted calls, which [`Analysis::lift`] prints.
    pub lifted: Vec<LiftedOperator>,
    /// The script's own tables of files, in the order of the text: each `CREATE TEMP EXTERNAL
    /// TABLE` and `LOAD DATA ... TEMP TABLE`, analyzed as a data script's statement is. A read of
    /// one plans a scan of the [`ScriptTable`] of its definition, which a loader replaces with
    /// the table of its files before the plans run, as [`Analysis::with_tables`] does.
    ///
    /// [`ScriptTable`]: crate::data::ScriptTable
    pub file_tables: Vec<crate::data::Definition>,
}

/// A statement of a script and its plan.
#[derive(Clone, Debug)]
pub struct Statement {
    pub range: TextRange,
    pub plan: StatementPlan,
}

/// What a statement of a script does when it runs.
#[derive(Clone, Debug)]
pub enum StatementPlan {
    /// The rows of a query.
    Query(LogicalPlan),
    /// The value of a script variable, by a `DECLARE` or a `SET`: a plan of one row and one
    /// column, whose value the plans after it read as the placeholder `id`, as in
    /// `LogicalPlan::with_param_values`. A runner evaluates each in order and binds its value
    /// into the plans after it.
    Variable {
        name: String,
        id: String,
        plan: LogicalPlan,
    },
    /// A statement that defines a name for the statements after it: a temporary table, a
    /// temporary function, or an import, which the pass inlines, or a table of files, which
    /// [`Analysis::file_tables`] holds.
    Definition,
}

impl Analysis {
    /// The rows of the last query of the script, as a DataFrame of `ctx`: each variable before
    /// it evaluates in `ctx` in order and binds its value into the plans after it. The queries
    /// before the last don't run. A script without a query is an error.
    pub async fn dataframe(
        &self,
        ctx: &datafusion::prelude::SessionContext,
    ) -> datafusion::common::Result<datafusion::dataframe::DataFrame> {
        use datafusion::{
            common::{plan_err, ParamValues},
            scalar::ScalarValue,
        };
        let Some(last) = (self.statements.iter())
            .rposition(|statement| matches!(statement.plan, StatementPlan::Query(_)))
        else {
            return plan_err!("The script has no query");
        };
        let mut values: std::collections::HashMap<String, ScalarValue> = Default::default();
        let bound = |plan: &LogicalPlan,
                     values: &std::collections::HashMap<String, ScalarValue>| {
            let map: std::collections::HashMap<String, _> = (values.iter())
                .map(|(id, value)| (id.trim_start_matches('$').to_string(), value.clone().into()))
                .collect();
            plan.clone().with_param_values(ParamValues::Map(map))
        };
        for statement in &self.statements[..last] {
            if let StatementPlan::Variable { id, plan, .. } = &statement.plan {
                let batches = ctx
                    .execute_logical_plan(bound(plan, &values)?)
                    .await?
                    .collect()
                    .await?;
                let value = match batches.iter().find(|batch| batch.num_rows() > 0) {
                    Some(batch) => ScalarValue::try_from_array(batch.column(0), 0)?,
                    None => ScalarValue::try_from(plan.schema().field(0).data_type())?,
                };
                values.insert(id.clone(), value);
            }
        }
        let StatementPlan::Query(plan) = &self.statements[last].plan else {
            unreachable!("the last query");
        };
        ctx.execute_logical_plan(bound(plan, &values)?).await
    }

    /// The rows of the last query of the script, as [`Analysis::dataframe`] plans them, or no
    /// rows for a script without a query.
    pub async fn run(
        &self,
        ctx: &datafusion::prelude::SessionContext,
    ) -> datafusion::common::Result<Vec<datafusion::arrow::array::RecordBatch>> {
        let has_query = (self.statements.iter())
            .any(|statement| matches!(statement.plan, StatementPlan::Query(_)));
        match has_query {
            true => self.dataframe(ctx).await?.collect().await,
            false => Ok(Vec::new()),
        }
    }
}

/// Plans each query of the script `root`, a [`avenger_sql_syntax::SyntaxKind::SCRIPT`] node, and
/// records its model. The tables of the script come from `catalog`, and its functions from
/// `library`.
pub fn analyze(
    root: &SyntaxNode,
    catalog: &dyn Catalog,
    library: &Library,
    options: &Options,
) -> Analysis {
    let mut pass = Pass::new(catalog, library, options);
    let mut plan = None;
    let mut statements = Vec::new();
    let mut data = DataScript::default();
    match ast::Script::cast(root.clone()) {
        Some(script) => {
            let stmts: Vec<ast::Stmt> = script.stmts().collect();
            data = DataScript::new(&stmts);
            for (index, stmt) in stmts.iter().enumerate() {
                let range = stmt.syntax().text_range();
                let errors = pass.diagnostics.len();
                data.later(&mut pass, index);
                let planned = pass.script_statement(stmt, &mut data);
                if pass.diagnostics.len() != errors {
                    if matches!(stmt, ast::Stmt::Query(_)) {
                        plan = None;
                    }
                    continue;
                }
                for planned in planned {
                    if let StatementPlan::Query(query) = &planned {
                        plan = Some(query.clone());
                    }
                    statements.push(Statement {
                        range,
                        plan: planned,
                    });
                }
            }
        }
        None => pass.error(
            root.text_range(),
            "avenger-sql analyzes scripts, not modules: a module is read by an IMPORT MODULE",
        ),
    }
    Analysis {
        statements,
        plan,
        model: pass.model,
        reads: pass.reads,
        diagnostics: pass.diagnostics,
        lifted: pass.lifted,
        file_tables: data.definitions,
    }
}

/// The state of the pass over a script.
pub(crate) struct Pass<'a> {
    catalog: &'a dyn Catalog,
    library: &'a Library,
    options: &'a Options,
    type_options: TypeOptions,
    qualifiers: Qualifiers,
    diagnostics: Vec<Diagnostic>,
    model: Model,
    /// The number of `?` parameters so far, which become `$1`, `$2` and so on.
    positional: usize,
    /// The named windows of the `WINDOW` clause of the operator that the pass plans.
    windows: Vec<(String, expr::WindowSyntax)>,
    /// The scopes of the queries that enclose the query that the pass plans, outermost first.
    outers: Vec<expr::Outer>,
    /// The names of the pipe `WITH`s in scope, innermost last.
    with_names: Vec<WithName>,
    /// The script variables, by their `DECLARE`s.
    variables: Vec<script::Variable>,
    /// The temporary tables, and the table arguments of a table function's body.
    temp_tables: Vec<WithName>,
    /// The temporary functions, and the functions of the imported modules under their paths.
    functions: Vec<script::TempFunction>,
    /// The constants of the imported modules under their paths.
    constants: Vec<script::Constant>,
    /// The arguments of the parameters of the body of a function that the pass inlines.
    params: Vec<(String, datafusion::logical_expr::Expr)>,
    /// The paths of the modules whose bodies the pass plans, innermost last, in which a name of a
    /// function resolves first.
    module_context: Vec<String>,
    /// The depth of the calls of temporary functions that the pass inlines.
    call_depth: usize,
    /// While the pass plans the `keep` of a `lump`, whose windows run over the categories, the
    /// order of its window `ranked`.
    categories: Option<Vec<datafusion::logical_expr::SortExpr>>,
    /// The clauses of the `MATCH_RECOGNIZE`s whose expressions the pass plans, innermost last.
    match_recognize: Vec<match_recognize::Frame>,
    /// The mark calls whose channels compute statistics, in the items of the pipe `AGGREGATE`s
    /// that the pass plans, which each operator takes when it plans its items.
    liftings: Vec<lifting::Lifting>,
    /// The pipe `AGGREGATE`s that hold lifted calls, for the printer of the stages.
    lifted: Vec<print::LiftedOperator>,
    /// The tables that the earlier statements of a data script define, latest last.
    data_tables: Vec<data::DataTable>,
    /// The paths of the tables that the later statements of a data script define, which a
    /// statement can't read yet.
    later_tables: Vec<Vec<String>>,
    /// The reads of tables that are not pipe `WITH` names, temporary tables, or tables of a data
    /// script, in the order of the text: the reads of the catalog.
    reads: Vec<crate::catalog::Reference>,
}

/// A name of a pipe `WITH`, and the rows of its query.
#[derive(Clone, Debug)]
struct WithName {
    name: String,
    definition: TextRange,
    rows: Rows,
}

impl<'a> Pass<'a> {
    fn new(catalog: &'a dyn Catalog, library: &'a Library, options: &'a Options) -> Self {
        Pass {
            catalog,
            library,
            options,
            type_options: catalog.type_options(),
            qualifiers: Qualifiers::default(),
            diagnostics: Vec::new(),
            model: Model::default(),
            positional: 0,
            windows: Vec::new(),
            outers: Vec::new(),
            with_names: Vec::new(),
            variables: Vec::new(),
            temp_tables: Vec::new(),
            functions: Vec::new(),
            constants: Vec::new(),
            params: Vec::new(),
            module_context: Vec::new(),
            call_depth: 0,
            categories: None,
            match_recognize: Vec::new(),
            liftings: Vec::new(),
            lifted: Vec::new(),
            data_tables: Vec::new(),
            later_tables: Vec::new(),
            reads: Vec::new(),
        }
    }
}

impl Pass<'_> {
    /// The plans of the statement `stmt` of a script, whose tables of files `data` analyzes.
    fn script_statement(&mut self, stmt: &ast::Stmt, data: &mut DataScript) -> Vec<StatementPlan> {
        match stmt {
            ast::Stmt::Query(query) => self
                .statement(query)
                .map(StatementPlan::Query)
                .into_iter()
                .collect(),
            ast::Stmt::DeclareStmt(declare) => self.declare(declare),
            ast::Stmt::SetStmt(set) => self.set_variable(set),
            ast::Stmt::CreateTableStmt(create) => {
                self.create_table(create);
                vec![StatementPlan::Definition]
            }
            ast::Stmt::CreateFunctionStmt(create) => {
                self.create_function(create, None);
                vec![StatementPlan::Definition]
            }
            ast::Stmt::ImportStmt(import) => {
                self.import(import, None);
                vec![StatementPlan::Definition]
            }
            ast::Stmt::CreateConstantStmt(constant) => {
                self.error(
                    constant.syntax().text_range(),
                    "A CONSTANT goes in a module, which a script imports",
                );
                Vec::new()
            }
            // A script's own tables of files, which a script's parse reports without `TEMP`.
            ast::Stmt::CreateExternalTableStmt(_) | ast::Stmt::LoadDataStmt(_) => {
                data.statement(self, stmt);
                vec![StatementPlan::Definition]
            }
            // The other statements of a data script, which a script's parse reports already.
            ast::Stmt::CreateViewStmt(_) | ast::Stmt::CreateSchemaStmt(_) => Vec::new(),
        }
    }

    /// The plan of the query statement `query`, sorted in its order, or `None` after an error.
    fn statement(&mut self, query: &ast::Query) -> Option<LogicalPlan> {
        let errors = self.diagnostics.len();
        let rows = self.query(query);
        match rows::finish(&mut self.qualifiers, rows) {
            Ok(plan) if self.diagnostics.len() == errors => plan,
            Ok(_) => None,
            Err(error) => {
                self.datafusion_error(query.syntax().text_range(), error);
                None
            }
        }
    }

    pub(crate) fn error(&mut self, range: TextRange, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            range,
            message: message.into(),
            code: None,
        });
    }

    /// Reports an error of DataFusion at `range`, which it shouldn't give for a query that the pass
    /// accepts, without its backtrace.
    pub(crate) fn datafusion_error(&mut self, range: TextRange, error: DataFusionError) {
        self.error(range, error.strip_backtrace());
    }

    /// The model's record of the column `col`, which the range variables `vars` hold, of the
    /// type that `schema` gives it.
    fn column_info(col: &Col, vars: &[RangeVar], schema: Option<&DFSchema>) -> Column {
        Column {
            name: col.name.clone(),
            qualifiers: (vars.iter())
                .filter(|var| var.columns.iter().any(|c| c.column == col.column))
                .map(|var| var.name.clone())
                .collect(),
            definition: col.definition,
            ty: schema
                .and_then(|schema| schema.field_from_column(&col.column).ok())
                .map(|field| field.data_type().clone()),
        }
    }

    fn columns_info(rows: &Rows) -> Vec<Column> {
        (rows.columns.iter())
            .map(|col| Self::column_info(col, &rows.vars, rows.schema()))
            .collect()
    }

    /// Records the scope of the operator at `range`, from `input` to `output`.
    fn record_operator(&mut self, range: TextRange, input: Option<&Rows>, output: &Rows) {
        let scope_rows = input.unwrap_or(output);
        self.model.operators.push(OperatorScope {
            range,
            input: input.map(Self::columns_info).unwrap_or_default(),
            range_variables: Self::range_variables_info(&scope_rows.vars, scope_rows.schema()),
            output: Self::columns_info(output),
            queries: self.queries_in_scope(),
            outer: self.outer_columns(),
            outer_range_variables: self.outer_range_variables(),
        });
    }

    /// The range variables `vars`, each with its columns, whose types `schema` gives.
    fn range_variables_info(vars: &[RangeVar], schema: Option<&DFSchema>) -> Vec<RangeVariable> {
        (vars.iter())
            .map(|var| RangeVariable {
                name: var.name.clone(),
                definition: var.definition,
                columns: (var.columns.iter())
                    .map(|col| Column {
                        qualifiers: vec![var.name.clone()],
                        ..Self::column_info(col, vars, schema)
                    })
                    .collect(),
            })
            .collect()
    }

    /// The enclosing queries whose columns a subquery reads, innermost first: not those of a
    /// `MATCH_RECOGNIZE` clause, whose columns its subqueries can't read.
    fn readable_outers(&self) -> impl Iterator<Item = &expr::Outer> {
        (self.outers.iter().rev()).filter(|outer| outer.match_recognize.is_none())
    }

    /// The columns of the enclosing queries that a subquery reads, innermost first, and the
    /// parameters of the function whose body the pass plans, which its queries read.
    pub(crate) fn outer_columns(&self) -> Vec<Column> {
        let params = self.params.iter().map(|(name, arg)| Column {
            name: name.clone(),
            qualifiers: Vec::new(),
            definition: None,
            ty: match arg {
                Expr::Placeholder(placeholder) => {
                    (placeholder.field.as_ref()).map(|field| field.data_type().clone())
                }
                Expr::Cast(cast) => Some(cast.field.data_type().clone()),
                _ => None,
            },
        });
        let outer = self.readable_outers().flat_map(|outer| {
            (outer.columns.iter())
                .map(|col| Self::column_info(col, &outer.vars, outer.schema.as_ref()))
        });
        outer.chain(params).collect()
    }

    /// The range variables of the enclosing queries that a subquery reads, innermost first.
    pub(crate) fn outer_range_variables(&self) -> Vec<RangeVariable> {
        self.readable_outers()
            .flat_map(|outer| Self::range_variables_info(&outer.vars, outer.schema.as_ref()))
            .collect()
    }

    /// The names of the pipe `WITH`s and the temporary tables in scope.
    pub(crate) fn queries_in_scope(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for with in self.with_names.iter().chain(&self.temp_tables) {
            if !names
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&with.name))
            {
                names.push(with.name.clone());
            }
        }
        names
    }

    /// Records the place `range` of the kind `kind`, where the pass plans expressions in
    /// `scope`, with the type of its value when the pass knows it.
    pub(crate) fn place(
        &mut self,
        range: TextRange,
        kind: model::PlaceKind,
        scope: &expr::Scope,
        expected: Option<datafusion::arrow::datatypes::DataType>,
    ) {
        use model::PlaceKind;
        let match_recognize = self.mr_frame().and_then(|frame| frame.clause.part());
        let expected = expected.or_else(|| {
            matches!(kind, PlaceKind::Filter | PlaceKind::Define)
                .then_some(datafusion::arrow::datatypes::DataType::Boolean)
        });
        self.model.places.push(model::Place {
            range,
            kind,
            // The body of a function reads its parameters as names, and a subquery the columns of
            // the queries around it.
            columns: !scope.columns.is_empty()
                || !self.params.is_empty()
                || !self.outers.is_empty(),
            aggregates: scope.aggregates,
            windows: scope.windows,
            match_recognize,
            expected,
        });
    }

    /// Records the place `range` that holds names of the input's columns only, as
    /// `CORRESPONDING BY (...)`, or a constant, which reads no columns.
    pub(crate) fn place_without_scope(&mut self, range: TextRange, kind: model::PlaceKind) {
        self.model.places.push(model::Place {
            range,
            kind,
            columns: kind == model::PlaceKind::ColumnList,
            aggregates: false,
            windows: false,
            match_recognize: None,
            expected: None,
        });
    }

    fn use_name(&mut self, range: TextRange, definition: Definition) {
        self.model.names.push(NameUse { range, definition });
    }
}
