use std::collections::HashMap;

use super::component_registry::{DatasetPropRegistration, ExprPropRegistration, PropRegistration, ValPropRegistration};
use crate::ast::{ComponentDef, Statement, SqlExprOrQuery};
use sqlparser::ast::{Expr as SqlExpr, Query as SqlQuery, Value as SqlValue};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropType {
    Val,
    Expr,
    Dataset,
}


#[derive(Clone, Debug)]
pub struct ComponentSpec {
    pub name: String,
    pub inherits: Option<String>,
    pub props: HashMap<String, PropRegistration>,
    pub bindings: HashMap<String, SqlExprOrQuery>,
    pub allow_children: bool,
    pub is_mark: bool,
}

impl ComponentSpec {
    pub fn from_component_def(component_def: &ComponentDef) -> Self {
        let name = component_def.name.clone();
        let mut props = HashMap::new();
        let mut bindings = HashMap::new();
        for statement in &component_def.statements {
            match statement {
                Statement::ValPropDecl(val_prop_decl) => {
                    props.insert(val_prop_decl.name.clone(), PropRegistration::Val(ValPropRegistration {
                        qualifier: val_prop_decl.qualifier,
                        default: Some(val_prop_decl.value.clone()),
                    }));
                }
                Statement::ExprPropDecl(expr_prop_decl) => {
                    props.insert(expr_prop_decl.name.clone(), PropRegistration::Expr(ExprPropRegistration {
                        qualifier: expr_prop_decl.qualifier,
                        default: Some(expr_prop_decl.value.clone()),
                    }));
                }
                Statement::DatasetPropDecl(dataset_prop_decl) => {
                    props.insert(dataset_prop_decl.name.clone(), PropRegistration::Dataset(DatasetPropRegistration {
                        qualifier: dataset_prop_decl.qualifier,
                        default: Some(dataset_prop_decl.value.clone()),
                    }));
                }
                Statement::PropBinding(binding) => {
                    bindings.insert(binding.name.clone(), binding.value.clone());
                }
                _ => {}
            }
        }
        
        Self {
            name,
            props,
            bindings,
            inherits: Some(component_def.inherits.clone()), 
            allow_children: true,
            is_mark: false
        }
    }
}

