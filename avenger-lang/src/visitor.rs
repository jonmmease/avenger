use std::ops::ControlFlow;

use sqlparser::ast::{Visit, VisitMut, Visitor, VisitorMut};

use crate::ast::{AvengerFile, ComponentProp, DatasetProp, ExprProp, FunctionDef, FunctionReturn, FunctionStatement, ImportStatement, PropBinding, SqlExprOrQuery, Statement, ValProp};

#[derive(Debug, Clone)]
pub struct VisitorContext {
    pub path: Vec<String>,
    // type of parent component
    pub component_type: String,
}

impl VisitorContext {
    pub fn new() -> Self {
        Self { path: vec![], component_type: "".to_string() }
    }

    pub fn child(&self, name: &str, component_type: &str) -> Self {
        let mut path = self.path.clone();
        path.push(name.to_string());
        Self { path, component_type: component_type.to_string() }
    }
}

pub trait AvengerVisitor: Visitor {
    fn pre_visit_avenger_file(&mut self, _file: &AvengerFile, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }   
    fn post_visit_avenger_file(&mut self, _file: &AvengerFile, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_avgr_statement(&mut self, _statement: &Statement, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_avgr_statement(&mut self, _statement: &Statement, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_import_statement(&mut self, _statement: &ImportStatement, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_import_statement(&mut self, _statement: &ImportStatement, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_val_prop(&mut self, _statement: &ValProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_val_prop(&mut self, _statement: &ValProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_expr_prop(&mut self, _statement: &ExprProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_expr_prop(&mut self, _statement: &ExprProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_dataset_prop(&mut self, _statement: &DatasetProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_dataset_prop(&mut self, _statement: &DatasetProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_component_prop(&mut self, _statement: &ComponentProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_component_prop(&mut self, _statement: &ComponentProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_prop_binding(&mut self, _statement: &PropBinding, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_prop_binding(&mut self, _statement: &PropBinding, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_function_def(&mut self, _statement: &FunctionDef, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_function_def(&mut self, _statement: &FunctionDef, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_function_statement(&mut self, _statement: &FunctionStatement, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_function_statement(&mut self, _statement: &FunctionStatement, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_function_return(&mut self, _statement: &FunctionReturn, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_function_return(&mut self, _statement: &FunctionReturn, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
}

pub trait AvengerVisitorMut: VisitorMut {
    fn pre_visit_avenger_file(&mut self, _file: &mut AvengerFile, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }   
    fn post_visit_avenger_file(&mut self, _file: &mut AvengerFile, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_avgr_statement(&mut self, _statement: &mut Statement, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_avgr_statement(&mut self, _statement: &mut Statement, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_import_statement(&mut self, _statement: &mut ImportStatement, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_import_statement(&mut self, _statement: &mut ImportStatement, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_val_prop(&mut self, _statement: &mut ValProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_val_prop(&mut self, _statement: &mut ValProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_expr_prop(&mut self, _statement: &mut ExprProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_expr_prop(&mut self, _statement: &mut ExprProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_dataset_prop(&mut self, _statement: &mut DatasetProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_dataset_prop(&mut self, _statement: &mut DatasetProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_component_prop(&mut self, _statement: &mut ComponentProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_component_prop(&mut self, _statement: &mut ComponentProp, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_prop_binding(&mut self, _statement: &mut PropBinding, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_prop_binding(&mut self, _statement: &mut PropBinding, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_function_def(&mut self, _statement: &mut FunctionDef, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_function_def(&mut self, _statement: &mut FunctionDef, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_function_statement(&mut self, _statement: &mut FunctionStatement, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_function_statement(&mut self, _statement: &mut FunctionStatement, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_function_return(&mut self, _statement: &mut FunctionReturn, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_function_return(&mut self, _statement: &mut FunctionReturn, _context: &VisitorContext) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
}

pub trait AvengerVisit {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break>;
}

pub trait AvengerVisitMut {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break>;
}

impl AvengerVisit for SqlExprOrQuery {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V, _context: &VisitorContext) -> ControlFlow<V::Break> {
        match self {
            SqlExprOrQuery::Expr(expr) => expr.visit(visitor),
            SqlExprOrQuery::Query(query) => query.visit(visitor),
        }
    }
}

impl AvengerVisitMut for SqlExprOrQuery {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V, _context: &VisitorContext) -> ControlFlow<V::Break> {
        match self {
            SqlExprOrQuery::Expr(expr) => VisitMut::visit(expr, visitor),
            SqlExprOrQuery::Query(query) => VisitMut::visit(query, visitor),
        }
    }
}

impl AvengerVisit for ImportStatement {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_import_statement(self, context)?;
        visitor.post_visit_import_statement(self, context)
    }
}

impl AvengerVisitMut for ImportStatement {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_import_statement(self, context)?;
        visitor.post_visit_import_statement(self, context)
    }
}

impl AvengerVisit for ValProp {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_val_prop(self, context)?;
        self.expr.visit(visitor)?;
        visitor.post_visit_val_prop(self, context)
    }
}

impl AvengerVisitMut for ValProp {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_val_prop(self, context)?;
        VisitMut::visit(&mut self.expr, visitor)?;
        visitor.post_visit_val_prop(self, context)
    }
}

impl AvengerVisit for ExprProp {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_expr_prop(self, context)?;
        self.expr.visit(visitor)?;
        visitor.post_visit_expr_prop(self, context)
    }
}

impl AvengerVisitMut for ExprProp {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_expr_prop(self, context)?;
        VisitMut::visit(&mut self.expr, visitor)?;
        visitor.post_visit_expr_prop(self, context)
    }
}

impl AvengerVisit for DatasetProp {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_dataset_prop(self, context)?;
        self.query.visit(visitor)?;
        visitor.post_visit_dataset_prop(self, context)
    }
}

impl AvengerVisitMut for DatasetProp {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_dataset_prop(self, context)?;
        VisitMut::visit(&mut self.query, visitor)?;
        visitor.post_visit_dataset_prop(self, context)
    }
}

impl AvengerVisit for ComponentProp {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_component_prop(self, context)?;

        // Process statements with child context
        let child_context = context.child(&self.name(), &self.component_type.value);
        for statement in &self.statements {
            statement.visit(visitor, &child_context)?;
        }

        visitor.post_visit_component_prop(self, context)
    }
}

impl AvengerVisitMut for ComponentProp {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_component_prop(self, context)?;

        // Process statements with child context
        let child_context = context.child(&self.name(), &self.component_type.value);
        for statement in &mut self.statements {
            statement.visit(visitor, &child_context)?;
        }

        visitor.post_visit_component_prop(self, context)
    }
}


impl AvengerVisit for PropBinding {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_prop_binding(self, context)?;
        self.expr.visit(visitor, context)?;
        visitor.post_visit_prop_binding(self, context)
    }
}

impl AvengerVisitMut for PropBinding {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_prop_binding(self, context)?;
        AvengerVisitMut::visit(&mut self.expr, visitor, context)?;
        visitor.post_visit_prop_binding(self, context)
    }
}

impl AvengerVisit for FunctionDef {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_function_def(self, context)?;
        for statement in &self.statements {
            statement.visit(visitor, context)?;
        }
        self.return_statement.visit(visitor, context)?;
        visitor.post_visit_function_def(self, context)
    }
}

impl AvengerVisitMut for FunctionDef {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_function_def(self, context)?;
        for statement in &mut self.statements {
            statement.visit(visitor, context)?;
        }
        AvengerVisitMut::visit(&mut self.return_statement, visitor, context)?;
        visitor.post_visit_function_def(self, context)
    }
}

impl AvengerVisit for FunctionStatement {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_function_statement(self, context)?;
        match self {
            FunctionStatement::ValProp(val_prop) => val_prop.visit(visitor, context)?,
            FunctionStatement::ExprProp(expr_prop) => expr_prop.visit(visitor, context)?,
            FunctionStatement::DatasetProp(dataset_prop) => dataset_prop.visit(visitor, context)?,
        }
        visitor.post_visit_function_statement(self, context)
    }
}

impl AvengerVisitMut for FunctionStatement {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_function_statement(self, context)?;
        match self {
            FunctionStatement::ValProp(val_prop) => AvengerVisitMut::visit(val_prop, visitor, context)?,
            FunctionStatement::ExprProp(expr_prop) => AvengerVisitMut::visit(expr_prop, visitor, context)?,
            FunctionStatement::DatasetProp(dataset_prop) => AvengerVisitMut::visit(dataset_prop, visitor, context)?,
        }
        visitor.post_visit_function_statement(self, context)
    }
}

impl AvengerVisit for FunctionReturn {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_function_return(self, context)?;
        self.value.visit(visitor, context)?;
        visitor.post_visit_function_return(self, context)
    }
}

impl AvengerVisitMut for FunctionReturn {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_function_return(self, context)?;
        AvengerVisitMut::visit(&mut self.value, visitor, context)?;
        visitor.post_visit_function_return(self, context)
    }
}

impl AvengerVisit for Statement {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_avgr_statement(self, context)?;
        match self {
            Statement::Import(import_statement) => import_statement.visit(visitor, context)?,
            Statement::ValProp(val_prop) => val_prop.visit(visitor, context)?,
            Statement::ExprProp(expr_prop) => expr_prop.visit(visitor, context)?,
            Statement::DatasetProp(dataset_prop) => dataset_prop.visit(visitor, context)?,
            Statement::ComponentProp(component_prop) => component_prop.visit(visitor, context)?,
            Statement::PropBinding(prop_binding) => prop_binding.visit(visitor, context)?,
            Statement::FunctionDef(function_def) => function_def.visit(visitor, context)?,
        }
        visitor.post_visit_avgr_statement(self, context)
    }
}

impl AvengerVisitMut for Statement {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V, context: &VisitorContext) -> ControlFlow<V::Break> {
        visitor.pre_visit_avgr_statement(self, context)?;
        match self {
            Statement::Import(import_statement) => AvengerVisitMut::visit(import_statement, visitor, context)?,
            Statement::ValProp(val_prop) => AvengerVisitMut::visit(val_prop, visitor, context)?,
            Statement::ExprProp(expr_prop) => AvengerVisitMut::visit(expr_prop, visitor, context)?,
            Statement::DatasetProp(dataset_prop) => AvengerVisitMut::visit(dataset_prop, visitor, context)?,
            Statement::ComponentProp(component_prop) => AvengerVisitMut::visit(component_prop, visitor, context)?,
            Statement::PropBinding(prop_binding) => AvengerVisitMut::visit(prop_binding, visitor, context)?,
            Statement::FunctionDef(function_def) => AvengerVisitMut::visit(function_def, visitor, context)?,
        }
        visitor.post_visit_avgr_statement(self, context)
    }
}

impl AvengerFile {
    pub fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        let context = VisitorContext { 
            path: vec![], component_type: self.name.clone() 
        };
        visitor.pre_visit_avenger_file(self, &context)?;

        // Visit top-level statements in the file
        let child_context = VisitorContext { 
            path: vec![], component_type: self.name.clone() 
        };
        for statement in &self.statements {
            statement.visit(visitor, &child_context)?;
        }

        visitor.post_visit_avenger_file(self, &context)
    }

    pub fn visit_mut<V: AvengerVisitorMut>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        let context = VisitorContext { 
            path: vec![], component_type: self.name.clone() 
        };
        visitor.pre_visit_avenger_file(self, &context)?;
        for statement in &mut self.statements {
            statement.visit(visitor, &context)?;
        }
        visitor.post_visit_avenger_file(self, &context)
    }
}
