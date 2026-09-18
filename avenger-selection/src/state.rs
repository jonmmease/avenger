use crate::{
    definitions::Producer,
    values::{canonical_tuples, canonical_value, same_meaning, tuple_cmp},
    Error, ProducerAddress, ProducerDefinition, Resolution, Result, SelectionDefinition,
    SelectionId, SelectionKind, SelectionTuple, SelectionValue,
};
use std::{collections::BTreeMap, sync::Arc};

/// One checked producer definition and its immutable selected values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contribution {
    pub(crate) producer: Producer,
    pub(crate) value: SelectionValue,
    effective_value: Option<SelectionValue>,
}
impl Contribution {
    /// Return the definition captured when this contribution was updated.
    pub fn producer(&self) -> &ProducerDefinition {
        &self.producer
    }
    /// Return normalized selected values.
    pub fn value(&self) -> &SelectionValue {
        &self.value
    }
    /// Return comparison values, with pixel ranges expressed as Int64 cell bounds.
    /// `value()` retains the original typed data bounds for overlays and regridding.
    pub fn effective_value(&self) -> &SelectionValue {
        self.effective_value.as_ref().unwrap_or(&self.value)
    }
    fn from_canonical(producer: Producer, value: SelectionValue) -> Result<Self> {
        let effective_value = crate::pixels::effective_value(&producer, &value)?;
        Ok(Self {
            producer,
            value,
            effective_value,
        })
    }
}

/// A pending update, validated atomically when applied to a snapshot.
#[derive(Clone, Debug)]
pub struct SelectionUpdate(Update);
#[derive(Clone, Debug)]
enum Update {
    Set(Producer, SelectionValue),
    Toggle(Producer, Vec<SelectionTuple>),
    Clear(ProducerAddress),
    ClearAll,
}
impl SelectionUpdate {
    /// Replace a contribution, or the entire named selection in Global mode.
    pub fn set(producer: &ProducerDefinition, value: SelectionValue) -> Self {
        Self(Update::Set(Arc::new(producer.clone()), value))
    }
    /// Toggle canonical point tuples, preserving their producer origin.
    /// Row-ID values use replacement updates.
    pub fn toggle(producer: &ProducerDefinition, tuples: Vec<SelectionTuple>) -> Self {
        Self(Update::Toggle(Arc::new(producer.clone()), tuples))
    }
    /// Remove only the specified producer in every resolution mode.
    pub fn clear(address: &ProducerAddress) -> Self {
        Self(Update::Clear(address.clone()))
    }
    /// Remove all active producers in this named selection.
    pub fn clear_all() -> Self {
        Self(Update::ClearAll)
    }
}

/// One named selection with independently attributed active contributions.
#[derive(Clone, Debug)]
pub struct SelectionSnapshot {
    pub(crate) definition: Arc<SelectionDefinition>,
    pub(crate) contributions: Arc<BTreeMap<ProducerAddress, Arc<Contribution>>>,
}
impl SelectionSnapshot {
    /// Start a named selection with no active producers.
    pub fn new(definition: SelectionDefinition) -> Result<Self> {
        Ok(Self {
            definition: Arc::new(definition),
            contributions: Arc::new(BTreeMap::new()),
        })
    }
    /// Return the shared name and resolution policy.
    pub fn definition(&self) -> &SelectionDefinition {
        &self.definition
    }
    /// Inspect active contributions in canonical address order.
    pub fn contributions(&self) -> impl Iterator<Item = &Contribution> {
        self.contributions.values().map(Arc::as_ref)
    }
    /// Apply an update without changing this snapshot or any retained contribution.
    pub fn apply(&self, update: SelectionUpdate) -> Result<Self> {
        let mut next = self.clone();
        match update.0 {
            Update::Set(producer, value) => {
                self.check_address(producer.address())?;
                let value = canonical_value(&producer, value)?;
                let contributions = Arc::make_mut(&mut next.contributions);
                if self.definition.resolution() == Resolution::Global {
                    contributions.clear();
                }
                contributions.insert(
                    producer.address().clone(),
                    Arc::new(Contribution::from_canonical(producer, value)?),
                );
            }
            Update::Clear(address) => {
                self.check_address(&address)?;
                Arc::make_mut(&mut next.contributions).remove(&address);
            }
            Update::ClearAll => Arc::make_mut(&mut next.contributions).clear(),
            Update::Toggle(producer, tuples) => {
                self.check_address(producer.address())?;
                if producer.kind() != SelectionKind::Point {
                    return Err(Error::InvalidUpdate(
                        "only point producers support toggle".into(),
                    ));
                }
                let tuples = canonical_tuples(&producer, tuples)?;
                // Configuration changes require a replacement so retained tuples
                // cannot silently acquire different projected meanings.
                if let Some(old) = self.contributions.get(producer.address()) {
                    if old.producer != producer {
                        return Err(Error::InvalidUpdate(
                            "replace a contribution before toggling with a changed definition"
                                .into(),
                        ));
                    }
                }
                let contributions = Arc::make_mut(&mut next.contributions);
                for tuple in tuples {
                    let mut removed = false;
                    let addresses: Vec<_> = if self.definition.resolution() == Resolution::Global {
                        contributions.keys().cloned().collect()
                    } else {
                        vec![producer.address().clone()]
                    };
                    for address in addresses {
                        let Some(old) = contributions.get(&address) else {
                            continue;
                        };
                        let SelectionValue::Tuples(old_tuples) = &old.value else {
                            continue;
                        };
                        let retained: Vec<_> = old_tuples
                            .iter()
                            .filter(|t| !same_meaning(&tuple, &producer, t, &old.producer))
                            .cloned()
                            .collect();
                        if retained.len() != old_tuples.len() {
                            removed = true;
                            if retained.is_empty() {
                                contributions.remove(&address);
                            } else {
                                contributions.insert(
                                    address,
                                    Arc::new(Contribution::from_canonical(
                                        old.producer.clone(),
                                        SelectionValue::Tuples(retained),
                                    )?),
                                );
                            }
                        }
                    }
                    if !removed {
                        let mut selected = contributions
                            .get(producer.address())
                            .map(|c| match &c.value {
                                SelectionValue::Tuples(t) => t.clone(),
                                SelectionValue::RowIds(_) => {
                                    unreachable!("tuple producer validated")
                                }
                            })
                            .unwrap_or_default();
                        selected.push(tuple);
                        selected.sort_by(tuple_cmp);
                        contributions.insert(
                            producer.address().clone(),
                            Arc::new(Contribution::from_canonical(
                                producer.clone(),
                                SelectionValue::Tuples(selected),
                            )?),
                        );
                    }
                }
            }
        }
        Ok(next)
    }
    fn check_address(&self, address: &ProducerAddress) -> Result<()> {
        if &address.selection != self.definition.id() {
            return Err(Error::InvalidUpdate(format!(
                "producer belongs to {}, not {}",
                address.selection,
                self.definition.id()
            )));
        }
        Ok(())
    }
}

/// A coherent immutable collection containing one snapshot per named selection.
#[derive(Clone, Debug)]
pub struct SelectionSet {
    selections: Arc<BTreeMap<SelectionId, SelectionSnapshot>>,
}
impl SelectionSet {
    /// Collect unique named snapshots. Duplicate names are errors.
    pub fn new(snapshots: impl IntoIterator<Item = SelectionSnapshot>) -> Result<Self> {
        let mut selections = BTreeMap::new();
        for snapshot in snapshots {
            let id = snapshot.definition.id().clone();
            if selections.insert(id.clone(), snapshot).is_some() {
                return Err(Error::DuplicateSelection(id));
            }
        }
        Ok(Self {
            selections: Arc::new(selections),
        })
    }
    /// Look up a required named selection, including an inactive one.
    pub fn get(&self, id: &SelectionId) -> Result<&SelectionSnapshot> {
        self.selections
            .get(id)
            .ok_or_else(|| Error::MissingSelection(id.clone()))
    }
    /// Return a set with one updated named snapshot.
    pub fn apply(&self, id: &SelectionId, update: SelectionUpdate) -> Result<Self> {
        self.apply_all([(id.clone(), update)])
    }
    /// Apply ordered updates atomically. An error publishes none of the updates.
    pub fn apply_all(
        &self,
        updates: impl IntoIterator<Item = (SelectionId, SelectionUpdate)>,
    ) -> Result<Self> {
        let mut candidate = self.clone();
        for (id, update) in updates {
            let updated = candidate.get(&id)?.apply(update)?;
            Arc::make_mut(&mut candidate.selections).insert(id, updated);
        }
        Ok(candidate)
    }
}
