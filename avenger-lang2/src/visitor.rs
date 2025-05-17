use std::ops::ControlFlow;

use sqlparser::ast::{CreateFunction, Statement as SqlStatement, Visit, VisitMut, Visitor, VisitorMut};
use crate::ast::{AvengerScript, Block, ExprDecl, IfStmt, ScriptStatement, SqlExprOrQuery, TableDecl, ValDecl, VarAssignment, WhileStmt};

pub trait AvengerVisitor: Visitor {
    fn pre_visit_avenger_script(&mut self, _file: &AvengerScript) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_avenger_script(&mut self, _file: &AvengerScript) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_avgr_statement(&mut self, _statement: &ScriptStatement) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_avgr_statement(&mut self, _statement: &ScriptStatement) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_val_decl(&mut self, _statement: &ValDecl) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_val_decl(&mut self, _statement: &ValDecl) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_expr_decl(&mut self, _statement: &ExprDecl) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_expr_decl(&mut self, _statement: &ExprDecl) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_table_decl(&mut self, _statement: &TableDecl) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_table_decl(&mut self, _statement: &TableDecl) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_var_assignment(&mut self, _statement: &VarAssignment) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_var_assignment(&mut self, _statement: &VarAssignment) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_block(&mut self, _block: &Block) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn post_visit_block(&mut self, _block: &Block) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_if_stmt(&mut self, _if_stmt: &IfStmt) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn post_visit_if_stmt(&mut self, _if_stmt: &IfStmt) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_while_stmt(&mut self, _while_stmt: &WhileStmt) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn post_visit_while_stmt(&mut self, _while_stmt: &WhileStmt) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
}


pub trait AvengerVisitorMut: VisitorMut {
    fn pre_visit_avenger_script(&mut self, _file: &mut AvengerScript) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_avenger_script(&mut self, _file: &mut AvengerScript) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_avgr_statement(&mut self, _statement: &mut ScriptStatement) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_avgr_statement(&mut self, _statement: &mut ScriptStatement) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_val_decl(&mut self, _statement: &mut ValDecl) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_val_decl(&mut self, _statement: &mut ValDecl) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_expr_decl(&mut self, _statement: &mut ExprDecl) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_expr_decl(&mut self, _statement: &mut ExprDecl) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_table_decl(&mut self, _statement: &mut TableDecl) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_table_decl(&mut self, _statement: &mut TableDecl) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_var_assignment(&mut self, _statement: &mut VarAssignment) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
    fn post_visit_var_assignment(&mut self, _statement: &mut VarAssignment) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_block(&mut self, _block: &mut Block) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn post_visit_block(&mut self, _block: &mut Block) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_if_stmt(&mut self, _if_stmt: &mut IfStmt) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn post_visit_if_stmt(&mut self, _if_stmt: &mut IfStmt) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_while_stmt(&mut self, _while_stmt: &mut WhileStmt) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn post_visit_while_stmt(&mut self, _while_stmt: &mut WhileStmt) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }
}

pub trait AvengerVisit {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break>;
}

pub trait AvengerVisitMut {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V) -> ControlFlow<V::Break>;
}

impl AvengerVisit for SqlExprOrQuery {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        match self {
            SqlExprOrQuery::Expr(expr) => expr.visit(visitor),
            SqlExprOrQuery::Query(query) => query.visit(visitor),
        }
    }
}

impl AvengerVisitMut for SqlExprOrQuery {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        match self {
            SqlExprOrQuery::Expr(expr) => VisitMut::visit(expr, visitor),
            SqlExprOrQuery::Query(query) => VisitMut::visit(query, visitor),
        }
    }
}

impl AvengerVisit for ValDecl {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_val_decl(self)?;
        self.expr.visit(visitor)?;
        visitor.post_visit_val_decl(self)
    }
}

impl AvengerVisitMut for ValDecl {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_val_decl(self)?;
        VisitMut::visit(&mut self.expr, visitor)?;
        visitor.post_visit_val_decl(self)
    }
}

impl AvengerVisit for ExprDecl {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_expr_decl(self)?;
        self.expr.visit(visitor)?;
        visitor.post_visit_expr_decl(self)
    }
}

impl AvengerVisitMut for ExprDecl {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_expr_decl(self)?;
        VisitMut::visit(&mut self.expr, visitor)?;
        visitor.post_visit_expr_decl(self)
    }
}

impl AvengerVisit for TableDecl {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_table_decl(self)?;
        self.query.visit(visitor)?;
        visitor.post_visit_table_decl(self)
    }
}

impl AvengerVisitMut for TableDecl {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_table_decl(self)?;
        VisitMut::visit(&mut self.query, visitor)?;
        visitor.post_visit_table_decl(self)
    }
}

impl AvengerVisit for VarAssignment {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_var_assignment(self)?;
        self.expr.visit(visitor)?;
        visitor.post_visit_var_assignment(self)
    }
}

impl AvengerVisitMut for VarAssignment {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_var_assignment(self)?;
        AvengerVisitMut::visit(&mut self.expr, visitor)?;
        visitor.post_visit_var_assignment(self)
    }
}

impl AvengerVisit for Block {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_block(self)?;
        for statement in &self.statements {
            statement.visit(visitor)?;
        }
        visitor.post_visit_block(self)
    }
}

impl AvengerVisitMut for Block {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_block(self)?;
        for statement in &mut self.statements {
            statement.visit(visitor)?;
        }
        visitor.post_visit_block(self)
    }
}

impl AvengerVisit for IfStmt {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_if_stmt(self)?;
        for branch in &self.branches {
            branch.condition.visit(visitor)?;
            branch.body.visit(visitor)?;
        }
        if let Some(else_branch) = &self.else_branch {
            else_branch.body.visit(visitor)?;
        }
        visitor.post_visit_if_stmt(self)
    }
}

impl AvengerVisitMut for IfStmt {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_if_stmt(self)?;
        for branch in &mut self.branches {
            VisitMut::visit(&mut branch.condition, visitor)?;
            AvengerVisitMut::visit(&mut branch.body, visitor)?;
        }
        if let Some(else_branch) = &mut self.else_branch {
            AvengerVisitMut::visit(&mut else_branch.body, visitor)?;
        }
        visitor.post_visit_if_stmt(self)
    }
}

impl AvengerVisit for WhileStmt {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_while_stmt(self)?;

        // visit condition
        self.condition.visit(visitor)?;

        // visit body
        self.body.visit(visitor)?;

        visitor.post_visit_while_stmt(self)
    }
}

impl AvengerVisitMut for WhileStmt {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_while_stmt(self)?;

        // visit condition
        VisitMut::visit(&mut self.condition, visitor)?;

        // visit body
        AvengerVisitMut::visit(&mut self.body, visitor)?;

        visitor.post_visit_while_stmt(self)
    }
}

impl AvengerVisit for ScriptStatement {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_avgr_statement(self)?;
        match self {
            ScriptStatement::ValDecl(val_decl) => val_decl.visit(visitor),
            ScriptStatement::ExprDecl(expr_decl) => expr_decl.visit(visitor),
            ScriptStatement::TableDecl(table_decl) => table_decl.visit(visitor),
            ScriptStatement::VarAssignment(var_assignment) => var_assignment.visit(visitor),
            ScriptStatement::Block(block) => block.visit(visitor),
            ScriptStatement::IfStmt(if_statement) => if_statement.visit(visitor),
            ScriptStatement::WhileStmt(while_statement) => while_statement.visit(visitor),
        }?;
        visitor.post_visit_avgr_statement(self)
    }
}


impl AvengerVisitMut for ScriptStatement {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_avgr_statement(self)?;
        match self {
            ScriptStatement::ValDecl(val_decl) => val_decl.visit(visitor),
            ScriptStatement::ExprDecl(expr_decl) => expr_decl.visit(visitor),
            ScriptStatement::TableDecl(table_decl) => table_decl.visit(visitor),
            ScriptStatement::VarAssignment(var_assignment) => var_assignment.visit(visitor),
            ScriptStatement::Block(block) => block.visit(visitor),
            ScriptStatement::IfStmt(if_statement) => if_statement.as_mut().visit(visitor),
            ScriptStatement::WhileStmt(while_statement) => while_statement.as_mut().visit(visitor),
        }?;
        visitor.post_visit_avgr_statement(self)
    }
}

impl AvengerVisit for AvengerScript {
    fn visit<V: AvengerVisitor>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_avenger_script(self)?;
        for statement in &self.statements {
            statement.visit(visitor)?;
        }
        visitor.post_visit_avenger_script(self)
    }
}

impl AvengerVisitMut for AvengerScript {
    fn visit<V: AvengerVisitorMut>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.pre_visit_avenger_script(self)?;
        for statement in &mut self.statements {
            statement.visit(visitor)?;
        }
        visitor.post_visit_avenger_script(self)
    }
}