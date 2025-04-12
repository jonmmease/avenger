use std::{collections::HashMap, ops::ControlFlow, sync::Arc};

use datafusion::{arrow::array::{record_batch, RecordBatch}, execution::SessionState, functions::core::expr_ext, logical_expr::{planner::{ExprPlanner, TypePlanner}, LogicalPlan, TableSource}, physical_plan::PhysicalExpr, prelude::{DataFrame, Expr, SessionContext}};
use datafusion_common::{DFSchema, ScalarValue};
use sqlparser::ast::{Query as SqlQuery, Expr as SqlExpr};
use datafusion_sql::{planner::{ContextProvider, SqlToRel}, sqlparser::ast::{Ident, VisitorMut}, unparser::{self, expr_to_sql}, ResolvedTableReference, TableReference};

use crate::{context::{mangle_name, EvaluationContext}, error::AvengerLangError, parser::AvengerParser};


pub fn parse_expr(expr_str: &str) -> Result<SqlExpr, AvengerLangError> {
    let parser = AvengerParser::new();
    let tokens = parser.tokenize(expr_str)?;
    let mut parser = parser.with_tokens_with_locations(tokens);
    let expr = parser.parser.parse_expr()?;
    Ok(expr)
}

/// Compile a SQL expression to a logical expression
pub async fn compile_expr(sql_expr: &SqlExpr, ctx: &EvaluationContext) -> Result<Expr, AvengerLangError> {
    
    let expr = ctx.session_ctx().parse_sql_expr(&sql_expr.to_string(), &DFSchema::empty())?;
    Ok(expr)
}

/// Evaluate a logical expression to a ScalarValue
pub async fn evaluate_val_expr(expr: Expr, ctx: &EvaluationContext) -> Result<ScalarValue, AvengerLangError> {
    let physical_expr = ctx.session_ctx().create_physical_expr(expr, &DFSchema::empty())?;
    let col_val = physical_expr.evaluate(
        &record_batch!(
            ("_dummy", Int32, [1])
        ).unwrap()
    )?;
    match col_val {
        datafusion::logical_expr::ColumnarValue::Scalar(scalar_value) => Ok(scalar_value),
        datafusion::logical_expr::ColumnarValue::Array(_) => {
            Err(AvengerLangError::InternalError("Array value not expected".to_string()))
        },
    }
}

/// Compile a SQL query to a logical plan

pub fn parse_query(query_str: &str) -> Result<Box<SqlQuery>, AvengerLangError> {
    let parser = AvengerParser::new();
    let tokens = parser.tokenize(query_str)?;
    let mut parser = parser.with_tokens_with_locations(tokens);
    let query = parser.parser.parse_query()?;
    Ok(query)
}

pub async fn compile_query(query: &SqlQuery, ctx: &EvaluationContext) -> Result<LogicalPlan, AvengerLangError> {
    let plan = ctx.session_ctx().state().create_logical_plan(&query.to_string()).await?;
    Ok(plan)
}

/// Execute a logical plan to a DataFrame
pub async fn evaluate_plan(plan: LogicalPlan, ctx: &EvaluationContext) -> Result<DataFrame, AvengerLangError> {
    let df = ctx.session_ctx().execute_logical_plan(plan).await?;
    Ok(df)
}

/// Execute a SQL query to a DataFrame
pub async fn evaluate_query(query: &SqlQuery, ctx: &EvaluationContext) -> Result<DataFrame, AvengerLangError> {
    let plan = compile_query(&query, ctx).await?;
    let df = evaluate_plan(plan, ctx).await?;
    Ok(df)
}




pub struct CompilationVisitor<'a> {
    ctx: &'a EvaluationContext,
}

impl<'a> CompilationVisitor<'a> {
    pub fn new(ctx: &'a EvaluationContext) -> Self {
        Self { ctx }
    }
}

impl<'a> VisitorMut for CompilationVisitor<'a> {
    type Break = Result<(), AvengerLangError>;

    /// Replace tables of the form @table_name with the true mangled table name
    fn pre_visit_relation(&mut self, relation: &mut datafusion_sql::sqlparser::ast::ObjectName) -> ControlFlow<Self::Break> {
        let table_name = relation.to_string();

        // Validate dataset reference exists. Ignore relations that don't start with @
        if table_name.starts_with("@") && !self.ctx.has_dataset(&table_name) {
            return ControlFlow::Break(Err(AvengerLangError::InternalError(format!("Dataset {} not found", table_name))));
        }

        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &mut SqlExpr) -> ControlFlow<Self::Break> {
        if let SqlExpr::Identifier(ident) = expr.clone() {
            if ident.value.starts_with("@") {
                // Check if this is a reference to an expression
                if let Ok(registered_expr) = self.ctx.get_expr(&ident.value) {
                    println!("Registered expr: {:#?}", registered_expr);
                    match expr_to_sql(&registered_expr) {
                        Ok(sql_expr) => {
                            *expr = sql_expr;
                            return ControlFlow::Continue(());
                        }
                        Err(err) => {
                            return ControlFlow::Break(
                                Err(AvengerLangError::InternalError(format!("Failed to unparse expression {}\n{:?}", ident.value, err)))
                            );
                        }
                    }
                }
                
                // Otherwise it must be a reference to a value
                if !self.ctx.has_val(&ident.value) {
                    return ControlFlow::Break(Err(AvengerLangError::InternalError(format!("Val or Expr {} not found", ident.value))));
                }
            }
        }
        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {
    use sqlparser::ast::VisitMut;

    use crate::parser::AvengerParser;

    use super::*;
    
    #[tokio::test]
    async fn test_compilation_visitor() -> Result<(), AvengerLangError> {
        let ctx = EvaluationContext::new();

        // Register empty table for my_table
        ctx.register_dataset("@my_table", ctx.session_ctx().read_empty()?)?;
        ctx.register_val("@v1", ScalarValue::Int32(Some(2)))?;

        let sql_expr = parse_expr("12 + 83")?;
        let expr = compile_expr(&sql_expr, &ctx).await?;
        ctx.register_expr("@expr_abc".to_string(), expr.clone())?;
        
        let mut query = parse_query("SELECT *, 1 + @v1 as a, @expr_abc as b FROM @my_table")?;

        println!("Pre visitor: {}", query.to_string());

        // Visit the query and validate references
        let mut visitor = CompilationVisitor::new(&ctx);
        if let ControlFlow::Break(err) = query.visit(&mut visitor) {
            return err;
        }

        println!("Post visitor: {}", query.to_string());
        println!("Post visitor: {:#?}", query);

        let plan = compile_query(&query, &ctx).await?;
        println!("Plan: {:#?}", plan);

        Ok(())
    }
}
