use datafusion_common::ScalarValue;

use avenger_chart_core::{
    AdjustItem, ColorChannelConfig, MarkAdjustmentCompileContext, MarkAdjustmentSpec,
    MarkAdjustmentTransform, MarkState, OpacityChannelConfig, PrimitiveMarkEffects,
    RuleGeometryItem, StrokeDashChannelConfig, StrokeWidthChannelConfig,
    TransformMarkAdjustmentSpec, define_common_mark_channels, extract_adjustment_assignments,
    impl_mark_base_with_extra_fields,
};

pub struct Rule<C> {
    pub(crate) state: MarkState,
    pub(crate) effects: PrimitiveMarkEffects,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base_with_extra_fields!(Rule {
    effects: PrimitiveMarkEffects::default(),
});

impl<C> Rule<C> {
    /// Apply a render-stage expression adjustment to rule items.
    pub fn adjust<F>(mut self, f: F) -> Self
    where
        F: FnOnce(AdjustItem<Self, RuleGeometryItem>) -> AdjustItem<Self, RuleGeometryItem>,
    {
        let adjustment = f(AdjustItem::default()).into_adjustment_spec();
        if !adjustment.is_empty() {
            self.effects
                .push_adjustment(MarkAdjustmentSpec::Expr(adjustment));
        }
        self
    }

    /// Apply a reusable render-stage adjustment transform to rule items.
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
    Rule {
        stroke: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            allow_column: true,
            with_config: StrokeWidthChannelConfig,
        },
        stroke_dash: {
            allow_column: true,
            with_config: StrokeDashChannelConfig,
        },
        stroke_cap: {
            allow_column: true,
        },
        opacity: {
            allow_column: true,
            with_config: OpacityChannelConfig,
        },
    }
}

/// Get default values for Rule mark channels.
pub fn rule_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
        "stroke_width" => Some(ScalarValue::Float32(Some(1.0))),
        "stroke_cap" => Some(ScalarValue::Utf8(Some("butt".to_string()))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        _ => None,
    }
}
