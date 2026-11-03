//! The clauses of a selection: the part of the selection that each owner wrote.

use std::collections::BTreeMap;

use crate::Condition;

/// The part of a selection that one owner, such as a plot, wrote: conditions that a row matches
/// when it matches all of them.
#[must_use]
#[derive(Clone, Debug)]
pub struct Clause {
    /// The id of the unit or the overlay that wrote it.
    pub owner: String,
    /// The partition values of the owner, its own and those of its ancestors, by column.
    pub partition: BTreeMap<String, avenger_scales::scalar::Scalar>,
    /// The conditions, all of which a row matches.
    pub conditions: Vec<Condition>,
}

impl Clause {
    /// A clause of the unit or the overlay with the id `owner`.
    pub fn new(owner: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            partition: BTreeMap::new(),
            conditions: Vec::new(),
        }
    }

    /// A partition value of the owner: its own values and those of its ancestors name it, with
    /// its id.
    pub fn partition(
        mut self,
        column: impl Into<String>,
        value: impl Into<avenger_scales::scalar::Scalar>,
    ) -> Self {
        self.partition.insert(column.into(), value.into());
        self
    }

    /// A condition, which a row matches together with the clause's other conditions.
    pub fn condition(mut self, condition: Condition) -> Self {
        self.conditions.push(condition);
        self
    }

    /// Conditions, which a row matches together with the clause's other conditions.
    pub fn conditions(mut self, conditions: impl IntoIterator<Item = Condition>) -> Self {
        self.conditions.extend(conditions);
        self
    }
}
