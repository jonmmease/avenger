//! Data-encoded CheckboxList and RadioButtonList widgets.

use avenger_chart::transforms::Filter;
use avenger_chart::{
    event::{self, ChartEventBinding, ChartEventType},
    pixel_frame::{
        PixelFrameRectPositionChannels, PixelFrameSymbolPositionChannels,
        PixelFrameTextPositionChannels,
    },
    prelude::{
        AvengerChartError, ChannelExpr, ChartWidget, CoordinationScope, CursorStyle, IntoExpr,
        Param, PixelFrame, Rect, Selection, SelectionUpdate, Symbol, Text, ToolExpansion,
        ToolParamSharing, WidgetAxisMeasureSpec, WidgetExpansion, WidgetExpansionContext,
        WidgetItemValidation, WidgetItems, WidgetMeasureExpr, WidgetMeasureSpec,
        WidgetPresentationBindings, WidgetStyleProperty, WidgetTextMeasureAxis,
    },
};
use datafusion::prelude::col;
use datafusion::{
    common::ScalarValue,
    logical_expr::{Expr, lit},
};

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
        let inline_padding = ctx.host_style(WidgetStyleProperty::PaddingInline);
        let block_padding = ctx.host_style(WidgetStyleProperty::PaddingBlock);
        let row_y = block_padding.clone() + col("__idx") * (row_height.clone() + item_gap.clone());
        let size = ctx.part_style("box", WidgetStyleProperty::ChoiceControlSize);
        let box_y = row_y.clone() + (row_height.clone() - size.clone()) / lit(2.0_f32);
        let label_gap = ctx.part_style("label", WidgetStyleProperty::ControlLabelGap);
        let focus_gap = ctx.part_style("focus-ring", WidgetStyleProperty::FocusGap);
        let checked = self
            .selection
            .contains_equality_value(self.value.clone(), col(VALUE));

        let container = Rect::<PixelFrame>::new()
            .id("container")
            .x(0.0)
            .x2(ctx.frame_width())
            .y(0.0)
            .y2(ctx.frame_height());
        let row = Rect::<PixelFrame>::new()
            .id("row")
            .x(0.0)
            .x2(ctx.frame_width())
            .y(row_y.clone())
            .y2(row_y.clone() + row_height.clone());
        let box_mark = Rect::<PixelFrame>::new()
            .id("box")
            .x(inline_padding.clone())
            .x2(inline_padding.clone() + size.clone())
            .y(box_y.clone())
            .y2(box_y.clone() + size.clone());
        let selected_box = Rect::<PixelFrame>::new()
            .id("selected-box")
            .x(inline_padding.clone())
            .x2(inline_padding.clone() + size.clone())
            .y(box_y.clone())
            .y2(box_y.clone() + size.clone())
            .transform_no_output(Filter::new(checked.clone()), |mark| mark);
        let check = Symbol::<PixelFrame>::new()
            .id("check")
            .x(inline_padding.clone() + size.clone() / lit(2.0_f32))
            .y(box_y.clone() + size.clone() * lit(0.48_f32))
            .size(ChannelExpr::value(
                size.clone() * size.clone() * lit(0.42_f32),
            ))
            .shape("M -0.48 0.00 L -0.12 0.36 L 0.52 -0.42")
            .fill("transparent")
            .transform_no_output(Filter::new(checked), |mark| mark);
        let label = Text::<PixelFrame>::new()
            .id("label")
            .x(inline_padding.clone() + size.clone() + label_gap)
            .y(row_y.clone() + row_height.clone() / lit(2.0_f32))
            .text(col(LABEL))
            .align("left")
            .baseline("middle");
        let focus = Rect::<PixelFrame>::new()
            .id("focus-ring")
            .x(inline_padding.clone() - focus_gap.clone())
            .x2(ctx.frame_width() - inline_padding.clone() + focus_gap.clone())
            .y(row_y.clone() - focus_gap.clone())
            .y2(row_y + row_height.clone() + focus_gap)
            .fill("transparent");

        let cursor = avenger_chart::prelude::Param::cursor(
            format!("{}__cursor", self.id),
            CursorStyle::Default,
        );
        let targets = [
            format!("{}.box", self.id),
            format!("{}.check", self.id),
            format!("{}.label", self.id),
        ];
        let shared = ToolParamSharing::Explicit(CoordinationScope::Shared);
        let expansion = ToolExpansion::new()
            .selection(self.selection.clone())
            .cursor_param(cursor.clone(), shared)
            .mark(container)
            .mark(row)
            .mark(box_mark)
            .mark(selected_box)
            .mark(check)
            .mark(label)
            .mark(focus)
            // Toggling changes which conditional control marks exist, so the
            // scene and its interaction index must be refreshed together.
            .event_binding(
                ChartEventBinding::on(ChartEventType::Click)
                    .marks(targets.clone())
                    .set_selection(
                        &self.selection.id,
                        SelectionUpdate::toggle_equality_value_in_scope(
                            CoordinationScope::Shared,
                            self.value.clone(),
                            event::datum(VALUE),
                            event::datum(ITEM_ID),
                        ),
                    )
                    .exact(),
            )
            .event_binding(
                ChartEventBinding::on(ChartEventType::MarkMouseEnter)
                    .marks(targets.clone())
                    .set_param(&cursor, event::cursor(CursorStyle::Pointer)),
            )
            .event_binding(
                ChartEventBinding::on(ChartEventType::MarkMouseLeave)
                    .marks(targets)
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
                    expr: WidgetMeasureExpr::Max(vec![
                        WidgetMeasureExpr::StyleLength {
                            part: None,
                            property: WidgetStyleProperty::MinWidth,
                        },
                        WidgetMeasureExpr::Add(vec![
                            WidgetMeasureExpr::StyleLength {
                                part: None,
                                property: WidgetStyleProperty::PaddingInline,
                            },
                            WidgetMeasureExpr::StyleLength {
                                part: None,
                                property: WidgetStyleProperty::PaddingInline,
                            },
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
                    ]),
                    min_px: 1.0,
                    max_px: None,
                },
                height: WidgetAxisMeasureSpec::Content {
                    expr: WidgetMeasureExpr::Max(vec![
                        WidgetMeasureExpr::StyleLength {
                            part: None,
                            property: WidgetStyleProperty::MinHeight,
                        },
                        WidgetMeasureExpr::Add(vec![
                            WidgetMeasureExpr::StyleLength {
                                part: None,
                                property: WidgetStyleProperty::PaddingBlock,
                            },
                            WidgetMeasureExpr::StyleLength {
                                part: None,
                                property: WidgetStyleProperty::PaddingBlock,
                            },
                            WidgetMeasureExpr::ItemCount {
                                extent: Box::new(WidgetMeasureExpr::StyleLength {
                                    part: None,
                                    property: WidgetStyleProperty::Height,
                                }),
                                gap: Box::new(WidgetMeasureExpr::StyleLength {
                                    part: None,
                                    property: WidgetStyleProperty::ItemGap,
                                }),
                            },
                        ]),
                    ]),
                    min_px: 0.0,
                    max_px: None,
                },
            },
            presentation: WidgetPresentationBindings::default(),
        })
    }
}

/// A data-encoded list that owns one scalar selected-value parameter.
#[derive(Clone)]
pub struct RadioButtonList {
    id: String,
    items: WidgetItems,
    item_value: Expr,
    label: Expr,
    default: Option<ScalarValue>,
}

impl RadioButtonList {
    /// Create a radio-button list. Static sources using the default `value`
    /// column select their first declaration-order item by default. DataFrame
    /// sources and computed static value expressions require [`Self::default`].
    pub fn new(id: impl Into<String>, items: WidgetItems) -> Self {
        Self {
            id: id.into(),
            items,
            item_value: col("value"),
            label: col("label"),
            default: None,
        }
    }

    /// Set the item expression stored in the selected-value parameter.
    pub fn item_value(mut self, value: impl IntoExpr) -> Self {
        self.item_value = value.into_expr();
        self
    }

    /// Set the item expression rendered as the row label.
    pub fn label(mut self, label: impl IntoExpr) -> Self {
        self.label = label.into_expr();
        self
    }

    /// Set the initial selected value. The prepared item relation validates
    /// that this value exists before the widget is rendered.
    pub fn default(mut self, value: impl Into<ScalarValue>) -> Self {
        self.default = Some(value.into());
        self
    }

    /// A scalar expression for the currently selected value.
    pub fn value(&self) -> Expr {
        Param::new(self.param_name(), self.placeholder_default()).expr()
    }

    fn param_name(&self) -> String {
        format!("{}__value", self.id)
    }

    fn placeholder_default(&self) -> ScalarValue {
        self.default
            .clone()
            .or_else(|| inferred_static_default(&self.items, &self.item_value).ok())
            .unwrap_or(ScalarValue::Null)
    }

    fn resolved_param(&self) -> Result<Param, AvengerChartError> {
        let default = match &self.default {
            Some(default) => default.clone(),
            None => inferred_static_default(&self.items, &self.item_value).map_err(|message| {
                AvengerChartError::InvalidArgument(format!(
                    "RadioButtonList '{}' requires .default(...): {message}",
                    self.id
                ))
            })?,
        };
        if default.is_null() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "RadioButtonList '{}' requires a non-null default value",
                self.id
            )));
        }
        Ok(Param::new(self.param_name(), default))
    }
}

fn base_widget_items(items: &WidgetItems) -> &WidgetItems {
    match items {
        WidgetItems::Configured { source, .. } => base_widget_items(source),
        source => source,
    }
}

fn inferred_static_default(items: &WidgetItems, value: &Expr) -> Result<ScalarValue, &'static str> {
    let WidgetItems::Static(rows) = base_widget_items(items) else {
        return Err("DataFrame item sources have no declaration-time first value");
    };
    let first = rows.first().ok_or("static item sources cannot be empty")?;
    match value {
        Expr::Column(column) => first
            .values
            .get(&column.name)
            .cloned()
            .ok_or("the selected-value column is absent from the first static item"),
        Expr::Literal(value, _) => Ok(value.clone()),
        _ => Err("computed static value expressions cannot be evaluated at declaration time"),
    }
}

impl ChartWidget for RadioButtonList {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &'static str {
        "radio-button-list"
    }

    fn expand(
        &self,
        ctx: WidgetExpansionContext<'_>,
    ) -> Result<WidgetExpansion, AvengerChartError> {
        const VALUE: &str = "__value";
        const LABEL: &str = "__label";
        const ITEM_ID: &str = "__item_id";

        let selected_value = self.resolved_param()?;
        let row_height = ctx.host_style(WidgetStyleProperty::Height);
        let item_gap = ctx.host_style(WidgetStyleProperty::ItemGap);
        let inline_padding = ctx.host_style(WidgetStyleProperty::PaddingInline);
        let block_padding = ctx.host_style(WidgetStyleProperty::PaddingBlock);
        let row_y = block_padding.clone() + col("__idx") * (row_height.clone() + item_gap.clone());
        let size = ctx.part_style("control", WidgetStyleProperty::ChoiceControlSize);
        let center_size = ctx.part_style("control", WidgetStyleProperty::RadioCenterSize);
        let selected_border =
            ctx.part_style("control", WidgetStyleProperty::RadioSelectedBorderWidth);
        let control_y = row_y.clone() + row_height.clone() / lit(2.0_f32);
        let control_x = inline_padding.clone() + size.clone() / lit(2.0_f32);
        let label_gap = ctx.part_style("label", WidgetStyleProperty::ControlLabelGap);
        let focus_gap = ctx.part_style("focus-ring", WidgetStyleProperty::FocusGap);
        let selected = col(VALUE).eq(selected_value.expr());

        let container = Rect::<PixelFrame>::new()
            .id("container")
            .x(0.0)
            .x2(ctx.frame_width())
            .y(0.0)
            .y2(ctx.frame_height());
        let row = Rect::<PixelFrame>::new()
            .id("row")
            .x(0.0)
            .x2(ctx.frame_width())
            .y(row_y.clone())
            .y2(row_y.clone() + row_height.clone());
        let control = Symbol::<PixelFrame>::new()
            .id("control")
            .x(control_x.clone())
            .y(control_y.clone())
            .size(ChannelExpr::value(size.clone() * size.clone()))
            .shape("circle");
        let selected_control = Symbol::<PixelFrame>::new()
            .id("selected-control")
            .x(control_x.clone())
            .y(control_y.clone())
            .size(ChannelExpr::value(size.clone() * size.clone()))
            .shape("circle")
            .stroke_width(ChannelExpr::value(selected_border))
            .transform_no_output(Filter::new(selected.clone()), |mark| mark);
        let center = Symbol::<PixelFrame>::new()
            .id("center")
            .x(control_x)
            .y(control_y)
            .size(ChannelExpr::value(center_size.clone() * center_size))
            .shape("circle")
            .transform_no_output(Filter::new(selected), |mark| mark);
        let label = Text::<PixelFrame>::new()
            .id("label")
            .x(inline_padding.clone() + size.clone() + label_gap)
            .y(row_y.clone() + row_height.clone() / lit(2.0_f32))
            .text(col(LABEL))
            .align("left")
            .baseline("middle");
        let focus_size = size.clone() + focus_gap.clone() * lit(2.0_f32);
        let focus = Symbol::<PixelFrame>::new()
            .id("focus-ring")
            .x(inline_padding.clone() + size.clone() / lit(2.0_f32))
            .y(row_y.clone() + row_height.clone() / lit(2.0_f32))
            .size(ChannelExpr::value(focus_size.clone() * focus_size))
            .shape("circle")
            .fill("transparent");

        let cursor = Param::cursor(format!("{}__cursor", self.id), CursorStyle::Default);
        let targets = [
            format!("{}.control", self.id),
            format!("{}.center", self.id),
            format!("{}.label", self.id),
        ];
        let shared = ToolParamSharing::Explicit(CoordinationScope::Shared);
        let expansion = ToolExpansion::new()
            .param(selected_value.clone(), shared.clone())
            .cursor_param(cursor.clone(), shared)
            .mark(container)
            .mark(row)
            .mark(control)
            .mark(selected_control)
            .mark(center)
            .mark(label)
            .mark(focus)
            // Selecting a row changes conditional control-mark topology.
            .event_binding(
                ChartEventBinding::on(ChartEventType::Click)
                    .marks(targets.clone())
                    .set_param(&selected_value, event::datum(VALUE))
                    .exact(),
            )
            .event_binding(
                ChartEventBinding::on(ChartEventType::MarkMouseEnter)
                    .marks(targets.clone())
                    .set_param(&cursor, event::cursor(CursorStyle::Pointer)),
            )
            .event_binding(
                ChartEventBinding::on(ChartEventType::MarkMouseLeave)
                    .marks(targets)
                    .set_param(&cursor, event::cursor(CursorStyle::Default)),
            );

        let items = self
            .items
            .clone()
            .project(self.item_value.clone(), self.label.clone())
            .derive_identity(VALUE, ITEM_ID)
            .validate(WidgetItemValidation::NonNullUnique {
                columns: vec![VALUE.to_string()],
                role: "radio-button-list values".to_string(),
            })
            .validate(WidgetItemValidation::NonNullUnique {
                columns: vec![ITEM_ID.to_string()],
                role: "radio-button-list item identities".to_string(),
            })
            .validate(WidgetItemValidation::ContainsScalar {
                column: VALUE.to_string(),
                value: selected_value.default.clone(),
                role: "radio-button-list default value".to_string(),
            })
            .validate(WidgetItemValidation::ContainsParam {
                column: VALUE.to_string(),
                param_name: selected_value.name.clone(),
                role: "radio-button-list selected value".to_string(),
            });

        Ok(WidgetExpansion {
            expansion,
            items: Some(items),
            measure: WidgetMeasureSpec {
                width: WidgetAxisMeasureSpec::Content {
                    expr: WidgetMeasureExpr::Max(vec![
                        WidgetMeasureExpr::StyleLength {
                            part: None,
                            property: WidgetStyleProperty::MinWidth,
                        },
                        WidgetMeasureExpr::Add(vec![
                            WidgetMeasureExpr::StyleLength {
                                part: None,
                                property: WidgetStyleProperty::PaddingInline,
                            },
                            WidgetMeasureExpr::StyleLength {
                                part: None,
                                property: WidgetStyleProperty::PaddingInline,
                            },
                            WidgetMeasureExpr::StyleLength {
                                part: Some("control".to_string()),
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
                    ]),
                    min_px: 1.0,
                    max_px: None,
                },
                height: WidgetAxisMeasureSpec::Content {
                    expr: WidgetMeasureExpr::Max(vec![
                        WidgetMeasureExpr::StyleLength {
                            part: None,
                            property: WidgetStyleProperty::MinHeight,
                        },
                        WidgetMeasureExpr::Add(vec![
                            WidgetMeasureExpr::StyleLength {
                                part: None,
                                property: WidgetStyleProperty::PaddingBlock,
                            },
                            WidgetMeasureExpr::StyleLength {
                                part: None,
                                property: WidgetStyleProperty::PaddingBlock,
                            },
                            WidgetMeasureExpr::ItemCount {
                                extent: Box::new(WidgetMeasureExpr::StyleLength {
                                    part: None,
                                    property: WidgetStyleProperty::Height,
                                }),
                                gap: Box::new(WidgetMeasureExpr::StyleLength {
                                    part: None,
                                    property: WidgetStyleProperty::ItemGap,
                                }),
                            },
                        ]),
                    ]),
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
        prelude::{ChartWidgetPlacementExt, ChromePosition, Theme, WidgetItemRow},
        zerod::ZeroDCoord,
    };
    use avenger_scenegraph::marks::{mark::SceneMark, rect::SceneRectMark};
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

        assert_eq!(expanded.expansion.marks.len(), 7);
        assert_eq!(expanded.expansion.selections.len(), 1);
        for binding in &expanded.expansion.event_bindings {
            assert_eq!(
                binding.mark_ids(),
                &["regions.box", "regions.check", "regions.label"]
            );
        }
        assert!(matches!(
            expanded.expansion.event_bindings[0]
                .action
                .selection_assignments[0]
                .update,
            SelectionUpdate::ToggleEqualityValue {
                facet_scope: CoordinationScope::Shared,
                ..
            }
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

    #[test]
    fn radio_button_list_infers_first_static_value_and_rejects_ambiguous_defaults() {
        let items = WidgetItems::Static(vec![
            WidgetItemRow::new([
                ("code".to_string(), ScalarValue::Int64(Some(7))),
                (
                    "name".to_string(),
                    ScalarValue::Utf8(Some("Seven".to_string())),
                ),
            ]),
            WidgetItemRow::new([
                ("code".to_string(), ScalarValue::Int64(Some(9))),
                (
                    "name".to_string(),
                    ScalarValue::Utf8(Some("Nine".to_string())),
                ),
            ]),
        ]);
        let widget = RadioButtonList::new("numbers", items)
            .item_value(col("code"))
            .label(col("name"));
        let expanded = widget
            .expand(WidgetExpansionContext::new("numbers"))
            .expect("radio-button list expansion");

        assert_eq!(expanded.expansion.marks.len(), 7);
        assert_eq!(expanded.expansion.params[0].param.name, "numbers__value");
        for binding in &expanded.expansion.event_bindings {
            assert_eq!(
                binding.mark_ids(),
                &["numbers.control", "numbers.center", "numbers.label"]
            );
        }
        assert_eq!(
            expanded.expansion.params[0].param.default,
            ScalarValue::Int64(Some(7))
        );
        assert_eq!(
            expanded.expansion.event_bindings[0]
                .action
                .assignments
                .len(),
            1
        );
        let Some(WidgetItems::Configured { validations, .. }) = expanded.items else {
            panic!("configured widget items");
        };
        assert!(validations.iter().any(|validation| matches!(
            validation,
            WidgetItemValidation::ContainsScalar { value, .. }
                if value == &ScalarValue::Int64(Some(7))
        )));
        assert!(validations.iter().any(|validation| matches!(
            validation,
            WidgetItemValidation::ContainsParam { param_name, .. }
                if param_name == "numbers__value"
        )));

        let empty = RadioButtonList::new("empty", WidgetItems::Static(Vec::new()));
        assert!(matches!(
            empty.expand(WidgetExpansionContext::new("empty")),
            Err(AvengerChartError::InvalidArgument(message))
                if message.contains("static item sources cannot be empty")
        ));
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

    #[tokio::test]
    async fn checkbox_list_theme_geometry_moves_rows_and_hit_rects_together() {
        async fn row_geometry(theme: Theme) -> Vec<(f32, f32, bool)> {
            let ctx = SessionContext::new();
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
            let compiled = Chart::<ZeroDCoord>::new()
                .theme(theme)
                .canvas_size(280.0, 180.0)
                .plot_size(96.0, 80.0)
                .mark(PlotSymbol::new().size(144.0).fill("#0072B2"))
                .widget(CheckboxList::new("regions", items).position(ChromePosition::Left))
                .compile(&ctx)
                .await
                .expect("compile themed checkbox list");
            let evaluated = compiled
                .evaluate(&ctx, None)
                .await
                .expect("evaluate themed checkbox list");

            fn find_row(marks: &[SceneMark]) -> Option<&SceneRectMark> {
                for mark in marks {
                    match mark {
                        SceneMark::Rect(rect) if rect.name == "row" => return Some(rect),
                        SceneMark::Group(group) => {
                            if let Some(rect) = find_row(&group.marks) {
                                return Some(rect);
                            }
                        }
                        _ => {}
                    }
                }
                None
            }

            let row = find_row(&evaluated.scene_graph.marks).expect("checkbox-list row mark");
            row.y_iter()
                .copied()
                .zip(row.height_iter())
                .map(|(y, height)| (y, height, row.interactive))
                .collect()
        }

        let default = row_geometry(Theme::light()).await;
        let mut custom_theme = Theme::light();
        custom_theme
            .append_css("checkbox-list#regions { height: 40px; item-gap: 12px; }")
            .expect("custom checkbox-list CSS");
        let custom = row_geometry(custom_theme).await;

        assert_eq!(default, vec![(0.0, 32.0, true), (40.0, 32.0, true)]);
        assert_eq!(custom, vec![(0.0, 40.0, true), (52.0, 40.0, true)]);
    }
}
