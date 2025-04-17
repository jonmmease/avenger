use std::collections::{HashMap, HashSet};
use std::cell::RefCell;
use std::ops::ControlFlow;

use super::variable::Variable;
use crate::ast::{AvengerFile, Visitor, ValPropDecl, ExprPropDecl, DatasetPropDecl, CompPropDecl};
use crate::error::AvengerLangError;
use sqlparser::ast::{Expr as SqlExpr, Ident, ObjectName, Query as SqlQuery, VisitMut, VisitorMut as SqlVisitorMut};

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

pub struct Scope {
    root_level: ScopeLevel,
    component_paths: HashMap<String, ScopePath>,
}

impl Scope {
    pub fn new(root_level: ScopeLevel) -> Self {
        let mut component_paths = HashMap::new();
        Scope::add_component_paths(&mut component_paths, &root_level, &vec![]);

        Self { root_level, component_paths }
    }

    fn add_component_paths(
        component_paths: &mut HashMap<String, ScopePath>,
        level: &ScopeLevel,
        prefix: &[String]
    ) {
        for (name, child) in level.children.iter() {
            let mut parts = prefix.to_vec();
            parts.push(name.clone());
            component_paths.insert(name.clone(), ScopePath::new(parts.clone()));
            Scope::add_component_paths(component_paths, child, &parts);
        }
    }
    
    // Create a Scope from an AvengerFile using the ScopeBuilder visitor
    pub fn from_file(file: &AvengerFile) -> Self {
        let mut builder = ScopeBuilder::new();
        file.accept(&mut builder);
        builder.build()
    }
}

// ScopeBuilder visitor that constructs a Scope from an AvengerFile
pub struct ScopeBuilder {
    root_level: RefCell<ScopeLevel>,
}

impl ScopeBuilder {
    pub fn new() -> Self {
        Self {
            root_level: RefCell::new(ScopeLevel {
                name: "root".to_string(),
                properties: HashSet::new(),
                children: HashMap::new(),
            }),
        }
    }
    
    fn add_property(&self, name: &str, scope_path: &[String]) {
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
            current = current.children.get_mut(part).unwrap();
        }
        
        // Add the property to the current scope level
        current.properties.insert(name.to_string());
    }

    pub fn build(&self) -> Scope {
        let borrowed = self.root_level.borrow();
        let root_level_clone = (*borrowed).clone();
        Scope::new(root_level_clone)
    }
}

impl Visitor for ScopeBuilder {
    fn visit_val_prop_decl(&mut self, val_prop: &ValPropDecl, scope_path: &[String]) {
        self.add_property(&val_prop.name, scope_path);
    }

    fn visit_expr_prop_decl(&mut self, expr_prop: &ExprPropDecl, scope_path: &[String]) {
        self.add_property(&expr_prop.name, scope_path);
    }

    fn visit_dataset_prop_decl(&mut self, dataset_prop: &DatasetPropDecl, scope_path: &[String]) {
        self.add_property(&dataset_prop.name, scope_path);
    }

    fn visit_comp_prop_decl(&mut self, comp_prop: &CompPropDecl, scope_path: &[String]) {
        self.add_property(&comp_prop.name, scope_path);
    }
}


#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ScopeAnchor {
    Self_,
    Parent,
    Root,
    Component(Vec<String>)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScopePath(pub Vec<String>);

impl ScopePath {
    pub fn new(parts: Vec<String>) -> Self {
        Self(parts)
    }
}

impl Scope {
    fn make_scope_stack<'a>(&'a self, path: &ScopePath) -> Vec<&'a ScopeLevel> {
        let mut scope_stack: Vec<&ScopeLevel> = vec![&self.root_level];
        for part in &path.0 {
            scope_stack.push(scope_stack[scope_stack.len() - 1].children.get(part).unwrap());
        }
        scope_stack
    }

    /// Resolve a variable reference used in a particular scope
    pub fn resolve_var(&self, name: &str, path: &ScopePath, anchor: Option<ScopeAnchor>) -> Option<Variable> {
        let scope_stack = self.make_scope_stack(path);
        
        // Walk the stack to find the variable
        match anchor {
            Some(ScopeAnchor::Self_) => {
                // check only the final level
                if scope_stack[scope_stack.len() - 1].properties.contains(name) {
                    let mut parts: Vec<String> = (1..scope_stack.len()).map(|i| scope_stack[i].name.clone()).collect();
                    parts.push(name.to_string());
                    return Some(Variable::with_parts(parts));
                }
                None
            }
            Some(ScopeAnchor::Parent) => {
                // Check only the parent level
                if scope_stack[scope_stack.len() - 2].properties.contains(name) {
                    let mut parts: Vec<String> = (1..scope_stack.len() - 1).map(|i| scope_stack[i].name.clone()).collect();
                    parts.push(name.to_string());
                    return Some(Variable::with_parts(parts));
                }
                None
            }
            Some(ScopeAnchor::Root) => {
                // Check only the root level
                if scope_stack[0].properties.contains(name) {
                    return Some(Variable::with_parts(vec![name.to_string()]));
                }
                None
            }
            Some(ScopeAnchor::Component(parts)) => {
                let Some(root_path) = self.component_paths.get(&parts[0]) else {
                    return None;
                };
                // 
                let mut path = root_path.0.clone();
                for child_part in parts[1..].iter() {
                    let Some(child_level) = scope_stack[0].children.get(child_part) else {
                        return None;
                    };
                    path.push(child_level.name.clone());
                }
                path.push(name.to_string());
                Some(Variable::with_parts(path))
            }
            None => {
                // Walk the stack to find the variable
                for lvl in (0..scope_stack.len()).rev() {
                    if scope_stack[lvl].properties.contains(name) {
                        // Build variable from the root
                        let mut parts: Vec<String> = (1..=lvl).map(|i| scope_stack[i].name.clone()).collect();
                        parts.push(name.to_string());
                        return Some(Variable::with_parts(parts));
                    }
                }
                None
            }
        }
    }

    pub fn resolve_sql_expr(&self, expr: &mut SqlExpr, scope_path: &ScopePath) -> Result<(), AvengerLangError> {
        let mut visitor = ResolveVarInSqlVisitor::new(
            self, scope_path
        );
        if let ControlFlow::Break(Err(err)) = expr.visit(&mut visitor) {
            return Err(err)
        }
        Ok(())
    }

    pub fn resolve_sql_query(&self, query: &mut SqlQuery, scope_path: &ScopePath) -> Result<(), AvengerLangError> {
        let mut visitor = ResolveVarInSqlVisitor::new(
            self, scope_path
        );
        if let ControlFlow::Break(Err(err)) = query.visit(&mut visitor) {
            return Err(err)
        }
        Ok(())
    }
}


pub struct ResolveVarInSqlVisitor<'a> {
    scope: &'a Scope,
    path: &'a ScopePath,
}

impl<'a> ResolveVarInSqlVisitor<'a> {
    pub fn new(scope: &'a Scope, path: &'a ScopePath) -> Self {
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
    type Break = Result<(), AvengerLangError>;

    fn pre_visit_relation(&mut self, relation: &mut ObjectName) -> ControlFlow<Self::Break> {
        if let Some(resolved) = self.resolve_idents(&relation.0) {
            *relation = ObjectName(resolved);
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &mut SqlExpr) -> ControlFlow<Self::Break> {
        match expr.clone() {
            SqlExpr::Identifier(ident) => {
                if let Some(resolved) = self.resolve_idents(&[ident]) {
                    *expr = SqlExpr::CompoundIdentifier(resolved);
                }
            }
            SqlExpr::CompoundIdentifier(idents) => {
                if let Some(resolved) = self.resolve_idents(&idents) {
                    *expr = SqlExpr::CompoundIdentifier(resolved);
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
        path: &ScopePath, 
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
            assert_eq!(var, Some(Variable::with_parts(parts)));
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
            &ScopePath::new(vec!["child1".to_string()]),
            None,
            Some(&["child1", "var1"])
        );

        // Can reference var1 directly from child1 with self
        assert_var_resolution(
            &scope, "var1", 
            &ScopePath::new(vec!["child1".to_string()]),
            Some(ScopeAnchor::Self_),
            Some(&["child1", "var1"])
        );

        // Cannot reference varA directly from child1
        assert_var_resolution(
            &scope, "varA", 
            &ScopePath::new(vec!["child1".to_string()]),
            None,
            None,
        );

        // Can reference root properties directly
        assert_var_resolution(
            &scope, "rootVarA", 
            &ScopePath::new(vec!["child1".to_string()]),
            None,
            Some(&["rootVarA"])
        );

        // Cannot reference root properties directly with self
        assert_var_resolution(
            &scope, "rootVarA", 
            &ScopePath::new(vec!["child1".to_string()]),
            Some(ScopeAnchor::Self_),
            None
        );

        // Can reference root properties directly with root
        assert_var_resolution(
            &scope, "rootVarA", 
            &ScopePath::new(vec!["child1".to_string()]),
            Some(ScopeAnchor::Root),
            Some(&["rootVarA"])
        );

        // Can reference root properties directly with root
        assert_var_resolution(
            &scope, "rootVarA", 
            &ScopePath::new(vec!["child1".to_string()]),
            Some(ScopeAnchor::Parent),
            Some(&["rootVarA"])
        );

        // Can reference parent properties directly with parent
        assert_var_resolution(
            &scope, "varB", 
            &ScopePath::new(vec!["child2".to_string(), "child2_a".to_string()]),
            Some(ScopeAnchor::Parent),
            Some(&["child2", "varB"])
        );

        // Can't reference self properties directly with parent
        assert_var_resolution(
            &scope, "varC", 
            &ScopePath::new(vec!["child2".to_string(), "child2_a".to_string()]),
            Some(ScopeAnchor::Parent),
            None
        );

        // Can reference varA from child2_a using component anchor
        assert_var_resolution(
            &scope, "varA", 
            &ScopePath::new(vec!["child1".to_string()]),
            Some(ScopeAnchor::Component(vec!["child2_a".to_string()])),
            Some(&["child2", "child2_a", "varA"])
        );
    }
}
