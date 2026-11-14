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
//! [`register`] adds the mark functions and the constructors of avenger-chart to a session, and
//! the transforms of avenger-datafusion-transforms: the window functions `bin`, `stack`,
//! `timeunit`, and `lump`, and the table function `impute`.
//!
//! The parts of planning GoogleSQL with [`avenger_sql_syntax`]: [`library`] maps GoogleSQL's
//! functions and avenger's onto DataFusion, [`types`] maps GoogleSQL's types to Arrow types, and
//! [`catalog`] looks up the tables that a script names.

pub mod catalog;
mod functions;
pub mod library;
mod pipeline;
pub mod types;

use datafusion::{common::Result, prelude::SessionContext};

pub use pipeline::{expand, sql};

/// Registers the mark functions, the constructors, `scaled`, and `direct` of avenger-chart, and
/// the transforms of avenger-datafusion-transforms, in `ctx`. The transforms plan their table
/// functions with a query planner of their own, so `ctx` must have DataFusion's default query
/// planner, and registering two times adds their optimizer rules once.
pub fn register(ctx: &SessionContext) -> Result<()> {
    functions::register(ctx);
    avenger_datafusion_transforms::register(ctx)
}
