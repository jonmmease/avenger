//! The aggregate of one bound call of a composition, which composes the charts of the rows of its
//! group.

use std::sync::Arc;

use arrow::{
    array::ArrayRef,
    datatypes::{DataType, FieldRef},
};
use avenger_chart::{
    chart_field, charts_from_arrow,
    functions::{Arg, Child, ChildOrder, Composition, CompositionKind},
    Decode,
};
use datafusion_common::{exec_datafusion_err, Result, ScalarValue};
use datafusion_expr::{
    function::{AccumulatorArgs, StateFieldsArgs},
    utils::AggregateOrderSensitivity,
    Accumulator, AggregateUDFImpl, Signature, SortExpr, Volatility,
};

use super::rows::{self, Rows};

/// The SQL name of the aggregate of a composition: `layer_agg`, `hconcat_agg`, and `vconcat_agg`
/// for a layer and the concats, as GoogleSQL names `STRING_AGG`, so that a scalar of the charts of
/// a row can take the short name, and its name for a facet, which composes only rows.
pub fn aggregate_name(spec: &Composition) -> &'static str {
    match spec.kind {
        CompositionKind::Layer => "layer_agg",
        CompositionKind::HConcat => "hconcat_agg",
        CompositionKind::VConcat => "vconcat_agg",
        _ => spec.name,
    }
}

/// The name of a bound composition, the column of its key, its options, and the sort options of
/// its `ORDER BY`.
pub(crate) type CompositionParts<'a> = (
    &'a str,
    Option<&'a str>,
    &'a [Option<ScalarValue>],
    &'a [(bool, bool)],
);

/// One bound call of a composition: the name of its composition, the column of its key, its
/// options, and the sort options of its `ORDER BY`. It takes the charts, the keys of a facet, then
/// the keys of its `ORDER BY`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CompositionInstance {
    spec: &'static str,
    /// The SQL name of the aggregate.
    name: &'static str,
    column: Option<String>,
    options: Vec<Option<ScalarValue>>,
    order: Vec<(bool, bool)>,
    signature: Signature,
}

impl CompositionInstance {
    /// The bound call of the composition `spec`, with the column of its key, its checked options,
    /// and the keys of its `ORDER BY`.
    pub fn new(
        spec: &'static Composition,
        column: Option<String>,
        options: Vec<Option<ScalarValue>>,
        order_by: &[SortExpr],
    ) -> Self {
        Self {
            spec: spec.name,
            name: aggregate_name(spec),
            column,
            options,
            order: rows::order(order_by),
            signature: Signature::user_defined(Volatility::Immutable),
        }
    }

    /// The bound call of the composition `spec`, with the column of its key, its options, and the
    /// sort options of its `ORDER BY`.
    pub(crate) fn with_order(
        spec: &'static Composition,
        column: Option<String>,
        options: Vec<Option<ScalarValue>>,
        order: Vec<(bool, bool)>,
    ) -> Self {
        Self {
            spec: spec.name,
            name: aggregate_name(spec),
            column,
            options,
            order,
            signature: Signature::user_defined(Volatility::Immutable),
        }
    }

    /// The name of the composition, the column of its key, its options, and the sort options of
    /// its `ORDER BY`.
    pub(crate) fn parts(&self) -> CompositionParts<'_> {
        (
            self.spec,
            self.column.as_deref(),
            &self.options,
            &self.order,
        )
    }

    fn composition(&self) -> &'static Composition {
        avenger_chart::functions::composition(self.spec).expect("a composition of the table")
    }

    /// The number of data columns: the charts, and the keys of a facet.
    fn data(&self) -> usize {
        1 + usize::from(self.column.is_some())
    }
}

impl AggregateUDFImpl for CompositionInstance {
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
        let types: Vec<DataType> = (args.input_fields.iter().take(self.data()))
            .map(|field| field.data_type().clone())
            .collect();
        Ok(rows::state_fields(&args, &types))
    }

    fn accumulator(&self, args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        let data = (args.exprs.iter().take(self.data())).map(|expr| expr.data_type(args.schema));
        let keys = (args.order_bys.iter()).map(|sort| sort.expr.data_type(args.schema));
        let types = data.chain(keys).collect::<Result<Vec<_>>>()?;
        Ok(Box::new(CompositionAccumulator {
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
        // The accumulator sorts the rows by the keys of the `ORDER BY`, so it needs no order of
        // its input.
        Ok(Some(self))
    }
}

/// The rows of one group: the charts, the keys of a facet, and the keys of the `ORDER BY`.
#[derive(Debug)]
struct CompositionAccumulator {
    instance: CompositionInstance,
    rows: Rows,
}

impl Accumulator for CompositionAccumulator {
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
        let name = instance.name;
        let columns = self.rows.sorted(instance.data(), &instance.order)?;
        let charts = charts_from_arrow(columns[0].as_ref(), Decode::Auto)
            .map_err(|error| exec_datafusion_err!("{name}(): {error}"))?;
        let children: Vec<Child> = (charts.into_iter().enumerate())
            .map(|(row, chart)| Child {
                chart,
                key: columns.get(1).map(|keys| keys.slice(row, 1)),
            })
            .collect();
        let order = match (instance.order.is_empty(), instance.column.is_some()) {
            (true, true) => ChildOrder::ByKey,
            _ => ChildOrder::AsGiven,
        };
        let options = (instance.options.iter())
            .map(|value| match value {
                Some(value) => super::arg(value),
                None => Ok(None),
            })
            .collect::<Result<Vec<Option<Arg>>>>()?;
        let chart = (instance.composition())
            .compose(
                instance.column.as_deref(),
                children,
                order,
                &options,
                super::FORMAT_SYNTAXES,
            )
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
