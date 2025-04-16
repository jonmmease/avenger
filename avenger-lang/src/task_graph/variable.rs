#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Variable {
    pub parts: Vec<String>,
}

impl Variable {
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self { parts: vec![name.into()] }
    }

    pub fn with_parts(parts: Vec<String>) -> Self {
        Self { parts }
    }

    pub fn name(&self) -> String {
        self.parts.join(".")
    }
}

impl<S: Into<String>> From<S> for Variable {
    fn from(name: S) -> Self {
        Self { parts: vec![name.into()] }
    }
}