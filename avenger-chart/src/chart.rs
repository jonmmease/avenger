use std::collections::BTreeMap;

use arrow::{
    array::{ArrayRef, AsArray},
    datatypes::DataType,
    record_batch::RecordBatch,
};
use avenger_scales::{
    scalar::Scalar,
    scales::domain_summary::{
        CategoryOptions, CategoryOrder, DomainOptions, DomainSummary, IntervalOptions,
    },
};

use crate::{
    proto::{self, category_fit, domain_fit, tuple_field, Channel},
    tree, validate, ChartError, Contribution, TupleTest,
};

/// A chart: its protobuf message, which is the model of the format, with the data tables that
/// its units refer to and the values that its fields refer to, decoded as Arrow arrays.
///
/// A chart is always valid: decoding and the builder check every rule of the format before they
/// return one. It holds no runtime state, so it renders the same after a round trip through
/// bytes. Cloning a chart copies its message and shares the buffers of its tables and values.
#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    /// The message, whose `tables` are empty and whose `values` is not set: its units refer to
    /// `tables` by `Unit.table`, and its fields refer to `values` by index.
    message: proto::Chart,
    tables: Vec<RecordBatch>,
    /// One value for each field that refers to one, in the order that `Chart.values` describes.
    values: Vec<ArrayRef>,
}

impl Chart {
    /// A chart from its message, the tables that its units refer to by `Unit.table`, and the
    /// values that its fields refer to by index, each an array of one element. The message's own
    /// `tables` are empty and its `values` is not set, because the chart holds them decoded. It
    /// fails for a chart that breaks a rule of the format.
    ///
    /// The chart orders its values as `Chart.values` describes and renumbers the fields, so equal
    /// charts are equal whatever the order of the values that they come from.
    pub fn new(
        message: proto::Chart,
        tables: Vec<RecordBatch>,
        values: Vec<ArrayRef>,
    ) -> Result<Self, ChartError> {
        let mut chart = Self {
            message,
            tables,
            values,
        };
        validate::check(&chart)?;
        chart.values = canonical(&mut chart.message, &chart.values);
        Ok(chart)
    }

    /// The message of the chart. Its `tables` are empty and its `values` is not set, because the
    /// chart holds them decoded.
    pub fn message(&self) -> &proto::Chart {
        &self.message
    }

    /// The root node of the chart.
    pub fn root(&self) -> &proto::Node {
        self.message
            .root
            .as_ref()
            .expect("a valid chart has a root")
    }

    /// The selections of the chart, in the order of their names.
    pub fn selections(&self) -> &[proto::Selection] {
        &self.message.selections
    }

    /// The tables that the units refer to by `Unit.table`.
    pub fn tables(&self) -> &[RecordBatch] {
        &self.tables
    }

    /// The table of `unit`, a unit of this chart.
    pub fn table(&self, unit: &proto::Unit) -> &RecordBatch {
        &self.tables[unit.table as usize]
    }

    /// The values that the fields of the message refer to by index.
    pub fn values(&self) -> &[ArrayRef] {
        &self.values
    }

    /// The value at `index`, an array of one element, such as a constant or a partition value.
    pub fn value(&self, index: u32) -> &ArrayRef {
        &self.values[index as usize]
    }

    /// The elements of the list at `index`, such as the ends of an explicit domain or the values
    /// of a range.
    pub fn list(&self, index: u32) -> ArrayRef {
        self.value(index).as_list::<i32>().value(0)
    }

    /// The domain summary at `index`.
    pub fn summary(&self, index: u32) -> DomainSummary {
        DomainSummary::from_arrow(self.value(index).as_ref(), 0)
            .expect("the summaries of a valid chart decode")
    }

    /// The `DomainOptions` of avenger-scales for a domain fit of this chart. A preferred order of
    /// categories fails, because avenger-scales has no preferred order yet.
    pub fn fit(&self, fit: &proto::DomainFit) -> Result<DomainOptions, ChartError> {
        match fit.fit.as_ref().expect("a valid fit has a kind") {
            domain_fit::Fit::Interval(interval) => Ok(DomainOptions::Interval(IntervalOptions {
                zero: interval.zero,
                nice: interval.nice,
                empty: interval.empty.map(|empty| self.list(empty)),
            })),
            domain_fit::Fit::Categories(categories) => {
                let order = match categories.order.as_ref() {
                    Some(category_fit::Order::Sort(sort))
                        if *sort == proto::CategorySort::Descending as i32 =>
                    {
                        CategoryOrder::Descending
                    }
                    Some(category_fit::Order::Sort(_)) => CategoryOrder::Ascending,
                    Some(category_fit::Order::Preferred(_)) | None => {
                        return Err(ChartError::Payload(
                            "avenger-scales has no preferred order of categories yet".to_string(),
                        ))
                    }
                };
                Ok(DomainOptions::Categories(CategoryOptions { order }))
            }
        }
    }

    /// The initial contributions of the selections of the chart, by name.
    pub fn initial_contributions(&self) -> BTreeMap<String, Vec<Contribution>> {
        self.selections()
            .iter()
            .map(|selection| {
                let contributions = selection
                    .initial
                    .iter()
                    .map(|contribution| self.contribution(contribution))
                    .collect();
                (selection.name.clone(), contributions)
            })
            .collect()
    }

    /// The contribution of a message of the chart, with its values.
    fn contribution(&self, message: &proto::Contribution) -> Contribution {
        let channel = |channel: i32| Channel::try_from(channel).unwrap_or(Channel::Unspecified);
        let tuples = message
            .tuples
            .iter()
            .map(|tuple| {
                tuple
                    .fields
                    .iter()
                    .filter_map(|field| {
                        Some(match field.test.as_ref()? {
                            tuple_field::Test::Set(set) => TupleTest::Set {
                                column: set.column.clone(),
                                channel: channel(set.channel),
                                values: self.list(set.values),
                            },
                            tuple_field::Test::Range(range) => TupleTest::Range {
                                column: range.column.clone(),
                                channel: channel(range.channel),
                                ends: self.list(range.ends),
                                exclude_end: range.exclude_end,
                            },
                            tuple_field::Test::Polygon(polygon) => {
                                let vertices = self.list(polygon.vertices);
                                let points = vertices.as_struct();
                                TupleTest::Polygon {
                                    x_column: polygon.x_column.clone(),
                                    y_column: polygon.y_column.clone(),
                                    x: points.column(0).clone(),
                                    y: points.column(1).clone(),
                                }
                            }
                            tuple_field::Test::CyclicRange(range) => TupleTest::CyclicRange {
                                column: range.column.clone(),
                                channel: channel(range.channel),
                                start: range.start,
                                end: range.end,
                                period: range.period,
                            },
                        })
                    })
                    .collect()
            })
            .collect();
        Contribution {
            owner: message.owner.clone(),
            partition: message
                .partition
                .iter()
                .map(|value| {
                    (
                        value.column.clone(),
                        Scalar(self.value(value.value).clone()),
                    )
                })
                .collect(),
            tuples,
        }
    }

    /// The message, the tables, and the values, which a combination of charts takes apart.
    pub(crate) fn into_parts(self) -> (proto::Chart, Vec<RecordBatch>, Vec<ArrayRef>) {
        (self.message, self.tables, self.values)
    }
}

/// The values that the fields of `message` refer to, in the order that `Chart.values` describes,
/// with the fields renumbered: one value for each field, the values of each type together, the
/// types and the values of each type in the order of the first field that refers to them.
fn canonical(message: &mut proto::Chart, values: &[ArrayRef]) -> Vec<ArrayRef> {
    let mut references: Vec<usize> = Vec::new();
    tree::for_each_value_mut(message, &mut |index| references.push(*index as usize));
    let mut types: Vec<&DataType> = Vec::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (position, index) in references.iter().enumerate() {
        let data_type = values[*index].data_type();
        let group = match types.iter().position(|known| *known == data_type) {
            Some(group) => group,
            None => {
                types.push(data_type);
                groups.push(Vec::new());
                groups.len() - 1
            }
        };
        groups[group].push(position);
    }
    let mut renumbered = vec![0; references.len()];
    let mut ordered = Vec::with_capacity(references.len());
    for position in groups.into_iter().flatten() {
        renumbered[position] = ordered.len() as u32;
        ordered.push(values[references[position]].clone());
    }
    let mut position = 0;
    tree::for_each_value_mut(message, &mut |index| {
        *index = renumbered[position];
        position += 1;
    });
    ordered
}
