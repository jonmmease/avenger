use crate::{
    graph::{
        reference::{GraphRead, TableRef},
        GraphDef, InputDef, InputKind, NodeKind,
    },
    Dataflow, Error, ExprInput, Result, ScalarInput, ScalarOutput, ScopeHandle, TableInput,
    TableOutput,
};
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct InterfaceDef {
    pub id: u64,
    pub semantics: crate::SemanticConfig,
    pub inputs: Vec<InputDef>,
    scopes: Vec<(Option<usize>, Option<ScopeHandle>)>,
    pub outputs: Vec<OutputInterface>,
}

#[derive(Debug)]
pub(crate) struct OutputInterface {
    pub scope: usize,
    pub name: Arc<str>,
    pub kind: NodeKind,
    pub schema: datafusion::common::DFSchemaRef,
}

/// Typed public names and scope ownership, without source assets or plan lineage.
#[derive(Clone, Debug)]
pub struct DataflowInterface {
    pub(crate) inner: Arc<InterfaceDef>,
}
impl DataflowInterface {
    /// Check that a complete input snapshot belongs to this dataflow.
    pub fn validate_inputs(&self, inputs: &crate::Inputs) -> Result<()> {
        if inputs.graph.id != self.inner.id {
            return Err(Error::ForeignHandle);
        }
        Ok(())
    }

    pub(crate) fn new(graph: &GraphDef) -> Self {
        Self {
            inner: Arc::new(InterfaceDef {
                id: graph.id,
                semantics: graph.semantics.clone(),
                inputs: graph.inputs.clone(),
                scopes: graph
                    .scopes
                    .iter()
                    .map(|s| (s.parent, s.handle.clone()))
                    .collect(),
                outputs: graph
                    .outputs
                    .iter()
                    .map(|o| OutputInterface {
                        scope: o.scope,
                        name: o.name.clone(),
                        kind: graph.nodes[o.node].kind,
                        schema: graph.nodes[o.node].plan.schema().clone(),
                    })
                    .collect(),
            }),
        }
    }
    /// Access names declared at the root.
    pub fn root(&self) -> ScopeInterface {
        ScopeInterface {
            interface: self.clone(),
            scope: 0,
        }
    }
}
impl Dataflow {
    /// Retrieve typed handles by name without retaining source assets in the interface.
    pub fn interface(&self) -> DataflowInterface {
        DataflowInterface::new(&self.inner)
    }
}

/// Name lookup for inputs, outputs, and immediate child scopes.
#[derive(Clone, Debug)]
pub struct ScopeInterface {
    interface: DataflowInterface,
    scope: usize,
}
impl ScopeInterface {
    /// Find an immediate child definition.
    pub fn scope(&self, name: &str) -> Result<Self> {
        let scope = self
            .interface
            .inner
            .scopes
            .iter()
            .position(|(parent, h)| {
                *parent == Some(self.scope) && h.as_ref().is_some_and(|h| h.name() == name)
            })
            .ok_or_else(|| Error::InvalidReference(name.into()))?;
        Ok(Self {
            interface: self.interface.clone(),
            scope,
        })
    }
    /// Get the scope handle. The root has no partition handle.
    pub fn handle(&self) -> Option<&ScopeHandle> {
        self.interface.inner.scopes[self.scope].1.as_ref()
    }
    /// Look up a table input declared directly in this scope.
    pub fn table_input(&self, name: &str) -> Result<TableInput> {
        let (index, input) = self.input(name)?;
        let InputKind::Table(schema) = &input.kind else {
            return Err(kind_error(name, "table", &input.kind));
        };
        Ok(TableInput {
            read: GraphRead {
                row_index: None,
                graph: self.interface.inner.id,
                source: TableRef::Input(index),
                schema: schema.clone(),
                label: input.name.clone(),
            },
        })
    }
    /// Look up a scalar input declared directly in this scope.
    pub fn scalar_input(&self, name: &str) -> Result<ScalarInput> {
        let (index, input) = self.input(name)?;
        let InputKind::Scalar(field) = &input.kind else {
            return Err(kind_error(name, "scalar", &input.kind));
        };
        Ok(ScalarInput {
            graph: self.interface.inner.id,
            index,
            name: input.name.clone(),
            field: field.clone(),
        })
    }
    /// Look up an expression input declared directly in this scope.
    pub fn expr_input(&self, name: &str) -> Result<ExprInput> {
        let (index, input) = self.input(name)?;
        let InputKind::Expr(field) = &input.kind else {
            return Err(kind_error(name, "expr", &input.kind));
        };
        Ok(ExprInput {
            graph: self.interface.inner.id,
            index,
            name: input.name.clone(),
            field: field.clone(),
        })
    }
    /// Look up a declared table output.
    pub fn table_output(&self, name: &str) -> Result<TableOutput> {
        let (index, kind) = self.output(name)?;
        if !matches!(kind, NodeKind::Table) {
            return Err(Error::InvalidReference(name.into()));
        }
        Ok(TableOutput {
            graph: self.interface.inner.id,
            index,
            scope: self.scope,
        })
    }
    /// Look up a declared scalar output.
    pub fn scalar_output(&self, name: &str) -> Result<ScalarOutput> {
        let (index, kind) = self.output(name)?;
        if !matches!(kind, NodeKind::Scalar) {
            return Err(Error::InvalidReference(name.into()));
        }
        Ok(ScalarOutput {
            graph: self.interface.inner.id,
            index,
            scope: self.scope,
        })
    }
    /// Enumerate directly declared inputs in declaration order, without executing data.
    pub fn inputs(&self) -> impl Iterator<Item = InputHandle> + '_ {
        self.interface
            .inner
            .inputs
            .iter()
            .filter(move |input| input.scope == self.scope)
            .map(|input| match &input.kind {
                InputKind::Table(_) => {
                    InputHandle::Table(self.table_input(&input.name).expect("declared input"))
                }
                InputKind::Scalar(_) => {
                    InputHandle::Scalar(self.scalar_input(&input.name).expect("declared input"))
                }
                InputKind::Expr(_) => {
                    InputHandle::Expr(self.expr_input(&input.name).expect("declared input"))
                }
            })
    }

    fn input(&self, name: &str) -> Result<(usize, &InputDef)> {
        self.interface
            .inner
            .inputs
            .iter()
            .enumerate()
            .find(|(_, i)| i.scope == self.scope && i.name.as_ref() == name)
            .ok_or_else(|| Error::InvalidReference(name.into()))
    }
    fn output(&self, name: &str) -> Result<(usize, NodeKind)> {
        self.interface
            .inner
            .outputs
            .iter()
            .enumerate()
            .find(|(_, output)| output.scope == self.scope && output.name.as_ref() == name)
            .map(|(i, output)| (i, output.kind))
            .ok_or_else(|| Error::InvalidReference(name.into()))
    }
}

/// A typed input discovered through a dataflow's public interface.
#[derive(Clone, Debug)]
pub enum InputHandle {
    Table(TableInput),
    Scalar(ScalarInput),
    Expr(ExprInput),
}
impl InputHandle {
    /// Return the name within the input's declaring scope.
    pub fn name(&self) -> &str {
        match self {
            Self::Table(i) => i.name(),
            Self::Scalar(i) => i.name(),
            Self::Expr(i) => i.name(),
        }
    }
}
fn kind_error(name: &str, expected: &'static str, kind: &InputKind) -> Error {
    Error::InputKindMismatch {
        name: name.into(),
        expected,
        actual: match kind {
            InputKind::Table(_) => "table",
            InputKind::Scalar(_) => "scalar",
            InputKind::Expr(_) => "expr",
        },
    }
}

/// Portable address of a named input, computation, or output.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Reference {
    /// Scope path. An empty path denotes the root.
    pub scope: Vec<String>,
    /// Name within the addressed scope and namespace.
    pub name: String,
}

/// A published output's portable address and declared schema.
#[derive(Clone, Debug)]
pub struct OutputMetadata {
    pub reference: Reference,
    pub schema: datafusion::common::DFSchemaRef,
}

impl DataflowInterface {
    /// Resolve a sequence of scope names. An empty path denotes the root.
    pub fn scope_at(&self, path: &[String]) -> Result<ScopeInterface> {
        path.iter()
            .try_fold(self.root(), |scope, name| scope.scope(name))
    }

    fn path(&self, index: usize) -> Vec<String> {
        self.inner.scopes[index]
            .1
            .as_ref()
            .map_or_else(Vec::new, |h| {
                h.path.iter().map(|s| s.name.to_string()).collect()
            })
    }

    /// Return a scope's portable path after checking graph ownership.
    pub fn scope_path(&self, handle: &ScopeHandle) -> Result<Vec<String>> {
        if handle.graph != self.inner.id {
            return Err(Error::ForeignHandle);
        }
        Ok(self.path(handle.index))
    }

    /// Inspect a table output without preparing or executing its plan.
    pub fn table_metadata(&self, handle: &TableOutput) -> Result<OutputMetadata> {
        self.output_metadata(handle.graph, handle.index)
    }

    /// Inspect a scalar output without evaluating its expression.
    pub fn scalar_metadata(&self, handle: &ScalarOutput) -> Result<OutputMetadata> {
        self.output_metadata(handle.graph, handle.index)
    }

    fn output_metadata(&self, graph: u64, index: usize) -> Result<OutputMetadata> {
        if graph != self.inner.id {
            return Err(Error::ForeignHandle);
        }
        let output = &self.inner.outputs[index];
        Ok(OutputMetadata {
            reference: Reference {
                scope: self.path(output.scope),
                name: output.name.to_string(),
            },
            schema: output.schema.clone(),
        })
    }

    /// Address a scalar input after checking graph ownership.
    pub fn scalar_input_reference(&self, handle: &ScalarInput) -> Result<Reference> {
        if handle.graph != self.inner.id {
            return Err(Error::ForeignHandle);
        }
        let input = &self.inner.inputs[handle.index];
        Ok(Reference {
            scope: self.path(input.scope),
            name: input.name.to_string(),
        })
    }
}
