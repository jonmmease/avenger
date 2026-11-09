//! The mark functions of DataFusion, behind the `datafusion` feature: an aggregate function for
//! each mark of the Cartesian and the polar layers, which builds a chart of the rows of its group,
//! and the scalar functions `scaled` and `direct`.
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
//! it ([`IntoEncoding`](crate::builder::IntoEncoding)):
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

mod argument;
mod constructor;
mod declaration;
mod instance;
mod kind;

use std::sync::Arc;

use datafusion::{logical_expr::AggregateUDF, logical_expr::ScalarUDF, prelude::SessionContext};

use crate::{layer::MarkMessage, proto, CoordKind};

pub use constructor::FRAGMENT_EXTENSION_NAME;
pub use kind::{DIRECT, SCALED};

/// A mark function: its name, the coordinate system of its mark, its documentation, its mark in
/// its layer's `mark` oneof, without encodings, and its channels, which are its parameters.
pub(crate) struct MarkSpec {
    pub(crate) name: &'static str,
    pub(crate) coord: CoordKind,
    pub(crate) doc: &'static str,
    pub(crate) mark: fn() -> LayerMark,
    pub(crate) channels: &'static [ChannelSpec],
}

/// A channel of a mark function: a parameter that takes any expression, named after its field.
pub(crate) struct ChannelSpec {
    pub(crate) name: &'static str,
    pub(crate) channel: proto::Channel,
    pub(crate) required: bool,
    pub(crate) doc: &'static str,
}

/// A mark in the `mark` oneof of its layer.
pub(crate) enum LayerMark {
    Cartesian(proto::cartesian_layer::Mark),
    Polar(proto::polar_layer::Mark),
}

impl LayerMark {
    fn as_mark(&mut self) -> &mut dyn MarkMessage {
        match self {
            Self::Cartesian(mark) => mark,
            Self::Polar(mark) => mark,
        }
    }
}

include!(concat!(env!("OUT_DIR"), "/functions.rs"));

/// The names of the mark functions, in the order of the format's layers and their marks.
pub fn mark_names() -> impl Iterator<Item = &'static str> {
    MARKS.iter().map(|spec| spec.name)
}

/// Registers the mark functions, the constructors, `scaled`, and `direct` in `ctx`.
pub fn register(ctx: &SessionContext) {
    for spec in MARKS {
        ctx.register_udaf(AggregateUDF::new_from_impl(
            declaration::MarkDeclaration::new(spec),
        ));
    }
    for spec in constructor::generated::CONSTRUCTORS {
        ctx.register_udf(ScalarUDF::new_from_impl(
            constructor::ConstructorFunction::new(spec),
        ));
    }
    for kind in [kind::Kind::Scaled, kind::Kind::Direct] {
        ctx.register_udf(ScalarUDF::new_from_shared_impl(Arc::new(
            kind::KindFunction::new(kind),
        )));
    }
}

/// The names of the constructors, in the order of the option tree.
pub fn constructor_names() -> impl Iterator<Item = &'static str> {
    constructor::generated::CONSTRUCTORS
        .iter()
        .map(|spec| spec.name)
}

/// The text of every constructor's parameters and the arguments that each takes, which the
/// snapshot test of the constructors compares.
pub fn constructor_surface() -> &'static str {
    constructor::generated::SURFACE
}
