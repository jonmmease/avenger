//! The rows of a group of an aggregate that collects them, such as a mark or a composition: the
//! arrays of each argument column and of each key of its `ORDER BY`, which partial states carry
//! as lists.

use std::sync::Arc;

use avenger_chart::list_value;
use datafusion::{
    arrow::{
        array::{new_empty_array, Array, ArrayRef, AsArray, UInt32Array},
        compute::{concat, lexsort_to_indices, take, SortColumn, SortOptions},
        datatypes::{DataType, Field, FieldRef},
    },
    common::{Result, ScalarValue},
    logical_expr::{function::StateFieldsArgs, SortExpr},
};

/// The fields of the partial state of an aggregate whose arguments have the types `types`: a list
/// of each argument, then of each key of its `ORDER BY`.
pub(super) fn state_fields(args: &StateFieldsArgs, types: &[DataType]) -> Vec<FieldRef> {
    let keys = args.ordering_fields.iter().map(|field| field.data_type());
    (types.iter().chain(keys).enumerate())
        .map(|(index, data_type)| {
            let item = Field::new_list_field(data_type.clone(), true);
            Arc::new(Field::new(
                format!("{}[{index}]", args.name),
                DataType::List(Arc::new(item)),
                true,
            ))
        })
        .collect()
}

/// The sort options of the keys of `order_by`: whether each is ascending, and whether its nulls
/// come first.
pub(super) fn order(order_by: &[SortExpr]) -> Vec<(bool, bool)> {
    (order_by.iter())
        .map(|sort| (sort.asc, sort.nulls_first))
        .collect()
}

/// The rows of one group: the arrays of each column, in the order in which they arrive.
#[derive(Debug)]
pub(super) struct Rows {
    columns: Vec<(DataType, Vec<ArrayRef>)>,
}

impl Rows {
    /// The rows of columns of the types `types`.
    pub(super) fn new(types: Vec<DataType>) -> Self {
        Self {
            columns: types.into_iter().map(|t| (t, Vec::new())).collect(),
        }
    }

    pub(super) fn update(&mut self, values: &[ArrayRef]) {
        for ((_, arrays), values) in self.columns.iter_mut().zip(values) {
            arrays.push(values.clone());
        }
    }

    pub(super) fn merge(&mut self, states: &[ArrayRef]) {
        for ((_, arrays), state) in self.columns.iter_mut().zip(states) {
            let lists = state.as_list::<i32>();
            for row in 0..lists.len() {
                if lists.is_valid(row) {
                    arrays.push(lists.value(row));
                }
            }
        }
    }

    pub(super) fn state(&self) -> Result<Vec<ScalarValue>> {
        (self.concatenated()?.into_iter())
            .map(|column| ScalarValue::try_from_array(&list_value(column), 0))
            .collect()
    }

    /// Each column, concatenated.
    pub(super) fn concatenated(&self) -> Result<Vec<ArrayRef>> {
        (self.columns.iter())
            .map(|(data_type, arrays)| match arrays.as_slice() {
                [] => Ok(new_empty_array(data_type)),
                [array] => Ok(array.clone()),
                arrays => {
                    let arrays: Vec<&dyn Array> =
                        arrays.iter().map(|array| array.as_ref()).collect();
                    Ok(concat(&arrays)?)
                }
            })
            .collect()
    }

    /// The columns, concatenated, split into the first `count` columns and the keys of the
    /// `ORDER BY`, with the first columns sorted by the keys in the order `order`.
    pub(super) fn sorted(&self, count: usize, order: &[(bool, bool)]) -> Result<Vec<ArrayRef>> {
        let mut columns = self.concatenated()?;
        let keys = columns.split_off(count);
        if keys.is_empty() {
            return Ok(columns);
        }
        let sort: Vec<SortColumn> = (keys.into_iter().zip(order))
            .map(|(values, (asc, nulls_first))| SortColumn {
                values,
                options: Some(SortOptions {
                    descending: !asc,
                    nulls_first: *nulls_first,
                }),
            })
            .collect();
        let indices: UInt32Array = lexsort_to_indices(&sort, None)?;
        (columns.iter())
            .map(|column| Ok(take(column.as_ref(), &indices, None)?))
            .collect()
    }

    pub(super) fn size(&self) -> usize {
        let arrays = self.columns.iter().flat_map(|(_, arrays)| arrays);
        std::mem::size_of_val(self)
            + arrays
                .map(|array| array.get_array_memory_size())
                .sum::<usize>()
    }
}
