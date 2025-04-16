use std::{collections::HashMap, ops::ControlFlow, sync::{Arc, Mutex}};

use datafusion::{arrow::array::record_batch, datasource::MemTable, logical_expr::{ColumnarValue, LogicalPlan}, prelude::{DataFrame, Expr, SessionContext}, variable::{VarProvider, VarType}};
use datafusion_common::{DFSchema, DataFusionError, ScalarValue};
use datafusion_sql::{unparser::expr_to_sql, TableReference};
use sqlparser::ast::{Expr as SqlExpr, Ident, ObjectName, Query as SqlQuery, VisitMut, VisitorMut};
use crate::{error::AvengerLangError, task_graph::{dependency::{Dependency, DependencyKind}, value::{ArrowTable, TaskDataset, TaskValue, TaskValueContext}, variable::Variable}};



/// The context for evaluating tasks
///  - When a Val tasks is evaluated, the value is stored in DataFusion sessions context as a variable
///  - When a Dataset taks is evaluated, the dataset is stored in DataFusion sessions context as a table
///  - When an Expr task is evaluated, the expression is stored in the exprs prop (since there's not 
///    aplace to store it in the SessionContext)
pub struct EvaluationContext {
    session_ctx: SessionContext,

    // Expressions already evaluated, stored using Variable
    exprs: Arc<Mutex<HashMap<Variable, SqlExpr>>>,

    // Values already evaluated, stored using Variable
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
            };
        }
        Ok(())
    }

    pub async fn register_task_value_context(&self, context: &TaskValueContext) -> Result<(), AvengerLangError> {
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
    /// 
    /// Maybe add an evaluation option in the future to control whether it's stored as a view
    /// or evaluated and registered as in-memory table
    pub async fn register_dataset(&self, variable: &Variable, dataset: TaskDataset) -> Result<(), AvengerLangError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerLangError::InternalError(format!("Dataset name should not include the @ prefix: {}", variable.name())));
        }
        let table_name = format!("@{}", variable.name()).replace(".", "__");
        println!("Registering dataset: {}", table_name);
        match dataset {
            TaskDataset::LogicalPlan(plan) => {
                let df = self.session_ctx.execute_logical_plan(plan).await?;
                self.session_ctx.register_table(table_name.clone(), df.into_view())?;
            }
            TaskDataset::ArrowTable(table) => {
                let table = MemTable::try_new(table.schema.clone(), vec![table.batches.clone()])?;
                self.session_ctx.register_table(
                    TableReference::Bare {
                        table: table_name.into(),
                    },
                    Arc::new(table),
                )?;
            }
        }
        Ok(())
    }

    /// Get a registered dataset from the context
    pub async fn get_dataset(&self, variable: &Variable) -> Result<DataFrame, AvengerLangError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerLangError::InternalError(format!("Dataset name should not include the @ prefix: {}", variable.name())));
        }
        let df = self.session_ctx.table(&format!("@{}", variable.name())).await?;
        Ok(df)
    }

    /// Check if a dataset is registered in the context, handling mangling
    pub fn has_dataset(&self, variable: &Variable) -> bool {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            panic!("Dataset name should not include the @ prefix: {}", variable.name());
        }
        self.session_ctx.table_exist(&format!("@{}", variable.name())).unwrap_or(false)
    }

    /// Register a value in the context
    pub fn register_val(&self, variable: &Variable, val: ScalarValue) -> Result<(), AvengerLangError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerLangError::InternalError(format!("Val name should not include the @ prefix: {}", variable.name())));
        }
        self.val_provider.insert(variable.clone(), val);
        Ok(())
    }

    /// Get a value from the context
    pub fn get_val(&self, variable: &Variable) -> Result<ScalarValue, AvengerLangError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerLangError::InternalError(format!("Val name should not include the @ prefix: {}", variable.name())));
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
    pub fn register_expr(&self, variable: &Variable, sql_expr: SqlExpr) -> Result<(), AvengerLangError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            panic!("Expr name should not include the @ prefix: {}", variable.name());
        }
        self.exprs.lock().unwrap().insert(variable.clone(), sql_expr);
        Ok(())
    }

    /// Get an expression from the context
    pub fn get_expr(&self, variable: &Variable) -> Result<SqlExpr, AvengerLangError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerLangError::InternalError(format!("Expr name should not include the @ prefix: {}", variable.name())));
        }   
        let locked = self.exprs.lock().unwrap();
        let expr = locked.get(variable).ok_or(
            AvengerLangError::ExpressionNotFound(format!("Expression {} not found", variable.name()))
        )?;
        Ok(expr.clone())
    }

    /// Check if an expression is stored in the context
    pub fn has_expr(&self, variable: &Variable) -> bool {
        self.exprs.lock().unwrap().contains_key(variable)
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

    pub async fn eval_query(&self, query: &SqlQuery) -> Result<ArrowTable, AvengerLangError> {
        let plan = self.compile_query(query).await?;
        let df = self.session_ctx.execute_logical_plan(plan).await?;
        let schema = df.schema().clone();
        let table = df.collect().await?;
        Ok(ArrowTable::try_new(schema.inner().clone(), table)?)
    }

    pub async fn eval_plan(&self, plan: LogicalPlan) -> Result<ArrowTable, AvengerLangError> {
        let df = self.session_ctx.execute_logical_plan(plan).await?;
        let schema = df.schema().clone();
        let table = df.collect().await?;
        Ok(ArrowTable::try_new(schema.inner().clone(), table)?)
    }

    /// Expand the sql expression with referenced expressions into a single expression,
    /// 
    /// Referenced expressions are inlined into the expression
    pub fn expand_expr(&self, expr: &SqlExpr) -> Result<SqlExpr, AvengerLangError> {
        // Visit the query and validate references
        let mut expr = expr.clone();
        let mut visitor = CompilationVisitor::new(&self);
        if let ControlFlow::Break(Result::Err(err)) = expr.visit(&mut visitor) {
            return Err(err);
        }
        Ok(expr)
    }

    pub async fn evaluate_expr(&self, expr: &SqlExpr) -> Result<ScalarValue, AvengerLangError> {
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
        let variable = Variable::with_parts(parts);

        println!("Getting variable: {:?}", variable);
        
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
        let variable = Variable::with_parts(parts);
        
        let locked = self.vals.lock().unwrap();
        let val = locked.get(&variable)?;
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

    fn pre_visit_relation(&mut self, relation: &mut datafusion_sql::sqlparser::ast::ObjectName) -> ControlFlow<Self::Break> {
        let table_name = relation.to_string();
    
        if table_name.starts_with("@") {
            let mut parts = relation.0.iter().map(|ident| ident.value.clone()).collect::<Vec<_>>();

            // Join on __ into a single string
            parts = vec![parts.join("__")];

            // // Check if the dataset exists
            // let variable = Variable::with_parts(parts.clone());
            // if !self.ctx.has_dataset(&variable) {
            //     return ControlFlow::Break(
            //         Err(AvengerLangError::InternalError(format!("Dataset {} not found", variable.name())))
            //     );
            // }

            // Update the relation to use the mangled name
            let idents = parts.iter().map(|s| Ident::new(s.to_string())).collect::<Vec<_>>();

            *relation = ObjectName(idents);
        }

        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &mut SqlExpr) -> ControlFlow<Self::Break> {
        if let SqlExpr::Identifier(ident) = expr.clone() {
            if ident.value.starts_with("@") {
                let mut parts: Vec<String> = ident.value.split(".").map(|s| s.to_string()).collect();
                // Drop the leading @
                parts[0] = parts[0][1..].to_string();
                let variable = Variable::with_parts(parts);

                // Check if this is a reference to an expression
                if let Ok(registered_expr) = self.ctx.get_expr(&variable) {
                    *expr = SqlExpr::Nested(Box::new(registered_expr.clone()));
                    return ControlFlow::Continue(());
                }
                
                // Otherwise it must be a reference to a value
                if !self.ctx.has_val(&variable) {
                    return ControlFlow::Break(Err(AvengerLangError::InternalError(
                        format!("Val or Expr {} not found", variable.name())))
                    );
                }
            }
        }
        ControlFlow::Continue(())
    }
}



