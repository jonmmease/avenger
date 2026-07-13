//! Momentary Button widget.

use avenger_chart::{
    event::{self, ChartEventBinding, ChartEventType},
    pixel_frame::{PixelFrameRectPositionChannels, PixelFrameTextPositionChannels},
    prelude::{
        AvengerChartError, ChartWidget, CoordinationScope, CursorStyle, Param, PixelFrame, Rect,
        Text, ToolExpansion, ToolParamSharing, WidgetAxisMeasureSpec, WidgetExpansion,
        WidgetExpansionContext, WidgetMeasureExpr, WidgetMeasureSpec, WidgetPresentationBindings,
        WidgetStyleProperty, WidgetTextMeasureAxis,
    },
};
use datafusion::{
    common::ScalarValue,
    logical_expr::lit,
    prelude::{Expr, when},
};

/// The semantic visual treatment of a [`Button`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    /// A neutral secondary action.
    #[default]
    Neutral,
    /// An accent-filled primary action.
    Accent,
}

impl ButtonVariant {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Accent => "accent",
        }
    }
}

/// A momentary action control represented by a monotonic activation count.
#[derive(Clone, Debug)]
pub struct Button {
    id: String,
    label: String,
    activations: Param,
    variant: ButtonVariant,
}

impl Button {
    /// Create a button with an auto-registered shared activation parameter.
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            activations: Param::new(format!("{id}__activations"), 0_u64),
            id,
            label: String::new(),
            variant: ButtonVariant::Neutral,
        }
    }

    /// Set the visible button label.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Set the button's semantic visual treatment.
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Use an externally named unsigned activation parameter.
    pub fn with_activation_param(mut self, param: Param) -> Self {
        self.activations = param;
        self
    }

    /// The parameter used to identify activation changes.
    pub fn activation_param(&self) -> Param {
        self.activations.clone()
    }

    /// An expression for the number of completed activations.
    pub fn activations(&self) -> Expr {
        self.activations.expr()
    }
}

impl ChartWidget for Button {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &'static str {
        "button"
    }

    fn expand(
        &self,
        ctx: WidgetExpansionContext<'_>,
    ) -> Result<WidgetExpansion, AvengerChartError> {
        if self.label.trim().is_empty() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Button '{}' requires a nonempty label",
                self.id
            )));
        }
        if !matches!(self.activations.default, ScalarValue::UInt64(Some(_))) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Button '{}' activation parameter '{}' must have a non-null UInt64 default",
                self.id, self.activations.name
            )));
        }

        let width = ctx.frame_width();
        let height = ctx.frame_height();
        let focus_gap = ctx.part_style("focus-ring", WidgetStyleProperty::FocusGap);
        let box_mark = Rect::<PixelFrame>::new()
            .id("box")
            .x(0.0)
            .x2(width.clone())
            .y(0.0)
            .y2(height.clone());
        let label_mark = Text::<PixelFrame>::new()
            .id("label")
            .x(width.clone() / lit(2.0_f32))
            .y(height.clone() / lit(2.0_f32))
            .text(self.label.as_str())
            .align("center")
            .baseline("middle");
        let focus_mark = Rect::<PixelFrame>::new()
            .id("focus-ring")
            .x(-focus_gap.clone())
            .x2(width + focus_gap.clone())
            .y(-focus_gap.clone())
            .y2(height + focus_gap)
            .fill("transparent");

        let cursor = Param::cursor(format!("{}__cursor", self.id), CursorStyle::Default);
        let shared = ToolParamSharing::Explicit(CoordinationScope::Shared);
        let next_activation = when(
            self.activations()
                .lt(lit(ScalarValue::UInt64(Some(u64::MAX)))),
            self.activations() + lit(ScalarValue::UInt64(Some(1))),
        )
        .otherwise(lit(ScalarValue::UInt64(None)))?;
        let expansion = ToolExpansion::new()
            .param(self.activations.clone(), shared.clone())
            .cursor_param(cursor.clone(), shared)
            .mark(box_mark)
            .mark(label_mark)
            .mark(focus_mark)
            .event_binding(
                ChartEventBinding::on(ChartEventType::Click)
                    .set_param_required(&self.activations, next_activation),
            )
            .event_binding(
                ChartEventBinding::on(ChartEventType::MarkMouseEnter)
                    .set_param(&cursor, event::cursor(CursorStyle::Pointer)),
            )
            .event_binding(
                ChartEventBinding::on(ChartEventType::MarkMouseLeave)
                    .set_param(&cursor, event::cursor(CursorStyle::Default)),
            );

        Ok(WidgetExpansion {
            expansion,
            items: None,
            measure: WidgetMeasureSpec {
                width: WidgetAxisMeasureSpec::Content {
                    expr: WidgetMeasureExpr::Max(vec![
                        WidgetMeasureExpr::StyleLength {
                            part: Some("box".to_string()),
                            property: WidgetStyleProperty::ButtonMinWidth,
                        },
                        WidgetMeasureExpr::Add(vec![
                            WidgetMeasureExpr::TextExtent {
                                part: "label".to_string(),
                                axis: WidgetTextMeasureAxis::Width,
                                data_encoded: false,
                            },
                            WidgetMeasureExpr::StyleLength {
                                part: Some("box".to_string()),
                                property: WidgetStyleProperty::ButtonInlinePadding,
                            },
                            WidgetMeasureExpr::StyleLength {
                                part: Some("box".to_string()),
                                property: WidgetStyleProperty::ButtonInlinePadding,
                            },
                        ]),
                    ]),
                    min_px: 1.0,
                    max_px: None,
                },
                height: WidgetAxisMeasureSpec::Content {
                    expr: WidgetMeasureExpr::StyleLength {
                        part: None,
                        property: WidgetStyleProperty::Height,
                    },
                    min_px: 1.0,
                    max_px: None,
                },
            },
            presentation: WidgetPresentationBindings::default().variant(self.variant.as_str()),
        })
    }
}
