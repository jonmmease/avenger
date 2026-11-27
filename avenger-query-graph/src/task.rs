//! The tasks of a graph's computations: Tokio's natively, and the event loop of the page in
//! WebAssembly, whose futures run on one thread. A task runs to its end when nothing awaits it,
//! so a computation that a run started lands in the store, and its abort handle stops it.

use std::{future::Future, sync::Arc};

use datafusion::common::{DataFusionError, Result};
use futures::{
    channel::oneshot,
    future::{abortable, AbortHandle},
};

use crate::store::Computed;

/// The result of a computation's task, which resolves when the task ends.
pub(crate) type Task = oneshot::Receiver<Result<Arc<Computed>>>;

/// Spawns `computation` on the target's tasks, and returns its result and the handle that stops
/// it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn spawn(
    computation: impl Future<Output = Result<Arc<Computed>>> + Send + 'static,
) -> (Task, AbortHandle) {
    let (sender, task) = oneshot::channel();
    let (computation, handle) = abortable(computation);
    tokio::spawn(async move {
        let _ = sender.send(computation.await.unwrap_or_else(|_| Err(stopped())));
    });
    (task, handle)
}

/// Spawns `computation` on the page's event loop, and returns its result and the handle that
/// stops it.
#[cfg(target_arch = "wasm32")]
pub(crate) fn spawn(
    computation: impl Future<Output = Result<Arc<Computed>>> + 'static,
) -> (Task, AbortHandle) {
    let (sender, task) = oneshot::channel();
    let (computation, handle) = abortable(computation);
    wasm_bindgen_futures::spawn_local(async move {
        let _ = sender.send(computation.await.unwrap_or_else(|_| Err(stopped())));
    });
    (task, handle)
}

fn stopped() -> DataFusionError {
    DataFusionError::Execution("the computation stopped".into())
}
