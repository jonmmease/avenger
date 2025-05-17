use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use arrow_schema::DataType;
use datafusion::variable::VarProvider;
use datafusion_common::{DataFusionError, ScalarValue};
use sqlparser::ast::{Expr as SqlExpr};
use crate::error::AvengerLangError;
use crate::table::ArrowTable;

#[derive(Debug, Clone)]
pub struct Environment {
    parent: Option<Arc<Environment>>,
    vals: Arc<Mutex<HashMap<Vec<String>, ScalarValue>>>,
    exprs: Arc<Mutex<HashMap<Vec<String>, SqlExpr>>>,
    tables: Arc<Mutex<HashMap<Vec<String>, ArrowTable>>>,
}

impl Environment {
    pub fn new() -> Self {
        Self {
            parent: None,
            vals: Arc::new(Mutex::new(HashMap::new())),
            exprs: Arc::new(Mutex::new(HashMap::new())),
            tables: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Push a new child environment
    pub fn push(&self) -> Self {
        Self {
            parent: Some(Arc::new(self.clone())),
            vals: Arc::new(Mutex::new(HashMap::new())),
            exprs: Arc::new(Mutex::new(HashMap::new())),
            tables: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn pop(&self) -> Option<Arc<Self>> {
        self.parent.clone()
    }

    pub fn insert_val(&self, variable: Vec<String>, val: ScalarValue) {
        self.vals.lock().unwrap().insert(variable, val);
    }

    pub fn insert_expr(&self, variable: Vec<String>, expr: SqlExpr) {
        self.exprs.lock().unwrap().insert(variable, expr);
    }

    pub fn insert_table(&self, variable: Vec<String>, table: ArrowTable) {
        self.tables.lock().unwrap().insert(variable, table);
    }

    pub fn has_val(&self, variable: &[String]) -> bool {
        self.vals.lock().unwrap().contains_key(variable) 
            || self.parent
                .as_ref()
                .map(|p| p.has_val(variable))
                .unwrap_or(false)
    }

    pub fn has_expr(&self, variable: &[String]) -> bool {
        self.exprs.lock().unwrap().contains_key(variable)
            || self
                .parent
                .as_ref()
                .map(|p| p.has_expr(variable))
                .unwrap_or(false)
    }

    pub fn has_table(&self, variable: &[String]) -> bool {
        self.tables.lock().unwrap().contains_key(variable) 
            || self
                .parent
                .as_ref()
                .map(|p| p.has_table(variable))
                .unwrap_or(false)
    }

    pub fn unmangle_name(names: &[String]) -> Vec<String> {
        let mut mangled_name = names[0].clone();
        if mangled_name.starts_with('@') {
            mangled_name = mangled_name[1..].to_string();
        }

        let parts = mangled_name
            .split("__")
            .into_iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        parts
    }

    pub fn get_expr(&self, variable: &[String]) -> datafusion_common::Result<SqlExpr> {
        self.exprs
            .lock()
            .unwrap()
            .get(variable)
            .cloned()
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_expr(variable).ok()))
            .ok_or(DataFusionError::Internal(format!(
                "Variable {:?} not found",
                variable
            )))
    }

    pub fn get_table(&self, variable: &[String]) -> datafusion_common::Result<ArrowTable> {
        self.tables
            .lock()
            .unwrap()
            .get(variable)
            .cloned()
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_table(variable).ok()))
            .ok_or(DataFusionError::Internal(format!(
                "Variable {:?} not found",
                variable
            )))
    }

    pub fn assign_val(&self, variable: Vec<String>, val: ScalarValue) -> Result<(), AvengerLangError> {
        // Check if the variable exists in the current environment
        if self.vals.lock().unwrap().contains_key(&variable) {
            // Update it in the current environment
            self.vals.lock().unwrap().insert(variable, val);
            Ok(())
        } else if let Some(parent) = &self.parent {
            // Delegate to parent if not in current environment
            parent.assign_val(variable, val)
        } else {
            // Error if no parent and not in current environment
            Err(AvengerLangError::InternalError(format!(
                "Cannot assign to non-existent variable {:?}",
                variable
            )))
        }
    }

    pub fn assign_expr(&self, variable: Vec<String>, expr: SqlExpr) -> Result<(), AvengerLangError> {
        // Check if the variable exists in the current environment
        if self.exprs.lock().unwrap().contains_key(&variable) {
            // Update it in the current environment
            self.exprs.lock().unwrap().insert(variable, expr);
            Ok(())
        } else if let Some(parent) = &self.parent {
            // Delegate to parent if not in current environment
            parent.assign_expr(variable, expr)
        } else {
            // Error if no parent and not in current environment
            Err(AvengerLangError::InternalError(format!(
                "Cannot assign to non-existent expression {:?}",
                variable
            )))
        }
    }

    pub fn assign_table(&self, variable: Vec<String>, table: ArrowTable) -> Result<(), AvengerLangError> {
        // Check if the variable exists in the current environment
        if self.tables.lock().unwrap().contains_key(&variable) {
            // Update it in the current environment
            self.tables.lock().unwrap().insert(variable, table);
            Ok(())
        } else if let Some(parent) = &self.parent {
            // Delegate to parent if not in current environment
            parent.assign_table(variable, table)
        } else {
            // Error if no parent and not in current environment
            Err(AvengerLangError::InternalError(format!(
                "Cannot assign to non-existent table {:?}",
                variable
            )))
        }
    }
}

impl VarProvider for Environment {
    fn get_value(&self, var_names: Vec<String>) -> datafusion_common::Result<ScalarValue> {
        let variable = Self::unmangle_name(&var_names);
        self.vals
            .lock()
            .unwrap()
            .get(&variable)
            .cloned()
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_value(var_names.clone()).ok()))
            .ok_or(DataFusionError::Internal(format!(
                "Variable {} not found",
                variable.join(".")
            )))
    }

    fn get_type(&self, var_names: &[String]) -> Option<DataType> {
        let variable = Self::unmangle_name(var_names);
        let locked = self.vals.lock().unwrap();
        locked.get(&variable)
            .map(|val| val.data_type())
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_type(var_names)))
    }
}