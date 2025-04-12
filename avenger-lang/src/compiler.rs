use std::{collections::HashMap, ops::ControlFlow, sync::Arc};

use datafusion::{arrow::array::{record_batch, RecordBatch}, execution::SessionState, functions::core::expr_ext, logical_expr::{planner::{ExprPlanner, TypePlanner}, LogicalPlan, TableSource}, physical_plan::PhysicalExpr, prelude::{DataFrame, Expr, SessionContext}};
use datafusion_common::{DFSchema, ScalarValue};
use sqlparser::ast::{Expr as SqlExpr, Query as SqlQuery, VisitMut};
use datafusion_sql::{planner::{ContextProvider, SqlToRel}, sqlparser::ast::{Ident, VisitorMut}, unparser::{self, expr_to_sql}, ResolvedTableReference, TableReference};

use crate::{context::{mangle_name, EvaluationContext}, error::AvengerLangError, parser::AvengerParser};


pub fn parse_expr(expr_str: &str) -> Result<SqlExpr, AvengerLangError> {
    let parser = AvengerParser::new();
    let tokens = parser.tokenize(expr_str)?;
    let mut parser = parser.with_tokens_with_locations(tokens);
    let expr = parser.parser.parse_expr()?;
    Ok(expr)
}


pub fn parse_query(query_str: &str) -> Result<Box<SqlQuery>, AvengerLangError> {
    let parser = AvengerParser::new();
    let tokens = parser.tokenize(query_str)?;
    let mut parser = parser.with_tokens_with_locations(tokens);
    let query = parser.parser.parse_query()?;
    Ok(query)
}




#[cfg(test)]
mod tests {
    use sqlparser::ast::VisitMut;

    use crate::parser::AvengerParser;

    use super::*;
    
    #[tokio::test]
    async fn test_compilation_visitor() -> Result<(), AvengerLangError> {
        // let ctx = EvaluationContext::new();

        // // Register empty table for my_table
        // ctx.register_dataset("@my_table", ctx.session_ctx().read_empty()?)?;
        // ctx.register_val("@v1", ScalarValue::Int32(Some(2)))?;

        // let sql_expr = parse_expr("12 + 83")?;
        // let expr = compile_expr(&sql_expr, &ctx).await?;
        // ctx.register_expr("@expr_abc".to_string(), expr.clone())?;
        
        // let mut query = parse_query("SELECT *, 1 + @v1 as a, @expr_abc as b FROM @my_table")?;

        // println!("Pre visitor: {}", query.to_string());

        // // Visit the query and validate references
        // let mut visitor = CompilationVisitor::new(&ctx);
        // if let ControlFlow::Break(err) = query.visit(&mut visitor) {
        //     return err;
        // }

        // println!("Post visitor: {}", query.to_string());
        // println!("Post visitor: {:#?}", query);

        // let plan = compile_query(&query, &ctx).await?;
        // println!("Plan: {:#?}", plan);

        Ok(())
    }
}
