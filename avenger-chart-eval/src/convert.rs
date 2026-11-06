//! The conversions of the format's messages to the models of avenger-scales and
//! avenger-selection: scales with their options and ranges, domains and their fits, selection
//! states and predicates, and the check of the chart's format syntaxes.

pub(crate) mod domain;
pub(crate) mod format;
pub(crate) mod scale;
pub(crate) mod selection;

#[cfg(test)]
mod tests;
