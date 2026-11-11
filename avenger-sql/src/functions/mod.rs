//! The functions of avenger-chart in DataFusion: an aggregate function for each mark of each layer,
//! which builds a chart of the rows of its group, a scalar function for each constructor, which
//! returns the fragment of its message, the compositions, as aggregates of the charts of a group
//! and as scalars of the charts of a row, `chart`, which marks a column of charts, and the scalar
//! functions `scaled` and `direct`. They are a binding of `avenger_chart::functions`.
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

mod charts;
mod combine;
mod composition;
mod constructor;
mod declaration;
mod instance;
mod kind;
mod order;
mod rows;

use std::sync::Arc;

use datafusion::{
    arrow::datatypes::{DataType, Field},
    logical_expr::AggregateUDF,
    logical_expr::ScalarUDF,
    optimizer::AnalyzerRule,
    prelude::SessionContext,
};

use avenger_chart::{
    functions::{compositions, constructors, marks, Arg, Kind},
    EXTENSION_NAME,
};

/// Registers the mark functions, the constructors, the compositions, `chart`, `scaled`, and
/// `direct` in `ctx`.
pub(crate) fn register(ctx: &SessionContext) {
    for mark in marks() {
        ctx.register_udaf(AggregateUDF::new_from_impl(
            declaration::MarkDeclaration::new(mark),
        ));
    }
    for constructor in constructors() {
        ctx.register_udf(ScalarUDF::new_from_impl(
            constructor::ConstructorFunction::new(constructor),
        ));
    }
    for composition in compositions() {
        ctx.register_udaf(AggregateUDF::new_from_impl(
            composition::CompositionDeclaration::new(composition),
        ));
        // A layer and the concats also compose the charts of a row; a facet lays out rows.
        if !composition.kind.takes_key() {
            ctx.register_udf(ScalarUDF::new_from_impl(combine::CombineFunction::new(
                composition,
            )));
        }
    }
    // The rule orders the children of compositions, one time however often the session registers.
    let rule = order::CompositionOrder;
    let known = (ctx.state().analyzer().rules.iter()).any(|known| known.name() == rule.name());
    if !known {
        ctx.add_analyzer_rule(Arc::new(rule));
    }
    ctx.register_udf(ScalarUDF::new_from_impl(charts::ChartFunction::new()));
    for kind in [Kind::Scaled, Kind::Direct] {
        ctx.register_udf(ScalarUDF::new_from_shared_impl(Arc::new(
            kind::KindFunction::new(kind),
        )));
    }
}

/// Whether a column of `field` can hold charts: a column of the chart extension, or any
/// `BinaryView` column, because DataFusion drops the extension of a field through many functions,
/// such as `CASE` and `coalesce`. A function that reads charts decodes the values of such a column
/// and fails on a value that is not a chart.
pub(crate) fn holds_charts(field: &Field) -> bool {
    let marked = field
        .metadata()
        .get("ARROW:extension:name")
        .map(String::as_str);
    marked == Some(EXTENSION_NAME) || field.data_type() == &DataType::BinaryView
}

/// The argument of the constant `value`, or `None` for `NULL`.
fn arg(value: &datafusion::common::ScalarValue) -> datafusion::common::Result<Option<Arg>> {
    Ok(Arg::from_array(value.to_array_of_size(1)?.as_ref(), 0))
}
