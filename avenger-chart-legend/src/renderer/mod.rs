//! Built-in legend renderers.

pub mod colorbar;
pub mod line;
pub mod rect;
pub mod symbol;

pub use avenger_chart_core::{
    ChannelInfo, ChannelLegendCapability, LegendChannel, LegendGroup, LegendRenderer, MergeKey,
    compute_range_hash, helpers, normalize_expression,
};
pub use colorbar::CompiledColorbar;
pub use line::CompiledLineLegend;
pub use rect::CompiledRectLegend;
pub use symbol::CompiledSymbolLegend;

use std::sync::Arc;

use avenger_chart_core::LegendRendererKind;

pub fn renderer_for_kind(kind: LegendRendererKind) -> Arc<dyn LegendRenderer> {
    match kind {
        LegendRendererKind::Symbol => Arc::new(CompiledSymbolLegend::new()),
        LegendRendererKind::Line => Arc::new(CompiledLineLegend::new()),
        LegendRendererKind::Rect => Arc::new(CompiledRectLegend::new()),
        LegendRendererKind::Colorbar => Arc::new(CompiledColorbar::new()),
    }
}
