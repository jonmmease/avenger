//! The aggregate of one bound call of a mark: its layer without a table, and the columns of the
//! table, which its accumulator collects.

use std::sync::Arc;

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
use prost::{bytes::Bytes, Message};

use super::{declaration::builder, LayerMark, MarkSpec};
use crate::{
    builder::{from_fragment, to_fragment, Parts},
    chart_field, list_value,
    proto::{self, Plot},
    Chart, ChartError, CoordKind,
};

/// One bound call of a mark: the encoded fragment of its layer, which holds its encodings and their
/// values and no table, the encoded fragment of the options of its plot, the fields of the columns
/// of its table, and the sort options of its draw order. It takes the data arguments of the call,
/// then the keys of its `ORDER BY`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MarkInstance {
    name: &'static str,
    coord: CoordKind,
    layer: Bytes,
    options: Option<Bytes>,
    fields: Vec<FieldRef>,
    order: Vec<(bool, bool)>,
    signature: Signature,
}

impl MarkInstance {
    pub(crate) fn new(
        spec: &'static MarkSpec,
        mark: LayerMark,
        parts: Parts,
        options: Option<proto::Fragment>,
        fields: Vec<FieldRef>,
        order_by: &[SortExpr],
    ) -> Result<Self> {
        let fragment = match mark {
            LayerMark::Cartesian(mark) => to_fragment(builder(
                proto::CartesianLayer {
                    mark: Some(mark),
                    ..Default::default()
                },
                parts,
            )),
            LayerMark::Polar(mark) => to_fragment(builder(
                proto::PolarLayer {
                    mark: Some(mark),
                    ..Default::default()
                },
                parts,
            )),
            LayerMark::Geo(mark) => to_fragment(builder(
                proto::GeoLayer {
                    mark: Some(mark),
                    ..Default::default()
                },
                parts,
            )),
        }
        .map_err(|error| plan_error(spec.name, error))?;
        Ok(Self {
            name: spec.name,
            coord: spec.coord,
            layer: fragment.encode_to_vec().into(),
            options: options.map(|options| options.encode_to_vec().into()),
            fields,
            order: (order_by.iter())
                .map(|sort| (sort.asc, sort.nulls_first))
                .collect(),
            signature: Signature::user_defined(Volatility::Immutable),
        })
    }

    /// The chart of one layer whose table is `table`, in a plot with the options of the call.
    fn chart(&self, table: RecordBatch) -> std::result::Result<Chart, ChartError> {
        let decode = |bytes: &Bytes| {
            proto::Fragment::decode(bytes.clone())
                .map_err(|error| ChartError::invalid("", error.to_string()))
        };
        let fragment = decode(&self.layer)?;
        let options = self.options.as_ref().map(decode).transpose()?;
        let plot = match self.coord {
            CoordKind::Cartesian => {
                let mut layer = from_fragment::<proto::CartesianLayer>(&fragment)?;
                layer.message.table = Some(layer.parts.table("table", table));
                let mut plot = proto::CartesianPlot::new([layer]);
                if let Some(options) = &options {
                    plot = plot.options(from_fragment::<proto::Cartesian>(options)?);
                }
                Plot::cartesian(plot)
            }
            CoordKind::Polar => {
                let mut layer = from_fragment::<proto::PolarLayer>(&fragment)?;
                layer.message.table = Some(layer.parts.table("table", table));
                let mut plot = proto::PolarPlot::new([layer]);
                if let Some(options) = &options {
                    plot = plot.options(from_fragment::<proto::Polar>(options)?);
                }
                Plot::polar(plot)
            }
            CoordKind::Geo => {
                let mut layer = from_fragment::<proto::GeoLayer>(&fragment)?;
                layer.message.table = Some(layer.parts.table("table", table));
                let options = options.expect("a geo mark has the options of its plot");
                let options = from_fragment::<proto::Geo>(&options)?;
                Plot::geo(proto::GeoPlot::new(options, [layer]))
            }
        };
        Chart::builder(plot).build()
    }
}

/// A planning error of the mark `name`.
fn plan_error(name: &str, error: ChartError) -> datafusion::error::DataFusionError {
    datafusion::error::DataFusionError::Plan(format!("{name}(): {error}"))
}

impl AggregateUDFImpl for MarkInstance {
    fn name(&self) -> &str {
        self.name
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
        Ok(Arc::new(chart_field(self.name, true)))
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
        let chart = instance
            .chart(table)
            .map_err(|error| exec_datafusion_err!("{}(): {error}", instance.name))?;
        let bytes = chart
            .to_bytes()
            .map_err(|error| exec_datafusion_err!("{}(): {error}", instance.name))?;
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
