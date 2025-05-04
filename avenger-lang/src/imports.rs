use std::{collections::HashMap, ops::ControlFlow, path::PathBuf};

use sqlparser::ast::{Ident, VisitorMut};

use crate::{
    ast::{AvengerFile, ComponentProp, ImportStatement, Statement}, error::AvengerLangError, loader::{AvengerFilesystemLoader, AvengerLoader}, visitor::{AvengerVisitorMut, VisitorContext}
};

use std::sync::Arc;

#[derive(Clone)]

pub struct InlineImportsVisitor {
    loader: Arc<dyn AvengerLoader>,

    /// alias to file ast with imports already inlined
    imports: HashMap<String, AvengerFile>,
}

impl InlineImportsVisitor {
    pub fn new(loader: Arc<dyn AvengerLoader>) -> Self {
        Self {
            loader,
            imports: HashMap::new(),
        }
    }
}

impl VisitorMut for InlineImportsVisitor {
    type Break = Result<(), AvengerLangError>;
}
impl AvengerVisitorMut for InlineImportsVisitor {
    fn pre_visit_import_statement(
        &mut self,
        statement: &mut ImportStatement,
        _context: &VisitorContext,
    ) -> ControlFlow<Self::Break> {
        // Load imports
        for item in &statement.items {
            let component_name = item.name.value.clone();
            let alias = item
                .alias
                .clone()
                .map(|alias| alias.value)
                .unwrap_or_else(|| component_name.clone());

            match self
                .loader
                .load_file(&component_name, &statement.from_path.clone().unwrap().value)
            {
                Ok(mut file_ast) => {
                    // Inline the imports recursively
                    let mut visitor = InlineImportsVisitor::new(self.loader.clone());
                    file_ast.visit_mut(&mut visitor)?;
                    self.imports.insert(alias, file_ast);
                }
                Err(e) => {
                    return ControlFlow::Break(Err(e));
                }
            }
        }

        ControlFlow::Continue(())
    }

    fn pre_visit_component_prop(
        &mut self,
        component_prop: &mut ComponentProp,
        _context: &VisitorContext,
    ) -> ControlFlow<Self::Break> {
        let component_name = &component_prop.component_type.value;
        if let Some(file_ast) = self.imports.get(component_name) {
            // TODO: replace @root references in file_ast with @{component_prop.prop_name}

            // Collect my binding properties
            let my_bindings = component_prop.prop_bindings();

            let mut new_statements = Vec::new();

            // Keep all of the imported file's statements, but replace any in prop values with our
            // own binding values
            for mut import_statement in file_ast.statements.clone() {
                match &mut import_statement {
                    Statement::ValProp(val_prop) => {
                        if my_bindings.contains_key(&val_prop.name.value) {
                            // replace value my binding
                            match my_bindings[&val_prop.name.value].expr.clone().into_expr() {
                                Ok(expr) => {
                                    val_prop.expr = expr;
                                }
                                Err(err) => {
                                    return ControlFlow::Break(Err(err));
                                }
                            }
                        }
                    }
                    Statement::ExprProp(expr_prop) => {
                        if my_bindings.contains_key(&expr_prop.name.value) {
                            // replace value my binding
                            match my_bindings[&expr_prop.name.value].expr.clone().into_expr() {
                                Ok(expr) => {
                                    expr_prop.expr = expr;
                                }
                                Err(err) => {
                                    return ControlFlow::Break(Err(err));
                                }
                            }
                        }
                    }
                    Statement::DatasetProp(dataset_prop) => {
                        if my_bindings.contains_key(&dataset_prop.name.value) {
                            // replace value my binding
                            match my_bindings[&dataset_prop.name.value]
                                .expr
                                .clone()
                                .into_query()
                            {
                                Ok(query) => {
                                    dataset_prop.query = query;
                                }
                                Err(err) => {
                                    return ControlFlow::Break(Err(err));
                                }
                            }
                        }
                    }
                    _ => {}
                }
                new_statements.push(import_statement.clone());
            }

            // Add our own statements, other than bindings
            // TODO: validate that we don't have any left over bindings
            for statement in component_prop.statements.clone() {
                match &statement {
                    Statement::PropBinding(prop_binding) => {
                        // skip
                    }
                    _ => {
                        new_statements.push(statement.clone());
                    }
                }
            }

            // Replace the statements
            component_prop.statements = new_statements;

            // Replace the component type with a group
            component_prop.component_type = Ident::new("Group");
        }

        ControlFlow::Continue(())
    }

    /// Remove top-level impor
    fn post_visit_avenger_file(
        &mut self,
        file: &mut AvengerFile,
        _context: &VisitorContext,
    ) -> ControlFlow<Self::Break> {
        // Remove imports from the file
        file.statements
            .retain(|statement| !matches!(statement, Statement::Import(_)));

        // Change the component type to Group
        file.name = "Group".to_string();

        ControlFlow::Continue(())
    }

    /// Remove nexted imports
    fn post_visit_component_prop(
        &mut self,
        component_prop: &mut ComponentProp,
        _context: &VisitorContext,
    ) -> ControlFlow<Self::Break> {
        component_prop
            .statements
            .retain(|statement| !matches!(statement, Statement::Import(_)));
        ControlFlow::Continue(())
    }
}

/// Load and parse a main component file from a project, inlining all imports.
/// path is the path to the main component file, and this file is assumed to be under the
/// root of the project.
pub fn load_main_component_file(
    path: PathBuf,
    verbose: bool,
) -> Result<AvengerFile, AvengerLangError> {
    let path = path.canonicalize()?;
    let file_name = path.file_name().unwrap().to_str().unwrap().to_string();
    let Some(component_type) = file_name.strip_suffix(".avgr") else {
        return Err(AvengerLangError::InvalidFileExtensionError(path));
    };

    let Some(base_path) = path.parent() else {
        return Err(AvengerLangError::InternalError(format!(
            "Failed to get parent of path: {:?}",
            path
        )));
    };

    let loader = Arc::new(AvengerFilesystemLoader::new(base_path, verbose));

    let mut main_component = loader.load_file(component_type, ".")?;
    let mut visitor = InlineImportsVisitor::new(loader);
    if let ControlFlow::Break(res) = main_component.visit_mut(&mut visitor) {
        match res {
            Ok(v) => {
                return Err(AvengerLangError::InternalError(format!(
                    "break without error: {:?}",
                    v
                )));
            }
            Err(e) => return Err(e),
        }
    };


    // println!("after: {}", main_component);

    Ok(main_component)
}

#[cfg(test)]
mod tests {
    use crate::{loader::AvengerMemoryLoader, parser::AvengerParser};

    use super::*;

    fn make_loader() -> Arc<dyn AvengerLoader> {
        // Foo
        let foo_src = "
            in val a_prop: 'black';
            Rect {
                fill := 'red';
                stroke := @a_prop;
            }
        ";
        let foo_name = "Foo";
        let foo_path = ".";
        let foo_ast = AvengerParser::new(foo_src, foo_name, foo_path)
            .unwrap()
            .parse()
            .unwrap();

        // Bar
        let bar_src = "
            import { Foo } from '.';
            in val b_prop: 32;
            Foo {
                a_prop := 'yellow';
            }
        ";
        let bar_name = "Bar";
        let bar_path = ".";
        let bar_ast = AvengerParser::new(bar_src, bar_name, bar_path)
            .unwrap()
            .parse()
            .unwrap();

        // App
        let app_src = "
            import { Bar } from '.';
            Bar {
                b_prop := 12;
            }
        ";
        let app_name = "App";
        let app_path = ".";
        let app_ast = AvengerParser::new(app_src, app_name, app_path)
            .unwrap()
            .parse()
            .unwrap();

        let loader = AvengerMemoryLoader::new(
            vec![
                (foo_name, foo_path, foo_ast),
                (app_name, app_path, app_ast),
                (bar_name, bar_path, bar_ast),
            ]
            .into_iter(),
        );
        Arc::new(loader)
    }

    #[test]
    fn test_inline_imports() {
        let loader = make_loader();
        let mut app_ast = loader.load_file("App", ".").unwrap();

        println!("before: {:#?}", app_ast);
        let mut visitor = InlineImportsVisitor::new(loader);
        if let ControlFlow::Break(e) = app_ast.visit_mut(&mut visitor) {
            panic!("Error: {:?}", e);
        }
        println!("after: {:#?}", app_ast);
    }
}
