use std::{collections::HashMap, ops::ControlFlow};

use sqlparser::{ast::{AccessExpr, Expr as SqlExpr, Function, FunctionArg, FunctionArgExpr, FunctionArgOperator, FunctionArguments, Ident, Query as SqlQuery, Visitor, VisitorMut}, tokenizer::Span};
use nanoid::nanoid;

use crate::{ast::{AvengerFile, ComponentProp, DatasetProp, ExprProp, KeywordComp, Statement, ValProp}, error::AvengerLangError, parser::AvengerParser, visitor::{AvengerVisitor, AvengerVisitorMut, VisitorContext}};


/// Visitor to expand binding expressions in SQL expressions
/// 
/// For example, given a sql function of the form:
/// 
/// ```avgr
/// val a: @foo(a:=12).res + 12;
/// ```
/// 
/// The visitor will expand the function call to 
/// ```avgr
/// comp foo_a12: @foo(a:=12);
/// val a: @foo_a12 + 12;
/// ```

#[derive(Debug, Clone)]
pub struct BoundFunctionInstance {
    pub instance_name: String,
    pub path: Vec<String>,
    pub args: Vec<(String, SqlExpr)>,
}

pub struct FunctionBindingExpander {
    /// A mapping from bound instance name to tuple of (original instance name, named args)
    bound_instances: HashMap<String, BoundFunctionInstance>,
    current_context: VisitorContext,
    /// A mapping from property name of a component property to the list of statements
    /// in the component.
    component_instances: HashMap<String, Vec<Statement>>,
}

impl FunctionBindingExpander {
    pub fn new() -> Self {
        Self {
            bound_instances: HashMap::new(),
            current_context: VisitorContext::new(),
            component_instances: HashMap::new(),
        }
    }

    fn set_context(&mut self, context: &VisitorContext) {
        self.current_context = context.clone();
    }

    fn push_component_instance(&mut self, statement: &ComponentProp) {
        self.component_instances.insert(
            statement.name().to_string(), statement.statements.clone()
        );
    }

    fn pop_component_instance(&mut self, statement: &ComponentProp) {
        self.component_instances.remove(&statement.name());
    }
}

impl VisitorMut for FunctionBindingExpander {
    type Break = Result<(), AvengerLangError>;

    fn pre_visit_expr(&mut self, expr: &mut SqlExpr) -> ControlFlow<Self::Break> {
        if let SqlExpr::CompoundFieldAccess { root, access_chain } = expr.clone() {
            if let SqlExpr::Function(Function {
                name, 
                args,
                ..
            }) = root.as_ref() {
                if name.0[0].value.starts_with("@") {
                    // We have a function call of the form @foo(...).res
                    
                    // Collect the property path after the function name
                    let mut parts = vec![];
                    for access in access_chain.iter() {
                        if let AccessExpr::Dot(expr) = access {
                            parts.push(expr.to_string());
                        } else {
                            return ControlFlow::Break(
                                Err(AvengerLangError::InternalError(format!("Expected a dot access expression, got {:?}", access)))
                            );
                        }
                    }

                    // Get component instance name, without the @
                    let instance_name = name.0[0].value[1..].to_string();

                    // Collect the named args
                    let mut named_args = Vec::new();
                    if let FunctionArguments::List(args_list) = args {
                        for arg in args_list.args.iter() {
                            if let FunctionArg::Named { name, arg, operator } = arg {
                                // Check operator
                                if operator != &FunctionArgOperator::Assignment {
                                    return ControlFlow::Break(
                                        Err(AvengerLangError::InternalError(format!("Expected an assignment operator (:=), got {:?}", operator)))
                                    );
                                }

                                // Collect the named arg
                                if let FunctionArgExpr::Expr(expr) = arg {
                                    named_args.push(
                                        (name.value.clone(), expr.clone())
                                    );
                                } else {
                                    return ControlFlow::Break(
                                        Err(AvengerLangError::InternalError(format!("Expected an expression for function argument, got {:?}", arg)))
                                    );
                                }
                            } else {
                                return ControlFlow::Break(
                                    Err(AvengerLangError::InternalError(format!("Expected a named function argument, got {:?}", arg)))
                                );
                            }
                        }
                    } else {
                        return ControlFlow::Break(
                            Err(AvengerLangError::InternalError(format!("Expected a list of arguments, got {:?}", args)))
                        );
                    }

                    // Generate a new name for the bound instance (e.g. foo_a1b2)
                    let bound_instance_name = create_bound_instance_name(&instance_name, &named_args);

                    self.bound_instances.insert(
                        bound_instance_name.clone(),
                        BoundFunctionInstance {
                            instance_name,
                            path: self.current_context.path.clone(),
                            args: named_args,
                        }
                    );

                    // Replace the function call with a reference to the bound instance
                    *expr = SqlExpr::CompoundFieldAccess {
                        root: Box::new(SqlExpr::Identifier(Ident::new(format!("@{}", bound_instance_name)))),
                        access_chain,
                    };
                }
            }
        }
        
        ControlFlow::Continue(())
    }
}

impl AvengerVisitorMut for FunctionBindingExpander {
    // Update the context
    fn pre_visit_avenger_file(&mut self, file: &mut AvengerFile, context: &VisitorContext) -> ControlFlow<Self::Break> {
        // Set the context
        self.set_context(context);

        // Push child component instances into the scope
        for statement in file.statements.iter() {
            if let Statement::ComponentProp(component_prop) = statement {
                self.push_component_instance(component_prop);
            }
        }

        ControlFlow::Continue(())
    }


    fn pre_visit_component_prop(&mut self, statement: &mut ComponentProp, context: &VisitorContext) -> ControlFlow<Self::Break> {
        self.set_context(context);

        // Push child component instances into the scope
        for statement in statement.statements.iter() {
            if let Statement::ComponentProp(component_prop) = statement {
                self.push_component_instance(component_prop);
            }
        }

        ControlFlow::Continue(())
    }

    fn pre_visit_val_prop(&mut self, _statement: &mut ValProp, context: &VisitorContext) -> ControlFlow<Self::Break> {
        self.set_context(context);
        ControlFlow::Continue(())
    }

    fn pre_visit_expr_prop(&mut self, _statement: &mut ExprProp, context: &VisitorContext) -> ControlFlow<Self::Break> {
        self.set_context(context);
        ControlFlow::Continue(())
    }

    fn pre_visit_dataset_prop(&mut self, _statement: &mut DatasetProp, context: &VisitorContext) -> ControlFlow<Self::Break> {
        self.set_context(context);
        ControlFlow::Continue(())
    }

    // Add new props
    fn post_visit_component_prop(&mut self, statement: &mut ComponentProp, context: &VisitorContext) -> ControlFlow<Self::Break> {
        let comp_path = context.child(&statement.name(), "").path;

        let mut remaining_bound_instances = HashMap::new();
        for (bound_instance_name, instance) in self.bound_instances.iter() {
            if instance.path == comp_path {
                let Some(inner_statements) = self.component_instances.get(&instance.instance_name) else {
                    println!("{:?}", self.component_instances);
                    return ControlFlow::Break(
                        Err(AvengerLangError::InternalError(format!("Expected component instance to exist for {}", bound_instance_name)))
                    );
                };

                // Override the statements with the named arguments
                let Ok(inner_statements) = override_statements_with_params(
                    inner_statements, &instance.instance_name, &instance.args
                ) else {
                    return ControlFlow::Break(
                        Err(AvengerLangError::InternalError(format!("Error overriding statements for {}", bound_instance_name)))
                    );
                };

                statement.statements.push(Statement::ComponentProp(ComponentProp {
                    prop_name: Some(Ident::new(bound_instance_name.clone())),
                    qualifier: None,
                    component_keyword: Some(KeywordComp { span: Span::empty() }),
                    component_type: Ident::new("Group"),
                    statements: inner_statements.clone(),
                }));
            } else {
                // Not from this path, so add to remaining statements
                remaining_bound_instances.insert(bound_instance_name.clone(), instance.clone());
            }
        }
        self.bound_instances = remaining_bound_instances;

        // Pop the child component instance from scope
        for statement in statement.statements.iter() {
            if let Statement::ComponentProp(component_prop) = statement {
                self.pop_component_instance(component_prop);
            }
        }
        ControlFlow::Continue(())
    }

    fn post_visit_avenger_file(&mut self, file: &mut AvengerFile, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        for (bound_instance_name, instance) in self.bound_instances.iter() {
            if !instance.path.is_empty() {
                // Expected path to be empty
                return ControlFlow::Break(
                    Err(AvengerLangError::InternalError(format!("Expected path to be empty, got {:?}", instance.path)))
                );
            }
            let Some(inner_statements) = self.component_instances.get(&instance.instance_name) else {
                println!("Not from this path, so adding to remaining statements: {:#?}", instance);
                return ControlFlow::Break(
                    Err(AvengerLangError::InternalError(format!("Expected component instance to exist for {}", bound_instance_name)))
                );
            };

            // Override the statements with the named arguments
            let Ok(inner_statements) = override_statements_with_params(
                inner_statements, &instance.instance_name, &instance.args
            ) else {
                return ControlFlow::Break(
                    Err(AvengerLangError::InternalError(format!("Error overriding statements for {}", bound_instance_name)))
                );
            };

            file.statements.push(Statement::ComponentProp(ComponentProp {
                prop_name: Some(Ident::new(bound_instance_name.clone())),
                qualifier: None,
                component_keyword: Some(KeywordComp { span: Span::empty() }),
                component_type: Ident::new("Group"),
                // todo: add statements
                statements: inner_statements,
            }));
        }
        
        // Clear the bound instances
        self.bound_instances.clear();
        self.component_instances.clear();
        ControlFlow::Continue(())
    }
}

/// Generate a short, friendly hash for named arguments using their string representation
fn create_bound_instance_name(instance_name: &str, named_args: &[(String, SqlExpr)]) -> String {
    if named_args.is_empty() {
        return format!("{}", instance_name);
    }
    
    // Build a string representation of all arguments
    let mut hash_input = String::new();
    for (name, expr) in named_args {
        hash_input.push_str(name);
        hash_input.push('=');
        hash_input.push_str(&expr.to_string());
        hash_input.push(';');
    }
    
    // Use friendly alphabet (lowercase alphanumeric)
    let alphabet: &[char] = &['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 
                             'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 
                             'u', 'v', 'w', 'x', 'y', 'z', '0', '1', '2', '3', 
                             '4', '5', '6', '7', '8', '9'];
    
    format!("{}_{}", instance_name, nanoid!(4, alphabet))
}

fn override_statements_with_params(statements: &[Statement], instance_name: &str, params: &[(String, SqlExpr)]) -> Result<Vec<Statement>, AvengerLangError> {
    let params: HashMap<_, _> = params.iter().map(
        |(name, expr)| (name.clone(), expr.clone())
    ).collect();

    let mut new_statements = Vec::new();
    for statement in statements.iter() {
        match statement {
            Statement::ValProp(val_prop) => {
                let mut new_val_prop = val_prop.clone();
                if let Some(expr) = params.get(val_prop.name()) {
                    new_val_prop.expr = expr.clone();
                } else {
                    // // Map some props to original component instance?
                    // new_val_prop.expr = SqlExpr::CompoundFieldAccess {
                    //     root: Box::new(SqlExpr::Identifier(Ident::new(format!("@{}", instance_name)))),
                    //     access_chain: vec![AccessExpr::Dot(
                    //         SqlExpr::Identifier(Ident::new(val_prop.name().to_string()
                    //     )))],
                    // };
                }
                new_statements.push(Statement::ValProp(new_val_prop));
            },
            Statement::ExprProp(expr_prop) => {
                let mut new_expr_prop = expr_prop.clone();
                if let Some(expr) = params.get(expr_prop.name()) {
                    new_expr_prop.expr = expr.clone();
                } else {
                    // // Map some props to original component instance?
                    // new_expr_prop.expr = SqlExpr::CompoundFieldAccess {
                    //     root: Box::new(SqlExpr::Identifier(Ident::new(format!("@{}", instance_name)))),
                    //     access_chain: vec![AccessExpr::Dot(
                    //         SqlExpr::Identifier(Ident::new(expr_prop.name().to_string()))
                    //     )],
                    // };
                }
                new_statements.push(Statement::ExprProp(new_expr_prop));
            },
            Statement::DatasetProp(dataset_prop) => {
                let mut new_dataset_prop = dataset_prop.clone();
                if let Some(expr) = params.get(dataset_prop.name()) {
                    return Err(AvengerLangError::InternalError(
                        format!("Function with dataset prop not supported: {}", dataset_prop.name())
                    ));
                } else {
                    // // Map some props to original component instance?
                    // let Ok(new_query) = build_select_star_from_name(instance_name,  dataset_prop.name()) else {
                    //     return Err(AvengerLangError::InternalError(
                    //         format!("Error building select star from name: {}", instance_name)
                    //     ));
                    // };
                    // new_dataset_prop.query = new_query;
                }
                new_statements.push(Statement::DatasetProp(new_dataset_prop));
            },
            _ => {
                new_statements.push(statement.clone());
            }
        }
    }
    Ok(new_statements)
}

pub fn build_select_star_from_name(instance_name: &str, prop_name: &str) -> Result<Box<SqlQuery>, AvengerLangError> {
    let sql = format!("select * from @{}.{}", instance_name, prop_name);
    let mut parser = AvengerParser::new(&sql, "App", "").unwrap();
    Ok(parser.parser.parse_query()?)
}


#[cfg(test)]
mod tests {
    use crate::parser::AvengerParser;

    use super::*;
    
    #[test]
    fn test_create_bound_instance_name() {
        let src = r#"
            comp foo: Group {
                in val a: 0;
                in val b: 1;
                val res: @a * 2 + @b;
            }
            val a: @foo(a:=177).res + 12;
            Group {
                val b: @foo(b:=277).res + 12;
            }
        "#;
        let mut parser = AvengerParser::new(
            src, "Test", "."
        ).unwrap();

        let mut file = parser.parse().unwrap();

        let mut visitor = FunctionBindingExpander::new();

        if let ControlFlow::Break(err) = file.visit_mut(&mut visitor) {
            panic!("Break: {:#?}", err);
        }

        println!("{file}");
    }
}
