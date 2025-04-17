use std::collections::HashMap;

use sqlparser::ast::{Expr, Query};

use crate::ast::SqlExprOrQuery;

#[derive(Clone, Debug)]
pub enum PropType {
    Val,
    Expr,
    Dataset,
}

// Should default values be stored here?
// =====================================
// pub enum PropSpec {
//     Val {
//         default: Option<SqlExpr>,
//     },
//     Expr {
//         default: Option<SqlExpr>,
//     },
//     Dataset {
//         default: Option<SqlQuery>,
//     },
// }
// 
// impl PropSpec {
//     pub fn val(default: SqlExpr) -> Self {
//         Self::Val { default: Some(default) }
//     }
// 
//     pub fn val_required() -> Self {
//         Self::Val { default: None }
//     }
// 
//     pub fn expr(default: SqlExpr) -> Self {
//         Self::Expr { default: Some(default) }
//     }
// 
//     pub fn expr_required() -> Self {
//         Self::Expr { default: None }
//     }
// 
//     pub fn dataset(default: SqlQuery) -> Self {
//         Self::Dataset { default: Some(default) }
//     }
// 
//     pub fn dataset_required() -> Self {
//         Self::Dataset { default: None }
//     }
// }
// =====================================

#[derive(Clone, Debug)]
pub struct ComponentSpec {
    pub name: String,
    pub props: HashMap<String, PropType>,
    pub allow_children: bool,
}

#[derive(Clone, Debug)]
pub struct ComponentRegistry {
    components: HashMap<String, ComponentSpec>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self { components: HashMap::new() }
    }

    pub fn register_component(&mut self, id: &str, spec: ComponentSpec) {
        self.components.insert(id.to_string(), spec);
    }

    pub fn lookup_component(&self, type_: &str) -> Option<&ComponentSpec> {
        self.components.get(type_)
    }

    pub fn lookup_prop(&self, type_: &str, prop: &str) -> Option<&PropType> {
        self.components.get(type_).and_then(|spec| spec.props.get(prop))
    }

    pub fn new_with_marks() -> Self {
        let mut registry = Self::new();
        registry.register_component("App", ComponentSpec {
            name: "App".to_string(),
            allow_children: false,
            props: vec![
                ("width", PropType::Val),
                ("height", PropType::Val),
            ].into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        });
        registry.register_component("Rect", ComponentSpec {
            name: "Rect".to_string(),
            allow_children: false,
            props: vec![
                ("data", PropType::Dataset),
                ("zindex", PropType::Val),
                ("clip", PropType::Val),
                ("x", PropType::Expr),
                ("y", PropType::Expr),
                ("x2", PropType::Expr),
                ("y2", PropType::Expr),
                ("width", PropType::Expr),
                ("height", PropType::Expr),
                ("fill", PropType::Expr),
                ("stroke", PropType::Expr),
                ("stroke_width", PropType::Expr),
                ("indices", PropType::Expr),
                ("corner_radius", PropType::Expr),
            ].into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        });
        
        registry.register_component("Arc", ComponentSpec {
            name: "Arc".to_string(),
            allow_children: false,
            props: vec![
                ("data", PropType::Dataset),
                ("zindex", PropType::Val),
                ("clip", PropType::Val),
                ("x", PropType::Expr),
                ("y", PropType::Expr),
                ("start_angle", PropType::Expr),
                ("end_angle", PropType::Expr),
                ("outer_radius", PropType::Expr),
                ("inner_radius", PropType::Expr),
                ("pad_angle", PropType::Expr),
                ("corner_radius", PropType::Expr),
                ("fill", PropType::Expr),
                ("stroke", PropType::Expr),
                ("stroke_width", PropType::Expr),
                ("indices", PropType::Expr),
            ].into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        });
        
        registry.register_component("Area", ComponentSpec {
            name: "Area".to_string(),
            allow_children: false,
            props: vec![
                ("data", PropType::Dataset),
                ("zindex", PropType::Val),
                ("clip", PropType::Val),
                ("orientation", PropType::Val),
                ("x", PropType::Expr),
                ("y", PropType::Expr),
                ("x2", PropType::Expr),
                ("y2", PropType::Expr),
                ("defined", PropType::Expr),
                ("fill", PropType::Expr),
                ("stroke", PropType::Expr),
                ("stroke_width", PropType::Expr),
                ("stroke_cap", PropType::Val),
                ("stroke_join", PropType::Val),
                ("stroke_dash", PropType::Expr),
            ].into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        });
        
        registry.register_component("Image", ComponentSpec {
            name: "Image".to_string(),
            allow_children: false,
            props: vec![
                ("data", PropType::Dataset),
                ("zindex", PropType::Val),
                ("clip", PropType::Val),
                ("aspect", PropType::Val),
                ("smooth", PropType::Val),
                ("image", PropType::Expr),
                ("x", PropType::Expr),
                ("y", PropType::Expr),
                ("width", PropType::Expr),
                ("height", PropType::Expr),
                ("align", PropType::Val),
                ("baseline", PropType::Val),
                ("indices", PropType::Expr),
            ].into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        });
        
        registry.register_component("Line", ComponentSpec {
            name: "Line".to_string(),
            allow_children: false,
            props: vec![
                ("data", PropType::Dataset),
                ("zindex", PropType::Val),
                ("clip", PropType::Val),
                ("x", PropType::Expr),
                ("y", PropType::Expr),
                ("defined", PropType::Expr),
                ("stroke", PropType::Expr),
                ("stroke_width", PropType::Expr),
                ("stroke_cap", PropType::Val),
                ("stroke_join", PropType::Val),
                ("stroke_dash", PropType::Expr),
            ].into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        });
        
        registry.register_component("Path", ComponentSpec {
            name: "Path".to_string(),
            allow_children: false,
            props: vec![
                ("data", PropType::Dataset),
                ("zindex", PropType::Val),
                ("clip", PropType::Val),
                ("path", PropType::Expr),
                ("fill", PropType::Expr),
                ("stroke", PropType::Expr),
                ("stroke_width", PropType::Expr),
                ("stroke_cap", PropType::Val),
                ("stroke_join", PropType::Val),
                ("transform", PropType::Expr),
                ("indices", PropType::Expr),
            ].into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        });
        
        registry.register_component("Rule", ComponentSpec {
            name: "Rule".to_string(),
            allow_children: false,
            props: vec![
                ("data", PropType::Dataset),
                ("zindex", PropType::Val),
                ("clip", PropType::Val),
                ("x", PropType::Expr),
                ("y", PropType::Expr),
                ("x2", PropType::Expr),
                ("y2", PropType::Expr),
                ("stroke", PropType::Expr),
                ("stroke_width", PropType::Expr),
                ("stroke_cap", PropType::Val),
                ("stroke_dash", PropType::Expr),
                ("indices", PropType::Expr),
            ].into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        });
        
        registry.register_component("Symbol", ComponentSpec {
            name: "Symbol".to_string(),
            allow_children: false,
            props: vec![
                ("data", PropType::Dataset),
                ("zindex", PropType::Val),
                ("clip", PropType::Val),
                ("shapes", PropType::Val),
                ("shape_index", PropType::Expr),
                ("x", PropType::Expr),
                ("y", PropType::Expr),
                ("fill", PropType::Expr),
                ("size", PropType::Expr),
                ("stroke", PropType::Expr),
                ("stroke_width", PropType::Expr),
                ("angle", PropType::Expr),
                ("indices", PropType::Expr),
                ("x_adjustment", PropType::Val),
                ("y_adjustment", PropType::Val),
            ].into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        });
        
        registry.register_component("Text", ComponentSpec {
            name: "Text".to_string(),
            allow_children: false,
            props: vec![
                ("data", PropType::Dataset),
                ("zindex", PropType::Val),
                ("clip", PropType::Val),
                ("text", PropType::Expr),
                ("x", PropType::Expr),
                ("y", PropType::Expr),
                ("align", PropType::Val),
                ("baseline", PropType::Val),
                ("angle", PropType::Expr),
                ("color", PropType::Expr),
                ("font", PropType::Expr),
                ("font_size", PropType::Expr),
                ("font_weight", PropType::Val),
                ("font_style", PropType::Val),
                ("limit", PropType::Expr),
                ("indices", PropType::Expr),
            ].into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        });
        
        registry.register_component("Trail", ComponentSpec {
            name: "Trail".to_string(),
            allow_children: false,
            props: vec![
                ("data", PropType::Dataset),
                ("zindex", PropType::Val),
                ("clip", PropType::Val),
                ("x", PropType::Expr),
                ("y", PropType::Expr),
                ("size", PropType::Expr),
                ("defined", PropType::Expr),
                ("stroke", PropType::Expr),
            ].into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        });

        registry.register_component("Group", ComponentSpec {
            name: "Group".to_string(),
            allow_children: true,
            props: vec![
                ("x", PropType::Val),
                ("y", PropType::Val),
                ("width", PropType::Val),
                ("height", PropType::Val),
                ("fill", PropType::Val),
                ("stroke", PropType::Val),
                ("stroke_width", PropType::Val),
                ("stroke_offset", PropType::Val),
                ("zindex", PropType::Val),
            ].into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        });
        
        registry
    }
}



