use crate::{
    DataflowResult, InputOverrides, Inputs, PlanNode, PreparedDataflow, Reference, Result,
    ScalarNode,
};

/// Ordered candidates, starting with complete preferred bindings.
#[derive(Clone, Debug)]
pub struct QueryInputs {
    pub(crate) candidates: Vec<Inputs>,
}

impl QueryInputs {
    /// Capture the preferred bindings as candidate zero.
    pub fn new(latest: Inputs) -> Self {
        Self {
            candidates: vec![latest],
        }
    }

    /// Append fallbacks in preference order, each inheriting directly from candidate zero.
    pub fn fallbacks(
        mut self,
        overrides: impl IntoIterator<Item = InputOverrides>,
    ) -> Result<Self> {
        for overrides in overrides {
            self.candidates.push(overrides.apply(&self.candidates[0])?);
        }
        Ok(self)
    }
}

/// Computations that must be retained for an input candidate to qualify.
#[derive(Clone, Debug, Default)]
pub enum CacheTargets {
    /// Require the nodes producing the requested outputs.
    #[default]
    RequestedOutputs,
    /// Require every listed reusable ancestor of the requested outputs.
    Nodes(Vec<CacheNode>),
}

/// A computation addressed by a typed handle or its definition name.
#[derive(Clone, Debug)]
pub enum CacheNode {
    /// A named table-producing computation.
    Plan(PlanNode),
    /// A named scalar-producing computation.
    Scalar(ScalarNode),
    /// A computation name, rather than a public output alias. Only root names are supported.
    Named(Reference),
}

/// Cache requirements and optional work for the preferred inputs.
#[derive(Clone, Debug, Default)]
pub struct CacheAwareOptions {
    /// Required cache hits and the endpoint of optional background work.
    pub targets: CacheTargets,
    /// Eagerly schedule target evaluation on the current Tokio runtime.
    /// Dropping the query cancels queued work. Admitted work finishes independently.
    pub start_latest: bool,
}

/// Work permitted by a foreground read. Neither mode waits for missing targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheRead {
    /// Require retained targets and outputs. Perform no computation.
    CachedOnly,
    /// Require retained targets, then evaluate remaining outputs with the selected bindings.
    FromCachedTargets,
}

/// Captured candidates and ownership of optional background target work.
///
/// Reads borrow this query. Dropping it removes queued background work. Once
/// admitted, a warm-up runs to completion or failure even after the query is dropped.
/// Retained queries remain eligible and do not supersede each other automatically.
#[must_use = "retain the query while its queued background work is wanted"]
pub struct CacheAwareQuery {
    pub(crate) prepared: PreparedDataflow,
    pub(crate) inputs: QueryInputs,
    pub(crate) requested: Vec<bool>,
    pub(crate) targets: Vec<usize>,
    pub(crate) _background: Option<crate::runtime::warming::Interest>,
}

/// Requested values and the complete bindings used to produce them.
#[derive(Debug)]
pub struct CacheAwareResult {
    pub(crate) result: DataflowResult,
    pub(crate) inputs: Inputs,
    pub(crate) candidate_index: usize,
}

impl CacheAwareResult {
    /// Read requested values and foreground diagnostics.
    pub fn result(&self) -> &DataflowResult {
        &self.result
    }

    /// Read the selected complete bindings, including inherited interaction values.
    pub fn inputs(&self) -> &Inputs {
        &self.inputs
    }

    /// Zero denotes the preferred inputs captured by this request.
    /// Subsequent indices identify fallbacks in their supplied order.
    pub fn candidate_index(&self) -> usize {
        self.candidate_index
    }
}
