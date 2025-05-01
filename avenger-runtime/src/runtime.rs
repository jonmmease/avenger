use std::{collections::HashMap, sync::Arc};
use async_recursion::async_recursion;
use avenger_lang2::ast::{AvengerProject, Statement};
use avenger_scales::utils::ScalarValueUtils;
use avenger_scenegraph::scene_graph::SceneGraph;
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
    ) -> Result<HashMap<Variable, TaskValue>, AvengerRuntimeError> {
        let mut futures = Vec::new();
        for variable in variables {
            futures.push(
                self.clone().evaluate_variable(graph.clone(), variable)
            );
        }

        let values = join_all(futures).await;
        let values = values.into_iter().collect::<Result<Vec<_>, AvengerRuntimeError>>()?;
        Ok(Vec::from(variables).into_iter().zip(values).collect())
    }


    pub async fn evaluate_file(
        self: Arc<Self>,
        project: &AvengerProject,
        file_name: &str,
    ) -> Result<SceneGraph, AvengerRuntimeError> {

        // let component_registry = Arc::new(ComponentRegistry::from(project));
        
        // Build task graph
        let task_graph = Arc::new(TaskGraph::from_file(&project, file_name)?);
        
        // Build variables for size of the scenegraph
        let mut dim_vars = vec![];

        let width_var = Variable::new(vec!["width".to_string()]);
        let height_var = Variable::new(vec!["height".to_string()]);
        let x_var = Variable::new(vec!["x".to_string()]);
        let y_var = Variable::new(vec!["y".to_string()]);
        for var in [&width_var, &height_var, &x_var, &y_var] {
            if task_graph.tasks().contains_key(var) {
                dim_vars.push(var.clone());
            }
        }

        // Build variables for the child marks
        let mut mark_vars = vec![];

        let Some(file_ast) = project.files.get(file_name) else {
            return Err(AvengerRuntimeError::InternalError(format!(
                "File {} not found in project", file_name
            )));
        };
        
        for stmt in &file_ast.statements {
            if let Statement::ComponentProp(comp) = stmt {
                let comp_type = comp.component_type.value.clone();
                println!("comp_type: {comp_type:?}");
                // let is_mark = component_registry.lookup_component(&comp_type).map(
                //     |spec| spec.is_mark
                // ).unwrap_or(false);
                
                // if is_mark || comp_type == "Group" { }
                // Build var with group components mark
                let mut parts = vec![comp.name()];
                parts.push("_mark".to_string());
                let mark_var = Variable::new(parts);
                mark_vars.push(mark_var);
            }
        }

        // Combine dimension and mark variables
        let all_vars = [dim_vars, mark_vars.clone()].concat();

        // Evaluate all variables
        let mut results = self.clone().evaluate_variables(task_graph.clone(), &all_vars).await?;

        // Get the width, height, x, and y from the results and apply defaults
        let width = results.get(&width_var)
            .and_then(|v| v.as_val().ok())
            .and_then(|v| v.as_f32().ok())
            .unwrap_or(500.0);
        
        let height = results.get(&height_var)
            .and_then(|v| v.as_val().ok())
            .and_then(|v| v.as_f32().ok())
            .unwrap_or(500.0);

        let x = results.get(&x_var)
            .and_then(|v| v.as_val().ok())
            .and_then(|v| v.as_f32().ok())
            .unwrap_or(0.0);

        let y = results.get(&y_var)
            .and_then(|v| v.as_val().ok())
            .and_then(|v| v.as_f32().ok())
            .unwrap_or(0.0);


        // Get the marks
        let marks = mark_vars.iter().map(|v| results.remove(v).unwrap().into_mark().unwrap()).collect::<Vec<_>>();

        println!("marks: {:?}", marks);

        Ok(SceneGraph {
            width,
            height,
            origin: [x, y],
            marks,
        })
    }
}
