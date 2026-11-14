//! The pass over a script's syntax tree, which plans each query as a DataFusion plan and records
//! its semantic model: what each operator reads and writes, and what each name refers to.
//!
//! The pass resolves names by GoogleSQL's rules, and each construct against forms: a call through
//! the function [`Library`], and an operator through the library's operators, so GoogleSQL's
//! coercion, not DataFusion's, decides each type. It tolerates errors: an operator with an error
//! reports a [`Diagnostic`] at its text, and the operators after it see the columns that its
//! syntax gives, without types and without a plan, so the model still holds their names.
//!
//! [`analyze`] runs the pass over the queries of a script. The tables of a script come from a
//! [`Catalog`], which [`crate::catalog::references`] tells what to load.

mod expr;
mod highlight;
mod match_recognize;
mod model;
mod nested;
mod query;
mod reshape;
mod rows;
mod script;
mod set;
mod transforms;

use avenger_sql_syntax::{ast, ast::AstNode, SyntaxNode, TextRange};
use datafusion::{common::DataFusionError, logical_expr::LogicalPlan};

pub use model::{
    CallForm, Column, Definition, EnumValue, Model, NameUse, OperatorScope, RangeVariable,
};

use crate::{catalog::Catalog, library::Library, types::TypeOptions};
use rows::{Col, Qualifiers, Rows};

/// The options of an analysis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// The default time zone, of the functions that take one and of `TIMESTAMP` literals without
    /// one.
    pub time_zone: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            time_zone: crate::types::UTC.to_string(),
        }
    }
}

/// An error of the analysis of a script, at the range of its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub range: TextRange,
    pub message: String,
}

/// The result of an analysis: the plans of the script's statements, the plan of its last query,
/// its semantic model, and its diagnostics, in the order in which the pass found them. The plan
/// is `None` when the last query has an error or the script has no query.
#[derive(Clone, Debug)]
pub struct Analysis {
    pub statements: Vec<Statement>,
    pub plan: Option<LogicalPlan>,
    pub model: Model,
    pub diagnostics: Vec<Diagnostic>,
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
    /// A statement that defines a name for the statements after it, which the pass inlines: a
    /// temporary table, a temporary function, or an import.
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
    let mut pass = Pass {
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
    };
    let mut plan = None;
    let mut statements = Vec::new();
    match ast::Script::cast(root.clone()) {
        Some(script) => {
            for stmt in script.stmts() {
                let range = stmt.syntax().text_range();
                let errors = pass.diagnostics.len();
                let planned = pass.script_statement(&stmt);
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
        diagnostics: pass.diagnostics,
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
}

/// A name of a pipe `WITH`, and the rows of its query.
#[derive(Clone, Debug)]
struct WithName {
    name: String,
    definition: TextRange,
    rows: Rows,
}

impl Pass<'_> {
    /// The plans of the statement `stmt` of a script.
    fn script_statement(&mut self, stmt: &ast::Stmt) -> Vec<StatementPlan> {
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
        });
    }

    /// Reports an error of DataFusion at `range`, which it shouldn't give for a query that the pass
    /// accepts, without its backtrace.
    pub(crate) fn datafusion_error(&mut self, range: TextRange, error: DataFusionError) {
        self.error(range, error.strip_backtrace());
    }

    /// The model's record of the column `col` of `rows`.
    fn column_info(rows: &Rows, col: &Col) -> Column {
        Column {
            name: col.name.clone(),
            qualifiers: (rows.vars.iter())
                .filter(|var| var.columns.iter().any(|c| c.column == col.column))
                .map(|var| var.name.clone())
                .collect(),
            definition: col.definition,
            ty: rows.ty(col),
        }
    }

    fn columns_info(rows: &Rows) -> Vec<Column> {
        (rows.columns.iter())
            .map(|col| Self::column_info(rows, col))
            .collect()
    }

    /// Records the scope of the operator at `range`, from `input` to `output`.
    fn record_operator(&mut self, range: TextRange, input: Option<&Rows>, output: &Rows) {
        let scope_rows = input.unwrap_or(output);
        self.model.operators.push(OperatorScope {
            range,
            input: input.map(Self::columns_info).unwrap_or_default(),
            range_variables: (scope_rows.vars.iter())
                .map(|var| RangeVariable {
                    name: var.name.clone(),
                    definition: var.definition,
                    columns: (var.columns.iter())
                        .map(|col| Column {
                            qualifiers: vec![var.name.clone()],
                            ..Self::column_info(scope_rows, col)
                        })
                        .collect(),
                })
                .collect(),
            output: Self::columns_info(output),
        });
    }

    fn use_name(&mut self, range: TextRange, definition: Definition) {
        self.model.names.push(NameUse { range, definition });
    }
}
