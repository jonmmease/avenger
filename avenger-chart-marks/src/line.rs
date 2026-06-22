use datafusion::arrow::{array::ArrayRef, compute::kernels::cast::cast, datatypes::DataType};
use datafusion_common::ScalarValue;

use avenger_chart_core::{
    AdjustItem, AvengerChartError, ColorChannelConfig, DefinedAdjustmentChannel,
    MarkAdjustmentCompileContext, MarkAdjustmentSpec, MarkAdjustmentTransform, MarkState,
    OpacityAdjustmentChannel, OpacityChannelConfig, PointGeometryItem, PrimitiveMarkEffects,
    StrokeAdjustmentChannel, StrokeCapAdjustmentChannel, StrokeDashAdjustmentChannel,
    StrokeDashChannelConfig, StrokeJoinAdjustmentChannel, StrokeWidthAdjustmentChannel,
    StrokeWidthChannelConfig, TransformMarkAdjustmentSpec, define_common_mark_channels,
    extract_adjustment_assignments, impl_mark_base_with_extra_fields,
};

pub struct Line<C> {
    pub(crate) state: MarkState,
    pub(crate) effects: PrimitiveMarkEffects,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base_with_extra_fields!(Line {
    effects: PrimitiveMarkEffects::default(),
});

impl<C> StrokeWidthAdjustmentChannel for Line<C> {}

impl<C> StrokeAdjustmentChannel for Line<C> {}

impl<C> OpacityAdjustmentChannel for Line<C> {}

impl<C> DefinedAdjustmentChannel for Line<C> {}

impl<C> StrokeDashAdjustmentChannel for Line<C> {}

impl<C> StrokeCapAdjustmentChannel for Line<C> {}

impl<C> StrokeJoinAdjustmentChannel for Line<C> {}

impl<C> Line<C> {
    /// Apply a render-stage expression adjustment to line vertices.
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

    /// Apply a reusable render-stage adjustment transform to line vertices.
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
    Line {
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

/// Partitioning key for multi-series lines.
#[doc(hidden)]
#[derive(Hash, Eq, PartialEq, Debug, Clone, Ord, PartialOrd)]
pub struct PartitionKey {
    pub stroke: Option<usize>,
    pub width: Option<usize>,
    pub dash: Option<usize>,
    pub opacity: Option<usize>,
}

/// Convert an array to dictionary encoding for efficient partitioning.
pub fn ensure_dictionary_array(array: &ArrayRef) -> Result<ArrayRef, AvengerChartError> {
    match array.data_type() {
        DataType::Dictionary(_, _) => Ok(array.clone()),
        _ => {
            let dict_type = DataType::Dictionary(
                Box::new(DataType::Int16),
                Box::new(array.data_type().clone()),
            );
            Ok(cast(array, &dict_type)?)
        }
    }
}

/// Get default values for Line mark channels.
pub fn line_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
        "stroke_width" => Some(ScalarValue::Float32(Some(2.0))),
        "stroke_cap" => Some(ScalarValue::Utf8(Some("round".to_string()))),
        "stroke_join" => Some(ScalarValue::Utf8(Some("round".to_string()))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        "interpolate" => Some(ScalarValue::Utf8(Some("linear".to_string()))),
        "defined" => Some(ScalarValue::Boolean(Some(true))),
        _ => None,
    }
}
