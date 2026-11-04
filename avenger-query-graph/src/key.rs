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

/// The address of a table provider, which identifies it while an `Arc` holds it.
pub(crate) fn address(provider: &Arc<dyn TableProvider>) -> usize {
    Arc::as_ptr(provider) as *const () as usize
}

/// A node's structural identity: the subplan that it computes, with each rollup replaced by the
/// aggregate that it replaces, and the providers of the tables that the subplan scans. DataFusion
/// compares a scan by its table's name and not its provider, so the identity holds the providers,
/// which it compares by address. Identities compare their subplans, not only their hashes.
#[derive(Clone)]
pub(crate) struct Identity(Arc<IdentityParts>);

struct IdentityParts {
    hash: u64,
    plan: LogicalPlan,
    providers: Vec<Arc<dyn TableProvider>>,
}

impl Identity {
    pub fn new(plan: LogicalPlan, providers: Vec<Arc<dyn TableProvider>>) -> Self {
        let mut hasher = hasher();
        plan.hash(&mut hasher);
        for provider in &providers {
            address(provider).hash(&mut hasher);
        }
        Self(Arc::new(IdentityParts {
            hash: hasher.finish(),
            plan,
            providers,
        }))
    }
}

impl PartialEq for Identity {
    fn eq(&self, other: &Self) -> bool {
        let (mine, theirs) = (&self.0, &other.0);
        Arc::ptr_eq(mine, theirs)
            || (mine.hash == theirs.hash
                && mine.providers.len() == theirs.providers.len()
                && (mine.providers.iter().zip(&theirs.providers))
                    .all(|(a, b)| address(a) == address(b))
                && mine.plan == theirs.plan)
    }
}

impl Eq for Identity {}

impl Hash for Identity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash.hash(state);
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
/// reads, the version of each provider that it scans, and the session's settings.
#[derive(Clone)]
pub(crate) struct NodeKey(Arc<KeyParts>);

#[derive(PartialEq, Eq, Hash)]
pub(crate) struct KeyParts {
    pub identity: Identity,
    pub params: Vec<ScalarValue>,
    pub selections: Vec<SelectionKey>,
    pub versions: Vec<u64>,
    pub settings: u64,
}

impl NodeKey {
    pub fn new(parts: KeyParts) -> Self {
        Self(Arc::new(parts))
    }

    /// The identity of the key's node.
    pub fn identity(&self) -> Identity {
        self.0.identity.clone()
    }

    /// The address and the version of each provider that the key's subplan scans.
    pub fn versions(&self) -> impl Iterator<Item = (usize, u64)> + '_ {
        let providers = self.0.identity.0.providers.iter().map(address);
        providers.zip(self.0.versions.iter().copied())
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
