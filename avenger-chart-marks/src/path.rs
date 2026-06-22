use datafusion_common::ScalarValue;

use avenger_chart_core::{
    AdjustItem, ColorChannelConfig, FillAdjustmentChannel, MarkAdjustmentCompileContext,
    MarkAdjustmentSpec, MarkAdjustmentTransform, MarkState, OpacityAdjustmentChannel,
    OpacityChannelConfig, PathAdjustmentChannels, PathChannelConfig, PathTransformChannelConfig,
    PointGeometryItem, PrimitiveMarkEffects, StrokeAdjustmentChannel, StrokeCapAdjustmentChannel,
    StrokeJoinAdjustmentChannel, StrokeWidthAdjustmentChannel, StrokeWidthChannelConfig,
    TransformMarkAdjustmentSpec, define_common_mark_channels, extract_adjustment_assignments,
    impl_mark_base_with_extra_fields,
};

pub struct PathMark<C> {
    pub(crate) state: MarkState,
    pub(crate) effects: PrimitiveMarkEffects,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base_with_extra_fields!(PathMark {
    effects: PrimitiveMarkEffects::default(),
});

impl<C> PathAdjustmentChannels for PathMark<C> {}

impl<C> FillAdjustmentChannel for PathMark<C> {}

impl<C> StrokeAdjustmentChannel for PathMark<C> {}

impl<C> StrokeWidthAdjustmentChannel for PathMark<C> {}

impl<C> StrokeCapAdjustmentChannel for PathMark<C> {}

impl<C> StrokeJoinAdjustmentChannel for PathMark<C> {}

impl<C> OpacityAdjustmentChannel for PathMark<C> {}

impl<C> PathMark<C> {
    /// Apply a render-stage expression adjustment to path anchor items.
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

    /// Apply a reusable render-stage adjustment transform to path anchor items.
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
    PathMark {
        path: {
            allow_column: true,
            with_config: PathChannelConfig,
        },
        path_transform: {
            allow_column: true,
            with_config: PathTransformChannelConfig,
        },
        fill: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        stroke: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            allow_column: false,
            with_config: StrokeWidthChannelConfig,
        },
        stroke_cap: {
            allow_column: false,
        },
        stroke_join: {
            allow_column: false,
        },
        opacity: {
            allow_column: true,
            with_config: OpacityChannelConfig,
        },
    }
}

/// Get default values for PathMark channels.
pub fn path_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "path_transform" => Some(ScalarValue::Utf8(Some("".to_string()))),
        "fill" => Some(ScalarValue::Utf8(Some("transparent".to_string()))),
        "stroke" => Some(ScalarValue::Utf8(Some("transparent".to_string()))),
        "stroke_width" => Some(ScalarValue::Float32(Some(0.0))),
        "stroke_cap" => Some(ScalarValue::Utf8(Some("butt".to_string()))),
        "stroke_join" => Some(ScalarValue::Utf8(Some("miter".to_string()))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        _ => None,
    }
}
