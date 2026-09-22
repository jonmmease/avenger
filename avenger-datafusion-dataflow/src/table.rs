use std::sync::{Arc, RwLock};

use datafusion::arrow::{datatypes::SchemaRef, error::ArrowError, record_batch::RecordBatch};

use crate::{fresh_id, Error, Result};

/// Process-local identity of one immutable table snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SnapshotId(u64);

/// A complete finite table. Clones share batches and preserve snapshot identity.
#[derive(Clone)]
pub struct TableSnapshot {
    id: SnapshotId,
    schema: SchemaRef,
    batches: Arc<[RecordBatch]>,
    num_rows: usize,
}

impl std::fmt::Debug for TableSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableSnapshot")
            .field("id", &self.id)
            .field("schema", &self.schema)
            .field("batches", &self.batches.len())
            .field("num_rows", &self.num_rows)
            .finish()
    }
}

impl TableSnapshot {
    /// Validate batch schemas and row-count arithmetic, then allocate a new identity.
    pub fn from_batches(schema: SchemaRef, batches: Vec<RecordBatch>) -> Result<Self> {
        if batches.iter().any(|batch| batch.schema() != schema) {
            return Err(Error::SchemaMismatch("table snapshot".into()));
        }
        let num_rows = checked_row_count(0, &batches)?;
        Ok(Self {
            id: SnapshotId(fresh_id()),
            schema,
            batches: batches.into(),
            num_rows,
        })
    }

    pub fn empty(schema: SchemaRef) -> Self {
        Self {
            id: SnapshotId(fresh_id()),
            schema,
            batches: Arc::from([]),
            num_rows: 0,
        }
    }

    pub fn id(&self) -> SnapshotId {
        self.id
    }
    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }
    pub fn batches(&self) -> &[RecordBatch] {
        &self.batches
    }

    /// Visit this snapshot's batches in order without copying their handles or data.
    ///
    /// Query results need not preserve the batch boundaries of their inputs.
    pub fn batch_iter(&self) -> impl Iterator<Item = &RecordBatch> + '_ {
        self.batches.iter()
    }

    pub fn num_rows(&self) -> usize {
        self.num_rows
    }
}

/// A shared owner for atomically publishing complete immutable table snapshots.
#[derive(Clone, Debug)]
pub struct TableStore {
    current: Arc<RwLock<TableSnapshot>>,
}

impl TableStore {
    pub fn new(initial: TableSnapshot) -> Self {
        Self {
            current: Arc::new(RwLock::new(initial)),
        }
    }

    pub fn snapshot(&self) -> TableSnapshot {
        self.current
            .read()
            .expect("table store lock poisoned")
            .clone()
    }

    /// Append one batch and return the complete snapshot committed by this call.
    ///
    /// Equivalent to [`Self::append_batches`] with a single batch, including its
    /// validation, empty-batch, and concurrency behavior.
    pub fn append_batch(&self, batch: RecordBatch) -> Result<TableSnapshot> {
        self.append_batches(vec![batch])
    }

    /// Atomically append batches in order and return this call's committed snapshot.
    ///
    /// Every schema must match exactly, including for zero-row batches. Schema
    /// mismatch or row-count overflow returns an error without changing the store.
    /// Valid zero-row batches are discarded. An empty commit returns the current
    /// snapshot without changing its identity. Other commits receive a fresh identity.
    ///
    /// Concurrent appends and replacements serialize against the current snapshot.
    /// Captured snapshots and input bindings remain unchanged. Appending the same
    /// batch again adds its rows again. No dataflow query runs during append.
    ///
    /// Arrow buffers are shared. Each nonempty commit clones the existing batch
    /// handles, so metadata work grows with the number of retained batches.
    pub fn append_batches(&self, batches: Vec<RecordBatch>) -> Result<TableSnapshot> {
        let mut current = self.current.write().expect("table store lock poisoned");
        if batches.iter().any(|batch| batch.schema() != current.schema) {
            return Err(Error::SchemaMismatch("table store append".into()));
        }
        let num_rows = checked_row_count(current.num_rows, &batches)?;
        if num_rows == current.num_rows {
            return Ok(current.clone());
        }
        let batches = current
            .batch_iter()
            .cloned()
            .chain(batches.into_iter().filter(|batch| batch.num_rows() != 0))
            .collect::<Vec<_>>();
        let snapshot = TableSnapshot {
            id: SnapshotId(fresh_id()),
            schema: current.schema.clone(),
            batches: batches.into(),
            num_rows,
        };
        *current = snapshot.clone();
        Ok(snapshot)
    }

    pub fn replace(&self, snapshot: TableSnapshot) -> Result<()> {
        let mut current = self.current.write().expect("table store lock poisoned");
        if current.schema() != snapshot.schema() {
            return Err(Error::SchemaMismatch("table store replacement".into()));
        }
        *current = snapshot;
        Ok(())
    }
}

fn checked_row_count(initial: usize, batches: &[RecordBatch]) -> Result<usize> {
    batches.iter().try_fold(initial, |rows, batch| {
        rows.checked_add(batch.num_rows()).ok_or_else(|| {
            ArrowError::InvalidArgumentError("table row count exceeds usize::MAX".into()).into()
        })
    })
}
