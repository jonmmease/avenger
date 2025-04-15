#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Variable {
    pub name: String,
}

impl Variable {
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self { name: name.into() }
    }
}

impl<S: Into<String>> From<S> for Variable {
    fn from(name: S) -> Self {
        Self { name: name.into() }
    }
}