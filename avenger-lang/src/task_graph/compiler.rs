use crate::{ast::{CompInstance, ComponentDef, Statement, VisitorContext, VisitorMut}, error::AvengerLangError};

/// Visitor to extract and remove component definitions from the AST
pub struct ExtractComponentDefinitionsVisitor {
    pub component_definitions: Vec<ComponentDef>,
}

impl ExtractComponentDefinitionsVisitor {
    pub fn new() -> Self {
        Self { component_definitions: Vec::new() }
    }
}

impl VisitorMut for ExtractComponentDefinitionsVisitor {
    fn visit_comp_instance(&mut self, comp_instance: &mut CompInstance, _ctx: &VisitorContext) -> Result<(), AvengerLangError> {
        let mut new_statements = vec![];

        // Remove component definitions from the statements
        for statement in comp_instance.statements.iter() {
            if let Statement::ComponentDef(component_def) = statement {
                self.component_definitions.push(component_def.clone());
            } else {
                new_statements.push(statement.clone());
            }
        }

        comp_instance.statements = new_statements;
        Ok(())
    }
}
