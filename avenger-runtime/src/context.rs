use std::{collections::HashMap, ops::ControlFlow, sync::{Arc, Mutex}};

use datafusion::{
    arrow::array::record_batch, 
    datasource::MemTable, 
    logical_expr::{ColumnarValue, LogicalPlan}, 
    prelude::{DataFrame, SessionContext}, 
    variable::{VarProvider, VarType}
};
use datafusion_common::{DFSchema, DataFusionError, ScalarValue};
use datafusion_sql::TableReference;
use sqlparser::ast::{Expr as SqlExpr, Query as SqlQuery, VisitMut};
use crate::{error::AvengerRuntimeError, visitors::CompilationVisitor};
use crate::{value::{ArrowTable, TaskDataset, TaskValue, TaskValueContext}, variable::Variable};


/// The context for evaluating tasks
///  - When a Val tasks is evaluated, the value is stored in DataFusion sessions context as a variable
///  - When a Dataset taks is evaluated, the dataset is stored in DataFusion sessions context as a table
///  - When an Expr task is evaluated, the expression is stored in the exprs prop (since there's not 
///    a place to store it in the SessionContext)
pub struct TaskEvaluationContext {
    session_ctx: SessionContext,
    val_provider: Arc<EvaluationValProvider>,
    exprs: Arc<Mutex<HashMap<Variable, SqlExpr>>>,
}

impl TaskEvaluationContext {
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
    pub async fn register_values(&self, variables: &[Variable], values: &[TaskValue]) -> Result<(), AvengerRuntimeError> {
        for (variable, value) in variables.iter().zip(values.iter()) {
            match value {
                TaskValue::Val { value: val } => self.register_val(&variable, val.clone())?,
                TaskValue::Expr { sql_expr: expr, context } => {
                    self.register_task_value_context(&context).await?;
                    self.register_expr(&variable, expr.clone())?;
                }
                TaskValue::Dataset { dataset, context } => {
                    self.register_task_value_context(&context).await?;
                    self.register_dataset(&variable, dataset.clone()).await?;
                }
                _ => {
                    // skip
                }
            };
        }
        Ok(())
    }

    pub async fn register_task_value_context(&self, context: &TaskValueContext) -> Result<(), AvengerRuntimeError> {
        for (name, value) in context.values().iter() {
            self.register_val(&name, value.clone())?;
        }
        for (name, dataset) in context.datasets().iter() {
            self.register_dataset(&name, dataset.clone()).await?;
        }
        Ok(())
    }

    /// Get the underlying DataFusion SessionContext
    pub fn session_ctx(&self) -> &SessionContext {
        &self.session_ctx
    }

    /// Register a DataFrame in the context under a mangled name
    pub async fn register_dataset(&self, variable: &Variable, dataset: TaskDataset) -> Result<(), AvengerRuntimeError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerRuntimeError::InternalError(
                format!("Dataset name should not include the @ prefix: {}", variable.name())
            ));
        }
        
        // Replace . with __ in the table name so that DataFusion doesn't interpret it as a schema
        let mangled_name = variable.mangled_var_name();
        match dataset {
            TaskDataset::LogicalPlan(plan) => {
                let df = self.session_ctx.execute_logical_plan(plan).await?;
                self.session_ctx.register_table(mangled_name.clone(), df.into_view())?;
            }
            TaskDataset::ArrowTable(table) => {
                let table = MemTable::try_new(table.schema.clone(), vec![table.batches.clone()])?;
                self.session_ctx.register_table(
                    TableReference::Bare {
                        table: mangled_name.into(),
                    },
                    Arc::new(table),
                )?;
            }
        }
        Ok(())
    }

    /// Get a registered dataset from the context
    pub async fn get_dataset(&self, variable: &Variable) -> Result<DataFrame, AvengerRuntimeError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerRuntimeError::InternalError(
                format!("Dataset name should not include the @ prefix: {}", variable.name())
            ));
        }
        let mangled_name = variable.mangled_var_name();
        let df = self.session_ctx.table(&mangled_name).await?;
        Ok(df)
    }

    /// Check if a dataset is registered in the context, handling mangling
    pub fn has_dataset(&self, variable: &Variable) -> bool {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            panic!("Dataset name should not include the @ prefix: {}", variable.name());
        }
        let mangled_name = variable.mangled_var_name();
        self.session_ctx.table_exist(&mangled_name).unwrap_or(false)
    }

    /// Register a value in the context
    pub fn register_val(&self, variable: &Variable, val: ScalarValue) -> Result<(), AvengerRuntimeError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerRuntimeError::InternalError(
                format!("Val name should not include the @ prefix: {}", variable.name())
            ));
        }
        self.val_provider.insert(variable.clone(), val);
        Ok(())
    }

    /// Get a value from the context
    pub fn get_val(&self, variable: &Variable) -> Result<ScalarValue, AvengerRuntimeError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerRuntimeError::InternalError(
                format!("Val name should not include the @ prefix: {}", variable.name())
            ));
        }
        let val = self.val_provider.get_scalar_value(variable)?;
        Ok(val)
    }

    /// Check if a value is registered in the context
    pub fn has_val(&self, variable: &Variable) -> bool {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            panic!("Val name should not include the @ prefix: {}", variable.name());
        }
        self.val_provider.has_variable(variable)
    }

    /// Add an expression to the context
    pub fn register_expr(&self, variable: &Variable, sql_expr: SqlExpr) -> Result<(), AvengerRuntimeError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            panic!("Expr name should not include the @ prefix: {}", variable.name());
        }
        self.exprs.lock().unwrap().insert(variable.clone(), sql_expr);
        Ok(())
    }

    /// Get an expression from the context
    pub fn get_expr(&self, variable: &Variable) -> Result<SqlExpr, AvengerRuntimeError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerRuntimeError::InternalError(
                format!("Expr name should not include the @ prefix: {}", variable.name())
            ));
        }   
        let locked = self.exprs.lock().unwrap();
        let expr = locked.get(variable).ok_or(
            AvengerRuntimeError::ExpressionNotFound(format!("Expression {} not found", variable.name()))
        )?;
        Ok(expr.clone())
    }

    /// Check if an expression is stored in the context
    pub fn has_expr(&self, variable: &Variable) -> bool {
        self.exprs.lock().unwrap().contains_key(variable)
    }

    /// Compile a SQL query to a logical plan, expanding sql with referenced expressions
    pub async fn compile_query(&self, query: &SqlQuery) -> Result<LogicalPlan, AvengerRuntimeError> {
        // Visit the query and validate references
        let mut expanded_query = self.expand_query(&query)?;
        let mut visitor = CompilationVisitor::new(&self);
        if let ControlFlow::Break(Result::Err(err)) = VisitMut::visit(
            &mut expanded_query, &mut visitor
        ) {
            return Err(err);
        }
        let plan = self.session_ctx.state().create_logical_plan(&expanded_query.to_string()).await?;
        Ok(plan)
    }

    pub async fn eval_query(&self, query: &SqlQuery) -> Result<ArrowTable, AvengerRuntimeError> {
        let plan = self.compile_query(query).await?;
        let df = self.session_ctx.execute_logical_plan(plan).await?;
        let schema = df.schema().clone();
        let table = df.collect().await?;
        Ok(ArrowTable::try_new(schema.inner().clone(), table)?)
    }

    pub async fn eval_plan(&self, plan: LogicalPlan) -> Result<ArrowTable, AvengerRuntimeError> {
        let df = self.session_ctx.execute_logical_plan(plan).await?;
        let schema = df.schema().clone();
        let table = df.collect().await?;
        Ok(ArrowTable::try_new(schema.inner().clone(), table)?)
    }

    /// Expand the sql expression with referenced expressions into a single expression,
    /// 
    /// Referenced expressions are inlined into the expression
    pub fn expand_expr(&self, expr: &SqlExpr) -> Result<SqlExpr, AvengerRuntimeError> {
        // Visit the query and validate references
        let mut expr = expr.clone();
        let mut visitor = CompilationVisitor::new(&self);
        if let ControlFlow::Break(Result::Err(err)) = expr.visit(&mut visitor) {
            return Err(err);
        }
        Ok(expr)
    }

    pub fn expand_query(&self, query: &SqlQuery) -> Result<SqlQuery, AvengerRuntimeError> {
        let mut query = query.clone();
        let mut visitor = CompilationVisitor::new(&self);
        if let ControlFlow::Break(Result::Err(err)) = query.visit(&mut visitor) {
            return Err(err);
        }
        Ok(query)
    }

    pub async fn evaluate_expr(&self, expr: &SqlExpr) -> Result<ScalarValue, AvengerRuntimeError> {
        let sql_expr = self.expand_expr(expr)?;
        let expr = self.session_ctx.parse_sql_expr(&sql_expr.to_string(), &DFSchema::empty())?;

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
                    return Err(AvengerRuntimeError::InternalError("Array value not expected".to_string()));
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
    vals: Arc<Mutex<HashMap<Variable, ScalarValue>>>,
}

impl EvaluationValProvider {
    pub fn new() -> Self {
        Self { vals: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn insert(&self, variable: Variable, val: ScalarValue) {
        self.vals.lock().unwrap().insert(variable, val);
    }
    
    pub fn get_scalar_value(&self, variable: &Variable) -> datafusion_common::Result<ScalarValue> {
        let val = self.vals.lock().unwrap().get(variable).cloned().ok_or(
            DataFusionError::Internal(format!("Variable {} not found", variable.name()))
        )?;
        Ok(val)
    }
    
    pub fn has_variable(&self, variable: &Variable) -> bool {
        self.vals.lock().unwrap().contains_key(variable)
    }
}

impl VarProvider for EvaluationValProvider {
    fn get_value(&self, var_names: Vec<String>) -> datafusion_common::Result<ScalarValue> {
        // Create a Variable from the provided var_names, stripping @ from first part if present
        let mut parts = var_names.clone();
        if !parts.is_empty() {
            // Always strip @ from the first part if present
            if let Some(name) = parts[0].strip_prefix("@") {
                parts[0] = name.to_string();
            }
        }
        let variable = Variable::new(parts);
        
        let val = self.vals.lock().unwrap().get(&variable).cloned().ok_or(
            DataFusionError::Internal(format!("Variable {} not found", variable.name()))
        )?;
        Ok(val)
    }
    
    fn get_type(&self, var_names: &[String]) -> Option<arrow_schema::DataType> {
        // Create a Variable from the provided var_names, stripping @ from first part if present
        let mut parts = var_names.to_vec();
        if !parts.is_empty() {
            // Always strip @ from the first part if present
            if let Some(name) = parts[0].strip_prefix("@") {
                parts[0] = name.to_string();
            }
        }
        let variable = Variable::new(parts);
        
        let locked = self.vals.lock().unwrap();
        let val = locked.get(&variable)?;
        Some(val.data_type())
    }
}

