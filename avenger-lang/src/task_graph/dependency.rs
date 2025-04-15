
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DependencyKind {
    Val,
    // Val is accepted as an expression everywhere
    ValOrExpr,
    Dataset,
    Table,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Dependency {
    pub name: String,
    pub kind: DependencyKind,
}

impl Dependency {
    pub fn new<T: Into<String>>(name: T, kind: DependencyKind) -> Self {
        Self { name: name.into(), kind }
    }
}