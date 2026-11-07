//! Selections and their initial clauses, and the predicates of conditional values.

use avenger_chart::proto::{self, condition, predicate, Channel};

use super::*;

pub(crate) fn selected(selected: proto::Selected) -> proto::Predicate {
    proto::Predicate {
        predicate: Some(predicate::Predicate::Selected(selected)),
    }
}

pub(crate) fn not(predicate: proto::Predicate) -> proto::Predicate {
    proto::Predicate {
        predicate: Some(predicate::Predicate::Negate(Box::new(predicate))),
    }
}

pub(crate) fn brush_of(name: &str) -> proto::Selected {
    proto::Selected {
        selection: name.to_string(),
        ..Default::default()
    }
}

/// A clause of `owner` with `condition`.
pub(crate) fn clause(owner: &str, condition: condition::Kind) -> proto::Clause {
    proto::Clause {
        owner: owner.to_string(),
        conditions: vec![proto::Condition {
            kind: Some(condition),
        }],
        ..Default::default()
    }
}

/// The clauses of the brush of a chart, with the values that they refer to.
pub(crate) type Clauses = Box<dyn FnOnce(&mut Parts) -> Vec<proto::Clause>>;

/// A clause of `owner` that selects the weights from `start` to `end`.
pub(crate) fn weights_between(owner: &'static str, start: f64, end: f64) -> Clauses {
    Box::new(move |parts| vec![weight_range(parts, owner, start, end)])
}

pub(crate) fn weight_range(parts: &mut Parts, owner: &str, start: f64, end: f64) -> proto::Clause {
    let low = parts.value(numbers(&[start]));
    let high = parts.value(numbers(&[end]));
    clause(
        owner,
        condition::Kind::Range(proto::ValueRange {
            column: "weight".to_string(),
            channel: Channel::X as i32,
            low: Some(low),
            high: Some(high),
            exclude_end: false,
        }),
    )
}

/// A `Select` tool of the selection `selection` with a box.
pub(crate) fn select(selection: &str) -> proto::Select {
    proto::Select {
        selection: selection.to_string(),
        form: Some(proto::select::Form::Interval(
            proto::IntervalSelect::default(),
        )),
        ..Default::default()
    }
}

pub(crate) fn no_clauses() -> Clauses {
    Box::new(|_| Vec::new())
}
