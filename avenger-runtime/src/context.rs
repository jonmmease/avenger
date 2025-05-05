use std::{
    collections::HashMap,
    ops::ControlFlow,
    sync::{Arc, Mutex},
};

use crate::{error::AvengerRuntimeError, function_factory::CustomFunctionFactory, udtf::read_csv::LocalCsvTableFunc};
use crate::{
    value::{ArrowTable, TaskDataset, TaskValue, TaskValueContext},
    variable::Variable,
};
use arrow::array::AsArray;
use datafusion::{
    arrow::array::record_batch,
    datasource::MemTable,
    logical_expr::{ColumnarValue, LogicalPlan},
    prelude::{DataFrame, SessionContext},
    variable::{VarProvider, VarType},
};
use datafusion_common::{DFSchema, DataFusionError, ScalarValue};
use datafusion_sql::{TableReference, parser::Statement as DfStatement};
use sqlparser::{
    ast::{CreateFunction, Expr as SqlExpr, Ident, ObjectName, Query as SqlQuery, Statement as SqlStatement, VisitMut, VisitorMut},
    keywords::SCHEMA,
};

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

        // register custom ud(.?)fs
        session_ctx.register_udtf("read_csv", Arc::new(LocalCsvTableFunc {}));

        // register function factory
        let session_ctx = session_ctx.with_function_factory(
            Arc::new(CustomFunctionFactory::default())
        );

        let val_provider = Arc::new(EvaluationValProvider::new());
        session_ctx.register_variable(VarType::UserDefined, val_provider.clone());
        Self {
            session_ctx,
            exprs: Arc::new(Mutex::new(HashMap::new())),
            val_provider,
        }
    }

    /// Register values corresponding to variables in the context
    pub async fn register_values(
        &self,
        variables: &[Variable],
        values: &[TaskValue],
    ) -> Result<(), AvengerRuntimeError> {
        for (variable, value) in variables.iter().zip(values.iter()) {
            match value {
                TaskValue::Val { value: val } => self.register_val(&variable, val.clone())?,
                TaskValue::Expr {
                    sql_expr: expr,
                    context,
                } => {
                    self.register_task_value_context(&context).await?;
                    self.register_expr(&variable, expr.clone())?;
                }
                TaskValue::Dataset { dataset, context } => {
                    self.register_task_value_context(&context).await?;
                    self.register_dataset(&variable, dataset.clone()).await?;
                }
                TaskValue::Function { function, context } => {
                    self.register_task_value_context(&context).await?;
                    self.register_function(&variable, function.clone()).await?;
                }
                _ => {
                    // skip
                }
            };
        }
        Ok(())
    }

    pub async fn register_task_value_context(
        &self,
        context: &TaskValueContext,
    ) -> Result<(), AvengerRuntimeError> {
        for (name, value) in context.values().iter() {
            self.register_val(&name, value.clone())?;
        }
        for (name, dataset) in context.datasets().iter() {
            self.register_dataset(&name, dataset.clone()).await?;
        }
        for (name, function) in context.functions().iter() {
            self.register_function(&name, function.clone()).await?;
        }
        Ok(())
    }

    /// Get the underlying DataFusion SessionContext
    pub fn session_ctx(&self) -> &SessionContext {
        &self.session_ctx
    }

    /// Register a DataFrame in the context under a mangled name
    pub async fn register_dataset(
        &self,
        variable: &Variable,
        dataset: TaskDataset,
    ) -> Result<(), AvengerRuntimeError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerRuntimeError::InternalError(format!(
                "Dataset name should not include the @ prefix: {}",
                variable.name()
            )));
        }

        // Replace . with __ in the table name so that DataFusion doesn't interpret it as a schema
        let mangled_name = variable.mangled_var_name();
        match dataset {
            TaskDataset::LogicalPlan(plan) => {
                let df = self.session_ctx.execute_logical_plan(plan).await?;
                self.session_ctx
                    .register_table(mangled_name.clone(), df.into_view())?;
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

    /// Register a function in the context
    pub async fn register_function(&self, variable: &Variable, mut function: CreateFunction) -> Result<(), AvengerRuntimeError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerRuntimeError::InternalError(format!(
                "Function name should not include the @ prefix: {}",
                variable.name()
            )));
        }

        // Compute mangled name and update function
        let mangled_name = variable.mangled_var_name();
        function.name = ObjectName(vec![Ident::new(mangled_name)]);

        // Register function with this name
        let plan = self.session_ctx.state().statement_to_plan(
            DfStatement::Statement(Box::new(SqlStatement::CreateFunction(function)))
        ).await?;
        self.session_ctx.execute_logical_plan(plan).await?;
        Ok(())
    }

    /// Get a registered dataset from the context
    pub async fn get_dataset(&self, variable: &Variable) -> Result<DataFrame, AvengerRuntimeError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerRuntimeError::InternalError(format!(
                "Dataset name should not include the @ prefix: {}",
                variable.name()
            )));
        }
        let mangled_name = variable.mangled_var_name();
        let df = self.session_ctx.table(&mangled_name).await?;
        Ok(df)
    }

    /// Check if a dataset is registered in the context, handling mangling
    pub fn has_dataset(&self, variable: &Variable) -> bool {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            panic!(
                "Dataset name should not include the @ prefix: {}",
                variable.name()
            );
        }
        let mangled_name = variable.mangled_var_name();
        self.session_ctx.table_exist(&mangled_name).unwrap_or(false)
    }

    /// Register a value in the context
    pub fn register_val(
        &self,
        variable: &Variable,
        val: ScalarValue,
    ) -> Result<(), AvengerRuntimeError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerRuntimeError::InternalError(format!(
                "Val name should not include the @ prefix: {}",
                variable.name()
            )));
        }
        self.val_provider.insert(variable.clone(), val);
        Ok(())
    }

    /// Get a value from the context
    pub fn get_val(&self, variable: &Variable) -> Result<ScalarValue, AvengerRuntimeError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerRuntimeError::InternalError(format!(
                "Val name should not include the @ prefix: {}",
                variable.name()
            )));
        }
        let val = self.val_provider.get_scalar_value(variable)?;
        Ok(val)
    }

    /// Check if a value is registered in the context
    pub fn has_val(&self, variable: &Variable) -> bool {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            panic!(
                "Val name should not include the @ prefix: {}",
                variable.name()
            );
        }
        self.val_provider.has_variable(variable)
    }

    /// Add an expression to the context
    pub fn register_expr(
        &self,
        variable: &Variable,
        sql_expr: SqlExpr,
    ) -> Result<(), AvengerRuntimeError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            panic!(
                "Expr name should not include the @ prefix: {}",
                variable.name()
            );
        }
        self.exprs
            .lock()
            .unwrap()
            .insert(variable.clone(), sql_expr);
        Ok(())
    }

    /// Get an expression from the context
    pub fn get_expr(&self, variable: &Variable) -> Result<SqlExpr, AvengerRuntimeError> {
        if !variable.parts.is_empty() && variable.parts[0].starts_with("@") {
            return Err(AvengerRuntimeError::InternalError(format!(
                "Expr name should not include the @ prefix: {}",
                variable.name()
            )));
        }
        let locked = self.exprs.lock().unwrap();
        let expr = locked
            .get(variable)
            .ok_or(AvengerRuntimeError::ExpressionNotFound(format!(
                "Expression {} not found",
                variable.name()
            )))?;
        Ok(expr.clone())
    }

    /// Check if an expression is stored in the context
    pub fn has_expr(&self, variable: &Variable) -> bool {
        self.exprs.lock().unwrap().contains_key(variable)
    }

    /// Compile a SQL query to a logical plan, expanding sql with referenced expressions
    pub async fn compile_query(
        &self,
        query: &SqlQuery,
    ) -> Result<LogicalPlan, AvengerRuntimeError> {
        // Visit the query and validate references
        let mut expanded_query = self.expand_query(&query)?;
        let mut visitor = CompilationVisitor::new(&self);
        if let ControlFlow::Break(Result::Err(err)) =
            VisitMut::visit(&mut expanded_query, &mut visitor)
        {
            return Err(err);
        }
        let plan = self
            .session_ctx
            .state()
            .create_logical_plan(&expanded_query.to_string())
            .await?;
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

    pub fn expand_function(&self, function: &CreateFunction) -> Result<CreateFunction, AvengerRuntimeError> {
        let mut function = function.clone();
        let mut visitor = CompilationVisitor::new(&self);
        if let ControlFlow::Break(Result::Err(err)) = function.visit(&mut visitor) {
            return Err(err);
        }
        Ok(function)
    }
    
    pub async fn evaluate_expr(&self, expr: &SqlExpr) -> Result<ScalarValue, AvengerRuntimeError> {
        let mut sql_expr = self.expand_expr(expr)?;

        // update references with compilation visitor
        let mut visitor = CompilationVisitor::new(&self);
        if let ControlFlow::Break(Result::Err(err)) = VisitMut::visit(&mut sql_expr, &mut visitor) {
            return Err(err);
        }

        let plan = self
            .session_ctx
            .state()
            .create_logical_plan(&format!("SELECT {} as val", sql_expr.to_string()))
            .await?;

        let df = self.session_ctx.execute_logical_plan(plan).await?;
        let schema = df.schema().inner().clone();
        let partitions = df.collect().await?;
        let table = ArrowTable::try_new(schema, partitions)?;

        let col = table.column("val")?;
        let v = ScalarValue::try_from_array(&col, 0)?;
        Ok(v)

        // println!("expanded expr: {}", sql_expr);
        // let expr = self
        //     .session_ctx
        //     .parse_sql_expr(&sql_expr.to_string(), &DFSchema::empty())?;

        // println!("parsed expr: {:?}", expr);

        // let val = self
        //     .session_ctx
        //     .create_physical_expr(expr, &DFSchema::empty())?;
        // let col_val = val.evaluate(&record_batch!(("_dummy", Int32, [1])).unwrap())?;
        // match col_val {
        //     ColumnarValue::Scalar(scalar_value) => Ok(scalar_value),
        //     ColumnarValue::Array(array) => {
        //         if array.len() != 1 {
        //             return Err(AvengerRuntimeError::InternalError(
        //                 "Array value not expected".to_string(),
        //             ));
        //         }
        //         // Handle single element array
        //         let val = ScalarValue::try_from_array(array.as_ref(), 0)?;
        //         Ok(val)
        //     }
        // }
    }
}

#[derive(Debug, Clone)]
struct EvaluationValProvider {
    vals: Arc<Mutex<HashMap<Variable, ScalarValue>>>,
}

impl EvaluationValProvider {
    pub fn new() -> Self {
        Self {
            vals: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn insert(&self, variable: Variable, val: ScalarValue) {
        self.vals.lock().unwrap().insert(variable, val);
    }

    pub fn get_scalar_value(&self, variable: &Variable) -> datafusion_common::Result<ScalarValue> {
        let val =
            self.vals
                .lock()
                .unwrap()
                .get(variable)
                .cloned()
                .ok_or(DataFusionError::Internal(format!(
                    "Variable {} not found",
                    variable.name()
                )))?;
        Ok(val)
    }

    pub fn has_variable(&self, variable: &Variable) -> bool {
        self.vals.lock().unwrap().contains_key(variable)
    }

    pub fn unmangle_name(names: &[String]) -> Variable {
        let mut mangled_name = names[0].clone();
        if mangled_name.starts_with('@') {
            mangled_name = mangled_name[1..].to_string();
        }

        let parts = mangled_name
            .split("__")
            .into_iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        Variable::new(parts)
    }
}

impl VarProvider for EvaluationValProvider {
    fn get_value(&self, var_names: Vec<String>) -> datafusion_common::Result<ScalarValue> {
        let variable = Self::unmangle_name(&var_names);
        let val =
            self.vals
                .lock()
                .unwrap()
                .get(&variable)
                .cloned()
                .ok_or(DataFusionError::Internal(format!(
                    "Variable {} not found",
                    variable.name()
                )))?;
        Ok(val)
    }

    fn get_type(&self, var_names: &[String]) -> Option<arrow_schema::DataType> {
        let variable = Self::unmangle_name(var_names);
        let locked = self.vals.lock().unwrap();
        let val = locked.get(&variable)?;
        Some(val.data_type())
    }
}

pub struct CompilationVisitor<'a> {
    ctx: &'a TaskEvaluationContext,
}

impl<'a> CompilationVisitor<'a> {
    pub fn new(ctx: &'a TaskEvaluationContext) -> Self {
        Self { ctx }
    }
}

impl<'a> VisitorMut for CompilationVisitor<'a> {
    type Break = Result<(), AvengerRuntimeError>;

    fn pre_visit_relation(
        &mut self,
        relation: &mut datafusion_sql::sqlparser::ast::ObjectName,
    ) -> ControlFlow<Self::Break> {
        let table_name = relation.to_string();

        if table_name.starts_with("@") {
            let parts = relation
                .0
                .iter()
                .map(|ident| ident.value.clone())
                .collect::<Vec<_>>();

            // Join on __ into a single string
            let idents = vec![Ident::new(parts.join("__"))];
            *relation = ObjectName(idents);
        }

        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &mut SqlExpr) -> ControlFlow<Self::Break> {
        match expr.clone() {
            SqlExpr::Function(mut func) => {
                if func.name.0[0].value.starts_with("@") {
                    // Update function with mangled name
                    let parts = func.name
                        .0
                        .iter()
                        .map(|ident| ident.value.clone())
                        .collect::<Vec<_>>();

                    let idents = vec![Ident::new(parts.join("__"))];
                    func.name = ObjectName(idents);
                    *expr = SqlExpr::Function(func);
                    return ControlFlow::Continue(());
                }
            }
            SqlExpr::Identifier(ident) => {
                if ident.value.starts_with("@") {
                    let variable = Variable::from_mangled_name(&ident.value);

                    // Check if this is a reference to an expression
                    if let Ok(registered_expr) = self.ctx.get_expr(&variable) {
                        *expr = SqlExpr::Nested(Box::new(registered_expr.clone()));
                        return ControlFlow::Continue(());
                    }

                    // Otherwise it must be a reference to a value
                    if !self.ctx.has_val(&variable) {
                        return ControlFlow::Break(Err(AvengerRuntimeError::ExpressionNotFound(
                            format!("Val or Expr {} not found", variable.name()),
                        )));
                    }
                }
            }
            SqlExpr::CompoundIdentifier(idents) => {
                if !idents.is_empty() && idents[0].value.starts_with("@") {
                    let mangled_name = idents
                        .iter()
                        .map(|s| s.value.clone())
                        .collect::<Vec<_>>()
                        .join("__");
                    let variable = Variable::from_mangled_name(&mangled_name);

                    // Check if this is a reference to an expression
                    if let Ok(registered_expr) = self.ctx.get_expr(&variable) {
                        *expr = SqlExpr::Nested(Box::new(registered_expr.clone()));
                        return ControlFlow::Continue(());
                    }

                    // Otherwise it must be a reference to a value
                    if !self.ctx.has_val(&variable) {
                        return ControlFlow::Break(Err(AvengerRuntimeError::ExpressionNotFound(
                            format!("Val or Expr {} not found", variable.name()),
                        )));
                    }

                    // Update with mangled name, joining on __ into a single string
                    *expr = SqlExpr::Identifier(Ident::new(variable.mangled_var_name()));
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}
