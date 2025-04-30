#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Variable {
    pub parts: Vec<String>,
}

impl Variable {
    pub fn new(parts: Vec<String>) -> Self {
        Self { parts }
    }

    pub fn name(&self) -> String {
        self.parts.join(".")
    }

    pub fn mangled_var_name(&self) -> String {
        format!("@{}", self.parts.join("__"))
    }
}

impl From<String> for Variable {
    fn from(name: String) -> Self {
        Self { parts: vec![name] }
    }
}

