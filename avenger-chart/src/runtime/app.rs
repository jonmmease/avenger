use crate::error::AvengerChartError;
use crate::runtime::scale::eval_scale;
use crate::runtime::AvengerRuntime;
use crate::types::group::Group;
use crate::types::scales::Scale;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::scene_graph::SceneGraph;
use datafusion::common::{ParamValues, ScalarValue};
use datafusion::prelude::SessionContext;
use std::collections::HashMap;
use std::sync::Arc;
use arrow::array::RecordBatch;

#[derive(Clone)]
pub struct AvengerChartState {
    pub runtime: Arc<AvengerRuntime>,
    pub chart: Group,
    pub param_values: HashMap<String, ScalarValue>,
    pub details: HashMap<Vec<usize>, RecordBatch>,
}

impl AvengerChartState {
    pub fn new(chart: Group, runtime: Arc<AvengerRuntime>) -> Self {
        // Initialize param values with initial values from controllers
        let mut param_values = chart
            .controllers
            .iter()
            .flat_map(|c| {
                c.params()
                    .iter()
                    .map(|p| (p.name.clone(), p.default.clone()))
                    .collect::<Vec<_>>()
            })
            .collect::<HashMap<_, _>>();

        // Add explicit chart params
        for param in &chart.params {
            param_values.insert(param.name.clone(), param.default.clone());
        }

        Self {
            runtime,
            chart,
            param_values,
            details: HashMap::new(),
        }
    }

    pub fn param_values(&self) -> ParamValues {
        // Collect
        ParamValues::Map(self.param_values.clone())
    }

    pub async fn eval_scale(&self, scale: &Scale) -> ConfiguredScale {
        eval_scale(&scale, self.runtime.ctx(), Some(&self.param_values()))
            .await
            .unwrap()
    }

    pub async fn compile_scene_graph(&mut self) -> Result<SceneGraph, AvengerChartError> {
        // println!("compile_scene_graph");
        let compiled_scene_group = self
            .runtime
            .compile_group(&self.chart, vec![], self.param_values())
            .await?;

        // Update details
        self.details = compiled_scene_group.details;

        let scene_graph = SceneGraph {
            marks: vec![compiled_scene_group.scene_group.into()],
            width: 440.0,
            height: 440.0,
            origin: [20.0, 20.0],
        };

        Ok(scene_graph)
    }
}
