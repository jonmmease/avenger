//! The index of a set's tuples, by factorization: each column's distinct values get positions,
//! and each tuple is the packed positions of its values, so matching a row looks up each of its
//! values once. Integer, temporal, and boolean columns use a table over their range or a hash
//! table, string and binary columns a hash table of their bytes, and a dictionary column maps its
//! dictionary's values once for each array. A set of other types has no index and matches through
//! Arrow's row format.

use std::collections::{HashMap, HashSet};

use arrow::{
    array::{
        Array, ArrayRef, AsArray, BinaryArray, BinaryViewArray, BooleanArray, LargeBinaryArray,
        LargeStringArray, StringArray, StringViewArray,
    },
    buffer::{BooleanBuffer, ScalarBuffer},
    datatypes::{
        DataType, Date32Type, Int16Type, Int32Type, Int64Type, Int8Type, Time32MillisecondType,
        Time32SecondType, TimeUnit, UInt16Type, UInt32Type, UInt64Type, UInt8Type,
    },
};
use foldhash::fast::FixedState;

/// The position of a value that is not among the set's values.
const NONE: u32 = u32::MAX;

/// The most entries of a table over a range of integers.
const DENSE_ENTRIES: u64 = 1 << 20;

/// A set's tuples, indexed for the value types of its columns.
pub(crate) struct SetIndex {
    columns: Vec<ColumnIndex>,
    /// The tuples as packed positions, or `None` for a set of one column, whose values are its
    /// tuples.
    tuples: Option<HashSet<u64, FixedState>>,
    /// The number of distinct values of each column, the radices of the packed positions.
    radices: Vec<u64>,
}

impl SetIndex {
    /// The index of the tuples of `values`, one list for each column, already in the value types
    /// of the columns, or `None` when a column's type has no index or the packed positions do not
    /// fit 64 bits. A tuple with a null is in no index.
    pub(crate) fn new(values: &[ArrayRef]) -> Option<Self> {
        let columns = (values.iter())
            .map(ColumnIndex::new)
            .collect::<Option<Vec<_>>>()?;
        let radices: Vec<u64> = (columns.iter()).map(|index| index.len() as u64).collect();
        if values.len() == 1 {
            return Some(Self {
                columns,
                tuples: None,
                radices,
            });
        }
        (radices.iter()).try_fold(1u64, |product, radix| product.checked_mul(*radix))?;
        let positions: Vec<Vec<u32>> = (columns.iter().zip(values))
            .map(|(index, list)| index.positions(list))
            .collect();
        let len = values.first().map_or(0, |list| list.len());
        let tuples = (0..len)
            .filter_map(|row| pack(&positions, &radices, row))
            .collect();
        Some(Self {
            columns,
            tuples: Some(tuples),
            radices,
        })
    }

    /// Whether each row of `columns`, of the value types that the index was built for, equals
    /// one of the tuples. A row with a null equals none.
    pub(crate) fn contains(&self, columns: &[ArrayRef]) -> BooleanArray {
        let len = columns.first().map_or(0, |column| column.len());
        let Some(tuples) = &self.tuples else {
            return BooleanArray::new(self.columns[0].contains(&columns[0]), None);
        };
        let positions: Vec<Vec<u32>> = (self.columns.iter().zip(columns))
            .map(|(index, column)| index.positions(column))
            .collect();
        let found = BooleanBuffer::collect_bool(len, |row| {
            pack(&positions, &self.radices, row).is_some_and(|tuple| tuples.contains(&tuple))
        });
        BooleanArray::new(found, None)
    }
}

/// The packed positions of the values of `row`, `None` when a value is not among the set's.
fn pack(positions: &[Vec<u32>], radices: &[u64], row: usize) -> Option<u64> {
    let mut packed = 0u64;
    for (positions, radix) in positions.iter().zip(radices) {
        let position = positions[row];
        if position == NONE {
            return None;
        }
        packed = packed * radix + position as u64;
    }
    Some(packed)
}

/// A dictionary key, as the index of its value. A negative key has no value.
trait Key: Copy {
    fn index(self) -> usize;
}

macro_rules! keys {
    ($($native:ty),*) => {$(
        impl Key for $native {
            #[inline]
            fn index(self) -> usize {
                self as usize
            }
        }
    )*};
}

keys!(i8, i16, i32, i64, u8, u16, u32, u64);

/// `$body` with `$keys` bound to the values of the dictionary keys `$array` in their own type.
macro_rules! by_key {
    ($array:expr, |$keys:ident| $body:expr) => {{
        let array = $array;
        match array.data_type() {
            DataType::Int8 => {
                let $keys = array.as_primitive::<Int8Type>().values().as_ref();
                $body
            }
            DataType::Int16 => {
                let $keys = array.as_primitive::<Int16Type>().values().as_ref();
                $body
            }
            DataType::Int32 => {
                let $keys = array.as_primitive::<Int32Type>().values().as_ref();
                $body
            }
            DataType::Int64 => {
                let $keys = array.as_primitive::<Int64Type>().values().as_ref();
                $body
            }
            DataType::UInt8 => {
                let $keys = array.as_primitive::<UInt8Type>().values().as_ref();
                $body
            }
            DataType::UInt16 => {
                let $keys = array.as_primitive::<UInt16Type>().values().as_ref();
                $body
            }
            DataType::UInt32 => {
                let $keys = array.as_primitive::<UInt32Type>().values().as_ref();
                $body
            }
            _ => {
                let $keys = array.as_primitive::<UInt64Type>().values().as_ref();
                $body
            }
        }
    }};
}

/// The positions of one column's distinct set values.
enum ColumnIndex {
    Integers(IntegerIndex),
    Bytes(HashMap<Box<[u8]>, u32, FixedState>),
}

impl ColumnIndex {
    /// The index of the non-null values of `values`, or `None` for a type without an index.
    fn new(values: &ArrayRef) -> Option<Self> {
        let valid = |row: &usize| values.is_valid(*row);
        if let Some(integers) = integers(values.as_ref()) {
            let distinct = (0..values.len()).filter(valid).map(|row| integers[row]);
            return Some(ColumnIndex::Integers(IntegerIndex::new(distinct.collect())));
        }
        let bytes = Bytes::new(values.as_ref())?;
        let mut positions: HashMap<Box<[u8]>, u32, FixedState> = HashMap::default();
        for row in (0..values.len()).filter(valid) {
            let next = positions.len() as u32;
            positions.entry(bytes.value(row).into()).or_insert(next);
        }
        Some(ColumnIndex::Bytes(positions))
    }

    /// The number of distinct values.
    fn len(&self) -> usize {
        match self {
            ColumnIndex::Integers(index) => index.len,
            ColumnIndex::Bytes(positions) => positions.len(),
        }
    }

    /// Whether each value of `column` is one of the set's values, without nulls.
    fn contains(&self, column: &ArrayRef) -> BooleanBuffer {
        let found = if let DataType::Dictionary(_, _) = column.data_type() {
            let dictionary = column.as_any_dictionary();
            let found: Vec<bool> = (self.positions(dictionary.values()).into_iter())
                .map(|position| position != NONE)
                .collect();
            by_key!(dictionary.keys(), |keys| BooleanBuffer::collect_bool(
                keys.len(),
                |row| found.get(keys[row].index()).copied().unwrap_or(false)
            ))
        } else {
            match self {
                ColumnIndex::Integers(index) => {
                    index.contains(&integers(column.as_ref()).expect("an integer column"))
                }
                ColumnIndex::Bytes(_) => {
                    let positions = self.positions(column);
                    BooleanBuffer::collect_bool(positions.len(), |row| positions[row] != NONE)
                }
            }
        };
        match column.nulls() {
            Some(nulls) => &found & nulls.inner(),
            None => found,
        }
    }

    /// The position of each value of `column` among the set's values, or [`NONE`] for a null and
    /// a value that is not one of them. A dictionary column maps its dictionary's values once.
    fn positions(&self, column: &ArrayRef) -> Vec<u32> {
        let len = column.len();
        let mut positions: Vec<u32> = if let DataType::Dictionary(_, _) = column.data_type() {
            let dictionary = column.as_any_dictionary();
            let by_key = self.positions(dictionary.values());
            by_key!(dictionary.keys(), |keys| (keys.iter())
                .map(|key| by_key.get(key.index()).copied().unwrap_or(NONE))
                .collect())
        } else {
            match self {
                ColumnIndex::Integers(index) => {
                    let integers = integers(column.as_ref()).expect("an integer column");
                    integers
                        .iter()
                        .map(|value| index.position(*value))
                        .collect()
                }
                ColumnIndex::Bytes(table) => {
                    let bytes = Bytes::new(column.as_ref()).expect("a string or binary column");
                    (0..len)
                        .map(|row| table.get(bytes.value(row)).copied().unwrap_or(NONE))
                        .collect()
                }
            }
        };
        if let Some(nulls) = column.nulls() {
            for row in (0..len).filter(|&row| nulls.is_null(row)) {
                positions[row] = NONE;
            }
        }
        positions
    }
}

/// The positions of the distinct values of an integer column: a table over their range when it
/// is dense, or a hash table.
struct IntegerIndex {
    len: usize,
    table: IntegerTable,
}

enum IntegerTable {
    /// The position of each integer from `low`, or [`NONE`].
    Dense {
        low: i64,
        positions: Vec<u32>,
    },
    Hashed(HashMap<i64, u32, FixedState>),
}

impl IntegerIndex {
    fn new(values: Vec<i64>) -> Self {
        let mut positions: HashMap<i64, u32, FixedState> = HashMap::default();
        for value in values {
            let next = positions.len() as u32;
            positions.entry(value).or_insert(next);
        }
        let len = positions.len();
        let (low, high) = (positions.keys()).fold((i64::MAX, i64::MIN), |(low, high), value| {
            (low.min(*value), high.max(*value))
        });
        let span = (high as i128 - low as i128 + 1) as u64;
        let dense = len > 0 && span <= DENSE_ENTRIES.min((len as u64 * 64).max(1024));
        let table = if dense {
            let mut table = vec![NONE; span as usize];
            for (value, position) in positions {
                table[(value - low) as usize] = position;
            }
            IntegerTable::Dense {
                low,
                positions: table,
            }
        } else {
            IntegerTable::Hashed(positions)
        };
        Self { len, table }
    }

    #[inline]
    fn position(&self, value: i64) -> u32 {
        match &self.table {
            IntegerTable::Dense { low, positions } => {
                let offset = value.wrapping_sub(*low) as u64;
                if offset < positions.len() as u64 {
                    positions[offset as usize]
                } else {
                    NONE
                }
            }
            IntegerTable::Hashed(positions) => positions.get(&value).copied().unwrap_or(NONE),
        }
    }

    /// Whether each of `values` is one of the set's values, ignoring nulls.
    fn contains(&self, values: &[i64]) -> BooleanBuffer {
        match &self.table {
            IntegerTable::Dense { low, positions } => {
                let entries = positions.len() as u64;
                BooleanBuffer::collect_bool(values.len(), |row| {
                    let offset = values[row].wrapping_sub(*low) as u64;
                    let within = offset < entries;
                    within & (positions[if within { offset as usize } else { 0 }] != NONE)
                })
            }
            IntegerTable::Hashed(positions) => BooleanBuffer::collect_bool(values.len(), |row| {
                positions.contains_key(&values[row])
            }),
        }
    }
}

/// The values of `array`, an integer, temporal, or boolean array, as `i64`: the buffer itself for a
/// type of 64 bits, whose bits keep their equality, or the values widened, or `None` for another
/// type. The values of null rows are arbitrary.
fn integers(array: &dyn Array) -> Option<ScalarBuffer<i64>> {
    macro_rules! widened {
        ($type:ty) => {
            (array.as_primitive::<$type>().values().iter())
                .map(|value| *value as i64)
                .collect()
        };
    }
    use DataType::*;
    Some(match array.data_type() {
        Int64 | UInt64 | Date64 | Time64(_) | Timestamp(_, _) | Duration(_) => {
            let data = array.to_data();
            ScalarBuffer::new(data.buffers()[0].clone(), data.offset(), data.len())
        }
        Int8 => widened!(Int8Type),
        Int16 => widened!(Int16Type),
        Int32 => widened!(Int32Type),
        UInt8 => widened!(UInt8Type),
        UInt16 => widened!(UInt16Type),
        UInt32 => widened!(UInt32Type),
        Date32 => widened!(Date32Type),
        Time32(TimeUnit::Second) => widened!(Time32SecondType),
        Time32(TimeUnit::Millisecond) => widened!(Time32MillisecondType),
        Boolean => (array.as_boolean().values().iter())
            .map(i64::from)
            .collect(),
        _ => return None,
    })
}

/// A string or binary array, whose rows read as bytes.
enum Bytes<'a> {
    Utf8(&'a StringArray),
    LargeUtf8(&'a LargeStringArray),
    Utf8View(&'a StringViewArray),
    Binary(&'a BinaryArray),
    LargeBinary(&'a LargeBinaryArray),
    BinaryView(&'a BinaryViewArray),
}

impl<'a> Bytes<'a> {
    /// `array` as bytes, or `None` for an array of another type.
    fn new(array: &'a dyn Array) -> Option<Self> {
        Some(match array.data_type() {
            DataType::Utf8 => Bytes::Utf8(array.as_string()),
            DataType::LargeUtf8 => Bytes::LargeUtf8(array.as_string()),
            DataType::Utf8View => Bytes::Utf8View(array.as_string_view()),
            DataType::Binary => Bytes::Binary(array.as_binary()),
            DataType::LargeBinary => Bytes::LargeBinary(array.as_binary()),
            DataType::BinaryView => Bytes::BinaryView(array.as_binary_view()),
            _ => return None,
        })
    }

    #[inline]
    fn value(&self, row: usize) -> &'a [u8] {
        match self {
            Bytes::Utf8(array) => array.value(row).as_bytes(),
            Bytes::LargeUtf8(array) => array.value(row).as_bytes(),
            Bytes::Utf8View(array) => array.value(row).as_bytes(),
            Bytes::Binary(array) => array.value(row),
            Bytes::LargeBinary(array) => array.value(row),
            Bytes::BinaryView(array) => array.value(row),
        }
    }
}
