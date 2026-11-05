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

include!(concat!(env!("OUT_DIR"), "/walk.rs"));
