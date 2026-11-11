//! DataFusion SQL that builds avenger charts.
//!
//! A mark of avenger-chart is an aggregate function, so a query builds a chart of its rows, or one
//! chart for each group with `GROUP BY`, in a `BinaryView` column whose field has the extension
//! name `avenger.chart`:
//!
//! ```sql
//! FROM penguins
//! |> AGGREGATE count(*) AS n GROUP BY species
//! |> AGGREGATE bar(x => species, y => n) AS chart
//! ```
//!
//! [`sql`] plans a query in a session after two passes that DataFusion 55.2 needs to plan it, and
//! [`expand`] returns the SQL after them. The token pass rewrites the tokens of the text before
//! parsing, and the syntax pass rewrites the parsed statement:
//!
//! - A word that starts with `__avenger_`, a prefix of the names that the passes generate, is an
//!   error at the word.
//! - A trailing comma, before `)`, `]`, `}`, `|>`, `;`, the end of the text, or a clause keyword
//!   that starts its clause, such as `GROUP BY`, is a space, so a call, a list, or the items of an
//!   operator can end in a comma.
//! - A query that starts with `FROM` selects `*`, because DataFusion plans no columns for it.
//! - A pipe `SELECT` or `EXTEND` that calls a window function or `unnest` becomes a `SELECT` over
//!   a derived table of the operators before it, because DataFusion fails to run a window of a
//!   pipe `EXTEND`. The derived table keeps the qualifiers of the tables before it.
//! - A pipe `CALL f(<args>) [OVER (PARTITION BY ...)]` becomes `SELECT * FROM f((<input>),
//!   <args>, partition_by => [...])`, the `FROM`-position call of the table function over the
//!   operators before it, because DataFusion does not plan a pipe `CALL` of a table function.
//! - A named option of `bin`, `timeunit`, `lump`, or `stack` becomes the option pair of the
//!   transform, so `bin(x, maxbins => 30)` is `bin(x, 'maxbins', 30)`. The `weight` and `keep` of
//!   `lump` pass the text of their expressions.
//! - A call with named arguments passes `NULL` for each parameter before its last named argument
//!   that it omits, because DataFusion binds named arguments only as a prefix of the parameters.
//!   An omitted parameter of an avenger function and `NULL` mean the same thing.
//!
//! - `|> CHART <item> [AS name], ... [GROUP BY ...]`, whose items are mark calls and aggregates,
//!   parses as a pipe `AGGREGATE` and becomes the `FROM`-position call `__avenger_chart((<input>),
//!   <mark call>, partition_by => [...], name => <name>)` of each mark call, with a chart for each
//!   group, which the mark planner plans as the lifting of the call: stages that compute the
//!   discretizers, `bin`, `timeunit`, and `lump`, the aggregates, the rows of its `impute` option,
//!   and the stacks and the window functions of its channels, then the mark aggregate over their
//!   columns. With aggregate items or several mark calls, the calls and the `AGGREGATE` of the
//!   aggregate items, each over a copy of the input, join on the groups. [`lift`] prints the
//!   lifting, which runs to the same charts.
//!
//! The parser reads the tokens of the token pass, so the syntax tree keeps the spans of the text,
//! and [`register`] has DataFusion record them, so an error of a column points to its text.
//!
//! [`register`] adds the mark functions and the constructors of avenger-chart to a session, and
//! the transforms of avenger-datafusion-transforms: the window functions `bin`, `stack`,
//! `timeunit`, and `lump`, and the table function `impute`.

mod chart;
mod error;
mod functions;
mod pipeline;
mod registry;
mod syntax;
mod tokens;

use datafusion::{common::Result, prelude::SessionContext};

pub use chart::lift;
pub use pipeline::{expand, sql};

/// Registers the mark functions, the constructors, `scaled`, and `direct` of avenger-chart, and
/// the transforms of avenger-datafusion-transforms, in `ctx`, and has its planner record the spans
/// of the text (`datafusion.sql_parser.collect_spans`). The transforms plan their table functions
/// with a query planner of their own, so `ctx` must have DataFusion's default query planner, and
/// registering two times adds their optimizer rules once.
pub fn register(ctx: &SessionContext) -> Result<()> {
    (ctx.state_ref().write().config_mut().options_mut())
        .sql_parser
        .collect_spans = true;
    functions::register(ctx);
    chart::register(ctx)?;
    avenger_datafusion_transforms::register(ctx)
}
