//! Cache execution nodes: [`CacheReadExec`] and [`CacheWriteExec`].
//!
//! `CacheReadExec` replays a committed entry's partitions with the exact
//! `PlanProperties` captured from the subtree it replaced, and reports exact
//! statistics computed from the materialized data (strictly better than the
//! original subtree's estimates).
//!
//! `CacheWriteExec` wraps an admitted subtree: each partition's stream is
//! teed into the write handle's staging buffers while batches pass through
//! unchanged, and the entry commits atomically when every partition
//! completes. Any error, early termination (a stream dropped before its
//! end), oversize staging, or teardown before completion discards the staged
//! data — the cache fails open and the query is never affected.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use datafusion::execution::TaskContext;
use datafusion::physical_plan::memory::MemoryStream;
use datafusion::physical_plan::{
    DisplayAs, DisplayFormatType, ExecutionPlan, PlanProperties, RecordBatchStream,
    SendableRecordBatchStream,
};
use datafusion_common::{Result, Statistics, internal_err};
use futures::Stream;
use futures::StreamExt;

use crate::store::{CacheEntryRef, CacheWriteHandle, CommittedEntry};

/// Replays a committed cache entry.
///
/// Leaf node: no children. Reports the `PlanProperties` captured from the
/// subtree the entry replaced, so substitution is property-compatible by
/// construction, and exact statistics computed from the cached data.
pub struct CacheReadExec {
    entry: Arc<CommittedEntry>,
}

impl CacheReadExec {
    /// Build a read node over a committed entry.
    pub fn new(entry: &CacheEntryRef) -> Self {
        Self {
            entry: Arc::clone(&entry.entry),
        }
    }
}

impl std::fmt::Debug for CacheReadExec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheReadExec")
            .field("rows", &self.entry.actual_rows)
            .field("partitions", &self.entry.partitions.len())
            .finish()
    }
}

impl DisplayAs for CacheReadExec {
    fn fmt_as(&self, _t: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CacheReadExec: key={:016x}, partitions={}, rows={}",
            (self.entry.key.subtree_fingerprint.0 >> 64) as u64,
            self.entry.partitions.len(),
            self.entry.actual_rows,
        )
    }
}

impl ExecutionPlan for CacheReadExec {
    fn name(&self) -> &str {
        "CacheReadExec"
    }

    fn properties(&self) -> &Arc<PlanProperties> {
        &self.entry.properties
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![]
    }

    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if children.is_empty() {
            Ok(self)
        } else {
            internal_err!("CacheReadExec is a leaf and accepts no children")
        }
    }

    fn execute(
        &self,
        partition: usize,
        _context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        let Some(batches) = self.entry.partitions.get(partition) else {
            return internal_err!(
                "CacheReadExec partition {partition} out of range ({} cached partitions)",
                self.entry.partitions.len()
            );
        };
        Ok(Box::pin(MemoryStream::try_new(
            batches.clone(),
            Arc::clone(&self.entry.schema),
            None,
        )?))
    }

    fn partition_statistics(&self, partition: Option<usize>) -> Result<Arc<Statistics>> {
        match partition {
            Some(partition) => match self.entry.partition_statistics.get(partition) {
                Some(statistics) => Ok(Arc::clone(statistics)),
                None => internal_err!("CacheReadExec partition {partition} out of range"),
            },
            None => Ok(Arc::clone(&self.entry.statistics)),
        }
    }
}

/// Tees an admitted subtree's output into the cache while passing batches
/// through unchanged.
///
/// Wraps exactly one child. Output properties are the child's own; the node
/// is semantically transparent. The shared [`CacheWriteHandle`] commits the
/// entry atomically when every partition's stream completes; every other
/// outcome discards (fail-open).
pub struct CacheWriteExec {
    child: Arc<dyn ExecutionPlan>,
    handle: CacheWriteHandle,
}

impl CacheWriteExec {
    /// Wrap `child` so its execution populates the cache through `handle`.
    pub fn new(child: Arc<dyn ExecutionPlan>, handle: CacheWriteHandle) -> Self {
        Self { child, handle }
    }
}

impl std::fmt::Debug for CacheWriteExec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheWriteExec")
            .field("child", &self.child)
            .finish()
    }
}

impl DisplayAs for CacheWriteExec {
    fn fmt_as(&self, _t: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CacheWriteExec: key={:016x}",
            (self.handle.key().subtree_fingerprint.0 >> 64) as u64
        )
    }
}

impl ExecutionPlan for CacheWriteExec {
    fn name(&self) -> &str {
        "CacheWriteExec"
    }

    fn properties(&self) -> &Arc<PlanProperties> {
        self.child.properties()
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![&self.child]
    }

    fn with_new_children(
        self: Arc<Self>,
        mut children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if children.len() != 1 {
            return internal_err!("CacheWriteExec wraps exactly one child");
        }
        Ok(Arc::new(CacheWriteExec::new(
            children.swap_remove(0),
            self.handle.clone(),
        )))
    }

    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        let inner = self.child.execute(partition, context)?;
        Ok(Box::pin(TeeStream {
            schema: inner.schema(),
            inner,
            handle: self.handle.clone(),
            partition,
            first_polled: false,
        }))
    }

    fn partition_statistics(&self, partition: Option<usize>) -> Result<Arc<Statistics>> {
        self.child.partition_statistics(partition)
    }

    fn maintains_input_order(&self) -> Vec<bool> {
        vec![true]
    }
}

/// Stream adapter that stages each batch before yielding it.
///
/// - `Ok(batch)` → staged (an `Arc`-cheap clone), then yielded unchanged;
/// - `Err` → the whole entry is discarded, the error propagates unchanged;
/// - end of stream → the partition is marked complete (commit fires on the
///   last one);
/// - dropped before the end → the partition simply stays incomplete; the
///   entry discards when the write handle itself drops without completion.
struct TeeStream {
    inner: SendableRecordBatchStream,
    schema: SchemaRef,
    handle: CacheWriteHandle,
    partition: usize,
    first_polled: bool,
}

impl Stream for TeeStream {
    type Item = Result<RecordBatch>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if !self.first_polled {
            self.first_polled = true;
            self.handle.note_first_poll();
        }
        match self.inner.poll_next_unpin(cx) {
            Poll::Ready(Some(Ok(batch))) => {
                self.handle.stage(self.partition, batch.clone());
                Poll::Ready(Some(Ok(batch)))
            }
            Poll::Ready(Some(Err(err))) => {
                self.handle.fail(self.partition);
                Poll::Ready(Some(Err(err)))
            }
            Poll::Ready(None) => {
                self.handle.complete_partition(self.partition);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl RecordBatchStream for TeeStream {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EvaluationCacheConfig;
    use crate::fingerprint::{PhysicalCacheKey, PlanFingerprint};
    use crate::store::{CacheCandidate, CacheLookup, EvaluationCache};

    use arrow::array::Int64Array;
    use arrow::datatypes::{DataType, Field, Schema};
    use datafusion::datasource::memory::MemorySourceConfig;
    use datafusion::physical_plan::limit::GlobalLimitExec;
    use datafusion::physical_plan::stream::RecordBatchStreamAdapter;
    use datafusion::physical_plan::{collect, execute_stream_partitioned};
    use datafusion_common::stats::Precision;

    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![Field::new("v", DataType::Int64, false)]))
    }

    fn batch(values: &[i64]) -> RecordBatch {
        RecordBatch::try_new(schema(), vec![Arc::new(Int64Array::from(values.to_vec()))]).unwrap()
    }

    fn memory_exec(partitions: &[Vec<RecordBatch>]) -> Arc<dyn ExecutionPlan> {
        MemorySourceConfig::try_new_exec(partitions, schema(), None).unwrap()
    }

    fn key(n: u128) -> PhysicalCacheKey {
        PhysicalCacheKey {
            subtree_fingerprint: PlanFingerprint(n),
            schema_fingerprint: 1,
            partitioning_fingerprint: 2,
            ordering_fingerprint: 3,
            bounded: true,
        }
    }

    fn candidate_for(n: u128, plan: &Arc<dyn ExecutionPlan>) -> CacheCandidate {
        use datafusion::physical_plan::execution_plan::ExecutionPlanProperties;
        CacheCandidate {
            key: key(n),
            schema: plan.schema(),
            properties: Arc::clone(plan.properties()),
            partition_count: plan.output_partitioning().partition_count(),
            estimated_bytes: None,
        }
    }

    fn task_ctx() -> Arc<TaskContext> {
        Arc::new(TaskContext::default())
    }

    async fn drain_all(plan: &Arc<dyn ExecutionPlan>) -> Vec<Vec<RecordBatch>> {
        let streams = execute_stream_partitioned(Arc::clone(plan), task_ctx()).unwrap();
        let mut partitions = Vec::new();
        for mut stream in streams {
            let mut batches = Vec::new();
            while let Some(item) = stream.next().await {
                batches.push(item.unwrap());
            }
            partitions.push(batches);
        }
        partitions
    }

    #[tokio::test]
    async fn write_then_read_roundtrip() {
        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let child = memory_exec(&[vec![batch(&[1, 2, 3]), batch(&[4, 5])]]);
        let handle = cache.begin_write(candidate_for(1, &child)).unwrap();
        let write: Arc<dyn ExecutionPlan> =
            Arc::new(CacheWriteExec::new(Arc::clone(&child), handle));

        let drained = drain_all(&write).await;
        assert_eq!(cache.metrics().committed_writes, 1);

        let CacheLookup::Ready(entry) = cache.lookup(&key(1)) else {
            panic!("expected committed entry");
        };
        assert_eq!(
            entry.entry.partitions, drained,
            "staged batches equal yielded batches"
        );

        let read: Arc<dyn ExecutionPlan> = Arc::new(CacheReadExec::new(&entry));
        assert_eq!(
            format!("{:?}", read.properties()),
            format!("{:?}", child.properties()),
            "read node must report the captured properties"
        );
        let replayed = drain_all(&read).await;
        assert_eq!(replayed, drained, "replay must be byte-identical");
    }

    #[tokio::test]
    async fn partition_count_preserved() {
        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let child = memory_exec(&[
            vec![batch(&[1])],
            vec![batch(&[2, 3])],
            vec![batch(&[4, 5, 6])],
        ]);
        let handle = cache.begin_write(candidate_for(1, &child)).unwrap();
        let write: Arc<dyn ExecutionPlan> = Arc::new(CacheWriteExec::new(child, handle));
        let drained = drain_all(&write).await;
        assert_eq!(drained.len(), 3);

        let CacheLookup::Ready(entry) = cache.lookup(&key(1)) else {
            panic!("expected committed entry");
        };
        assert_eq!(entry.partition_count(), 3);
        let read: Arc<dyn ExecutionPlan> = Arc::new(CacheReadExec::new(&entry));
        use datafusion::physical_plan::execution_plan::ExecutionPlanProperties;
        assert_eq!(read.output_partitioning().partition_count(), 3);
        assert_eq!(drain_all(&read).await, drained);
    }

    #[tokio::test]
    async fn early_termination_discards_partial_result() {
        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let child = memory_exec(&[vec![batch(&[1, 2, 3]), batch(&[4, 5, 6])]]);
        let handle = cache.begin_write(candidate_for(1, &child)).unwrap();
        let write: Arc<dyn ExecutionPlan> = Arc::new(CacheWriteExec::new(child, handle));
        let limited: Arc<dyn ExecutionPlan> = Arc::new(GlobalLimitExec::new(write, 0, Some(2)));

        let results = collect(Arc::clone(&limited), task_ctx()).await.unwrap();
        let rows: usize = results.iter().map(|b| b.num_rows()).sum();
        assert_eq!(rows, 2, "limit output correct");

        drop(results);
        drop(limited); // last handle reference drops with the plan
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Miss));
        let metrics = cache.metrics();
        assert_eq!(metrics.committed_writes, 0);
        assert_eq!(metrics.discarded_writes, 1);
    }

    #[tokio::test]
    async fn child_error_discards_and_propagates() {
        /// One good batch, then an error.
        #[derive(Debug)]
        struct FailingExec {
            properties: Arc<PlanProperties>,
            schema: SchemaRef,
        }
        impl DisplayAs for FailingExec {
            fn fmt_as(
                &self,
                _t: DisplayFormatType,
                f: &mut std::fmt::Formatter<'_>,
            ) -> std::fmt::Result {
                f.write_str("FailingExec")
            }
        }
        impl ExecutionPlan for FailingExec {
            fn name(&self) -> &str {
                "FailingExec"
            }
            fn properties(&self) -> &Arc<PlanProperties> {
                &self.properties
            }
            fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
                vec![]
            }
            fn with_new_children(
                self: Arc<Self>,
                _children: Vec<Arc<dyn ExecutionPlan>>,
            ) -> Result<Arc<dyn ExecutionPlan>> {
                Ok(self)
            }
            fn execute(
                &self,
                _partition: usize,
                _context: Arc<TaskContext>,
            ) -> Result<SendableRecordBatchStream> {
                let items: Vec<Result<RecordBatch>> = vec![
                    Ok(batch(&[1, 2])),
                    Err(datafusion_common::DataFusionError::Execution(
                        "synthetic mid-stream failure".to_string(),
                    )),
                ];
                Ok(Box::pin(RecordBatchStreamAdapter::new(
                    Arc::clone(&self.schema),
                    futures::stream::iter(items),
                )))
            }
        }

        let template = memory_exec(&[vec![batch(&[1])]]);
        let failing: Arc<dyn ExecutionPlan> = Arc::new(FailingExec {
            properties: Arc::clone(template.properties()),
            schema: schema(),
        });

        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let handle = cache.begin_write(candidate_for(1, &failing)).unwrap();
        let write: Arc<dyn ExecutionPlan> = Arc::new(CacheWriteExec::new(failing, handle));

        let result = collect(write, task_ctx()).await;
        let err = result.expect_err("child error must propagate");
        assert!(
            err.to_string().contains("synthetic mid-stream failure"),
            "error propagates unchanged, got: {err}"
        );
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Miss));
        assert_eq!(cache.metrics().discarded_writes, 1);
        assert_eq!(cache.metrics().committed_writes, 0);
    }

    #[tokio::test]
    async fn never_executed_partition_never_commits() {
        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let child = memory_exec(&[vec![batch(&[1])], vec![batch(&[2])]]);
        let handle = cache.begin_write(candidate_for(1, &child)).unwrap();
        let write: Arc<dyn ExecutionPlan> = Arc::new(CacheWriteExec::new(child, handle));

        // Execute and drain only partition 0 of 2.
        let mut stream = write.execute(0, task_ctx()).unwrap();
        while let Some(item) = stream.next().await {
            item.unwrap();
        }
        drop(stream);
        assert_eq!(
            cache.metrics().committed_writes,
            0,
            "half-executed: no commit"
        );

        drop(write);
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Miss));
        assert_eq!(cache.metrics().discarded_writes, 1);
    }

    #[tokio::test]
    async fn re_executed_partition_commits_once_with_last_write() {
        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let child = memory_exec(&[vec![batch(&[1, 2, 3])], vec![batch(&[9])]]);
        let handle = cache.begin_write(candidate_for(1, &child)).unwrap();
        let write: Arc<dyn ExecutionPlan> = Arc::new(CacheWriteExec::new(child, handle));

        for _ in 0..2 {
            let mut stream = write.execute(0, task_ctx()).unwrap();
            while let Some(item) = stream.next().await {
                item.unwrap();
            }
        }
        let mut stream = write.execute(1, task_ctx()).unwrap();
        while let Some(item) = stream.next().await {
            item.unwrap();
        }

        assert_eq!(cache.metrics().committed_writes, 1, "commits exactly once");
        let CacheLookup::Ready(entry) = cache.lookup(&key(1)) else {
            panic!("expected committed entry");
        };
        assert_eq!(
            entry.rows(),
            4,
            "3 rows (last write of partition 0) + 1 row"
        );
    }

    #[tokio::test]
    async fn oversize_entry_fails_open() {
        let cache = EvaluationCache::new(EvaluationCacheConfig {
            max_entry_bytes: 8,
            ..Default::default()
        });
        let child = memory_exec(&[vec![batch(&[1, 2, 3]), batch(&[4, 5, 6])]]);
        let handle = cache.begin_write(candidate_for(1, &child)).unwrap();
        let write: Arc<dyn ExecutionPlan> = Arc::new(CacheWriteExec::new(child, handle));

        let results = collect(write, task_ctx()).await.unwrap();
        let rows: usize = results.iter().map(|b| b.num_rows()).sum();
        assert_eq!(rows, 6, "query output complete and correct despite abort");
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Miss));
        assert_eq!(cache.metrics().committed_writes, 0);
        assert_eq!(cache.metrics().discarded_writes, 1);
    }

    #[tokio::test]
    async fn committed_statistics_are_exact() {
        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let child = memory_exec(&[vec![batch(&[1, 2, 3])], vec![batch(&[4, 5])]]);
        let handle = cache.begin_write(candidate_for(1, &child)).unwrap();
        let write: Arc<dyn ExecutionPlan> = Arc::new(CacheWriteExec::new(child, handle));
        drain_all(&write).await;

        let CacheLookup::Ready(entry) = cache.lookup(&key(1)) else {
            panic!("expected committed entry");
        };
        let read: Arc<dyn ExecutionPlan> = Arc::new(CacheReadExec::new(&entry));

        let totals = read.partition_statistics(None).unwrap();
        assert_eq!(totals.num_rows, Precision::Exact(5));
        assert!(matches!(totals.total_byte_size, Precision::Exact(_)));

        let p0 = read.partition_statistics(Some(0)).unwrap();
        assert_eq!(p0.num_rows, Precision::Exact(3));
        let p1 = read.partition_statistics(Some(1)).unwrap();
        assert_eq!(p1.num_rows, Precision::Exact(2));

        assert!(read.partition_statistics(Some(9)).is_err());
    }
}
