use std::{any::Any, sync::Arc};

use datafusion::{arrow::record_batch::RecordBatch, dataframe::DataFrame, prelude::SessionContext};
use serde::{Deserialize, Serialize};

use crate::{
    AvengerChartError, ColumnDimensionConfig, CompiledMarkState, DataContext, FacetDimensionConfig,
    FacetEmptyCellPolicy, RowDimensionConfig, ScaleSharing,
};

/// Data source selected for a compiled subplot's child plot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubplotDataSource {
    /// The child plot has explicit plot-level data.
    ExplicitChild,
    /// The child plot has no plot-level data and should inherit container data.
    InheritParent,
}

/// Serializable compiled child plot owned by a `Subplot` mark.
///
/// Core owns this erased handle so `Subplot` authoring and coordinate-specific
/// subplot compilation do not have to traffic in the top-level `CompiledPlot`
/// type. The facade still downcasts this handle for its built-in layout engine.
#[typetag::serde(tag = "type")]
pub trait CompiledSubplotChildPlot: Any + Send + Sync {
    /// Downcast support for the facade-owned layout/runtime engine.
    fn as_any(&self) -> &dyn Any;

    /// Convert an erased child plot handle into an erased `Any` handle for
    /// downcasting while preserving `Arc` ownership.
    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

#[async_trait::async_trait]
#[doc(hidden)]
pub trait SubplotChildPlotSpec: Send + Sync {
    fn clone_box(&self) -> Box<dyn SubplotChildPlotSpec>;
    fn has_plot_level_data(&self) -> bool;
    async fn compile_boxed(
        &self,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError>;
}

impl Clone for Box<dyn SubplotChildPlotSpec> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// Core view over a neutral subplot mark.
///
/// This lets shared subplot helpers live below the top-level facade without
/// making `avenger-chart-core` depend on the concrete `Subplot<C>` type.
#[async_trait::async_trait]
#[doc(hidden)]
pub trait SubplotMarkCore: Send + Sync {
    fn data_context_ref(&self) -> &DataContext;

    fn label_config(&self) -> Option<&str>;

    fn key_config(&self) -> Option<&str>;

    fn plot_width_config(&self) -> Option<f32> {
        None
    }

    fn plot_height_config(&self) -> Option<f32> {
        None
    }

    fn facet_row_title_config(&self) -> Option<&str> {
        None
    }

    fn facet_row_slot_sharing_config(&self) -> Option<ScaleSharing> {
        None
    }

    fn facet_row_position_config(&self) -> Option<&str> {
        None
    }

    fn facet_row_empty_cell_policy_config(&self) -> Option<FacetEmptyCellPolicy> {
        None
    }

    fn facet_col_title_config(&self) -> Option<&str> {
        None
    }

    fn facet_col_slot_sharing_config(&self) -> Option<ScaleSharing> {
        None
    }

    fn facet_col_position_config(&self) -> Option<&str> {
        None
    }

    fn facet_col_empty_cell_policy_config(&self) -> Option<FacetEmptyCellPolicy> {
        None
    }

    fn has_plot_level_data(&self) -> bool;

    async fn compile_child_plot(
        &self,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError>;

    fn validate_no_facet_channels(&self, outer_label: &str) -> Result<(), AvengerChartError> {
        for channel_name in [
            RowDimensionConfig::channel_name(),
            ColumnDimensionConfig::channel_name(),
        ] {
            self.validate_no_channel(channel_name, outer_label)?;
        }
        Ok(())
    }

    fn validate_no_channel(
        &self,
        channel_name: &'static str,
        outer_label: &str,
    ) -> Result<(), AvengerChartError> {
        if self
            .data_context_ref()
            .channels()
            .contains_key(channel_name)
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{outer_label} subplots do not support channel `{channel_name}`"
            )));
        }
        Ok(())
    }
}

/// Shared compiled state for a child plot owned by a container subplot mark.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledSubplotPayload {
    state: CompiledMarkState,
    compiled_subplot: Arc<dyn CompiledSubplotChildPlot>,
    label: Option<String>,
    key: Option<String>,
    data_source: SubplotDataSource,
}

impl CompiledSubplotPayload {
    pub fn new(
        state: CompiledMarkState,
        compiled_subplot: Arc<dyn CompiledSubplotChildPlot>,
        label: Option<String>,
        key: Option<String>,
        data_source: SubplotDataSource,
    ) -> Self {
        Self {
            state,
            compiled_subplot,
            label,
            key,
            data_source,
        }
    }

    pub fn compiled_child_plot(&self) -> &Arc<dyn CompiledSubplotChildPlot> {
        &self.compiled_subplot
    }

    pub fn compiled_state(&self) -> &CompiledMarkState {
        &self.state
    }

    pub fn compiled_state_mut(&mut self) -> &mut CompiledMarkState {
        &mut self.state
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    pub fn mark_index(&self) -> usize {
        self.state.mark_index()
    }

    pub fn data_source(&self) -> SubplotDataSource {
        self.data_source
    }

    pub fn inherits_parent_data(&self) -> bool {
        self.data_source == SubplotDataSource::InheritParent
    }

    pub fn has_explicit_child_data(&self) -> bool {
        self.data_source == SubplotDataSource::ExplicitChild
    }

    #[doc(hidden)]
    pub fn inherited_data_override(
        &self,
        data: Option<&RecordBatch>,
        session_context: &SessionContext,
    ) -> Result<Option<DataFrame>, AvengerChartError> {
        if !self.inherits_parent_data() {
            return Ok(None);
        }

        data.map(|batch| {
            session_context
                .read_batch(batch.clone())
                .map_err(AvengerChartError::DataFusionError)
        })
        .transpose()
    }
}

pub async fn compile_subplot_payload<S: SubplotMarkCore + ?Sized>(
    subplot: &S,
    compiled_state: CompiledMarkState,
    session_context: &SessionContext,
) -> Result<CompiledSubplotPayload, AvengerChartError> {
    let data_source = if subplot.has_plot_level_data() {
        SubplotDataSource::ExplicitChild
    } else {
        SubplotDataSource::InheritParent
    };
    let compiled_subplot = subplot.compile_child_plot(session_context).await?;

    Ok(CompiledSubplotPayload::new(
        compiled_state,
        compiled_subplot,
        subplot.label_config().map(ToOwned::to_owned),
        subplot.key_config().map(ToOwned::to_owned),
        data_source,
    ))
}
