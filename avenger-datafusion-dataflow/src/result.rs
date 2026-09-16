use std::collections::HashMap;

use datafusion::common::ScalarValue;

use crate::{
    inputs::InputValue, Error, EvaluationReport, PartitionKey, Result, ScalarOutput, ScopeHandle,
    ScopeInstance, TableOutput, TableSnapshot,
};

#[derive(Debug)]
pub(crate) struct InstanceResult {
    pub scope: usize,
    pub instance: Option<ScopeInstance>,
    pub outputs: HashMap<usize, InputValue>,
    pub children: HashMap<usize, HashMap<PartitionKey, InstanceResult>>,
}

impl InstanceResult {
    fn table(&self, output: &TableOutput) -> Result<&TableSnapshot> {
        if output.scope != self.scope {
            return Err(Error::OutOfScope(
                "table output belongs to another scope".into(),
            ));
        }
        match self.outputs.get(&output.index) {
            Some(InputValue::Table(value)) => Ok(value),
            _ => Err(Error::UnrequestedOutput),
        }
    }
    fn scalar(&self, output: &ScalarOutput) -> Result<&ScalarValue> {
        if output.scope != self.scope {
            return Err(Error::OutOfScope(
                "scalar output belongs to another scope".into(),
            ));
        }
        match self.outputs.get(&output.index) {
            Some(InputValue::Scalar(value)) => Ok(value),
            _ => Err(Error::UnrequestedOutput),
        }
    }
    fn scope(&self, graph: u64, scope: &ScopeHandle) -> Result<ScopeResults<'_>> {
        if scope.graph != graph {
            return Err(Error::ForeignHandle);
        }
        if scope.parent != self.scope {
            return Err(Error::OutOfScope("scope is not an immediate child".into()));
        }
        let entries = self
            .children
            .get(&scope.index)
            .ok_or(Error::UnrequestedScope)?;
        Ok(ScopeResults { graph, entries })
    }
}

/// Owns fully materialized requested values and their indexed scope hierarchy.
#[derive(Debug)]
pub struct GraphResult {
    pub(crate) graph: u64,
    pub(crate) root: InstanceResult,
    pub(crate) report: EvaluationReport,
}

impl GraphResult {
    /// Read a requested root table without execution or row gathering.
    pub fn table(&self, output: &TableOutput) -> Result<&TableSnapshot> {
        if output.graph != self.graph {
            return Err(Error::ForeignHandle);
        }
        self.root.table(output)
    }
    /// Read a requested root scalar without execution.
    pub fn scalar(&self, output: &ScalarOutput) -> Result<&ScalarValue> {
        if output.graph != self.graph {
            return Err(Error::ForeignHandle);
        }
        self.root.scalar(output)
    }
    /// Access an available top-level collection, including empty collections.
    pub fn scope(&self, scope: &ScopeHandle) -> Result<ScopeResults<'_>> {
        self.root.scope(self.graph, scope)
    }
    /// Return work performed before this result became available.
    pub fn report(&self) -> &EvaluationReport {
        &self.report
    }
}

/// Borrowed indexed instances under one parent. Iteration order is unspecified.
#[derive(Clone, Copy, Debug)]
pub struct ScopeResults<'a> {
    graph: u64,
    entries: &'a HashMap<PartitionKey, InstanceResult>,
}

impl<'a> ScopeResults<'a> {
    /// Iterate observed local keys and materialized result views.
    pub fn iter(&self) -> impl Iterator<Item = (&'a PartitionKey, ScopeResult<'a>)> + 'a {
        let graph = self.graph;
        self.entries
            .iter()
            .map(move |(key, data)| (key, ScopeResult { graph, data }))
    }
    /// Look up a local key. Absent keys and incompatible key types return `None`.
    pub fn get(&self, key: &PartitionKey) -> Option<ScopeResult<'a>> {
        self.entries.get(key).map(|data| ScopeResult {
            graph: self.graph,
            data,
        })
    }
    /// Return the number of observed instances in this collection.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    /// Return whether the source contained no observed keys.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Borrowed materialized outputs and child collections for one instance.
#[derive(Clone, Copy, Debug)]
pub struct ScopeResult<'a> {
    graph: u64,
    data: &'a InstanceResult,
}

impl<'a> ScopeResult<'a> {
    /// Return a complete address suitable for later input overrides.
    pub fn instance(&self) -> &'a ScopeInstance {
        self.data.instance.as_ref().expect("child instance")
    }
    /// Read a requested table in this defining scope.
    pub fn table(&self, output: &TableOutput) -> Result<&'a TableSnapshot> {
        if output.graph != self.graph {
            return Err(Error::ForeignHandle);
        }
        self.data.table(output)
    }
    /// Read a requested scalar in this defining scope.
    pub fn scalar(&self, output: &ScalarOutput) -> Result<&'a ScalarValue> {
        if output.graph != self.graph {
            return Err(Error::ForeignHandle);
        }
        self.data.scalar(output)
    }
    /// Access an available immediate child collection without execution.
    pub fn scope(&self, scope: &ScopeHandle) -> Result<ScopeResults<'a>> {
        self.data.scope(self.graph, scope)
    }
}
