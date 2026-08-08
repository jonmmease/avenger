//! Generated semantic JSON Schema layered over the frozen interchange schema.

use std::collections::BTreeMap;

use avenger_chart_schema::{
    BodyMode, KindSchema, NativeKindNamespace, NativeSchemaSnapshot, PropertySchema, ValueShape,
};
use serde_json::{Map, Value, json};

use crate::{LANGUAGE_MAJOR, ast::StateActionVerb, interchange::CORE_SCHEMA_V1};

/// Generate a deterministic Draft 2020-12 schema from the active native
/// authoring registry plus the language's fixed core declarations.
pub fn semantic_json_schema(registry: &NativeSchemaSnapshot, profile_id: &str) -> Value {
    let mut schema: Value =
        serde_json::from_str(CORE_SCHEMA_V1).expect("the frozen core schema is valid JSON");
    schema["$id"] = json!("https://avenger.dev/schemas/semantic-1.json");
    schema["title"] = json!("Avenger AST interchange form, full semantic schema");
    schema["x-avenger-language-major"] = json!(LANGUAGE_MAJOR);
    schema["x-avenger-native-profile"] = json!(profile_id);
    schema["x-avenger-native-schema"] =
        serde_json::to_value(registry).expect("native schema is serializable");

    let conditions = semantic_conditions(registry);
    schema["$defs"]["decl"]["allOf"] = Value::Array(conditions);
    schema
}

fn semantic_conditions(registry: &NativeSchemaSnapshot) -> Vec<Value> {
    let mut conditions = vec![
        declaration_condition("param", None, param_body_schema()),
        declaration_condition("store", None, store_body_schema()),
        declaration_condition("selection", None, selection_body_schema()),
        declaration_condition("on", None, event_body_schema()),
        declaration_condition("widget", None, binder_required_schema()),
        declaration_condition("view", None, view_schema()),
        declaration_condition("mark", Some("group"), mark_group_body_schema()),
    ];
    let mut native = BTreeMap::<(String, String), Vec<&KindSchema>>::new();
    for schema in registry.entries.values() {
        for keyword in namespace_keywords(schema.key.namespace) {
            native
                .entry(((*keyword).to_owned(), schema.key.kind.clone()))
                .or_default()
                .push(schema);
        }
    }
    for ((keyword, kind), schemas) in native {
        let mut bodies = schemas
            .into_iter()
            .map(|schema| native_body_schema(schema, &keyword))
            .collect::<Vec<_>>();
        let body = if bodies.len() == 1 {
            bodies.pop().expect("one body")
        } else {
            json!({ "anyOf": bodies })
        };
        conditions.push(declaration_condition(&keyword, Some(&kind), body));
    }
    conditions
}

fn mark_group_body_schema() -> Value {
    json!({
        "properties": {
            "props": object_properties_schema(
                Map::from_iter([
                    ("data".to_owned(), value_ref()),
                    ("component_kind".to_owned(), value_ref()),
                    ("label".to_owned(), value_ref()),
                    ("visible".to_owned(), value_ref()),
                    ("details".to_owned(), value_ref()),
                    ("zindex".to_owned(), value_ref()),
                    ("facet_data_scope".to_owned(), value_ref()),
                    ("geometry_space".to_owned(), value_ref()),
                ]),
                Vec::new(),
                true,
            ),
        },
        "x-avenger-core-kind": "mark_group",
    })
}

fn declaration_condition(keyword: &str, kind: Option<&str>, then: Value) -> Value {
    let mut properties = Map::from_iter([("decl".to_owned(), json!({ "const": keyword }))]);
    let mut required = vec![json!("decl")];
    if let Some(kind) = kind {
        properties.insert("kind".to_owned(), json!({ "const": kind }));
        required.push(json!("kind"));
    }
    json!({
        "if": {
            "properties": properties,
            "required": required,
        },
        "then": then,
    })
}

fn native_body_schema(schema: &KindSchema, keyword: &str) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for (name, property) in &schema.properties {
        properties.insert(name.clone(), value_shape_schema(&property.shape));
        if property.required {
            required.push(json!(name));
        }
    }
    for (name, channel) in &schema.channels {
        properties.insert(
            name.clone(),
            channel_value_schema(&channel.shape, !channel.required),
        );
        if channel.required {
            required.push(json!(name));
        }
    }
    for name in core_native_properties(keyword) {
        properties
            .entry((*name).to_owned())
            .or_insert_with(value_ref);
    }
    let mut props = object_properties_schema(properties, required, true);
    if let Some(additional) = &schema.additional_properties {
        props["additionalProperties"] = value_shape_schema(&additional.shape);
    }
    let mut then = json!({
        "properties": { "props": props },
    });
    if matches!(
        schema.key.namespace,
        NativeKindNamespace::Widget | NativeKindNamespace::Adjust
    ) {
        then["required"] = json!(["name"]);
    }
    if schema.body_mode == BodyMode::Properties {
        then["properties"]["children"] = json!({ "maxItems": 0 });
    }
    if !schema.child_rules.is_empty() {
        then["x-avenger-child-rules"] =
            serde_json::to_value(&schema.child_rules).expect("child rules serialize");
        let constraints = schema
            .child_rules
            .iter()
            .map(|rule| {
                let mut children = json!({
                    "properties": {
                        "children": {
                            "contains": {
                                "properties": { "decl": { "const": rule.role } },
                                "required": ["decl"]
                            },
                            "minContains": rule.min
                        }
                    }
                });
                if let Some(max) = rule.max {
                    children["properties"]["children"]["maxContains"] = json!(max);
                }
                children
            })
            .collect::<Vec<_>>();
        then["allOf"] = Value::Array(constraints);
    }
    if !schema.compatible_coordinates.is_empty() {
        then["x-avenger-compatible-coordinates"] =
            serde_json::to_value(&schema.compatible_coordinates)
                .expect("coordinate set serializes");
    }
    if !schema.allowed_parents.is_empty() {
        then["x-avenger-allowed-parents"] =
            serde_json::to_value(&schema.allowed_parents).expect("parent set serializes");
    }
    then
}

fn param_body_schema() -> Value {
    json!({
        "required": ["name", "props"],
        "properties": {
            "props": object_properties_schema(
                Map::from_iter([
                    ("value".to_owned(), value_ref()),
                    ("sharing".to_owned(), sharing_schema()),
                ]),
                vec![json!("value")],
                true,
            ),
            "children": { "maxItems": 0 },
        },
    })
}

fn store_body_schema() -> Value {
    json!({
        "required": ["name"],
        "properties": {
            "props": object_properties_schema(
                Map::from_iter([
                    ("primary_key".to_owned(), json!({
                        "type": "array",
                        "items": atom_schema(),
                        "minItems": 1,
                        "uniqueItems": true,
                    })),
                    ("sharing".to_owned(), sharing_schema()),
                ]),
                Vec::new(),
                true,
            ),
            "children": {
                "type": "array",
                "items": {
                    "allOf": [
                        { "$ref": "#/$defs/decl" },
                        {
                            "properties": { "decl": { "enum": ["field", "row"] } },
                            "required": ["decl"]
                        }
                    ]
                }
            },
        },
    })
}

fn selection_body_schema() -> Value {
    json!({
        "required": ["name"],
        "properties": {
            "props": object_properties_schema(
                Map::from_iter([
                    (
                        "empty".to_owned(),
                        json!({ "oneOf": [tagged_schema("none"), enum_atom_schema(&["all"])] }),
                    ),
                    ("combine".to_owned(), enum_atom_schema(&["union", "intersect"])),
                ]),
                Vec::new(),
                true,
            ),
            "children": { "maxItems": 0 }
        }
    })
}

fn event_body_schema() -> Value {
    let mut child_keywords = StateActionVerb::ALL
        .into_iter()
        .map(StateActionVerb::as_str)
        .collect::<Vec<_>>();
    child_keywords.push("on");
    json!({
        "properties": {
            "kind": { "enum": [
                "mouse_down", "mouse_up", "click", "double_click", "mouse_wheel",
                "key_press", "key_release", "cursor_moved", "mark_mouse_enter",
                "mark_mouse_leave", "window_resize", "window_resize_settled",
                "canvas_resize", "canvas_resize_settled", "window_moved",
                "window_focused", "window_close_requested"
            ]},
            "props": object_properties_schema(
                Map::from_iter([
                    ("target".to_owned(), value_ref()),
                    ("scope".to_owned(), value_ref()),
                    ("surface".to_owned(), value_ref()),
                    ("filter".to_owned(), value_ref()),
                    ("throttle_ms".to_owned(), number_schema()),
                    ("consume".to_owned(), json!({ "type": "boolean" })),
                    ("mode".to_owned(), enum_atom_schema(&["preview", "exact"])),
                    ("settle_exact".to_owned(), json!({ "type": "boolean" })),
                    ("between".to_owned(), value_ref()),
                ]),
                Vec::new(),
                true,
            ),
            "children": declaration_children_schema(&child_keywords)
        }
    })
}

fn binder_required_schema() -> Value {
    json!({ "required": ["name"] })
}

fn view_schema() -> Value {
    json!({
        "not": {
            "properties": { "visibility": {} },
            "required": ["visibility"]
        },
        "properties": {
            "children": declaration_children_schema(&["transform", "mark", "group"])
        }
    })
}

fn declaration_children_schema(keywords: &[&str]) -> Value {
    json!({
        "type": "array",
        "items": {
            "allOf": [
                { "$ref": "#/$defs/decl" },
                {
                    "properties": { "decl": { "enum": keywords } },
                    "required": ["decl"]
                }
            ]
        }
    })
}

fn object_properties_schema(
    properties: Map<String, Value>,
    required: Vec<Value>,
    closed: bool,
) -> Value {
    let mut schema = json!({
        "type": "object",
        "properties": properties,
    });
    if !required.is_empty() {
        schema["required"] = Value::Array(required);
    }
    if closed {
        schema["additionalProperties"] = json!(false);
    }
    schema
}

fn channel_value_schema(shape: &ValueShape, optional: bool) -> Value {
    if matches!(shape, ValueShape::ChannelConfig) {
        return tagged_schema("block");
    }
    if matches!(shape, ValueShape::RasterDimensionChannel) {
        let dimension = tagged_schema("dim");
        let mut alternatives = vec![
            dimension.clone(),
            json!({
                "type": "object",
                "required": ["block"],
                "properties": {
                    "block": {
                        "allOf": [
                            { "$ref": "#/$defs/body" },
                            {
                                "required": ["head"],
                                "properties": { "head": dimension }
                            }
                        ]
                    }
                },
                "additionalProperties": false
            }),
        ];
        if optional {
            alternatives.push(tagged_payload_schema("none", json!({ "const": true })));
        }
        return json!({ "oneOf": alternatives });
    }

    let expression = value_shape_schema(&ValueShape::SqlExpression);
    let encoded = tagged_payload_schema("encoded", expression.clone());
    let direct = tagged_payload_schema("direct", expression.clone());
    let otherwise = conditional_channel_branch_schema(&expression, false);
    let when = conditional_channel_declaration_schema(&expression);
    let mut alternatives = vec![
        encoded.clone(),
        direct.clone(),
        json!({
            "type": "object",
            "required": ["block"],
            "properties": {
                "block": {
                    "allOf": [
                        { "$ref": "#/$defs/body" },
                        {
                            "required": ["head"],
                            "properties": {
                                "head": { "oneOf": [encoded, direct] },
                                "props": {
                                    "type": "object",
                                    "properties": { "otherwise": otherwise },
                                    "additionalProperties": { "$ref": "#/$defs/value" }
                                },
                                "children": {
                                    "type": "array",
                                    "items": when
                                }
                            },
                            "additionalProperties": false
                        }
                    ]
                }
            },
            "additionalProperties": false
        }),
    ];
    if matches!(shape, ValueShape::PatternChannel) {
        alternatives.push(tagged_schema("pattern"));
    }
    if optional {
        alternatives.push(tagged_payload_schema("none", json!({ "const": true })));
    }
    json!({ "oneOf": alternatives })
}

fn conditional_channel_branch_schema(expression: &Value, predicate: bool) -> Value {
    let mut properties = serde_json::Map::from_iter([
        ("encoded".to_owned(), expression.clone()),
        ("direct".to_owned(), expression.clone()),
    ]);
    if predicate {
        properties.insert("predicate".to_owned(), expression.clone());
    }
    let required_prefix = if predicate {
        vec![json!("predicate")]
    } else {
        Vec::new()
    };
    let mut encoded_required = required_prefix.clone();
    encoded_required.push(json!("encoded"));
    let mut direct_required = required_prefix;
    direct_required.push(json!("direct"));
    let props = json!({
        "type": "object",
        "properties": properties,
        "oneOf": [
            {
                "required": encoded_required,
                "not": { "required": ["direct"] }
            },
            {
                "required": direct_required,
                "not": { "required": ["encoded"] }
            }
        ],
        "additionalProperties": false
    });
    if predicate {
        props
    } else {
        json!({
            "type": "object",
            "required": ["block"],
            "properties": {
                "block": {
                    "allOf": [
                        { "$ref": "#/$defs/body" },
                        {
                            "required": ["props"],
                            "properties": {
                                "props": props,
                                "children": { "maxItems": 0 }
                            },
                            "not": { "required": ["head"] }
                        }
                    ]
                }
            },
            "additionalProperties": false
        })
    }
}

fn conditional_channel_declaration_schema(expression: &Value) -> Value {
    json!({
        "allOf": [
            { "$ref": "#/$defs/decl" },
            {
                "required": ["decl", "props"],
                "properties": {
                    "decl": { "const": "when" },
                    "props": conditional_channel_branch_schema(expression, true),
                    "children": { "maxItems": 0 }
                }
            }
        ]
    })
}

fn value_shape_schema(shape: &ValueShape) -> Value {
    match shape {
        ValueShape::Boolean => json!({ "type": "boolean" }),
        ValueShape::Integer => integer_schema(),
        ValueShape::Number => number_schema(),
        ValueShape::String => json!({ "type": "string" }),
        ValueShape::Identifier => json!({
            "oneOf": [
                { "type": "string" },
                tagged_schema("atom")
            ]
        }),
        ValueShape::Atom { values } => enum_atom_schema(
            &values
                .iter()
                .map(|value| value.value.as_str())
                .collect::<Vec<_>>(),
        ),
        ValueShape::SqlExpression => json!({
            "oneOf": [
                { "type": ["string", "boolean", "null"] },
                number_schema(),
                tagged_schema("col"),
                tagged_schema("atom"),
                tagged_schema("binding"),
                tagged_schema("expr")
            ]
        }),
        ValueShape::SqlProjection { .. } => tagged_schema("projection"),
        ValueShape::SqlQuery => tagged_schema("query"),
        ValueShape::ChannelConfig => tagged_schema("block"),
        ValueShape::ConfiguredExpression(fields) => configured_expression_schema(fields),
        ValueShape::ConfiguredReference {
            namespaces,
            properties,
        } => configured_reference_schema(namespaces, properties),
        ValueShape::PatternChannel => json!({
            "oneOf": [
                tagged_schema("pattern"),
                tagged_schema("encoded"),
                tagged_schema("direct"),
                tagged_schema("block")
            ]
        }),
        ValueShape::CoordinationScope => sharing_schema(),
        ValueShape::FacetDataScope => facet_data_scope_schema(),
        ValueShape::RasterDimension => tagged_schema("dim"),
        ValueShape::RasterDimensionChannel => json!({
            "oneOf": [
                tagged_schema("dim"),
                tagged_schema("block")
            ]
        }),
        ValueShape::ScalarBinding => binding_schema("param"),
        ValueShape::TableBinding => binding_schema("store"),
        ValueShape::SelectionBinding => tagged_schema("ref"),
        ValueShape::WidgetData => json!({
            "oneOf": [tagged_schema("block"), binding_schema("store")]
        }),
        ValueShape::MarkBlock => json!({
            "type": "object",
            "required": ["block"],
            "properties": {
                "block": {
                    "type": "object",
                    "required": ["children"],
                    "properties": {
                        "children": {
                            "type": "array",
                            "minItems": 1,
                            "items": {
                                "allOf": [
                                    { "$ref": "#/$defs/decl" },
                                    {
                                        "properties": { "decl": { "const": "mark" } },
                                        "required": ["decl"]
                                    }
                                ]
                            }
                        }
                    },
                    "additionalProperties": false
                }
            },
            "additionalProperties": false
        }),
        ValueShape::TypedReference { namespaces } => {
            let kinds = namespaces
                .iter()
                .filter_map(reference_kind_for_namespace)
                .collect::<Vec<_>>();
            json!({
                "type": "object",
                "required": ["ref"],
                "properties": {
                    "ref": {
                        "allOf": [
                            { "$ref": "#/$defs/ref" },
                            { "properties": { "kind": { "enum": kinds } } }
                        ]
                    }
                },
                "additionalProperties": false
            })
        }
        ValueShape::Union(shapes) => json!({
            "oneOf": shapes.iter().map(value_shape_schema).collect::<Vec<_>>()
        }),
        ValueShape::OneOrMany(inner) => json!({
            "oneOf": [
                value_shape_schema(inner),
                { "type": "array", "items": value_shape_schema(inner) }
            ]
        }),
        ValueShape::Array(inner) => {
            json!({ "type": "array", "items": value_shape_schema(inner) })
        }
        ValueShape::Map(inner) => json!({
            "type": "object",
            "additionalProperties": value_shape_schema(inner)
        }),
        ValueShape::ChannelMap => json!({
            "type": "object",
            "additionalProperties": channel_value_schema(&ValueShape::SqlExpression, true)
        }),
        ValueShape::Object(fields) => object_value_schema(fields),
        ValueShape::Any => value_ref(),
        ValueShape::StateActionBlock => {
            let mut schema = tagged_schema("block");
            schema["description"] =
                json!("An ordered block of shared-state mutation declarations.");
            schema["x-avenger-value-shape"] = json!("state_action_block");
            schema
        }
    }
}

fn object_value_schema(fields: &std::collections::BTreeMap<String, PropertySchema>) -> Value {
    let properties = fields
        .iter()
        .map(|(name, field)| (name.clone(), value_shape_schema(&field.shape)))
        .collect::<Map<_, _>>();
    let required = fields
        .iter()
        .filter(|(_, field)| field.required)
        .map(|(name, _)| json!(name))
        .collect::<Vec<_>>();
    json!({
        "type": "object",
        "required": ["block"],
        "properties": {
            "block": {
                "type": "object",
                "properties": {
                    "head": value_ref(),
                    "props": object_properties_schema(properties, required, true),
                    "children": { "maxItems": 0 }
                },
                "required": ["props"],
                "additionalProperties": false
            }
        },
        "additionalProperties": false
    })
}

fn configured_expression_schema(
    fields: &std::collections::BTreeMap<String, PropertySchema>,
) -> Value {
    let properties = fields
        .iter()
        .map(|(name, field)| (name.clone(), value_shape_schema(&field.shape)))
        .collect::<Map<_, _>>();
    let required = fields
        .iter()
        .filter(|(_, field)| field.required)
        .map(|(name, _)| json!(name))
        .collect::<Vec<_>>();
    json!({
        "type": "object",
        "required": ["block"],
        "properties": {
            "block": {
                "type": "object",
                "properties": {
                    "head": value_shape_schema(&ValueShape::SqlExpression),
                    "props": object_properties_schema(properties, required, true),
                    "children": { "maxItems": 0 }
                },
                "required": ["head", "props"],
                "additionalProperties": false
            }
        },
        "additionalProperties": false
    })
}

fn configured_reference_schema(
    namespaces: &std::collections::BTreeSet<NativeKindNamespace>,
    fields: &std::collections::BTreeMap<String, PropertySchema>,
) -> Value {
    let properties = fields
        .iter()
        .map(|(name, field)| (name.clone(), value_shape_schema(&field.shape)))
        .collect::<Map<_, _>>();
    let required = fields
        .iter()
        .filter(|(_, field)| field.required)
        .map(|(name, _)| json!(name))
        .collect::<Vec<_>>();
    json!({
        "type": "object",
        "required": ["block"],
        "properties": {
            "block": {
                "type": "object",
                "properties": {
                    "head": value_shape_schema(&ValueShape::TypedReference {
                        namespaces: namespaces.clone()
                    }),
                    "props": object_properties_schema(properties, required, true),
                    "children": { "maxItems": 0 }
                },
                "required": ["head", "props"],
                "additionalProperties": false
            }
        },
        "additionalProperties": false
    })
}

fn number_schema() -> Value {
    json!({
        "type": "object",
        "required": ["num"],
        "properties": { "num": { "type": "string" } },
        "additionalProperties": false
    })
}

fn integer_schema() -> Value {
    json!({
        "type": "object",
        "required": ["num"],
        "properties": { "num": { "type": "string", "pattern": "^-?(0|[1-9][0-9]*)$" } },
        "additionalProperties": false
    })
}

fn atom_schema() -> Value {
    tagged_schema("atom")
}

fn enum_atom_schema(values: &[&str]) -> Value {
    json!({
        "type": "object",
        "required": ["atom"],
        "properties": { "atom": { "enum": values } },
        "additionalProperties": false
    })
}

fn tagged_schema(tag: &str) -> Value {
    json!({
        "type": "object",
        "required": [tag],
        "properties": { tag: {} },
        "additionalProperties": false
    })
}

fn tagged_payload_schema(tag: &str, payload: Value) -> Value {
    json!({
        "type": "object",
        "required": [tag],
        "properties": { tag: payload },
        "additionalProperties": false
    })
}

fn binding_schema(kind: &str) -> Value {
    json!({
        "type": "object",
        "required": ["binding"],
        "properties": {
            "binding": {
                "allOf": [
                    { "$ref": "#/$defs/binding" },
                    { "properties": { "kind": { "const": kind } } }
                ]
            }
        },
        "additionalProperties": false
    })
}

fn sharing_schema() -> Value {
    json!({
        "oneOf": [
            enum_atom_schema(&["shared", "free"]),
            tagged_schema("call")
        ]
    })
}

fn facet_data_scope_schema() -> Value {
    json!({
        "oneOf": [
            enum_atom_schema(&["filtered", "broadcast"]),
            tagged_schema("call")
        ]
    })
}

fn value_ref() -> Value {
    json!({ "$ref": "#/$defs/value" })
}

fn namespace_keywords(namespace: NativeKindNamespace) -> &'static [&'static str] {
    match namespace {
        NativeKindNamespace::Coordinate => &["chart", "cell", "plot"],
        NativeKindNamespace::Adjust => &["adjust"],
        NativeKindNamespace::Mark => &["mark"],
        NativeKindNamespace::Transform => &["transform"],
        NativeKindNamespace::Tool => &["tool"],
        NativeKindNamespace::Widget => &["widget"],
        NativeKindNamespace::View => &["view"],
        NativeKindNamespace::Resource => &["resource"],
        // These namespaces are currently represented in configured value
        // blocks rather than top-level declaration nodes.
        NativeKindNamespace::Scale
        | NativeKindNamespace::Axis
        | NativeKindNamespace::Legend
        | NativeKindNamespace::Layout => &[],
    }
}

fn core_native_properties(keyword: &str) -> &'static [&'static str] {
    match keyword {
        "chart" | "plot" => &["data", "title", "subtitle", "layout", "theme"],
        "cell" => &["at", "data", "label"],
        "view" => &["data"],
        "mark" => &["data"],
        "tool" => &["id"],
        _ => &[],
    }
}

fn reference_kind_for_namespace(namespace: &NativeKindNamespace) -> Option<&'static str> {
    match namespace {
        NativeKindNamespace::Mark => Some("mark"),
        NativeKindNamespace::Tool => Some("tool"),
        NativeKindNamespace::Widget => Some("widget"),
        NativeKindNamespace::Resource => Some("resource"),
        _ => None,
    }
}
