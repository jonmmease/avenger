pub mod compiled_data_context;
pub mod data_context;
pub mod facet_strategy;
pub mod line;
#[macro_use]
pub mod macros;
pub mod rect;
pub mod state;
pub mod subplot;
pub mod symbol;
pub mod util;

use std::sync::Arc;

use avenger_scenegraph::marks::mark::SceneMark;

use datafusion::arrow::record_batch::RecordBatch;

use crate::{
    chart_core::CoordinateSystemCore, coords::CoordinateSystemTransformCore,
    error::AvengerChartError, render::RenderContext,
};

pub use crate::cartesian::positioned_subplot::CompiledCartesianSubplot;
pub use crate::channel::{ChannelDefault, ChannelDescriptor, ChannelValue, ConditionalValue};
pub(crate) use crate::chart_core::default_channel_value_for_eval;
pub use crate::chart_core::{CompiledMarkCore, RadiusExpression, default_scale_type_for_data_type};
pub use crate::concat::CompiledConcatSubplot;
pub use compiled_data_context::CompiledDataContext;
pub use data_context::DataContext;
pub use facet_strategy::FacetStrategy;
pub use state::{CompiledMarkState, MarkState};
pub use subplot::{
    CompiledSubplotPayload, Subplot, SubplotChildPlotSpec, SubplotContainerCoordinateSystem,
    SubplotDataSource, compile_subplot_payload,
};

/// Core trait for all mark types (uncompiled)
#[async_trait::async_trait]
pub trait Mark<C: CoordinateSystemCore>: Send + Sync + 'static {
    /// Get the mark's state (uncompiled version with DataContext)
    fn state(&self) -> &MarkState;

    /// Get mutable reference to the mark's state
    fn state_mut(&mut self) -> &mut MarkState;

    /// Get the data context for this mark (for accessing encodings and data during construction)
    fn data_context(&self) -> &DataContext;

    /// Build a CompiledMark from this Mark with the provided compiled state
    /// This enables type-erased rendering without the coordinate system generic
    /// The compiled_state contains the transformed DataFrame (e.g., after aggregation)
    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError>;

    /// Convenience method to compile a mark without transforming its DataFrame
    /// This is useful for tests and simple cases where no aggregation is needed
    async fn compile_untransformed(
        &self,
        ctx: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        let mark_state = self.state();
        let df = mark_state.data.dataframe().cloned();
        let compiled_state = CompiledMarkState::from_mark_state(mark_state, df);
        self.compile(compiled_state, ctx).await
    }
}

#[typetag::serde(tag = "type")]
#[async_trait::async_trait]
pub trait CompiledMark: CompiledMarkCore {
    /// Render the mark from prepared data batches.
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &RenderContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;
}
