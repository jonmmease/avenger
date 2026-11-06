use arrow::array::ArrayRef;

use crate::walk::VisitValues;

/// A message of the format and the values that its `ValueRef`s refer to, counted from 0 in the
/// fragment's own values, such as the options of a node with its partition values or a scale spec
/// with its explicit domain. [`ChartParts::embed`](super::ChartParts::embed) adds a fragment to a
/// chart.
#[derive(Clone, Debug, PartialEq)]
pub struct Fragment<M> {
    /// The message.
    pub message: M,
    /// The values, each an array of one element.
    pub values: Vec<ArrayRef>,
}

impl<M> Fragment<M> {
    /// A fragment of a message without values.
    pub fn new(message: M) -> Self {
        Self {
            message,
            values: Vec::new(),
        }
    }

    /// A fragment of a message and the values that its `ValueRef`s refer to.
    pub fn with_values(message: M, values: Vec<ArrayRef>) -> Self {
        Self { message, values }
    }
}

impl<M: VisitValues> Fragment<M> {
    /// Appends the values of the fragment to `values` and returns its message, whose `ValueRef`s
    /// refer to them.
    pub(crate) fn embed_in(self, values: &mut Vec<ArrayRef>) -> M {
        let Self {
            mut message,
            values: own,
        } = self;
        let offset = values.len() as u32;
        message.visit_values_mut(&mut |value| value.index += offset);
        values.extend(own);
        message
    }
}
