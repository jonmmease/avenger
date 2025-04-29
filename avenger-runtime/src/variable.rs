#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Variable {
    pub parts: Vec<String>,
}

impl Variable {
    pub fn new(parts: Vec<String>) -> Self {
        Self { parts }
    }
}

impl From<String> for Variable {
    fn from(name: String) -> Self {
        Self { parts: vec![name] }
    }
}

