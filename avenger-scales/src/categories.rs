//! Categories of any Arrow type, matched by value: the position of each value among a domain's
//! categories, and the distinct values of an array. Structs, lists, and dictionaries match by
//! value as numbers and strings do, and a null matches a null.

use std::{collections::HashMap, fmt, slice, sync::Arc};

use arrow::{
    array::{ArrayRef, UInt32Array},
    compute::{cast, take},
    datatypes::DataType,
    row::{RowConverter, SortField},
};

use crate::error::AvengerScaleError;

/// The categories of a domain, which find the position of values among them.
#[derive(Clone)]
pub struct CategoryIndex {
    data_type: DataType,
    converter: Arc<RowConverter>,
    /// The first position of each category, by its bytes in Arrow's row format, which compare as
    /// the categories do.
    positions: HashMap<Box<[u8]>, usize>,
}

impl fmt::Debug for CategoryIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CategoryIndex")
            .field("data_type", &self.data_type)
            .field("categories", &self.positions.len())
            .finish()
    }
}

impl CategoryIndex {
    /// The index of `categories`. A category that repeats has the position of its first
    /// occurrence.
    pub fn new(categories: &ArrayRef) -> Result<Self, AvengerScaleError> {
        let data_type = categories.data_type().clone();
        let converter = RowConverter::new(vec![SortField::new(data_type.clone())])?;
        let rows = converter.convert_columns(slice::from_ref(categories))?;
        let mut positions = HashMap::with_capacity(rows.num_rows());
        for (position, row) in rows.iter().enumerate() {
            positions.entry(row.as_ref().into()).or_insert(position);
        }
        Ok(Self {
            data_type,
            converter: Arc::new(converter),
            positions,
        })
    }

    /// The position of each of `values` among the categories, or `None` for a value that is not
    /// one of them. Values of another type are cast to the categories' type first, so Utf8 values
    /// match dictionary-encoded strings, and values that do not cast are an error.
    pub fn positions(&self, values: &ArrayRef) -> Result<Vec<Option<usize>>, AvengerScaleError> {
        let values = match values.data_type() == &self.data_type {
            true => values.clone(),
            false => cast(values, &self.data_type).map_err(|_| {
                AvengerScaleError::ScaleOperationNotSupported(format!(
                    "Cannot cast values of type {:?} to domain type {:?}",
                    values.data_type(),
                    self.data_type
                ))
            })?,
        };
        let rows = self.converter.convert_columns(slice::from_ref(&values))?;
        Ok((rows.iter())
            .map(|row| self.positions.get(row.as_ref()).copied())
            .collect())
    }
}

/// The distinct values of `values`, in the order in which each first occurs.
pub fn distinct(values: &ArrayRef) -> Result<ArrayRef, AvengerScaleError> {
    let index = CategoryIndex::new(values)?;
    let mut first: Vec<u32> = index.positions.values().map(|&p| p as u32).collect();
    first.sort_unstable();
    Ok(take(values.as_ref(), &UInt32Array::from(first), None)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::{
        array::{Array, AsArray, DictionaryArray, Int64Array, StringArray, StructArray},
        datatypes::{Field, Int32Type},
    };

    fn keys(values: Vec<Option<i64>>, others: Vec<Option<&str>>) -> ArrayRef {
        Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new("value", DataType::Int64, true)),
                Arc::new(Int64Array::from(values)) as ArrayRef,
            ),
            (
                Arc::new(Field::new("other", DataType::Utf8, true)),
                Arc::new(StringArray::from(others)) as ArrayRef,
            ),
        ]))
    }

    #[test]
    fn structs_match_by_value_with_nulls() {
        let index = CategoryIndex::new(&keys(
            vec![Some(4), Some(8), None],
            vec![None, None, Some("Other")],
        ))
        .unwrap();
        let positions = index
            .positions(&keys(
                vec![None, Some(8), Some(5)],
                vec![Some("Other"), None, None],
            ))
            .unwrap();
        assert_eq!(positions, [Some(2), Some(1), None]);
    }

    #[test]
    fn values_cast_to_the_categories_type_and_repeats_keep_the_first_position() {
        let categories: DictionaryArray<Int32Type> = vec!["a", "b", "a"].into_iter().collect();
        let index = CategoryIndex::new(&(Arc::new(categories) as ArrayRef)).unwrap();
        let values: ArrayRef = Arc::new(StringArray::from(vec![Some("a"), Some("c"), None]));
        assert_eq!(index.positions(&values).unwrap(), [Some(0), None, None]);
    }

    #[test]
    fn distinct_values_keep_their_first_order() {
        let values: ArrayRef = Arc::new(StringArray::from(vec!["b", "a", "b", "c", "a"]));
        let distinct = distinct(&values).unwrap();
        let distinct = distinct.as_string::<i32>();
        assert_eq!(distinct.len(), 3);
        assert_eq!(
            (0..3).map(|i| distinct.value(i)).collect::<Vec<_>>(),
            ["b", "a", "c"]
        );
    }
}
