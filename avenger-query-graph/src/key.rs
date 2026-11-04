//! The identities and keys of nodes.

use std::{
    fmt,
    hash::{Hash, Hasher},
    sync::Arc,
};

use avenger_selection::Resolution;
use datafusion::{catalog::TableProvider, common::ScalarValue, logical_expr::LogicalPlan};

/// The hasher of identities and keys.
fn hasher() -> impl Hasher {
    std::hash::DefaultHasher::new()
}

/// A node's structural identity: the subplan that it computes, with each rollup replaced by the
/// aggregate that it replaces. Identities compare their subplans, not only their hashes.
#[derive(Clone)]
pub(crate) struct Identity(Arc<(u64, LogicalPlan)>);

impl Identity {
    pub fn new(plan: LogicalPlan) -> Self {
        let mut hasher = hasher();
        plan.hash(&mut hasher);
        Self(Arc::new((hasher.finish(), plan)))
    }
}

impl PartialEq for Identity {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || (self.0 .0 == other.0 .0 && self.0 .1 == other.0 .1)
    }
}

impl Eq for Identity {}

impl Hash for Identity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0 .0.hash(state);
    }
}

/// A table that a key reads: its provider, compared by address, and its version.
#[derive(Clone)]
pub(crate) struct TableVersion {
    pub provider: Arc<dyn TableProvider>,
    pub version: u64,
}

impl TableVersion {
    fn address(&self) -> usize {
        Arc::as_ptr(&self.provider) as *const () as usize
    }
}

impl PartialEq for TableVersion {
    fn eq(&self, other: &Self) -> bool {
        self.address() == other.address() && self.version == other.version
    }
}

impl Eq for TableVersion {}

impl Hash for TableVersion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.address(), self.version).hash(state);
    }
}

/// The part of a key for one selection: the fingerprints of the clauses of the owners that the
/// node reads, in the state's order, and the state's resolution when there are any.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct SelectionKey {
    pub resolution: Option<u8>,
    pub clauses: Vec<u64>,
}

pub(crate) fn resolution_code(resolution: Resolution) -> u8 {
    match resolution {
        Resolution::Global => 0,
        Resolution::Union => 1,
        Resolution::Intersect => 2,
    }
}

/// A node's key: its identity, the values of the parameters and the clauses that its subplan
/// reads, the versions of the tables that it scans, and the session's settings.
#[derive(Clone)]
pub(crate) struct NodeKey(Arc<KeyParts>);

#[derive(PartialEq, Eq, Hash)]
pub(crate) struct KeyParts {
    pub identity: Identity,
    pub params: Vec<ScalarValue>,
    pub selections: Vec<SelectionKey>,
    pub tables: Vec<TableVersion>,
    pub settings: u64,
}

impl NodeKey {
    pub fn new(parts: KeyParts) -> Self {
        Self(Arc::new(parts))
    }
}

impl PartialEq for NodeKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

impl Eq for NodeKey {}

impl Hash for NodeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl fmt::Debug for NodeKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut hasher = hasher();
        self.0.hash(&mut hasher);
        write!(formatter, "NodeKey({:016x})", hasher.finish())
    }
}

/// A hash of the session's settings, which change results, such as the time zone.
pub(crate) fn settings_hash(state: &datafusion::execution::SessionState) -> u64 {
    let mut entries: Vec<(String, Option<String>)> = (state.config_options().entries().into_iter())
        .map(|entry| (entry.key, entry.value))
        .collect();
    entries.sort();
    let mut hasher = hasher();
    entries.hash(&mut hasher);
    hasher.finish()
}
