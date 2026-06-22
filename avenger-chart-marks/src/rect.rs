use datafusion_common::ScalarValue;

use avenger_chart_core::{
    AdjustItem, ColorChannelConfig, MarkAdjustmentCompileContext, MarkAdjustmentSpec,
    MarkAdjustmentTransform, MarkState, OpacityChannelConfig, PrimitiveMarkEffects,
    RectGeometryItem, StrokeWidthChannelConfig, TransformMarkAdjustmentSpec,
    define_common_mark_channels, extract_adjustment_assignments, impl_mark_base_with_extra_fields,
};

use crate::IntoDerivedPrimitiveMark;

pub struct Rect<C> {
    pub(crate) state: MarkState,
    pub(crate) effects: PrimitiveMarkEffects,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base_with_extra_fields!(Rect {
    effects: PrimitiveMarkEffects::default(),
});

impl<C> Rect<C> {
    /// Apply a render-stage expression adjustment to rect items.
    pub fn adjust<F>(mut self, f: F) -> Self
    where
        F: FnOnce(AdjustItem<Self, RectGeometryItem>) -> AdjustItem<Self, RectGeometryItem>,
    {
        let adjustment = f(AdjustItem::default()).into_adjustment_spec();
        if !adjustment.is_empty() {
            self.effects
                .push_adjustment(MarkAdjustmentSpec::Expr(adjustment));
        }
        self
    }

    /// Apply a reusable render-stage adjustment transform to rect items.
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

    /// Derive a built-in primitive mark from each source rect item.
    pub fn derive<D, F>(mut self, f: F) -> Self
    where
        D: IntoDerivedPrimitiveMark<C>,
        F: FnOnce(AdjustItem<Self, RectGeometryItem>) -> D,
    {
        let derived = f(AdjustItem::default());
        self.effects.push_derived(derived.into_derived_spec());
        self
    }

    #[doc(hidden)]
    pub fn mark_effects(&self) -> &PrimitiveMarkEffects {
        &self.effects
    }
}

define_common_mark_channels! {
    Rect {
        fill: {
            with_config: ColorChannelConfig,
        },
        stroke: {
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            with_config: StrokeWidthChannelConfig,
        },
        opacity: {
            with_config: OpacityChannelConfig,
        },
        corner_radius: {},
    }
}

/// Get default values for Rect mark channels.
pub fn rect_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))),
        "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
        "stroke_width" => Some(ScalarValue::Float32(Some(1.0))),
        "corner_radius" => Some(ScalarValue::Float32(Some(0.0))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        _ => None,
    }
}
