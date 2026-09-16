use crate::{
    graph::{
        reference::{GraphRead, TableRef},
        GraphDef, InputDef, InputKind, NodeKind,
    },
    Dataflow, Error, Result, ScalarInput, ScalarOutput, ScopeHandle, TableInput, TableOutput,
};
use std::sync::Arc;

#[derive(Debug)]
struct InterfaceDef {
    id: u64,
    inputs: Vec<InputDef>,
    scopes: Vec<(Option<usize>, Option<ScopeHandle>)>,
    outputs: Vec<(usize, Arc<str>, NodeKind)>,
}

/// Typed public names and scope ownership, without source assets or plan lineage.
#[derive(Clone, Debug)]
pub struct DataflowInterface {
    inner: Arc<InterfaceDef>,
}
impl DataflowInterface {
    pub(crate) fn new(graph: &GraphDef) -> Self {
        Self {
            inner: Arc::new(InterfaceDef {
                id: graph.id,
                inputs: graph.inputs.clone(),
                scopes: graph
                    .scopes
                    .iter()
                    .map(|s| (s.parent, s.handle.clone()))
                    .collect(),
                outputs: graph
                    .outputs
                    .iter()
                    .map(|o| (o.scope, o.name.clone(), graph.nodes[o.node].kind))
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
            return Err(Error::InvalidReference(name.into()));
        };
        Ok(TableInput {
            read: GraphRead {
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
            return Err(Error::InvalidReference(name.into()));
        };
        Ok(ScalarInput {
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
            .find(|(_, (s, n, _))| *s == self.scope && n.as_ref() == name)
            .map(|(i, (_, _, k))| (i, *k))
            .ok_or_else(|| Error::InvalidReference(name.into()))
    }
}
