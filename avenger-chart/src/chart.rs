use arrow::{
    array::{ArrayRef, AsArray},
    datatypes::DataType,
    record_batch::RecordBatch,
};
use prost::bytes::Bytes;

use crate::{codec, proto, read, ChartError};

/// A chart: its protobuf message, in the canonical form of the format, with the data tables that
/// its layers refer to and the values that its fields refer to, decoded as Arrow arrays.
///
/// A chart is always valid and canonical: each constructor checks every rule of the format and
/// rewrites the message into the one form that the writer writes, so the ways of writing one chart
/// that the format's comments name, such as keyed lists in any order or partition values on any
/// node, make equal charts. A field set to the default that its comment gives when it is not set
/// stays as it is written. A chart holds no runtime state, so it renders the same after a round trip
/// through bytes. Cloning a chart copies its message and shares the buffers of its tables and
/// values.
///
/// A chart read from bytes keeps the IPC stream of each table, and its writer writes the stream
/// again instead of encoding the table, so a chart that combines charts copies their data.
#[derive(Debug, Clone)]
pub struct Chart {
    /// The message, whose `tables` are empty and whose `values` is not set: its layers refer to
    /// `tables` by `TableRef`, and its fields refer to `values` by `ValueRef`.
    pub(crate) message: proto::Chart,
    pub(crate) tables: Vec<RecordBatch>,
    /// For each table, the IPC stream that it was decoded from, which the writer writes again, or
    /// `None` for a table that has no stream.
    pub(crate) streams: Vec<Option<Bytes>>,
    /// One value for each field that refers to one, in the order that `Chart.values` describes.
    pub(crate) values: Vec<ArrayRef>,
}

/// Charts are equal when their messages, their tables, and their values are, whatever streams
/// their tables were decoded from.
impl PartialEq for Chart {
    fn eq(&self, other: &Self) -> bool {
        self.message == other.message && self.tables == other.tables && self.values == other.values
    }
}

impl Chart {
    /// A chart from a message whose tables and values are IPC streams inside it, as a producer
    /// that writes the message writes them. It fails for a chart that breaks a rule of the
    /// format.
    pub fn from_message(mut message: proto::Chart) -> Result<Self, ChartError> {
        let (tables, streams, values) = codec::decode_parts(&mut message)?;
        Self::read_parts(message, tables, streams, values)
    }

    /// A chart from its message, the tables that its layers refer to by `TableRef`, and the values
    /// that its fields refer to by `ValueRef`, each an array of one element, for a producer that
    /// holds its Arrow data already. The message's own `tables` are empty and its `values` is not
    /// set. A dictionary-encoded column becomes the values that its dictionary holds. It fails for
    /// a chart that breaks a rule of the format.
    pub fn from_parts(
        message: proto::Chart,
        tables: Vec<RecordBatch>,
        values: Vec<ArrayRef>,
    ) -> Result<Self, ChartError> {
        let streams = vec![None; tables.len()];
        Self::read_parts(message, tables, streams, values)
    }

    /// A chart from its parts and the IPC stream of each table, checked and in the canonical form.
    pub(crate) fn read_parts(
        message: proto::Chart,
        tables: Vec<RecordBatch>,
        streams: Vec<Option<Bytes>>,
        values: Vec<ArrayRef>,
    ) -> Result<Self, ChartError> {
        // The canonical form holds the values of dictionary-encoded columns, which the stream of
        // their table does not, so such a table keeps no stream.
        let (tables, streams) = (tables.into_iter().zip(streams))
            .map(|(table, stream)| match codec::has_dictionary(&table) {
                true => Ok((codec::plain(table)?, None)),
                false => Ok((table, stream)),
            })
            .collect::<Result<(Vec<_>, Vec<_>), ChartError>>()?;
        read::read(Self {
            message,
            tables,
            streams,
            values,
        })
    }

    /// The message of the chart, in the canonical form. Its `tables` are empty and its `values` is
    /// not set, because the chart holds them decoded.
    pub fn message(&self) -> &proto::Chart {
        &self.message
    }

    /// The root node of the chart.
    pub fn root(&self) -> &proto::Node {
        (self.message.root.as_ref()).expect("a valid chart has a root")
    }

    /// The selections of the chart, in the order of their names.
    pub fn selections(&self) -> &[proto::Selection] {
        &self.message.selections
    }

    /// The tables that the layers refer to by `TableRef`.
    pub fn tables(&self) -> &[RecordBatch] {
        &self.tables
    }

    /// The table that `table` refers to.
    pub fn table(&self, table: &proto::TableRef) -> &RecordBatch {
        &self.tables[table.index as usize]
    }

    /// The values that the fields of the message refer to by `ValueRef`.
    pub fn values(&self) -> &[ArrayRef] {
        &self.values
    }

    /// The value that `value` refers to, an array of one element, such as a constant or a
    /// partition value.
    pub fn value(&self, value: &proto::ValueRef) -> &ArrayRef {
        &self.values[value.index as usize]
    }

    /// The elements of the list that `value` refers to, such as the ends of an explicit domain.
    pub fn list_elements(&self, value: &proto::ValueRef) -> ArrayRef {
        match self.value(value).data_type() {
            DataType::List(_) => self.value(value).as_list::<i32>().value(0),
            other => panic!("a valid chart refers to a list here, not {other}"),
        }
    }

    /// The value of the partition value `value` of this chart.
    pub fn partition_value(&self, value: &proto::PartitionValue) -> &ArrayRef {
        self.value((value.value.as_ref()).expect("a valid partition value refers to its value"))
    }

    /// The models of the chart that a renderer reads: its tree of plots and layers, its scales, its
    /// projections, and its tables of tracks.
    pub fn resolve(&self) -> crate::resolve::Resolved<'_> {
        let tree = crate::resolve::Tree::new(self).expect("a valid chart has a valid tree");
        crate::resolve::resolve_tree(self, tree).expect("a valid chart resolves")
    }

    /// The message, the tables, and the values of the chart.
    pub fn into_parts(self) -> (proto::Chart, Vec<RecordBatch>, Vec<ArrayRef>) {
        (self.message, self.tables, self.values)
    }
}
