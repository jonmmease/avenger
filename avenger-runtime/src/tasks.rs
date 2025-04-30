use async_trait::async_trait;
use avenger_lang2::ast::{DatasetProp, ExprProp, ValProp};
use avenger_scales::scales::coerce::Coercer;
use avenger_scenegraph::marks::{group::SceneGroup, mark::{SceneMark, SceneMarkType}};
use std::{fmt::Debug, hash::{DefaultHasher, Hash, Hasher}, ops::ControlFlow, sync::Arc};

use crate::{context::TaskEvaluationContext, dependency::{Dependency, DependencyKind}, error::AvengerRuntimeError, marks::{build_arc_mark, build_area_mark, build_image_mark, build_line_mark, build_path_mark, build_rect_mark, build_rule_mark, build_symbol_mark, build_text_mark, build_trail_mark}, runtime::TaskGraphRuntime, value::{TaskDataset, TaskValue, TaskValueContext}, variable::Variable, visitors::{collect_expr_dependencies, collect_query_dependencies}};

use sqlparser::ast::{Expr as SqlExpr, Query as SqlQuery, Visit};


#[async_trait]
pub trait Task: Debug + Send + Sync {
    /// Get the dependencies of the task
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerRuntimeError> {
        Ok(vec![])
    }

    /// Get the input variables of the task
    fn input_variables(&self) -> Result<Vec<Variable>, AvengerRuntimeError> {
        Ok(self.input_dependencies()?.iter().map(
            |dep| dep.variable.clone()
        ).collect())
    }

    fn fingerprint(&self) -> Result<u64, AvengerRuntimeError>;

    /// Evaluate the task in a session context with the given dependencies
    async fn evaluate(
        &self,
        runtime: Arc<TaskGraphRuntime>,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerRuntimeError>;
}


/// Task storing an inline value
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
        _runtime: Arc<TaskGraphRuntime>,
        _input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerRuntimeError> {
        Ok(self.value.clone())
    }

    fn fingerprint(&self) -> Result<u64, AvengerRuntimeError> {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        Ok(hasher.finish())
    }
}


/// A task that evaluates to a scalarvalue
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValPropTask {
    pub value: SqlExpr,
}

impl ValPropTask {
    pub fn new(value: SqlExpr) -> Self {
        Self { value }
    }
}

#[async_trait]
impl Task for ValPropTask {    
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerRuntimeError> {
        collect_expr_dependencies(&self.value)
    }

    async fn evaluate(
        &self,
        _runtime: Arc<TaskGraphRuntime>,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerRuntimeError> {
        let ctx = TaskEvaluationContext::new();
        ctx.register_values(&self.input_variables()?, &input_values).await?;
        let val = ctx.evaluate_expr(&self.value).await?;
        Ok(TaskValue::Val { value: val })
    }

    fn fingerprint(&self) -> Result<u64, AvengerRuntimeError> {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        Ok(hasher.finish())
    }
}

impl From<ValProp> for ValPropTask {
    fn from(val_prop: ValProp) -> Self {
        Self { value: val_prop.expr }
    }
}

/// A task that evaluates to an expression
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExprPropTask {
    pub expr: SqlExpr,
}

impl ExprPropTask {
    pub fn new(expr: SqlExpr) -> Self {
        Self { expr }
    }
}

#[async_trait]
impl Task for ExprPropTask {
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerRuntimeError> {
        collect_expr_dependencies(&self.expr)
    }

    async fn evaluate(
        &self,
        _runtime: Arc<TaskGraphRuntime>,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerRuntimeError> {
        let ctx = TaskEvaluationContext::new();
        ctx.register_values(&self.input_variables()?, &input_values).await?;

        let sql_expr = ctx.expand_expr(&self.expr)?;
        let task_value_context = TaskValueContext::from_vars_and_vals(
            &self.input_variables()?, &input_values
        )?;
        Ok(TaskValue::Expr { context: task_value_context, sql_expr })
    }

    fn fingerprint(&self) -> Result<u64, AvengerRuntimeError> {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        Ok(hasher.finish())
    }
}

impl From<ExprProp> for ExprPropTask {
    fn from(expr_prop: ExprProp) -> Self {
        Self { expr: expr_prop.expr }
    }
}

/// A task that evaluates to a dataset
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DatasetPropTask {
    pub query: Box<SqlQuery>,
    pub eval: bool,
}

impl DatasetPropTask {
    pub fn new(query: Box<SqlQuery>, eval: bool) -> Self {
        Self { query, eval }
    }
}

#[async_trait]
impl Task for DatasetPropTask {
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerRuntimeError> {
        collect_query_dependencies(&self.query)
    }

    async fn evaluate(
        &self,
        _runtime: Arc<TaskGraphRuntime>,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerRuntimeError> {
        let ctx = TaskEvaluationContext::new();
        ctx.register_values(&self.input_variables()?, &input_values).await?;
        let plan = ctx.compile_query(&self.query).await?;

        if self.eval {
            // Eager evaluation, evaluate the logical plan
            let table = ctx.eval_query(&self.query).await?;
            Ok(TaskValue::Dataset { context: Default::default() , dataset: TaskDataset::ArrowTable(table) })
        } else {
            // Lazy evaluation, return the logical plan, along with the reference value context
            let task_value_context = TaskValueContext::from_vars_and_vals(
                &self.input_variables()?, &input_values
            )?;
            Ok(TaskValue::Dataset { context: task_value_context, dataset: TaskDataset::LogicalPlan(plan) })
        }
    }

    fn fingerprint(&self) -> Result<u64, AvengerRuntimeError> {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        Ok(hasher.finish())
    }
}

impl From<DatasetProp> for DatasetPropTask {
    fn from(dataset_prop: DatasetProp) -> Self {
        Self { query: dataset_prop.query, eval: true }
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
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerRuntimeError> {
        Ok(vec![
            Dependency { variable: self.encoded_data.clone(), kind: DependencyKind::Dataset },
            Dependency { variable: self.config_data.clone(), kind: DependencyKind::Dataset }
        ])
    }

    async fn evaluate(
        &self,
        _runtime: Arc<TaskGraphRuntime>,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerRuntimeError> {
        let TaskValue::Dataset { dataset: TaskDataset::ArrowTable(encoded_table), .. } = &input_values[0] else {
            return Err(AvengerRuntimeError::InternalError(
                "Expected a dataset with arrow table for encoded_data input".to_string(),
            ));
        };
        let TaskValue::Dataset { dataset: TaskDataset::ArrowTable(config_table), .. } = &input_values[1] else {
            return Err(AvengerRuntimeError::InternalError(
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
                return Err(AvengerRuntimeError::InternalError(
                    "Group marks should use GroupMarkTask instead of MarkTask".to_string(),
                ));
            }
        };
        
        Ok(TaskValue::Mark {mark})
    }

    fn fingerprint(&self) -> Result<u64, AvengerRuntimeError> {
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
    fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerRuntimeError> {
        let mut deps = vec![Dependency { variable: self.config.clone(), kind: DependencyKind::Dataset }];
        for mark in &self.marks {
            deps.push(Dependency { variable: mark.clone(), kind: DependencyKind::Mark });
        }
        Ok(deps)
    }

    async fn evaluate(
        &self,
        _runtime: Arc<TaskGraphRuntime>,
        input_values: &[TaskValue],
    ) -> Result<TaskValue, AvengerRuntimeError> {

        // Extract config table
        let TaskValue::Dataset { dataset: TaskDataset::ArrowTable(config_table), .. } = &input_values[0] else {
            return Err(AvengerRuntimeError::InternalError(
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

        println!("group marks: {:#?}", self.marks);

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

    fn fingerprint(&self) -> Result<u64, AvengerRuntimeError> {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        Ok(hasher.finish())
    }
}
