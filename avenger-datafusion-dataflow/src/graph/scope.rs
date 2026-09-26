use std::{collections::HashSet, sync::Arc};

use datafusion::{
    arrow::datatypes::{DataType, SchemaRef},
    logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder},
};

use super::{
    analysis, check_name, normalize,
    reference::{GraphRead, TableRef},
    DataflowBuilder, ExprInput, GraphDef, NodeDef, NodeKind, PlanNode, ScalarInput, ScalarNode,
    ScalarOutput, TableInput, TableOutput,
};
use crate::{partition::ScopeIdentity, Error, Result, ScopeHandle};

#[derive(Clone, Debug)]
pub(crate) struct ScopeDef {
    pub parent: Option<usize>,
    pub handle: Option<ScopeHandle>,
    pub discovery: Option<usize>,
    pub rows: Option<PlanNode>,
    pub input_names: HashSet<String>,
    pub node_names: HashSet<String>,
    pub output_names: HashSet<String>,
    pub child_names: HashSet<String>,
}

impl ScopeDef {
    pub fn root() -> Self {
        Self {
            parent: None,
            handle: None,
            discovery: None,
            rows: None,
            input_names: HashSet::new(),
            node_names: HashSet::new(),
            output_names: HashSet::new(),
            child_names: HashSet::new(),
        }
    }
}

impl GraphDef {
    pub fn is_ancestor(&self, ancestor: usize, mut scope: usize) -> bool {
        loop {
            if scope == ancestor {
                return true;
            }
            match self.scopes[scope].parent {
                Some(parent) => scope = parent,
                None => return false,
            }
        }
    }

    pub fn check_visible(&self, owner: usize, from: usize, label: &str) -> Result<()> {
        if self.is_ancestor(owner, from) {
            Ok(())
        } else {
            Err(Error::OutOfScope(label.into()))
        }
    }

    pub fn scope_name(&self, scope: usize) -> String {
        match &self.scopes[scope].handle {
            None => "root".into(),
            Some(handle) => handle
                .path
                .iter()
                .map(|part| part.name.as_ref())
                .collect::<Vec<_>>()
                .join("/"),
        }
    }
}

impl DataflowBuilder {
    /// Build a child template once. Queries discover instances from observed key tuples.
    /// A failed callback leaves the builder unable to finish, so leaked partial handles
    /// cannot alias later definitions.
    pub fn partition_by<T>(
        &mut self,
        name: impl Into<String>,
        input: LogicalPlan,
        keys: Vec<Expr>,
        build: impl FnOnce(&mut ScopeBuilder<'_>) -> Result<T>,
    ) -> Result<(ScopeHandle, T)> {
        let name = name.into();
        let parent = self.current_scope;
        check_name(&self.def.scopes[parent].child_names, "scope", &name)?;
        if keys.is_empty() {
            return Err(Error::InvalidKey(
                "partitioning requires at least one key expression".into(),
            ));
        }
        let input = normalize::normalize_plan(input)?;
        let key_plan = normalize::normalize_plan(
            LogicalPlanBuilder::from(input.clone())
                .project(keys.clone())?
                .build()?,
        )?;
        analysis::analyze(&key_plan, &self.def, parent)?;
        let key_schema = Arc::new(key_plan.schema().as_arrow().clone());
        let mut expressions: Vec<_> = input
            .schema()
            .columns()
            .into_iter()
            .map(Expr::Column)
            .collect();
        for (index, key) in keys.into_iter().enumerate() {
            let mut alias = format!("__scope_key_{index}");
            while input
                .schema()
                .fields()
                .iter()
                .any(|field| field.name() == &alias)
            {
                alias.push('_');
            }
            expressions.push(key.alias(alias));
        }
        let discovery_plan = normalize::normalize_plan(
            LogicalPlanBuilder::from(input.clone())
                .project(expressions)?
                .build()?,
        )?;
        let discovery_analysis = analysis::analyze(&discovery_plan, &self.def, parent)?;
        let discovery = self.def.nodes.len();
        self.def.nodes.push(NodeDef {
            scope: parent,
            local_rows: false,
            name: Arc::from(format!("{name}::$partition")),
            kind: NodeKind::Table,
            plan: discovery_plan,
            analysis: discovery_analysis,
        });
        let index = self.def.scopes.len();
        let mut path = self.def.scopes[parent]
            .handle
            .as_ref()
            .map(|handle| handle.path.to_vec())
            .unwrap_or_default();
        path.push(ScopeIdentity {
            index,
            name: Arc::from(name.as_str()),
        });
        let handle = ScopeHandle {
            graph: self.def.id,
            index,
            parent,
            path: path.into(),
            key_schema,
        };
        self.def.scopes[parent].child_names.insert(name.clone());
        self.def.scopes.push(ScopeDef {
            parent: Some(parent),
            handle: Some(handle.clone()),
            discovery: Some(discovery),
            ..ScopeDef::root()
        });
        let automatic_read = GraphRead {
            graph: self.def.id,
            source: TableRef::Rows(index),
            schema: input.schema().clone(),
            label: Arc::from("$rows"),
        };
        let plan = automatic_read.clone().plan();
        let rows = PlanNode {
            read: GraphRead {
                source: TableRef::Node(self.def.nodes.len()),
                ..automatic_read
            },
        };
        let analysis = analysis::analyze(&plan, &self.def, index)?;
        self.def.nodes.push(NodeDef {
            scope: index,
            local_rows: true,
            name: Arc::from("$rows"),
            kind: NodeKind::Table,
            plan,
            analysis,
        });
        self.def.scopes[index].rows = Some(rows);
        self.current_scope = index;
        let mut scope = ScopeBuilder {
            builder: self,
            parent,
            completed: false,
        };
        let value = build(&mut scope)?;
        scope.completed = true;
        Ok((handle, value))
    }
}

/// Builds one child definition. Captured handles may refer to the current scope or ancestors.
#[derive(Debug)]
pub struct ScopeBuilder<'a> {
    pub(crate) builder: &'a mut DataflowBuilder,
    parent: usize,
    completed: bool,
}

impl Drop for ScopeBuilder<'_> {
    fn drop(&mut self) {
        self.builder.current_scope = self.parent;
        if !self.completed {
            self.builder.failed_scope = true;
        }
    }
}

impl ScopeBuilder<'_> {
    /// Reference the automatically bound local partition, with its source schema.
    pub fn rows(&self) -> PlanNode {
        self.builder.def.scopes[self.builder.current_scope]
            .rows
            .clone()
            .expect("child local rows")
    }
    /// Declare a directly owned table parameter with a fixed schema.
    pub fn table_input(
        &mut self,
        name: impl Into<String>,
        schema: SchemaRef,
    ) -> Result<TableInput> {
        self.builder.table_input(name, schema)
    }
    /// Declare a directly owned scalar parameter with a fixed type.
    pub fn scalar_input(
        &mut self,
        name: impl Into<String>,
        data_type: DataType,
    ) -> Result<ScalarInput> {
        self.builder.scalar_input(name, data_type)
    }
    /// Register one fixed snapshot shared by instances of this scope.
    /// Declare a row expression input owned by this scope.
    pub fn expr_input(
        &mut self,
        name: impl Into<String>,
        data_type: DataType,
    ) -> Result<ExprInput> {
        self.builder.expr_input(name, data_type)
    }
    /// Register one fixed snapshot shared by instances of this scope.
    pub fn table_snapshot(
        &mut self,
        name: impl Into<String>,
        snapshot: crate::TableSnapshot,
    ) -> Result<PlanNode> {
        self.builder.table_snapshot(name, snapshot)
    }
    /// Register a named local table computation.
    pub fn add_plan(&mut self, name: impl Into<String>, plan: LogicalPlan) -> Result<PlanNode> {
        self.builder.add_plan(name, plan)
    }
    /// Register a named local scalar computation.
    /// Register one scalar computation per instance of this scope.
    pub fn add_scalar(&mut self, name: impl Into<String>, expr: Expr) -> Result<ScalarNode> {
        self.builder.add_scalar(name, expr)
    }
    /// Declare a local table value available through scoped results.
    pub fn table_output(
        &mut self,
        name: impl Into<String>,
        node: &PlanNode,
    ) -> Result<TableOutput> {
        self.builder.table_output(name, node)
    }
    /// Declare a local scalar value available through scoped results.
    pub fn scalar_output(
        &mut self,
        name: impl Into<String>,
        node: &ScalarNode,
    ) -> Result<ScalarOutput> {
        self.builder.scalar_output(name, node)
    }
    /// Build a nested template whose source and key expressions belong to this scope.
    pub fn partition_by<T>(
        &mut self,
        name: impl Into<String>,
        input: LogicalPlan,
        keys: Vec<Expr>,
        build: impl FnOnce(&mut ScopeBuilder<'_>) -> Result<T>,
    ) -> Result<(ScopeHandle, T)> {
        self.builder.partition_by(name, input, keys, build)
    }
}
