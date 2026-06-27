use datafusion_common::ScalarValue;

use avenger_chart_core::{
    AdjustItem, AngleAdjustmentChannel, AngleChannelConfig, ChannelValue, ColorChannelConfig,
    DefinedAdjustmentChannel, MarkAdjustmentCompileContext, MarkAdjustmentSpec,
    MarkAdjustmentTransform, MarkState, OpacityAdjustmentChannel, OpacityChannelConfig,
    PointGeometryItem, PrimitiveMarkEffects, SizeChannelConfig, StrokeDashChannelConfig,
    StrokeWidthChannelConfig, TextAdjustmentChannels, TransformMarkAdjustmentSpec,
    define_common_mark_channels, extract_adjustment_assignments, impl_mark_base_with_extra_fields,
};
use avenger_text::types::TextSyntaxMode;

pub struct Text<C> {
    pub(crate) state: MarkState,
    pub(crate) effects: PrimitiveMarkEffects,
    pub(crate) syntax_mode: TextSyntaxMode,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base_with_extra_fields!(Text {
    effects: PrimitiveMarkEffects::default(),
    syntax_mode: TextSyntaxMode::Plain,
});

impl<C> AngleAdjustmentChannel for Text<C> {}

impl<C> TextAdjustmentChannels for Text<C> {}

impl<C> DefinedAdjustmentChannel for Text<C> {}

impl<C> OpacityAdjustmentChannel for Text<C> {}

impl<C> Text<C> {
    /// Apply a render-stage expression adjustment to text items.
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

    /// Apply a reusable render-stage adjustment transform to text items.
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
    pub fn text_syntax_mode(&self) -> TextSyntaxMode {
        self.syntax_mode
    }

    pub fn typst(mut self) -> Self {
        self.syntax_mode = TextSyntaxMode::TypstMarkup;
        self
    }

    pub fn plain_text(mut self) -> Self {
        self.syntax_mode = TextSyntaxMode::Plain;
        self
    }

    pub fn syntax_mode(mut self, mode: TextSyntaxMode) -> Self {
        self.syntax_mode = mode;
        self
    }
}

define_common_mark_channels! {
    Text {
        color: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        angle: {
            allow_column: true,
            with_config: AngleChannelConfig,
        },
        font_size: {
            allow_column: true,
            with_config: SizeChannelConfig,
        },
        opacity: {
            allow_column: true,
            with_config: OpacityChannelConfig,
        },
        leader_stroke: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        leader_stroke_width: {
            allow_column: true,
            with_config: StrokeWidthChannelConfig,
        },
        leader_stroke_dash: {
            allow_column: true,
            with_config: StrokeDashChannelConfig,
        },
        leader_stroke_cap: {
            allow_column: true,
        },
        leader_stroke_join: {
            allow_column: true,
        },
    }
}

impl<C> Text<C>
where
    Text<C>: Sized,
{
    pub fn text<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("text", value.into().no_scale())
    }

    pub fn align<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("align", value.into().no_scale())
    }

    pub fn baseline<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("baseline", value.into().no_scale())
    }

    pub fn font<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("font", value.into().no_scale())
    }

    pub fn font_weight<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("font_weight", value.into().no_scale())
    }

    pub fn font_style<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("font_style", value.into().no_scale())
    }

    pub fn limit<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("limit", value.into().no_scale())
    }

    pub fn defined<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("defined", value.into().no_scale())
    }

    pub fn leader<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("leader", value.into().no_scale())
    }

    pub fn leader_offset_x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("leader_offset_x", value.into().no_scale())
    }

    pub fn leader_offset_y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("leader_offset_y", value.into().no_scale())
    }

    pub fn leader_label_padding<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("leader_label_padding", value.into().no_scale())
    }

    pub fn leader_target_radius<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("leader_target_radius", value.into().no_scale())
    }

    pub fn leader_min_length<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("leader_min_length", value.into().no_scale())
    }

    pub fn leader_shape<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("leader_shape", value.into().no_scale())
    }

    pub fn leader_arrow<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("leader_arrow", value.into().no_scale())
    }

    pub fn leader_arrow_length<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("leader_arrow_length", value.into().no_scale())
    }

    pub fn leader_arrow_width<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("leader_arrow_width", value.into().no_scale())
    }
}

/// Get default values for Text mark channels.
pub fn text_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "text" => Some(ScalarValue::Utf8(Some(String::new()))),
        "color" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        "align" => Some(ScalarValue::Utf8(Some("left".to_string()))),
        "baseline" => Some(ScalarValue::Utf8(Some("alphabetic".to_string()))),
        "angle" => Some(ScalarValue::Float32(Some(0.0))),
        "font" => Some(ScalarValue::Utf8(Some("sans-serif".to_string()))),
        "font_size" => Some(ScalarValue::Float32(Some(10.0))),
        "font_weight" => Some(ScalarValue::Utf8(Some("normal".to_string()))),
        "font_style" => Some(ScalarValue::Utf8(Some("normal".to_string()))),
        "limit" => Some(ScalarValue::Float32(Some(0.0))),
        "defined" => Some(ScalarValue::Boolean(Some(true))),
        "leader" => Some(ScalarValue::Boolean(Some(false))),
        "leader_offset_x" => Some(ScalarValue::Float32(Some(0.0))),
        "leader_offset_y" => Some(ScalarValue::Float32(Some(0.0))),
        "leader_stroke" => Some(ScalarValue::Utf8(Some("rgba(0, 0, 0, 0.7)".to_string()))),
        "leader_stroke_width" => Some(ScalarValue::Float32(Some(1.0))),
        "leader_stroke_cap" => Some(ScalarValue::Utf8(Some("round".to_string()))),
        "leader_stroke_join" => Some(ScalarValue::Utf8(Some("round".to_string()))),
        "leader_label_padding" => Some(ScalarValue::Float32(Some(2.0))),
        "leader_target_radius" => Some(ScalarValue::Float32(Some(0.0))),
        "leader_min_length" => Some(ScalarValue::Float32(Some(1.0))),
        "leader_shape" => Some(ScalarValue::Utf8(Some("straight".to_string()))),
        "leader_arrow" => Some(ScalarValue::Utf8(Some("none".to_string()))),
        "leader_arrow_length" => Some(ScalarValue::Float32(Some(6.0))),
        "leader_arrow_width" => Some(ScalarValue::Float32(Some(5.0))),
        _ => None,
    }
}
