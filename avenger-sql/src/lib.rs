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
//! [`sql`] plans a query in a session after the rewrites that DataFusion 55.2 needs to plan it,
//! and [`expand`] returns the SQL after them:
//!
//! - A query that starts with `FROM` selects `*`, because DataFusion plans no columns for it.
//! - A call with named arguments passes `NULL` for each parameter before its last named argument
//!   that it omits, because DataFusion binds named arguments only as a prefix of the parameters.
//!   An omitted parameter of an avenger function and `NULL` mean the same thing.
//!
//! [`register`] adds the mark functions of avenger-chart to a session.

mod pipeline;

use datafusion::prelude::SessionContext;

pub use pipeline::{expand, sql};

/// Registers the mark functions of avenger-chart, `scaled`, and `direct` in `ctx`.
pub fn register(ctx: &SessionContext) {
    avenger_chart::datafusion::register(ctx);
}
