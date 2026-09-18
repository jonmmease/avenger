use std::{
    fmt,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    common::{Result, Statistics},
    execution::TaskContext,
    physical_plan::{
        DisplayAs, DisplayFormatType, ExecutionPlan, PlanProperties, RecordBatchStream,
        SendableRecordBatchStream,
    },
};
use futures::Stream;
use tokio::sync::oneshot;

use super::{PreparedInner, Reservation};

/// Plan and stream ownership survives an abort request to DataFusion's worker tasks.
/// The last owner releases the materialization charge and acknowledges teardown.
struct Lease {
    _budget: Arc<Reservation>,
    _prepared: Vec<Arc<PreparedInner>>,
    _completion: oneshot::Sender<()>,
}

pub(super) fn track(
    plan: Arc<dyn ExecutionPlan>,
    budget: Arc<Reservation>,
    prepared: Vec<Arc<PreparedInner>>,
) -> Result<(Arc<dyn ExecutionPlan>, oneshot::Receiver<()>)> {
    let (sender, receiver) = oneshot::channel();
    let lease = Arc::new(Lease {
        _budget: budget,
        _prepared: prepared,
        _completion: sender,
    });
    Ok((wrap(plan, &lease)?, receiver))
}

fn wrap(plan: Arc<dyn ExecutionPlan>, lease: &Arc<Lease>) -> Result<Arc<dyn ExecutionPlan>> {
    let children = plan.children();
    let plan = if children.is_empty() {
        plan
    } else {
        let children = children
            .into_iter()
            .map(|child| wrap(child.clone(), lease))
            .collect::<Result<_>>()?;
        plan.with_new_children(children)?
    };
    Ok(Arc::new(TrackedPlan {
        plan,
        lease: lease.clone(),
    }))
}

struct TrackedPlan {
    plan: Arc<dyn ExecutionPlan>,
    lease: Arc<Lease>,
}
impl fmt::Debug for TrackedPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.plan.fmt(f)
    }
}
impl DisplayAs for TrackedPlan {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.plan.fmt_as(t, f)
    }
}
impl ExecutionPlan for TrackedPlan {
    fn name(&self) -> &str {
        self.plan.name()
    }
    fn downcast_delegate(&self) -> Option<&dyn ExecutionPlan> {
        Some(self.plan.as_ref())
    }
    fn properties(&self) -> &Arc<PlanProperties> {
        self.plan.properties()
    }
    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        self.plan.children()
    }
    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        Ok(Arc::new(Self {
            plan: self.plan.clone().with_new_children(children)?,
            lease: self.lease.clone(),
        }))
    }
    fn metrics(&self) -> Option<datafusion::physical_plan::metrics::MetricsSet> {
        self.plan.metrics()
    }
    fn partition_statistics(&self, partition: Option<usize>) -> Result<Arc<Statistics>> {
        self.plan.partition_statistics(partition)
    }
    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        Ok(Box::pin(TrackedStream {
            stream: self.plan.execute(partition, context)?,
            _lease: self.lease.clone(),
        }))
    }
}

struct TrackedStream {
    // Drop the stream (and its abort-on-drop tasks) before releasing this lease.
    stream: SendableRecordBatchStream,
    _lease: Arc<Lease>,
}
impl Stream for TrackedStream {
    type Item = Result<RecordBatch>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.stream.as_mut().poll_next(cx)
    }
}
impl RecordBatchStream for TrackedStream {
    fn schema(&self) -> SchemaRef {
        self.stream.schema()
    }
}
