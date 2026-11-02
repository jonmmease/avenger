//! Which rows the selections of a chart select, through avenger-selection: the chart's predicates
//! convert to avenger-selection's for the owners of the rows, the clauses and the resolutions of
//! its selections to selection states, and the label of each condition's channel is the
//! channel's name.

use std::collections::BTreeMap;

use avenger_selection::{Predicate, Resolution, SelectedOptions, SelectionState};

use crate::{
    proto::{self, predicate, Channel},
    Chart, Clause,
};

impl Chart {
    /// The predicate of avenger-selection for `predicate`, which tests the rows of a plot whose
    /// owners, its unit and the overlays that hold it, have the ids `owners`. A `Selected` with
    /// `exclude_own` skips the owners' clauses, and with `faceted`, each row skips only the
    /// clauses of its own panel, by the partition columns of the owners' clauses. A predicate
    /// without a kind selects no row.
    pub fn predicate(
        &self,
        predicate: &proto::Predicate,
        owners: &[String],
        faceted: bool,
    ) -> Predicate {
        convert(predicate, owners, faceted)
    }

    /// The state of each of the chart's selections by name: its resolution, and its clauses in
    /// `clauses`, the clauses of the selections by name, which a selection without clauses
    /// lacks.
    pub fn selection_states(
        &self,
        clauses: &BTreeMap<String, Vec<Clause>>,
    ) -> BTreeMap<String, SelectionState> {
        (self.selections().iter())
            .map(|selection| {
                let resolution = proto::SelectionResolution::try_from(selection.resolution)
                    .map_or(Resolution::Global, resolution);
                let clauses = clauses.get(&selection.name).cloned().unwrap_or_default();
                (
                    selection.name.clone(),
                    SelectionState::new(resolution, clauses),
                )
            })
            .collect()
    }
}

/// The resolution of avenger-selection for a resolution of the format.
pub fn resolution(resolution: proto::SelectionResolution) -> Resolution {
    match resolution {
        proto::SelectionResolution::Intersect => Resolution::Intersect,
        proto::SelectionResolution::Union => Resolution::Union,
        _ => Resolution::Global,
    }
}

/// A predicate of the format as a predicate of avenger-selection, for rows of the owners
/// `owners`, faceted when `faceted`.
fn convert(predicate: &proto::Predicate, owners: &[String], faceted: bool) -> Predicate {
    let list = |list: &proto::PredicateList| {
        (list.predicates.iter())
            .map(|predicate| convert(predicate, owners, faceted))
            .collect()
    };
    match predicate.predicate.as_ref() {
        Some(predicate::Predicate::Selected(selected)) => Predicate::selected(
            selected.selection.clone(),
            SelectedOptions {
                exclude_own: match selected.exclude_own {
                    true => owners.to_vec(),
                    false => Vec::new(),
                },
                faceted,
                empty_selects_none: selected.empty_selects_none,
                without: Vec::new(),
            },
        ),
        Some(predicate::Predicate::And(predicates)) => Predicate::And(list(predicates)),
        Some(predicate::Predicate::Or(predicates)) => Predicate::Or(list(predicates)),
        Some(predicate::Predicate::Not(inner)) => {
            Predicate::Not(Box::new(convert(inner, owners, faceted)))
        }
        None => Predicate::Or(Vec::new()),
    }
}

impl Channel {
    /// The label of the channel in avenger-selection's conditions: its name in lowercase without
    /// the `CHANNEL_` prefix, such as `x` or `theta2`, or `None` for an unspecified channel.
    pub fn label(self) -> Option<String> {
        match self {
            Channel::Unspecified => None,
            channel => {
                let name = channel.as_str_name();
                Some(name.strip_prefix("CHANNEL_").unwrap_or(name).to_lowercase())
            }
        }
    }

    /// The channel of the label `label`, as [`label`](Self::label) gives it, or
    /// [`Channel::Unspecified`] for `None` or an unknown label.
    pub fn from_label(label: Option<&str>) -> Self {
        (label
            .and_then(|label| Channel::from_str_name(&format!("CHANNEL_{}", label.to_uppercase()))))
        .unwrap_or(Channel::Unspecified)
    }
}
