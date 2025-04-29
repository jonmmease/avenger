use std::ops::ControlFlow;

use sqlparser::ast::{Visit, Visitor};

use crate::ast::{AvengerFile, ComponentProp, DatasetProp, ExprProp, FunctionDef, FunctionReturn, FunctionStatement, ImportStatement, PropBinding, SqlExprOrQuery, Statement, ValProp};


pub trait AvengerVisitor: Visitor {
    fn pre_visit_avenger_file(&mut self, _file: &AvengerFile) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }   
    fn post_visit_avenger_file(&mut self, _file: &AvengerFile) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_avgr_statement(&mut self, _statement: &Statement) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_avgr_statement(&mut self, _statement: &Statement) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_import_statement(&mut self, _statement: &ImportStatement) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_import_statement(&mut self, _statement: &ImportStatement) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_val_prop(&mut self, _statement: &ValProp) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_val_prop(&mut self, _statement: &ValProp) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_expr_prop(&mut self, _statement: &ExprProp) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_expr_prop(&mut self, _statement: &ExprProp) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_dataset_prop(&mut self, _statement: &DatasetProp) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_dataset_prop(&mut self, _statement: &DatasetProp) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_component_prop(&mut self, _statement: &ComponentProp) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_component_prop(&mut self, _statement: &ComponentProp) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_prop_binding(&mut self, _statement: &PropBinding) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_prop_binding(&mut self, _statement: &PropBinding) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_function_def(&mut self, _statement: &FunctionDef) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_function_def(&mut self, _statement: &FunctionDef) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_function_statement(&mut self, _statement: &FunctionStatement) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_function_statement(&mut self, _statement: &FunctionStatement) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_function_return(&mut self, _statement: &FunctionReturn) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_function_return(&mut self, _statement: &FunctionReturn) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
}

pub trait AvengerVisit {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break>;
}

impl AvengerVisit for SqlExprOrQuery {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        match self {
            SqlExprOrQuery::Expr(expr) => expr.visit(visitor),
            SqlExprOrQuery::Query(query) => query.visit(visitor),
        }
    }
}

impl AvengerVisit for ImportStatement {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_import_statement(self)?;
        visitor.post_visit_import_statement(self)
    }
}

impl AvengerVisit for ValProp {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_val_prop(self)?;
        self.expr.visit(visitor)?;
        visitor.post_visit_val_prop(self)
    }
}

impl AvengerVisit for ExprProp {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_expr_prop(self)?;
        self.expr.visit(visitor)?;
        visitor.post_visit_expr_prop(self)
    }
}

impl AvengerVisit for DatasetProp {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_dataset_prop(self)?;
        self.query.visit(visitor)?;
        visitor.post_visit_dataset_prop(self)
    }
}

impl AvengerVisit for ComponentProp {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_component_prop(self)?;
        for statement in &self.statements {
            statement.visit(visitor)?;
        }
        visitor.post_visit_component_prop(self)
    }
}


impl AvengerVisit for PropBinding {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_prop_binding(self)?;
        self.expr.visit(visitor)?;
        visitor.post_visit_prop_binding(self)
    }
}

impl AvengerVisit for FunctionDef {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_function_def(self)?;
        for statement in &self.statements {
            statement.visit(visitor)?;
        }
        self.return_statement.visit(visitor)?;
        visitor.post_visit_function_def(self)
    }
}

impl AvengerVisit for FunctionStatement {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_function_statement(self)?;
        match self {
            FunctionStatement::ValProp(val_prop) => val_prop.visit(visitor)?,
            FunctionStatement::ExprProp(expr_prop) => expr_prop.visit(visitor)?,
            FunctionStatement::DatasetProp(dataset_prop) => dataset_prop.visit(visitor)?,
        }
        visitor.post_visit_function_statement(self)
    }
}

impl AvengerVisit for FunctionReturn {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_function_return(self)?;
        self.value.visit(visitor)?;
        visitor.post_visit_function_return(self)
    }
}

impl AvengerVisit for Statement {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_avgr_statement(self)?;
        match self {
            Statement::Import(import_statement) => import_statement.visit(visitor)?,
            Statement::ValProp(val_prop) => val_prop.visit(visitor)?,
            Statement::ExprProp(expr_prop) => expr_prop.visit(visitor)?,
            Statement::DatasetProp(dataset_prop) => dataset_prop.visit(visitor)?,
            Statement::ComponentProp(component_prop) => component_prop.visit(visitor)?,
            Statement::PropBinding(prop_binding) => prop_binding.visit(visitor)?,
            Statement::FunctionDef(function_def) => function_def.visit(visitor)?,
        }
        visitor.post_visit_avgr_statement(self)
    }
}

impl AvengerVisit for AvengerFile {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_avenger_file(self)?;
        for statement in &self.statements {
            statement.visit(visitor)?;
        }
        visitor.post_visit_avenger_file(self)
    }
}