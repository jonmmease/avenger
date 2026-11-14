//! The node of `MATCH_RECOGNIZE` in a logical plan: a [`TableTransform`] of
//! [`TableFunctionNode`], which [`crate::table_function::TableFunctionExec`] runs on each
//! partition of its input.
//!
//! The node's input holds, for each row, its position in its partition, as a front end numbers
//! the rows by the clause's `ORDER BY`, and one boolean column for each pattern variable: whether
//! the row meets the variable's condition. The node orders each partition by the position, runs
//! the matcher, and outputs each row of each match, with the match's number and the row's
//! variable, and for an empty match, one row without a variable. An aggregate over the node's
//! output, grouped by the partition columns and the match number, computes the measures.

use std::fmt;
use std::sync::Arc;

use datafusion::arrow::array::{
    Array, ArrayRef, AsArray, BooleanArray, Int64Array, RecordBatch, StringArray, UInt32Array,
};
use datafusion::arrow::compute::{cast, sort_to_indices, take};
use datafusion::arrow::datatypes::{DataType, Field, SchemaRef};
use datafusion::common::{Column, DFSchema, DFSchemaRef, ExprSchema, Result, plan_err};
use datafusion::logical_expr::{Extension, LogicalPlan};

use super::matcher::{Skip, matches};
use super::pattern::{Pattern, Program};
use crate::table_function::{PartitionTransform, TableFunctionNode, TableTransform};

/// A call of `MATCH_RECOGNIZE` over an input, whose columns a front end computes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MatchRecognizeCall {
    /// The partition columns, `PARTITION BY`.
    pub partition_by: Vec<Column>,
    /// The position of each row in its partition, an integer, in the clause's order.
    pub position: Column,
    /// The boolean column of each pattern variable, in the order of the pattern's variables.
    pub variables: Vec<Column>,
    /// The name of each pattern variable, which the output's variable column holds.
    pub names: Vec<String>,
    pub pattern: Pattern,
    pub skip: Skip,
    /// `use_longest_match`.
    pub longest: bool,
    /// The names of the output's columns of the match number and the row's variable.
    pub match_number: String,
    pub classifier: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MatchRecognizeTransform {
    position: Column,
    variables: Vec<Column>,
    names: Vec<String>,
    pattern: Pattern,
    skip: Skip,
    longest: bool,
    match_number: String,
    classifier: String,
}

/// The plan of `call` over `input`: the node, whose output is the input's columns but the
/// variables' columns, the match number and the row's variable. An error where the pattern's
/// program is over the cap, or a column is missing or of another type.
pub fn plan(input: LogicalPlan, call: MatchRecognizeCall) -> Result<LogicalPlan> {
    Program::compile(&call.pattern).map_err(datafusion::common::DataFusionError::Plan)?;
    if call.names.len() != call.variables.len() {
        return plan_err!("MATCH_RECOGNIZE names each of its variables");
    }
    let schema = input.schema();
    if !schema
        .field_from_column(&call.position)?
        .data_type()
        .is_integer()
    {
        return plan_err!("the position column of MATCH_RECOGNIZE is an integer");
    }
    for variable in &call.variables {
        if schema.field_from_column(variable)?.data_type() != &DataType::Boolean {
            return plan_err!("the variable column {variable} of MATCH_RECOGNIZE is a BOOL");
        }
    }
    let transform = MatchRecognizeTransform {
        position: call.position,
        variables: call.variables,
        names: call.names,
        pattern: call.pattern,
        skip: call.skip,
        longest: call.longest,
        match_number: call.match_number,
        classifier: call.classifier,
    };
    let node = TableFunctionNode::try_new(input, call.partition_by, Arc::new(transform))?;
    Ok(LogicalPlan::Extension(Extension {
        node: Arc::new(node),
    }))
}

/// The call of a node of `MATCH_RECOGNIZE`, or `None` for another node, as a streaming runtime
/// reads it from a batch plan.
pub fn call_of(node: &TableFunctionNode) -> Option<MatchRecognizeCall> {
    let transform = node
        .transform
        .as_any()
        .downcast_ref::<MatchRecognizeTransform>()?;
    Some(MatchRecognizeCall {
        partition_by: node.partition_by.clone(),
        position: transform.position.clone(),
        variables: transform.variables.clone(),
        names: transform.names.clone(),
        pattern: transform.pattern.clone(),
        skip: transform.skip,
        longest: transform.longest,
        match_number: transform.match_number.clone(),
        classifier: transform.classifier.clone(),
    })
}

impl TableTransform for MatchRecognizeTransform {
    fn name(&self) -> &'static str {
        "MatchRecognize"
    }

    fn columns(&self) -> Vec<Column> {
        std::iter::once(self.position.clone())
            .chain(self.variables.iter().cloned())
            .collect()
    }

    fn with_columns(&self, columns: Vec<Column>) -> Result<Arc<dyn TableTransform>> {
        Ok(Arc::new(Self {
            position: columns[0].clone(),
            variables: columns[1..].to_vec(),
            ..self.clone()
        }))
    }

    fn schema(&self, input: &DFSchema, partition_by: &[Column]) -> Result<DFSchemaRef> {
        let variables = self.variable_indices(input)?;
        let partitions = (partition_by.iter())
            .map(|c| input.index_of_column(c))
            .collect::<Result<Vec<_>>>()?;
        // An empty match's row has no input row, so its columns but the partition's are NULL.
        let mut fields: Vec<_> = (input.iter().enumerate())
            .filter(|(i, _)| !variables.contains(i))
            .map(|(i, (qualifier, field))| {
                let field = match partitions.contains(&i) {
                    true => field.clone(),
                    false => Arc::new(field.as_ref().clone().with_nullable(true)),
                };
                (qualifier.cloned(), field)
            })
            .collect();
        fields.push((
            None,
            Arc::new(Field::new(&self.match_number, DataType::Int64, false)),
        ));
        fields.push((
            None,
            Arc::new(Field::new(&self.classifier, DataType::Utf8, true)),
        ));
        Ok(Arc::new(DFSchema::new_with_metadata(
            fields,
            input.metadata().clone(),
        )?))
    }

    fn partition_transform(&self, input: &DFSchema) -> Result<Arc<dyn PartitionTransform>> {
        let program =
            Program::compile(&self.pattern).map_err(datafusion::common::DataFusionError::Plan)?;
        Ok(Arc::new(PartitionMatch {
            position: input.index_of_column(&self.position)?,
            variables: self.variable_indices(input)?,
            names: self.names.clone(),
            program,
            skip: self.skip,
            longest: self.longest,
        }))
    }

    fn fmt_for_explain(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let variables: Vec<String> = (self.names.iter().zip(&self.variables))
            .map(|(name, column)| format!("{name}={column}"))
            .collect();
        write!(
            f,
            "pattern={:?}, variables=[{}], position={}, skip={:?}, longest={}",
            self.pattern,
            variables.join(", "),
            self.position,
            self.skip,
            self.longest
        )
    }
}

impl MatchRecognizeTransform {
    fn variable_indices(&self, input: &DFSchema) -> Result<Vec<usize>> {
        (self.variables.iter())
            .map(|c| input.index_of_column(c))
            .collect()
    }
}

/// The matches of one partition.
#[derive(Debug)]
struct PartitionMatch {
    position: usize,
    variables: Vec<usize>,
    names: Vec<String>,
    program: Program,
    skip: Skip,
    longest: bool,
}

impl PartitionTransform for PartitionMatch {
    fn transform(
        &self,
        input: &RecordBatch,
        partitions: &[usize],
        schema: SchemaRef,
    ) -> Result<RecordBatch> {
        // The rows in their order in the partition.
        let position = cast(input.column(self.position), &DataType::Int64)?;
        let order = sort_to_indices(&position, None, None)?;
        let rows = order.len();
        let flags: Vec<BooleanArray> = (self.variables.iter())
            .map(|&i| take(input.column(i), &order, None).map(|a| a.as_boolean().clone()))
            .collect::<std::result::Result<_, _>>()?;
        let meets = |row: usize, variable: usize| {
            let flag = &flags[variable];
            flag.is_valid(row) && flag.value(row)
        };
        let found = matches(&self.program, rows, meets, self.skip, self.longest);
        // The input row of each output row, or none for an empty match's row, and the row whose
        // partition values it carries.
        let mut sources: Vec<Option<u32>> = Vec::new();
        let mut carriers: Vec<u32> = Vec::new();
        let mut numbers: Vec<i64> = Vec::new();
        let mut classes: Vec<Option<&str>> = Vec::new();
        for m in &found {
            let start_row = order.value(m.start);
            if m.variables.is_empty() {
                sources.push(None);
                carriers.push(start_row);
                numbers.push(m.number as i64);
                classes.push(None);
            }
            for (offset, variable) in m.variables.iter().enumerate() {
                let row = order.value(m.start + offset);
                sources.push(Some(row));
                carriers.push(row);
                numbers.push(m.number as i64);
                classes.push(Some(&self.names[*variable]));
            }
        }
        let sources = UInt32Array::from(sources);
        let carriers = UInt32Array::from(carriers);
        let mut columns: Vec<ArrayRef> = Vec::with_capacity(schema.fields().len());
        for (i, column) in input.columns().iter().enumerate() {
            if self.variables.contains(&i) {
                continue;
            }
            let indices = match partitions.contains(&i) {
                true => &carriers,
                false => &sources,
            };
            columns.push(take(column, indices, None)?);
        }
        columns.push(Arc::new(Int64Array::from(numbers)));
        columns.push(Arc::new(StringArray::from(classes)));
        Ok(RecordBatch::try_new(schema, columns)?)
    }
}
