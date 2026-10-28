use arrow::record_batch::RecordBatch;

use crate::{proto, validate, ChartError};

/// A chart: its protobuf message, which is the model of the format, with the data tables that
/// its units refer to, decoded as Arrow record batches.
///
/// A chart is always valid: decoding and the builder check every rule of the format before they
/// return one. It holds no runtime state, so it renders the same after a round trip through
/// bytes. Cloning a chart copies its message and shares the buffers of its tables.
#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    /// The message, whose `tables` are empty: its units refer to `tables` by `Unit.table`.
    message: proto::Chart,
    tables: Vec<RecordBatch>,
}

impl Chart {
    /// A chart from its message and the tables that its units refer to by `Unit.table`. The
    /// message's own `tables` are empty, because the chart holds its tables decoded. It fails for a
    /// chart that breaks a rule of the format.
    pub fn new(message: proto::Chart, tables: Vec<RecordBatch>) -> Result<Self, ChartError> {
        let chart = Self { message, tables };
        validate::check(&chart)?;
        Ok(chart)
    }

    /// The message of the chart. Its `tables` are empty, because the chart holds them decoded.
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
}
