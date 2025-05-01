use std::collections::{HashMap, HashSet};
use std::cell::RefCell;
use std::ops::ControlFlow;

use avenger_lang2::ast::{AvengerFile, AvengerProject, ComponentProp, DatasetProp, ExprProp, PropBinding, ValProp};
use sqlparser::ast::{Expr as SqlExpr, Ident, ObjectName, Query as SqlQuery, VisitMut, Visitor, VisitorMut as SqlVisitorMut};
use avenger_lang2::visitor::{AvengerVisit, AvengerVisitor, VisitorContext};

use crate::component_registry::ComponentRegistry;
use crate::error::AvengerRuntimeError;
use crate::variable::Variable;

#[derive(Debug, PartialEq)]
pub struct ScopeLevel {
    // The name of the scope level. Top level is "root"
    pub name: String,
    // The val/expr/dataset properties of this scope level
    pub properties: HashSet<String>,
    // The child scopes
    pub children: HashMap<String, ScopeLevel>,
}

impl Clone for ScopeLevel {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            properties: self.properties.clone(),
            children: self.children.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scope {
    root_level: ScopeLevel,
    component_paths: HashMap<String, Vec<String>>,
}

impl Scope {
    pub fn new(root_level: ScopeLevel) -> Self {
        let mut component_paths = HashMap::new();
        Scope::add_component_paths(&mut component_paths, &root_level, &vec![]);

        Self { root_level, component_paths }
    }

    fn add_component_paths(
        component_paths: &mut HashMap<String, Vec<String>>,
        level: &ScopeLevel,
        prefix: &[String]
    ) {
        for (name, child) in level.children.iter() {
            let mut parts = prefix.to_vec();
            parts.push(name.clone());
            component_paths.insert(name.clone(), parts.clone());
            Scope::add_component_paths(component_paths, child, &parts);
        }
    }
    
    // Create a Scope from an AvengerFile using the ScopeBuilder visitor
    pub fn from_file(project: &AvengerProject, file_name: &str) -> Result<Self, AvengerRuntimeError> {
        let registry = ComponentRegistry::from(project);
        let mut builder = ScopeBuilder::new(&registry);
        let file_ast = project.files.get(file_name).ok_or_else(|| AvengerRuntimeError::InternalError(format!(
            "File {} not found in project", file_name
        )))?;
        if let ControlFlow::Break(err) = file_ast.visit(&mut builder) {
            return Err(AvengerRuntimeError::InternalError(format!(
                "Error building scope: {:?}", err
            )));
        }
        Ok(builder.build())
    }
}

// ScopeBuilder visitor that constructs a Scope from an AvengerFile
pub struct ScopeBuilder<'a> {
    root_level: RefCell<ScopeLevel>,
    registry: &'a ComponentRegistry,
}

impl<'a> ScopeBuilder<'a> {
    pub fn new(registry: &'a ComponentRegistry) -> Self {
        Self {
            root_level: RefCell::new(ScopeLevel {
                name: "root".to_string(),
                properties: HashSet::new(),
                children: HashMap::new(),
            }),
            registry,
        }
    }
    
    fn add_property(&self, name: &str, scope_path: &[String]) -> Result<(), AvengerRuntimeError> {
        let mut root_level = self.root_level.borrow_mut();
        let mut current = &mut *root_level;
        
        // Navigate to the current scope level
        for part in scope_path {
            if !current.children.contains_key(part) {
                current.children.insert(part.clone(), ScopeLevel {
                    name: part.clone(),
                    properties: HashSet::new(),
                    children: HashMap::new(),
                });
            }
            current = current.children.get_mut(part)
                .ok_or_else(|| AvengerRuntimeError::InternalError(format!(
                    "Failed to find scope level part: {}", part)))?;
        }
        
        // Add the property to the current scope level
        current.properties.insert(name.to_string());
        Ok(())
    }

    pub fn build(&self) -> Scope {
        let borrowed = self.root_level.borrow();
        let root_level_clone = (*borrowed).clone();
        Scope::new(root_level_clone)
    }
}

impl<'a> Visitor for ScopeBuilder<'a> {
    type Break = Result<(), AvengerRuntimeError>;
}

impl<'a> AvengerVisitor for ScopeBuilder<'a> {
    fn pre_visit_val_prop(&mut self, val_prop: &ValProp, context: &VisitorContext) -> ControlFlow<Self::Break> {
        if let Err(err) = self.add_property(&val_prop.name.value, &context.path) {
            return ControlFlow::Break(Err(err));
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr_prop(&mut self, expr_prop: &ExprProp, context: &VisitorContext) -> ControlFlow<Self::Break> {
        if let Err(err) = self.add_property(&expr_prop.name.value, &context.path) {
            return ControlFlow::Break(Err(err));
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_dataset_prop(&mut self, dataset_prop: &DatasetProp, context: &VisitorContext) -> ControlFlow<Self::Break> {
        if let Err(err) = self.add_property(&dataset_prop.name.value, &context.path) {
            return ControlFlow::Break(Err(err));
        }
        ControlFlow::Continue(())
    }

    fn post_visit_component_prop(&mut self, comp_prop: &ComponentProp, context: &VisitorContext) -> ControlFlow<Self::Break> {
        // Add all properties of the component type to the scope
        let Some(component_type) = self.registry.lookup_component(&comp_prop.component_type.value) else {
            return ControlFlow::Break(Err(AvengerRuntimeError::InternalError(format!(
                "Component type {} not found in registry", comp_prop.component_type.value
            ))));
        };
        let mut child_path = context.path.clone();
        child_path.push(comp_prop.name());
        for (name, prop) in component_type.props.iter() {
            if let Err(err) = self.add_property(&name, &child_path) {
                return ControlFlow::Break(Err(err));
            }
        }
        ControlFlow::Continue(())
    }
}


#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ScopeAnchor {
    Self_,
    Parent,
    Root,
    Component(Vec<String>)
}


impl Scope {
    fn make_scope_stack<'a>(&'a self, path: &[String]) -> Result<Vec<&'a ScopeLevel>, AvengerRuntimeError> {
        let mut scope_stack: Vec<&ScopeLevel> = vec![&self.root_level];
        for part in path {
            let next_level = scope_stack[scope_stack.len() - 1].children.get(part)
                .ok_or_else(|| AvengerRuntimeError::InternalError(format!(
                    "Failed to find scope level part: {}", part)))?;
            scope_stack.push(next_level);
        }
        Ok(scope_stack)
    }

    /// Resolve a variable reference used in a particular scope
    pub fn resolve_var(&self, name: &str, path: &[String], anchor: Option<ScopeAnchor>) -> Option<Variable> {
        let scope_stack = match self.make_scope_stack(path) {
            Ok(stack) => stack,
            Err(err) => {
                println!("Error making scope stack: {:?}", err);
                return None;
            }
        };
        
        // Walk the stack to find the variable
        match anchor {
            Some(ScopeAnchor::Self_) => {
                // check only the final level
                if scope_stack[scope_stack.len() - 1].properties.contains(name) {
                    let mut parts: Vec<String> = (1..scope_stack.len()).map(|i| scope_stack[i].name.clone()).collect();
                    parts.push(name.to_string());
                    return Some(Variable::new(parts));
                }
                None
            }
            Some(ScopeAnchor::Parent) => {
                // Check only the parent level
                if scope_stack[scope_stack.len() - 2].properties.contains(name) {
                    let mut parts: Vec<String> = (1..scope_stack.len() - 1).map(|i| scope_stack[i].name.clone()).collect();
                    parts.push(name.to_string());
                    return Some(Variable::new(parts));
                }
                None
            }
            Some(ScopeAnchor::Root) => {
                // Check only the root level
                if scope_stack[0].properties.contains(name) {
                    return Some(Variable::new(vec![name.to_string()]));
                }
                None
            }
            Some(ScopeAnchor::Component(parts)) => {
                let Some(root_path) = self.component_paths.get(&parts[0]) else {
                    return None;
                };
                // 
                let mut path = root_path.clone();
                for child_part in parts[1..].iter() {
                    let Some(child_level) = scope_stack[0].children.get(child_part) else {
                        return None;
                    };
                    path.push(child_level.name.clone());
                }
                path.push(name.to_string());
                Some(Variable::new(path))
            }
            None => {
                // Walk the stack to find the variable
                for lvl in (0..scope_stack.len()).rev() {
                    if scope_stack[lvl].properties.contains(name) {
                        // Build variable from the root
                        let mut parts: Vec<String> = (1..=lvl).map(|i| scope_stack[i].name.clone()).collect();
                        parts.push(name.to_string());
                        return Some(Variable::new(parts));
                    }
                }
                None
            }
        }
    }

    pub fn resolve_sql_expr(&self, expr: &mut SqlExpr, scope_path: &[String]) -> Result<(), AvengerRuntimeError> {
        let mut visitor = ResolveVarInSqlVisitor::new(
            self, scope_path
        );
        if let ControlFlow::Break(Err(err)) = expr.visit(&mut visitor) {
            return Err(err)
        }
        Ok(())
    }

    pub fn resolve_sql_query(&self, query: &mut SqlQuery, scope_path: &[String]) -> Result<(), AvengerRuntimeError> {
        let mut visitor = ResolveVarInSqlVisitor::new(
            self, scope_path
        );
        if let ControlFlow::Break(Err(err)) = query.visit(&mut visitor) {
            println!("Scope: {:#?}", self);
            return Err(err)
        }
        Ok(())
    }
}


pub struct ResolveVarInSqlVisitor<'a> {
    scope: &'a Scope,
    path: &'a [String],
}

impl<'a> ResolveVarInSqlVisitor<'a> {
    pub fn new(scope: &'a Scope, path: &'a [String]) -> Self {
        Self { scope, path }
    }

    fn resolve_idents(&self, idents: &[Ident]) -> Option<Vec<Ident>> {
        if idents.is_empty() || !idents[0].value.starts_with("@") {
            // Not a variable reference, so no need to resolve
            return None;
        } else if idents.len() == 1 {
            // Name without leading @
            let name = idents[0].value[1..].to_string();
            let anchor = None;
            if let Some(resolved) = self.scope.resolve_var(&name, &self.path, anchor) {
                return Some(resolved.to_idents())
            }
        } else if idents.len() == 2 && idents[0].value == "@self" {
            let name = idents[1].value.clone();
            let anchor = Some(ScopeAnchor::Self_);
            if let Some(resolved) = self.scope.resolve_var(&name, &self.path, anchor) {
                return Some(resolved.to_idents());
            }
        } else if idents.len() == 2 && idents[0].value == "@parent" {
            let name = idents[1].value.clone();
            let anchor = Some(ScopeAnchor::Parent);
            if let Some(resolved) = self.scope.resolve_var(&name, &self.path, anchor) {
                return Some(resolved.to_idents());
            }
        } else if idents.len() == 2 && idents[0].value == "@root" {
            let name = idents[1].value.clone();
            let anchor = Some(ScopeAnchor::Root);
            if let Some(resolved) = self.scope.resolve_var(&name, &self.path, anchor) {
                return Some(resolved.to_idents());
            }
        } else {
            let name = idents[idents.len() - 1].value.clone();
            let mut anchor_parts = idents[0..idents.len() - 1].iter().map(|i| i.value.clone()).collect::<Vec<_>>();

            // Remove leading @ from first part
            anchor_parts[0] = anchor_parts[0][1..].to_string();

            let anchor = Some(ScopeAnchor::Component(anchor_parts));
            if let Some(resolved) = self.scope.resolve_var(&name, &self.path, anchor) {
                return Some(resolved.to_idents());
            }
        }
        None
    }
}

impl<'a> SqlVisitorMut for ResolveVarInSqlVisitor<'a> {
    type Break = Result<(), AvengerRuntimeError>;

    fn pre_visit_relation(&mut self, relation: &mut ObjectName) -> ControlFlow<Self::Break> {
        if let Some(resolved) = self.resolve_idents(&relation.0) {
            *relation = ObjectName(resolved);
        } else if relation.0.first().map_or(false, |ident| ident.value.starts_with("@")) {
            return ControlFlow::Break(Err(AvengerRuntimeError::InternalError(
                format!("Failed to resolve variable reference: {:?} in path: {:?}", relation, self.path)
            )));
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &mut SqlExpr) -> ControlFlow<Self::Break> {
        match expr.clone() {
            SqlExpr::Identifier(ident) => {
                if ident.value.starts_with("@") {
                    if let Some(resolved) = self.resolve_idents(&[ident.clone()]) {
                        *expr = SqlExpr::CompoundIdentifier(resolved);
                    } else {
                        return ControlFlow::Break(Err(AvengerRuntimeError::InternalError(
                            format!("Failed to resolve variable reference: {:?} in path: {:?}", ident, self.path)
                        )));
                    }
                }
            }
            SqlExpr::CompoundIdentifier(idents) => {
                if idents.first().map_or(false, |id| id.value.starts_with("@")) {
                    if let Some(resolved) = self.resolve_idents(&idents) {
                        *expr = SqlExpr::CompoundIdentifier(resolved);
                    } else {
                        return ControlFlow::Break(Err(AvengerRuntimeError::InternalError(
                            format!("Failed to resolve variable reference: {:?} in path: {:?}", idents, self.path)
                        )));
                    }
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    
    fn assert_var_resolution(
        scope: &Scope, 
        name: &str, 
        path: &[String], 
        anchor: Option<ScopeAnchor>,
        expected: Option<&[&str]>
    ) {
        let var = scope.resolve_var(
            name,
            path,
            anchor
        );
        if let Some(expected) = expected {
            let parts = expected.iter().map(|s| s.to_string()).collect();
            assert_eq!(var, Some(Variable::new(parts)));
        } else {
            assert_eq!(var, None);
        }
    }

    #[test]
    fn test_resolve_var() {
        let scope = Scope::new( 
            ScopeLevel { 
                name: "root".to_string(), 
                properties: vec![
                    "rootVarA".to_string(),
                    "rootVarB".to_string(),
                ].into_iter().collect(), 
                children: vec![
                    ("child1".to_string(), ScopeLevel {
                        name: "child1".to_string(),
                        properties: vec![
                            "var1".to_string(),
                            "var2".to_string(),
                        ].into_iter().collect(),
                        children: HashMap::new(),
                    }),
                    ("child2".to_string(), ScopeLevel {
                        name: "child2".to_string(),
                        properties: vec![
                            "varA".to_string(),
                            "varB".to_string(),
                        ].into_iter().collect(),
                        children: vec![
                            ("child2_a".to_string(), ScopeLevel {
                                name: "child2_a".to_string(),
                                properties: vec![
                                    "varC".to_string(),
                                    "varD".to_string(),
                                ].into_iter().collect(),
                                children: HashMap::new(),
                            }),
                        ].into_iter().collect(),
                    }),
                ].into_iter().collect()
            } 
        );

        // Can reference var1 directly from child1 without an anchor
        assert_var_resolution(
            &scope, "var1", 
            &["child1".to_string()],
            None,
            Some(&["child1", "var1"])
        );

        // Can reference var1 directly from child1 with self
        assert_var_resolution(
            &scope, "var1", 
            &["child1".to_string()],
            Some(ScopeAnchor::Self_),
            Some(&["child1", "var1"])
        );

        // Cannot reference varA directly from child1
        assert_var_resolution(
            &scope, "varA", 
            &["child1".to_string()],
            None,
            None,
        );

        // Can reference root properties directly
        assert_var_resolution(
            &scope, "rootVarA", 
            &["child1".to_string()],
            None,
            Some(&["rootVarA"])
        );

        // Cannot reference root properties directly with self
        assert_var_resolution(
            &scope, "rootVarA", 
            &["child1".to_string()],
            Some(ScopeAnchor::Self_),
            None
        );

        // Can reference root properties directly with root
        assert_var_resolution(
            &scope, "rootVarA", 
            &["child1".to_string()],
            Some(ScopeAnchor::Root),
            Some(&["rootVarA"])
        );

        // Can reference root properties directly with root
        assert_var_resolution(
            &scope, "rootVarA", 
            &["child1".to_string()],
            Some(ScopeAnchor::Parent),
            Some(&["rootVarA"])
        );

        // Can reference parent properties directly with parent
        assert_var_resolution(
            &scope, "varB", 
            &["child2".to_string(), "child2_a".to_string()],
            Some(ScopeAnchor::Parent),
            Some(&["child2", "varB"])
        );

        // Can't reference self properties directly with parent
        assert_var_resolution(
            &scope, "varC", 
            &["child2".to_string(), "child2_a".to_string()],
            Some(ScopeAnchor::Parent),
            None
        );

        // Can reference varA from child2_a using component anchor
        assert_var_resolution(
            &scope, "varA", 
            &["child1".to_string()],
            Some(ScopeAnchor::Component(vec!["child2_a".to_string()])),
            Some(&["child2", "child2_a", "varA"])
        );
    }
}
