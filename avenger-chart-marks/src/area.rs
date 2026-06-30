use datafusion_common::ScalarValue;

use avenger_chart_core::{
    AdjustItem, AreaGeometryItem, ColorChannelConfig, MarkAdjustmentCompileContext,
    MarkAdjustmentSpec, MarkAdjustmentTransform, MarkState, OpacityChannelConfig,
    PatternChannelValue, PrimitiveMarkEffects, StrokeDashChannelConfig, StrokeWidthChannelConfig,
    TransformMarkAdjustmentSpec, define_common_mark_channels, extract_adjustment_assignments,
    impl_mark_base_with_extra_fields,
};

pub struct Area<C> {
    pub(crate) state: MarkState,
    pub(crate) effects: PrimitiveMarkEffects,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base_with_extra_fields!(Area {
    effects: PrimitiveMarkEffects::default(),
});

impl<C> Area<C> {
    /// Apply a render-stage expression adjustment to area vertices.
    pub fn adjust<F>(mut self, f: F) -> Self
    where
        F: FnOnce(AdjustItem<Self, AreaGeometryItem>) -> AdjustItem<Self, AreaGeometryItem>,
    {
        let adjustment = f(AdjustItem::default()).into_adjustment_spec();
        if !adjustment.is_empty() {
            self.effects
                .push_adjustment(MarkAdjustmentSpec::Expr(adjustment));
        }
        self
    }

    /// Apply a reusable render-stage adjustment transform to area vertices.
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

    /// Set the structured fill pattern channel.
    pub fn fill_pattern<V: Into<PatternChannelValue>>(self, value: V) -> Self {
        self.with_pattern_channel_value("fill_pattern", value.into())
    }
}

define_common_mark_channels! {
    Area {
        orientation: {
            allow_column: false,
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
            allow_column: true,
            with_config: StrokeWidthChannelConfig,
        },
        stroke_dash: {
            allow_column: true,
            with_config: StrokeDashChannelConfig,
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
        defined: {},
        order: {
            allow_column: true,
        },
    }
}

/// Partitioning key for area marks with varying scalar style fields.
#[doc(hidden)]
#[derive(Hash, Eq, PartialEq, Debug, Clone, Ord, PartialOrd)]
pub struct AreaPartitionKey {
    pub fill: Option<usize>,
    pub stroke: Option<usize>,
    pub width: Option<usize>,
    pub dash: Option<usize>,
    pub opacity: Option<usize>,
}

/// Get default values for Area mark channels.
pub fn area_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "orientation" => Some(ScalarValue::Utf8(Some("vertical".to_string()))),
        "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))),
        "stroke" => Some(ScalarValue::Utf8(Some("transparent".to_string()))),
        "stroke_width" => Some(ScalarValue::Float32(Some(0.0))),
        "stroke_cap" => Some(ScalarValue::Utf8(Some("butt".to_string()))),
        "stroke_join" => Some(ScalarValue::Utf8(Some("round".to_string()))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        "defined" => Some(ScalarValue::Boolean(Some(true))),
        _ => None,
    }
}
