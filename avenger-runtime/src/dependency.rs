#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DependencyKind {
    Val,
    // Val is accepted as an expression everywhere
    ValOrExpr,
    Dataset,
    Mark,
}

use super::variable::Variable;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Dependency {
    pub variable: Variable,
    pub kind: DependencyKind,
}

impl Dependency {    
    pub fn new(parts: Vec<String>, kind: DependencyKind) -> Self {
        Self { variable: Variable::new(parts), kind }
    }
}