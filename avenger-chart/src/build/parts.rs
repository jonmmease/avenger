use arrow::array::{ArrayRef, RecordBatch};

use super::{infer_scales, Fragment};
use crate::{
    codec::list_value,
    proto::{self, encoding, node, plot},
    walk::VisitValues,
    Chart, ChartError, FORMAT_VERSION,
};

/// A chart under construction: the tables and the values that its message refers to, which hand
/// out the references to them.
///
/// It does not merge equal tables or values, because the canonical form of the reader does, so a
/// producer can add one table twice and still get canonical bytes.
#[derive(Clone, Debug, Default)]
pub struct ChartParts {
    tables: Vec<RecordBatch>,
    values: Vec<ArrayRef>,
}

impl ChartParts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a table and returns its reference.
    pub fn table(&mut self, batch: RecordBatch) -> proto::TableRef {
        self.tables.push(batch);
        proto::TableRef {
            index: (self.tables.len() - 1) as u32,
        }
    }

    /// Adds a value, an array of one element, and returns its reference.
    pub fn value(&mut self, value: ArrayRef) -> proto::ValueRef {
        self.values.push(value);
        proto::ValueRef {
            index: (self.values.len() - 1) as u32,
        }
    }

    /// Adds a list value of `elements`, such as an explicit domain, and returns its reference.
    pub fn list(&mut self, elements: ArrayRef) -> proto::ValueRef {
        self.value(list_value(elements))
    }

    /// Adds `value`, an array of one element, and returns a partition value of `column` that
    /// refers to it.
    pub fn partition(
        &mut self,
        column: impl Into<String>,
        value: ArrayRef,
    ) -> proto::PartitionValue {
        proto::PartitionValue {
            column: column.into(),
            value: Some(self.value(value)),
        }
    }

    /// Adds the values of `fragment` and returns its message, whose `ValueRef`s refer to them.
    pub fn embed<M: VisitValues>(&mut self, fragment: Fragment<M>) -> M {
        fragment.embed_in(&mut self.values)
    }

    /// The chart of `message` and these parts, with the scales that
    /// [`infer_scales`](super::infer_scales) infers, which the reader checks and rewrites into the
    /// canonical form. `message` holds the root and any other field of a chart, such as its format
    /// syntaxes and its selections. Inference never changes a field that is present, so a message
    /// whose scales are all explicit reads as it is.
    pub fn finish(self, mut message: proto::Chart) -> Result<Chart, ChartError> {
        infer_scales(&mut message, &self.tables, &self.values);
        Chart::from_parts(message, self.tables, self.values)
    }
}

/// An encoding of the column `name`, whose values the mark draws as they are.
pub fn column(name: impl Into<String>) -> proto::Encoding {
    proto::Encoding {
        value: Some(encoding::Value::Column(name.into())),
        ..Default::default()
    }
}

/// An encoding of the column `name` through the channel's scale, whose type and domain
/// [`infer_scales`](super::infer_scales) infers.
pub fn scaled_column(name: impl Into<String>) -> proto::Encoding {
    proto::Encoding {
        value: Some(encoding::Value::ScaledColumn(name.into())),
        ..Default::default()
    }
}

/// A Cartesian plot of `layers`.
pub fn cartesian(layers: Vec<proto::CartesianLayer>) -> proto::Node {
    plot_node(plot::Coord::Cartesian(proto::CartesianPlot {
        layers,
        ..Default::default()
    }))
}

/// A polar plot of `layers`.
pub fn polar(layers: Vec<proto::PolarLayer>) -> proto::Node {
    plot_node(plot::Coord::Polar(proto::PolarPlot {
        layers,
        ..Default::default()
    }))
}

/// A map plot of `layers`, with the options of its coordinate system, which a map plot always
/// holds.
pub fn geo(options: proto::Geo, layers: Vec<proto::GeoLayer>) -> proto::Node {
    plot_node(plot::Coord::Geo(proto::GeoPlot {
        options: Some(options),
        layers,
    }))
}

fn plot_node(coord: plot::Coord) -> proto::Node {
    proto::Node {
        node: Some(node::Node::Plot(proto::Plot {
            coord: Some(coord),
            ..Default::default()
        })),
    }
}

/// A chart message of this format version with `root`, which [`ChartParts::finish`] reads as a
/// chart.
pub fn message(root: proto::Node) -> proto::Chart {
    proto::Chart {
        version: FORMAT_VERSION,
        root: Some(root),
        ..Default::default()
    }
}
