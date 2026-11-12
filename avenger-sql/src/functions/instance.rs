//! The aggregate of one bound call of a mark: its layer without a table, and the columns of the
//! table, which its accumulator collects.

use std::sync::Arc;

use super::rows::{self, Rows};
use avenger_chart::{chart_field, functions::MarkTemplate};
use datafusion::{
    arrow::{
        array::{ArrayRef, RecordBatch, RecordBatchOptions},
        datatypes::{DataType, FieldRef, Schema},
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
            order: rows::order(order_by),
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
        let types: Vec<DataType> = (self.fields.iter())
            .map(|field| field.data_type().clone())
            .collect();
        Ok(rows::state_fields(&args, &types))
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
            rows: Rows::new(types),
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
    rows: Rows,
}

impl Accumulator for MarkAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        self.rows.update(values);
        Ok(())
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        self.rows.merge(states);
        Ok(())
    }

    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        self.rows.state()
    }

    fn evaluate(&mut self) -> Result<ScalarValue> {
        let instance = &self.instance;
        let columns = self.rows.sorted(instance.fields.len(), &instance.order)?;
        let rows = columns.first().map_or(0, |column| column.len());
        let schema = Arc::new(Schema::new(instance.fields.clone()));
        let options = RecordBatchOptions::new().with_row_count(Some(rows));
        let table = RecordBatch::try_new_with_options(schema, columns, &options)?;
        let name = instance.template.name();
        let chart = (instance.template.chart(table))
            .map_err(|error| exec_datafusion_err!("{name}(): {error}"))?;
        let bytes = chart
            .to_bytes()
            .map_err(|error| exec_datafusion_err!("{name}(): {error}"))?;
        Ok(ScalarValue::BinaryView(Some(bytes.to_vec())))
    }

    fn size(&self) -> usize {
        self.rows.size()
    }
}
