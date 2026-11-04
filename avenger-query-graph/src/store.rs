//! The store of kept results, sources, and computations that its graphs share.

use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use avenger_common::time::{Duration, Instant};
use datafusion::{
    catalog::TableProvider,
    common::{DataFusionError, ResolvedTableReference, Result, TableReference},
    execution::SessionState,
    logical_expr::LogicalPlan,
};
use futures::future::{BoxFuture, Shared};
use tokio::task::AbortHandle;

use crate::{
    cut::Kept,
    graph::{GraphId, QueryGraph},
    key::NodeKey,
    sources,
};

/// The limits of a store's kept results.
#[derive(Clone, Debug)]
pub struct StoreConfig {
    /// The bytes of kept results that the store keeps before it evicts the least recently used.
    /// It bounds the kept results, not the memory of the computations in progress.
    pub budget: usize,
    /// The bytes of the largest result that the store keeps. A run uses a larger result and does
    /// not keep it.
    pub max_result: usize,
}

/// The options of a source, by its table's name.
#[derive(Clone, Debug, Default)]
pub struct SourceOptions {
    /// Whether each scan of the source is a node, so later runs can reuse the scan's kept result
    /// instead of reading the source. Applies to the graphs that the store compiles afterwards.
    pub materialize: bool,
    /// The minimum interval between checks of the source, by a run or by
    /// [`ResultStore::revalidate`]. When several names of one provider set one, the shortest
    /// applies, and the provider is unchecked only when none of its names sets one. An unchecked
    /// source keeps its version until [`ResultStore::table_changed`]. Applies at the next run of
    /// every graph.
    pub ttl: Option<Duration>,
}

/// Kept results, sources, and computations, shared by the graphs that it compiles. Clones share
/// the store.
#[derive(Clone)]
pub struct ResultStore {
    inner: Arc<Inner>,
}

struct Inner {
    config: StoreConfig,
    state: Mutex<State>,
}

#[derive(Default)]
pub(crate) struct State {
    /// The options of the sources, by the references that `set_source_options` took.
    pub options: Vec<(TableReference, SourceOptions)>,
    /// The sources that the store's graphs scan, by the address of their provider.
    pub sources: HashMap<usize, Source>,
    pub entries: HashMap<NodeKey, Entry>,
    pub bytes: usize,
    pub tick: u64,
    /// The computations in progress: those of a node that is kept by its key, which other runs
    /// can join, and the others under `None`.
    pub flights: HashMap<Option<NodeKey>, Vec<Arc<Flight>>>,
    pub next_graph: u64,
    pub next_flight: u64,
}

/// A source: one provider, each name under which the store's graphs scan it with the session of
/// that name, its version, and what its last check found.
pub(crate) struct Source {
    pub provider: Arc<dyn TableProvider>,
    pub names: Vec<(ResolvedTableReference, SessionState)>,
    pub version: u64,
    pub checked: Option<Instant>,
    pub files: Option<Vec<sources::FileMeta>>,
}

/// A kept result, with the tick of its last use.
pub(crate) struct Entry {
    pub kept: Arc<Kept>,
    pub used: u64,
}

/// The result of a computation that runs share.
pub(crate) type FlightResult =
    Shared<BoxFuture<'static, Result<Arc<Computed>, Arc<DataFusionError>>>>;

/// A computed result and how it was computed.
#[derive(Debug)]
pub(crate) struct Computed {
    pub kept: Arc<Kept>,
    pub elapsed: Duration,
    pub widened: Vec<String>,
    /// Whether the store kept the result.
    pub stored: bool,
}

/// A computation in progress, under its key.
pub(crate) struct Flight {
    pub id: u64,
    pub columns: BTreeSet<usize>,
    pub result: FlightResult,
    pub abort: AbortHandle,
    /// The runs that wait for the computation, and the computations that read its result.
    pub waiters: Mutex<usize>,
    /// The computations whose results this computation reads, once its run sends its job.
    pub reads: Mutex<Vec<Arc<Flight>>>,
    /// The graph whose run started the computation.
    pub graph: GraphId,
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The default catalog and schema of `session`, which resolve its unqualified references.
pub(crate) fn defaults(session: &SessionState) -> (String, String) {
    let catalog = &session.config_options().catalog;
    (
        catalog.default_catalog.clone(),
        catalog.default_schema.clone(),
    )
}

/// `table` resolved with the default catalog and schema of `session`.
pub(crate) fn resolve(table: &TableReference, session: &SessionState) -> ResolvedTableReference {
    let catalog = &session.config_options().catalog;
    table
        .clone()
        .resolve(&catalog.default_catalog, &catalog.default_schema)
}

/// The shortest time to live that the options of any name of `source` set.
pub(crate) fn ttl_of(
    options: &[(TableReference, SourceOptions)],
    source: &Source,
) -> Option<Duration> {
    (source.names.iter())
        .filter_map(|(table, session)| options_of(options, table, session)?.ttl)
        .min()
}

/// The options of the table `table`, by the last of the references that match it.
pub(crate) fn options_of<'a>(
    options: &'a [(TableReference, SourceOptions)],
    table: &ResolvedTableReference,
    session: &SessionState,
) -> Option<&'a SourceOptions> {
    (options.iter().rev())
        .find(|(reference, _)| resolve(reference, session) == *table)
        .map(|(_, options)| options)
}

impl ResultStore {
    /// A store with the limits `config`.
    pub fn new(config: StoreConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                config,
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// Set the options of the source `table`, a table reference such as `flights` or
    /// `public.flights`, which resolves with each session's default catalog and schema.
    pub fn set_source_options(&self, table: &str, options: SourceOptions) {
        let reference = TableReference::from(table);
        let mut state = self.state();
        state.options.retain(|(kept, _)| *kept != reference);
        state.options.push((reference, options));
    }

    /// Report a change inside the table `table`, such as an `INSERT INTO` a `MemTable`, which
    /// gives each provider that the store's graphs scan under that name a new version, so the
    /// nodes that read it compute again.
    pub fn table_changed(&self, table: &str) {
        let reference = TableReference::from(table);
        let mut state = self.state();
        for source in state.sources.values_mut() {
            let named = (source.names.iter())
                .any(|(table, session)| resolve(&reference, session) == *table);
            if named {
                source.version += 1;
            }
        }
    }

    /// Check each source whose time to live has passed. True when any of them has a new version.
    pub async fn revalidate(&self) -> Result<bool> {
        let sources: Vec<usize> = self.state().sources.keys().copied().collect();
        sources::check(self, &sources).await
    }

    /// Cut each plan of one document, an unoptimized plan whose parameters and selection states
    /// are placeholders, into one graph, which runs with `state`'s settings and runtime.
    pub fn compile(&self, state: &SessionState, plans: Vec<LogicalPlan>) -> Result<QueryGraph> {
        QueryGraph::compile(self.clone(), state, plans)
    }

    pub(crate) fn config(&self) -> &StoreConfig {
        &self.inner.config
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.inner.state)
    }

    /// Whether the scans of `table` are materialized in `session`.
    pub(crate) fn materialized(&self, table: &TableReference, session: &SessionState) -> bool {
        let state = self.state();
        options_of(&state.options, &resolve(table, session), session)
            .is_some_and(|options| options.materialize)
    }

    /// A new graph's id.
    pub(crate) fn graph_id(&self) -> GraphId {
        let mut state = self.state();
        state.next_graph += 1;
        GraphId(state.next_graph)
    }
}

impl State {
    /// The kept result of `key`, marked as used, when it holds `columns`.
    pub fn hit(&mut self, key: &NodeKey, columns: &BTreeSet<usize>) -> Option<Arc<Kept>> {
        self.tick += 1;
        let tick = self.tick;
        let entry = self.entries.get_mut(key)?;
        entry.used = tick;
        entry.kept.holds(columns).then(|| entry.kept.clone())
    }

    /// The columns of the kept result of `key`.
    pub fn kept_columns(&self, key: &NodeKey) -> Option<Vec<usize>> {
        self.entries
            .get(key)
            .map(|entry| entry.kept.columns.clone())
    }

    /// Keep `kept` under `key` unless the kept result holds a column that it lacks, then evict the
    /// least recently used results until the kept results fit `budget`. False when `kept` is
    /// larger than `max_result` or a wider result is kept.
    pub fn keep(&mut self, config: &StoreConfig, key: NodeKey, kept: Arc<Kept>) -> bool {
        // A result that the store cannot keep evicts nothing.
        if kept.bytes > config.max_result || kept.bytes > config.budget {
            return false;
        }
        if let Some(entry) = self.entries.get(&key) {
            let wider = (entry.kept.columns.iter()).any(|column| !kept.columns.contains(column));
            if wider {
                return false;
            }
            self.bytes -= entry.kept.bytes;
        }
        self.tick += 1;
        self.bytes += kept.bytes;
        self.entries.insert(
            key.clone(),
            Entry {
                kept,
                used: self.tick,
            },
        );
        while self.bytes > config.budget {
            let Some(oldest) = (self.entries.iter())
                .filter(|(candidate, _)| **candidate != key)
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            let entry = self
                .entries
                .remove(&oldest)
                .expect("the oldest entry is kept");
            self.bytes -= entry.kept.bytes;
        }
        true
    }

    /// The computation of `key` that computes every column of `columns`.
    pub fn flight(&self, key: &NodeKey, columns: &BTreeSet<usize>) -> Option<Arc<Flight>> {
        (self.flights.get(&Some(key.clone()))?.iter())
            .find(|flight| columns.is_subset(&flight.columns))
            .cloned()
    }

    /// The columns of the computations of `key`.
    pub fn flight_columns(&self, key: &NodeKey) -> BTreeSet<usize> {
        (self.flights.get(&Some(key.clone())).into_iter().flatten())
            .flat_map(|flight| flight.columns.iter().copied())
            .collect()
    }

    /// Remove the computation `id` of `key`.
    pub fn land(&mut self, key: &Option<NodeKey>, id: u64) {
        if let Some(flights) = self.flights.get_mut(key) {
            flights.retain(|candidate| candidate.id != id);
            if flights.is_empty() {
                self.flights.remove(key);
            }
        }
    }
}
