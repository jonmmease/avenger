use avenger_scales::scales::ConfiguredScale;
use datafusion::prelude::SessionContext;
use datafusion_common::ScalarValue;

use avenger_chart_core::{
    AdjustItem, AngleAdjustmentChannel, AngleChannelConfig, ColorChannelConfig,
    DerivedPrimitiveMarkSpec, DerivedRectMarkSpec, DerivedRuleMarkSpec, DerivedSymbolMarkSpec,
    DerivedTextMarkSpec, FillAdjustmentChannel, ItemChannelAssignment, LegendRendererKind,
    MarkAdjustmentCompileContext, MarkAdjustmentSpec, MarkAdjustmentTransform, MarkState,
    OpacityAdjustmentChannel, OpacityChannelConfig, PatternChannelValue, PointGeometryItem,
    PrimitiveMarkEffects, ShapeAdjustmentChannel, ShapeChannelConfig, SizeAdjustmentChannel,
    SizeChannelConfig, StrokeAdjustmentChannel, StrokeWidthAdjustmentChannel,
    StrokeWidthChannelConfig, TransformMarkAdjustmentSpec, define_common_mark_channels,
    extract_adjustment_assignments, impl_mark_base_with_extra_fields, is_continuous_scale,
    is_item_frame_column_name,
};

use crate::{Rect, Rule, Text};

pub struct Symbol<C> {
    pub(crate) state: MarkState,
    effects: PrimitiveMarkEffects,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base_with_extra_fields!(Symbol {
    effects: PrimitiveMarkEffects::default(),
});

impl<C> SizeAdjustmentChannel for Symbol<C> {}

impl<C> AngleAdjustmentChannel for Symbol<C> {}

impl<C> FillAdjustmentChannel for Symbol<C> {}

impl<C> StrokeAdjustmentChannel for Symbol<C> {}

impl<C> OpacityAdjustmentChannel for Symbol<C> {}

impl<C> ShapeAdjustmentChannel for Symbol<C> {}

impl<C> StrokeWidthAdjustmentChannel for Symbol<C> {}

impl<C> Symbol<C> {
    /// Apply a render-stage expression adjustment to symbol items.
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

    /// Apply a reusable render-stage adjustment transform to symbol items.
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

    /// Derive a built-in primitive mark from each source symbol item.
    pub fn derive<D, F>(mut self, f: F) -> Self
    where
        D: IntoDerivedPrimitiveMark<C>,
        F: FnOnce(AdjustItem<Self, PointGeometryItem>) -> D,
    {
        let derived = f(AdjustItem::default());
        self.effects.push_derived(derived.into_derived_spec());
        self
    }

    #[doc(hidden)]
    pub fn mark_effects(&self) -> &PrimitiveMarkEffects {
        &self.effects
    }

    /// Set the structured fill pattern channel.
    pub fn fill_pattern<V: Into<PatternChannelValue>>(self, value: V) -> Self {
        self.with_pattern_channel_value("fill_pattern", value.into())
    }

    #[doc(hidden)]
    pub fn with_mark_effects(mut self, effects: PrimitiveMarkEffects) -> Self {
        self.effects = effects;
        self
    }
}

mod private {
    pub trait SealedDerivedPrimitive<C> {}
}

pub trait IntoDerivedPrimitiveMark<C>: private::SealedDerivedPrimitive<C> {
    fn into_derived_spec(self) -> DerivedPrimitiveMarkSpec;
}

impl<C> private::SealedDerivedPrimitive<C> for Symbol<C> {}

impl<C> IntoDerivedPrimitiveMark<C> for Symbol<C> {
    fn into_derived_spec(self) -> DerivedPrimitiveMarkSpec {
        if !self.effects.is_empty() {
            panic!("Nested derived Symbol effects are not implemented yet");
        }
        DerivedPrimitiveMarkSpec::Symbol(DerivedSymbolMarkSpec::new(
            derived_assignments("Symbol", &self.state),
            self.state.zindex,
        ))
    }
}

impl<C> private::SealedDerivedPrimitive<C> for Rule<C> {}

impl<C> IntoDerivedPrimitiveMark<C> for Rule<C> {
    fn into_derived_spec(self) -> DerivedPrimitiveMarkSpec {
        DerivedPrimitiveMarkSpec::Rule(DerivedRuleMarkSpec::new(
            derived_assignments("Rule", &self.state),
            self.state.zindex,
        ))
    }
}

impl<C> private::SealedDerivedPrimitive<C> for Rect<C> {}

impl<C> IntoDerivedPrimitiveMark<C> for Rect<C> {
    fn into_derived_spec(self) -> DerivedPrimitiveMarkSpec {
        if !self.effects.is_empty() {
            panic!("Nested derived Rect effects are not implemented yet");
        }
        DerivedPrimitiveMarkSpec::Rect(DerivedRectMarkSpec::new(
            derived_assignments("Rect", &self.state),
            self.state.zindex,
        ))
    }
}

impl<C> private::SealedDerivedPrimitive<C> for Text<C> {}

impl<C> IntoDerivedPrimitiveMark<C> for Text<C> {
    fn into_derived_spec(self) -> DerivedPrimitiveMarkSpec {
        if self.effects.has_derived() {
            panic!("Nested derived Text marks are not implemented yet");
        }
        DerivedPrimitiveMarkSpec::Text(DerivedTextMarkSpec::new(
            derived_assignments("Text", &self.state),
            self.effects,
            self.state.zindex,
        ))
    }
}

fn derived_assignments(mark_name: &str, state: &MarkState) -> Vec<ItemChannelAssignment> {
    if state.data.has_explicit_data_source() || !state.data.transforms().is_empty() {
        panic!("Derived {mark_name} marks cannot declare mark-local data or data transforms yet");
    }

    let ctx = SessionContext::new();
    let mut assignments = Vec::new();
    for (channel, value) in state.data.channels() {
        let expr = value.expr(&ctx).unwrap_or_else(|| {
            panic!("Derived {mark_name} channel '{channel}' must be a single scalar expression")
        });
        for column in expr.column_refs() {
            if !is_item_frame_column_name(&column.name) {
                panic!(
                    "Derived {mark_name} channel '{channel}' referenced ordinary data column '{}'; use point.data(...) to read source data",
                    column.name
                );
            }
        }
        assignments.push(
            ItemChannelAssignment::new(channel.clone(), expr)
                .expect("Failed to serialize derived mark channel expression"),
        );
    }
    assignments
}

define_common_mark_channels! {
    Symbol {
        size: {
            with_config: SizeChannelConfig,
        },
        fill: {
            with_config: ColorChannelConfig,
        },
        stroke: {
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            allow_column: false,
            with_config: StrokeWidthChannelConfig,
        },
        shape: {
            with_config: ShapeChannelConfig,
        },
        angle: {
            with_config: AngleChannelConfig,
        },
        opacity: {
            with_config: OpacityChannelConfig,
        },
    }
}

/// Get default values for Symbol mark channels.
pub fn symbol_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "size" => Some(ScalarValue::Float32(Some(64.0))),
        "shape" => Some(ScalarValue::Utf8(Some("circle".to_string()))),
        "angle" => Some(ScalarValue::Float32(Some(0.0))),
        "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))),
        "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
        "stroke_width" => Some(ScalarValue::Float32(Some(1.0))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        _ => None,
    }
}

/// Get the preferred legend renderer for Symbol marks.
pub fn symbol_legend_renderer_kind(
    channel: &str,
    scale: &ConfiguredScale,
    position_channels: &[&str],
) -> Option<LegendRendererKind> {
    let is_continuous = is_continuous_scale(scale.scale_impl.as_ref());

    match channel {
        "fill" | "stroke" | "color" if is_continuous => Some(LegendRendererKind::Colorbar),
        "fill" | "stroke" | "color" | "size" | "shape" | "opacity" | "stroke_width" => {
            Some(LegendRendererKind::Symbol)
        }
        "angle" | "defined" | "order" => None,
        _ => {
            if position_channels.contains(&channel) {
                None
            } else {
                Some(LegendRendererKind::Symbol)
            }
        }
    }
}
