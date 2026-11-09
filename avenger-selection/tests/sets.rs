//! Sets match through the index of their tuples exactly as through Arrow's row format, for every
//! indexed type, dictionaries, several columns, and prepared conditions that match many arrays.

use std::{collections::HashSet, sync::Arc};

use arrow::array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Date32Array, DictionaryArray, Float64Array,
    Int32Array, Int64Array, Int8Array, LargeStringArray, StringArray, StringViewArray,
    TimestampMillisecondArray, UInt64Array,
};
use arrow::compute::cast;
use arrow::datatypes::{DataType, Int32Type, Int8Type};
use arrow::row::{RowConverter, SortField};
use avenger_selection::Condition;

mod common;

use common::Noise;

/// The value of a column's type for the integer `value`, or a null.
fn column(data_type: &DataType, values: &[Option<i64>]) -> ArrayRef {
    let strings = || values.iter().map(|v| v.map(|v| format!("v{v}")));
    match data_type {
        DataType::Int8 => Arc::new(Int8Array::from_iter(
            values.iter().map(|v| v.map(|v| v as i8)),
        )),
        DataType::Int32 => Arc::new(Int32Array::from_iter(
            values.iter().map(|v| v.map(|v| v as i32)),
        )),
        DataType::Int64 => Arc::new(Int64Array::from_iter(values.iter().copied())),
        DataType::UInt64 => Arc::new(UInt64Array::from_iter(
            values.iter().map(|v| v.map(|v| u64::MAX - v as u64)),
        )),
        DataType::Date32 => Arc::new(Date32Array::from_iter(
            values.iter().map(|v| v.map(|v| v as i32)),
        )),
        DataType::Timestamp(_, _) => Arc::new(
            TimestampMillisecondArray::from_iter(
                values.iter().map(|v| v.map(|v| v * 1_000_000_007)),
            )
            .with_timezone("UTC"),
        ),
        DataType::Utf8 => Arc::new(StringArray::from_iter(strings())),
        DataType::LargeUtf8 => Arc::new(LargeStringArray::from_iter(strings())),
        DataType::Utf8View => Arc::new(StringViewArray::from_iter(strings())),
        DataType::Binary => Arc::new(BinaryArray::from_iter(
            strings().map(|v| v.map(String::into_bytes)),
        )),
        DataType::Boolean => Arc::new(BooleanArray::from_iter(
            values.iter().map(|v| v.map(|v| v % 2 == 0)),
        )),
        DataType::Float64 => Arc::new(Float64Array::from_iter(
            values.iter().map(|v| v.map(|v| v as f64 * 0.5)),
        )),
        DataType::Dictionary(_, value) => {
            let plain = column(value, values);
            cast(&plain, data_type).unwrap()
        }
        other => panic!("no column of {other}"),
    }
}

/// The rows of `columns` that equal one tuple of `values`, where a null equals a null, through
/// Arrow's row format, with the tuples cast to the columns' value types.
fn reference(columns: &[ArrayRef], values: &[ArrayRef]) -> Vec<bool> {
    let plain = |array: &ArrayRef| match array.data_type() {
        DataType::Dictionary(_, value) => cast(array, value).unwrap(),
        _ => array.clone(),
    };
    let columns: Vec<ArrayRef> = columns.iter().map(plain).collect();
    let values: Vec<ArrayRef> = (values.iter().zip(&columns))
        .map(|(list, column)| cast(&plain(list), column.data_type()).unwrap())
        .collect();
    let fields = columns
        .iter()
        .map(|column| SortField::new(column.data_type().clone()))
        .collect();
    let converter = RowConverter::new(fields).unwrap();
    let tuples = converter.convert_columns(&values).unwrap();
    let set: HashSet<_> = (0..values[0].len())
        .map(|tuple| tuples.row(tuple))
        .collect();
    let rows = converter.convert_columns(&columns).unwrap();
    rows.iter().map(|row| set.contains(&row)).collect()
}

fn bits(mask: BooleanArray) -> Vec<bool> {
    assert_eq!(mask.null_count(), 0, "a mask has no nulls");
    mask.values().iter().collect()
}

/// Random values below `distinct`, with nulls.
fn values(noise: &mut Noise, len: usize, distinct: u64) -> Vec<Option<i64>> {
    (0..len)
        .map(|_| (noise.below(10) != 0).then(|| noise.below(distinct) as i64))
        .collect()
}

/// Sets of one column of every indexed type, and of a type that matches through the row format,
/// match the rows of the reference, with dense and sparse values and nulls on both sides.
#[test]
fn sets_of_one_column_match_the_reference() {
    let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
    let types = [
        DataType::Int8,
        DataType::Int32,
        DataType::Int64,
        DataType::UInt64,
        DataType::Date32,
        DataType::Timestamp(arrow::datatypes::TimeUnit::Millisecond, Some("UTC".into())),
        DataType::Utf8,
        DataType::LargeUtf8,
        DataType::Utf8View,
        DataType::Binary,
        DataType::Boolean,
        DataType::Float64,
        DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::Utf8)),
        DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Int64)),
    ];
    for data_type in &types {
        // Few distinct values make a dense table, and a wide spread of values a hash table.
        for distinct in [20, 100, 1 << 40] {
            // Eight-bit integers and dictionary keys hold at most 128 distinct values.
            let narrow = match data_type {
                DataType::Int8 => true,
                DataType::Dictionary(keys, _) => **keys == DataType::Int8,
                _ => false,
            };
            let distinct = if narrow { distinct.min(100) } else { distinct };
            let rows = column(data_type, &values(&mut noise, 2_000, distinct));
            let set = column(data_type, &values(&mut noise, 30, distinct));
            let condition = Condition::values("c", None, set.clone());
            assert_eq!(
                bits(condition.matches(std::slice::from_ref(&rows)).unwrap()),
                reference(std::slice::from_ref(&rows), &[set]),
                "{data_type} with {distinct} distinct values"
            );
        }
    }
}

/// A set's values cast to the column's type, so an Int64 set matches a dictionary of Int8 keys
/// and Utf8 values through the values, a string set matches a Utf8View column, and an Int64 set
/// matches a column of floats.
#[test]
fn set_values_cast_to_the_columns_types() {
    let keys: DictionaryArray<Int8Type> = vec![Some("a"), Some("b"), None, Some("c"), Some("a")]
        .into_iter()
        .collect();
    let set = Condition::values("c", None, Arc::new(LargeStringArray::from(vec!["a", "c"])));
    assert_eq!(
        bits(set.matches(&[Arc::new(keys)]).unwrap()),
        [true, false, false, true, true]
    );
    let view: ArrayRef = Arc::new(StringViewArray::from(vec![Some("c"), None, Some("d")]));
    assert_eq!(bits(set.matches(&[view]).unwrap()), [true, false, false]);
    let narrow = Condition::values("n", None, Arc::new(Int64Array::from(vec![3, 300])));
    let column: ArrayRef = Arc::new(Int8Array::from(vec![3, 4, -3]));
    assert_eq!(
        bits(narrow.matches(&[column]).unwrap()),
        [true, false, false]
    );
    let floats: ArrayRef = Arc::new(Float64Array::from(vec![3.0, 3.5, 300.0]));
    assert_eq!(
        bits(narrow.matches(&[floats]).unwrap()),
        [true, false, true]
    );
}

/// A set holds a label and a list of values for each of its columns, at least one, and its lists
/// are of one length.
#[test]
fn sets_reject_lists_of_other_lengths() {
    let list = |values: &[i64]| -> ArrayRef { Arc::new(Int64Array::from(values.to_vec())) };
    let uneven = Condition::set(
        vec!["a".into(), "b".into()],
        vec![None, None],
        vec![list(&[1]), list(&[1, 2])],
    );
    assert!(uneven.is_err());
    let unlabeled = Condition::set(vec!["a".into()], vec![], vec![list(&[1])]);
    assert!(unlabeled.is_err());
    assert!(Condition::set(vec![], vec![], vec![]).is_err());
}

/// Sets of two and three columns match their tuples and not their cross combinations, through
/// dictionaries, strings, and integers, as the reference does.
#[test]
fn sets_of_several_columns_match_the_reference() {
    let mut noise = Noise(0xD1B5_4A32_D192_ED03);
    let types = [
        vec![
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            DataType::Int64,
        ],
        vec![DataType::Utf8, DataType::Int32, DataType::Boolean],
        vec![DataType::Int64, DataType::Float64],
    ];
    for column_types in &types {
        for distinct in [5, 50, 1 << 33] {
            let rows: Vec<ArrayRef> = (column_types.iter())
                .map(|t| column(t, &values(&mut noise, 3_000, distinct)))
                .collect();
            let tuples: Vec<ArrayRef> = (column_types.iter())
                .map(|t| column(t, &values(&mut noise, 40, distinct)))
                .collect();
            let names = (0..column_types.len()).map(|i| format!("c{i}")).collect();
            let set =
                Condition::set(names, vec![None; column_types.len()], tuples.clone()).unwrap();
            assert_eq!(
                bits(set.matches(&rows).unwrap()),
                reference(&rows, &tuples),
                "{column_types:?} with {distinct} distinct values"
            );
        }
    }
}

/// A prepared set matches many arrays, including dictionaries with different dictionaries and a
/// column of another type than it was prepared for.
#[test]
fn a_prepared_set_matches_many_arrays() {
    let set = Condition::values("c", None, Arc::new(StringArray::from(vec!["b", "d"])));
    let dictionary = DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8));
    let prepared = set.prepare(std::slice::from_ref(&dictionary)).unwrap();
    let first: DictionaryArray<Int32Type> = vec!["a", "b", "c"].into_iter().collect();
    let second: DictionaryArray<Int32Type> = vec!["d", "d", "e", "b"].into_iter().collect();
    assert_eq!(
        bits(prepared.matches(&[Arc::new(first)]).unwrap()),
        [false, true, false]
    );
    assert_eq!(
        bits(prepared.matches(&[Arc::new(second)]).unwrap()),
        [true, true, false, true]
    );
    let other: ArrayRef = Arc::new(LargeStringArray::from(vec!["d", "x"]));
    assert_eq!(bits(prepared.matches(&[other]).unwrap()), [true, false]);
}

#[test]
fn a_set_matches_a_null_as_a_value() {
    let column: ArrayRef = Arc::new(StringArray::from(vec![Some("a"), None, Some("b")]));
    let with_null = Condition::values(
        "category",
        None,
        Arc::new(StringArray::from(vec![Some("b"), None])),
    );
    let matched = bits(with_null.matches(std::slice::from_ref(&column)).unwrap());
    assert_eq!(matched, [false, true, true]);
    // A set without a null leaves the null row out, and its complement takes it.
    let without = Condition::values("category", None, Arc::new(StringArray::from(vec!["b"])));
    assert_eq!(
        bits(without.matches(std::slice::from_ref(&column)).unwrap()),
        [false, false, true]
    );
    assert_eq!(
        bits(
            (without.complement().unwrap())
                .matches(std::slice::from_ref(&column))
                .unwrap()
        ),
        [true, true, false]
    );
    // A tuple of two columns with a null matches the rows of that pair, also through the index.
    let pairs: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![Some("a"), Some("a"), None])),
        Arc::new(Int64Array::from(vec![None, Some(1), None])),
    ];
    let set = Condition::set(
        vec!["c".into(), "n".into()],
        vec![None, None],
        vec![
            Arc::new(StringArray::from(vec![Some("a")])),
            Arc::new(Int64Array::from(vec![None])),
        ],
    )
    .unwrap();
    assert_eq!(bits(set.matches(&pairs).unwrap()), [true, false, false]);
    let types = [DataType::Utf8, DataType::Int64];
    let prepared = set.prepare(&types).unwrap();
    assert_eq!(
        bits(prepared.matches(&pairs).unwrap()),
        [true, false, false]
    );
}

#[test]
fn a_complement_matches_the_values_outside_its_tuples() {
    let column: ArrayRef = Arc::new(StringArray::from(vec![
        Some("a"),
        Some("b"),
        Some("c"),
        None,
        Some("d"),
    ]));
    // Everything except `a`: so `b`, `c`, `d`, and the null row.
    let condition = Condition::values("category", None, Arc::new(StringArray::from(vec!["a"])))
        .complement()
        .unwrap();
    let matched = bits(condition.matches(std::slice::from_ref(&column)).unwrap());
    assert_eq!(matched, [false, true, true, true, true]);

    // When the tuples hold a null, the complement drops the null row.
    let condition = Condition::values(
        "category",
        None,
        Arc::new(StringArray::from(vec![Some("a"), None])),
    )
    .complement()
    .unwrap();
    let matched = bits(condition.matches(std::slice::from_ref(&column)).unwrap());
    assert_eq!(matched, [false, true, true, false, true]);

    // A prepared complement matches the same rows, and the complement changes the fingerprint.
    let prepared = condition.prepare(&[DataType::Utf8]).unwrap();
    assert_eq!(bits(prepared.matches(&[column]).unwrap()), matched);
    let plain = avenger_selection::Clause::new("plot").condition(Condition::values(
        "category",
        None,
        Arc::new(StringArray::from(vec![Some("a"), None])),
    ));
    let complement = avenger_selection::Clause::new("plot").condition(condition.clone());
    assert_ne!(plain.fingerprint(), complement.fingerprint());

    // The complement of a complement is the set.
    let set = condition.complement().unwrap();
    assert!(matches!(
        set,
        Condition::Set {
            complement: false,
            ..
        }
    ));
}

#[test]
fn only_a_set_has_a_complement() {
    let range = Condition::Range {
        column: "a".into(),
        channel: None,
        ends: Arc::new(arrow::array::Float64Array::from(vec![0.0, 1.0])),
        exclude_end: false,
        cells: None,
    };
    assert!(range.complement().is_err());
}
