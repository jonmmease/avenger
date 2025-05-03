use sqlparser::{ast::Ident, tokenizer::Span};

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

    pub fn from_mangled_name(mangled_name: &str) -> Self {
        let name = if mangled_name.starts_with("@") {
            &mangled_name[1..]
        } else {
            mangled_name
        };
        Self::new(
            name.split("__")
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
        )
    }

    pub fn mangled_var_name(&self) -> String {
        format!("@{}", self.parts.join("__"))
    }

    pub fn to_idents(&self) -> Vec<Ident> {
        let mut idents = self
            .parts
            .iter()
            .map(|part| Ident {
                value: part.clone(),
                quote_style: None,
                span: Span::empty(),
            })
            .collect::<Vec<_>>();
        // Prefix first partwith @ if it's a variable reference
        if self.parts.len() > 0 {
            idents[0].value = format!("@{}", idents[0].value);
        }
        idents
    }
}

impl From<String> for Variable {
    fn from(name: String) -> Self {
        Self { parts: vec![name] }
    }
}
