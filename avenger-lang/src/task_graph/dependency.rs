#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DependencyKind {
    Val,
    // Val is accepted as an expression everywhere
    ValOrExpr,
    Dataset,
    Table,
}

use super::variable::Variable;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Dependency {
    pub variable: Variable,
    pub kind: DependencyKind,
}

impl Dependency {
    pub fn new<T: Into<String>>(name: T, kind: DependencyKind) -> Self {
        Self { variable: Variable::new(name), kind }
    }
    
    pub fn with_parts(parts: Vec<String>, kind: DependencyKind) -> Self {
        Self { variable: Variable::with_parts(parts), kind }
    }
    
    pub fn name(&self) -> String {
        self.variable.name()
    }
}