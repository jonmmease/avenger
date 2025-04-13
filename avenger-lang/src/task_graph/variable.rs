
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VariableKind {
    Val,
    // Val is accepted as an expression everywhere
    ValOrExpr,
    Dataset,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Variable {
    pub name: String,
    pub kind: VariableKind,
}

impl Variable {
    pub fn new<T: Into<String>>(name: T, kind: VariableKind) -> Self {
        Self { name: name.into(), kind }
    }
}