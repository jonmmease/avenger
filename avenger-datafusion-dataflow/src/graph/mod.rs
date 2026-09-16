pub(crate) mod analysis;
pub(crate) mod normalize;
pub(crate) mod scope;
pub use scope::ScopeBuilder;
pub(crate) use scope::ScopeDef;
pub(crate) mod reference;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use datafusion::{
    arrow::datatypes::{DataType, Field, FieldRef, SchemaRef},
    common::{DFSchema, DFSchemaRef},
    logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder},
};

use crate::{fresh_id, Error, Result};
use reference::{placeholder_id, scalar_ref, GraphRead, ScalarRef, TableRef};

#[derive(Clone, Debug)]
pub struct TableInput {
    pub(crate) read: GraphRead,
}
impl TableInput {
    pub fn plan_ref(&self) -> LogicalPlan {
        self.read.clone().plan()
    }
    pub fn schema(&self) -> &DFSchemaRef {
        &self.read.schema
    }
    pub fn name(&self) -> &str {
        &self.read.label
    }
}

#[derive(Clone, Debug)]
pub struct PlanNode {
    pub(crate) read: GraphRead,
}
impl PlanNode {
    pub fn plan_ref(&self) -> LogicalPlan {
        self.read.clone().plan()
    }
    pub fn schema(&self) -> &DFSchemaRef {
        &self.read.schema
    }
    pub fn name(&self) -> &str {
        &self.read.label
    }
}

#[derive(Clone, Debug)]
pub struct ScalarInput {
    pub(crate) graph: u64,
    pub(crate) index: usize,
    pub(crate) name: Arc<str>,
    pub(crate) field: FieldRef,
}
impl ScalarInput {
    pub fn expr_ref(&self) -> Expr {
        scalar_ref(self.graph, ScalarRef::Input(self.index), self.field.clone())
    }
    pub fn field(&self) -> &FieldRef {
        &self.field
    }
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone, Debug)]
pub struct ExprNode {
    graph: u64,
    index: usize,
    name: Arc<str>,
    field: FieldRef,
}
impl ExprNode {
    pub fn expr_ref(&self) -> Expr {
        scalar_ref(self.graph, ScalarRef::Node(self.index), self.field.clone())
    }
    pub fn field(&self) -> &FieldRef {
        &self.field
    }
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// A declared table output. Values remain private to their graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TableOutput {
    pub(crate) graph: u64,
    pub(crate) index: usize,
    pub(crate) scope: usize,
}

/// A declared scalar output. Values remain private to their graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScalarOutput {
    pub(crate) graph: u64,
    pub(crate) index: usize,
    pub(crate) scope: usize,
}

#[derive(Clone, Debug)]
pub(crate) enum InputKind {
    Table(DFSchemaRef),
    Scalar(FieldRef),
}
#[derive(Clone, Debug)]
pub(crate) struct InputDef {
    pub scope: usize,
    pub name: Arc<str>,
    pub kind: InputKind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeKind {
    Table,
    Scalar,
}

#[derive(Clone, Debug)]
pub(crate) struct NodeDef {
    pub scope: usize,
    pub local_rows: bool,
    pub name: Arc<str>,
    pub kind: NodeKind,
    pub plan: LogicalPlan,
    pub analysis: analysis::Analysis,
}

#[derive(Clone, Debug)]
pub(crate) struct OutputDef {
    pub scope: usize,
    pub name: Arc<str>,
    pub node: usize,
}

#[derive(Debug)]
pub(crate) struct BindingMetadata {
    pub id: u64,
    pub inputs: Vec<InputDef>,
}

#[derive(Clone, Debug)]
pub(crate) struct GraphDef {
    pub assets: Vec<crate::TableSnapshot>,
    pub semantics: crate::SemanticConfig,
    pub id: u64,
    pub scopes: Vec<ScopeDef>,
    pub inputs: Vec<InputDef>,
    pub nodes: Vec<NodeDef>,
    pub outputs: Vec<OutputDef>,
    pub placeholders: HashMap<String, (ScalarRef, FieldRef)>,
}

#[derive(Clone, Debug)]
pub struct Dataflow {
    pub(crate) inner: Arc<GraphDef>,
}

impl Dataflow {
    pub fn num_nodes(&self) -> usize {
        self.inner.nodes.len()
    }
    pub fn num_inputs(&self) -> usize {
        self.inner.inputs.len()
    }
    pub fn num_outputs(&self) -> usize {
        self.inner.outputs.len()
    }
}

/// Registers nodes in dependency order. Returned references never duplicate lineage.
#[derive(Debug)]
pub struct DataflowBuilder {
    def: GraphDef,
    current_scope: usize,
    failed_scope: bool,
}

impl Default for DataflowBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DataflowBuilder {
    /// Construct a definition with explicit result-affecting requirements.
    pub fn with_semantics(semantics: crate::SemanticConfig) -> Self {
        let mut builder = Self::new();
        builder.def.semantics = semantics;
        builder
    }

    pub fn new() -> Self {
        Self {
            def: GraphDef {
                id: fresh_id(),
                assets: vec![],
                semantics: crate::SemanticConfig::default(),
                scopes: vec![ScopeDef::root()],
                inputs: vec![],
                nodes: vec![],
                outputs: vec![],
                placeholders: HashMap::new(),
            },
            current_scope: 0,
            failed_scope: false,
        }
    }

    pub fn table_input(
        &mut self,
        name: impl Into<String>,
        schema: SchemaRef,
    ) -> Result<TableInput> {
        let name = name.into();
        check_name(
            &self.def.scopes[self.current_scope].input_names,
            "input",
            &name,
        )?;
        let schema = Arc::new(DFSchema::try_from_qualified_schema(
            name.as_str(),
            schema.as_ref(),
        )?);
        let read = GraphRead {
            graph: self.def.id,
            source: TableRef::Input(self.def.inputs.len()),
            schema: schema.clone(),
            label: Arc::from(name.as_str()),
        };
        self.def.inputs.push(InputDef {
            scope: self.current_scope,
            name: read.label.clone(),
            kind: InputKind::Table(schema),
        });
        self.def.scopes[self.current_scope].input_names.insert(name);
        Ok(TableInput { read })
    }

    pub fn scalar_input(
        &mut self,
        name: impl Into<String>,
        data_type: DataType,
    ) -> Result<ScalarInput> {
        let name = name.into();
        check_name(
            &self.def.scopes[self.current_scope].input_names,
            "input",
            &name,
        )?;
        let index = self.def.inputs.len();
        let field = Arc::new(Field::new(&name, data_type, true));
        let input = ScalarInput {
            graph: self.def.id,
            index,
            name: Arc::from(name.as_str()),
            field: field.clone(),
        };
        self.def.inputs.push(InputDef {
            scope: self.current_scope,
            name: input.name.clone(),
            kind: InputKind::Scalar(field.clone()),
        });
        self.def.placeholders.insert(
            placeholder_id(self.def.id, ScalarRef::Input(index)),
            (ScalarRef::Input(index), field),
        );
        self.def.scopes[self.current_scope].input_names.insert(name);
        Ok(input)
    }

    /// Register an immutable graph-owned table, independent of query bindings.
    pub fn table_snapshot(
        &mut self,
        name: impl Into<String>,
        snapshot: crate::TableSnapshot,
    ) -> Result<PlanNode> {
        let name = name.into();
        check_name(
            &self.def.scopes[self.current_scope].node_names,
            "computation",
            &name,
        )?;
        let read = GraphRead {
            graph: self.def.id,
            source: TableRef::Asset(self.def.assets.len()),
            schema: Arc::new(DFSchema::try_from_qualified_schema(
                name.as_str(),
                snapshot.schema().as_ref(),
            )?),
            label: Arc::from(name.as_str()),
        };
        self.def.assets.push(snapshot);
        self.add_plan(name, read.plan())
    }

    pub fn add_plan(&mut self, name: impl Into<String>, plan: LogicalPlan) -> Result<PlanNode> {
        let name = name.into();
        check_name(
            &self.def.scopes[self.current_scope].node_names,
            "computation",
            &name,
        )?;
        let plan = normalize::normalize_plan(plan)?;
        let analysis = analysis::analyze(&plan, &self.def, self.current_scope)?;
        let read = GraphRead {
            graph: self.def.id,
            source: TableRef::Node(self.def.nodes.len()),
            schema: plan.schema().clone(),
            label: Arc::from(name.as_str()),
        };
        self.def.nodes.push(NodeDef {
            scope: self.current_scope,
            local_rows: false,
            name: read.label.clone(),
            kind: NodeKind::Table,
            plan,
            analysis,
        });
        self.def.scopes[self.current_scope].node_names.insert(name);
        Ok(PlanNode { read })
    }

    pub fn add_expr(&mut self, name: impl Into<String>, expr: Expr) -> Result<ExprNode> {
        let name = name.into();
        check_name(
            &self.def.scopes[self.current_scope].node_names,
            "computation",
            &name,
        )?;
        analysis::validate_scalar(&expr)?;
        let expr = normalize::normalize_expr(expr)?;
        let plan = LogicalPlanBuilder::empty(true)
            .project(vec![expr.alias(&name)])?
            .build()?;
        let analysis = analysis::analyze(&plan, &self.def, self.current_scope)?;
        let field = plan.schema().field(0).clone();
        let index = self.def.nodes.len();
        let node = ExprNode {
            graph: self.def.id,
            index,
            name: Arc::from(name.as_str()),
            field: field.clone(),
        };
        self.def.nodes.push(NodeDef {
            scope: self.current_scope,
            local_rows: false,
            name: node.name.clone(),
            kind: NodeKind::Scalar,
            plan,
            analysis,
        });
        self.def.placeholders.insert(
            placeholder_id(self.def.id, ScalarRef::Node(index)),
            (ScalarRef::Node(index), field),
        );
        self.def.scopes[self.current_scope].node_names.insert(name);
        Ok(node)
    }

    pub fn table_output(
        &mut self,
        name: impl Into<String>,
        node: &PlanNode,
    ) -> Result<TableOutput> {
        if node.read.graph != self.def.id {
            return Err(Error::ForeignHandle);
        }
        let TableRef::Node(index) = node.read.source else {
            unreachable!()
        };
        Ok(TableOutput {
            graph: self.def.id,
            index: self.add_output(name.into(), index)?,
            scope: self.current_scope,
        })
    }

    pub fn scalar_output(
        &mut self,
        name: impl Into<String>,
        node: &ExprNode,
    ) -> Result<ScalarOutput> {
        if node.graph != self.def.id {
            return Err(Error::ForeignHandle);
        }
        Ok(ScalarOutput {
            graph: self.def.id,
            index: self.add_output(name.into(), node.index)?,
            scope: self.current_scope,
        })
    }

    fn add_output(&mut self, name: String, node: usize) -> Result<usize> {
        check_name(
            &self.def.scopes[self.current_scope].output_names,
            "output",
            &name,
        )?;
        if self.def.nodes[node].scope != self.current_scope {
            return Err(Error::OutOfScope(
                "output must be declared in its producer scope".into(),
            ));
        }
        let index = self.def.outputs.len();
        self.def.outputs.push(OutputDef {
            scope: self.current_scope,
            name: Arc::from(name.as_str()),
            node,
        });
        self.def.scopes[self.current_scope]
            .output_names
            .insert(name);
        Ok(index)
    }

    pub fn finish(self) -> Result<Dataflow> {
        if self.failed_scope {
            return Err(Error::InvalidReference(
                "scope construction failed, discard this builder".into(),
            ));
        }
        // Registration permits only existing, same-graph references, so insertion
        // order is a topological order and cycles cannot be constructed.
        Ok(Dataflow {
            inner: Arc::new(self.def),
        })
    }
}

fn check_name(names: &HashSet<String>, namespace: &'static str, name: &str) -> Result<()> {
    if names.contains(name) {
        return Err(Error::DuplicateName {
            namespace,
            name: name.into(),
        });
    }
    Ok(())
}

/// Functional dependencies are derived optimizer metadata, not part of the interface.
pub(crate) fn same_schema(left: &DFSchema, right: &DFSchema) -> bool {
    left.as_arrow() == right.as_arrow()
        && left
            .iter()
            .map(|(qualifier, _)| qualifier)
            .eq(right.iter().map(|(qualifier, _)| qualifier))
}
