use async_trait::async_trait;
use std::{fmt::Debug, sync::Arc};

use crate::{dependency::Dependency, error::AvengerRuntimeError, runtime::TaskGraphRuntime, value::TaskValue, variable::Variable};




#[async_trait]
pub trait Task: Debug + Send + Sync {
    /// Get the dependencies of the task
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerRuntimeError> {
        Ok(vec![])
    }

    /// Get the input variables of the task
    fn input_variables(&self) -> Result<Vec<Variable>, AvengerRuntimeError> {
        Ok(self.input_dependencies()?.iter().map(
            |dep| dep.variable.clone()
        ).collect())
    }

    fn fingerprint(&self) -> Result<u64, AvengerRuntimeError>;

    /// Evaluate the task in a session context with the given dependencies
    async fn evaluate(
        &self,
        runtime: Arc<TaskGraphRuntime>,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerRuntimeError>;
}
