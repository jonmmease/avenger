use std::{collections::HashMap, ops::ControlFlow, sync::{Arc, Mutex}};

use datafusion::{arrow::array::record_batch, logical_expr::{ColumnarValue, LogicalPlan}, prelude::{DataFrame, Expr, SessionContext}, variable::{VarProvider, VarType}};
use datafusion_common::{DFSchema, DataFusionError, ScalarValue};
use datafusion_sql::unparser::expr_to_sql;
use sqlparser::ast::{Expr as SqlExpr, Query as SqlQuery, VisitMut, VisitorMut};
use crate::{error::AvengerLangError, task_graph::value::{TaskValue, Variable, VariableKind}};


const MANGLED_PREFIX: &str = "_at_";

/// Mangle a name to avoid conflicts with existing variables.
/// Accepts names with or without the @ prefix.
/// Detects whether the name is already mangled and returns the mangled 
/// name unchanged.
/// 
/// "name" -> "_at_name"
/// "@name" -> "_at_name"
/// "_at_name" -> "_at_name"
pub fn mangle_name(name: &str) -> String {
    format!("{}{}", MANGLED_PREFIX, unmangle_name(name))
}

/// Unmangle a name if mangled, otherwise return the name unchanged
/// 
/// "_at_name" -> "name"
/// "name" -> "name"
/// "@name" -> "name"
pub fn unmangle_name(name: &str) -> String {
    let name = name.trim_start_matches('@');
    if name.starts_with(MANGLED_PREFIX) {
        name[MANGLED_PREFIX.len()..].to_string()
    } else {
        name.to_string()
    }
}   


/// The context for evaluating tasks
///  - When a Val tasks is evaluated, the value is stored in DataFusion sessions context as a variable
///  - When a Dataset taks is evaluated, the dataset is stored in DataFusion sessions context as a table
///  - When an Expr task is evaluated, the expression is stored in the exprs prop (since there's not 
///    aplace to store it in the SessionContext)
pub struct EvaluationContext {
    session_ctx: SessionContext,

    // Expressions already evaluated, stored using plain
    exprs: Arc<Mutex<HashMap<String, Expr>>>,

    // Values already evaluated, stored using plain
    val_provider: Arc<EvaluationValProvider>,
}

impl EvaluationContext {
    pub fn new() -> Self {
        // Build a session context with our own variable provider
        let session_ctx = SessionContext::new();
        let val_provider = Arc::new(EvaluationValProvider::new());
        session_ctx.register_variable(VarType::UserDefined, val_provider.clone());
        Self {
            session_ctx,
            exprs: Arc::new(Mutex::new(HashMap::new())),
            val_provider, 
        }
    }

    /// Register values corresponding to variables in the context
    pub async fn register_values(&self, variables: &[Variable], values: &[TaskValue]) -> Result<(), AvengerLangError> {
        for (variable, value) in variables.iter().zip(values.iter()) {
            match (&variable.kind, value) {
                (VariableKind::Val, TaskValue::Val(val)) => self.register_val(&variable.name, val.clone())?,
                (VariableKind::ValOrExpr, TaskValue::Val(val)) => self.register_val(&variable.name, val.clone())?,
                (VariableKind::ValOrExpr, TaskValue::Expr(expr)) => self.register_expr(&variable.name, expr.clone())?,
                (VariableKind::Dataset, TaskValue::Dataset(plan)) => self.register_dataset(&variable.name, plan.clone()).await?,
                _ => return Err(
                    AvengerLangError::InternalError(format!("Invalid variable kind and value type: {:?} {:?}", variable.kind, value))
                ),
            };
        }
        Ok(())
    }

    /// Get the underlying DataFusion SessionContext
    pub fn session_ctx(&self) -> &SessionContext {
        &self.session_ctx
    }

    /// Register a DataFrame in the context under a mangled name
    /// 
    /// Maybe add an evaluation option in the future to control whether it's stored as a view
    /// or evaluated and registered as in-memory table
    pub async fn register_dataset(&self, name: &str, plan: LogicalPlan) -> Result<(), AvengerLangError> {
        if !name.starts_with("@") {
            return Err(AvengerLangError::InternalError(format!("Dataset name should start with @ prefix: {}", name)));
        }
        let df = self.session_ctx.execute_logical_plan(plan).await?;
        self.session_ctx.register_table(name, df.into_view())?;
        Ok(())
    }

    /// Get a registered dataset from the context
    pub async fn get_dataset(&self, name: &str) -> Result<DataFrame, AvengerLangError> {
        let df = self.session_ctx.table(name).await?;
        Ok(df)
    }

    /// Check if a dataset is registered in the context, handling mangling
    pub fn has_dataset(&self, name: &str) -> bool {
        self.session_ctx.table_exist(name).unwrap_or(false)
    }

    /// Register a value in the context
    pub fn register_val(&self, name: &str, val: ScalarValue) -> Result<(), AvengerLangError> {
        self.val_provider.insert(format!("@{}", name), val);
        Ok(())
    }

    /// Get a value from the context
    pub fn get_val(&self, name: &str) -> Result<ScalarValue, AvengerLangError> {
        let val = self.val_provider.get_value(vec![format!("@{}", name)])?;
        Ok(val)
    }

    /// Check if a value is registered in the context
    pub fn has_val(&self, name: &str) -> bool {
        self.val_provider.get_type(&[format!("@{}", name)]).is_some()
    }

    /// Add an expression to the context
    pub fn register_expr(&self, name: &str, expr: Expr) -> Result<(), AvengerLangError> {
        self.exprs.lock().unwrap().insert(format!("@{}", name), expr);
        Ok(())
    }

    /// Get an expression from the context
    pub fn get_expr(&self, name: &str) -> Result<Expr, AvengerLangError> {
        let locked = self.exprs.lock().unwrap();
        let expr = locked.get(&format!("@{}", name)).ok_or(
            AvengerLangError::ExpressionNotFound(format!("Expression {} not found", name))
        )?;
        Ok(expr.clone())
    }

    /// Check if an expression is stored in the context
    pub fn has_expr(&self, name: &str) -> bool {
        self.exprs.lock().unwrap().contains_key(&format!("@{}", name))
    }

    /// Compile a SQL query to a logical plan, expanding sql with referenced expressions
    pub async fn compile_query(&self, query: &SqlQuery) -> Result<LogicalPlan, AvengerLangError> {
        // Visit the query and validate references
        let mut query = query.clone();
        let mut visitor = CompilationVisitor::new(&self);
        if let ControlFlow::Break(Result::Err(err)) = query.visit(&mut visitor) {
            return Err(err);
        }
        let plan = self.session_ctx.state().create_logical_plan(&query.to_string()).await?;
        Ok(plan)
    }

    /// Compile a SQL expression to a logical expression, expanding sql with referenced expressions
    pub fn compile_expr(&self, expr: &SqlExpr) -> Result<Expr, AvengerLangError> {
        // Visit the query and validate references
        let mut expr = expr.clone();
        let mut visitor = CompilationVisitor::new(&self);
        if let ControlFlow::Break(Result::Err(err)) = expr.visit(&mut visitor) {
            return Err(err);
        }

        let expr = self.session_ctx.parse_sql_expr(&expr.to_string(), &DFSchema::empty())?;
        Ok(expr)
    }

    pub async fn evaluate_expr(&self, expr: &SqlExpr) -> Result<ScalarValue, AvengerLangError> {
        let expr = self.compile_expr(expr)?;
        let val = self.session_ctx.create_physical_expr(expr, &DFSchema::empty())?;
        let col_val = val.evaluate(
            &record_batch!(
                ("_dummy", Int32, [1])
            ).unwrap()
        )?;
        match col_val {
            ColumnarValue::Scalar(scalar_value) => Ok(scalar_value),
            ColumnarValue::Array(array) => {
                if array.len() != 1 {
                    return Err(AvengerLangError::InternalError("Array value not expected".to_string()));
                }
                // Handle single element array
                let val = ScalarValue::try_from_array(array.as_ref(), 0)?;
                Ok(val)
            }
        }
    }

}


#[derive(Debug, Clone)]
struct EvaluationValProvider {
    vals: Arc<Mutex<HashMap<String, ScalarValue>>>,
}

impl EvaluationValProvider {
    pub fn new() -> Self {
        Self { vals: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn insert(&self, name: String, val: ScalarValue) {
        self.vals.lock().unwrap().insert(name, val);
    }
}

impl VarProvider for EvaluationValProvider {
    fn get_value(&self, var_names: Vec<String>) -> datafusion_common::Result<ScalarValue> {
        let val = self.vals.lock().unwrap().get(&var_names[0]).cloned().ok_or(
            DataFusionError::Internal(format!("Variable {} not found", var_names[0]))
        )?;
        Ok(val)
    }
    
    fn get_type(&self, var_names: &[String]) -> Option<arrow_schema::DataType> {
        let locked = self.vals.lock().unwrap();
        let val = locked.get(&var_names[0])?;
        Some(val.data_type())
    }
}


pub struct CompilationVisitor<'a> {
    ctx: &'a EvaluationContext,
}

impl<'a> CompilationVisitor<'a> {
    pub fn new(ctx: &'a EvaluationContext) -> Self {
        Self { ctx }
    }
}

impl<'a> VisitorMut for CompilationVisitor<'a> {
    type Break = Result<(), AvengerLangError>;

    /// Replace tables of the form @table_name with the true mangled table name
    fn pre_visit_relation(&mut self, relation: &mut datafusion_sql::sqlparser::ast::ObjectName) -> ControlFlow<Self::Break> {
        let table_name = relation.to_string();

        // Validate dataset reference exists. Ignore relations that don't start with @
        if table_name.starts_with("@") && !self.ctx.has_dataset(&table_name) {
            return ControlFlow::Break(Err(AvengerLangError::InternalError(format!("Dataset {} not found", table_name))));
        }

        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &mut SqlExpr) -> ControlFlow<Self::Break> {
        if let SqlExpr::Identifier(ident) = expr.clone() {
            if ident.value.starts_with("@") {
                // Check if this is a reference to an expression
                if let Ok(registered_expr) = self.ctx.get_expr(&ident.value) {
                    println!("Registered expr: {:#?}", registered_expr);
                    match expr_to_sql(&registered_expr) {
                        Ok(sql_expr) => {
                            *expr = sql_expr;
                            return ControlFlow::Continue(());
                        }
                        Err(err) => {
                            return ControlFlow::Break(
                                Err(AvengerLangError::InternalError(format!("Failed to unparse expression {}\n{:?}", ident.value, err)))
                            );
                        }
                    }
                }
                
                // Otherwise it must be a reference to a value
                if !self.ctx.has_val(&ident.value) {
                    return ControlFlow::Break(Err(AvengerLangError::InternalError(format!("Val or Expr {} not found", ident.value))));
                }
            }
        }
        ControlFlow::Continue(())
    }
}



