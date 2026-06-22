use datafusion_common::ScalarValue;

use avenger_chart_core::{
    AdjustItem, ImageGeometryItem, MarkAdjustmentCompileContext, MarkAdjustmentSpec,
    MarkAdjustmentTransform, MarkState, PrimitiveMarkEffects, TransformMarkAdjustmentSpec,
    define_common_mark_channels, extract_adjustment_assignments, impl_mark_base_with_extra_fields,
};

pub struct Image<C> {
    pub(crate) state: MarkState,
    pub(crate) effects: PrimitiveMarkEffects,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base_with_extra_fields!(Image {
    effects: PrimitiveMarkEffects::default(),
});

impl<C> Image<C> {
    /// Apply a render-stage expression adjustment to image items.
    pub fn adjust<F>(mut self, f: F) -> Self
    where
        F: FnOnce(AdjustItem<Self, ImageGeometryItem>) -> AdjustItem<Self, ImageGeometryItem>,
    {
        let adjustment = f(AdjustItem::default()).into_adjustment_spec();
        if !adjustment.is_empty() {
            self.effects
                .push_adjustment(MarkAdjustmentSpec::Expr(adjustment));
        }
        self
    }

    /// Apply a reusable render-stage adjustment transform to image items.
    pub fn adjust_transform<A, F>(self, adjustment: A, f: F) -> Self
    where
        A: MarkAdjustmentTransform,
        F: FnOnce(Self, A::Output) -> Self,
    {
        let stage_index = self.effects.adjustments.len();
        let original_channels = self.state.data.channels().clone();
        let (compiled, output) = adjustment
            .compile(MarkAdjustmentCompileContext::new(stage_index))
            .expect("Failed to build mark adjustment transform");
        let mut routed = f(self, output);
        let (channels, assignments) = extract_adjustment_assignments(
            routed.state.data.channels().clone(),
            &original_channels,
        );
        routed.state.data = routed.state.data.with_channels(channels);
        routed
            .effects
            .push_adjustment(MarkAdjustmentSpec::Transform(
                TransformMarkAdjustmentSpec::new(compiled, assignments),
            ));
        routed
    }

    #[doc(hidden)]
    pub fn mark_effects(&self) -> &PrimitiveMarkEffects {
        &self.effects
    }
}

define_common_mark_channels! {
    Image {
        image: {
            allow_column: true,
        },
        width: {
            allow_column: true,
        },
        height: {
            allow_column: true,
        },
        align: {
            allow_column: true,
        },
        baseline: {
            allow_column: true,
        },
        aspect: {
            allow_column: false,
        },
        smooth: {
            allow_column: false,
        },
    }
}

/// Get default values for Image mark channels.
pub fn image_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "width" => Some(ScalarValue::Float32(Some(0.0))),
        "height" => Some(ScalarValue::Float32(Some(0.0))),
        "align" => Some(ScalarValue::Utf8(Some("left".to_string()))),
        "baseline" => Some(ScalarValue::Utf8(Some("top".to_string()))),
        "aspect" => Some(ScalarValue::Boolean(Some(true))),
        "smooth" => Some(ScalarValue::Boolean(Some(true))),
        _ => None,
    }
}
