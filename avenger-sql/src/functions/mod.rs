//! The functions of avenger-chart in DataFusion: an aggregate function for each mark of each layer,
//! which builds a chart of the rows of its group, an aggregate function for each composition,
//! which composes the charts of the rows of its group, a scalar function for each layer and
//! concat, which composes the charts of a row, a scalar function for each constructor,
//! which returns the fragment of its message, a scalar function for each channel with a scale,
//! such as `x`, and the scalar functions `datum` and `value`. They are a binding of
//! `avenger_chart::functions`.
//!
//! A mark function takes each channel of its mark as a named parameter:
//!
//! ```sql
//! FROM counts
//! |> AGGREGATE mark_bar(x => species, y => n, color => island) AS chart
//! ```
//!
//! A channel argument that reads the rows is a column of the chart's layer table, named by the
//! expression without the qualifiers of its columns, such as `body_mass * 2`. Channels with equal
//! expressions share one column. A bare value takes the kind that the builders of the format give
//! it ([`IntoEncoding`](avenger_chart::builder::IntoEncoding)):
//!
//! - `NULL`, the value of an omitted parameter, leaves the channel unset.
//! - A value of the rows is a scaled field on a channel with a scale, such as `color`, and a
//!   direct field on a channel without one, such as `text`.
//! - A constant is a scaled value on a position with a scale, such as `y => 0` for a reference
//!   line, and a direct value on another channel, such as `fill => 'steelblue'`.
//!
//! `datum(value)` gives a constant the scaled kind, such as `color => datum('Adelie')` for the
//! color of a category, and `value(value)` the direct kind, such as `fill => value(color)` for a
//! column of colors, or `x => value(0.5)` for the middle of the plot. A channel with a scale also
//! takes its channel function, which keeps the kind of its value and sets the options of the
//! channel's encoding, as `color => color(datum('Adelie'), legend => FALSE)` and
//! `y => y(mass, title => 'Mass')` do. The `ORDER BY` of the call is the draw order of the rows,
//! and `GROUP BY` gives one chart for each group.
//!
//! Every chart that the functions build names GoogleSQL's syntaxes for its format patterns,
//! [`FORMAT_SYNTAXES`]: a number pattern is a `FORMAT` specifier, such as `%'.2f`, and a date
//! pattern holds the format elements of `FORMAT_DATE`, such as `%Y-%m`.
//!
//! The registered function of a mark is a declaration. DataFusion's expression simplifier binds
//! each call of it, through `AggregateUDFImpl::simplify`, to an aggregate of its own that takes
//! only the columns of the call, so the errors of a call are planning errors.

pub(crate) mod combine;
pub(crate) mod composition;
pub(crate) mod constructor;
pub(crate) mod declaration;
pub(crate) mod kind;

use avenger_chart::functions::Arg;
pub(crate) use avenger_physical_graph::functions::FORMAT_SYNTAXES;

/// The argument of the constant `value`, or `None` for `NULL`.
fn arg(value: &datafusion::common::ScalarValue) -> datafusion::common::Result<Option<Arg>> {
    Ok(Arg::from_array(value.to_array_of_size(1)?.as_ref(), 0))
}
