use std::sync::Arc;
use async_recursion::async_recursion;
use futures::future::join_all;
use crate::{cache::{RuntimeCache, RuntimeCacheConfig}, error::AvengerRuntimeError, task_graph::TaskGraph, value::TaskValue, variable::Variable};

pub struct TaskGraphRuntime {
    cache: Arc<RuntimeCache>,
}

impl TaskGraphRuntime {
    pub fn new(config: RuntimeCacheConfig) -> Self {
        Self {
            cache: Arc::new(RuntimeCache::new(config)),
        }
    }

    /// Evaluate a single variable and its dependencies
    #[async_recursion]
    async fn evaluate_variable(
        self: Arc<Self>,
        graph: Arc<TaskGraph>,
        variable: &Variable,
    ) -> Result<TaskValue, AvengerRuntimeError> {        
        // Lookup the node for this variable
        let node = graph.tasks().get(variable).ok_or_else(|| {
            AvengerRuntimeError::VariableNotFound(format!("Variable not found: {:?}", variable))
        })?.clone();

        // Build future to compute the value of this variable, this will only be 
        // executed if the value is not cached
        let inner_self = self.clone();
        let inner_graph = graph.clone();
        let fut = async move {
            // Create future to compute node value (will only be executed if not present in cache)
            let mut inputs_futures = Vec::new();
            for input_node in &node.inputs {
                let node_fut = inner_self.clone().evaluate_variable(
                    inner_graph.clone(),
                    &input_node.source,
                );

                // In non-wasm environment, use tokio::spawn for multi-threading
                inputs_futures.push(node_fut);
            }

            let input_values = join_all(inputs_futures).await;

            // Extract the appropriate value from
            let input_values = input_values
                .into_iter()
                .collect::<Result<Vec<_>, AvengerRuntimeError>>()?;

            node.task.evaluate(inner_self.clone(), &input_values).await
        };

        // get or construct from cache
        Ok(self.cache.get_or_try_insert_with(node.fingerprint, node.identity_fingerprint, fut).await?)
    }

    pub async fn evaluate_variables(
        self: Arc<Self>,
        graph: Arc<TaskGraph>,
        variables: &[Variable],
    ) -> Result<Vec<TaskValue>, AvengerRuntimeError> {
        let mut futures = Vec::new();
        for variable in variables {
            futures.push(
                self.clone().evaluate_variable(graph.clone(), variable)
            );
        }

        let values = join_all(futures).await;
        Ok(values.into_iter().collect::<Result<Vec<_>, AvengerRuntimeError>>()?)
    }
}
