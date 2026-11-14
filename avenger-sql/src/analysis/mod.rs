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
mod model;
mod query;
mod reshape;
mod rows;
mod set;

use avenger_sql_syntax::{ast, ast::AstNode, SyntaxNode, TextRange};
use datafusion::{common::DataFusionError, logical_expr::LogicalPlan};

pub use model::{CallForm, Column, Definition, Model, NameUse, OperatorScope, RangeVariable};

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

/// The result of an analysis: the plan of the script's last query, its semantic model, and its
/// diagnostics, in the order in which the pass found them. The plan is `None` when the last
/// query has an error or the script has no query.
#[derive(Clone, Debug)]
pub struct Analysis {
    pub plan: Option<LogicalPlan>,
    pub model: Model,
    pub diagnostics: Vec<Diagnostic>,
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
    };
    let mut plan = None;
    match ast::Script::cast(root.clone()) {
        Some(script) => {
            for stmt in script.stmts() {
                match stmt {
                    ast::Stmt::Query(query) => plan = pass.statement(&query),
                    other => pass.error(
                        other.syntax().text_range(),
                        "avenger-sql doesn't run this statement yet",
                    ),
                }
            }
        }
        None => pass.error(
            root.text_range(),
            "avenger-sql analyzes scripts, not modules, yet",
        ),
    }
    Analysis {
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
}

impl Pass<'_> {
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
