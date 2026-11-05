//! The rules of selections: their names and resolutions, the fields of their clauses, and the
//! predicates of conditional values.

use std::collections::HashSet;

use arrow::datatypes::DataType;

use super::{fail, named, Checker, Result};
use crate::{
    proto::{self, condition, predicate},
    Chart,
};

/// Checks the selections of `chart` and returns their names.
pub(super) fn selections(chart: &Chart) -> Result<HashSet<&str>> {
    let mut names = HashSet::new();
    for (index, selection) in chart.message.selections.iter().enumerate() {
        named(
            &selection.name,
            &format!("selections[{index}].name"),
            "the name of a selection",
        )?;
        names.insert(selection.name.as_str());
    }
    Ok(names)
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
        named(
            &clause.owner,
            &format!("{path}.owner"),
            "the owner of a clause",
        )?;
        if clause.conditions.is_empty() {
            return Err(fail(
                format!("{path}.conditions"),
                "a clause has a condition or more",
            ));
        }
        self.partition(&clause.partition, path)?;
        for (index, condition) in clause.conditions.iter().enumerate() {
            let path = format!("{path}.conditions[{index}]");
            match (condition.kind.as_ref())
                .ok_or_else(|| fail(&path, "a condition always sets its kind"))?
            {
                condition::Kind::Set(set) => {
                    let path = format!("{path}.set");
                    if set.columns.is_empty() {
                        return Err(fail(
                            format!("{path}.columns"),
                            "a set has a column or more",
                        ));
                    }
                    let mut length = None;
                    for (position, column) in set.columns.iter().enumerate() {
                        let path = format!("{path}.columns[{position}]");
                        named(
                            &column.column,
                            &format!("{path}.column"),
                            "the column of a condition",
                        )?;
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
                    }
                }
                condition::Kind::Range(range) => {
                    let path = format!("{path}.range");
                    named(
                        &range.column,
                        &format!("{path}.column"),
                        "the column of a condition",
                    )?;
                    let low = self
                        .values
                        .scalar(range.low.as_ref(), &format!("{path}.low"))?;
                    let high = self
                        .values
                        .scalar(range.high.as_ref(), &format!("{path}.high"))?;
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
                condition::Kind::Polygon(polygon) => {
                    let path = format!("{path}.polygon");
                    named(
                        &polygon.x_column,
                        &format!("{path}.x_column"),
                        "the column of a condition",
                    )?;
                    named(
                        &polygon.y_column,
                        &format!("{path}.y_column"),
                        "the column of a condition",
                    )?;
                    if polygon.vertices.is_empty() {
                        return Err(fail(
                            format!("{path}.vertices"),
                            "a polygon condition has a polygon or more",
                        ));
                    }
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
                    named(
                        &range.column,
                        &format!("{path}.column"),
                        "the column of a condition",
                    )?;
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
        match (predicate.predicate.as_ref())
            .ok_or_else(|| fail(path, "a predicate always sets its kind"))?
        {
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
