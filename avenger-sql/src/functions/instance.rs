//! The aggregate of one bound call of a mark: its layer without a table, and the columns of the
//! table, which its accumulator collects.

use std::sync::Arc;

use avenger_chart::{chart_field, functions::MarkTemplate, list_value};
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, RecordBatch, RecordBatchOptions},
        compute::{concat, lexsort_to_indices, take_record_batch, SortColumn, SortOptions},
        datatypes::{DataType, Field, FieldRef, Schema},
    },
    common::{exec_datafusion_err, Result, ScalarValue},
    logical_expr::{
        function::{AccumulatorArgs, StateFieldsArgs},
        utils::AggregateOrderSensitivity,
        Accumulator, AggregateUDFImpl, Signature, SortExpr, Volatility,
    },
};

/// One bound call of a mark: its template, the fields of the columns of its table, and the sort
/// options of its draw order. It takes the data arguments of the call, then the keys of its
/// `ORDER BY`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MarkInstance {
    template: MarkTemplate,
    fields: Vec<FieldRef>,
    order: Vec<(bool, bool)>,
    signature: Signature,
}

impl MarkInstance {
    pub(crate) fn new(
        template: MarkTemplate,
        fields: Vec<FieldRef>,
        order_by: &[SortExpr],
    ) -> Self {
        Self {
            template,
            fields,
            order: (order_by.iter())
                .map(|sort| (sort.asc, sort.nulls_first))
                .collect(),
            signature: Signature::user_defined(Volatility::Immutable),
        }
    }
}

impl AggregateUDFImpl for MarkInstance {
    fn name(&self) -> &str {
        self.template.name()
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn coerce_types(&self, arg_types: &[DataType]) -> Result<Vec<DataType>> {
        Ok(arg_types.to_vec())
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::BinaryView)
    }

    fn return_field(&self, _arg_fields: &[FieldRef]) -> Result<FieldRef> {
        Ok(Arc::new(chart_field(self.template.name(), true)))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        let keys = args.ordering_fields.iter().map(|field| field.data_type());
        let types = (self.fields.iter().map(|field| field.data_type())).chain(keys);
        Ok(types
            .enumerate()
            .map(|(index, data_type)| {
                let item = Field::new_list_field(data_type.clone(), true);
                Arc::new(Field::new(
                    format!("{}[{index}]", args.name),
                    DataType::List(Arc::new(item)),
                    true,
                ))
            })
            .collect())
    }

    fn accumulator(&self, args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        let keys = args
            .order_bys
            .iter()
            .map(|sort| sort.expr.data_type(args.schema));
        let types = (self
            .fields
            .iter()
            .map(|field| Ok(field.data_type().clone())))
        .chain(keys)
        .collect::<Result<Vec<_>>>()?;
        Ok(Box::new(MarkAccumulator {
            instance: self.clone(),
            columns: types.into_iter().map(|t| (t, Vec::new())).collect(),
        }))
    }

    fn order_sensitivity(&self) -> AggregateOrderSensitivity {
        AggregateOrderSensitivity::Beneficial
    }

    fn with_beneficial_ordering(
        self: Arc<Self>,
        _beneficial_ordering: bool,
    ) -> Result<Option<Arc<dyn AggregateUDFImpl>>> {
        // The accumulator sorts the rows by the keys of the draw order, so it needs no order of
        // its input.
        Ok(Some(self))
    }
}

/// The rows of one group: the arrays of each data column, then of each key of the draw order.
#[derive(Debug)]
struct MarkAccumulator {
    instance: MarkInstance,
    columns: Vec<(DataType, Vec<ArrayRef>)>,
}

impl MarkAccumulator {
    /// Each column, concatenated.
    fn concatenated(&self) -> Result<Vec<ArrayRef>> {
        (self.columns.iter())
            .map(|(data_type, arrays)| match arrays.as_slice() {
                [] => Ok(datafusion::arrow::array::new_empty_array(data_type)),
                [array] => Ok(array.clone()),
                arrays => {
                    let arrays: Vec<&dyn Array> =
                        arrays.iter().map(|array| array.as_ref()).collect();
                    Ok(concat(&arrays)?)
                }
            })
            .collect()
    }
}

impl Accumulator for MarkAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        for ((_, arrays), values) in self.columns.iter_mut().zip(values) {
            arrays.push(values.clone());
        }
        Ok(())
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        for ((_, arrays), state) in self.columns.iter_mut().zip(states) {
            let lists = state.as_list::<i32>();
            for row in 0..lists.len() {
                if lists.is_valid(row) {
                    arrays.push(lists.value(row));
                }
            }
        }
        Ok(())
    }

    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        (self.concatenated()?.into_iter())
            .map(|column| ScalarValue::try_from_array(&list_value(column), 0))
            .collect()
    }

    fn evaluate(&mut self) -> Result<ScalarValue> {
        let instance = &self.instance;
        let mut columns = self.concatenated()?;
        let keys = columns.split_off(instance.fields.len());
        let rows = columns.first().map_or(0, |column| column.len());
        let schema = Arc::new(Schema::new(instance.fields.clone()));
        let options = RecordBatchOptions::new().with_row_count(Some(rows));
        let mut table = RecordBatch::try_new_with_options(schema, columns, &options)?;
        if !keys.is_empty() {
            let sort: Vec<SortColumn> = (keys.into_iter().zip(&instance.order))
                .map(|(values, (asc, nulls_first))| SortColumn {
                    values,
                    options: Some(SortOptions {
                        descending: !asc,
                        nulls_first: *nulls_first,
                    }),
                })
                .collect();
            table = take_record_batch(&table, &lexsort_to_indices(&sort, None)?)?;
        }
        let name = instance.template.name();
        let chart = (instance.template.chart(table))
            .map_err(|error| exec_datafusion_err!("{name}(): {error}"))?;
        let bytes = chart
            .to_bytes()
            .map_err(|error| exec_datafusion_err!("{name}(): {error}"))?;
        Ok(ScalarValue::BinaryView(Some(bytes.to_vec())))
    }

    fn size(&self) -> usize {
        let arrays = self.columns.iter().flat_map(|(_, arrays)| arrays);
        std::mem::size_of_val(self)
            + arrays
                .map(|array| array.get_array_memory_size())
                .sum::<usize>()
    }
}
