//! The functions that build avenger charts in a plan: a bound call of a mark, which
//! builds the chart of one layer from the rows of its group, and a bound call of a composition,
//! which composes the charts of the rows of its group. A planner binds each call to one of them,
//! so they hold their options and take only data columns.
//!
//! Every chart that they build names GoogleSQL's syntaxes for its format patterns,
//! [`FORMAT_SYNTAXES`].

mod bytes;
mod combine;
mod composition;
mod mark;
mod rows;

pub use combine::{combine, Combine};
pub use composition::{aggregate_name, CompositionInstance};
pub use mark::MarkInstance;

use avenger_chart::functions::{Arg, FormatSyntaxes};

/// The syntaxes of the format patterns of every chart that the aggregates build: GoogleSQL's
/// `FORMAT` specifiers for numbers and the format elements of `FORMAT_DATE` for dates, which a
/// host reads with the `googlesql` providers of avenger-format-number-googlesql and
/// avenger-format-datetime-googlesql.
pub const FORMAT_SYNTAXES: FormatSyntaxes<'static> = FormatSyntaxes {
    number: "googlesql",
    datetime: "googlesql",
};

/// The argument of the constant `value`, or `None` for `NULL`.
fn arg(value: &datafusion_common::ScalarValue) -> datafusion_common::Result<Option<Arg>> {
    Ok(Arg::from_array(value.to_array_of_size(1)?.as_ref(), 0))
}
