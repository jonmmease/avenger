//! The bytes of a chart: decoding in place, and the writer that aligns each table.

use std::collections::HashMap;

use arrow::{
    buffer::{Buffer, MutableBuffer},
    error::ArrowError,
    ipc::{reader::StreamDecoder, writer::StreamWriter},
    record_batch::RecordBatch,
};
use bytes::{BufMut, Bytes};
use prost::{
    encoding::{encode_key, encode_varint, encoded_len_varint, key_len, message, uint32, WireType},
    Message,
};

use crate::{proto, tree, Chart, ChartError};

/// The format version that this crate reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// The alignment of each table's IPC stream from the start of a chart, which the IPC writer also
/// uses for the buffers inside a stream.
const ALIGNMENT: usize = 64;

/// The field numbers of `Chart` and `Table` that the writer encodes itself.
const CHART_VERSION: u32 = 1;
const CHART_TABLES: u32 = 2;
const CHART_ROOT: u32 = 3;
const CHART_SELECTIONS: u32 = 4;
const TABLE_PADDING: u32 = 1;
const TABLE_IPC: u32 = 2;

impl Chart {
    /// Decodes a chart, with the columns of its tables in place in `bytes`.
    ///
    /// A `Vec<u8>` or a `Bytes` converts without a copy. When the bytes do not start at a multiple
    /// of 64 bytes, the chart is copied one time into an aligned buffer, and its tables decode in
    /// place there. It fails for bytes that are not a chart, for a format version other than
    /// [`FORMAT_VERSION`], and for a chart that breaks a rule of the format.
    pub fn from_bytes(bytes: impl Into<Bytes>) -> Result<Self, ChartError> {
        let bytes = aligned(bytes.into());
        let mut message = proto::Chart::decode(bytes)
            .map_err(|error| ChartError::NotAChart(error.to_string()))?;
        match message.version {
            FORMAT_VERSION => {}
            0 => {
                return Err(ChartError::NotAChart(
                    "its format version is 0, which no chart has".to_string(),
                ))
            }
            version => return Err(ChartError::UnsupportedVersion(version)),
        }
        let tables = std::mem::take(&mut message.tables)
            .iter()
            .enumerate()
            .map(|(index, table)| {
                decode_ipc(&table.ipc).map_err(|error| {
                    ChartError::invalid(format!("tables[{index}]"), error.to_string())
                })
            })
            .collect::<Result<_, _>>()?;
        Chart::new(message, tables)
    }

    /// Encodes the chart, with each distinct table one time, by its IPC bytes, in the order of
    /// the units that refer to it, so equal charts encode to equal bytes.
    ///
    /// The bytes are in a 64-byte-aligned allocation, and each table's IPC stream starts at a
    /// multiple of 64 bytes from their start, after the padding of its `Table` message, so a
    /// reader decodes every table in place. It fails only when Arrow cannot write a table.
    pub fn to_bytes(&self) -> Result<Bytes, ChartError> {
        let mut message = self.message().clone();
        let mut streams: Vec<Vec<u8>> = Vec::new();
        let mut by_bytes: HashMap<Vec<u8>, u32> = HashMap::new();
        let mut by_table: HashMap<u32, u32> = HashMap::new();
        let mut failure = None;
        if let Some(root) = &mut message.root {
            tree::for_each_unit_mut(root, &mut |unit| {
                if let Some(&index) = by_table.get(&unit.table) {
                    unit.table = index;
                    return;
                }
                let ipc = match encode_ipc(&self.tables()[unit.table as usize]) {
                    Ok(ipc) => ipc,
                    Err(error) => {
                        failure.get_or_insert(error);
                        return;
                    }
                };
                let index = *by_bytes.entry(ipc.clone()).or_insert_with(|| {
                    streams.push(ipc);
                    (streams.len() - 1) as u32
                });
                by_table.insert(unit.table, index);
                unit.table = index;
            });
        }
        if let Some(error) = failure {
            return Err(error.into());
        }

        // Lay out the tables first, since each one's padding depends on the bytes before it.
        let mut offset = uint32::encoded_len(CHART_VERSION, &message.version);
        let mut paddings = Vec::with_capacity(streams.len());
        for ipc in &streams {
            let padding = padding(offset, ipc.len());
            offset += table_field_len(padding, ipc.len());
            paddings.push(padding);
        }
        let len = offset
            + message
                .root
                .as_ref()
                .map_or(0, |root| message::encoded_len(CHART_ROOT, root))
            + message::encoded_len_repeated(CHART_SELECTIONS, &message.selections);

        let mut buffer = MutableBuffer::from_len_zeroed(len);
        {
            let mut out: &mut [u8] = buffer.as_slice_mut();
            uint32::encode(CHART_VERSION, &message.version, &mut out);
            for (ipc, padding) in streams.iter().zip(paddings) {
                encode_key(CHART_TABLES, WireType::LengthDelimited, &mut out);
                encode_varint(table_message_len(padding, ipc.len()) as u64, &mut out);
                if let Some(zeros) = padding {
                    encode_key(TABLE_PADDING, WireType::LengthDelimited, &mut out);
                    encode_varint(zeros as u64, &mut out);
                    out.put_bytes(0, zeros);
                }
                encode_key(TABLE_IPC, WireType::LengthDelimited, &mut out);
                encode_varint(ipc.len() as u64, &mut out);
                out.put_slice(ipc);
            }
            if let Some(root) = &message.root {
                message::encode(CHART_ROOT, root, &mut out);
            }
            message::encode_repeated(CHART_SELECTIONS, &message.selections, &mut out);
            debug_assert!(out.is_empty(), "the layout covers every byte");
        }
        Ok(Bytes::from_owner(BufferOwner(buffer.into())))
    }
}

/// The zeros of the padding field of a table that starts `offset` bytes from the start of the
/// chart, with an IPC stream of `ipc_len` bytes, or `None` when its stream starts at a multiple of
/// 64 bytes without one. The field has a tag and a length besides its zeros, so it takes 2 bytes or
/// more, and it is the smallest that aligns the stream. A field that lengthens the table's length
/// prefix moves the stream by one more byte, which the search takes into account.
fn padding(offset: usize, ipc_len: usize) -> Option<usize> {
    let ipc_start = |padding: Option<usize>| offset + table_field_len(padding, ipc_len) - ipc_len;
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

/// The length of a `Chart.tables` field: its tag, its length, and its `Table` message.
fn table_field_len(padding: Option<usize>, ipc_len: usize) -> usize {
    let message = table_message_len(padding, ipc_len);
    key_len(CHART_TABLES) + encoded_len_varint(message as u64) + message
}

/// `bytes`, or a copy of them in a 64-byte-aligned allocation when they start elsewhere.
fn aligned(bytes: Bytes) -> Bytes {
    if bytes.as_ptr().align_offset(ALIGNMENT) == 0 {
        return bytes;
    }
    let mut buffer = MutableBuffer::new(bytes.len());
    buffer.extend_from_slice(&bytes);
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

/// The record batch of an IPC stream of one record batch, whose aligned buffers reference `bytes`.
pub(crate) fn decode_ipc(bytes: &Bytes) -> Result<RecordBatch, ArrowError> {
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
                let padding = padding(offset, ipc_len);
                let start = offset + table_field_len(padding, ipc_len) - ipc_len;
                assert!(
                    start.is_multiple_of(ALIGNMENT),
                    "offset {offset} length {ipc_len}"
                );
                if let Some(zeros) = padding {
                    assert!(zeros < 2 * ALIGNMENT, "offset {offset} length {ipc_len}");
                    let smaller = (0..zeros).any(|fewer| {
                        (offset + table_field_len(Some(fewer), ipc_len) - ipc_len)
                            .is_multiple_of(ALIGNMENT)
                    });
                    assert!(!smaller, "offset {offset} length {ipc_len}");
                }
            }
        }
    }
}
