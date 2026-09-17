use async_trait::async_trait;
use avenger_datafusion_dataflow::{
    arrow::datatypes::SchemaRef,
    datafusion::{
        catalog::{Session, TableProvider},
        common::Result as DFResult,
        datasource::{provider_as_source, MemTable},
        logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder, TableType},
        physical_plan::ExecutionPlan,
    },
    Result, TableSnapshot,
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, RwLock,
};
use tokio::sync::Notify;

#[derive(Debug)]
pub struct ControlledSource {
    table: RwLock<Arc<MemTable>>,
    pub gated: AtomicBool,
    pub scans: AtomicUsize,
    pub entered: Notify,
    pub release: Notify,
}
impl ControlledSource {
    pub fn new(data: &TableSnapshot) -> Result<Self> {
        Ok(Self {
            table: RwLock::new(Arc::new(MemTable::try_new(
                data.schema().clone(),
                vec![data.batches().to_vec()],
            )?)),
            gated: AtomicBool::new(false),
            scans: AtomicUsize::new(0),
            entered: Notify::new(),
            release: Notify::new(),
        })
    }
    pub fn plan(self: &Arc<Self>) -> Result<LogicalPlan> {
        Ok(LogicalPlanBuilder::scan("source", provider_as_source(self.clone()), None)?.build()?)
    }
    pub fn replace(&self, data: &TableSnapshot) -> Result<()> {
        *self.table.write().unwrap() = Arc::new(MemTable::try_new(
            data.schema().clone(),
            vec![data.batches().to_vec()],
        )?);
        Ok(())
    }
}
#[async_trait]
impl TableProvider for ControlledSource {
    fn schema(&self) -> SchemaRef {
        self.table.read().unwrap().schema()
    }
    fn table_type(&self) -> TableType {
        TableType::Base
    }
    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DFResult<Arc<dyn ExecutionPlan>> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        if self.gated.load(Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        let table = self.table.read().unwrap().clone();
        table.scan(state, projection, filters, limit).await
    }
}
