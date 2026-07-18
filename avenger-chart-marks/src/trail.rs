use datafusion_common::ScalarValue;

use avenger_chart_core::{
    AdjustItem, ColorChannelConfig, DefinedAdjustmentChannel, MarkAdjustmentCompileContext,
    MarkAdjustmentSpec, MarkAdjustmentTransform, MarkState, OpacityAdjustmentChannel,
    OpacityChannelConfig, PointGeometryItem, PrimitiveMarkEffects, SizeAdjustmentChannel,
    SizeChannelConfig, StrokeAdjustmentChannel, TransformMarkAdjustmentSpec,
    define_common_mark_channels, extract_adjustment_assignments, impl_mark_base_with_extra_fields,
};

pub struct Trail<C> {
    pub(crate) state: MarkState,
    pub(crate) effects: PrimitiveMarkEffects,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base_with_extra_fields!(Trail {
    effects: PrimitiveMarkEffects::default(),
});

impl<C> SizeAdjustmentChannel for Trail<C> {}

impl<C> StrokeAdjustmentChannel for Trail<C> {}

impl<C> OpacityAdjustmentChannel for Trail<C> {}

impl<C> DefinedAdjustmentChannel for Trail<C> {}

impl<C> Trail<C> {
    /// Apply a render-stage expression adjustment to trail vertices.
    pub fn adjust<F>(mut self, f: F) -> Self
    where
        F: FnOnce(AdjustItem<Self, PointGeometryItem>) -> AdjustItem<Self, PointGeometryItem>,
    {
        let adjustment = f(AdjustItem::default()).into_adjustment_spec();
        if !adjustment.is_empty() {
            self.effects
                .push_adjustment(MarkAdjustmentSpec::Expr(adjustment));
        }
        self
    }

    /// Apply a reusable render-stage adjustment transform to trail vertices.
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

    #[doc(hidden)]
    pub fn with_mark_effects(mut self, effects: PrimitiveMarkEffects) -> Self {
        self.effects = effects;
        self
    }
}

define_common_mark_channels! {
    Trail {
        size: {
            allow_column: true,
            with_config: SizeChannelConfig,
        },
        stroke: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        opacity: {
            allow_column: true,
            with_config: OpacityChannelConfig,
        },
        defined: {},
        order: {
            allow_column: true,
        },
    }
}

/// Partitioning key for trail marks with varying scalar style fields.
#[doc(hidden)]
#[derive(Hash, Eq, PartialEq, Debug, Clone, Ord, PartialOrd)]
pub struct TrailPartitionKey {
    pub stroke: Option<usize>,
    pub opacity: Option<usize>,
}

/// Get default values for Trail mark channels.
pub fn trail_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "size" => Some(ScalarValue::Float32(Some(1.0))),
        "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        "defined" => Some(ScalarValue::Boolean(Some(true))),
        _ => None,
    }
}
