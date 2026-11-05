//! Walks over the fields of a chart message that refer to its values and its tables.

use crate::proto;

/// A message that holds `ValueRef`s, directly or through its fields. build.rs implements it for
/// each such message.
pub trait VisitValues {
    /// Calls `f` with each `ValueRef` of the message, in the order that the `.proto` files declare
    /// the fields, depth first, where a oneof takes the place of its first member.
    fn visit_values(&self, f: &mut dyn FnMut(&proto::ValueRef));
    /// Calls `f` with each `ValueRef` of the message to change, in the order of `visit_values`.
    fn visit_values_mut(&mut self, f: &mut dyn FnMut(&mut proto::ValueRef));
}

/// A message that holds `TableRef`s, directly or through its fields. build.rs implements it for
/// each such message.
pub trait VisitTables {
    /// Calls `f` with each `TableRef` of the message, in the order of `VisitValues`.
    fn visit_tables(&self, f: &mut dyn FnMut(&proto::TableRef));
    /// Calls `f` with each `TableRef` of the message to change, in the order of `visit_tables`.
    fn visit_tables_mut(&mut self, f: &mut dyn FnMut(&mut proto::TableRef));
}

/// A message that holds enum fields, directly or through its fields. build.rs implements it for
/// each such message.
pub(crate) trait VisitEnums {
    /// Calls `f` with the path of each enum field of the message from `path` that holds a value,
    /// the full name of the field's enum, such as `.avenger.chart.v1.Side`, and the field's value.
    /// An `optional` enum field that is not set holds none.
    fn visit_enums(&self, path: &str, f: &mut dyn FnMut(&str, &'static str, i32));
}

include!(concat!(env!("OUT_DIR"), "/walk.rs"));
