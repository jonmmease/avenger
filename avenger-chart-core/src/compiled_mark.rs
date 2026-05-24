use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use datafusion::{
    arrow::datatypes::DataType,
    logical_expr::{Expr, lit},
    prelude::SessionContext,
    scalar::ScalarValue,
};
use datafusion_common::ScalarValue as DatafusionScalarValue;
use indexmap::IndexMap;

use crate::{
    ChannelDescriptor, CompiledDataContext, CompiledMarkState, EvaluationContext,
    LegendRendererKind, MarkRenderContext, RadiusExpression, ResolvedDomain, ScaleRange,
    ScaleTypePreference, Theme, default_scale_type_for_data_type, is_continuous_scale,
};

/// Core-safe compiled mark metadata and planning behavior.
///
/// This trait contains the compiled-mark surface needed for channel planning,
/// scale/domain inference, legend selection, and mark default lookup. The
/// top-level chart crate layers its render hook on top while rendering still
/// needs chart runtime state.
pub trait CompiledMarkCore: Any + Send + Sync {
    /// Get the mark's state (compiled version with CompiledDataContext).
    fn state(&self) -> &CompiledMarkState;

    /// Get mutable reference to the mark's state.
    fn state_mut(&mut self) -> &mut CompiledMarkState;

    /// Get the data context for this mark.
    fn data_context(&self) -> &CompiledDataContext;

    /// Get the mark type name (e.g., "rect", "line", "symbol").
    fn mark_type(&self) -> &str;

    /// Downcast support.
    fn as_any(&self) -> &dyn Any {
        panic!("as_any not implemented for this mark type")
    }

    /// Declare channels this mark supports.
    fn supported_channels(&self) -> Vec<ChannelDescriptor>;

    /// Whether this mark type supports the order encoding channel.
    fn supports_order(&self) -> bool {
        false
    }

    /// Whether this mark needs the full DataFrame as RecordBatch.
    fn wants_full_data_batch(&self) -> bool {
        false
    }

    /// Returns the default value for a channel if not explicitly mapped.
    fn default_channel_value(
        &self,
        channel: &str,
        context: &MarkRenderContext<'_>,
    ) -> Option<ScalarValue> {
        default_channel_value_for_eval(self, channel, context.eval())
    }

    /// Mark-specific default values.
    fn mark_specific_default(&self, _channel: &str) -> Option<ScalarValue> {
        None
    }

    /// Returns expressions for computing radius/padding along a dimension.
    fn radius_expression(
        &self,
        _dimension: &str,
        _resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        None
    }

    /// Get the name of the channel used for sorting this mark's data.
    fn sorting_channel(&self) -> Option<&str> {
        if self.supports_order() {
            Some("order")
        } else {
            None
        }
    }

    /// Get the preferred legend renderer kind for a channel.
    fn preferred_legend_renderer_kind(
        &self,
        _channel: &str,
        _scale: &ConfiguredScale,
    ) -> Option<LegendRendererKind> {
        None
    }

    /// Get the preferred scale type for a channel based on data type.
    fn preferred_scale_type(
        &self,
        _channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        default_scale_type_for_data_type(data_type)
    }

    /// Get default scale options for a channel and scale type.
    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        _data_type: &DataType,
    ) -> HashMap<String, Expr> {
        let mut options = HashMap::new();

        if matches!(channel, "fill" | "stroke" | "color") && is_continuous_scale(scale_impl) {
            options.insert("nice".to_string(), lit(true));
        }

        options
    }

    /// Get default range for a channel after domain has been determined.
    fn default_channel_range(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
        _domain: &ResolvedDomain,
        _data_type: &DataType,
        _theme: &Theme,
        _params: &IndexMap<String, DatafusionScalarValue>,
    ) -> Option<ScaleRange> {
        None
    }
}

/// Resolve a compiled mark's default channel value from the base evaluation context.
pub fn default_channel_value_for_eval<M: CompiledMarkCore + ?Sized>(
    mark: &M,
    channel: &str,
    eval_ctx: &EvaluationContext,
) -> Option<ScalarValue> {
    if let Some(default) = eval_ctx.mark_default(mark.mark_type(), channel) {
        return Some(default);
    }

    mark.mark_specific_default(channel)
}

/// Extract a display title for a channel from compiled mark encodings.
///
/// Coordinate guides use this to derive default axis titles without depending
/// on the top-level render-capable compiled mark trait.
pub fn extract_channel_title_from_marks<M: CompiledMarkCore + ?Sized>(
    marks: &[Arc<M>],
    channel: &str,
    session_context: &SessionContext,
) -> Option<String> {
    for mark in marks {
        if let Some(channel_value) = mark.data_context().channels().get(channel)
            && let Some(col_name) = channel_value.as_column_name(session_context)
            && let Some(expr) = channel_value.expr(session_context)
            && !expr.column_refs().is_empty()
        {
            return Some(col_name);
        }
    }

    let secondary_channel = match channel {
        "x" => "x2",
        "y" => "y2",
        _ => return None,
    };

    for mark in marks {
        if let Some(channel_value) = mark.data_context().channels().get(secondary_channel)
            && let Some(col_name) = channel_value.as_column_name(session_context)
            && let Some(expr) = channel_value.expr(session_context)
            && !expr.column_refs().is_empty()
        {
            return Some(col_name);
        }
    }

    None
}
