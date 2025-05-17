use crate::visitor::AvengerVisit;
use std::ops::ControlFlow;
use std::sync::Arc;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SessionContext;
use datafusion::variable::VarType;
use datafusion_common::ScalarValue;
use datafusion_common::tree_node::TreeNode;
use sqlparser::ast::{Expr, Ident, ObjectName, VisitMut, Visitor, VisitorMut};
use sqlparser::ast::{Expr as SqlExpr, Query as SqlQuery};
use crate::ast::{AvengerScript, Block, ExprDecl, IfStmt, ScriptStatement, TableDecl, ValDecl, VarAssignment, WhileStmt};
use crate::environment::Environment;
use crate::error::AvengerLangError;
use crate::table::ArrowTable;
use crate::udtf::read_csv::LocalCsvTableFunc;
use crate::visitor::AvengerVisitor;


pub struct AvengerInterpreter {
    environment: Arc<Environment>,
    tokio_runtime: tokio::runtime::Runtime,
}

impl AvengerInterpreter {
    pub fn new() -> Self {
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        Self {
            environment: Arc::new(Environment::new()),
            tokio_runtime,
        }
    }

    fn make_context(&self) -> SessionContext {
        let ctx = SessionContext::new();

        // register custom ud(.?)fs
        ctx.register_udtf("read_csv", Arc::new(LocalCsvTableFunc {}));

        // Register environment as val provider
        ctx.register_variable(VarType::UserDefined, self.environment.clone());

        // TODO: register tables from environment

        ctx
    }

    pub async fn evaluate_val(&self, expr: &SqlExpr) -> Result<ScalarValue, AvengerLangError> {
        let sql_expr = self.pre_eval_expr(expr)?;
        let ctx = self.make_context();

        let plan = ctx
            .state()
            .create_logical_plan(&format!("SELECT {} as val", sql_expr.to_string()))
            .await?;

        let df = ctx.execute_logical_plan(plan).await?;
        let schema = df.schema().inner().clone();
        let partitions = df.collect().await?;
        let table = ArrowTable::try_new(schema, partitions)?;

        let col = table.column("val")?;
        let v = ScalarValue::try_from_array(&col, 0)?;
        Ok(v)
    }

    pub fn evaluate_expr(&self, expr: &SqlExpr) -> Result<SqlExpr, AvengerLangError> {
        self.pre_eval_expr(expr)
    }

    pub async fn evaluate_table(&self, query: &SqlQuery) -> Result<ArrowTable, AvengerLangError> {
        let ctx = self.make_context();
        let plan = self.compile_query(query, &ctx).await?;
        let df = ctx.execute_logical_plan(plan).await?;
        let schema = df.schema().clone();
        let table = df.collect().await?;
        Ok(ArrowTable::try_new(schema.inner().clone(), table)?)
    }

    /// Compile a SQL query to a logical plan, expanding sql with referenced expressions
    pub async fn compile_query(
        &self,
        query: &SqlQuery,
        ctx: &SessionContext,
    ) -> Result<LogicalPlan, AvengerLangError> {
        // Visit the query and validate references
        let mut expanded_query = self.pre_eval_query(&query)?;
        let plan = ctx
            .state()
            .create_logical_plan(&expanded_query.to_string())
            .await?;
        Ok(plan)
    }

    fn pre_eval_expr(&self, expr: &SqlExpr) -> Result<SqlExpr, AvengerLangError> {
        // Visit the query and validate references
        let mut expr = expr.clone();
        let mut visitor = PreEvalVisitor::new(&self.environment);
        if let ControlFlow::Break(Err(err)) = expr.visit(&mut visitor) {
            return Err(err);
        }
        Ok(expr)
    }

    fn pre_eval_query(&self, query: &SqlQuery) -> Result<SqlQuery, AvengerLangError> {
        // Visit the query and validate references
        let mut query = query.clone();
        let mut visitor = PreEvalVisitor::new(&self.environment);
        if let ControlFlow::Break(Err(err)) = query.visit(&mut visitor) {
            return Err(err);
        }
        Ok(query)
    }
}

/// Interpreter traversal
impl AvengerInterpreter {
    fn interpret_val_decl(&mut self, statement: &ValDecl) -> Result<(), AvengerLangError> {
        // Evaluate value expression
        let val = self.tokio_runtime.block_on(self.evaluate_val(&statement.expr))?;

        // Store value in environment
        self.environment.insert_val(vec![statement.name.value.clone()], val);
        Ok(())
    }

    fn interpret_expr_decl(&mut self, statement: &ExprDecl) -> Result<(), AvengerLangError> {
        let expr = self.evaluate_expr(&statement.expr)?;

        // Store expression in environment
        self.environment.insert_expr(vec![statement.name.value.clone()], expr);
        Ok(())
    }

    fn interpret_table_decl(&mut self, statement: &TableDecl) -> Result<(), AvengerLangError> {
        let table = self.tokio_runtime.block_on(self.evaluate_table(&statement.query))?;

        // Store table in environment
        self.environment.insert_table(vec![statement.name.value.clone()], table);
        Ok(())
    }

    fn interpret_var_assignment(&mut self, statement: &VarAssignment) -> Result<(), AvengerLangError> {
        let name = vec![statement.name.value.clone()];
        if self.environment.has_val(&name) {
            let expr = statement.expr.clone().into_expr()?;
            let val = self.tokio_runtime.block_on(self.evaluate_val(&expr))?;

            if let Err(err) = self.environment.assign_val(name, val) {
                return Err(err)
            }
        } else if self.environment.has_expr(&name) {
            let expr = statement.expr.clone().into_expr()?;
            let expr = self.evaluate_expr(&expr)?;

            if let Err(err) = self.environment.assign_expr(name, expr) {
                return Err(err)
            }
        } else if self.environment.has_table(&name) {
            let query = statement.expr.clone().into_query()?;
            let table = self.tokio_runtime.block_on(self.evaluate_table(&query))?;

            if let Err(err) = self.environment.assign_table(name, table) {
                return Err(err)
            }
        } else {
            return Err(AvengerLangError::InternalError(
                format!("Variable {} not found", name.join(".")),
            ));
        }
        Ok(())
    }

    fn interpret_if_stmt(&mut self, if_stmt: &IfStmt) -> Result<(), AvengerLangError> {
        for branch in &if_stmt.branches {
            let val = self.tokio_runtime.block_on(self.evaluate_val(&branch.condition))?;
            if !(val.is_null() || val == ScalarValue::Boolean(Some(false))) {
                // Execute block and return
                self.interpret_statement(&branch.body)?;
                return Ok(());
            }
        }
        // Check for else branch
        if let Some(else_branch) = &if_stmt.else_branch {
            // Execute else block
            self.interpret_statement(&else_branch.body)?;
            return Ok(());
        }
        Ok(())
    }

    fn interpret_while_stmt(&mut self, while_stmt: &WhileStmt) -> Result<(), AvengerLangError> {
        loop {
            let val = self.tokio_runtime.block_on(self.evaluate_val(&while_stmt.condition))?;
            if val.is_null() || val == ScalarValue::Boolean(Some(false)) {
                break;
            }
            // Execute block
            self.interpret_statement(&while_stmt.body)?;
        }
        Ok(())
    }

    fn interpret_block(&mut self, block: &Block) -> Result<(), AvengerLangError> {
        // Push new scope
        self.environment = Arc::new(self.environment.push());

        for statement in &block.statements {
            if let Err(err) = self.interpret_statement(statement) {
                return Err(err);
            }
        }

        // Pop scope
        self.environment = self.environment.pop().unwrap();

        Ok(())
    }

    fn interpret_statement(&mut self, statement: &ScriptStatement) -> Result<(), AvengerLangError> {
        match statement {
            ScriptStatement::ValDecl(val_decl) => self.interpret_val_decl(val_decl),
            ScriptStatement::ExprDecl(expr_decl) => self.interpret_expr_decl(expr_decl),
            ScriptStatement::TableDecl(table_decl) => self.interpret_table_decl(table_decl),
            ScriptStatement::VarAssignment(var_assignment) => self.interpret_var_assignment(var_assignment),
            ScriptStatement::IfStmt(if_stmt) => self.interpret_if_stmt(if_stmt),
            ScriptStatement::Block(block) => self.interpret_block(block),
            ScriptStatement::WhileStmt(while_stmt) => self.interpret_while_stmt(while_stmt),
        }
    }

    pub fn interpret_script(&mut self, script: &AvengerScript) -> Result<(), AvengerLangError> {
        for statement in &script.statements {
            if let Err(err) = self.interpret_statement(statement) {
                return Err(err);
            }
        }
        Ok(())
    }
}

/// Visitor that runs prior to expression/query evaluation with DataFusion
pub struct PreEvalVisitor<'a> {
    env: &'a Environment,
}

impl<'a> PreEvalVisitor<'a> {
    pub fn new(env: &'a Environment) -> Self {
        Self { env }
    }
}

impl<'a> VisitorMut for PreEvalVisitor<'a> {
    type Break = Result<(), AvengerLangError>;

    fn pre_visit_relation(
        &mut self,
        relation: &mut ObjectName,
    ) -> ControlFlow<Self::Break> {
        let table_name = relation.to_string();

        if table_name.starts_with("@") {
            let parts = relation
                .0
                .iter()
                .map(|ident| ident.value.clone())
                .collect::<Vec<_>>();

            // Join on __ into a single string
            let idents = vec![Ident::new(parts.join("__"))];
            *relation = ObjectName(idents);
        }

        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &mut SqlExpr) -> ControlFlow<Self::Break> {
        match expr.clone() {
            SqlExpr::Function(mut func) => {
                if func.name.0[0].value.starts_with("@") {
                    // Update function with mangled name
                    let parts = func.name
                        .0
                        .iter()
                        .map(|ident| ident.value.clone())
                        .collect::<Vec<_>>();

                    let idents = vec![Ident::new(parts.join("__"))];
                    func.name = ObjectName(idents);
                    *expr = SqlExpr::Function(func);
                    return ControlFlow::Continue(());
                }
            }
            SqlExpr::Identifier(ident) => {
                if ident.value.starts_with("@") {
                    let variable = vec![ident.value[1..].to_string()];

                    // Check if this is a reference to an expression
                    if let Ok(registered_expr) = self.env.get_expr(&variable) {
                        *expr = SqlExpr::Nested(Box::new(registered_expr.clone()));
                        return ControlFlow::Continue(());
                    }

                    // Otherwise it must be a reference to a value variable
                    if !self.env.has_val(&variable) {
                        return ControlFlow::Break(Err(AvengerLangError::InternalError(
                            format!("Val or Expr {} not found", variable.join(".")),
                        )));
                    }
                }
            }
            SqlExpr::CompoundIdentifier(idents) => {
                if !idents.is_empty() && idents[0].value.starts_with("@") {
                    let mut variable = idents.iter().map(|s| s.value.clone()).collect::<Vec<_>>();
                    variable[0] = variable[0][1..].to_string();

                    // Check if this is a reference to an expression
                    if let Ok(registered_expr) = self.env.get_expr(&variable) {
                        *expr = SqlExpr::Nested(Box::new(registered_expr.clone()));
                        return ControlFlow::Continue(());
                    }

                    // Otherwise it must be a reference to a value
                    if !self.env.has_val(&variable) {
                        return ControlFlow::Break(Err(AvengerLangError::InternalError(
                            format!("Val or Expr {} not found", variable.join(".")),
                        )));
                    }

                    // Update with mangled name, joining on __ into a single string
                    *expr = SqlExpr::Identifier(Ident::new(variable.join("__")));
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}


#[cfg(test)]
mod tests {
    use crate::parser::AvengerParser;
    use super::*;

    #[test]
    fn test_interpret_script1() {
        let src = r#"
val foo: 23;
val bar: @foo + 100;
expr abc: @bar + "colA";
if (2 > 23) {
    table my_table: SELECT @foo as c1;
    foo := @bar + 1;
} else {
    foo := 0;
}
"#;
        let mut parser = AvengerParser::new(src).unwrap();
        let script = parser.parse_script().unwrap();

        let mut interpreter = AvengerInterpreter::new();
        interpreter.interpret_script(&script).expect("Failed to interpret script");

        println!("{:#?}", interpreter.environment);
    }

    #[test]
    fn test_interpret_script2() {
        let src = r#"
val i: 0;
val total: 0;
while (@i < 10) {
    if (@i % 2 == 0) {
        total := @total + @i;
    }
    i := @i + 1;
}
"#;
        let mut parser = AvengerParser::new(src).unwrap();
        let script = parser.parse_script().unwrap();

        let mut interpreter = AvengerInterpreter::new();
        interpreter.interpret_script(&script).expect("Failed to interpret script");

        println!("{:#?}", interpreter.environment);
    }
}


