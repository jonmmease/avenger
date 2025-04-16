use std::collections::{HashMap, HashSet};

use super::variable::Variable;


pub struct ScopeLevel {
    // The name of the scope level. Top level is "root"
    pub name: String,
    // The val/expr/dataset properties of this scope level
    pub properties: HashSet<String>,
    // The child scopes
    pub children: HashMap<String, ScopeLevel>,
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
