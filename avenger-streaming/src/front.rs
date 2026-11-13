//! The state of a front: the accumulators of each group of an `AGGREGATE`, or the kept rows of an
//! `ORDER BY ... LIMIT`.

use std::{collections::HashMap, sync::Arc};

use avenger_sql::streaming::Front;
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, RecordBatch, UInt32Array, new_empty_array},
        compute::{SortColumn, concat_batches, lexsort_to_indices, take, take_record_batch},
        datatypes::{DataType, SchemaRef},
        row::{OwnedRow, RowConverter, SortField},
    },
    common::{DFSchemaRef, ParamValues, Result, ScalarValue},
    execution::context::ExecutionProps,
    logical_expr::{Accumulator, Expr, physical_planning_context::PhysicalPlanningContext},
    physical_expr::{
        PhysicalExpr, PhysicalSortExpr, aggregate::LoweredAggregateBuilder, create_physical_expr,
        create_physical_sort_exprs,
    },
};

pub(crate) enum FrontState {
    /// Before the query prepares its front.
    Empty,
    Aggregate(Box<AggregateState>),
    Limit(LimitState),
}

impl FrontState {
    /// The state of `front` over the rows of the schema `input`, the stages' output.
    pub(crate) fn new(
        front: &Front,
        input: &DFSchemaRef,
        params: &Option<ParamValues>,
        props: &ExecutionProps,
    ) -> Result<FrontState> {
        let bind = |expr: &Expr| bind_params(expr, params);
        let planning = PhysicalPlanningContext::default();
        let schema: SchemaRef = Arc::new(input.as_arrow().clone());
        match front {
            Front::Aggregate { groups, aggregates } => {
                let groups: Vec<Arc<dyn PhysicalExpr>> = (groups.iter())
                    .map(|e| create_physical_expr(&bind(e)?, input, props, &planning))
                    .collect::<Result<_>>()?;
                let mut lowered = Vec::new();
                for expr in aggregates {
                    let expr = bind(expr)?;
                    let built =
                        LoweredAggregateBuilder::new(&expr, input, &schema, props, &planning)
                            .build()?;
                    // An ordered aggregate takes its order's columns after its arguments, as
                    // DataFusion's aggregate gives them.
                    let mut inputs = built.aggregate.expressions();
                    inputs.extend(built.aggregate.order_bys().iter().map(|e| e.expr.clone()));
                    lowered.push(Lowered {
                        inputs,
                        filter: built.filter,
                        aggregate: built.aggregate,
                    });
                }
                let group_types: Vec<DataType> = (groups.iter())
                    .map(|g| g.data_type(&schema))
                    .collect::<Result<_>>()?;
                let converter = match group_types.is_empty() {
                    true => None,
                    false => Some(RowConverter::new(
                        group_types.iter().cloned().map(SortField::new).collect(),
                    )?),
                };
                let mut state = AggregateState {
                    groups,
                    aggregates: lowered,
                    converter,
                    group_types,
                    index: HashMap::new(),
                    keys: Vec::new(),
                    accumulators: Vec::new(),
                    values: Vec::new(),
                };
                // Without groups, the aggregate has one row, also over no rows.
                if state.converter.is_none() {
                    state.add_group(None)?;
                }
                Ok(FrontState::Aggregate(Box::new(state)))
            }
            Front::Limit { order, skip, fetch } => {
                let order: Vec<datafusion::logical_expr::SortExpr> = (order.iter())
                    .map(|key| {
                        Ok(datafusion::logical_expr::SortExpr {
                            expr: bind(&key.expr)?,
                            ..key.clone()
                        })
                    })
                    .collect::<Result<_>>()?;
                let order = create_physical_sort_exprs(&order, input, props, &planning)?;
                Ok(FrontState::Limit(LimitState {
                    order,
                    keep: skip + fetch,
                    skip: *skip,
                    schema,
                    kept: None,
                }))
            }
        }
    }

    /// Applies `rows` of the stages, and returns whether the output changed.
    pub(crate) fn apply(&mut self, rows: &RecordBatch) -> Result<bool> {
        match self {
            FrontState::Empty => Ok(false),
            FrontState::Aggregate(state) => state.apply(rows),
            FrontState::Limit(state) => state.apply(rows),
        }
    }

    /// The output's columns.
    pub(crate) fn output(&self) -> Result<Vec<ArrayRef>> {
        match self {
            FrontState::Empty => Ok(Vec::new()),
            FrontState::Aggregate(state) => state.output(),
            FrontState::Limit(state) => state.output(),
        }
    }
}

/// An aggregate of an `AGGREGATE` front: the accumulator's expression, the columns that its
/// accumulator takes, and its `FILTER`.
struct Lowered {
    aggregate: Arc<datafusion::physical_expr::aggregate::AggregateFunctionExpr>,
    inputs: Vec<Arc<dyn PhysicalExpr>>,
    filter: Option<Arc<dyn PhysicalExpr>>,
}

pub(crate) struct AggregateState {
    groups: Vec<Arc<dyn PhysicalExpr>>,
    aggregates: Vec<Lowered>,
    /// The converter of the group values to rows, or `None` without groups.
    converter: Option<RowConverter>,
    group_types: Vec<DataType>,
    /// Each group's index, by its group values.
    index: HashMap<OwnedRow, usize>,
    keys: Vec<OwnedRow>,
    /// Each group's accumulators, and their values after the last frame.
    accumulators: Vec<Vec<Box<dyn Accumulator>>>,
    values: Vec<Vec<ScalarValue>>,
}

impl AggregateState {
    fn add_group(&mut self, key: Option<OwnedRow>) -> Result<usize> {
        let mut accumulators: Vec<Box<dyn Accumulator>> = (self.aggregates.iter())
            .map(|a| a.aggregate.create_accumulator())
            .collect::<Result<_>>()?;
        let values = (accumulators.iter_mut())
            .map(|a| a.evaluate())
            .collect::<Result<_>>()?;
        let index = self.accumulators.len();
        if let Some(key) = key {
            self.index.insert(key.clone(), index);
            self.keys.push(key);
        }
        self.accumulators.push(accumulators);
        self.values.push(values);
        Ok(index)
    }

    fn apply(&mut self, rows: &RecordBatch) -> Result<bool> {
        let n = rows.num_rows();
        let mut changed = false;
        // The rows of each group, in their order.
        let mut by_group: Vec<(usize, Vec<u32>)> = Vec::new();
        let mut slot: HashMap<usize, usize> = HashMap::new();
        let groups: Vec<usize> = match &self.converter {
            None => vec![0; n],
            Some(converter) => {
                let values: Vec<ArrayRef> = (self.groups.iter())
                    .map(|g| g.evaluate(rows).and_then(|v| v.into_array(n)))
                    .collect::<Result<_>>()?;
                let converted = converter.convert_columns(&values)?;
                let mut groups = Vec::with_capacity(n);
                for row in converted.iter() {
                    let index = match self.index.get(&row.owned()) {
                        Some(index) => *index,
                        None => {
                            changed = true;
                            self.add_group(Some(row.owned()))?
                        }
                    };
                    groups.push(index);
                }
                groups
            }
        };
        for (row, group) in groups.into_iter().enumerate() {
            let at = *slot.entry(group).or_insert_with(|| {
                by_group.push((group, Vec::new()));
                by_group.len() - 1
            });
            by_group[at].1.push(row as u32);
        }
        for (a, aggregate) in self.aggregates.iter().enumerate() {
            let inputs: Vec<ArrayRef> = (aggregate.inputs.iter())
                .map(|e| e.evaluate(rows).and_then(|v| v.into_array(n)))
                .collect::<Result<_>>()?;
            let keep = match &aggregate.filter {
                Some(filter) => Some(filter.evaluate(rows)?.into_array(n)?),
                None => None,
            };
            let keep = keep.as_ref().map(|k| k.as_boolean());
            for (group, rows) in &by_group {
                let rows: Vec<u32> = match keep {
                    Some(keep) => (rows.iter().copied())
                        .filter(|&r| keep.is_valid(r as usize) && keep.value(r as usize))
                        .collect(),
                    None => rows.clone(),
                };
                if rows.is_empty() {
                    continue;
                }
                let indices = UInt32Array::from(rows);
                let columns: Vec<ArrayRef> = (inputs.iter())
                    .map(|column| take(column, &indices, None))
                    .collect::<std::result::Result<_, _>>()?;
                self.accumulators[*group][a].update_batch(&columns)?;
            }
        }
        for (group, _) in &by_group {
            let values: Vec<ScalarValue> = (self.accumulators[*group].iter_mut())
                .map(|a| a.evaluate())
                .collect::<Result<_>>()?;
            if values != self.values[*group] {
                self.values[*group] = values;
                changed = true;
            }
        }
        Ok(changed)
    }

    fn output(&self) -> Result<Vec<ArrayRef>> {
        let mut columns = match &self.converter {
            None => Vec::new(),
            Some(converter) => match self.keys.is_empty() {
                true => (self.group_types.iter()).map(new_empty_array).collect(),
                false => converter.convert_rows(self.keys.iter().map(|k| k.row()))?,
            },
        };
        for (a, aggregate) in self.aggregates.iter().enumerate() {
            let column = match self.values.is_empty() {
                true => new_empty_array(aggregate.aggregate.field().data_type()),
                false => ScalarValue::iter_to_array(self.values.iter().map(|v| v[a].clone()))?,
            };
            columns.push(column);
        }
        Ok(columns)
    }
}

pub(crate) struct LimitState {
    order: Vec<PhysicalSortExpr>,
    /// The number of rows to keep, the skip and the fetch.
    keep: usize,
    skip: usize,
    schema: SchemaRef,
    /// The first rows of the order so far.
    kept: Option<RecordBatch>,
}

impl LimitState {
    fn apply(&mut self, rows: &RecordBatch) -> Result<bool> {
        let candidates = match &self.kept {
            Some(kept) => concat_batches(&self.schema, [kept, rows])?,
            None => rows.clone(),
        };
        let n = candidates.num_rows();
        let keys: Vec<SortColumn> = (self.order.iter())
            .map(|key| {
                Ok(SortColumn {
                    values: key.expr.evaluate(&candidates)?.into_array(n)?,
                    options: Some(key.options),
                })
            })
            .collect::<Result<_>>()?;
        let indices = lexsort_to_indices(&keys, Some(self.keep))?;
        let kept = take_record_batch(&candidates, &indices)?;
        let changed = self.kept.as_ref() != Some(&kept);
        self.kept = Some(kept);
        Ok(changed)
    }

    fn output(&self) -> Result<Vec<ArrayRef>> {
        Ok(match &self.kept {
            Some(kept) => {
                let skip = self.skip.min(kept.num_rows());
                kept.slice(skip, kept.num_rows() - skip).columns().to_vec()
            }
            None => (self.schema.fields().iter())
                .map(|f| new_empty_array(f.data_type()))
                .collect(),
        })
    }
}

/// `expr` with each parameter as its value.
fn bind_params(expr: &Expr, params: &Option<ParamValues>) -> Result<Expr> {
    use datafusion::common::tree_node::{Transformed, TreeNode};
    let Some(params) = params else {
        return Ok(expr.clone());
    };
    expr.clone()
        .transform(|e| match &e {
            Expr::Placeholder(placeholder) => {
                let value = params.get_placeholders_with_values(&placeholder.id)?;
                Ok(Transformed::yes(Expr::Literal(value.value, value.metadata)))
            }
            _ => Ok(Transformed::no(e)),
        })
        .map(|t| t.data)
}
