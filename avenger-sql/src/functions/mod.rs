//! The functions of avenger-chart in DataFusion: an aggregate function for each mark of each layer,
//! which builds a chart of the rows of its group, a scalar function for each constructor, which
//! returns the fragment of its message, and the scalar functions `scaled` and `direct`. They are
//! a binding of `avenger_chart::functions`.
//!
//! A mark function takes each channel of its mark as a named parameter:
//!
//! ```sql
//! SELECT bar(x => species, y => n, color => island) AS chart FROM counts
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
//! `scaled(value)` gives the scaled kind, such as `color => scaled('Adelie')` for the color of a
//! category, and `direct(value)` the direct kind, such as `fill => direct(color)` for a column of
//! colors, or `x => direct(0.5)` for the middle of the plot. The `ORDER BY` of the call is the
//! draw order of the rows, and `GROUP BY` gives one chart for each group.
//!
//! The registered function of a mark is a declaration. DataFusion's expression simplifier binds
//! each call of it, through `AggregateUDFImpl::simplify`, to an aggregate of its own that takes
//! only the columns of the call, so the errors of a call are planning errors.

pub(crate) mod constructor;
pub(crate) mod declaration;
mod instance;
pub(crate) mod kind;

use avenger_chart::functions::Arg;

/// The argument of the constant `value`, or `None` for `NULL`.
fn arg(value: &datafusion::common::ScalarValue) -> datafusion::common::Result<Option<Arg>> {
    Ok(Arg::from_array(value.to_array_of_size(1)?.as_ref(), 0))
}
