mod analysis;
mod normalize;
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
}

/// A declared scalar output. Values remain private to their graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScalarOutput {
    pub(crate) graph: u64,
    pub(crate) index: usize,
}

#[derive(Clone, Debug)]
pub(crate) enum InputKind {
    Table(DFSchemaRef),
    Scalar(FieldRef),
}
#[derive(Clone, Debug)]
pub(crate) struct InputDef {
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
    pub name: Arc<str>,
    pub kind: NodeKind,
    pub plan: LogicalPlan,
    pub analysis: analysis::Analysis,
}

#[derive(Clone, Debug)]
pub(crate) struct OutputDef {
    pub name: Arc<str>,
    pub node: usize,
}

#[derive(Debug)]
pub(crate) struct GraphDef {
    pub id: u64,
    pub inputs: Vec<InputDef>,
    pub nodes: Vec<NodeDef>,
    pub outputs: Vec<OutputDef>,
    pub placeholders: HashMap<String, (ScalarRef, FieldRef)>,
}

#[derive(Clone, Debug)]
pub struct Graph {
    pub(crate) inner: Arc<GraphDef>,
}

impl Graph {
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
pub struct GraphBuilder {
    def: GraphDef,
    input_names: HashSet<String>,
    node_names: HashSet<String>,
    output_names: HashSet<String>,
}

impl Default for GraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphBuilder {
    pub fn new() -> Self {
        Self {
            def: GraphDef {
                id: fresh_id(),
                inputs: vec![],
                nodes: vec![],
                outputs: vec![],
                placeholders: HashMap::new(),
            },
            input_names: HashSet::new(),
            node_names: HashSet::new(),
            output_names: HashSet::new(),
        }
    }

    pub fn table_input(
        &mut self,
        name: impl Into<String>,
        schema: SchemaRef,
    ) -> Result<TableInput> {
        let name = name.into();
        check_name(&self.input_names, "input", &name)?;
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
            name: read.label.clone(),
            kind: InputKind::Table(schema),
        });
        self.input_names.insert(name);
        Ok(TableInput { read })
    }

    pub fn scalar_input(
        &mut self,
        name: impl Into<String>,
        data_type: DataType,
    ) -> Result<ScalarInput> {
        let name = name.into();
        check_name(&self.input_names, "input", &name)?;
        let index = self.def.inputs.len();
        let field = Arc::new(Field::new(&name, data_type, true));
        let input = ScalarInput {
            graph: self.def.id,
            index,
            name: Arc::from(name.as_str()),
            field: field.clone(),
        };
        self.def.inputs.push(InputDef {
            name: input.name.clone(),
            kind: InputKind::Scalar(field.clone()),
        });
        self.def.placeholders.insert(
            placeholder_id(self.def.id, ScalarRef::Input(index)),
            (ScalarRef::Input(index), field),
        );
        self.input_names.insert(name);
        Ok(input)
    }

    pub fn add_plan(&mut self, name: impl Into<String>, plan: LogicalPlan) -> Result<PlanNode> {
        let name = name.into();
        check_name(&self.node_names, "computation", &name)?;
        let plan = normalize::normalize_plan(plan)?;
        let analysis = analysis::analyze(&plan, &self.def)?;
        let read = GraphRead {
            graph: self.def.id,
            source: TableRef::Node(self.def.nodes.len()),
            schema: plan.schema().clone(),
            label: Arc::from(name.as_str()),
        };
        self.def.nodes.push(NodeDef {
            name: read.label.clone(),
            kind: NodeKind::Table,
            plan,
            analysis,
        });
        self.node_names.insert(name);
        Ok(PlanNode { read })
    }

    pub fn add_expr(&mut self, name: impl Into<String>, expr: Expr) -> Result<ExprNode> {
        let name = name.into();
        check_name(&self.node_names, "computation", &name)?;
        analysis::validate_scalar(&expr)?;
        let expr = normalize::normalize_expr(expr)?;
        let plan = LogicalPlanBuilder::empty(true)
            .project(vec![expr.alias(&name)])?
            .build()?;
        let analysis = analysis::analyze(&plan, &self.def)?;
        let field = plan.schema().field(0).clone();
        let index = self.def.nodes.len();
        let node = ExprNode {
            graph: self.def.id,
            index,
            name: Arc::from(name.as_str()),
            field: field.clone(),
        };
        self.def.nodes.push(NodeDef {
            name: node.name.clone(),
            kind: NodeKind::Scalar,
            plan,
            analysis,
        });
        self.def.placeholders.insert(
            placeholder_id(self.def.id, ScalarRef::Node(index)),
            (ScalarRef::Node(index), field),
        );
        self.node_names.insert(name);
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
        })
    }

    fn add_output(&mut self, name: String, node: usize) -> Result<usize> {
        check_name(&self.output_names, "output", &name)?;
        let index = self.def.outputs.len();
        self.def.outputs.push(OutputDef {
            name: Arc::from(name.as_str()),
            node,
        });
        self.output_names.insert(name);
        Ok(index)
    }

    pub fn finish(self) -> Result<Graph> {
        // Registration permits only existing, same-graph references, so insertion
        // order is a topological order and cycles cannot be constructed.
        Ok(Graph {
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
