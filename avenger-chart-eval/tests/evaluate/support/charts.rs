//! Charts built from the messages of the format: the parts of a chart and the encodings of columns,
//! constants, and scales.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_chart::{
    build::{self, ChartParts},
    proto::{self, cartesian_layer, domain, domain_fit, encoding},
    Chart,
};
use avenger_scales::scales::{domain_summary::Extents, linear::LinearScale};

use super::*;

/// The parts of a chart, with helpers for the encodings of constants.
pub(crate) struct Parts(pub(crate) ChartParts);

impl Parts {
    pub(crate) fn new() -> Self {
        Self(ChartParts::new())
    }

    pub(crate) fn table(&mut self, batch: RecordBatch) -> proto::TableRef {
        self.0.table(batch)
    }

    pub(crate) fn list(&mut self, elements: ArrayRef) -> proto::ValueRef {
        self.0.list(elements)
    }

    pub(crate) fn value(&mut self, value: ArrayRef) -> proto::ValueRef {
        self.0.value(value)
    }

    /// A constant that the mark draws as it is.
    pub(crate) fn constant(&mut self, value: ArrayRef) -> proto::Encoding {
        proto::Encoding {
            value: Some(encoding::Value::Constant(self.value(value))),
            ..Default::default()
        }
    }

    pub(crate) fn number(&mut self, value: f64) -> proto::Encoding {
        self.constant(numbers(&[value]))
    }

    pub(crate) fn text(&mut self, value: &str) -> proto::Encoding {
        self.constant(Arc::new(StringArray::from(vec![value])))
    }

    /// A constant through the channel's scale.
    pub(crate) fn scaled_number(&mut self, value: f64) -> proto::Encoding {
        proto::Encoding {
            value: Some(encoding::Value::ScaledConstant(
                self.value(numbers(&[value])),
            )),
            ..Default::default()
        }
    }

    /// A summary domain of the values `values`, without the reach of their marks.
    pub(crate) fn summary(&mut self, values: &[f64]) -> proto::Domain {
        let summary = LinearScale::configured((0.0, 1.0), (0.0, 1.0))
            .summarize(&numbers(values), Extents::None)
            .unwrap();
        proto::Domain {
            domain: Some(domain::Domain::Summary(proto::SummaryDomain {
                value: Some(self.value(Arc::new(summary.to_arrow()))),
                fit: None,
            })),
        }
    }

    /// The chart of one Cartesian plot of `layers`.
    pub(crate) fn plot(self, layers: Vec<proto::CartesianLayer>) -> Chart {
        self.chart(build::cartesian(layers), Vec::new())
    }

    pub(crate) fn chart(self, root: proto::Node, selections: Vec<proto::Selection>) -> Chart {
        let mut message = build::message(root);
        message.selections = selections;
        message.number_format_syntax = "d3".to_string();
        message.datetime_format_syntax = "d3".to_string();
        self.0.finish(message).unwrap()
    }
}

pub(crate) fn numbers(values: &[f64]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

/// An encoding of the column `name` through a scale that the chart infers.
pub(crate) fn col(name: &str) -> proto::Encoding {
    build::scaled_column(name)
}

/// An encoding of the column `name` without a scale.
pub(crate) fn raw(name: &str) -> proto::Encoding {
    build::column(name)
}

/// An encoding of the column `name` through `scale`, whose absent fields the chart infers.
pub(crate) fn scaled(name: &str, scale: proto::ScaleSpec) -> proto::Encoding {
    proto::Encoding {
        scale: Some(scale),
        ..col(name)
    }
}

/// A data domain that fits with `fit`.
pub(crate) fn fitted(fit: proto::IntervalFit) -> proto::ScaleSpec {
    proto::ScaleSpec {
        domain: Some(proto::Domain {
            domain: Some(domain::Domain::Data(proto::DataDomain {
                fit: Some(proto::DomainFit {
                    fit: Some(domain_fit::Fit::Interval(fit)),
                }),
            })),
        }),
        ..Default::default()
    }
}

/// An encoding of the column `name` through a scale whose domain contains zero, as the lengths of
/// bars and areas take.
pub(crate) fn zeroed(name: &str) -> proto::Encoding {
    scaled(
        name,
        fitted(proto::IntervalFit {
            zero: Some(true),
            ..Default::default()
        }),
    )
}

pub(crate) fn explicit(domain: proto::ValueRef) -> proto::Domain {
    proto::Domain {
        domain: Some(domain::Domain::Explicit(domain)),
    }
}

pub(crate) fn layer(table: proto::TableRef, mark: cartesian_layer::Mark) -> proto::CartesianLayer {
    proto::CartesianLayer {
        table: Some(table),
        mark: Some(mark),
        ..Default::default()
    }
}

/// The chart of one layer of `mark` over the table `batch`.
pub(crate) fn one(
    batch: RecordBatch,
    mark: impl FnOnce(&mut Parts) -> cartesian_layer::Mark,
) -> Chart {
    let mut parts = Parts::new();
    let table = parts.table(batch);
    let mark = mark(&mut parts);
    parts.plot(vec![layer(table, mark)])
}

/// Symbols of the cars' fuel economies over their weights.
pub(crate) fn cars_symbols(extra: impl FnOnce(&mut Parts, &mut proto::SymbolMark)) -> Chart {
    one(cars(), |parts| {
        let mut mark = proto::SymbolMark {
            x: Some(col("weight")),
            y: Some(col("mpg")),
            ..Default::default()
        };
        extra(parts, &mut mark);
        cartesian_layer::Mark::Symbol(mark)
    })
}
