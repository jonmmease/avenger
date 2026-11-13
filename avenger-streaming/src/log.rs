//! The append-only log of a chart's events, which is also the table `events`.

use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
};

use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array, RecordBatch},
        compute::concat_batches,
        datatypes::{DataType, Field, Schema, SchemaRef},
    },
    catalog::TableProvider,
    common::{Result, plan_err},
    datasource::MemTable,
};

/// The name of the column of an event's place in the log.
pub const SEQ: &str = "seq";

/// The append-only log of a chart's events. It numbers each event in the order of arrival, in the
/// column `seq` from 1, so the order of the events is a column that queries read and that has no
/// ties. The log is also the table `events`: [`EventLog::table`] goes in avenger-sql's catalog, so
/// batch plans read it and [`avenger_sql::streaming::split`] recognizes it.
#[derive(Clone)]
pub struct EventLog {
    inner: Arc<Inner>,
}

struct Inner {
    /// The columns of an appended event, without `seq`.
    input: SchemaRef,
    /// The columns of the table: `seq`, then the input's.
    schema: SchemaRef,
    table: Arc<MemTable>,
    provider: Arc<dyn TableProvider>,
    /// The `seq` of the next event, and the first of the small batches at the end of the
    /// table, which change while the table's lock is held.
    next: AtomicI64,
    small_from: AtomicUsize,
}

/// The rows of a batch of the table, at least, once the log merges small batches.
const MERGED_ROWS: usize = 8_192;

/// The most small batches at the end of the table, which the log merges into one.
const SMALL_BATCHES: usize = 64;

impl EventLog {
    /// A log of events with the columns of `schema`, which has no column `seq`.
    pub fn new(schema: SchemaRef) -> Result<EventLog> {
        if schema.field_with_name(SEQ).is_ok() {
            return plan_err!("the log numbers its events in `{SEQ}`, which the events can't have");
        }
        let mut fields = vec![Arc::new(Field::new(SEQ, DataType::Int64, false))];
        fields.extend(schema.fields().iter().cloned());
        let table_schema = Arc::new(Schema::new(fields));
        let table = Arc::new(MemTable::try_new(table_schema.clone(), vec![vec![]])?);
        let provider: Arc<dyn TableProvider> = table.clone();
        Ok(EventLog {
            inner: Arc::new(Inner {
                input: schema,
                schema: table_schema,
                table,
                provider,
                next: AtomicI64::new(1),
                small_from: AtomicUsize::new(0),
            }),
        })
    }

    /// The columns of an event that [`EventLog::append`] takes, without `seq`.
    pub fn input_schema(&self) -> SchemaRef {
        self.inner.input.clone()
    }

    /// The columns of the table `events`: `seq`, then the columns of an event.
    pub fn schema(&self) -> SchemaRef {
        self.inner.schema.clone()
    }

    /// The table of every event so far, for avenger-sql's catalog as `events`.
    pub fn table(&self) -> Arc<dyn TableProvider> {
        self.inner.provider.clone()
    }

    /// The number of events so far.
    pub fn len(&self) -> usize {
        (self.inner.next.load(Ordering::SeqCst) - 1) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Appends a frame's events, in the order of arrival, with the columns of
    /// [`EventLog::input_schema`], and numbers them from the next `seq`. Returns their `seq`.
    pub async fn append(&self, events: RecordBatch) -> Result<Range<i64>> {
        let input = &self.inner.input;
        let fits = events.num_columns() == input.fields().len()
            && (events.schema().fields().iter().zip(input.fields())).all(|(found, want)| {
                found.name() == want.name() && found.data_type() == want.data_type()
            });
        if !fits {
            return plan_err!(
                "the events have the columns {}, where the log takes {}",
                events.schema(),
                input
            );
        }
        let mut batches = self.inner.table.batches[0].write().await;
        let first = self.inner.next.load(Ordering::SeqCst);
        let count = events.num_rows() as i64;
        if count == 0 {
            return Ok(first..first);
        }
        let seq: ArrayRef = Arc::new(Int64Array::from_iter_values(first..first + count));
        let mut columns = vec![seq];
        columns.extend(events.columns().iter().cloned());
        // The table's schema checks that a column that can't be NULL has no NULL.
        let batch = RecordBatch::try_new(self.inner.schema.clone(), columns)?;
        batches.push(batch);
        // Each frame appends a few events, and a plan reads many small batches slowly, so the
        // log merges the small batches at the end of the table into one.
        let small_from = self.inner.small_from.load(Ordering::SeqCst);
        let small_rows: usize = batches[small_from..].iter().map(|b| b.num_rows()).sum();
        if small_rows >= MERGED_ROWS || batches.len() - small_from >= SMALL_BATCHES {
            let merged = concat_batches(&self.inner.schema, &batches[small_from..])?;
            batches.truncate(small_from);
            batches.push(merged);
            if small_rows >= MERGED_ROWS {
                self.inner.small_from.store(batches.len(), Ordering::SeqCst);
            }
        }
        self.inner.next.store(first + count, Ordering::SeqCst);
        Ok(first..first + count)
    }

    /// The events after the first `from`, in batches.
    pub(crate) async fn events_after(&self, from: usize) -> Vec<RecordBatch> {
        let batches = self.inner.table.batches[0].read().await;
        let mut out = Vec::new();
        let mut at = 0;
        for batch in batches.iter() {
            let rows = batch.num_rows();
            if at + rows > from {
                let skip = from.saturating_sub(at);
                out.push(batch.slice(skip, rows - skip));
            }
            at += rows;
        }
        out
    }
}
