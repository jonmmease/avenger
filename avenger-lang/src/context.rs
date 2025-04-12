use std::{collections::HashMap, sync::{Arc, Mutex}};

use datafusion::{prelude::{DataFrame, Expr, SessionContext}, variable::{VarProvider, VarType}};
use datafusion_common::{DataFusionError, ScalarValue};

use crate::error::AvengerLangError;


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

    /// Get the underlying DataFusion SessionContext
    pub fn session_ctx(&self) -> &SessionContext {
        &self.session_ctx
    }

    /// Register a DataFrame in the context under a mangled name
    /// 
    /// Maybe add an evaluation option in the future to control whether it's stored as a view
    /// or evaluated and registered as in-memory table
    pub fn register_dataset(&self, name: &str, df: DataFrame) -> Result<(), AvengerLangError> {
        if !name.starts_with("@") {
            return Err(AvengerLangError::InternalError(format!("Dataset name should start with @ prefix: {}", name)));
        }
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
        if !name.starts_with("@") {
            return Err(AvengerLangError::InternalError(format!("Val name should start with @ prefix: {}", name)));
        }
        self.val_provider.insert(name.to_string(), val);
        Ok(())
    }

    /// Get a value from the context
    pub fn get_val(&self, name: &str) -> Result<ScalarValue, AvengerLangError> {
        let val = self.val_provider.get_value(vec![name.to_string()])?;
        Ok(val)
    }

    /// Check if a value is registered in the context
    pub fn has_val(&self, name: &str) -> bool {
        self.val_provider.get_type(&[name.to_string()]).is_some()
    }

    /// Add an expression to the context
    pub fn register_expr(&self, name: String, expr: Expr) -> Result<(), AvengerLangError> {
        if !name.starts_with("@") {
            return Err(AvengerLangError::InternalError(format!("Expr name should start with @ prefix: {}", name)));
        }
        self.exprs.lock().unwrap().insert(name, expr);
        Ok(())
    }

    /// Get an expression from the context
    pub fn get_expr(&self, name: &str) -> Result<Expr, AvengerLangError> {
        let locked = self.exprs.lock().unwrap();
        let expr = locked.get(name).ok_or(
            AvengerLangError::ExpressionNotFound(format!("Expression {} not found", name))
        )?;
        Ok(expr.clone())
    }

    /// Check if an expression is stored in the context
    pub fn has_expr(&self, name: &str) -> bool {
        self.exprs.lock().unwrap().contains_key(name)
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
