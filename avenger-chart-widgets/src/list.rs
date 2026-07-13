//! Data-encoded CheckboxList and RadioButtonList widgets.

use avenger_chart::transforms::Filter;
use avenger_chart::{
    event::{self, ChartEventBinding, ChartEventType},
    pixel_frame::{
        PixelFrameRectPositionChannels, PixelFrameSymbolPositionChannels,
        PixelFrameTextPositionChannels,
    },
    prelude::{
        AvengerChartError, ChartWidget, CoordinationScope, CursorStyle, IntoExpr, PixelFrame, Rect,
        Selection, SelectionUpdate, Symbol, Text, ToolExpansion, ToolParamSharing,
        WidgetAxisMeasureSpec, WidgetExpansion, WidgetExpansionContext, WidgetItemValidation,
        WidgetItems, WidgetMeasureExpr, WidgetMeasureSpec, WidgetPresentationBindings,
        WidgetStyleProperty, WidgetTextMeasureAxis,
    },
};
use datafusion::logical_expr::{Expr, lit};
use datafusion::prelude::col;

/// A data-encoded list of independently toggleable equality-selection values.
#[derive(Clone)]
pub struct CheckboxList {
    id: String,
    items: WidgetItems,
    value: Expr,
    label: Expr,
    selection: Selection,
}

impl CheckboxList {
    /// Create a checkbox list. Item sources default to `value` and `label`
    /// columns and can be remapped with [`Self::value`] and [`Self::label`].
    pub fn new(id: impl Into<String>, items: WidgetItems) -> Self {
        let id = id.into();
        Self {
            selection: Selection::new(format!("{id}__selection")).empty_selects_all(),
            id,
            items,
            value: col("value"),
            label: col("label"),
        }
    }

    /// Set the item expression stored in downstream equality clauses.
    pub fn value(mut self, value: impl IntoExpr) -> Self {
        self.value = value.into_expr();
        self
    }

    /// Set the item expression rendered as the row label.
    pub fn label(mut self, label: impl IntoExpr) -> Self {
        self.label = label.into_expr();
        self
    }

    /// Use an externally named/configured selection.
    pub fn selection(mut self, selection: &Selection) -> Self {
        self.selection = selection.clone();
        self
    }

    /// The selection managed by this widget.
    pub fn selection_spec(&self) -> &Selection {
        &self.selection
    }

    /// The downstream filtering expression. This deliberately follows the
    /// selection's empty behavior, unlike the per-row checked overlay.
    pub fn selected(&self) -> Expr {
        self.selection.predicate()
    }
}

impl ChartWidget for CheckboxList {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &'static str {
        "checkbox-list"
    }

    fn expand(
        &self,
        ctx: WidgetExpansionContext<'_>,
    ) -> Result<WidgetExpansion, AvengerChartError> {
        const VALUE: &str = "__value";
        const LABEL: &str = "__label";
        const ITEM_ID: &str = "__item_id";

        let row_height = ctx.host_style(WidgetStyleProperty::Height);
        let item_gap = ctx.host_style(WidgetStyleProperty::ItemGap);
        let row_y = col("__idx") * (row_height.clone() + item_gap.clone());
        let size = ctx.part_style("box", WidgetStyleProperty::ChoiceControlSize);
        let box_y = row_y.clone() + (row_height.clone() - size.clone()) / lit(2.0_f32);
        let label_gap = ctx.part_style("label", WidgetStyleProperty::ControlLabelGap);
        let focus_gap = ctx.part_style("focus-ring", WidgetStyleProperty::FocusGap);
        let checked = self
            .selection
            .contains_equality_value(self.value.clone(), col(VALUE));

        let row = Rect::<PixelFrame>::new()
            .id("row")
            .x(0.0)
            .x2(ctx.frame_width())
            .y(row_y.clone())
            .y2(row_y.clone() + row_height.clone());
        let box_mark = Rect::<PixelFrame>::new()
            .id("box")
            .x(0.0)
            .x2(size.clone())
            .y(box_y.clone())
            .y2(box_y.clone() + size.clone());
        let selected_box = Rect::<PixelFrame>::new()
            .id("selected-box")
            .x(0.0)
            .x2(size.clone())
            .y(box_y.clone())
            .y2(box_y.clone() + size.clone())
            .transform_no_output(Filter::new(checked.clone()), |mark| mark);
        let check = Symbol::<PixelFrame>::new()
            .id("check")
            .x(size.clone() / lit(2.0_f32))
            .y(box_y.clone() + size.clone() * lit(0.48_f32))
            .size(size.clone() * size.clone() * lit(0.42_f32))
            .shape("M -0.48 0.00 L -0.12 0.36 L 0.52 -0.42")
            .fill("transparent")
            .transform_no_output(Filter::new(checked), |mark| mark);
        let label = Text::<PixelFrame>::new()
            .id("label")
            .x(size.clone() + label_gap)
            .y(row_y.clone() + row_height.clone() / lit(2.0_f32))
            .text(col(LABEL))
            .align("left")
            .baseline("middle");
        let focus = Rect::<PixelFrame>::new()
            .id("focus-ring")
            .x(-focus_gap.clone())
            .x2(ctx.frame_width() + focus_gap.clone())
            .y(row_y.clone() - focus_gap.clone())
            .y2(row_y + row_height.clone() + focus_gap)
            .fill("transparent");

        let cursor = avenger_chart::prelude::Param::cursor(
            format!("{}__cursor", self.id),
            CursorStyle::Default,
        );
        let shared = ToolParamSharing::Explicit(CoordinationScope::Shared);
        let expansion = ToolExpansion::new()
            .selection(self.selection.clone())
            .cursor_param(cursor.clone(), shared)
            .mark(row)
            .mark(box_mark)
            .mark(selected_box)
            .mark(check)
            .mark(label)
            .mark(focus)
            .event_binding(
                ChartEventBinding::on(ChartEventType::Click)
                    .mark(format!("{}.row", self.id))
                    .set_selection(
                        &self.selection.id,
                        SelectionUpdate::toggle_equality_value(
                            self.value.clone(),
                            event::datum(VALUE),
                            event::datum(ITEM_ID),
                        ),
                    ),
            )
            .event_binding(
                ChartEventBinding::on(ChartEventType::MarkMouseEnter)
                    .mark(format!("{}.row", self.id))
                    .set_param(&cursor, event::cursor(CursorStyle::Pointer)),
            )
            .event_binding(
                ChartEventBinding::on(ChartEventType::MarkMouseLeave)
                    .mark(format!("{}.row", self.id))
                    .set_param(&cursor, event::cursor(CursorStyle::Default)),
            );

        let items = self
            .items
            .clone()
            .project(self.value.clone(), self.label.clone())
            .derive_identity(VALUE, ITEM_ID)
            .validate(WidgetItemValidation::NonNullUnique {
                columns: vec![VALUE.to_string()],
                role: "checkbox-list values".to_string(),
            })
            .validate(WidgetItemValidation::NonNullUnique {
                columns: vec![ITEM_ID.to_string()],
                role: "checkbox-list item identities".to_string(),
            });

        Ok(WidgetExpansion {
            expansion,
            items: Some(items),
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
                            data_encoded: true,
                        },
                    ]),
                    min_px: 1.0,
                    max_px: None,
                },
                height: WidgetAxisMeasureSpec::Content {
                    expr: WidgetMeasureExpr::ItemCount {
                        extent: Box::new(WidgetMeasureExpr::StyleLength {
                            part: None,
                            property: WidgetStyleProperty::Height,
                        }),
                        gap: Box::new(WidgetMeasureExpr::StyleLength {
                            part: None,
                            property: WidgetStyleProperty::ItemGap,
                        }),
                    },
                    min_px: 0.0,
                    max_px: None,
                },
            },
            presentation: WidgetPresentationBindings::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use avenger_chart::{
        marks::symbol::Symbol as PlotSymbol,
        plot::{Chart, CompiledPlot, EvaluationRequest},
        prelude::{ChartWidgetPlacementExt, ChromePosition, WidgetItemRow},
        zerod::ZeroDCoord,
    };
    use datafusion::common::ScalarValue;
    use datafusion::prelude::SessionContext;

    #[test]
    fn checkbox_list_expands_with_identity_validation_and_semantic_toggle() {
        let items = WidgetItems::Static(vec![
            WidgetItemRow::new([
                (
                    "value".to_string(),
                    ScalarValue::Utf8(Some("north".to_string())),
                ),
                (
                    "label".to_string(),
                    ScalarValue::Utf8(Some("North".to_string())),
                ),
            ]),
            WidgetItemRow::new([
                (
                    "value".to_string(),
                    ScalarValue::Utf8(Some("south".to_string())),
                ),
                (
                    "label".to_string(),
                    ScalarValue::Utf8(Some("South".to_string())),
                ),
            ]),
        ]);
        let widget = CheckboxList::new("regions", items);
        let expanded = widget
            .expand(WidgetExpansionContext::new("regions"))
            .expect("checkbox list expansion");

        assert_eq!(expanded.expansion.marks.len(), 6);
        assert_eq!(expanded.expansion.selections.len(), 1);
        assert!(matches!(
            expanded.expansion.event_bindings[0].selection_assignments[0].update,
            SelectionUpdate::ToggleEqualityValue { .. }
        ));
        let Some(WidgetItems::Configured {
            identity,
            validations,
            ..
        }) = expanded.items
        else {
            panic!("configured widget items");
        };
        let identity = identity.expect("item identity derivation");
        assert_eq!(identity.source_column, "__value");
        assert_eq!(identity.output_column, "__item_id");
        assert_eq!(validations.len(), 2);
    }

    #[tokio::test]
    async fn checkbox_list_evaluates_direct_and_after_bincode() {
        let ctx = Arc::new(SessionContext::new());
        let items = WidgetItems::Static(vec![
            WidgetItemRow::new([
                ("code".to_string(), ScalarValue::Int64(Some(1))),
                (
                    "name".to_string(),
                    ScalarValue::Utf8(Some("North".to_string())),
                ),
            ]),
            WidgetItemRow::new([
                ("code".to_string(), ScalarValue::Int64(Some(2))),
                (
                    "name".to_string(),
                    ScalarValue::Utf8(Some("South".to_string())),
                ),
            ]),
        ]);
        let compiled = Chart::<ZeroDCoord>::new()
            .canvas_size(280.0, 160.0)
            .plot_size(96.0, 80.0)
            .mark(PlotSymbol::new().size(144.0).fill("#0072B2"))
            .widget(
                CheckboxList::new("regions", items)
                    .value(col("code"))
                    .label(col("name"))
                    .position(ChromePosition::Left),
            )
            .compile(ctx.as_ref())
            .await
            .expect("compile checkbox list");
        let bytes = bincode::serialize(&compiled).expect("serialize checkbox list");
        let mut direct_session = Arc::new(compiled).instantiate(ctx.clone());
        let (_, first_metrics) = direct_session
            .evaluate_with_metrics(EvaluationRequest::new())
            .await
            .expect("evaluate direct checkbox list");
        assert_eq!(first_metrics.pipeline.widget_item_collects, 1);
        let (_, cached_metrics) = direct_session
            .evaluate_with_metrics(EvaluationRequest::new())
            .await
            .expect("reevaluate direct checkbox list");
        assert_eq!(cached_metrics.pipeline.widget_item_collects, 0);
        assert!(cached_metrics.pipeline.widget_item_cache_hits >= 1);

        let restored: CompiledPlot = bincode::deserialize(&bytes).expect("restore checkbox list");
        let mut restored_session = Arc::new(restored).instantiate(ctx);
        let (_, restored_metrics) = restored_session
            .evaluate_with_metrics(EvaluationRequest::new())
            .await
            .expect("evaluate restored checkbox list");
        assert_eq!(restored_metrics.pipeline.widget_item_collects, 1);
    }
}
