use crate::{
    definitions::Producer,
    identity::ProducerAddress,
    values::{canonical_tuples, canonical_value, same_meaning, tuple_cmp},
    Error, ProducerDefinition, Resolution, Result, SelectionId, SelectionValue,
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

/// A pending update carrying its destination selection, validated when applied to a set.
#[derive(Clone, Debug)]
pub struct SelectionUpdate(Update);
#[derive(Clone, Debug)]
enum Update {
    Set(Producer, SelectionValue),
    Toggle(Producer, SelectionValue),
    Clear(ProducerAddress),
    ClearAll(SelectionId),
}
impl SelectionUpdate {
    /// Replace a contribution, or the entire named selection in Global mode.
    pub fn set(producer: &ProducerDefinition, value: SelectionValue) -> Self {
        Self(Update::Set(Arc::new(producer.clone()), value))
    }
    /// Toggle whole normalized tuples, preserving their producer origin.
    /// Compare raw values, including range bounds, before applying pixel grids.
    pub fn toggle(producer: &ProducerDefinition, value: SelectionValue) -> Self {
        Self(Update::Toggle(Arc::new(producer.clone()), value))
    }
    /// Remove only the specified producer in every resolution mode.
    pub fn clear(producer: &ProducerDefinition) -> Self {
        Self(Update::Clear(producer.address().clone()))
    }
    /// Remove all active producers in this named selection.
    pub fn clear_all(selection: &SelectionId) -> Self {
        Self(Update::ClearAll(selection.clone()))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct NamedSelection {
    pub(crate) resolution: Resolution,
    pub(crate) contributions: Arc<BTreeMap<ProducerAddress, Arc<Contribution>>>,
}
impl NamedSelection {
    fn apply(&self, update: SelectionUpdate) -> Result<Self> {
        let mut next = self.clone();
        match update.0 {
            Update::Set(producer, value) => {
                let value = canonical_value(&producer, value)?;
                let contributions = Arc::make_mut(&mut next.contributions);
                if self.resolution == Resolution::Global {
                    contributions.clear();
                }
                contributions.insert(
                    producer.address().clone(),
                    Arc::new(Contribution::from_canonical(producer, value)?),
                );
            }
            Update::Clear(address) => {
                Arc::make_mut(&mut next.contributions).remove(&address);
            }
            Update::ClearAll(_) => Arc::make_mut(&mut next.contributions).clear(),
            Update::Toggle(producer, value) => {
                let tuples = canonical_tuples(&producer, value.tuples)?;
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
                    let addresses: Vec<_> = if self.resolution == Resolution::Global {
                        contributions.keys().cloned().collect()
                    } else {
                        vec![producer.address().clone()]
                    };
                    for address in addresses {
                        let Some(old) = contributions.get(&address) else {
                            continue;
                        };
                        let old_tuples = &old.value.tuples;
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
                                        SelectionValue { tuples: retained },
                                    )?),
                                );
                            }
                        }
                    }
                    if !removed {
                        let mut selected = contributions
                            .get(producer.address())
                            .map(|c| c.value.tuples.clone())
                            .unwrap_or_default();
                        selected.push(tuple);
                        selected.sort_by(tuple_cmp);
                        contributions.insert(
                            producer.address().clone(),
                            Arc::new(Contribution::from_canonical(
                                producer.clone(),
                                SelectionValue { tuples: selected },
                            )?),
                        );
                    }
                }
            }
        }
        Ok(next)
    }
}

/// Immutable state for named selections and their active producer contributions.
#[derive(Clone, Debug)]
pub struct SelectionSet {
    selections: Arc<BTreeMap<SelectionId, NamedSelection>>,
}
impl SelectionSet {
    /// Start each named selection with no active producers. Duplicate names are errors.
    pub fn new(definitions: impl IntoIterator<Item = (SelectionId, Resolution)>) -> Result<Self> {
        let mut selections = BTreeMap::new();
        for (id, resolution) in definitions {
            let selection = NamedSelection {
                resolution,
                contributions: Arc::new(BTreeMap::new()),
            };
            if selections.insert(id.clone(), selection).is_some() {
                return Err(Error::DuplicateSelection(id));
            }
        }
        Ok(Self {
            selections: Arc::new(selections),
        })
    }

    /// Inspect active contributions in canonical address order, for example to draw overlays.
    pub fn contributions(&self, id: &SelectionId) -> Result<impl Iterator<Item = &Contribution>> {
        Ok(self.get(id)?.contributions.values().map(Arc::as_ref))
    }

    /// Return the combination policy for a named selection, including an inactive one.
    pub fn resolution(&self, id: &SelectionId) -> Result<Resolution> {
        Ok(self.get(id)?.resolution)
    }

    /// Replace this producer's contribution, or the whole named selection in Global mode.
    pub fn set(&self, producer: &ProducerDefinition, value: SelectionValue) -> Result<Self> {
        self.apply(SelectionUpdate::set(producer, value))
    }

    /// Toggle whole normalized tuples. Overlapping ranges remain separate tuples.
    /// Pixel grids affect membership, while raw values determine which tuple to toggle.
    pub fn toggle(&self, producer: &ProducerDefinition, value: SelectionValue) -> Result<Self> {
        self.apply(SelectionUpdate::toggle(producer, value))
    }

    /// Remove one producer's contribution.
    pub fn clear(&self, producer: &ProducerDefinition) -> Result<Self> {
        self.apply(SelectionUpdate::clear(producer))
    }

    /// Remove all contributions to one named selection.
    pub fn clear_all(&self, selection: &SelectionId) -> Result<Self> {
        self.apply(SelectionUpdate::clear_all(selection))
    }

    /// Apply one update to the named selection carried by the update.
    pub fn apply(&self, update: SelectionUpdate) -> Result<Self> {
        self.apply_all([update])
    }

    /// Apply ordered updates atomically. An error publishes none of the updates.
    pub fn apply_all(&self, updates: impl IntoIterator<Item = SelectionUpdate>) -> Result<Self> {
        let mut candidate = self.clone();
        for update in updates {
            let id = match &update.0 {
                Update::Set(producer, _) | Update::Toggle(producer, _) => {
                    &producer.address().selection
                }
                Update::Clear(address) => &address.selection,
                Update::ClearAll(id) => id,
            }
            .clone();
            let updated = candidate.get(&id)?.apply(update)?;
            Arc::make_mut(&mut candidate.selections).insert(id, updated);
        }
        Ok(candidate)
    }

    pub(crate) fn get(&self, id: &SelectionId) -> Result<&NamedSelection> {
        self.selections
            .get(id)
            .ok_or_else(|| Error::MissingSelection(id.clone()))
    }
}
