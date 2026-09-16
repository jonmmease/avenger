use std::sync::{Arc, RwLock};

use datafusion::arrow::{datatypes::SchemaRef, record_batch::RecordBatch};

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
    /// Validate batch schemas and allocate a new identity, without hashing data.
    pub fn from_batches(schema: SchemaRef, batches: Vec<RecordBatch>) -> Result<Self> {
        if batches.iter().any(|batch| batch.schema() != schema) {
            return Err(Error::SchemaMismatch("table snapshot".into()));
        }
        let num_rows = batches.iter().map(RecordBatch::num_rows).sum();
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
    pub fn num_rows(&self) -> usize {
        self.num_rows
    }
}

/// An optional shared owner for atomically publishing replacement snapshots.
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

    pub fn replace(&self, snapshot: TableSnapshot) -> Result<()> {
        let mut current = self.current.write().expect("table store lock poisoned");
        if current.schema() != snapshot.schema() {
            return Err(Error::SchemaMismatch("table store replacement".into()));
        }
        *current = snapshot;
        Ok(())
    }
}
