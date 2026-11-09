//! The rules of selections: their names and resolutions, the fields of their clauses, and the
//! predicates of conditional values.

use std::collections::HashSet;

use arrow::datatypes::DataType;

use super::{fail, named, Checker, Result, REQUIRED};
use crate::{
    proto::{self, condition, predicate},
    Chart,
};

/// The names of the selections of `chart`.
pub(super) fn selections(chart: &Chart) -> HashSet<&str> {
    (chart.message.selections.iter())
        .map(|selection| selection.name.as_str())
        .collect()
}

impl Checker<'_> {
    /// Checks the fields of the initial clauses of each selection.
    pub(super) fn initial_clauses(&self) -> Result<()> {
        for (index, selection) in self.chart.message.selections.iter().enumerate() {
            for (position, clause) in selection.initial.iter().enumerate() {
                self.clause(clause, &format!("selections[{index}].initial[{position}]"))?;
            }
        }
        Ok(())
    }

    fn clause(&self, clause: &proto::Clause, path: &str) -> Result<()> {
        // A widget whose group anchors at the root writes a clause without an owner.
        if clause.widget.is_empty() {
            named(
                &clause.owner,
                &format!("{path}.owner"),
                "the owner of a clause",
            )?;
        }
        self.partition(&clause.partition, path)?;
        for (index, condition) in clause.conditions.iter().enumerate() {
            let path = format!("{path}.conditions[{index}]");
            match condition.kind.as_ref().expect(REQUIRED) {
                condition::Kind::Set(set) => {
                    let path = format!("{path}.set");
                    let mut length = None;
                    let mut others_length = None;
                    let complements = set.columns.iter().filter(|c| c.others.is_some()).count();
                    if complements != 0 && complements != set.columns.len() {
                        return Err(fail(
                            &path,
                            "every column of a set has `others`, or none does",
                        ));
                    }
                    for (position, column) in set.columns.iter().enumerate() {
                        let path = format!("{path}.columns[{position}]");
                        let values = self
                            .values
                            .list(column.values.as_ref(), &format!("{path}.values"))?;
                        if length.is_some_and(|length| length != values.len()) {
                            return Err(fail(
                                &path,
                                "the lists of the columns of a set have one length",
                            ));
                        }
                        length = Some(values.len());
                        if let Some(others) = &column.others {
                            let others_path = format!("{path}.others");
                            let others = self.values.list(Some(others), &others_path)?;
                            if !values.is_empty() && others.data_type() != values.data_type() {
                                return Err(fail(
                                    &others_path,
                                    "the `others` of a column of a set have the type of its values",
                                ));
                            }
                            if others_length.is_some_and(|length| length != others.len()) {
                                return Err(fail(
                                    &others_path,
                                    "the `others` of the columns of a set have one length",
                                ));
                            }
                            others_length = Some(others.len());
                        }
                    }
                }
                condition::Kind::Range(range) => {
                    let path = format!("{path}.range");
                    let low = (range.low.as_ref())
                        .map(|low| self.values.scalar(Some(low), &format!("{path}.low")))
                        .transpose()?;
                    let high = (range.high.as_ref())
                        .map(|high| self.values.scalar(Some(high), &format!("{path}.high")))
                        .transpose()?;
                    match (low, high) {
                        (None, None) => {
                            return Err(fail(&path, "a range has a low end, a high end, or both"))
                        }
                        (Some(low), Some(high)) => {
                            if low.data_type() != high.data_type() {
                                return Err(fail(&path, "the ends of a range have one type"));
                            }
                            let less = arrow::compute::kernels::cmp::lt(high, low)
                                .map_err(|error| fail(&path, error.to_string()))?;
                            if less.true_count() > 0 {
                                return Err(fail(
                                    &path,
                                    "the high end of a range is at or above its low end",
                                ));
                            }
                        }
                        _ => {}
                    }
                }
                condition::Kind::Polygon(polygon) => {
                    let path = format!("{path}.polygon");
                    for (position, vertices) in polygon.vertices.iter().enumerate() {
                        let path = format!("{path}.vertices[{position}]");
                        let vertices = self.values.list(vertices, &path)?;
                        let points = matches!(vertices.data_type(), DataType::Struct(fields)
                            if fields.len() == 2 && fields[0].name() == "x" && fields[1].name() == "y");
                        if !points || vertices.len() < 3 {
                            return Err(fail(
                                &path,
                                "the vertices of a polygon are three structs or more of an `x` and a `y` field",
                            ));
                        }
                    }
                }
                condition::Kind::CyclicRange(range) => {
                    let path = format!("{path}.cyclic_range");
                    if !(range.start.is_finite() && range.end.is_finite()) {
                        return Err(fail(&path, "a cyclic range has finite ends"));
                    }
                    if !(range.period.is_finite() && range.period > 0.0) {
                        return Err(fail(
                            format!("{path}.period"),
                            "the period of a cyclic range is more than 0 and finite",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn predicate(&self, predicate: &proto::Predicate, path: &str) -> Result<()> {
        match predicate.predicate.as_ref().expect(REQUIRED) {
            predicate::Predicate::Selected(selected) => {
                self.selection(&selected.selection, &format!("{path}.selected.selection"))
            }
            predicate::Predicate::All(list) | predicate::Predicate::Any(list) => {
                for (index, inner) in list.predicates.iter().enumerate() {
                    self.predicate(inner, &format!("{path}.predicates[{index}]"))?;
                }
                Ok(())
            }
            predicate::Predicate::Negate(inner) => self.predicate(inner, &format!("{path}.negate")),
        }
    }
}
