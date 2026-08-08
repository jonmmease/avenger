//! Avenger-language schemas and erased lowerers for the built-in widgets.

use avenger_chart::prelude::{
    ChartWidgetPlacementExt, ChromePosition, NativeWidgetPlacementExt, WidgetAttachment,
    WidgetItems,
};
use avenger_chart_lang_types::{
    NativeLoweringError, ResolvedDeclaration, ResolvedValue, WidgetLanguageDefinition,
};
use avenger_chart_schema::{
    EnumValueSchema, ExportSchema, KindSchema, NativeKindKey, NativeKindNamespace, PartSchema,
    PropertySchema, ValueShape,
};
use datafusion::common::ScalarValue;

use crate::{
    Button, ButtonVariant, Checkbox, CheckboxList, RadioButtonList, Slider, TextCommit, TextInput,
    style::BuiltinWidgetKind,
};

/// Complete stock widget inventory. The runtime implementation tier is
/// deliberately absent from the schema: composed and native widgets share one
/// opaque authoring contract.
pub fn definitions() -> Vec<WidgetLanguageDefinition> {
    vec![
        WidgetLanguageDefinition {
            schema: checkbox_schema(),
            lowerer: lower_checkbox,
        },
        WidgetLanguageDefinition {
            schema: button_schema(),
            lowerer: lower_button,
        },
        WidgetLanguageDefinition {
            schema: checkbox_list_schema(),
            lowerer: lower_checkbox_list,
        },
        WidgetLanguageDefinition {
            schema: radio_button_list_schema(),
            lowerer: lower_radio_button_list,
        },
        WidgetLanguageDefinition {
            schema: slider_schema(),
            lowerer: lower_slider,
        },
        WidgetLanguageDefinition {
            schema: text_input_schema(),
            lowerer: lower_text_input,
        },
    ]
}

fn widget_schema(
    kind: &str,
    runtime_kind: &str,
    docs: &str,
    parts: BuiltinWidgetKind,
) -> KindSchema {
    let mut schema = KindSchema::new(NativeKindKey::new(NativeKindNamespace::Widget, kind), docs)
        .allowed_parent("chart")
        .runtime_kind(runtime_kind)
        .property(
            "position",
            PropertySchema::required(position_shape(), "Containing chart guide-slot edge."),
        );
    for part in parts.part_manifest() {
        schema = schema.part(PartSchema {
            alias: part.name.replace('-', "_"),
            runtime_kind: runtime_kind.to_string(),
            runtime_alias: Some(part.name.clone()),
            targetable: part.interactive,
            docs: format!("The widget's `{}` visual part.", part.name),
        });
    }
    schema
}

fn position_shape() -> ValueShape {
    ValueShape::Atom {
        values: ["top", "right", "bottom", "left"]
            .into_iter()
            .map(|value| EnumValueSchema {
                value: value.to_string(),
                docs: format!("Place the widget on the chart's {value} edge."),
            })
            .collect(),
    }
}

fn checkbox_schema() -> KindSchema {
    widget_schema(
        "checkbox",
        "checkbox",
        "A scalar boolean checkbox control.",
        BuiltinWidgetKind::Checkbox,
    )
    .property(
        "label",
        PropertySchema::required(ValueShape::String, "Nonempty visible label."),
    )
    .property(
        "default",
        PropertySchema::required(ValueShape::Boolean, "Initial checked state."),
    )
    .property(
        "checked_param",
        PropertySchema::optional(
            ValueShape::ScalarBinding,
            "Existing boolean parameter bound to the checked state.",
        ),
    )
    .export(ExportSchema {
        alias: "checked".to_string(),
        value_kind: "param<boolean>".to_string(),
        lazy: false,
        binding_property: Some("checked_param".to_string()),
        default_property: Some("default".to_string()),
        docs: "The current checked state.".to_string(),
    })
}

fn button_schema() -> KindSchema {
    widget_schema(
        "button",
        "button",
        "A momentary button with a monotonic activation count.",
        BuiltinWidgetKind::Button,
    )
    .property(
        "label",
        PropertySchema::required(ValueShape::String, "Nonempty visible label."),
    )
    .property(
        "variant",
        PropertySchema::optional(
            ValueShape::Atom {
                values: [
                    ("neutral", "Neutral secondary treatment."),
                    ("accent", "Accent-filled primary treatment."),
                ]
                .into_iter()
                .map(|(value, docs)| EnumValueSchema {
                    value: value.to_string(),
                    docs: docs.to_string(),
                })
                .collect(),
            },
            "Semantic visual treatment; defaults to neutral.",
        ),
    )
    .property(
        "activation_param",
        PropertySchema::optional(
            ValueShape::ScalarBinding,
            "Existing UInt64 parameter bound to the activation count.",
        ),
    )
    .property(
        "action",
        PropertySchema::optional(
            ValueShape::StateActionBlock,
            "Ordered shared-state mutations run atomically after each activation.",
        ),
    )
    .export(ExportSchema {
        alias: "activations".to_string(),
        value_kind: "param<uint64>".to_string(),
        lazy: false,
        binding_property: Some("activation_param".to_string()),
        default_property: None,
        docs: "Number of completed activations.".to_string(),
    })
}

fn list_properties(schema: KindSchema) -> KindSchema {
    schema
        .property(
            "data",
            PropertySchema::required(ValueShape::WidgetData, "Ordered widget item relation."),
        )
        .property(
            "value",
            PropertySchema::optional(
                ValueShape::SqlExpression,
                "Item value expression; defaults to the `value` column.",
            ),
        )
        .property(
            "label",
            PropertySchema::optional(
                ValueShape::SqlExpression,
                "Item label expression; defaults to the `label` column.",
            ),
        )
        .property(
            "order_by",
            PropertySchema::optional(
                ValueShape::OneOrMany(Box::new(ValueShape::SqlExpression)),
                "Nonempty total order required for non-inline data.",
            ),
        )
}

fn checkbox_list_schema() -> KindSchema {
    list_properties(widget_schema(
        "checkbox_list",
        "checkbox-list",
        "An ordered list of independently toggleable selection values.",
        BuiltinWidgetKind::CheckboxList,
    ))
    .property(
        "selection",
        PropertySchema::optional(
            ValueShape::SelectionBinding,
            "Existing equality selection managed by the widget.",
        ),
    )
    .export(ExportSchema {
        alias: "selection".to_string(),
        value_kind: "selection".to_string(),
        lazy: false,
        binding_property: Some("selection".to_string()),
        default_property: None,
        docs: "The equality selection managed by the list.".to_string(),
    })
}

fn radio_button_list_schema() -> KindSchema {
    list_properties(widget_schema(
        "radio_button_list",
        "radio-button-list",
        "An ordered list that selects exactly one scalar value.",
        BuiltinWidgetKind::RadioButtonList,
    ))
    .property(
        "default",
        PropertySchema::optional(
            ValueShape::Any,
            "Initial selected scalar; inferred only for compatible inline data.",
        ),
    )
    .property(
        "value_param",
        PropertySchema::optional(
            ValueShape::ScalarBinding,
            "Existing item-typed parameter bound to the selected value.",
        ),
    )
    .export(ExportSchema {
        alias: "value".to_string(),
        value_kind: "param<item_scalar>".to_string(),
        lazy: false,
        binding_property: Some("value_param".to_string()),
        default_property: Some("default".to_string()),
        docs: "The currently selected item value.".to_string(),
    })
}

fn slider_schema() -> KindSchema {
    widget_schema(
        "slider",
        "slider",
        "A bounded Float64 slider control.",
        BuiltinWidgetKind::Slider,
    )
    .property(
        "min",
        PropertySchema::required(ValueShape::Number, "Finite lower bound."),
    )
    .property(
        "max",
        PropertySchema::required(ValueShape::Number, "Finite upper bound greater than min."),
    )
    .property(
        "step",
        PropertySchema::optional(
            ValueShape::Number,
            "Positive quantization step; defaults to 1.",
        ),
    )
    .property(
        "default",
        PropertySchema::optional(ValueShape::Number, "Initial value; defaults to min."),
    )
    .property(
        "title",
        PropertySchema::optional(ValueShape::String, "Visible caption above the track."),
    )
    .property(
        "format",
        PropertySchema::optional(ValueShape::String, "d3-compatible number format."),
    )
    .property(
        "throttle_ms",
        PropertySchema::optional(
            ValueShape::Integer,
            "Optional drag-update throttle interval.",
        ),
    )
    .property(
        "value_param",
        PropertySchema::optional(
            ValueShape::ScalarBinding,
            "Existing Float64 parameter bound to the value.",
        ),
    )
    .export(ExportSchema {
        alias: "value".to_string(),
        value_kind: "param<float64>".to_string(),
        lazy: false,
        binding_property: Some("value_param".to_string()),
        default_property: Some("default".to_string()),
        docs: "The current slider value.".to_string(),
    })
}

fn text_input_schema() -> KindSchema {
    widget_schema(
        "text_input",
        "text-input",
        "A native single-line UTF-8 text input.",
        BuiltinWidgetKind::TextInput,
    )
    .property(
        "default",
        PropertySchema::optional(
            ValueShape::String,
            "Initial committed value; defaults to empty.",
        ),
    )
    .property(
        "placeholder",
        PropertySchema::optional(ValueShape::String, "Hint shown while the value is empty."),
    )
    .property(
        "commit",
        PropertySchema::optional(
            ValueShape::Atom {
                values: [
                    ("on_change", "Commit after a quiet period."),
                    ("on_enter_or_blur", "Commit only on Enter or blur."),
                ]
                .into_iter()
                .map(|(value, docs)| EnumValueSchema {
                    value: value.to_string(),
                    docs: docs.to_string(),
                })
                .collect(),
            },
            "Commit policy; defaults to on_change.",
        ),
    )
    .property(
        "debounce_ms",
        PropertySchema::optional(
            ValueShape::Integer,
            "On-change quiet period; defaults to 150 ms.",
        ),
    )
    .property(
        "value_param",
        PropertySchema::optional(
            ValueShape::ScalarBinding,
            "Existing UTF-8 parameter bound to the committed value.",
        ),
    )
    .export(ExportSchema {
        alias: "value".to_string(),
        value_kind: "param<utf8>".to_string(),
        lazy: false,
        binding_property: Some("value_param".to_string()),
        default_property: Some("default".to_string()),
        docs: "The committed text value.".to_string(),
    })
    .export(ExportSchema {
        alias: "cursor_position".to_string(),
        value_kind: "param<uint64>".to_string(),
        lazy: true,
        binding_property: None,
        default_property: None,
        docs: "Lazy committed-text cursor position in grapheme units.".to_string(),
    })
    .export(ExportSchema {
        alias: "selected_text".to_string(),
        value_kind: "param<utf8>".to_string(),
        lazy: true,
        binding_property: None,
        default_property: None,
        docs: "Lazy selected committed text.".to_string(),
    })
}

fn lower_checkbox(
    declaration: &ResolvedDeclaration,
) -> Result<WidgetAttachment, NativeLoweringError> {
    let mut widget = Checkbox::new(
        widget_id(declaration)?,
        string(declaration, "label")?,
        boolean(declaration, "default")?,
    );
    if let Some(ResolvedValue::Param(param)) = declaration.properties.get("checked_param") {
        widget = widget.checked_param(param.clone());
    }
    Ok(WidgetAttachment::composed(
        widget.position(position(declaration)?),
    ))
}

fn lower_button(
    declaration: &ResolvedDeclaration,
) -> Result<WidgetAttachment, NativeLoweringError> {
    let mut widget = Button::new(widget_id(declaration)?).label(string(declaration, "label")?);
    if matches!(declaration.properties.get("variant"), Some(ResolvedValue::String(value)) if value == "accent")
    {
        widget = widget.variant(ButtonVariant::Accent);
    }
    if let Some(ResolvedValue::Param(param)) = declaration.properties.get("activation_param") {
        widget = widget.with_activation_param(param.clone());
    }
    if let Some(action) = &declaration.state_action {
        widget = widget.action(action.clone());
    }
    Ok(WidgetAttachment::composed(
        widget.position(position(declaration)?),
    ))
}

fn lower_checkbox_list(
    declaration: &ResolvedDeclaration,
) -> Result<WidgetAttachment, NativeLoweringError> {
    let mut widget = CheckboxList::new(widget_id(declaration)?, items(declaration)?);
    if let Some(ResolvedValue::Expr(value)) = declaration.properties.get("value") {
        widget = widget.value(value.clone());
    }
    if let Some(ResolvedValue::Expr(value)) = declaration.properties.get("label") {
        widget = widget.label(value.clone());
    }
    if let Some(ResolvedValue::Selection(selection)) = declaration.properties.get("selection") {
        widget = widget.selection(selection);
    }
    Ok(WidgetAttachment::composed(
        widget.position(position(declaration)?),
    ))
}

fn lower_radio_button_list(
    declaration: &ResolvedDeclaration,
) -> Result<WidgetAttachment, NativeLoweringError> {
    let mut widget = RadioButtonList::new(widget_id(declaration)?, items(declaration)?);
    if let Some(ResolvedValue::Expr(value)) = declaration.properties.get("value") {
        widget = widget.item_value(value.clone());
    }
    if let Some(ResolvedValue::Expr(value)) = declaration.properties.get("label") {
        widget = widget.label(value.clone());
    }
    if let Some(value) = declaration.properties.get("default") {
        widget = widget.default(scalar(value, "default")?);
    }
    if let Some(ResolvedValue::Param(param)) = declaration.properties.get("value_param") {
        widget = widget.value_param(param.clone());
    }
    Ok(WidgetAttachment::composed(
        widget.position(position(declaration)?),
    ))
}

fn lower_slider(
    declaration: &ResolvedDeclaration,
) -> Result<WidgetAttachment, NativeLoweringError> {
    let mut widget = Slider::new(
        widget_id(declaration)?,
        number(declaration, "min")?,
        number(declaration, "max")?,
    );
    if let Some(ResolvedValue::Number(value)) = declaration.properties.get("step") {
        widget = widget.step(*value);
    }
    if let Some(ResolvedValue::Number(value)) = declaration.properties.get("default") {
        widget = widget.default(*value);
    }
    if let Some(ResolvedValue::String(value)) = declaration.properties.get("title") {
        widget = widget.title(value.clone());
    }
    if let Some(ResolvedValue::String(value)) = declaration.properties.get("format") {
        widget = widget.format(value.clone());
    }
    if let Some(ResolvedValue::Integer(value)) = declaration.properties.get("throttle_ms") {
        widget = widget.throttle_ms(
            (*value)
                .try_into()
                .map_err(|_| invalid("throttle_ms", "non-negative integer"))?,
        );
    }
    if let Some(ResolvedValue::Param(param)) = declaration.properties.get("value_param") {
        widget = widget.value_param(param.clone());
    }
    Ok(WidgetAttachment::composed(
        widget.position(position(declaration)?),
    ))
}

fn lower_text_input(
    declaration: &ResolvedDeclaration,
) -> Result<WidgetAttachment, NativeLoweringError> {
    let mut widget = TextInput::new(widget_id(declaration)?);
    if let Some(ResolvedValue::String(value)) = declaration.properties.get("default") {
        widget = widget.initial_value(value.clone());
    }
    if let Some(ResolvedValue::String(value)) = declaration.properties.get("placeholder") {
        widget = widget.placeholder(value.clone());
    }
    if matches!(declaration.properties.get("commit"), Some(ResolvedValue::String(value)) if value == "on_enter_or_blur")
    {
        widget = widget.commit(TextCommit::OnEnterOrBlur);
    }
    if let Some(ResolvedValue::Integer(value)) = declaration.properties.get("debounce_ms") {
        widget = widget.debounce(
            (*value)
                .try_into()
                .map_err(|_| invalid("debounce_ms", "non-negative integer"))?,
        );
    }
    if let Some(ResolvedValue::Param(param)) = declaration.properties.get("value_param") {
        widget = widget.value_param(param.clone());
    }
    if declaration.live_exports.contains("cursor_position") {
        let _ = widget.cursor_position();
    }
    if declaration.live_exports.contains("selected_text") {
        let _ = widget.selected_text();
    }
    Ok(WidgetAttachment::native(
        widget.position(position(declaration)?),
    ))
}

fn widget_id(declaration: &ResolvedDeclaration) -> Result<String, NativeLoweringError> {
    declaration
        .source_name
        .clone()
        .ok_or_else(|| NativeLoweringError::Lowering {
            kind: declaration.kind.clone(),
            message: "widget requires a source binder".to_string(),
        })
}

fn position(declaration: &ResolvedDeclaration) -> Result<ChromePosition, NativeLoweringError> {
    match declaration.get("position")? {
        ResolvedValue::String(value) if value == "top" => Ok(ChromePosition::Top),
        ResolvedValue::String(value) if value == "right" => Ok(ChromePosition::Right),
        ResolvedValue::String(value) if value == "bottom" => Ok(ChromePosition::Bottom),
        ResolvedValue::String(value) if value == "left" => Ok(ChromePosition::Left),
        _ => Err(invalid("position", "top, right, bottom, or left")),
    }
}

fn items(declaration: &ResolvedDeclaration) -> Result<WidgetItems, NativeLoweringError> {
    match declaration.get("data")? {
        ResolvedValue::WidgetItems(items) => Ok(items.clone()),
        _ => Err(invalid("data", "resolved widget item relation")),
    }
}

fn string(declaration: &ResolvedDeclaration, name: &str) -> Result<String, NativeLoweringError> {
    match declaration.get(name)? {
        ResolvedValue::String(value) => Ok(value.clone()),
        _ => Err(invalid(name, "string")),
    }
}

fn boolean(declaration: &ResolvedDeclaration, name: &str) -> Result<bool, NativeLoweringError> {
    match declaration.get(name)? {
        ResolvedValue::Boolean(value) => Ok(*value),
        _ => Err(invalid(name, "boolean")),
    }
}

fn number(declaration: &ResolvedDeclaration, name: &str) -> Result<f64, NativeLoweringError> {
    match declaration.get(name)? {
        ResolvedValue::Number(value) => Ok(*value),
        _ => Err(invalid(name, "number")),
    }
}

fn scalar(value: &ResolvedValue, name: &str) -> Result<ScalarValue, NativeLoweringError> {
    match value {
        ResolvedValue::Boolean(value) => Ok(ScalarValue::Boolean(Some(*value))),
        ResolvedValue::Integer(value) => Ok(ScalarValue::Int64(Some(*value))),
        ResolvedValue::Number(value) => Ok(ScalarValue::Float64(Some(*value))),
        ResolvedValue::String(value) => Ok(ScalarValue::Utf8(Some(value.clone()))),
        ResolvedValue::Scalar(value) => Ok(value.clone()),
        _ => Err(invalid(name, "scalar literal")),
    }
}

fn invalid(property: &str, expected: &str) -> NativeLoweringError {
    NativeLoweringError::InvalidPropertyType {
        property: property.to_string(),
        expected: expected.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_cover_the_stable_widget_inventory_and_parts() {
        let definitions = definitions();
        assert_eq!(
            definitions
                .iter()
                .map(|definition| definition.schema.key.kind.as_str())
                .collect::<Vec<_>>(),
            [
                "checkbox",
                "button",
                "checkbox_list",
                "radio_button_list",
                "slider",
                "text_input"
            ]
        );
        for definition in definitions {
            assert!(!definition.schema.parts.is_empty());
            assert!(definition.schema.properties.contains_key("position"));
        }
    }
}
