//! The flow of a statement: the tokens of the text, the token pass, parsing of the rewritten
//! tokens, the syntax pass, and planning. The parser reads the tokens of the token pass, never
//! printed text, so every token keeps its span in the source.

use datafusion::{
    common::{plan_err, DataFusionError, Result},
    dataframe::DataFrame,
    execution::SessionState,
    prelude::SessionContext,
    sql::{
        parser::Statement as DFStatement,
        sqlparser::{
            ast::Statement,
            dialect::GenericDialect,
            parser::{Parser, ParserError},
            tokenizer::Tokenizer,
        },
    },
};

use crate::{syntax, tokens};

/// The SQL of the query `text` after the token pass and the syntax pass, for the functions of
/// `state`, without planning it.
pub fn expand(state: &SessionState, text: &str) -> Result<String> {
    Ok(parse(state, text)?.to_string())
}

/// Plans the query `text` in `ctx`, after the token pass and the syntax pass. `text` holds one
/// statement.
pub async fn sql(ctx: &SessionContext, text: &str) -> Result<DataFrame> {
    let state = ctx.state();
    let statement = parse(&state, text)?;
    let plan = state
        .statement_to_plan(DFStatement::Statement(Box::new(statement)))
        .await?;
    ctx.execute_logical_plan(plan).await
}

/// The one statement of `text`, after the token pass and the syntax pass.
fn parse(state: &SessionState, text: &str) -> Result<Statement> {
    let mut statement = parse_tokens(text)?;
    syntax::lower(state, &mut statement)?;
    Ok(statement)
}

/// The one statement of `text`, after the token pass.
pub(crate) fn parse_tokens(text: &str) -> Result<Statement> {
    let dialect = GenericDialect {};
    let tokens = Tokenizer::new(&dialect, text)
        .tokenize_with_location()
        .map_err(|error| sql_error(error.into()))?;
    let tokens = tokens::rewrite(tokens)?;
    let mut statements = Parser::new(&dialect)
        .with_tokens_with_locations(tokens)
        .parse_statements()
        .map_err(sql_error)?;
    match statements.len() {
        1 => Ok(statements.remove(0)),
        count => plan_err!("a query holds one statement, not {count}"),
    }
}

/// A parser error as DataFusion reports one.
fn sql_error(error: ParserError) -> DataFusionError {
    DataFusionError::SQL(Box::new(error), None)
}
