use std::{fmt::Debug, hash::{DefaultHasher, Hash, Hasher}, ops::ControlFlow};
use std::sync::Arc;

use sqlparser::ast::{Expr as SqlExpr, ObjectName, Query as SqlQuery, Visit, VisitMut, Visitor as SqlVisitor};
use async_trait::async_trait;
use avenger_scales::scales::coerce::Coercer;
use avenger_scales::utils::ScalarValueUtils;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::{SceneMark, SceneMarkType};
use crate::{ast::{DatasetPropDecl, ExprPropDecl, ValPropDecl}, context::EvaluationContext, error::AvengerLangError, task_graph::{dependency::{Dependency, DependencyKind}, value::{TaskDataset, TaskValue}}};
use crate::marks::{
    build_rect_mark, build_arc_mark, build_area_mark, build_image_mark, 
    build_line_mark, build_path_mark, build_rule_mark, build_symbol_mark,
    build_text_mark, build_trail_mark
};
use super::{value::TaskValueContext, variable::Variable};


#[async_trait]
pub trait Task: Debug + Send + Sync {
    /// Get the dependencies of the task
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerLangError> {
        Ok(vec![])
    }

    /// Get the input variables of the task
    fn input_variables(&self) -> Result<Vec<Variable>, AvengerLangError> {
        Ok(self.input_dependencies()?.iter().map(
            |dep| dep.variable.clone()
        ).collect())
    }

    fn fingerprint(&self) -> Result<u64, AvengerLangError>;

    /// Evaluate the task in a session context with the given dependencies
    async fn evaluate(
        &self,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerLangError>;
}


/// Task storing a scalar value
#[derive(Debug, Clone, PartialEq, Hash)]
pub struct TaskValueTask {
    pub value: TaskValue,
}

impl TaskValueTask {
    pub fn new(value: TaskValue) -> Self {
        Self { value }
    }
}

#[async_trait]
impl Task for TaskValueTask {
    async fn evaluate(
        &self,
        _input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerLangError> {
        Ok(self.value.clone())
    }

    fn fingerprint(&self) -> Result<u64, AvengerLangError> {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        Ok(hasher.finish())
    }
}

/// A task that evaluates to a scalarvalue
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValDeclTask {
    pub value: SqlExpr,
}

impl ValDeclTask {
    pub fn new(value: SqlExpr) -> Self {
        Self { value }
    }
}

#[async_trait]
impl Task for ValDeclTask {    
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerLangError> {
        let mut visitor = CollectDependenciesVisitor::new();
        if let ControlFlow::Break(Result::Err(err)) = self.value.visit(&mut visitor) {
            return Err(err);
        }
        Ok(visitor.deps)
    }

    async fn evaluate(
        &self,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerLangError> {
        let ctx = EvaluationContext::new();
        ctx.register_values(&self.input_variables()?, &input_values).await?;
        let val = ctx.evaluate_expr(&self.value).await?;
        Ok(TaskValue::Val { value: val })
    }

    fn fingerprint(&self) -> Result<u64, AvengerLangError> {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        Ok(hasher.finish())
    }
}

impl From<ValPropDecl> for ValDeclTask {
    fn from(val_prop_decl: ValPropDecl) -> Self {
        Self { value: val_prop_decl.value }
    }
}

/// A task that evaluates to an expression
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExprDeclTask {
    pub expr: SqlExpr,
}

impl ExprDeclTask {
    pub fn new(expr: SqlExpr) -> Self {
        Self { expr }
    }
}

#[async_trait]
impl Task for ExprDeclTask {
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerLangError> {
        let mut visitor = CollectDependenciesVisitor::new();
        if let ControlFlow::Break(Result::Err(err)) = self.expr.visit(&mut visitor) {
            return Err(err);
        }
        Ok(visitor.deps)
    }

    async fn evaluate(
        &self,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerLangError> {
        let ctx = EvaluationContext::new();
        ctx.register_values(&self.input_variables()?, &input_values).await?;

        let sql_expr = ctx.expand_expr(&self.expr)?;
        let task_value_context = TaskValueContext::from_combined_task_value_context(
            &self.input_variables()?, &input_values
        )?;
        Ok(TaskValue::Expr { context: task_value_context, sql_expr })
    }

    fn fingerprint(&self) -> Result<u64, AvengerLangError> {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        Ok(hasher.finish())
    }
}

impl From<ExprPropDecl> for ExprDeclTask {
    fn from(expr_prop_decl: ExprPropDecl) -> Self {
        Self { expr: expr_prop_decl.value }
    }
}

/// A task that evaluates to a dataset
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DatasetDeclTask {
    pub query: Box<SqlQuery>,
    pub eval: bool,
}

impl DatasetDeclTask {
    pub fn new(query: SqlQuery, eval: bool) -> Self {
        Self { query: Box::new(query), eval }
    }
}

#[async_trait]
impl Task for DatasetDeclTask {
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerLangError> {
        let mut visitor = CollectDependenciesVisitor::new();
        if let ControlFlow::Break(Result::Err(err)) = self.query.visit(&mut visitor) {
            return Err(err);
        }
        Ok(visitor.deps)
    }

    async fn evaluate(
        &self,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerLangError> {
        let ctx = EvaluationContext::new();
        ctx.register_values(&self.input_variables()?, &input_values).await?;
        let plan = ctx.compile_query(&self.query).await?;

        if self.eval {
            // Eager evaluation, evaluate the logical plan
            let table = ctx.eval_query(&self.query).await?;
            Ok(TaskValue::Dataset { context: Default::default() , dataset: TaskDataset::ArrowTable(table) })
        } else {
            // Lazy evaluation, return the logical plan, along with the reference value context
            let task_value_context = TaskValueContext::from_combined_task_value_context(
                &self.input_variables()?, &input_values
            )?;
            Ok(TaskValue::Dataset { context: task_value_context, dataset: TaskDataset::LogicalPlan(plan) })
        }
    }

    fn fingerprint(&self) -> Result<u64, AvengerLangError> {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        Ok(hasher.finish())
    }
}

impl From<DatasetPropDecl> for DatasetDeclTask {
    fn from(dataset_prop_decl: DatasetPropDecl) -> Self {
        Self { query: dataset_prop_decl.value, eval: true }
    }
}

// Generic Mark Task to handle all mark types
#[derive(Debug, Clone, PartialEq, Hash)]
pub struct MarkTask {
    encoded_data: Variable,
    config_data: Variable,
    mark_type: SceneMarkType,
}

impl MarkTask {
    pub fn new(encoded_data: Variable, config_data: Variable, mark_type: SceneMarkType) -> Self {
        Self {
            encoded_data,
            config_data,
            mark_type,
        }
    }
}

#[async_trait]
impl Task for MarkTask {
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerLangError> {
        Ok(vec![
            Dependency { variable: self.encoded_data.clone(), kind: DependencyKind::Dataset },
            Dependency { variable: self.config_data.clone(), kind: DependencyKind::Dataset }
        ])
    }

    async fn evaluate(
        &self,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerLangError> {
        let TaskValue::Dataset { dataset: TaskDataset::ArrowTable(encoded_table), .. } = &input_values[0] else {
            return Err(AvengerLangError::InternalError(
                "Expected a dataset with arrow table for encoded_data input".to_string(),
            ));
        };
        let TaskValue::Dataset { dataset: TaskDataset::ArrowTable(config_table), .. } = &input_values[1] else {
            return Err(AvengerLangError::InternalError(
                "Expected a dataset with arrow table for config_data input".to_string(),
            ));
        };
        
        let mark = match &self.mark_type {
            SceneMarkType::Rect => SceneMark::Rect(build_rect_mark(encoded_table, config_table)?),
            SceneMarkType::Arc => SceneMark::Arc(build_arc_mark(encoded_table, config_table)?),
            SceneMarkType::Area => SceneMark::Area(build_area_mark(encoded_table, config_table)?),
            SceneMarkType::Image => SceneMark::Image(Arc::new(build_image_mark(encoded_table, config_table)?)),
            SceneMarkType::Line => SceneMark::Line(build_line_mark(encoded_table, config_table)?),
            SceneMarkType::Path => SceneMark::Path(build_path_mark(encoded_table, config_table)?),
            SceneMarkType::Rule => SceneMark::Rule(build_rule_mark(encoded_table, config_table)?),
            SceneMarkType::Symbol => SceneMark::Symbol(build_symbol_mark(encoded_table, config_table)?),
            SceneMarkType::Text => SceneMark::Text(Arc::new(build_text_mark(encoded_table, config_table)?)),
            SceneMarkType::Trail => SceneMark::Trail(build_trail_mark(encoded_table, config_table)?),
            SceneMarkType::Group => {
                // Group marks require a different approach, so this is a placeholder that returns an error
                return Err(AvengerLangError::InternalError(
                    "Group marks should use GroupMarkTask instead of MarkTask".to_string(),
                ));
            }
        };
        
        Ok(TaskValue::Mark {mark})
    }

    fn fingerprint(&self) -> Result<u64, AvengerLangError> {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        Ok(hasher.finish())
    }
}

// For backward compatibility, keep RectMarkTask as a thin wrapper around MarkTask
#[derive(Debug, Clone, PartialEq, Hash)]
pub struct RectMarkTask {
    encoded_data: Variable,
    config_data: Variable,
}

impl RectMarkTask {
    pub fn new(encoded_data: Variable, config_data: Variable) -> Self {
        Self {
            encoded_data,
            config_data,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Hash)]
pub struct GroupMarkTask {
    config: Variable,
    marks: Vec<Variable>,
}

impl GroupMarkTask {
    pub fn new(config: Variable, marks: Vec<Variable>) -> Self {
        Self { config, marks }
    }
}

#[async_trait]
impl Task for GroupMarkTask {
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerLangError> {
        let mut deps = vec![Dependency { variable: self.config.clone(), kind: DependencyKind::Dataset }];
        for mark in &self.marks {
            deps.push(Dependency { variable: mark.clone(), kind: DependencyKind::Mark });
        }
        Ok(deps)
    }

    async fn evaluate(
        &self,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerLangError> {

        // Extract config table
        let TaskValue::Dataset { dataset: TaskDataset::ArrowTable(config_table), .. } = &input_values[0] else {
            return Err(AvengerLangError::InternalError(
                format!("Expected a dataset with arrow table for config input. Got {:?}", input_values[0] ),
            ));
        };

        // Extract marks
        let mut marks = vec![];
        for value in input_values {
            if let TaskValue::Mark { mark, .. } = value {
                marks.push(mark.clone());
            }
        }

        // Get scalar config values
        let coercer = Coercer::default();
        let zindex = if let Ok(z) = config_table.column("zindex") {
            let zindex = coercer.to_numeric(&z, None)?;
            Some(*zindex.first().unwrap() as i32)
        } else {
            None
        };
        let x = if let Ok(x) = config_table.column("x") {
            *coercer.to_numeric(&x, None)?.first().unwrap()
        } else {
            0.0
        };
        let y = if let Ok(y) = config_table.column("y") {
            *coercer.to_numeric(&y, None)?.first().unwrap()
        } else {
            0.0
        };
        let fill = if let Ok(fill) = config_table.column("fill") {
            Some(coercer.to_color(&fill, None)?.first().unwrap().clone())
        } else {
            None
        };
        let stroke = if let Ok(stroke) = config_table.column("stroke") {
            Some(coercer.to_color(&stroke, None)?.first().unwrap().clone())
        } else {
            None
        };
        let stroke_width = if let Ok(stroke_width) = config_table.column("stroke_width") {
            Some(*coercer.to_numeric(&stroke_width, None)?.first().unwrap())
        } else {
            None
        };
        let stroke_offset = if let Ok(stroke_offset) = config_table.column("stroke_offset") {
            Some(*coercer.to_numeric(&stroke_offset, None)?.first().unwrap())
        } else {
            None
        };

        let group_mark = SceneGroup {
            name: "".to_string(),
            origin: [x, y],
            clip: Default::default(),
            marks,
            gradients: vec![],
            fill,
            stroke,
            stroke_width,
            stroke_offset,
            zindex,
        };

        Ok(TaskValue::Mark { mark: SceneMark::Group(group_mark) })
    }

    fn fingerprint(&self) -> Result<u64, AvengerLangError> {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        Ok(hasher.finish())
    }
}

pub struct CollectDependenciesVisitor {
    /// The variables that are dependencies of the task, without leading @
    deps: Vec<Dependency>,
}

impl CollectDependenciesVisitor {
    pub fn new() -> Self {
        Self { deps: vec![] }
    }
}

impl SqlVisitor for CollectDependenciesVisitor {
    type Break = Result<(), AvengerLangError>;

    /// Replace tables of the form @table_name with the true mangled table name
    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<Self::Break> {
        let table_name = relation.to_string();

        // Handle dataset references
        if table_name.starts_with("@") {            // Drop leading @ and split on __
            let parts = table_name[1..].split(".").map(|s| s.to_string()).collect::<Vec<_>>();

            self.deps.push(Dependency::with_parts(
                parts, DependencyKind::Dataset)
            );
        }

        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &SqlExpr) -> ControlFlow<Self::Break> {
        match &expr {
            SqlExpr::Identifier(ident) => {
                if ident.value.starts_with("@") {
                    self.deps.push(Dependency::new(
                        ident.value[1..].to_string(), DependencyKind::ValOrExpr)
                    );
                }
            }
            SqlExpr::CompoundIdentifier(idents) => {
                if !idents.is_empty() && idents[0].value.starts_with("@") {
                    let mut parts: Vec<String> = idents.iter().map(|ident| ident.value.clone()).collect();
                    // Drop the leading @
                    parts[0] = parts[0][1..].to_string();
                    self.deps.push(Dependency::with_parts(
                        parts, DependencyKind::ValOrExpr)
                    );
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}
