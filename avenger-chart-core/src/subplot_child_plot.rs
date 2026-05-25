use std::{any::Any, sync::Arc};

use datafusion::prelude::SessionContext;
use serde::{Deserialize, Serialize};

use crate::AvengerChartError;

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
