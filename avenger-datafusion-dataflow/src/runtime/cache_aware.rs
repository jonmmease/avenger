use super::*;
use crate::{
    CacheAwareOptions, CacheAwareQuery, CacheAwareResult, CacheNode, CacheRead, CacheTargets,
    QueryInputs, Reference,
};

impl PreparedDataflow {
    /// Capture ordered input candidates and optionally start warming their preferred targets.
    ///
    /// Only root outputs and reusable root targets are supported. Explicit targets must
    /// be ancestors of requested outputs. Starting background work requires a running
    /// Tokio runtime. The constructor does not wait for an execution slot.
    pub fn cache_aware_query(
        &self,
        tables: &[TableOutput],
        scalars: &[ScalarOutput],
        inputs: QueryInputs,
        options: CacheAwareOptions,
    ) -> Result<CacheAwareQuery> {
        let graph = &self.inner.graph;
        if inputs
            .candidates
            .iter()
            .any(|inputs| inputs.graph.id != graph.id)
        {
            return Err(Error::ForeignHandle);
        }
        let mut requested = vec![false; graph.outputs.len()];
        for (owner, index, scope) in tables
            .iter()
            .map(|o| (o.graph, o.index, o.scope))
            .chain(scalars.iter().map(|o| (o.graph, o.index, o.scope)))
        {
            if owner != graph.id {
                return Err(Error::ForeignHandle);
            }
            if scope != 0 {
                return Err(Error::OutOfScope(
                    "cache-aware queries support root outputs only".into(),
                ));
            }
            requested[index] = true;
        }
        let (needed, _) = demand(graph, &requested);
        let targets = match options.targets {
            CacheTargets::RequestedOutputs => graph
                .outputs
                .iter()
                .enumerate()
                .filter(|(index, _)| requested[*index])
                .map(|(_, output)| output.node)
                .collect::<Vec<_>>(),
            CacheTargets::Nodes(nodes) => {
                if nodes.is_empty() {
                    return Err(Error::InvalidReference(
                        "cache targets must not be empty".into(),
                    ));
                }
                nodes
                    .iter()
                    .map(|node| self.cache_node(node))
                    .collect::<Result<Vec<_>>>()?
            }
        };
        let targets = targets
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        for index in &targets {
            let node = &graph.nodes[*index];
            if node.scope != 0 {
                return Err(Error::OutOfScope(node.name.to_string()));
            }
            let asset = matches!(&node.plan, LogicalPlan::Extension(e) if e.node.as_any()
                .downcast_ref::<crate::graph::reference::GraphRead>()
                .is_some_and(|read| matches!(read.source, TableRef::Asset(_))));
            if !needed[*index] || node.analysis.reuse_scope != crate::ReuseScope::Reusable || asset
            {
                return Err(Error::InvalidReference(format!("cache target {} must be a reusable computation contributing to the requested outputs", node.name)));
            }
        }
        let background = if options.start_latest {
            let executor = tokio::runtime::Handle::try_current().map_err(|_| {
                Error::InvalidConfig("background warming requires an active Tokio runtime".into())
            })?;
            let (interest, cancelled) = tokio::sync::oneshot::channel();
            let prepared = self.clone();
            let preferred = inputs.candidates[0].clone();
            let targets = targets.clone();
            executor.spawn(async move {
                prepared.warm_targets(preferred, targets, cancelled).await;
            });
            Some(interest)
        } else {
            None
        };
        Ok(CacheAwareQuery {
            prepared: self.clone(),
            inputs,
            requested,
            targets,
            _background: background,
        })
    }

    fn cache_node(&self, node: &CacheNode) -> Result<usize> {
        let graph = &self.inner.graph;
        match node {
            CacheNode::Plan(node) => {
                if node.read.graph != graph.id {
                    return Err(Error::ForeignHandle);
                }
                match node.read.source {
                    TableRef::Node(index) => Ok(index),
                    _ => Err(Error::InvalidReference(node.name().into())),
                }
            }
            CacheNode::Scalar(node) => {
                if node.graph != graph.id {
                    return Err(Error::ForeignHandle);
                }
                Ok(node.index)
            }
            CacheNode::Named(reference) => {
                if !reference.scope.is_empty() {
                    return Err(Error::OutOfScope(reference.name.clone()));
                }
                graph
                    .nodes
                    .iter()
                    .position(|n| n.scope == 0 && n.name.as_ref() == reference.name)
                    .ok_or_else(|| Error::InvalidReference(reference.name.clone()))
            }
        }
    }

    async fn warm_targets(
        &self,
        inputs: Inputs,
        targets: Vec<usize>,
        mut cancelled: tokio::sync::oneshot::Receiver<()>,
    ) {
        let runtime = &self.inner.runtime;
        let permits = async {
            let warming = runtime
                .warming
                .acquire()
                .await
                .expect("private semaphore stays open");
            let execution = runtime
                .queries
                .acquire()
                .await
                .expect("private semaphore stays open");
            (warming, execution)
        };
        let (_warming, _execution) = tokio::select! {
            biased;
            _ = &mut cancelled => return,
            permits = permits => permits,
        };
        let mut evaluation = Evaluation::root(
            self.inner.clone(),
            inputs,
            vec![false; self.inner.graph.outputs.len()],
        );
        let outcome = {
            let calculation = async {
                for target in &targets {
                    evaluation.node(Origin::Local, *target).await?;
                }
                Ok::<_, Error>(())
            };
            tokio::select! {
                biased;
                _ = &mut cancelled => None,
                outcome = std::panic::AssertUnwindSafe(calculation).catch_unwind() => Some(outcome),
            }
        };
        evaluation.settle_execution().await;
        match outcome {
            Some(Ok(Err(error))) => {
                tracing::warn!(namespace = self.inner.namespace, ?targets, %error, "background cache warming failed")
            }
            Some(Err(_)) => tracing::warn!(
                namespace = self.inner.namespace,
                ?targets,
                "background cache warming panicked"
            ),
            _ => {}
        }
    }
}

impl CacheAwareQuery {
    /// Select the first qualifying candidate from the retained cache.
    ///
    /// `CachedOnly` assembles retained outputs without taking an execution slot.
    /// `FromCachedTargets` can execute downstream nodes, using owned target values
    /// even if the cache is cleared or entries are evicted after selection.
    pub async fn read(&self, mode: CacheRead) -> Result<CacheAwareResult> {
        let graph = &self.prepared.inner.graph;
        let outputs = graph
            .outputs
            .iter()
            .enumerate()
            .filter(|(index, _)| self.requested[*index])
            .map(|(_, output)| output.node)
            .collect::<BTreeSet<_>>();
        let nodes = self
            .targets
            .iter()
            .copied()
            .chain(outputs.iter().copied())
            .collect::<BTreeSet<_>>();
        let mut preferred_missing = vec![];
        for (candidate_index, inputs) in self.inputs.candidates.iter().enumerate() {
            let mut evaluation = Evaluation::root(
                self.prepared.inner.clone(),
                inputs.clone(),
                self.requested.clone(),
            );
            let mut keys = Vec::with_capacity(nodes.len());
            for index in &nodes {
                evaluation.load_inputs(Origin::Local, &graph.nodes[*index].analysis.inputs)?;
                let key = evaluation.value_key(Origin::Local, *index)?;
                if let Some(key) = &key {
                    evaluation.reservation.charge(key.size())?;
                }
                keys.push((*index, key));
            }
            let runtime = &self.prepared.inner.runtime;
            let acquired = {
                let mut cache = runtime.cache.lock().expect("cache lock");
                let missing = keys
                    .iter()
                    .filter(|(index, key)| {
                        (mode == CacheRead::CachedOnly || self.targets.contains(index))
                            && !key
                                .as_ref()
                                .is_some_and(|key| cache.contains(key, evaluation.epoch))
                    })
                    .map(|(index, _)| Reference {
                        scope: vec![],
                        name: graph.nodes[*index].name.to_string(),
                    })
                    .collect::<Vec<_>>();
                if !missing.is_empty() {
                    if candidate_index == 0 {
                        preferred_missing = missing;
                    }
                    None
                } else {
                    Some(
                        keys.iter()
                            .filter_map(|(index, key)| {
                                key.as_ref()
                                    .and_then(|key| cache.get(key, evaluation.epoch))
                                    .map(|value| (*index, value))
                            })
                            .collect::<Vec<_>>(),
                    )
                }
            };
            let Some(acquired) = acquired else {
                continue;
            };
            for (index, value) in acquired {
                evaluation.reservation.charge(value.size() + 128)?;
                let value = evaluation.node_value(Origin::Local, index, value, vec![]);
                evaluation.frames[0].values.insert(index, value);
                evaluation.work.lock().expect("work lock").cache_hits += 1;
            }
            let ready = outputs
                .iter()
                .all(|index| evaluation.frames[0].values.contains_key(index));
            let _permit = if ready {
                None
            } else {
                Some(
                    runtime
                        .queries
                        .acquire()
                        .await
                        .expect("private semaphore stays open"),
                )
            };
            let root = evaluation.frame().await?;
            evaluation.finish_report();
            evaluation.report.materialized_bytes =
                evaluation.reservation.bytes.load(Ordering::Relaxed);
            evaluation.report.retained_bytes =
                runtime.cache.lock().expect("cache lock").stats().bytes;
            return Ok(CacheAwareResult {
                result: DataflowResult {
                    graph: graph.id,
                    root,
                    report: evaluation.report.clone(),
                },
                inputs: inputs.clone(),
                candidate_index,
            });
        }
        Err(Error::CacheMiss {
            missing: preferred_missing,
            candidates_checked: self.inputs.candidates.len(),
        })
    }
}

impl Evaluation {
    fn root(prepared: Arc<PreparedInner>, inputs: Inputs, requested: Vec<bool>) -> Self {
        let runtime = &prepared.runtime;
        let epoch = runtime
            .cache
            .lock()
            .expect("cache lock")
            .epoch(prepared.namespace);
        let (_, scope_needed) = demand(&prepared.graph, &requested);
        Self {
            base: None,
            inputs,
            requested,
            scope_needed,
            epoch,
            frames: vec![Frame::root(Origin::Local)],
            reservation: Arc::new(Reservation {
                runtime: runtime.clone(),
                bytes: AtomicUsize::new(0),
            }),
            work: Arc::new(Mutex::new(Work::default())),
            cleanup: vec![],
            report: evaluation_report(&prepared.graph, false),
            prepared,
        }
    }
}

#[cfg(test)]
mod tests;
