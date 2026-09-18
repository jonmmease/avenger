use std::{
    collections::hash_map::Entry,
    sync::{Arc, Weak},
};

use futures::{
    channel::oneshot,
    future::{BoxFuture, Shared},
    FutureExt,
};
use tokio::{sync::watch, task::JoinHandle};

use crate::{
    cache::{Cache, ValueKey},
    runtime::{NodeValue, RuntimeInner},
    Error, Result,
};

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    value: ValueKey,
    epoch: u64,
}

#[derive(Clone)]
enum Outcome {
    Value(Arc<NodeValue>),
    Failed(Arc<Error>),
    Cancelled,
}

type Completion = Shared<BoxFuture<'static, Outcome>>;

pub(crate) struct Flight {
    id: u64,
    completion: Completion,
    interest: Weak<Interest>,
    unwatched: watch::Sender<bool>,
    stopping: bool,
    task: Option<JoinHandle<()>>,
}

struct Interest {
    runtime: Weak<RuntimeInner>,
    key: Key,
    id: u64,
}

impl Drop for Interest {
    fn drop(&mut self) {
        let Some(runtime) = self.runtime.upgrade() else {
            return;
        };
        let mut cache = runtime.cache.lock().expect("cache lock");
        if let Some(flight) = cache.flights.get_mut(&self.key) {
            if flight.id == self.id && flight.interest.strong_count() == 0 {
                flight.unwatched.send_replace(true);
            }
        }
    }
}

pub(crate) enum Lookup {
    Ready(crate::inputs::MaterializedValue),
    Pending(Pending),
    Reserved(Reservation, Pending),
}

pub(crate) struct Pending {
    completion: Completion,
    interest: Arc<Interest>,
}

impl Pending {
    pub(crate) fn track(&self, task: JoinHandle<()>) {
        let Some(runtime) = self.interest.runtime.upgrade() else {
            return;
        };
        let mut cache = runtime.cache.lock().expect("cache lock");
        if let Some(flight) = cache.flights.get_mut(&self.interest.key) {
            if flight.id == self.interest.id {
                flight.task = Some(task);
            }
        }
    }

    pub(crate) async fn wait(self) -> Result<Option<Arc<NodeValue>>> {
        let outcome = self.completion.await;
        // Keep consumer interest until the completion future has settled.
        drop(self.interest);
        match outcome {
            Outcome::Value(value) => Ok(Some(value)),
            Outcome::Cancelled => Ok(None),
            Outcome::Failed(error) => Err(Arc::try_unwrap(error).unwrap_or_else(Error::Shared)),
        }
    }
}

pub(crate) struct Reservation {
    runtime: Weak<RuntimeInner>,
    key: Key,
    id: u64,
    sender: Option<oneshot::Sender<Outcome>>,
    unwatched: watch::Receiver<bool>,
}

impl Cache {
    pub(crate) fn lookup(
        &mut self,
        value: ValueKey,
        epoch: u64,
        runtime: &Arc<RuntimeInner>,
        name: String,
    ) -> Lookup {
        if let Some(value) = self.get(&value, epoch) {
            return Lookup::Ready(value);
        }
        let key = Key { value, epoch };
        match self.flights.entry(key.clone()) {
            Entry::Occupied(mut entry) => {
                let flight = entry.get_mut();
                let interest = flight.interest.upgrade().unwrap_or_else(|| {
                    let interest = Arc::new(Interest {
                        runtime: Arc::downgrade(runtime),
                        key,
                        id: flight.id,
                    });
                    flight.interest = Arc::downgrade(&interest);
                    if !flight.stopping {
                        flight.unwatched.send_replace(false);
                    }
                    interest
                });
                Lookup::Pending(Pending {
                    completion: flight.completion.clone(),
                    interest,
                })
            }
            Entry::Vacant(entry) => {
                let id = crate::fresh_id();
                let interest = Arc::new(Interest {
                    runtime: Arc::downgrade(runtime),
                    key: key.clone(),
                    id,
                });
                let (sender, receiver) = oneshot::channel();
                let completion = async move {
                    receiver.await.unwrap_or_else(|_| {
                        Outcome::Failed(Arc::new(Error::Execution {
                            node: name,
                            source: datafusion::common::DataFusionError::Execution(
                                "calculation task ended without a result".into(),
                            ),
                        }))
                    })
                }
                .boxed()
                .shared();
                let (unwatched_sender, unwatched) = watch::channel(false);
                entry.insert(Flight {
                    id,
                    completion: completion.clone(),
                    interest: Arc::downgrade(&interest),
                    unwatched: unwatched_sender,
                    stopping: false,
                    task: None,
                });
                Lookup::Reserved(
                    Reservation {
                        runtime: Arc::downgrade(runtime),
                        key,
                        id,
                        sender: Some(sender),
                        unwatched,
                    },
                    Pending {
                        completion,
                        interest,
                    },
                )
            }
        }
    }
}

impl Reservation {
    pub(crate) fn unwatched(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        let runtime = self.runtime.clone();
        let key = self.key.clone();
        let id = self.id;
        let mut receiver = self.unwatched.clone();
        async move {
            loop {
                match receiver.wait_for(|value| *value).await {
                    Ok(value) => drop(value),
                    Err(_) => return,
                }
                let Some(runtime) = runtime.upgrade() else {
                    return;
                };
                let mut cache = runtime.cache.lock().expect("cache lock");
                let Some(flight) = cache.flights.get_mut(&key) else {
                    return;
                };
                if flight.id != id {
                    return;
                }
                // An upgrade could become the last owner and reenter Interest::drop
                // while this lock is held. New subscriptions also take this lock.
                if flight.interest.strong_count() == 0 {
                    flight.stopping = true;
                    return;
                }
            }
        }
    }

    pub(crate) fn finish(
        mut self,
        result: Result<Arc<NodeValue>>,
        before_notify: impl FnOnce(bool),
    ) {
        let outcome = match result {
            Ok(value) => Outcome::Value(value),
            Err(error) => Outcome::Failed(Arc::new(error)),
        };
        self.settle(Some(outcome), before_notify)
    }

    pub(crate) fn cancel(mut self) {
        self.settle(Some(Outcome::Cancelled), |_| {});
    }

    fn settle(&mut self, outcome: Option<Outcome>, before_notify: impl FnOnce(bool)) {
        let Some(sender) = self.sender.take() else {
            return;
        };
        let mut bypassed = false;
        if let Some(runtime) = self.runtime.upgrade() {
            let mut cache = runtime.cache.lock().expect("cache lock");
            if cache
                .flights
                .get(&self.key)
                .is_some_and(|flight| flight.id == self.id)
            {
                cache.flights.remove(&self.key);
                if let Some(Outcome::Value(value)) = &outcome {
                    bypassed =
                        !cache.insert(self.key.value.clone(), self.key.epoch, value.value.clone());
                }
                before_notify(bypassed);
                if let Some(outcome) = outcome {
                    let _ = sender.send(outcome);
                }
            }
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.settle(None, |_| {});
    }
}

#[cfg(test)]
impl Cache {
    pub(crate) fn stopping(&self) -> bool {
        self.flights.values().any(|flight| flight.stopping)
    }
    pub(crate) fn watchers(&self, namespace: u64, node: usize) -> usize {
        self.flights
            .iter()
            .filter(|(key, _)| key.value.namespace == namespace && key.value.node == node)
            .map(|(_, flight)| flight.interest.strong_count())
            .sum()
    }
}
