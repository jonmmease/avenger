use sqlparser::{ast::Ident, tokenizer::Span};

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

    pub fn to_idents(&self) -> Vec<Ident> {
        let mut idents = self.parts.iter().map(
            |part| Ident { value: part.clone(), quote_style: None, span: Span::empty() }
        ).collect::<Vec<_>>();
        // Prefix first partwith @ if it's a variable reference
        if self.parts.len() > 0 {
            idents[0].value = format!("@{}", idents[0].value);
        }
        idents
    }
}

impl<S: Into<String>> From<S> for Variable {
    fn from(name: S) -> Self {
        Self { parts: vec![name.into()] }
    }
}