//! Convenient re-exports for widget authors.

pub use crate::Checkbox;
pub use crate::style::BuiltinWidgetKind;
pub use avenger_chart::pixel_frame::{
    PixelFrame, PixelFramePositionConfig, PixelFrameRectPositionChannels,
    PixelFrameRulePositionChannels, PixelFrameSymbolPositionChannels,
    PixelFrameTextPositionChannels,
};
pub use avenger_chart::prelude::{
    CanonicalJson, ChartWidget, ChartWidgetPlacementExt, ChromePosition, NativeWidget,
    NativeWidgetMeasureSpec, NativeWidgetPlacementExt, NativeWidgetStateSpec,
    ResolvedWidgetAxisSize, ResolvedWidgetPartStyle, ResolvedWidgetStyleSet, WidgetAxisMeasureSpec,
    WidgetExpansion, WidgetExpansionContext, WidgetItemRow, WidgetItemValidation, WidgetItems,
    WidgetMeasureExpr, WidgetMeasureSpec, WidgetPartManifest, WidgetPlacement,
    WidgetPresentationBindings, WidgetPresentationState, WidgetStyleProperty, WidgetStyleValueType,
    WidgetTextMeasureAxis,
};
