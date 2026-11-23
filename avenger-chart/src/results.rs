//! The result of a query that holds charts: its batches, the columns that hold charts, decoded,
//! and its shape, a single chart or a table, which every host that shows results, an image, a
//! window, or an editor, shows alike.

use std::collections::BTreeMap;

use crate::{charts_from_arrow, Chart, ChartError, Decode, EXTENSION_NAME};
use arrow::{
    array::{Array, RecordBatch},
    datatypes::{Field, SchemaRef},
    util::display::array_value_to_string,
};

/// How a host shows a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// One row and one column, which holds a chart: the chart alone.
    Chart,
    /// Every other result: a table with zero or more chart columns.
    Table,
}

/// The batches of a result, the running total of their rows, and the decoded charts of its chart
/// columns.
pub struct QueryResult {
    schema: SchemaRef,
    batches: Vec<RecordBatch>,
    /// The rows before each batch, and last, the rows of the result.
    offsets: Vec<usize>,
    /// The charts of each chart column, by column index, one for each row.
    charts: BTreeMap<usize, Vec<Option<Chart>>>,
}

/// Whether `field` holds charts: it has the extension name `avenger.chart`.
pub fn is_chart_field(field: &Field) -> bool {
    field
        .metadata()
        .get("ARROW:extension:name")
        .is_some_and(|name| name == EXTENSION_NAME)
}

impl QueryResult {
    /// The result of `batches`, whose schema is `schema`. Batches without rows are dropped. It
    /// fails when a chart column holds bytes that are not a chart.
    pub fn new(schema: SchemaRef, batches: Vec<RecordBatch>) -> Result<Self, ChartError> {
        let batches: Vec<RecordBatch> = batches
            .into_iter()
            .filter(|batch| batch.num_rows() > 0)
            .collect();
        let mut offsets = Vec::with_capacity(batches.len() + 1);
        let mut total = 0;
        for batch in &batches {
            offsets.push(total);
            total += batch.num_rows();
        }
        offsets.push(total);
        let mut charts = BTreeMap::new();
        for (index, field) in schema.fields().iter().enumerate() {
            if !is_chart_field(field) {
                continue;
            }
            let mut column = Vec::with_capacity(total);
            for batch in &batches {
                column.extend(charts_from_arrow(
                    batch.column(index).as_ref(),
                    Decode::Auto,
                )?);
            }
            charts.insert(index, column);
        }
        Ok(Self {
            schema,
            batches,
            offsets,
            charts,
        })
    }

    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    pub fn num_rows(&self) -> usize {
        *self.offsets.last().expect("the offsets end with the rows")
    }

    /// The indices of the chart columns, in order.
    pub fn chart_columns(&self) -> Vec<usize> {
        self.charts.keys().copied().collect()
    }

    pub fn is_chart_column(&self, column: usize) -> bool {
        self.charts.contains_key(&column)
    }

    /// The shape of the result: a chart when it has one row and one column, which is a chart
    /// column whose row is not null, and a table otherwise.
    pub fn shape(&self) -> Shape {
        let single = self.num_rows() == 1 && self.schema.fields().len() == 1;
        if single && self.chart(0, 0).is_some() {
            Shape::Chart
        } else {
            Shape::Table
        }
    }

    /// The batch that holds `row`, and the row's offset in it.
    pub fn locate(&self, row: usize) -> Option<(&RecordBatch, usize)> {
        if row >= self.num_rows() {
            return None;
        }
        // The last offset that is at most `row` starts the batch of `row`.
        let batch = self.offsets.partition_point(|&start| start <= row) - 1;
        Some((&self.batches[batch], row - self.offsets[batch]))
    }

    /// The text of the value of `row` in `column`, or `None` when the value is null.
    pub fn value_text(&self, row: usize, column: usize) -> Option<String> {
        let (batch, offset) = self.locate(row)?;
        let array = batch.column(column);
        if array.is_null(offset) {
            return None;
        }
        Some(array_value_to_string(array, offset).unwrap_or_else(|error| error.to_string()))
    }

    /// The chart of `row` in the chart column `column`, unless the row is null.
    pub fn chart(&self, row: usize, column: usize) -> Option<&Chart> {
        self.charts.get(&column)?.get(row)?.as_ref()
    }
}
