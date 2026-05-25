use std::{any::Any, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CompiledSubplotPayload, CoordinateSystemTransformCore, MarkRuntimeContext,
    RadiusExpression, SubplotContainerCoordinateSystem, SubplotMarkCore, compile_subplot_payload,
};
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::{arrow::record_batch::RecordBatch, logical_expr::Expr, prelude::SessionContext};
use serde::{Deserialize, Serialize};

use crate::Cartesian;

#[async_trait::async_trait]
impl SubplotContainerCoordinateSystem for Cartesian {
    async fn compile_subplot_mark(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_facet_channels("Cartesian")?;

        Ok(Arc::new(CompiledCartesianSubplot {
            payload: compile_subplot_payload(subplot, compiled_state, session_context).await?,
            plot_width: subplot.plot_width_config().unwrap_or(80.0).max(1.0),
            plot_height: subplot.plot_height_config().unwrap_or(80.0).max(1.0),
        }))
    }
}

/// Compiled child-plot mark positioned by Cartesian x/y channels.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianSubplot {
    payload: CompiledSubplotPayload,
    plot_width: f32,
    plot_height: f32,
}

impl CompiledCartesianSubplot {
    pub fn payload(&self) -> &CompiledSubplotPayload {
        &self.payload
    }

    pub fn label(&self) -> Option<&str> {
        self.payload.label()
    }

    pub fn key(&self) -> Option<&str> {
        self.payload.key()
    }

    pub fn mark_index(&self) -> usize {
        self.payload.mark_index()
    }

    pub fn plot_width(&self) -> f32 {
        self.plot_width
    }

    pub fn plot_height(&self) -> f32 {
        self.plot_height
    }

    pub fn inherits_parent_data(&self) -> bool {
        self.payload.inherits_parent_data()
    }

    pub fn group_name(&self, child_index: usize) -> String {
        match self.key() {
            Some(key) => format!(
                "cartesian_subplot_{}_{}_{}",
                self.mark_index(),
                child_index,
                key
            ),
            None => format!("cartesian_subplot_{}_{}", self.mark_index(), child_index),
        }
    }
}

impl CompiledMarkCore for CompiledCartesianSubplot {
    fn state(&self) -> &CompiledMarkState {
        self.payload.compiled_state()
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        self.payload.compiled_state_mut()
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.payload.compiled_state().data
    }

    fn mark_type(&self) -> &str {
        "subplot"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "x",
                required: true,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "y",
                required: true,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn radius_expression(
        &self,
        _dimension: &str,
        _resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        None
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianSubplot {
    async fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Cartesian subplot marks require the top-level layout render dispatcher".to_string(),
        ))
    }
}
