//! The bytes of a chart: decoding in place, the writer that aligns each table and the values, and
//! columns of charts in Arrow arrays.

use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::{
        Array, ArrayRef, AsArray, BinaryViewArray, BinaryViewBuilder, ByteView, GenericBinaryArray,
        ListArray, OffsetSizeTrait,
    },
    buffer::OffsetBuffer,
    buffer::{Buffer, MutableBuffer},
    compute::{cast, concat},
    datatypes::{DataType, Field, Schema},
    error::ArrowError,
    ipc::{reader::StreamDecoder, writer::StreamWriter},
    record_batch::RecordBatch,
};
use prost::{
    bytes::{BufMut, Bytes},
    encoding::{encode_key, encode_varint, encoded_len_varint, key_len, message, uint32, WireType},
    Message,
};

use crate::{proto, Chart, ChartError};

/// The format version that this crate reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// The alignment of the IPC stream of each table and of the values from the start of a chart,
/// which the IPC writer also uses for the buffers inside a stream.
const ALIGNMENT: usize = 64;

/// The field numbers of `Chart` and `Table` that the writer encodes itself.
const CHART_VERSION: u32 = 1;
const CHART_TABLES: u32 = 2;
const CHART_ROOT: u32 = 3;
const CHART_SELECTIONS: u32 = 4;
const CHART_VALUES: u32 = 5;
const TABLE_PADDING: u32 = 1;
const TABLE_IPC: u32 = 2;

impl Chart {
    /// Decodes a chart, with the columns of its tables and its values in place in `bytes`.
    ///
    /// A `Vec<u8>` or a `Bytes` converts without a copy. When the bytes do not start at a multiple
    /// of 64 bytes, the chart is copied one time into an aligned buffer, and its tables and values
    /// decode in place there. It fails for bytes that are not a chart, for a format version other than
    /// [`FORMAT_VERSION`], and for a chart that breaks a rule of the format.
    pub fn from_bytes(bytes: impl Into<Bytes>) -> Result<Self, ChartError> {
        let bytes = aligned(bytes.into());
        let message = proto::Chart::decode(bytes)
            .map_err(|error| ChartError::NotAChart(error.to_string()))?;
        Chart::from_message(message)
    }

    /// Encodes the chart, whose message, tables, and values are in the canonical form, so equal
    /// charts encode to equal bytes, unless their Arrow data holds equal values in different
    /// buffers, such as different bytes under a null.
    ///
    /// A table that the chart read from an IPC stream is written as that stream, unchanged, so the
    /// bytes of a table that another IPC writer made, such as one with another alignment, stay
    /// that writer's bytes, and two equal charts whose tables came from such writers can encode to
    /// different bytes. Each other table is written with the default IPC writer options.
    ///
    /// The bytes are in a 64-byte-aligned allocation, and the IPC stream of each table and of the
    /// values starts at a multiple of 64 bytes from their start, after the padding of its `Table`
    /// message, so a reader decodes every table and every value in place. It fails only when Arrow
    /// cannot write a table or the values.
    pub fn to_bytes(&self) -> Result<Bytes, ChartError> {
        Ok(Bytes::from_owner(BufferOwner(self.to_buffer()?)))
    }

    /// The bytes of [`Chart::to_bytes`] in the Arrow buffer of their allocation.
    fn to_buffer(&self) -> Result<Buffer, ChartError> {
        let mut message = self.message().clone();
        // The tables are canonical: one copy of each, in the order of the first layer that refers
        // to each. A table that was read from a stream is written as that stream.
        let streams: Vec<Bytes> = (self.tables().iter().zip(&self.streams))
            .map(|(table, stream)| match stream {
                Some(stream) => Ok(stream.clone()),
                None => encode_ipc(table).map(Bytes::from),
            })
            .collect::<Result<_, ArrowError>>()?;
        // The fields after the values, which no padding depends on.
        let tail = proto::Chart {
            number_format_syntax: std::mem::take(&mut message.number_format_syntax),
            datetime_format_syntax: std::mem::take(&mut message.datetime_format_syntax),
            width: message.width.take(),
            height: message.height.take(),
            padding: message.padding.take(),
            modes: std::mem::take(&mut message.modes),
            ..Default::default()
        };

        // Lay out the tables and the values first, since the padding of each depends on the bytes
        // before it.
        let mut offset = uint32::encoded_len(CHART_VERSION, &message.version);
        let mut paddings = Vec::with_capacity(streams.len());
        for ipc in &streams {
            let padding = padding(CHART_TABLES, offset, ipc.len());
            offset += table_field_len(CHART_TABLES, padding, ipc.len());
            paddings.push(padding);
        }
        offset += message
            .root
            .as_ref()
            .map_or(0, |root| message::encoded_len(CHART_ROOT, root))
            + message::encoded_len_repeated(CHART_SELECTIONS, &message.selections);
        let values = encode_values(self.values())?
            .map(|ipc| (padding(CHART_VALUES, offset, ipc.len()), ipc));
        let len = offset
            + values.as_ref().map_or(0, |(padding, ipc)| {
                table_field_len(CHART_VALUES, *padding, ipc.len())
            })
            + tail.encoded_len();

        let mut buffer = MutableBuffer::from_len_zeroed(len);
        {
            let mut out: &mut [u8] = buffer.as_slice_mut();
            uint32::encode(CHART_VERSION, &message.version, &mut out);
            for (ipc, padding) in streams.iter().zip(paddings) {
                encode_table(CHART_TABLES, padding, ipc, &mut out);
            }
            if let Some(root) = &message.root {
                message::encode(CHART_ROOT, root, &mut out);
            }
            message::encode_repeated(CHART_SELECTIONS, &message.selections, &mut out);
            if let Some((padding, ipc)) = &values {
                encode_table(CHART_VALUES, *padding, ipc, &mut out);
            }
            tail.encode_raw(&mut out);
            debug_assert!(out.is_empty(), "the layout covers every byte");
        }
        Ok(buffer.into())
    }
}

/// The extension name of the field of a column of charts.
pub const EXTENSION_NAME: &str = "avenger.chart";

/// The size from which a chart in a column takes a data buffer of its own.
const OWN_BUFFER: usize = 64 * 1024;

/// The most bytes of the data buffers that smaller charts share.
const SHARED_BUFFER: usize = 1024 * 1024;

/// The field of a column of charts: `BinaryView` values with the extension name.
pub fn chart_field(name: impl Into<String>, nullable: bool) -> Field {
    Field::new(name, DataType::BinaryView, nullable).with_metadata(HashMap::from([(
        "ARROW:extension:name".to_string(),
        EXTENSION_NAME.to_string(),
    )]))
}

/// The charts as a `BinaryView` array, the values of a field from [`chart_field`], with a null
/// row for each `None`.
///
/// Each chart starts at a multiple of 64 bytes from the start of its data buffer, so it decodes
/// in place. A chart of 64 KiB or more takes a data buffer of its own, the allocation of its
/// bytes, so a chart that decodes from the column keeps only its own bytes alive. Smaller charts
/// share data buffers of up to 1 MiB.
pub fn charts_to_arrow<'a>(
    charts: impl IntoIterator<Item = Option<&'a Chart>>,
) -> Result<BinaryViewArray, ChartError> {
    // Lay out the data buffers first, because a view names a buffer that the builder has.
    let mut blocks: Vec<Block> = Vec::new();
    let mut rows: Vec<Option<(usize, usize, usize)>> = Vec::new();
    let mut shared: Option<usize> = None;
    for chart in charts {
        let Some(chart) = chart else {
            rows.push(None);
            continue;
        };
        let bytes = chart.to_buffer()?;
        let len = bytes.len();
        if len >= OWN_BUFFER {
            rows.push(Some((blocks.len(), 0, len)));
            blocks.push(Block::Own(bytes));
            continue;
        }
        let index = match shared {
            Some(index)
                if matches!(&blocks[index], Block::Shared(buffer)
                if buffer.len().next_multiple_of(ALIGNMENT) + len <= SHARED_BUFFER) =>
            {
                index
            }
            _ => {
                blocks.push(Block::Shared(MutableBuffer::new(0)));
                *shared.insert(blocks.len() - 1)
            }
        };
        let Block::Shared(buffer) = &mut blocks[index] else {
            unreachable!("the shared block is shared");
        };
        buffer.resize(buffer.len().next_multiple_of(ALIGNMENT), 0);
        rows.push(Some((index, buffer.len(), len)));
        buffer.extend_from_slice(&bytes);
    }

    let mut builder = BinaryViewBuilder::new();
    for block in blocks {
        builder.append_block(match block {
            Block::Own(buffer) => buffer,
            Block::Shared(buffer) => buffer.into(),
        });
    }
    for row in rows {
        match row {
            Some((block, offset, len)) => {
                builder.try_append_view(block as u32, offset as u32, len as u32)?
            }
            None => builder.append_null(),
        }
    }
    Ok(builder.finish())
}

/// A data buffer of a column of charts.
enum Block {
    /// The bytes of one chart, in their own allocation.
    Own(Buffer),
    /// Charts at multiples of 64 bytes.
    Shared(MutableBuffer),
}

/// How [`charts_from_arrow`] decodes the rows of a column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Decode {
    /// In place when the bytes of a row are at least half of the allocation behind its buffer,
    /// and from a copy of the bytes otherwise, and when Arrow does not know the size of the
    /// allocation, as for a buffer from FFI. A chart then never keeps much more than its own
    /// bytes alive.
    #[default]
    Auto,
    /// In place, so each chart keeps the allocation behind its row alive.
    InPlace,
    /// From a copy of the bytes of each row.
    Copy,
}

/// The charts of a `Binary`, `LargeBinary`, or `BinaryView` column, with `None` for each null
/// row, decoded as `decode` says. A column whose field lost the extension name decodes too, and
/// a value that is not a chart fails, as for [`Chart::from_bytes`].
pub fn charts_from_arrow(
    array: &dyn Array,
    decode: Decode,
) -> Result<Vec<Option<Chart>>, ChartError> {
    let chart = |buffer: &Buffer, start: usize, len: usize| {
        let in_place = match decode {
            Decode::Auto => buffer.capacity() > 0 && 2 * len >= buffer.capacity(),
            Decode::InPlace => true,
            Decode::Copy => false,
        };
        let bytes = if in_place {
            Bytes::from_owner(BufferOwner(buffer.slice_with_length(start, len)))
        } else {
            aligned_copy(&buffer.as_slice()[start..start + len])
        };
        Chart::from_bytes(bytes)
    };
    match array.data_type() {
        DataType::Binary => binary_rows(array.as_binary::<i32>(), chart),
        DataType::LargeBinary => binary_rows(array.as_binary::<i64>(), chart),
        DataType::BinaryView => {
            let array = array.as_binary_view();
            (0..array.len())
                .map(|row| {
                    if array.is_null(row) {
                        return Ok(None);
                    }
                    let view = ByteView::from(array.views()[row]);
                    if view.length <= 12 {
                        // An inline value is too short to be a chart, and fails to decode.
                        return Chart::from_bytes(Bytes::copy_from_slice(array.value(row)))
                            .map(Some);
                    }
                    let buffer = &array.data_buffers()[view.buffer_index as usize];
                    chart(buffer, view.offset as usize, view.length as usize).map(Some)
                })
                .collect()
        }
        other => Err(ChartError::NotAChart(format!(
            "a column of charts is Binary, LargeBinary, or BinaryView, not {other}"
        ))),
    }
}

fn binary_rows<O: OffsetSizeTrait>(
    array: &GenericBinaryArray<O>,
    chart: impl Fn(&Buffer, usize, usize) -> Result<Chart, ChartError>,
) -> Result<Vec<Option<Chart>>, ChartError> {
    let offsets = array.value_offsets();
    (0..array.len())
        .map(|row| {
            if array.is_null(row) {
                return Ok(None);
            }
            let start = offsets[row].as_usize();
            let len = offsets[row + 1].as_usize() - start;
            chart(array.values(), start, len).map(Some)
        })
        .collect()
}

/// The zeros of the padding field of a table in field `field` of `Chart` that starts `offset`
/// bytes from the start of the chart, with an IPC stream of `ipc_len` bytes, or `None` when its
/// stream starts at a multiple of 64 bytes without one. The field has a tag and a length besides its zeros, so it takes 2 bytes or
/// more, and it is the smallest that aligns the stream. A field that lengthens the table's length
/// prefix moves the stream by one more byte, which the search takes into account.
fn padding(field: u32, offset: usize, ipc_len: usize) -> Option<usize> {
    let ipc_start =
        |padding: Option<usize>| offset + table_field_len(field, padding, ipc_len) - ipc_len;
    if ipc_start(None).is_multiple_of(ALIGNMENT) {
        return None;
    }
    // Field lengths of 2 and more cover every remainder within two runs of 64, also when the
    // length prefix grows by a byte among them.
    (0..2 * ALIGNMENT)
        .find(|&zeros| ipc_start(Some(zeros)).is_multiple_of(ALIGNMENT))
        .map(Some)
        .expect("a padding of fewer than 128 zeros aligns every stream")
}

/// The length of a `Table` message with `padding` zeros and an IPC stream of `ipc_len` bytes.
fn table_message_len(padding: Option<usize>, ipc_len: usize) -> usize {
    let padding = padding.map_or(0, |zeros| {
        key_len(TABLE_PADDING) + encoded_len_varint(zeros as u64) + zeros
    });
    padding + key_len(TABLE_IPC) + encoded_len_varint(ipc_len as u64) + ipc_len
}

/// The length of a `Table` field of `Chart`: its tag, its length, and its `Table` message.
fn table_field_len(field: u32, padding: Option<usize>, ipc_len: usize) -> usize {
    let message = table_message_len(padding, ipc_len);
    key_len(field) + encoded_len_varint(message as u64) + message
}

/// Writes a `Table` field of `Chart`, with `padding` zeros before its IPC stream.
fn encode_table(field: u32, padding: Option<usize>, ipc: &[u8], out: &mut &mut [u8]) {
    encode_key(field, WireType::LengthDelimited, out);
    encode_varint(table_message_len(padding, ipc.len()) as u64, out);
    if let Some(zeros) = padding {
        encode_key(TABLE_PADDING, WireType::LengthDelimited, out);
        encode_varint(zeros as u64, out);
        out.put_bytes(0, zeros);
    }
    encode_key(TABLE_IPC, WireType::LengthDelimited, out);
    encode_varint(ipc.len() as u64, out);
    out.put_slice(ipc);
}

/// The IPC stream of `Chart.values` for `values` in their order, or `None` for no values: one row
/// with a column for each run of values of one type, which lists them.
fn encode_values(values: &[ArrayRef]) -> Result<Option<Vec<u8>>, ArrowError> {
    if values.is_empty() {
        return Ok(None);
    }
    let mut fields = Vec::new();
    let mut columns = Vec::new();
    for run in values.chunk_by(|a, b| a.data_type() == b.data_type()) {
        let elements: Vec<&dyn Array> = run.iter().map(|value| value.as_ref()).collect();
        let column = list_value(concat(&elements)?);
        fields.push(Field::new(
            fields.len().to_string(),
            column.data_type().clone(),
            false,
        ));
        columns.push(column);
    }
    let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)?;
    encode_ipc(&batch).map(Some)
}

/// The values of the IPC stream of `Chart.values`: the elements of the list in each column, in
/// the order of the columns, each an array of one element in place in `ipc`. A column is a
/// `List` of values of one type.
/// The IPC stream of `values`, each an array of one element, in the layout of `Chart.values`, or
/// an empty stream for no values, as the byte form of a fragment of a message with its values.
pub fn values_to_ipc(values: &[ArrayRef]) -> Result<Bytes, ChartError> {
    Ok(encode_values(values)?.map(Bytes::from).unwrap_or_default())
}

/// The values of an IPC stream that [`values_to_ipc`] wrote, each an array of one element, in
/// order. An empty stream holds no values.
pub fn values_from_ipc(ipc: &Bytes) -> Result<Vec<ArrayRef>, ChartError> {
    if ipc.is_empty() {
        return Ok(Vec::new());
    }
    decode_values(ipc).map_err(|reason| ChartError::invalid("values", reason))
}

fn decode_values(ipc: &Bytes) -> Result<Vec<ArrayRef>, String> {
    let batch = decode_ipc(ipc).map_err(|error| error.to_string())?;
    if batch.num_rows() != 1 {
        return Err(format!("the values are one row, not {}", batch.num_rows()));
    }
    let mut values = Vec::new();
    for column in batch.columns() {
        let list = column.as_list_opt::<i32>().ok_or_else(|| {
            format!(
                "each column of the values is a list, not {}",
                column.data_type()
            )
        })?;
        let elements = list.value(0);
        values.extend((0..elements.len()).map(|index| elements.slice(index, 1)));
    }
    Ok(values)
}

/// `bytes`, or a copy of them in a 64-byte-aligned allocation when they start elsewhere.
fn aligned(bytes: Bytes) -> Bytes {
    if bytes.as_ptr().align_offset(ALIGNMENT) == 0 {
        return bytes;
    }
    aligned_copy(&bytes)
}

/// A copy of `bytes` in a 64-byte-aligned allocation.
fn aligned_copy(bytes: &[u8]) -> Bytes {
    let mut buffer = MutableBuffer::new(bytes.len());
    buffer.extend_from_slice(bytes);
    Bytes::from_owner(BufferOwner(buffer.into()))
}

/// An Arrow buffer as the owner of a `Bytes`, so that bytes and the arrays that decode from them
/// share one allocation.
pub(crate) struct BufferOwner(pub(crate) Buffer);

impl AsRef<[u8]> for BufferOwner {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

/// The decoded tables of a chart, the IPC stream of each table, and the decoded values.
type DecodedParts = (Vec<RecordBatch>, Vec<Option<Bytes>>, Vec<ArrayRef>);

/// The tables and the values of `message`, decoded from the IPC streams of its `tables` and its
/// `values`, which it leaves empty and not set, with the stream of each table.
pub(crate) fn decode_parts(message: &mut proto::Chart) -> Result<DecodedParts, ChartError> {
    let (tables, streams) = std::mem::take(&mut message.tables)
        .into_iter()
        .enumerate()
        .map(|(index, table)| {
            let batch = decode_ipc(&table.ipc).map_err(|error| {
                ChartError::invalid(format!("tables[{index}]"), error.to_string())
            })?;
            Ok((batch, Some(table.ipc)))
        })
        .collect::<Result<(Vec<_>, Vec<_>), ChartError>>()?;
    let values = match message.values.take() {
        Some(values) => {
            decode_values(&values.ipc).map_err(|reason| ChartError::invalid("values", reason))?
        }
        None => Vec::new(),
    };
    Ok((tables, streams, values))
}

/// `values` as one list, the form of a list value, such as an explicit domain.
pub fn list_value(values: ArrayRef) -> ArrayRef {
    let field = Arc::new(Field::new_list_field(values.data_type().clone(), true));
    let offsets = OffsetBuffer::from_lengths([values.len()]);
    Arc::new(ListArray::new(field, offsets, values, None))
}

/// The record batch of an IPC stream of one record batch, whose aligned buffers reference `bytes`.
/// A stream with compressed buffers fails.
pub(crate) fn decode_ipc(bytes: &Bytes) -> Result<RecordBatch, ArrowError> {
    uncompressed(bytes)?;
    decode_stream(bytes)
}

/// Whether a column of `batch` is dictionary-encoded.
pub(crate) fn has_dictionary(batch: &RecordBatch) -> bool {
    (batch.columns().iter()).any(|column| matches!(column.data_type(), DataType::Dictionary(..)))
}

/// `batch` with each dictionary-encoded column as the values that its dictionary holds.
pub(crate) fn plain(batch: RecordBatch) -> Result<RecordBatch, ArrowError> {
    if !has_dictionary(&batch) {
        return Ok(batch);
    }
    let mut fields = Vec::with_capacity(batch.num_columns());
    let mut columns = Vec::with_capacity(batch.num_columns());
    for (field, column) in batch.schema().fields().iter().zip(batch.columns()) {
        match column.data_type() {
            DataType::Dictionary(_, value) => {
                columns.push(cast(column, value)?);
                fields.push(
                    field
                        .as_ref()
                        .clone()
                        .with_data_type(value.as_ref().clone()),
                );
            }
            _ => {
                columns.push(column.clone());
                fields.push(field.as_ref().clone());
            }
        }
    }
    let schema = Schema::new_with_metadata(fields, batch.schema().metadata().clone());
    RecordBatch::try_new(Arc::new(schema), columns)
}

/// Fails for an IPC stream whose record batches or dictionary batches compress their buffers,
/// which the format does not allow.
fn uncompressed(bytes: &[u8]) -> Result<(), ArrowError> {
    let invalid = |reason: &str| ArrowError::IpcError(reason.to_string());
    let mut position = 0;
    while position + 4 <= bytes.len() {
        let word = |at: usize| -> Result<i32, ArrowError> {
            let word = bytes
                .get(at..at + 4)
                .ok_or_else(|| invalid("an IPC message is cut short"))?;
            Ok(i32::from_le_bytes(word.try_into().expect("four bytes")))
        };
        // A message is a continuation marker, its length, its metadata, and its body. Streams
        // from old writers leave out the marker.
        let (mut len, mut start) = (word(position)?, position + 4);
        if len == -1 {
            (len, start) = (word(position + 4)?, position + 8);
        }
        if len <= 0 {
            break;
        }
        let end = start + len as usize;
        let metadata = bytes
            .get(start..end)
            .ok_or_else(|| invalid("an IPC message is cut short"))?;
        let message = arrow::ipc::root_as_message(metadata)
            .map_err(|error| ArrowError::IpcError(error.to_string()))?;
        let compressed = message
            .header_as_record_batch()
            .and_then(|batch| batch.compression());
        let dictionary = (message.header_as_dictionary_batch())
            .and_then(|batch| batch.data())
            .and_then(|batch| batch.compression());
        if compressed.is_some() || dictionary.is_some() {
            return Err(invalid(
                "a chart's IPC stream does not compress its buffers",
            ));
        }
        position = end + message.bodyLength() as usize;
    }
    Ok(())
}

/// The record batch of an IPC stream of one record batch.
fn decode_stream(bytes: &Bytes) -> Result<RecordBatch, ArrowError> {
    let mut buffer = Buffer::from(bytes.clone());
    let mut decoder = StreamDecoder::new();
    let mut batches = Vec::new();
    while !buffer.is_empty() {
        let before = buffer.len();
        if let Some(batch) = decoder.decode(&mut buffer)? {
            batches.push(batch);
        } else if buffer.len() == before {
            break;
        }
    }
    decoder.finish()?;
    match <[RecordBatch; 1]>::try_from(batches) {
        Ok([batch]) => Ok(batch),
        Err(batches) => Err(ArrowError::IpcError(format!(
            "an IPC stream of a chart holds one record batch, not {}",
            batches.len()
        ))),
    }
}

/// The IPC stream of `batch`, with the default writer options, which place each buffer at a
/// multiple of 64 bytes from the start of the stream.
pub(crate) fn encode_ipc(batch: &RecordBatch) -> Result<Vec<u8>, ArrowError> {
    let mut writer = StreamWriter::try_new(Vec::new(), &batch.schema())?;
    writer.write(batch)?;
    writer.finish()?;
    writer.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every offset and every length near the boundaries of the length prefix gets a padding that
    /// aligns the stream, and the smallest one.
    #[test]
    fn padding_aligns_every_stream() {
        for ipc_len in [0, 1, 50, 120, 127, 128, 200, 16_300, 16_383, 16_384, 70_000] {
            for offset in 0..3 * ALIGNMENT {
                let padding = padding(CHART_TABLES, offset, ipc_len);
                let start = offset + table_field_len(CHART_TABLES, padding, ipc_len) - ipc_len;
                assert!(
                    start.is_multiple_of(ALIGNMENT),
                    "offset {offset} length {ipc_len}"
                );
                if let Some(zeros) = padding {
                    assert!(zeros < 2 * ALIGNMENT, "offset {offset} length {ipc_len}");
                    let smaller = (0..zeros).any(|fewer| {
                        (offset + table_field_len(CHART_TABLES, Some(fewer), ipc_len) - ipc_len)
                            .is_multiple_of(ALIGNMENT)
                    });
                    assert!(!smaller, "offset {offset} length {ipc_len}");
                }
            }
        }
    }
}
