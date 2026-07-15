//! Scalar Checkbox widget.

use std::sync::Arc;

use avenger_chart::{
    event::{self, ChartEventBinding, ChartEventType},
    pixel_frame::{
        PixelFrameRectPositionChannels, PixelFrameRulePositionChannels,
        PixelFrameTextPositionChannels,
    },
    prelude::{
        AvengerChartError, ChartWidget, CoordinationScope, CursorStyle, Param, PixelFrame, Rect,
        Rule, Text, ToolBehaviorExpansion, ToolParamSharing, WidgetAxisMeasureSpec,
        WidgetExpansion, WidgetExpansionContext, WidgetMeasureExpr, WidgetMeasureSpec,
        WidgetPresentationBindings, WidgetStyleProperty, WidgetTextMeasureAxis,
    },
};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float32Array},
        datatypes::{Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    logical_expr::{Expr, lit},
    prelude::{SessionContext, col},
};

/// A scalar boolean control rendered as compiler-owned scene-graph marks.
#[derive(Clone, Debug)]
pub struct Checkbox {
    id: String,
    label: String,
    checked: Param,
}

impl Checkbox {
    /// Create a checkbox with an auto-registered shared boolean parameter.
    pub fn new(id: impl Into<String>, label: impl Into<String>, checked: bool) -> Self {
        let id = id.into();
        Self {
            checked: {
                let __avenger_param_name = format!("{id}__checked");
                let __avenger_param_default: datafusion::common::ScalarValue = (checked).into();
                Param::typed(
                    __avenger_param_name,
                    __avenger_param_default.data_type(),
                    __avenger_param_default,
                )
                .expect("a parameter default must match its selected physical type")
            },
            id,
            label: label.into(),
        }
    }

    /// Use an externally named boolean parameter for the checked state.
    pub fn checked_param(mut self, param: Param) -> Self {
        self.checked = param;
        self
    }

    /// The checked-state parameter.
    pub fn param(&self) -> &Param {
        &self.checked
    }

    /// A boolean expression for the current checked state.
    pub fn checked(&self) -> Expr {
        self.checked.expr()
    }
}

impl ChartWidget for Checkbox {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &'static str {
        "checkbox"
    }

    fn expand(
        &self,
        ctx: WidgetExpansionContext<'_>,
    ) -> Result<WidgetExpansion, AvengerChartError> {
        if self.label.trim().is_empty() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Checkbox '{}' requires a nonempty label",
                self.id
            )));
        }
        if !matches!(self.checked.default, ScalarValue::Boolean(Some(_))) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Checkbox '{}' checked parameter '{}' must have a non-null boolean default",
                self.id, self.checked.name
            )));
        }

        let size = ctx.part_style("box", WidgetStyleProperty::ChoiceControlSize);
        let row_height = ctx.frame_height();
        let box_y = (row_height.clone() - size.clone()) / lit(2.0_f32);
        let label_gap = ctx.part_style("label", WidgetStyleProperty::ControlLabelGap);
        let focus_gap = ctx.part_style("focus-ring", WidgetStyleProperty::FocusGap);
        let check_center_x = size.clone() / lit(2.0_f32);
        let check_center_y = box_y.clone() + size.clone() * lit(0.45_f32);

        let box_mark = Rect::<PixelFrame>::new()
            .id("box")
            .x(0.0)
            .x2(size.clone())
            .y(box_y.clone())
            .y2(box_y.clone() + size.clone());
        let check_mark = Rule::<PixelFrame>::new()
            .id("check")
            .data(checkmark_segments()?)
            .x(check_center_x.clone() + col("x"))
            .x2(check_center_x + col("x2"))
            .y(check_center_y.clone() + col("y"))
            .y2(check_center_y + col("y2"))
            .visible(self.checked());
        let label_mark = Text::<PixelFrame>::new()
            .id("label")
            .x(size.clone() + label_gap)
            .y(row_height / lit(2.0_f32))
            .text(self.label.as_str())
            .align("left")
            .baseline("middle");
        let focus_mark = Rect::<PixelFrame>::new()
            .id("focus-ring")
            .x(-focus_gap.clone())
            .x2(size.clone() + focus_gap.clone())
            .y(box_y.clone() - focus_gap.clone())
            .y2(box_y + size.clone() + focus_gap)
            .fill("transparent");

        let shared = ToolParamSharing::Explicit(CoordinationScope::Shared);
        let behavior = ToolBehaviorExpansion::new(ctx.behavior_instance_id.clone())
            .param(self.checked.clone(), shared)
            .mark(box_mark)
            .mark(check_mark)
            .mark(label_mark)
            .mark(focus_mark)
            // The check mark is conditional, so keep the interaction index in
            // sync with the newly evaluated scene after each toggle.
            .event_binding(
                ChartEventBinding::on(ChartEventType::Click)
                    .set_param(&self.checked, Expr::Not(Box::new(self.checked())))
                    .exact(),
            )
            .event_binding(
                ChartEventBinding::on(ChartEventType::MarkMouseEnter)
                    .set_cursor(event::cursor(CursorStyle::Pointer)),
            )
            .event_binding(
                ChartEventBinding::on(ChartEventType::MarkMouseLeave)
                    .set_cursor(event::cursor(CursorStyle::Default)),
            );

        Ok(WidgetExpansion {
            instance_id: ctx.instance_id,
            behavior,
            items: None,
            measure: WidgetMeasureSpec {
                width: WidgetAxisMeasureSpec::Content {
                    expr: WidgetMeasureExpr::Add(vec![
                        WidgetMeasureExpr::StyleLength {
                            part: Some("box".to_string()),
                            property: WidgetStyleProperty::ChoiceControlSize,
                        },
                        WidgetMeasureExpr::StyleLength {
                            part: Some("label".to_string()),
                            property: WidgetStyleProperty::ControlLabelGap,
                        },
                        WidgetMeasureExpr::TextExtent {
                            part: "label".to_string(),
                            axis: WidgetTextMeasureAxis::Width,
                            data_encoded: false,
                        },
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
            presentation: WidgetPresentationBindings::default().checked(self.checked()),
        })
    }
}

fn checkmark_segments() -> Result<DataFrame, AvengerChartError> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", datafusion::arrow::datatypes::DataType::Float32, false),
        Field::new("y", datafusion::arrow::datatypes::DataType::Float32, false),
        Field::new("x2", datafusion::arrow::datatypes::DataType::Float32, false),
        Field::new("y2", datafusion::arrow::datatypes::DataType::Float32, false),
    ]));
    let columns: Vec<ArrayRef> = vec![
        Arc::new(Float32Array::from(vec![-4.0, -1.0])),
        Arc::new(Float32Array::from(vec![0.0, 2.5])),
        Arc::new(Float32Array::from(vec![-1.0, 4.0])),
        Arc::new(Float32Array::from(vec![2.5, -2.5])),
    ];
    let batch = RecordBatch::try_new(schema, columns)?;
    SessionContext::new()
        .read_batch(batch)
        .map_err(AvengerChartError::DataFusionError)
}
