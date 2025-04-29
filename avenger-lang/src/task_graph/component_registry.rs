use std::collections::HashMap;

use avenger_scenegraph::marks::mark::SceneMarkType;
use sqlparser::ast::{Expr as SqlExpr, Query as SqlQuery, Value as SqlValue};

use crate::ast::{PropQualifier, SqlExprOrQuery, Type};

use super::component::{ComponentSpec, PropType};

#[derive(Clone, Debug)]
pub struct ComponentRegistry {
    components: HashMap<String, ComponentSpec>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self {
            components: HashMap::new(),
        }
    }

    pub fn register_component(&mut self, id: &str, spec: ComponentSpec) {
        println!("registering component: {:#?}", spec);
        self.components.insert(id.to_string(), spec);
    }

    pub fn lookup_component(&self, component: &str) -> Option<&ComponentSpec> {
        self.components.get(component)
    }

    pub fn lookup_prop(&self, component: &str, prop: &str) -> Option<&PropRegistration> {
        let mut current_component = component.to_string();
        while let Some(spec) = self.components.get(&current_component) {
            if let Some(prop) = spec.props.get(prop) {
                return Some(prop);
            }
            match &spec.inherits {
                Some(inherits) => current_component = inherits.clone(),
                None => break,
            }
        }
        None
    }

    pub fn all_props(&self, component: &str) -> HashMap<String, PropRegistration> {
        let mut current_component = component.to_string();
        let mut props = HashMap::new();
        while let Some(spec) = self.components.get(&current_component) {
            for (name, prop) in &spec.props {
                // Add props, but don't overwrite so that child props override parent props
                if !props.contains_key(name) {
                    props.insert(name.clone(), prop.clone());
                }
            }
            match &spec.inherits {
                Some(inherits) => current_component = inherits.clone(),
                None => break,
            }
        }
        props
    }

    pub fn lookup_mark_type(&self, component: &str) -> Option<SceneMarkType> {
        let mut current_component = component.to_string();
        while let Some(spec) = self.components.get(&current_component) {
            if spec.is_mark {
                return match spec.name.as_str() {
                    "Rect" => Some(SceneMarkType::Rect),
                    "Arc" => Some(SceneMarkType::Arc),
                    "Area" => Some(SceneMarkType::Area),
                    "Image" => Some(SceneMarkType::Image),
                    "Line" => Some(SceneMarkType::Line),
                    "Path" => Some(SceneMarkType::Path),
                    "Rule" => Some(SceneMarkType::Rule),
                    "Symbol" => Some(SceneMarkType::Symbol),
                    "Text" => Some(SceneMarkType::Text),
                    "Trail" => Some(SceneMarkType::Trail),
                    _ => None,
                };
            }
            match &spec.inherits {
                Some(inherits) => current_component = inherits.clone(),
                None => break,
            }
        }
        None
    }

    pub fn new_with_marks() -> Self {
        let mut registry = Self::new();
        registry.register_component(
            "App",
            ComponentSpec {
                name: "App".to_string(),
                props: vec![
                    (
                        "width",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "600.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "height",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "400.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                inherits: None,
                allow_children: false,
                is_mark: false,
            },
        );
        registry.register_component(
            "Rect",
            ComponentSpec {
                name: "Rect".to_string(),
                props: vec![
                    (
                        "data",
                        PropRegistration::Dataset(DatasetPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "zindex",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "clip",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Boolean(false))),
                        }),
                    ),
                    (
                        "x",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "y",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "x2",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "y2",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "width",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "height",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "fill",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "rgba(0, 0, 0, 0)".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "rgba(0, 0, 0, 0)".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke_width",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "indices",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "corner_radius",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                inherits: None,
                allow_children: false,
                is_mark: true,
            },
        );

        registry.register_component(
            "Arc",
            ComponentSpec {
                name: "Arc".to_string(),
                props: vec![
                    (
                        "data",
                        PropRegistration::Dataset(DatasetPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "zindex",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "clip",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Boolean(false))),
                        }),
                    ),
                    (
                        "x",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "y",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "start_angle",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "end_angle",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "6.283185307179586".to_string(),
                                false,
                            ))), // 2*PI
                        }),
                    ),
                    (
                        "outer_radius",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "50.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "inner_radius",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "pad_angle",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "corner_radius",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "fill",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "rgba(0, 0, 0, 0)".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "rgba(0, 0, 0, 0)".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke_width",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "indices",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                inherits: None,
                allow_children: false,
                is_mark: true,
            },
        );

        registry.register_component(
            "Area",
            ComponentSpec {
                name: "Area".to_string(),
                props: vec![
                    (
                        "data",
                        PropRegistration::Dataset(DatasetPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "zindex",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "clip",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Boolean(false))),
                        }),
                    ),
                    (
                        "orientation",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "vertical".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "x",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "y",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "x2",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "y2",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "defined",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Boolean(true))),
                        }),
                    ),
                    (
                        "fill",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "rgba(0, 0, 0, 0)".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "rgba(0, 0, 0, 0)".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke_width",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "1.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "stroke_cap",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "butt".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke_join",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "miter".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke_dash",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                inherits: None,
                allow_children: false,
                is_mark: true,
            },
        );

        registry.register_component(
            "Image",
            ComponentSpec {
                name: "Image".to_string(),
                props: vec![
                    (
                        "data",
                        PropRegistration::Dataset(DatasetPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "zindex",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "clip",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Boolean(false))),
                        }),
                    ),
                    (
                        "aspect",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Boolean(true))),
                        }),
                    ),
                    (
                        "smooth",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Boolean(true))),
                        }),
                    ),
                    (
                        "image",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "x",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "y",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "width",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "height",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "align",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "center".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "baseline",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "middle".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "indices",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                inherits: None,
                allow_children: false,
                is_mark: true,
            },
        );

        registry.register_component(
            "Line",
            ComponentSpec {
                name: "Line".to_string(),
                props: vec![
                    (
                        "data",
                        PropRegistration::Dataset(DatasetPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "zindex",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "clip",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Boolean(false))),
                        }),
                    ),
                    (
                        "x",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "y",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "defined",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Boolean(true))),
                        }),
                    ),
                    (
                        "stroke",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "rgba(0, 0, 0, 0)".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke_width",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "1.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "stroke_cap",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "butt".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke_join",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "miter".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke_dash",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                inherits: None,
                allow_children: false,
                is_mark: true,
            },
        );

        registry.register_component(
            "Path",
            ComponentSpec {
                name: "Path".to_string(),
                props: vec![
                    (
                        "data",
                        PropRegistration::Dataset(DatasetPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "zindex",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "clip",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Boolean(false))),
                        }),
                    ),
                    (
                        "path",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "fill",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "rgba(0, 0, 0, 0)".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "rgba(0, 0, 0, 0)".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke_width",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "1.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "stroke_cap",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "butt".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke_join",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "miter".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "transform",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "indices",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                inherits: None,
                allow_children: false,
                is_mark: true,
            },
        );

        registry.register_component(
            "Rule",
            ComponentSpec {
                name: "Rule".to_string(),
                props: vec![
                    (
                        "data",
                        PropRegistration::Dataset(DatasetPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "zindex",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "clip",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Boolean(false))),
                        }),
                    ),
                    (
                        "x",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "y",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "x2",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "y2",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "stroke",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "rgba(0, 0, 0, 1.0)".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke_width",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "1.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "stroke_cap",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "butt".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke_dash",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "indices",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                inherits: None,
                allow_children: false,
                is_mark: true,
            },
        );

        registry.register_component(
            "Symbol",
            ComponentSpec {
                name: "Symbol".to_string(),
                props: vec![
                    (
                        "data",
                        PropRegistration::Dataset(DatasetPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "zindex",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "clip",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Boolean(false))),
                        }),
                    ),
                    (
                        "shapes",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "circle".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "shape_index",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "x",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "y",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "fill",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "rgba(0, 0, 0, 1.0)".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "size",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "25.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "stroke",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "rgba(0, 0, 0, 0)".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "stroke_width",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "1.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "angle",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "indices",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "x_adjustment",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "y_adjustment",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                inherits: None,
                allow_children: false,
                is_mark: true,
            },
        );

        registry.register_component(
            "Text",
            ComponentSpec {
                name: "Text".to_string(),
                props: vec![
                    (
                        "data",
                        PropRegistration::Dataset(DatasetPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "zindex",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "clip",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Boolean(false))),
                        }),
                    ),
                    (
                        "text",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "x",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "y",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "align",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "left".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "baseline",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "alphabetic".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "angle",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "color",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "rgba(0, 0, 0, 1.0)".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "font",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "sans-serif".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "font_size",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "11.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "font_weight",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "normal".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "font_style",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "normal".to_string(),
                            ))),
                        }),
                    ),
                    (
                        "limit",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "indices",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                inherits: None,
                allow_children: false,
                is_mark: true,
            },
        );

        registry.register_component(
            "Trail",
            ComponentSpec {
                name: "Trail".to_string(),
                props: vec![
                    (
                        "data",
                        PropRegistration::Dataset(DatasetPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "zindex",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "clip",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Boolean(false))),
                        }),
                    ),
                    (
                        "x",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "y",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "size",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "2.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "defined",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Boolean(true))),
                        }),
                    ),
                    (
                        "stroke",
                        PropRegistration::Expr(ExprPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::SingleQuotedString(
                                "rgba(0, 0, 0, 1.0)".to_string(),
                            ))),
                        }),
                    ),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                inherits: None,
                allow_children: false,
                is_mark: true,
            },
        );

        registry.register_component(
            "Group",
            ComponentSpec {
                name: "Group".to_string(),
                props: vec![
                    (
                        "x",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "y",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: Some(SqlExpr::Value(SqlValue::Number(
                                "0.0".to_string(),
                                false,
                            ))),
                        }),
                    ),
                    (
                        "width",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "height",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "fill",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "stroke",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "stroke_width",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "stroke_offset",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                    (
                        "zindex",
                        PropRegistration::Val(ValPropRegistration {
                            qualifier: None,
                            default: None,
                        }),
                    ),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                inherits: None,
                allow_children: true,
                is_mark: true,
            },
        );

        registry
    }
}

/// If qualifier is In, and default is None, then the prop is required.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValPropRegistration {
    pub qualifier: Option<PropQualifier>,
    pub default: Option<SqlExpr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExprPropRegistration {
    pub qualifier: Option<PropQualifier>,
    pub default: Option<SqlExpr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DatasetPropRegistration {
    pub qualifier: Option<PropQualifier>,
    pub default: Option<Box<SqlQuery>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PropRegistration {
    Val(ValPropRegistration),
    Expr(ExprPropRegistration),
    Dataset(DatasetPropRegistration),
}
