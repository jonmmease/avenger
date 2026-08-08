//! Avenger-language schemas and erased lowerers for Cartesian tools.

use std::sync::Arc;

use avenger_chart_cartesian::Cartesian;
use avenger_chart_core::{
    ChartTool, CoordinationScope, EmptySelectionBehavior, Param, Selection, SelectionCombine,
};
use avenger_chart_lang_types::{
    NativeLoweringError, ResolvedDeclaration, ResolvedValue, ToolLanguageDefinition,
};
use avenger_chart_schema::{
    EnumValueSchema, ExportSchema, KindSchema, NativeKindKey, NativeKindNamespace, PartSchema,
    PropertySchema, ValueShape,
};

use crate::{
    BoxSelection, BoxSelectionResolve, BoxZoom, LassoSelection, PanScrollZoom, PointSelection,
    UnitAspectBox,
};

pub fn definitions() -> Vec<ToolLanguageDefinition<Cartesian>> {
    vec![
        ToolLanguageDefinition {
            kind: "pan_scroll_zoom",
            schema: pan_scroll_zoom_schema(),
            lowerer: lower_pan_scroll_zoom,
        },
        ToolLanguageDefinition {
            kind: "point_selection",
            schema: point_selection_schema(),
            lowerer: lower_point_selection,
        },
        ToolLanguageDefinition {
            kind: "lasso_selection",
            schema: lasso_selection_schema(),
            lowerer: lower_lasso_selection,
        },
        ToolLanguageDefinition {
            kind: "box_selection",
            schema: box_selection_schema(),
            lowerer: lower_box_selection,
        },
        ToolLanguageDefinition {
            kind: "box_zoom",
            schema: box_zoom_schema(),
            lowerer: lower_box_zoom,
        },
    ]
}

fn tool_schema(kind: &str, docs: &str) -> KindSchema {
    let mut schema = KindSchema::new(NativeKindKey::new(NativeKindNamespace::Tool, kind), docs);
    schema
        .compatible_coordinates
        .insert("cartesian".to_string());
    schema
        .property(
            "enabled_by_default",
            PropertySchema::optional(ValueShape::Boolean, "Initial enabled state."),
        )
        .export(param_export(
            "enabled",
            "param<boolean>",
            "Whether this tool currently handles input.",
        ))
}

fn param_export(alias: &str, value_kind: &str, docs: &str) -> ExportSchema {
    ExportSchema {
        alias: alias.to_string(),
        value_kind: value_kind.to_string(),
        lazy: false,
        binding_property: None,
        default_property: None,
        docs: docs.to_string(),
    }
}

fn bound_selection_export(alias: &str, property: &str, docs: &str) -> ExportSchema {
    ExportSchema {
        alias: alias.to_string(),
        value_kind: "selection".to_string(),
        lazy: false,
        binding_property: Some(property.to_string()),
        default_property: None,
        docs: docs.to_string(),
    }
}

fn pan_scroll_zoom_schema() -> KindSchema {
    let mut schema = tool_schema(
        "pan_scroll_zoom",
        "Pointer-drag panning and wheel zoom for Cartesian scale domains.",
    )
    .property(
        "x_channel",
        optional_identifier("Horizontal scale channel name."),
    )
    .property(
        "y_channel",
        optional_identifier("Vertical scale channel name."),
    )
    .property(
        "x_domain_param",
        PropertySchema::optional(ValueShape::ScalarBinding, "Existing x-domain parameter."),
    )
    .property(
        "y_domain_param",
        PropertySchema::optional(ValueShape::ScalarBinding, "Existing y-domain parameter."),
    )
    .property(
        "x_sharing",
        optional_scope("Explicit x-domain sharing scope."),
    )
    .property(
        "y_sharing",
        optional_scope("Explicit y-domain sharing scope."),
    )
    .property(
        "drag_button",
        optional_identifier("Pointer button used for panning."),
    )
    .property(
        "scroll_zoom",
        PropertySchema::optional(ValueShape::Boolean, "Enable wheel zoom."),
    )
    .property(
        "zoom_base",
        PropertySchema::optional(ValueShape::Number, "Multiplicative wheel-zoom base."),
    )
    .property(
        "consume_wheel",
        PropertySchema::optional(ValueShape::Boolean, "Consume handled wheel events."),
    )
    .property(
        "settle_exact",
        PropertySchema::optional(
            ValueShape::Boolean,
            "Run an exact evaluation after previews.",
        ),
    );
    schema = schema.export(param_export(
        "x_domain",
        "param<fixed_size_list(float64,2)>",
        "Current x-domain override.",
    ));
    schema.export(param_export(
        "y_domain",
        "param<fixed_size_list(float64,2)>",
        "Current y-domain override.",
    ))
}

fn selection_schema(kind: &str, docs: &str) -> KindSchema {
    tool_schema(kind, docs)
        .property(
            "selection",
            PropertySchema::required(
                ValueShape::SelectionBinding,
                "Selection state updated by this tool.",
            ),
        )
        .property(
            "facet_scope",
            optional_scope("Facet scope of generated clauses."),
        )
        .property(
            "double_click_clear",
            PropertySchema::optional(ValueShape::Boolean, "Clear on double click."),
        )
        .export(bound_selection_export(
            "selection",
            "selection",
            "Selection state owned and updated by this tool.",
        ))
}

fn point_selection_schema() -> KindSchema {
    selection_schema(
        "point_selection",
        "Click-driven equality selection over one or more datum fields.",
    )
    .property(
        "fields",
        PropertySchema::required(
            ValueShape::OneOrMany(Box::new(ValueShape::Identifier)),
            "Selection field names evaluated against same-named datum fields.",
        ),
    )
    .property(
        "clause_id",
        PropertySchema::optional(
            ValueShape::SqlExpression,
            "Clause identity expression required for multiple dimensions.",
        ),
    )
    .property(
        "shift_toggle",
        PropertySchema::optional(ValueShape::Boolean, "Enable shift-click clause toggling."),
    )
}

fn lasso_selection_schema() -> KindSchema {
    selection_schema(
        "lasso_selection",
        "Freehand polygon selection over rendered mark geometry.",
    )
    .property(
        "fields",
        PropertySchema::required(
            ValueShape::OneOrMany(Box::new(ValueShape::Identifier)),
            "Selection field names evaluated against same-named datum fields.",
        ),
    )
    .property(
        "marks",
        PropertySchema::optional(
            ValueShape::OneOrMany(Box::new(ValueShape::Identifier)),
            "Optional target mark ids.",
        ),
    )
    .property(
        "drag_button",
        optional_identifier("Pointer button used for lassoing."),
    )
    .property(
        "event_path_min_distance_px",
        PropertySchema::optional(
            ValueShape::Number,
            "Minimum sampled distance between event-path points.",
        ),
    )
}

fn box_selection_schema() -> KindSchema {
    let mut schema = selection_schema(
        "box_selection",
        "Rectangular interval selection with a visible Cartesian overlay.",
    )
    .property(
        "channels",
        PropertySchema::optional(
            ValueShape::Array(Box::new(ValueShape::Identifier)),
            "Exactly two channel names, horizontal then vertical.",
        ),
    )
    .property(
        "x_channel",
        optional_identifier("Horizontal scale channel."),
    )
    .property("y_channel", optional_identifier("Vertical scale channel."))
    .property(
        "x_dimension",
        PropertySchema::optional(
            ValueShape::SqlExpression,
            "Horizontal selection expression.",
        ),
    )
    .property(
        "y_dimension",
        PropertySchema::optional(ValueShape::SqlExpression, "Vertical selection expression."),
    )
    .property(
        "resolve",
        PropertySchema::optional(
            atom(&[
                ("global", "Replace one global box clause."),
                ("union", "Union keyed box clauses."),
                ("intersect", "Intersect keyed box clauses."),
            ]),
            "Multi-box selection resolution.",
        ),
    )
    .property(
        "drag_button",
        optional_identifier("Pointer button used for dragging."),
    )
    .property(
        "repeat_cell_chrome",
        PropertySchema::optional(ValueShape::Boolean, "Draw one overlay per repeat cell."),
    )
    .property("unit_aspect_box", optional_unit_aspect())
    .export(param_export(
        "store",
        concat!(
            "store<struct(",
            "field(utf8,'id'),",
            "field(utf8,'cell_id'),",
            "field(utf8,'row_id'),",
            "field(utf8,'column_id'),",
            "field(float64,'x_min'),",
            "field(float64,'x_max'),",
            "field(float64,'y_min'),",
            "field(float64,'y_max')",
            ")>"
        ),
        "Hidden interval-row backing store.",
    ));
    schema = schema.part(selection_part());
    schema
}

fn box_zoom_schema() -> KindSchema {
    let mut schema = tool_schema(
        "box_zoom",
        "Rectangular drag zoom for Cartesian scale domains.",
    )
    .property(
        "channels",
        PropertySchema::optional(
            ValueShape::Array(Box::new(ValueShape::Identifier)),
            "Exactly two channel names, horizontal then vertical.",
        ),
    )
    .property(
        "x_channel",
        optional_identifier("Horizontal scale channel."),
    )
    .property("y_channel", optional_identifier("Vertical scale channel."))
    .property(
        "x_domain_param",
        PropertySchema::optional(ValueShape::ScalarBinding, "Existing x-domain parameter."),
    )
    .property(
        "y_domain_param",
        PropertySchema::optional(ValueShape::ScalarBinding, "Existing y-domain parameter."),
    )
    .property(
        "x_sharing",
        optional_scope("Explicit x-domain sharing scope."),
    )
    .property(
        "y_sharing",
        optional_scope("Explicit y-domain sharing scope."),
    )
    .property(
        "drag_button",
        optional_identifier("Pointer button used for dragging."),
    )
    .property(
        "min_size_px",
        PropertySchema::optional(ValueShape::Number, "Minimum accepted drag-box size."),
    )
    .property("unit_aspect_box", optional_unit_aspect());
    for (alias, kind, docs) in [
        (
            "active",
            "param<boolean>",
            "Whether a box gesture is active.",
        ),
        ("box_x0", "param<float64>", "Overlay starting x coordinate."),
        ("box_y0", "param<float64>", "Overlay starting y coordinate."),
        ("box_x1", "param<float64>", "Overlay ending x coordinate."),
        ("box_y1", "param<float64>", "Overlay ending y coordinate."),
        (
            "x_domain",
            "param<fixed_size_list(float64,2)>",
            "Current x domain.",
        ),
        (
            "y_domain",
            "param<fixed_size_list(float64,2)>",
            "Current y domain.",
        ),
    ] {
        schema = schema.export(param_export(alias, kind, docs));
    }
    schema.part(selection_part())
}

fn selection_part() -> PartSchema {
    PartSchema {
        alias: "selection".to_string(),
        runtime_kind: "rect".to_string(),
        runtime_alias: Some("selection".to_string()),
        targetable: true,
        docs: "Visible rectangular interaction overlay.".to_string(),
    }
}

fn optional_identifier(docs: &str) -> PropertySchema {
    PropertySchema::optional(ValueShape::Identifier, docs)
}

fn optional_scope(docs: &str) -> PropertySchema {
    PropertySchema::optional(ValueShape::CoordinationScope, docs)
}

fn optional_unit_aspect() -> PropertySchema {
    PropertySchema::optional(
        atom(&[
            (
                "coordinate_metric",
                "Preserve coordinate-system unit aspect.",
            ),
            ("viewport", "Preserve viewport pixel aspect."),
        ]),
        "Optional aspect constraint for the interaction box.",
    )
}

fn atom(values: &[(&str, &str)]) -> ValueShape {
    ValueShape::Atom {
        values: values
            .iter()
            .map(|(value, docs)| EnumValueSchema {
                value: (*value).to_string(),
                docs: (*docs).to_string(),
            })
            .collect(),
    }
}

fn lower_pan_scroll_zoom(
    declaration: &ResolvedDeclaration,
) -> Result<Arc<dyn ChartTool<Cartesian>>, NativeLoweringError> {
    let mut tool = PanScrollZoom::cartesian();
    if let Some(name) = &declaration.source_name {
        tool = tool.id(name.clone());
    }
    for (name, value) in &declaration.properties {
        tool = match name.as_str() {
            "x_channel" => tool.x_channel(string(name, value)?),
            "y_channel" => tool.y_channel(string(name, value)?),
            "x_domain_param" => tool.x_domain_param(param(name, value)?),
            "y_domain_param" => tool.y_domain_param(param(name, value)?),
            "x_sharing" => tool.x_sharing(scope(name, value)?),
            "y_sharing" => tool.y_sharing(scope(name, value)?),
            "drag_button" => tool.drag_button(string(name, value)?),
            "scroll_zoom" => tool.scroll_zoom(boolean(name, value)?),
            "zoom_base" => tool.zoom_base(number(name, value)?),
            "consume_wheel" => tool.consume_wheel(boolean(name, value)?),
            "settle_exact" => tool.settle_exact(boolean(name, value)?),
            "enabled_by_default" => tool.enabled_by_default(boolean(name, value)?),
            _ => return Err(invalid(name, "a registered pan_scroll_zoom property")),
        };
    }
    Ok(Arc::new(tool))
}

fn lower_point_selection(
    declaration: &ResolvedDeclaration,
) -> Result<Arc<dyn ChartTool<Cartesian>>, NativeLoweringError> {
    let selection = selection(declaration)?;
    let mut tool = PointSelection::new(&selection.id).combine(selection.combine);
    tool = match selection.empty {
        EmptySelectionBehavior::SelectAll => tool.empty_selects_all(),
        EmptySelectionBehavior::SelectNothing => tool.empty_selects_nothing(),
    };
    if let Some(name) = &declaration.source_name {
        tool = tool.id(name.clone());
    }
    for (name, value) in &declaration.properties {
        tool = match name.as_str() {
            "selection" => tool,
            "fields" => strings(name, value)?
                .into_iter()
                .fold(tool, |tool, field| tool.field(field)),
            "clause_id" => tool.clause_id(expr(name, value)?),
            "facet_scope" => tool.facet_scope(scope(name, value)?),
            "shift_toggle" => tool.shift_toggle(boolean(name, value)?),
            "double_click_clear" => tool.double_click_clear(boolean(name, value)?),
            "enabled_by_default" => tool.enabled_by_default(boolean(name, value)?),
            _ => return Err(invalid(name, "a registered point_selection property")),
        };
    }
    Ok(Arc::new(tool))
}

fn lower_lasso_selection(
    declaration: &ResolvedDeclaration,
) -> Result<Arc<dyn ChartTool<Cartesian>>, NativeLoweringError> {
    let selection = selection(declaration)?;
    let mut tool = LassoSelection::new(&selection.id).combine(selection.combine);
    tool = match selection.empty {
        EmptySelectionBehavior::SelectAll => tool.empty_selects_all(),
        EmptySelectionBehavior::SelectNothing => tool.empty_selects_nothing(),
    };
    if let Some(name) = &declaration.source_name {
        tool = tool.id(name.clone());
    }
    for (name, value) in &declaration.properties {
        tool = match name.as_str() {
            "selection" => tool,
            "fields" => strings(name, value)?
                .into_iter()
                .fold(tool, |tool, field| tool.field(field)),
            "marks" => tool.marks(strings(name, value)?),
            "facet_scope" => tool.facet_scope(scope(name, value)?),
            "drag_button" => tool.drag_button(string(name, value)?),
            "event_path_min_distance_px" => {
                tool.event_path_min_distance_px(number(name, value)? as f32)
            }
            "double_click_clear" => tool.double_click_clear(boolean(name, value)?),
            "enabled_by_default" => tool.enabled_by_default(boolean(name, value)?),
            _ => return Err(invalid(name, "a registered lasso_selection property")),
        };
    }
    Ok(Arc::new(tool))
}

fn lower_box_selection(
    declaration: &ResolvedDeclaration,
) -> Result<Arc<dyn ChartTool<Cartesian>>, NativeLoweringError> {
    let selection = selection(declaration)?;
    let mut tool = BoxSelection::cartesian(&selection.id).resolve(match selection.combine {
        SelectionCombine::Union => BoxSelectionResolve::Global,
        SelectionCombine::Intersect => BoxSelectionResolve::Intersect,
    });
    tool = match selection.empty {
        EmptySelectionBehavior::SelectAll => tool.empty_selects_all(),
        EmptySelectionBehavior::SelectNothing => tool.empty_selects_nothing(),
    };
    if let Some(name) = &declaration.source_name {
        tool = tool.id(name.clone());
    }
    for (name, value) in &declaration.properties {
        tool = match name.as_str() {
            "selection" => tool,
            "channels" => {
                let values = strings(name, value)?;
                let [x, y] = values.as_slice() else {
                    return Err(invalid(name, "exactly two channel names"));
                };
                tool.channels(x, y)
            }
            "x_channel" => tool.x_channel(string(name, value)?),
            "y_channel" => tool.y_channel(string(name, value)?),
            "x_dimension" => tool.x_dimension(expr(name, value)?),
            "y_dimension" => tool.y_dimension(expr(name, value)?),
            "resolve" => {
                let resolve = match string(name, value)? {
                    "global" => BoxSelectionResolve::Global,
                    "union" => BoxSelectionResolve::Union,
                    "intersect" => BoxSelectionResolve::Intersect,
                    _ => return Err(invalid(name, "global, union, or intersect")),
                };
                let compatible = matches!(
                    (selection.combine, resolve),
                    (SelectionCombine::Union, BoxSelectionResolve::Global)
                        | (SelectionCombine::Union, BoxSelectionResolve::Union)
                        | (SelectionCombine::Intersect, BoxSelectionResolve::Intersect)
                );
                if !compatible {
                    return Err(NativeLoweringError::Lowering {
                        kind: "box_selection".to_string(),
                        message: "resolve must agree with the bound selection's combine mode"
                            .to_string(),
                    });
                }
                tool.resolve(resolve)
            }
            "facet_scope" => tool.facet_scope(scope(name, value)?),
            "drag_button" => tool.drag_button(string(name, value)?),
            "repeat_cell_chrome" => {
                if boolean(name, value)? {
                    tool.repeat_cell_chrome()
                } else {
                    tool
                }
            }
            "double_click_clear" => tool.double_click_clear(boolean(name, value)?),
            "enabled_by_default" => tool.enabled_by_default(boolean(name, value)?),
            "unit_aspect_box" => tool.unit_aspect_box(unit_aspect(name, value)?),
            _ => return Err(invalid(name, "a registered box_selection property")),
        };
    }
    Ok(Arc::new(tool))
}

fn lower_box_zoom(
    declaration: &ResolvedDeclaration,
) -> Result<Arc<dyn ChartTool<Cartesian>>, NativeLoweringError> {
    let mut tool = BoxZoom::cartesian();
    if let Some(name) = &declaration.source_name {
        tool = tool.id(name.clone());
    }
    for (name, value) in &declaration.properties {
        tool = match name.as_str() {
            "channels" => {
                let values = strings(name, value)?;
                let [x, y] = values.as_slice() else {
                    return Err(invalid(name, "exactly two channel names"));
                };
                tool.x_channel(x).y_channel(y)
            }
            "x_channel" => tool.x_channel(string(name, value)?),
            "y_channel" => tool.y_channel(string(name, value)?),
            "x_domain_param" => tool.x_domain_param(param(name, value)?),
            "y_domain_param" => tool.y_domain_param(param(name, value)?),
            "x_sharing" => tool.x_sharing(scope(name, value)?),
            "y_sharing" => tool.y_sharing(scope(name, value)?),
            "drag_button" => tool.drag_button(string(name, value)?),
            "min_size_px" => tool.min_size_px(number(name, value)?),
            "enabled_by_default" => tool.enabled_by_default(boolean(name, value)?),
            "unit_aspect_box" => tool.unit_aspect_box(unit_aspect(name, value)?),
            _ => return Err(invalid(name, "a registered box_zoom property")),
        };
    }
    Ok(Arc::new(tool))
}

fn selection(declaration: &ResolvedDeclaration) -> Result<Selection, NativeLoweringError> {
    match declaration.get("selection")? {
        ResolvedValue::Selection(selection) => Ok(selection.clone()),
        _ => Err(invalid("selection", "selection binding")),
    }
}

fn expr(
    property: &str,
    value: &ResolvedValue,
) -> Result<datafusion::logical_expr::Expr, NativeLoweringError> {
    avenger_chart_lang_types::resolved_expr(value)
        .ok_or_else(|| invalid(property, "scalar SQL expression"))
}
fn string<'a>(property: &str, value: &'a ResolvedValue) -> Result<&'a str, NativeLoweringError> {
    let ResolvedValue::String(value) = value else {
        return Err(invalid(property, "identifier"));
    };
    Ok(value)
}
fn strings(property: &str, value: &ResolvedValue) -> Result<Vec<String>, NativeLoweringError> {
    match value {
        ResolvedValue::String(value) => Ok(vec![value.clone()]),
        ResolvedValue::Array(values) => values
            .iter()
            .map(|value| string(property, value).map(str::to_string))
            .collect(),
        _ => Err(invalid(property, "identifier or identifier array")),
    }
}
fn boolean(property: &str, value: &ResolvedValue) -> Result<bool, NativeLoweringError> {
    let ResolvedValue::Boolean(value) = value else {
        return Err(invalid(property, "boolean"));
    };
    Ok(*value)
}
fn number(property: &str, value: &ResolvedValue) -> Result<f64, NativeLoweringError> {
    match value {
        ResolvedValue::Number(value) => Ok(*value),
        ResolvedValue::Integer(value) => Ok(*value as f64),
        _ => Err(invalid(property, "number")),
    }
}
fn param(property: &str, value: &ResolvedValue) -> Result<Param, NativeLoweringError> {
    let ResolvedValue::Param(value) = value else {
        return Err(invalid(property, "parameter binding"));
    };
    Ok(value.clone())
}
fn scope(property: &str, value: &ResolvedValue) -> Result<CoordinationScope, NativeLoweringError> {
    match value {
        ResolvedValue::String(value) if value == "shared" => Ok(CoordinationScope::Shared),
        ResolvedValue::String(value) if value == "free" => Ok(CoordinationScope::Free),
        ResolvedValue::Integer(level) => (*level)
            .try_into()
            .map(CoordinationScope::Level)
            .map_err(|_| invalid(property, "scope level from 0 through 255")),
        _ => Err(invalid(property, "shared, free, or level")),
    }
}
fn unit_aspect(
    property: &str,
    value: &ResolvedValue,
) -> Result<UnitAspectBox, NativeLoweringError> {
    match string(property, value)? {
        "coordinate_metric" => Ok(UnitAspectBox::CoordinateMetric),
        "viewport" => Ok(UnitAspectBox::Viewport),
        _ => Err(invalid(property, "coordinate_metric or viewport")),
    }
}
fn invalid(property: &str, expected: &str) -> NativeLoweringError {
    NativeLoweringError::InvalidPropertyType {
        property: property.to_string(),
        expected: expected.to_string(),
    }
}
