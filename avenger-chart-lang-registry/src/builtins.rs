//! Bootstrap built-ins used to prove the public registry mechanism end to end.
//!
//! This is intentionally not the complete Avenger v1 inventory. The language
//! compiler plan owns the family-by-family expansion from this slice.

use std::{collections::BTreeMap, sync::Arc};

use avenger_chart::{
    layout::LayoutSpec,
    prelude::{
        Cartesian, CartesianAxis, CartesianSymbolPositionChannels, ChartWidgetPlacementExt,
        ChromePosition, IntoPlotMark, Legend, Linear, PanScrollZoom, Scale, Symbol,
        WidgetAttachment, WidgetItemRow, WidgetItems,
    },
};
use avenger_chart_core::DataTransform;
use avenger_chart_schema::{
    BodyMode, ChannelSchema, EnumValueSchema, ExportSchema, KindSchema, NativeKindKey,
    NativeKindNamespace, PartSchema, PropertySchema, TransformOutputSchema, ValueShape,
};
use avenger_chart_transforms::{Aggregate, Filter, Sql};
use avenger_chart_widgets::RadioButtonList;
use datafusion::{common::ScalarValue, logical_expr::col};
use indexmap::IndexMap;

use crate::{
    CoordinatePack, LoweredTransform, NativeRegistry, NativeRegistryBuilder, RegistryError,
    ResolvedValue, expr_property, string_property,
};

pub const BOOTSTRAP_PROFILE_LABEL: &str = "bootstrap-vertical-slice";

pub fn bootstrap_registry() -> Result<NativeRegistry, RegistryError> {
    let mut builder = NativeRegistryBuilder::new(1, BOOTSTRAP_PROFILE_LABEL);
    register_bootstrap_builtins(&mut builder)?;
    builder.build()
}

pub fn register_bootstrap_builtins(
    builder: &mut NativeRegistryBuilder,
) -> Result<(), RegistryError> {
    builder.register_coordinate_pack(cartesian_base_pack())?;
    builder.register_mark::<Cartesian>("cartesian", "symbol", symbol_schema(), lower_symbol)?;
    builder.register_tool::<Cartesian>(
        "cartesian",
        "pan_scroll_zoom",
        pan_scroll_zoom_schema(),
        lower_pan_scroll_zoom,
    )?;
    register_bootstrap_noncoordinate_builtins(builder)
}

/// Register the coordinate-independent bootstrap families. Downstream hosts
/// can use this when assembling a custom coordinate-pack set manually.
pub fn register_bootstrap_noncoordinate_builtins(
    builder: &mut NativeRegistryBuilder,
) -> Result<(), RegistryError> {
    register_transforms(builder)?;
    register_radio_button_list(builder)?;
    register_objects(builder)
}

pub fn cartesian_pack() -> CoordinatePack<Cartesian> {
    cartesian_base_pack()
        .mark("symbol", symbol_schema(), lower_symbol)
        .tool(
            "pan_scroll_zoom",
            pan_scroll_zoom_schema(),
            lower_pan_scroll_zoom,
        )
}

fn cartesian_base_pack() -> CoordinatePack<Cartesian> {
    let coordinate = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Coordinate, "cartesian"),
        "A two-dimensional Cartesian coordinate system.",
    )
    .body_mode(BodyMode::Mixed)
    .property(
        "unit_aspect",
        PropertySchema::optional(
            ValueShape::Number,
            "Optional positive ratio between x and y data units.",
        ),
    );

    CoordinatePack::new("cartesian", coordinate, |declaration| {
        let mut coordinate = Cartesian::new();
        if let Some(ResolvedValue::Number(ratio)) = declaration.properties.get("unit_aspect") {
            coordinate = coordinate.unit_aspect(*ratio);
        }
        Ok(coordinate)
    })
}

fn pan_scroll_zoom_schema() -> KindSchema {
    let mut tool = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Tool, "pan_scroll_zoom"),
        "Pointer-drag panning and wheel zoom for Cartesian domains.",
    )
    .export(ExportSchema {
        alias: "x_domain".to_string(),
        value_kind: "param<fixed_size_list<float64, 2>>".to_string(),
        binding_property: None,
        default_property: None,
        docs: "The tool-owned current x domain.".to_string(),
    })
    .export(ExportSchema {
        alias: "y_domain".to_string(),
        value_kind: "param<fixed_size_list<float64, 2>>".to_string(),
        binding_property: None,
        default_property: None,
        docs: "The tool-owned current y domain.".to_string(),
    });
    tool.compatible_coordinates.insert("cartesian".to_string());
    tool
}

fn lower_symbol(
    declaration: &crate::ResolvedDeclaration,
) -> Result<Vec<avenger_chart::prelude::PlotMark<Cartesian>>, RegistryError> {
    let mut mark = Symbol::<Cartesian>::new();
    for (name, value) in &declaration.properties {
        let ResolvedValue::Expr(expr) = value else {
            return Err(RegistryError::InvalidPropertyType {
                property: name.clone(),
                expected: "SQL expression".to_string(),
            });
        };
        mark = match name.as_str() {
            "x" => mark.x(expr.clone()),
            "y" => mark.y(expr.clone()),
            "fill_pattern" => mark.fill_pattern(expr.clone()),
            channel => mark.with_channel_value(channel, expr.clone().into()),
        };
    }
    Ok(mark.into_plot_marks())
}

fn lower_pan_scroll_zoom(
    _declaration: &crate::ResolvedDeclaration,
) -> Result<Arc<dyn avenger_chart::prelude::ChartTool<Cartesian>>, RegistryError> {
    Ok(Arc::new(PanScrollZoom::cartesian()))
}

pub fn symbol_schema() -> KindSchema {
    let mut schema = KindSchema::new(
        NativeKindKey::mark("cartesian", "symbol"),
        "A point symbol positioned in Cartesian coordinates.",
    )
    .body_mode(BodyMode::Mixed)
    .child_rule(avenger_chart_schema::ChildRule {
        role: "view".to_string(),
        min: 0,
        max: Some(1),
        docs: "Optional inline view scope owned by this mark.".to_string(),
    });
    for name in [
        "x",
        "y",
        "size",
        "fill",
        "fill_pattern",
        "stroke",
        "stroke_width",
        "shape",
        "angle",
        "opacity",
    ] {
        schema = schema.channel(ChannelSchema {
            name: name.to_string(),
            required: matches!(name, "x" | "y"),
            shape: ValueShape::SqlExpression,
            docs: format!("The symbol `{name}` encoding expression."),
        });
    }
    schema
}

fn register_transforms(builder: &mut NativeRegistryBuilder) -> Result<(), RegistryError> {
    let filter = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "filter"),
        "Retain rows for which a predicate is true.",
    )
    .property(
        "predicate",
        PropertySchema::required(ValueShape::SqlExpression, "Boolean row predicate."),
    );
    builder.register_transform(
        filter,
        Arc::new(|declaration, context| {
            let (transform, ()) = Filter::new(expr_property(declaration, "predicate")?)
                .into_compiled_and_output(context)?;
            Ok(LoweredTransform {
                transform,
                outputs: BTreeMap::new(),
            })
        }),
    )?;

    let operation = ValueShape::Atom {
        values: ["sum", "count", "mean", "min", "max", "median"]
            .into_iter()
            .map(|value| EnumValueSchema {
                value: value.to_string(),
                docs: format!("The `{value}` aggregation operation."),
            })
            .collect(),
    };
    let measure = ValueShape::Object(
        [
            (
                "name".to_string(),
                PropertySchema::required(ValueShape::String, "Output column name."),
            ),
            (
                "op".to_string(),
                PropertySchema::required(operation, "Aggregation operation."),
            ),
            (
                "expr".to_string(),
                PropertySchema::optional(
                    ValueShape::SqlExpression,
                    "Input expression; omitted for count.",
                ),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let aggregate = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "aggregate"),
        "Group rows and compute named aggregate measures.",
    )
    .property(
        "group_by",
        PropertySchema::optional(
            ValueShape::Array(Box::new(ValueShape::SqlExpression)),
            "Grouping expressions.",
        ),
    )
    .property(
        "measures",
        PropertySchema::required(
            ValueShape::Array(Box::new(measure)),
            "Named aggregate measures.",
        ),
    )
    .output(TransformOutputSchema {
        name: "fields".to_string(),
        shape: ValueShape::Object(BTreeMap::new()),
        docs: "Named output fields declared by group keys and measures.".to_string(),
    });
    builder.register_transform(
        aggregate,
        Arc::new(|declaration, context| {
            let mut aggregate = Aggregate::new();
            if let Some(ResolvedValue::Array(groups)) = declaration.properties.get("group_by") {
                for group in groups {
                    let ResolvedValue::Expr(expr) = group else {
                        unreachable!("schema validation checks group expressions")
                    };
                    aggregate = aggregate.group_by([expr.clone()]);
                }
            }
            let ResolvedValue::Array(measures) = declaration.get("measures")? else {
                unreachable!("schema validation checks aggregate measures")
            };
            let mut names = Vec::new();
            for measure in measures {
                let ResolvedValue::Object(fields) = measure else {
                    unreachable!("schema validation checks aggregate measure objects")
                };
                let name = object_string(fields, "name")?;
                let op = object_string(fields, "op")?;
                let expr = fields
                    .get("expr")
                    .map(|value| match value {
                        ResolvedValue::Expr(expr) => Ok(expr.clone()),
                        _ => Err(RegistryError::InvalidPropertyType {
                            property: "expr".to_string(),
                            expected: "SQL expression".to_string(),
                        }),
                    })
                    .transpose()?;
                aggregate = match (op.as_str(), expr) {
                    ("count", None) => aggregate.count(&name),
                    ("sum", Some(expr)) => aggregate.sum(&name, expr),
                    ("mean", Some(expr)) => aggregate.mean(&name, expr),
                    ("min", Some(expr)) => aggregate.min(&name, expr),
                    ("max", Some(expr)) => aggregate.max(&name, expr),
                    ("median", Some(expr)) => aggregate.median(&name, expr),
                    _ => {
                        return Err(RegistryError::Lowering {
                            kind: "aggregate".to_string(),
                            message: format!("operation '{op}' has an invalid expression shape"),
                        });
                    }
                };
                names.push(name);
            }
            let (transform, _output) = aggregate.into_compiled_and_output(context)?;
            Ok(LoweredTransform {
                transform,
                outputs: names
                    .into_iter()
                    .map(|name| (name.clone(), col(name)))
                    .collect(),
            })
        }),
    )?;

    let sql = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "sql"),
        "Run one DataFusion SQL query against the reserved `input` relation.",
    )
    .property(
        "query",
        PropertySchema::required(ValueShape::SqlQuery, "The SQL query."),
    )
    .output(TransformOutputSchema {
        name: "fields".to_string(),
        shape: ValueShape::Object(BTreeMap::new()),
        docs: "Fields projected by the SQL query.".to_string(),
    });
    builder.register_transform(
        sql,
        Arc::new(|declaration, context| {
            let (transform, _output) = Sql::new(string_property(declaration, "query")?)
                .into_compiled_and_output(context)?;
            Ok(LoweredTransform {
                transform,
                outputs: BTreeMap::new(),
            })
        }),
    )?;
    Ok(())
}

fn register_radio_button_list(builder: &mut NativeRegistryBuilder) -> Result<(), RegistryError> {
    let item = ValueShape::Object(
        [
            (
                "value".to_string(),
                PropertySchema::required(ValueShape::Any, "The selected scalar value."),
            ),
            (
                "label".to_string(),
                PropertySchema::required(ValueShape::String, "The displayed item label."),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let position = ValueShape::Atom {
        values: ["top", "right", "bottom", "left"]
            .into_iter()
            .map(|value| EnumValueSchema {
                value: value.to_string(),
                docs: format!("Place the widget on the {value} chart edge."),
            })
            .collect(),
    };
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Widget, "radio_button_list"),
        "A list that selects exactly one scalar value.",
    )
    .allowed_parent("chart")
    .runtime_kind("radio-button-list")
    .property(
        "id",
        PropertySchema::required(ValueShape::String, "Widget source id."),
    )
    .property(
        "items",
        PropertySchema::required(ValueShape::Array(Box::new(item)), "Static list items."),
    )
    .property(
        "default",
        PropertySchema::optional(ValueShape::Any, "Initial selected scalar value."),
    )
    .property(
        "value_param",
        PropertySchema::optional(
            ValueShape::ScalarBinding,
            "Optional existing typed parameter bound to the value state slot.",
        ),
    )
    .property(
        "position",
        PropertySchema::optional(position, "Chart chrome placement edge."),
    )
    .export(ExportSchema {
        alias: "value".to_string(),
        value_kind: "param<item_scalar>".to_string(),
        binding_property: Some("value_param".to_string()),
        default_property: Some("default".to_string()),
        docs: "The currently selected item value.".to_string(),
    })
    .part(PartSchema {
        alias: "control".to_string(),
        runtime_kind: "radio-button-list".to_string(),
        runtime_alias: Some("control".to_string()),
        targetable: true,
        docs: "The outer radio control.".to_string(),
    })
    .part(PartSchema {
        alias: "center".to_string(),
        runtime_kind: "radio-button-list".to_string(),
        runtime_alias: Some("center".to_string()),
        targetable: true,
        docs: "The selected radio center.".to_string(),
    })
    .part(PartSchema {
        alias: "label".to_string(),
        runtime_kind: "radio-button-list".to_string(),
        runtime_alias: Some("label".to_string()),
        targetable: true,
        docs: "The row label.".to_string(),
    })
    .part(PartSchema {
        alias: "focus_ring".to_string(),
        runtime_kind: "radio-button-list".to_string(),
        runtime_alias: Some("focus-ring".to_string()),
        targetable: false,
        docs: "The keyboard focus indicator.".to_string(),
    });

    builder.register_widget(
        schema,
        Arc::new(|declaration| {
            let id = string_property(declaration, "id")?;
            let ResolvedValue::Array(items) = declaration.get("items")? else {
                unreachable!("schema validation checks widget items")
            };
            let mut rows = Vec::with_capacity(items.len());
            for item in items {
                let ResolvedValue::Object(fields) = item else {
                    unreachable!("schema validation checks widget item objects")
                };
                rows.push(WidgetItemRow::new([
                    ("value".to_string(), object_scalar(fields, "value")?),
                    (
                        "label".to_string(),
                        ScalarValue::Utf8(Some(object_string(fields, "label")?)),
                    ),
                ]));
            }
            let mut widget = RadioButtonList::new(id, WidgetItems::Static(rows));
            if let Some(default) = declaration.properties.get("default") {
                widget = widget.default(resolved_scalar(default, "default")?);
            }
            if let Some(value) = declaration.properties.get("value_param") {
                let ResolvedValue::Param(param) = value else {
                    return Err(RegistryError::InvalidPropertyType {
                        property: "value_param".to_string(),
                        expected: "typed scalar parameter".to_string(),
                    });
                };
                widget = widget.value_param(param.clone());
            }
            let position = match declaration.properties.get("position") {
                None => ChromePosition::Right,
                Some(ResolvedValue::String(value)) if value == "right" => ChromePosition::Right,
                Some(ResolvedValue::String(value)) if value == "top" => ChromePosition::Top,
                Some(ResolvedValue::String(value)) if value == "bottom" => ChromePosition::Bottom,
                Some(ResolvedValue::String(value)) if value == "left" => ChromePosition::Left,
                _ => ChromePosition::Right,
            };
            Ok(WidgetAttachment::composed(widget.position(position)))
        }),
    )
}

fn register_objects(builder: &mut NativeRegistryBuilder) -> Result<(), RegistryError> {
    let scale = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Scale, "linear"),
        "A continuous linear scale.",
    );
    builder.register_object(scale, Arc::new(|_| Ok(Box::new(Scale::<Linear>::new()))))?;

    let axis = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Axis, "cartesian"),
        "A Cartesian axis configuration.",
    );
    builder.register_object(axis, Arc::new(|_| Ok(Box::new(CartesianAxis::new()))))?;

    let legend = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Legend, "standard"),
        "A standard chart legend configuration.",
    );
    builder.register_object(legend, Arc::new(|_| Ok(Box::new(Legend::new()))))?;

    let layout = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Layout, "chart"),
        "The default chart frame layout.",
    );
    builder.register_object(layout, Arc::new(|_| Ok(Box::new(LayoutSpec::default()))))?;
    Ok(())
}

fn object_string(
    fields: &IndexMap<String, ResolvedValue>,
    name: &str,
) -> Result<String, RegistryError> {
    match fields.get(name) {
        Some(ResolvedValue::String(value)) => Ok(value.clone()),
        _ => Err(RegistryError::InvalidPropertyType {
            property: name.to_string(),
            expected: "string".to_string(),
        }),
    }
}

fn object_scalar(
    fields: &IndexMap<String, ResolvedValue>,
    name: &str,
) -> Result<ScalarValue, RegistryError> {
    fields
        .get(name)
        .ok_or_else(|| RegistryError::MissingProperty {
            kind: "object".to_string(),
            property: name.to_string(),
        })
        .and_then(|value| resolved_scalar(value, name))
}

fn resolved_scalar(value: &ResolvedValue, name: &str) -> Result<ScalarValue, RegistryError> {
    match value {
        ResolvedValue::Boolean(value) => Ok(ScalarValue::Boolean(Some(*value))),
        ResolvedValue::Integer(value) => Ok(ScalarValue::Int64(Some(*value))),
        ResolvedValue::Number(value) => Ok(ScalarValue::Float64(Some(*value))),
        ResolvedValue::String(value) => Ok(ScalarValue::Utf8(Some(value.clone()))),
        ResolvedValue::Scalar(value) => Ok(value.clone()),
        _ => Err(RegistryError::InvalidPropertyType {
            property: name.to_string(),
            expected: "scalar value".to_string(),
        }),
    }
}
