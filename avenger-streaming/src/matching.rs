//! The front of `MATCH_RECOGNIZE`: each partition's rows after its settled frontier, which the
//! matcher matches again as rows arrive, the final matches' measures, and the open matches'.
//!
//! A frame's rows go to their partitions. For each partition that they reach, the front runs the
//! clause's conditions over the partition's tail, the rows from `back` rows before the frontier,
//! which holds every row that a condition or an attempt reads again, and runs the matcher from the
//! frontier. A match that starts before the new frontier is final: its measures join the final
//! rows once. The partition's other matches are open, and their measures replace the partition's
//! open rows. A row's position is its partition's count of rows before it, which is the batch
//! plan's `MATCH_ROW_NUMBER()`, since `seq` orders the rows.

use std::{collections::HashMap, sync::Arc};

use avenger_datafusion_transforms::match_recognize::{
    Match, MatchRecognizeCall, Program, Tail, tail_matches,
};
use avenger_sql::streaming::{MatchFront, Slot};
use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, AsArray, Int64Array, RecordBatch, StringArray, UInt32Array,
            new_empty_array,
        },
        compute::{SortOptions, concat_batches, sort_to_indices, take, take_record_batch},
        datatypes::SchemaRef,
        row::{OwnedRow, RowConverter, SortField},
    },
    common::{DFSchemaRef, ParamValues, Result},
    execution::context::ExecutionProps,
    logical_expr::{LogicalPlan, physical_planning_context::PhysicalPlanningContext},
    physical_expr::{PhysicalExpr, create_physical_expr},
    prelude::SessionContext,
};

use crate::{execute, front::bind_params};

/// A match and its number in its partition.
type Numbered<'m> = (u64, &'m Match);

pub(crate) struct MatchState {
    ctx: SessionContext,
    /// The conditions and the measures, bound to the parameters and optimized, and the slots of
    /// the rows that they read.
    conditions: LogicalPlan,
    staged: Slot,
    measures: LogicalPlan,
    node_rows: Slot,
    program: Program,
    call: MatchRecognizeCall,
    back: usize,
    ahead: usize,
    /// The stages' rows, and the place of `seq` in them.
    staged_schema: SchemaRef,
    seq: usize,
    /// The places, in the conditions' output, of the variables and the position, and of the
    /// columns that the node's rows carry.
    variables: Vec<usize>,
    position: usize,
    carried: Vec<usize>,
    partition_columns: Vec<usize>,
    /// The partition expressions over the stages' rows, and the converter of their values.
    partition: Vec<Arc<dyn PhysicalExpr>>,
    converter: Option<RowConverter>,
    index: HashMap<OwnedRow, usize>,
    partitions: Vec<Partition>,
    /// The measures of the final matches, the number of them that `take_new_finals` gave, and
    /// the measures' schema.
    finals: Vec<RecordBatch>,
    given: usize,
    output_schema: SchemaRef,
}

struct Partition {
    /// The rows from the partition's row `base` on.
    rows: RecordBatch,
    base: usize,
    /// The partition's rows so far, its settled frontier, and its final matches.
    total: usize,
    frontier: usize,
    finals: u64,
    /// The measures of the open matches.
    open: Vec<RecordBatch>,
}

impl MatchState {
    pub(crate) fn new(
        ctx: &SessionContext,
        front: &MatchFront,
        input: &DFSchemaRef,
        params: &Option<ParamValues>,
        props: &ExecutionProps,
    ) -> Result<MatchState> {
        let bind = |plan: &LogicalPlan| match params {
            Some(params) => plan.clone().with_param_values(params.clone()),
            None => Ok(plan.clone()),
        };
        let state = ctx.state();
        let conditions = state.optimize(&bind(&front.conditions)?)?;
        let measures = state.optimize(&bind(&front.measures)?)?;
        let planning = PhysicalPlanningContext::default();
        let partition: Vec<Arc<dyn PhysicalExpr>> = (front.partition.iter())
            .map(|e| create_physical_expr(&bind_params(e, params)?, input, props, &planning))
            .collect::<Result<_>>()?;
        let staged_schema: SchemaRef = Arc::new(input.as_arrow().clone());
        let converter = match partition.is_empty() {
            true => None,
            false => Some(RowConverter::new(
                (partition.iter())
                    .map(|p| p.data_type(&staged_schema).map(SortField::new))
                    .collect::<Result<_>>()?,
            )?),
        };
        // The conditions' output, as the node reads it, under the unoptimized names.
        let output = front.conditions.schema();
        let call = &front.call;
        let variables: Vec<usize> = (call.variables.iter())
            .map(|c| output.index_of_column(c))
            .collect::<Result<_>>()?;
        let carried = (0..output.fields().len())
            .filter(|i| !variables.contains(i))
            .collect();
        let partition_columns = (call.partition_by.iter())
            .map(|c| output.index_of_column(c))
            .collect::<Result<_>>()?;
        Ok(MatchState {
            ctx: ctx.clone(),
            program: Program::compile(&call.pattern)
                .map_err(datafusion::common::DataFusionError::Plan)?,
            staged: front.staged.clone(),
            node_rows: front.node_rows.clone(),
            back: front.back,
            ahead: front.ahead,
            seq: input.index_of_column(&front.seq)?,
            staged_schema,
            position: output.index_of_column(&call.position)?,
            variables,
            carried,
            partition_columns,
            partition,
            converter,
            index: HashMap::new(),
            partitions: Vec::new(),
            finals: Vec::new(),
            given: 0,
            output_schema: Arc::new(front.measures.schema().as_arrow().clone()),
            call: call.clone(),
            conditions,
            measures,
        })
    }

    /// Applies the stages' `rows`, and returns whether the measures changed.
    pub(crate) async fn apply(&mut self, rows: &RecordBatch) -> Result<bool> {
        let n = rows.num_rows();
        // The rows of each partition, in their order.
        let mut by_partition: Vec<(usize, Vec<u32>)> = Vec::new();
        let mut slot: HashMap<usize, usize> = HashMap::new();
        let partitions: Vec<usize> = match &self.converter {
            None => {
                if self.partitions.is_empty() {
                    self.add_partition();
                }
                vec![0; n]
            }
            Some(converter) => {
                let values: Vec<ArrayRef> = (self.partition.iter())
                    .map(|p| p.evaluate(rows).and_then(|v| v.into_array(n)))
                    .collect::<Result<_>>()?;
                let keys = converter.convert_columns(&values)?;
                let mut out = Vec::with_capacity(n);
                for key in keys.iter() {
                    let index = match self.index.get(&key.owned()) {
                        Some(index) => *index,
                        None => {
                            let index = self.add_partition();
                            self.index.insert(key.owned(), index);
                            index
                        }
                    };
                    out.push(index);
                }
                out
            }
        };
        for (row, partition) in partitions.into_iter().enumerate() {
            let at = *slot.entry(partition).or_insert_with(|| {
                by_partition.push((partition, Vec::new()));
                by_partition.len() - 1
            });
            by_partition[at].1.push(row as u32);
        }
        let mut changed = false;
        for (partition, indices) in by_partition {
            let new = take_record_batch(rows, &UInt32Array::from(indices))?;
            changed |= self.apply_partition(partition, new).await?;
        }
        Ok(changed)
    }

    fn add_partition(&mut self) -> usize {
        self.partitions.push(Partition {
            rows: RecordBatch::new_empty(self.staged_schema.clone()),
            base: 0,
            total: 0,
            frontier: 0,
            finals: 0,
            open: Vec::new(),
        });
        self.partitions.len() - 1
    }

    async fn apply_partition(&mut self, index: usize, new: RecordBatch) -> Result<bool> {
        let partition = &mut self.partitions[index];
        partition.rows = concat_batches(&self.staged_schema, [&partition.rows, &new])?;
        partition.total += new.num_rows();
        let (base, total, frontier) = (partition.base, partition.total, partition.frontier);
        // The conditions of the tail, in the order of `seq`.
        self.staged.set(partition.rows.columns().to_vec());
        let output = execute(&self.ctx, &self.conditions).await?;
        let output = concat_batches(
            &Arc::new(self.conditions.schema().as_arrow().clone()),
            &output,
        )?;
        let seq = output.column(self.seq_in_output()?).clone();
        let order = sort_to_indices(&seq, Some(SortOptions::default()), None)?;
        let output = take_record_batch(&output, &order)?;
        let flags: Vec<Vec<bool>> = (self.variables.iter())
            .map(|&v| {
                let column = output.column(v).as_boolean();
                (0..column.len())
                    .map(|i| column.is_valid(i) && column.value(i))
                    .collect()
            })
            .collect();
        let meets = |row: usize, variable: usize| flags[variable][row - base];
        let tail = Tail {
            start: frontier,
            partition_start: true,
            firm: total.saturating_sub(self.ahead),
        };
        let (found, settled) = tail_matches(
            &self.program,
            total,
            meets,
            self.call.skip,
            self.call.longest,
            tail,
        );
        // The measures of the final matches once, and of the open ones again.
        let finals = self.partitions[index].finals;
        let (closed, open): (Vec<Numbered>, Vec<Numbered>) = (found.iter().enumerate())
            .map(|(i, m)| (finals + i as u64 + 1, m))
            .partition(|(_, m)| m.start < settled);
        let closed_rows = self.measure(&output, base, &closed).await?;
        let open_rows = self.measure(&output, base, &open).await?;
        let partition = &mut self.partitions[index];
        let changed = !closed.is_empty() || open_rows != partition.open;
        partition.finals += closed.len() as u64;
        partition.open = open_rows;
        self.finals.extend(closed_rows);
        // The rows that no condition or attempt reads again.
        let keep_from = settled.saturating_sub(self.back).max(base);
        if keep_from > base {
            let skip = keep_from - base;
            partition.rows = partition.rows.slice(skip, partition.rows.num_rows() - skip);
            partition.base = keep_from;
        }
        partition.frontier = settled;
        Ok(changed)
    }

    /// The place of `seq` in the conditions' output, which carries the stages' columns.
    fn seq_in_output(&self) -> Result<usize> {
        let name = self.staged_schema.field(self.seq).name();
        let schema = self.conditions.schema();
        (0..schema.fields().len())
            .find(|&i| schema.field(i).name() == name)
            .ok_or_else(|| {
                datafusion::common::DataFusionError::Plan(format!(
                    "the conditions carry no `{name}`"
                ))
            })
    }

    /// The measures of `matches`, numbered, over the conditions' `output` of the partition's rows
    /// from `base` on: one node row for each row of each match, and one for each empty match.
    async fn measure(
        &self,
        output: &RecordBatch,
        base: usize,
        matches: &[Numbered<'_>],
    ) -> Result<Vec<RecordBatch>> {
        if matches.is_empty() {
            return Ok(Vec::new());
        }
        let mut sources: Vec<Option<u32>> = Vec::new();
        let mut carriers: Vec<u32> = Vec::new();
        let mut positions: Vec<Option<i64>> = Vec::new();
        let mut numbers: Vec<i64> = Vec::new();
        let mut classes: Vec<Option<&str>> = Vec::new();
        for (number, m) in matches {
            if m.variables.is_empty() {
                sources.push(None);
                carriers.push((m.start - base) as u32);
                positions.push(None);
                numbers.push(*number as i64);
                classes.push(None);
            }
            for (offset, variable) in m.variables.iter().enumerate() {
                let row = m.start + offset;
                sources.push(Some((row - base) as u32));
                carriers.push((row - base) as u32);
                positions.push(Some(row as i64 + 1));
                numbers.push(*number as i64);
                classes.push(Some(self.call.names[*variable].as_str()));
            }
        }
        let (sources, carriers) = (UInt32Array::from(sources), UInt32Array::from(carriers));
        let mut columns: Vec<ArrayRef> = Vec::new();
        for &i in &self.carried {
            let column = match (i == self.position, self.partition_columns.contains(&i)) {
                (true, _) => Arc::new(Int64Array::from(positions.clone())) as ArrayRef,
                (false, true) => take(output.column(i), &carriers, None)?,
                (false, false) => take(output.column(i), &sources, None)?,
            };
            columns.push(column);
        }
        columns.push(Arc::new(Int64Array::from(numbers)));
        columns.push(Arc::new(StringArray::from(classes)));
        self.node_rows.set(columns);
        execute(&self.ctx, &self.measures).await
    }

    /// The measures of the final matches since the last call.
    pub(crate) fn take_new_finals(&mut self) -> Vec<RecordBatch> {
        let new = self.finals[self.given..].to_vec();
        self.given = self.finals.len();
        new
    }

    /// Gives every final match's measures at the next `take_new_finals`.
    pub(crate) fn give_finals_again(&mut self) {
        self.given = 0;
    }

    /// The measures of the open matches of every partition.
    pub(crate) fn open_rows(&self) -> Vec<RecordBatch> {
        (self.partitions.iter())
            .flat_map(|p| p.open.iter().cloned())
            .collect()
    }

    /// The measures of every match: the final ones, then each partition's open ones.
    pub(crate) fn output(&self) -> Result<Vec<ArrayRef>> {
        let batches: Vec<&RecordBatch> = (self.finals.iter())
            .chain(self.partitions.iter().flat_map(|p| p.open.iter()))
            .collect();
        if batches.is_empty() {
            return Ok((self.output_schema.fields().iter())
                .map(|f| new_empty_array(f.data_type()))
                .collect());
        }
        let schema = batches[0].schema();
        Ok(concat_batches(&schema, batches)?.columns().to_vec())
    }
}
