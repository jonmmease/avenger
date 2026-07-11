//! Safety-gated physical-plan fingerprinting.
//!
//! The cache key is a fingerprint of a physical subtree, not a logical query
//! identity. Fingerprints are computed bottom-up and node-locally: each node
//! is serialized through `datafusion-proto` with its children replaced by
//! [`EmptyExec`] placeholders, then combined with its children's
//! fingerprints. Memory-source leaves are never proto-serialized (their proto
//! encoding embeds the full partition data); they hash their structural
//! fields plus a [`CacheVersion`] resolved through the registered
//! [`CacheVersionProvider`] chain, falling back to a memoized content hash of
//! the partitions.
//!
//! Fingerprinting is *fail-safe*: any subtree containing a runtime-mutated
//! (dynamic filter) expression, a volatile expression, an unbounded source,
//! or a node that cannot be serialized is [`FingerprintOutcome::Excluded`],
//! and exclusion propagates to every ancestor. An excluded subtree can never
//! produce a false cache hit; it simply never participates.
//!
//! The dynamic-filter gate deserves emphasis. `datafusion-proto`
//! serialization does NOT error on runtime-mutated expressions: a
//! `DynamicFilterPhysicalExpr` serializes its runtime snapshot plus a
//! process-global id (a silent never-hit), and a join-pushed
//! `HashTableLookupExpr` serializes as a `lit(true)` normalization (a
//! potential FALSE hit). The only safe gate is the deep pre-serialization
//! check in [`GuardedProtoConverter`]: any expression whose recursive
//! snapshot generation is non-zero, or that contains a volatile node, fails
//! serialization with a typed marker error before any proto bytes exist.
//! The check must be deep because the default converter recurses through
//! itself for child expressions, bypassing the interception hook.
//!
//! Fingerprints are process-lifetime only: the cache is in-memory and never
//! persisted, so hash stability across processes or compiler versions is a
//! non-goal.
//!
//! [`EmptyExec`]: datafusion::physical_plan::empty::EmptyExec

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use arrow::array::{Array, ArrayRef};
use arrow::datatypes::{Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use datafusion::config::ConfigOptions;
use datafusion::datasource::memory::MemorySourceConfig;
use datafusion::datasource::source::{DataSource, DataSourceExec};
use datafusion::physical_expr_common::physical_expr::{is_volatile, snapshot_generation};
use datafusion::physical_plan::empty::EmptyExec;
use datafusion::physical_plan::execution_plan::{Boundedness, ExecutionPlanProperties};
use datafusion::physical_plan::{ExecutionPlan, PhysicalExpr};
use datafusion_common::{DataFusionError, Result, internal_err};
use datafusion_proto::physical_plan::{
    DefaultPhysicalExtensionCodec, DefaultPhysicalProtoConverter, PhysicalExtensionCodec,
    PhysicalPlanDecodeContext, PhysicalProtoConverterExtension,
};
use datafusion_proto::protobuf;

/// Crate-level fingerprint format version.
///
/// Bump on any change to what a fingerprint hashes so stale keys from an
/// older in-process cache generation can never collide with new ones.
pub(crate) const KEY_VERSION: u32 = 1;

/// 128-bit fingerprint of a physical plan subtree.
///
/// Covers the node's own structure, its children's fingerprints, source
/// versions, the DataFusion version, and a subset of the execution
/// configuration. See the module docs for what is deliberately excluded.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct PlanFingerprint(pub u128);

/// Stable version identity for a source, used to invalidate cached results
/// when the underlying data changes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CacheVersion(pub u128);

/// Resolves version identity for source leaves of a physical plan.
///
/// Providers are registered on the cache and consulted in registration
/// order. The first `Some` wins. For memory sources with no provider claim,
/// the built-in fallback is a content hash of the partitions.
///
/// This is a physical-layer resolver rather than a method on table sources
/// because `TableProvider`s are not reachable from physical plans.
pub trait CacheVersionProvider: Send + Sync {
    /// Stable version for a source node, or `None` if this provider does not
    /// recognize the node.
    fn source_version(&self, plan: &dyn ExecutionPlan) -> Option<CacheVersion>;
}

/// Why a subtree was excluded from fingerprinting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExclusionReason {
    /// The subtree contains a runtime-mutated expression (dynamic filter).
    /// Caching under one would replay data pruned for a different consumer.
    DynamicExpr,
    /// The subtree contains a volatile expression such as `random()`.
    VolatileExpr,
    /// The subtree is not bounded; a cache entry can never be complete.
    Unbounded,
    /// The node could not be fingerprinted (unserializable, or child
    /// replacement failed on a subtree containing a memory source).
    UnsupportedNode {
        /// Human-readable diagnostic.
        message: String,
    },
    /// A child subtree was excluded; exclusion propagates upward.
    ExcludedChild,
}

/// Result of fingerprinting one subtree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FingerprintOutcome {
    /// The subtree is safe to cache under this fingerprint.
    Cacheable(PlanFingerprint),
    /// The subtree must not participate in caching.
    Excluded(ExclusionReason),
}

/// Full cache key: subtree fingerprint plus output-property fingerprints.
///
/// A committed entry is only served when every component matches, so a hit
/// can substitute a `CacheReadExec` that reports the same schema,
/// partitioning, ordering, and boundedness as the subtree it replaces.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PhysicalCacheKey {
    /// Fingerprint of the plan subtree.
    pub subtree_fingerprint: PlanFingerprint,
    /// Hash of the output schema (fields plus sorted metadata).
    pub schema_fingerprint: u64,
    /// Hash of the output partitioning's display form.
    pub partitioning_fingerprint: u64,
    /// Hash of the output ordering's display form (0 when unordered).
    pub ordering_fingerprint: u64,
    /// Whether the subtree is bounded. Unbounded subtrees are excluded from
    /// caching, so committed keys always carry `true`; the field keeps the
    /// key self-describing.
    pub bounded: bool,
}

/// Build the full cache key for a plan whose subtree fingerprint is known.
pub(crate) fn cache_key_for(
    plan: &Arc<dyn ExecutionPlan>,
    fingerprint: PlanFingerprint,
) -> PhysicalCacheKey {
    let mut schema_hasher = DefaultHasher::new();
    hash_schema(plan.schema().as_ref(), &mut schema_hasher);

    let mut partitioning_hasher = DefaultHasher::new();
    format!("{}", plan.output_partitioning()).hash(&mut partitioning_hasher);

    let ordering_fingerprint = match plan.output_ordering() {
        Some(ordering) => {
            let mut hasher = DefaultHasher::new();
            format!("{ordering}").hash(&mut hasher);
            hasher.finish()
        }
        None => 0,
    };

    PhysicalCacheKey {
        subtree_fingerprint: fingerprint,
        schema_fingerprint: schema_hasher.finish(),
        partitioning_fingerprint: partitioning_hasher.finish(),
        ordering_fingerprint,
        bounded: matches!(plan.boundedness(), Boundedness::Bounded),
    }
}

/// Ambient inputs to a fingerprint pass.
///
/// Constructed by the planner once per rewrite. Carries a hash of the
/// execution-configuration subset that affects results, the registered
/// version providers, and the memory-source content-hash memo.
#[derive(Clone)]
pub struct FingerprintContext {
    pub(crate) config_fingerprint: u64,
    pub(crate) providers: Vec<Arc<dyn CacheVersionProvider>>,
    pub(crate) memo: Arc<ContentHashMemo>,
    /// Nodes whose child replacement failed, forcing whole-subtree
    /// serialization (see gotcha: order/partition-validating constructors).
    pub(crate) whole_subtree_fallbacks: Arc<AtomicU64>,
}

impl std::fmt::Debug for FingerprintContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FingerprintContext")
            .field("config_fingerprint", &self.config_fingerprint)
            .field("providers", &self.providers.len())
            .finish()
    }
}

impl FingerprintContext {
    pub(crate) fn new(
        config_fingerprint: u64,
        providers: Vec<Arc<dyn CacheVersionProvider>>,
        memo: Arc<ContentHashMemo>,
    ) -> Self {
        Self {
            config_fingerprint,
            providers,
            memo,
            whole_subtree_fallbacks: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Hash of the execution-configuration subset included in fingerprints.
    pub fn config_fingerprint(&self) -> u64 {
        self.config_fingerprint
    }

    fn provider_version(&self, plan: &dyn ExecutionPlan) -> Option<CacheVersion> {
        self.providers.iter().find_map(|p| p.source_version(plan))
    }
}

/// Hash of the execution-configuration subset that participates in
/// fingerprints.
///
/// Included, because they change results or physical compatibility:
/// `execution.target_partitions` (partitioning of repartition outputs),
/// `execution.batch_size` (batch boundaries in replayed output),
/// `execution.time_zone` (timestamp semantics), and
/// `optimizer.enable_dynamic_filter_pushdown` (plan shape near scans).
///
/// Everything else is a deliberate under-approximation: options that change
/// results outside this subset (same-name UDF re-registration, catalog
/// content changes behind an unchanged table identity) are the host's
/// responsibility via registration discipline and source versions.
pub(crate) fn config_fingerprint(options: &ConfigOptions) -> u64 {
    let mut hasher = DefaultHasher::new();
    options.execution.target_partitions.hash(&mut hasher);
    options.execution.batch_size.hash(&mut hasher);
    options.execution.time_zone.hash(&mut hasher);
    options
        .optimizer
        .enable_dynamic_filter_pushdown
        .hash(&mut hasher);
    hasher.finish()
}

/// Computes fingerprints for physical plan nodes.
///
/// The default implementation ([`ProtoFingerprinter`]) hashes
/// `datafusion-proto` bytes with the safety gates described in the module
/// docs. Implement this trait to support extension `ExecutionPlan` nodes the
/// proto codec cannot encode.
pub trait PhysicalPlanFingerprinter: Send + Sync {
    /// Fingerprint `plan` given its children's already-computed
    /// fingerprints. Called bottom-up; `children` are in `plan.children()`
    /// order and are all `Cacheable` (the tree walk short-circuits excluded
    /// children before calling).
    fn fingerprint(
        &self,
        plan: &Arc<dyn ExecutionPlan>,
        children: &[PlanFingerprint],
        ctx: &FingerprintContext,
    ) -> FingerprintOutcome;
}

/// The default proto-bytes fingerprinter.
#[derive(Debug, Default)]
pub struct ProtoFingerprinter;

impl PhysicalPlanFingerprinter for ProtoFingerprinter {
    fn fingerprint(
        &self,
        plan: &Arc<dyn ExecutionPlan>,
        children: &[PlanFingerprint],
        ctx: &FingerprintContext,
    ) -> FingerprintOutcome {
        node_fingerprint(plan, children, ctx)
    }
}

/// A fingerprinted plan node with its fingerprinted children, mirroring the
/// plan's own tree shape.
#[derive(Debug)]
pub(crate) struct FingerprintedNode {
    pub(crate) outcome: FingerprintOutcome,
    pub(crate) children: Vec<FingerprintedNode>,
}

/// Fingerprint an entire plan bottom-up. Any excluded child makes every
/// ancestor `Excluded(ExcludedChild)` without invoking the fingerprinter on
/// it.
pub(crate) fn fingerprint_tree(
    plan: &Arc<dyn ExecutionPlan>,
    fingerprinter: &dyn PhysicalPlanFingerprinter,
    ctx: &FingerprintContext,
) -> FingerprintedNode {
    let children: Vec<FingerprintedNode> = plan
        .children()
        .into_iter()
        .map(|child| fingerprint_tree(child, fingerprinter, ctx))
        .collect();

    let mut child_fps = Vec::with_capacity(children.len());
    let mut any_excluded = false;
    for child in &children {
        match &child.outcome {
            FingerprintOutcome::Cacheable(fp) => child_fps.push(*fp),
            FingerprintOutcome::Excluded(_) => any_excluded = true,
        }
    }

    let outcome = if any_excluded {
        FingerprintOutcome::Excluded(ExclusionReason::ExcludedChild)
    } else {
        fingerprinter.fingerprint(plan, &child_fps, ctx)
    };

    FingerprintedNode { outcome, children }
}

/// Fingerprint one node given its children's fingerprints.
pub(crate) fn node_fingerprint(
    plan: &Arc<dyn ExecutionPlan>,
    children: &[PlanFingerprint],
    ctx: &FingerprintContext,
) -> FingerprintOutcome {
    if !matches!(plan.boundedness(), Boundedness::Bounded) {
        return FingerprintOutcome::Excluded(ExclusionReason::Unbounded);
    }

    let mut wide = WideHasher::new();
    wide.hash_value(&KEY_VERSION);
    wide.write(datafusion::DATAFUSION_VERSION.as_bytes());
    wide.hash_value(&ctx.config_fingerprint);

    // Memory-source leaves are never proto-serialized: their proto encoding
    // embeds the full partition data, which would make every fingerprint
    // walk O(data).
    if let Some(exec) = plan.downcast_ref::<DataSourceExec>() {
        if let Some(memory) = exec.data_source().downcast_ref::<MemorySourceConfig>() {
            if let Err(reason) = hash_memory_leaf(plan, memory, ctx, &mut wide) {
                return FingerprintOutcome::Excluded(reason);
            }
            for fp in children {
                wide.hash_value(&fp.0);
            }
            return FingerprintOutcome::Cacheable(PlanFingerprint(wide.finish()));
        }
    }

    match node_local_bytes(plan, ctx) {
        Ok(bytes) => {
            wide.write(b"proto-node");
            wide.write(&bytes);
        }
        Err(NodeBytesError::Guard(GuardViolation::Dynamic)) => {
            return FingerprintOutcome::Excluded(ExclusionReason::DynamicExpr);
        }
        Err(NodeBytesError::Guard(GuardViolation::Volatile)) => {
            return FingerprintOutcome::Excluded(ExclusionReason::VolatileExpr);
        }
        Err(NodeBytesError::Unsupported(message)) => {
            return FingerprintOutcome::Excluded(ExclusionReason::UnsupportedNode { message });
        }
    }

    // Non-memory leaves may still carry a provider-supplied source version
    // (hashed alongside the proto bytes, which for file scans already carry
    // paths, sizes, and mtimes).
    if children.is_empty() {
        if let Some(version) = ctx.provider_version(plan.as_ref()) {
            wide.write(b"leaf-version");
            wide.hash_value(&version.0);
        }
    }

    for fp in children {
        wide.hash_value(&fp.0);
    }
    FingerprintOutcome::Cacheable(PlanFingerprint(wide.finish()))
}

/// Hash a memory-source leaf: schema (fields plus sorted metadata),
/// projection, sort information, fetch, and the source version (provider
/// chain, else memoized content hash). Never touches proto.
fn hash_memory_leaf(
    plan: &Arc<dyn ExecutionPlan>,
    memory: &MemorySourceConfig,
    ctx: &FingerprintContext,
    wide: &mut WideHasher,
) -> std::result::Result<(), ExclusionReason> {
    wide.write(b"memory-leaf");

    let schema = memory.original_schema();
    let mut schema_hasher = DefaultHasher::new();
    hash_schema(schema.as_ref(), &mut schema_hasher);
    wide.hash_value(&schema_hasher.finish());

    wide.hash_value(memory.projection());
    for ordering in memory.sort_information() {
        wide.write(format!("{ordering}").as_bytes());
    }
    wide.hash_value(&memory.fetch());

    let version = match ctx.provider_version(plan.as_ref()) {
        Some(version) => version,
        None => ctx
            .memo
            .version_for(memory.partitions(), &schema)
            .map_err(|err| ExclusionReason::UnsupportedNode {
                message: format!("memory partition content hash failed: {err}"),
            })?,
    };
    wide.hash_value(&version.0);
    Ok(())
}

/// Hash schema fields, field metadata (sorted), and schema metadata
/// (sorted). Sorting sidesteps `HashMap` iteration-order instability.
fn hash_schema(schema: &Schema, hasher: &mut DefaultHasher) {
    for field in schema.fields() {
        field.name().hash(hasher);
        format!("{:?}", field.data_type()).hash(hasher);
        field.is_nullable().hash(hasher);
        let mut metadata: Vec<_> = field.metadata().iter().collect();
        metadata.sort();
        for (key, value) in metadata {
            key.hash(hasher);
            value.hash(hasher);
        }
    }
    let mut metadata: Vec<_> = schema.metadata().iter().collect();
    metadata.sort();
    for (key, value) in metadata {
        key.hash(hasher);
        value.hash(hasher);
    }
}

enum NodeBytesError {
    Guard(GuardViolation),
    Unsupported(String),
}

/// Serialize one node with its children replaced by [`EmptyExec`]
/// placeholders, so the bytes cover the node's own structure only.
///
/// Placeholder schemas are stripped of field and schema metadata: the
/// child's real identity is covered by its own fingerprint, and arrow
/// metadata maps proto-encode in `HashMap` iteration order, which is not
/// stable across separately constructed schemas (a miss-only instability;
/// see plan gotcha 3).
///
/// Nodes whose constructors reject `EmptyExec` children (partition- or
/// order-validating nodes) fall back to whole-subtree serialization — unless
/// the subtree contains a memory source, in which case the fallback would be
/// O(data) and the node is excluded instead.
fn node_local_bytes(
    plan: &Arc<dyn ExecutionPlan>,
    ctx: &FingerprintContext,
) -> std::result::Result<Vec<u8>, NodeBytesError> {
    let children = plan.children();
    let node_local: Arc<dyn ExecutionPlan> = if children.is_empty() {
        Arc::clone(plan)
    } else {
        let placeholders: Vec<Arc<dyn ExecutionPlan>> = children
            .iter()
            .map(|child| {
                Arc::new(EmptyExec::new(strip_schema_metadata(&child.schema())))
                    as Arc<dyn ExecutionPlan>
            })
            .collect();
        match Arc::clone(plan).with_new_children(placeholders) {
            Ok(node_local) => node_local,
            Err(_) if contains_memory_source(plan) => {
                return Err(NodeBytesError::Unsupported(format!(
                    "child replacement failed on '{}' over a memory source; \
                     whole-subtree fallback would serialize the data",
                    plan.name()
                )));
            }
            Err(_) => {
                ctx.whole_subtree_fallbacks.fetch_add(1, Ordering::Relaxed);
                Arc::clone(plan)
            }
        }
    };

    match datafusion_proto::bytes::physical_plan_to_bytes_with_proto_converter(
        node_local,
        &DefaultPhysicalExtensionCodec {},
        &GuardedProtoConverter,
    ) {
        Ok(bytes) => Ok(bytes.to_vec()),
        Err(err) => match find_guard_violation(&err) {
            Some(violation) => Err(NodeBytesError::Guard(violation)),
            None => Err(NodeBytesError::Unsupported(format!(
                "proto serialization failed for '{}': {err}",
                plan.name()
            ))),
        },
    }
}

fn strip_schema_metadata(schema: &SchemaRef) -> SchemaRef {
    let fields: Vec<arrow::datatypes::Field> = schema
        .fields()
        .iter()
        .map(|field| {
            arrow::datatypes::Field::new(
                field.name(),
                field.data_type().clone(),
                field.is_nullable(),
            )
        })
        .collect();
    Arc::new(Schema::new(fields))
}

fn contains_memory_source(plan: &Arc<dyn ExecutionPlan>) -> bool {
    if let Some(exec) = plan.downcast_ref::<DataSourceExec>() {
        if exec
            .data_source()
            .downcast_ref::<MemorySourceConfig>()
            .is_some()
        {
            return true;
        }
    }
    plan.children().iter().any(|c| contains_memory_source(c))
}

/// What the serialization guard tripped on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuardViolation {
    Dynamic,
    Volatile,
}

/// Typed marker error threaded through the proto converter so exclusion
/// reasons never depend on string matching.
#[derive(Debug)]
struct GuardError(GuardViolation);

impl std::fmt::Display for GuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            GuardViolation::Dynamic => f.write_str(
                "subtree contains a runtime-mutated (dynamic filter) expression; \
                 excluded from caching",
            ),
            GuardViolation::Volatile => {
                f.write_str("subtree contains a volatile expression; excluded from caching")
            }
        }
    }
}

impl std::error::Error for GuardError {}

fn find_guard_violation(err: &DataFusionError) -> Option<GuardViolation> {
    match err {
        DataFusionError::Context(_, inner) => find_guard_violation(inner),
        DataFusionError::Shared(inner) => find_guard_violation(inner),
        DataFusionError::External(external) => {
            external.downcast_ref::<GuardError>().map(|guard| guard.0)
        }
        _ => None,
    }
}

/// Encode-only proto converter with the deep dynamic/volatile safety gate.
///
/// `physical_expr_to_proto` checks the WHOLE expression tree (recursive
/// [`snapshot_generation`] and [`is_volatile`]) before delegating to the
/// default converter; a shallow check would miss nested dynamic or volatile
/// expressions because the default converter recurses through itself, not
/// through this hook, for child expressions.
struct GuardedProtoConverter;

impl PhysicalProtoConverterExtension for GuardedProtoConverter {
    fn proto_to_execution_plan(
        &self,
        _proto: &protobuf::PhysicalPlanNode,
        _ctx: &PhysicalPlanDecodeContext<'_>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        internal_err!("GuardedProtoConverter is encode-only (fingerprinting never decodes)")
    }

    fn proto_to_physical_expr(
        &self,
        _proto: &protobuf::PhysicalExprNode,
        _input_schema: &Schema,
        _ctx: &PhysicalPlanDecodeContext<'_>,
    ) -> Result<Arc<dyn PhysicalExpr>> {
        internal_err!("GuardedProtoConverter is encode-only (fingerprinting never decodes)")
    }

    fn execution_plan_to_proto(
        &self,
        plan: &Arc<dyn ExecutionPlan>,
        codec: &dyn PhysicalExtensionCodec,
    ) -> Result<protobuf::PhysicalPlanNode> {
        protobuf::PhysicalPlanNode::try_from_physical_plan_with_converter(
            Arc::clone(plan),
            codec,
            self,
        )
    }

    fn physical_expr_to_proto(
        &self,
        expr: &Arc<dyn PhysicalExpr>,
        codec: &dyn PhysicalExtensionCodec,
    ) -> Result<protobuf::PhysicalExprNode> {
        if snapshot_generation(expr) != 0 {
            return Err(DataFusionError::External(Box::new(GuardError(
                GuardViolation::Dynamic,
            ))));
        }
        if is_volatile(expr) {
            return Err(DataFusionError::External(Box::new(GuardError(
                GuardViolation::Volatile,
            ))));
        }
        DefaultPhysicalProtoConverter {}.physical_expr_to_proto(expr, codec)
    }
}

const MEMO_CAPACITY: usize = 256;

/// Memoized content hashes for memory-source partitions.
///
/// Keyed by the identity (`Arc` pointers) of every column array across the
/// partitions, guarded by weak-reference liveness so a reused allocation
/// address can never satisfy a stale key (ABA safety: if the original arrays
/// were dropped, the weak refs are dead and the entry misses). Bounded;
/// overflow evicts the oldest insertion.
#[derive(Debug, Default)]
pub(crate) struct ContentHashMemo {
    inner: Mutex<MemoInner>,
}

#[derive(Debug, Default)]
struct MemoInner {
    entries: HashMap<u64, MemoEntry>,
    insertions: u64,
    hits: u64,
}

#[derive(Debug)]
struct MemoEntry {
    arrays: Vec<Weak<dyn Array>>,
    version: CacheVersion,
    inserted: u64,
}

impl ContentHashMemo {
    /// Content-hash version for a memory source's partitions, memoized by
    /// array identity.
    pub(crate) fn version_for(
        &self,
        partitions: &[Vec<RecordBatch>],
        schema: &SchemaRef,
    ) -> std::result::Result<CacheVersion, arrow::error::ArrowError> {
        let mut key_hasher = DefaultHasher::new();
        let mut candidate: Vec<ArrayRef> = Vec::new();
        for partition in partitions {
            for batch in partition {
                for column in batch.columns() {
                    (Arc::as_ptr(column) as *const u8 as usize).hash(&mut key_hasher);
                    candidate.push(Arc::clone(column));
                }
            }
            // Partition boundaries are part of identity.
            0xB0_u8.hash(&mut key_hasher);
        }
        candidate.len().hash(&mut key_hasher);
        let key = key_hasher.finish();

        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(entry) = inner.entries.get(&key) {
                let alive = entry.arrays.len() == candidate.len()
                    && entry.arrays.iter().zip(&candidate).all(|(weak, current)| {
                        match weak.upgrade() {
                            Some(stored) => Arc::ptr_eq(&stored, current),
                            None => false,
                        }
                    });
                if alive {
                    let version = entry.version;
                    inner.hits += 1;
                    return Ok(version);
                }
                inner.entries.remove(&key);
            }
        }

        // Hash outside the lock (never hold a cache lock while hashing
        // content).
        let version = content_hash(partitions, schema)?;

        let mut inner = self.inner.lock().unwrap();
        if inner.entries.len() >= MEMO_CAPACITY {
            if let Some((&oldest, _)) = inner.entries.iter().min_by_key(|(_, entry)| entry.inserted)
            {
                inner.entries.remove(&oldest);
            }
        }
        inner.insertions += 1;
        let inserted = inner.insertions;
        inner.entries.insert(
            key,
            MemoEntry {
                arrays: candidate.iter().map(Arc::downgrade).collect(),
                version,
                inserted,
            },
        );
        Ok(version)
    }

    /// Number of memoized (hash-skipping) lookups; used by tests.
    #[cfg(test)]
    pub(crate) fn hits(&self) -> u64 {
        self.inner.lock().unwrap().hits
    }
}

/// Content hash of memory partitions via Arrow IPC streaming into the
/// hasher; one pass feeds both 64-bit lanes of the wide hash.
fn content_hash(
    partitions: &[Vec<RecordBatch>],
    schema: &SchemaRef,
) -> std::result::Result<CacheVersion, arrow::error::ArrowError> {
    struct HashingWrite<'a>(&'a mut WideHasher);
    impl std::io::Write for HashingWrite<'_> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut wide = WideHasher::new();
    for partition in partitions {
        {
            let mut writer =
                arrow::ipc::writer::StreamWriter::try_new(HashingWrite(&mut wide), schema)?;
            for batch in partition {
                writer.write(batch)?;
            }
            writer.finish()?;
        }
        wide.write(b"partition-boundary");
    }
    Ok(CacheVersion(wide.finish()))
}

/// Widen hashing to 128 bits by running the same byte stream through two
/// [`DefaultHasher`]s seeded with distinct domain-separation prefixes.
/// `DefaultHasher::new()` is deterministic (unlike `RandomState`), which is
/// what makes fingerprints stable across separately constructed plans within
/// one process.
pub(crate) struct WideHasher {
    lo: DefaultHasher,
    hi: DefaultHasher,
}

impl WideHasher {
    pub(crate) fn new() -> Self {
        let mut lo = DefaultHasher::new();
        let mut hi = DefaultHasher::new();
        0xA5A5_5A5A_u64.hash(&mut lo);
        0xC3C3_3C3C_u64.hash(&mut hi);
        Self { lo, hi }
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) {
        self.lo.write(bytes);
        self.hi.write(bytes);
    }

    pub(crate) fn hash_value<T: Hash>(&mut self, value: &T) {
        value.hash(&mut self.lo);
        value.hash(&mut self.hi);
    }

    pub(crate) fn finish(&self) -> u128 {
        ((self.hi.finish() as u128) << 64) | (self.lo.finish() as u128)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Int64Array, StringArray};
    use arrow::datatypes::{DataType, Field};
    use datafusion::datasource::MemTable;
    use datafusion::prelude::SessionContext;

    fn ctx_with_table(name: &str, values: &[i64]) -> SessionContext {
        let ctx = SessionContext::new();
        register_table(&ctx, name, values);
        ctx
    }

    fn register_table(ctx: &SessionContext, name: &str, values: &[i64]) {
        let field_metadata: HashMap<String, String> = [
            ("unit".to_string(), "widgets".to_string()),
            ("origin".to_string(), "test".to_string()),
            ("zeta".to_string(), "last".to_string()),
        ]
        .into();
        let schema_metadata: HashMap<String, String> = [
            ("table.note".to_string(), "fixture".to_string()),
            ("table.rev".to_string(), "1".to_string()),
            ("a.first".to_string(), "entry".to_string()),
        ]
        .into();
        let schema = Arc::new(Schema::new_with_metadata(
            vec![
                Field::new("v", DataType::Int64, false).with_metadata(field_metadata),
                Field::new("k", DataType::Utf8, false),
            ],
            schema_metadata,
        ));
        let keys: Vec<String> = values.iter().map(|v| format!("k{}", v % 3)).collect();
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(values.to_vec())),
                Arc::new(StringArray::from(keys)),
            ],
        )
        .unwrap();
        let table = MemTable::try_new(schema, vec![vec![batch]]).unwrap();
        ctx.register_table(name, Arc::new(table)).unwrap();
    }

    async fn plan_for(ctx: &SessionContext, sql: &str) -> Arc<dyn ExecutionPlan> {
        ctx.sql(sql)
            .await
            .unwrap()
            .create_physical_plan()
            .await
            .unwrap()
    }

    fn test_ctx() -> FingerprintContext {
        FingerprintContext::new(0, Vec::new(), Arc::new(ContentHashMemo::default()))
    }

    /// Flatten (node name, outcome) pairs in pre-order for structural
    /// comparison between two plans of the same shape.
    fn flatten(
        plan: &Arc<dyn ExecutionPlan>,
        node: &FingerprintedNode,
        out: &mut Vec<(String, FingerprintOutcome)>,
    ) {
        out.push((plan.name().to_string(), node.outcome.clone()));
        for (child_plan, child_node) in plan.children().iter().zip(&node.children) {
            flatten(child_plan, child_node, out);
        }
    }

    fn fingerprints_of(
        plan: &Arc<dyn ExecutionPlan>,
        ctx: &FingerprintContext,
    ) -> Vec<(String, FingerprintOutcome)> {
        let tree = fingerprint_tree(plan, &ProtoFingerprinter, ctx);
        let mut out = Vec::new();
        flatten(plan, &tree, &mut out);
        out
    }

    #[tokio::test]
    async fn determinism_same_sql_twice() {
        let ctx = ctx_with_table("t", &[1, 2, 3, 4, 5, 6]);
        let fctx = test_ctx();
        let sql = "SELECT k, sum(v) FROM t WHERE v > 1 GROUP BY k";
        let plan_a = plan_for(&ctx, sql).await;
        let plan_b = plan_for(&ctx, sql).await;
        let fps_a = fingerprints_of(&plan_a, &fctx);
        let fps_b = fingerprints_of(&plan_b, &fctx);
        assert_eq!(fps_a.len(), fps_b.len());
        for ((name_a, out_a), (name_b, out_b)) in fps_a.iter().zip(fps_b.iter()) {
            assert_eq!(name_a, name_b);
            assert_eq!(
                out_a, out_b,
                "node '{name_a}' fingerprint not deterministic across replans \
                 (schema carries multi-entry metadata; see plan gotcha 3)"
            );
            assert!(
                matches!(out_a, FingerprintOutcome::Cacheable(_)),
                "node '{name_a}' unexpectedly excluded: {out_a:?}"
            );
        }
    }

    #[tokio::test]
    async fn sensitivity_literal_change_localized() {
        let ctx = ctx_with_table("t", &[1, 2, 3, 4, 5, 6]);
        let fctx = test_ctx();
        let plan_a = plan_for(&ctx, "SELECT v FROM t WHERE v > 5").await;
        let plan_b = plan_for(&ctx, "SELECT v FROM t WHERE v > 6").await;
        let fps_a = fingerprints_of(&plan_a, &fctx);
        let fps_b = fingerprints_of(&plan_b, &fctx);
        assert_eq!(fps_a.len(), fps_b.len(), "plan shapes diverged");

        let mut saw_scan = false;
        let mut saw_difference = false;
        for ((name, out_a), (_, out_b)) in fps_a.iter().zip(fps_b.iter()) {
            if name == "DataSourceExec" {
                saw_scan = true;
                assert_eq!(
                    out_a, out_b,
                    "scan fingerprint must not change with the literal"
                );
            }
        }
        // Roots must differ (they contain the literal).
        saw_difference |= fps_a[0].1 != fps_b[0].1;
        assert!(saw_scan, "expected a DataSourceExec in the plan");
        assert!(
            saw_difference,
            "literal change must change the root fingerprint"
        );
    }

    #[tokio::test]
    async fn volatile_filter_excluded_scan_cacheable() {
        let ctx = ctx_with_table("t", &[1, 2, 3, 4, 5, 6]);
        let fctx = test_ctx();
        let plan = plan_for(&ctx, "SELECT v FROM t WHERE random() < 2.0").await;
        let fps = fingerprints_of(&plan, &fctx);

        let filter = fps
            .iter()
            .find(|(name, _)| name == "FilterExec")
            .expect("plan should contain a FilterExec");
        assert_eq!(
            filter.1,
            FingerprintOutcome::Excluded(ExclusionReason::VolatileExpr),
            "volatile filter must be excluded"
        );
        let scan = fps
            .iter()
            .find(|(name, _)| name == "DataSourceExec")
            .expect("plan should contain a scan");
        assert!(
            matches!(scan.1, FingerprintOutcome::Cacheable(_)),
            "scan below the volatile filter stays cacheable, got {:?}",
            scan.1
        );
        // The root either IS the filter (VolatileExpr) or sits above it
        // (ExcludedChild); either way it must be excluded.
        assert!(
            matches!(
                fps[0].1,
                FingerprintOutcome::Excluded(
                    ExclusionReason::VolatileExpr | ExclusionReason::ExcludedChild
                )
            ),
            "exclusion must propagate to the root, got {:?}",
            fps[0].1
        );
    }

    #[tokio::test]
    async fn dynamic_filter_topk_excluded_but_serializable() {
        // Write a parquet file so dynamic filter pushdown has a target
        // (memory sources do not accept pushed filters).
        let ctx = ctx_with_table("t", &(0..2000).collect::<Vec<i64>>());
        let dir = std::env::temp_dir().join(format!(
            "avenger-datafusion-cache-topk-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.parquet");
        let path_str = path.to_str().unwrap().to_string();
        ctx.sql("SELECT v, k FROM t")
            .await
            .unwrap()
            .write_parquet(
                &path_str,
                datafusion::dataframe::DataFrameWriteOptions::new(),
                None,
            )
            .await
            .unwrap();
        ctx.register_parquet("p", &path_str, Default::default())
            .await
            .unwrap();

        let plan = plan_for(&ctx, "SELECT v FROM p ORDER BY v LIMIT 10").await;

        // Pin the fixture: the unguarded serializer SUCCEEDS on this plan
        // (exclusion is our gate, not a serializer error).
        datafusion_proto::bytes::physical_plan_to_bytes(Arc::clone(&plan))
            .expect("unguarded proto serialization must succeed on a dynamic-filter plan");

        let fctx = test_ctx();
        let fps = fingerprints_of(&plan, &fctx);
        let dynamic_excluded = fps
            .iter()
            .filter(|(_, out)| {
                matches!(
                    out,
                    FingerprintOutcome::Excluded(ExclusionReason::DynamicExpr)
                )
            })
            .count();
        assert!(
            dynamic_excluded > 0,
            "expected TopK dynamic filter pushdown to fire at default settings; \
             plan outcomes: {fps:?}"
        );
        assert!(
            matches!(fps[0].1, FingerprintOutcome::Excluded(_)),
            "exclusion must reach the root, got {:?}",
            fps[0].1
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn unbounded_streaming_source_excluded() {
        use datafusion::execution::TaskContext;
        use datafusion::physical_plan::stream::EmptyRecordBatchStream;
        use datafusion::physical_plan::streaming::{PartitionStream, StreamingTableExec};

        #[derive(Debug)]
        struct NeverEndingStream {
            schema: SchemaRef,
        }
        impl PartitionStream for NeverEndingStream {
            fn schema(&self) -> &SchemaRef {
                &self.schema
            }
            fn execute(
                &self,
                _ctx: Arc<TaskContext>,
            ) -> datafusion::physical_plan::SendableRecordBatchStream {
                Box::pin(EmptyRecordBatchStream::new(Arc::clone(&self.schema)))
            }
        }

        let schema: SchemaRef =
            Arc::new(Schema::new(vec![Field::new("v", DataType::Int64, false)]));
        let streaming = StreamingTableExec::try_new(
            Arc::clone(&schema),
            vec![Arc::new(NeverEndingStream {
                schema: Arc::clone(&schema),
            })],
            None,
            vec![],
            true, // infinite
            None,
        )
        .unwrap();
        let plan: Arc<dyn ExecutionPlan> = Arc::new(streaming);
        let fctx = test_ctx();
        let outcome = node_fingerprint(&plan, &[], &fctx);
        assert_eq!(
            outcome,
            FingerprintOutcome::Excluded(ExclusionReason::Unbounded)
        );
    }

    #[tokio::test]
    async fn memory_source_content_and_memo() {
        // Same table content, separately planned: stable fingerprints and a
        // memo hit on the second walk.
        let ctx = ctx_with_table("t", &[1, 2, 3]);
        let fctx = test_ctx();
        let plan_a = plan_for(&ctx, "SELECT v, k FROM t").await;
        let plan_b = plan_for(&ctx, "SELECT v, k FROM t").await;
        let fps_a = fingerprints_of(&plan_a, &fctx);
        assert_eq!(fctx.memo.hits(), 0, "first walk computes the hash");
        let fps_b = fingerprints_of(&plan_b, &fctx);
        assert_eq!(fps_a, fps_b);
        assert!(
            fctx.memo.hits() >= 1,
            "second walk must hit the content-hash memo (MemTable batches share arrays)"
        );

        // Different content behind the same name: different scan fingerprint.
        let ctx2 = ctx_with_table("t", &[1, 2, 4]);
        let plan_c = plan_for(&ctx2, "SELECT v, k FROM t").await;
        let fps_c = fingerprints_of(&plan_c, &fctx);
        let scan = |fps: &[(String, FingerprintOutcome)]| {
            fps.iter()
                .find(|(name, _)| name == "DataSourceExec")
                .unwrap()
                .1
                .clone()
        };
        assert_ne!(
            scan(&fps_a),
            scan(&fps_c),
            "content change must change the memory-leaf fingerprint"
        );
    }

    #[tokio::test]
    async fn version_provider_overrides_content_hash() {
        struct FixedVersion(u128);
        impl CacheVersionProvider for FixedVersion {
            fn source_version(&self, plan: &dyn ExecutionPlan) -> Option<CacheVersion> {
                plan.downcast_ref::<DataSourceExec>()
                    .map(|_| CacheVersion(self.0))
            }
        }

        let ctx = ctx_with_table("t", &[1, 2, 3]);
        let plan = plan_for(&ctx, "SELECT v FROM t").await;

        let content_ctx = test_ctx();
        let v7_ctx = FingerprintContext::new(
            0,
            vec![Arc::new(FixedVersion(7))],
            Arc::new(ContentHashMemo::default()),
        );
        let v8_ctx = FingerprintContext::new(
            0,
            vec![Arc::new(FixedVersion(8))],
            Arc::new(ContentHashMemo::default()),
        );

        let content = fingerprints_of(&plan, &content_ctx);
        let v7 = fingerprints_of(&plan, &v7_ctx);
        let v8 = fingerprints_of(&plan, &v8_ctx);
        assert_ne!(
            content, v7,
            "provider version must override the content hash"
        );
        assert_ne!(v7, v8, "bumping the provider version must invalidate");
        assert_eq!(
            v7,
            fingerprints_of(&plan, &v7_ctx),
            "provider-versioned fingerprints are stable"
        );
    }

    #[tokio::test]
    async fn empty_exec_placeholder_gives_child_independent_node_bytes() {
        let ctx = SessionContext::new();
        register_table(&ctx, "t1", &[1, 2, 3]);
        register_table(&ctx, "t2", &[7, 8, 9, 10]);
        let fctx = test_ctx();
        let plan_a = plan_for(&ctx, "SELECT v FROM t1 WHERE v > 5").await;
        let plan_b = plan_for(&ctx, "SELECT v FROM t2 WHERE v > 5").await;

        fn find_filter(plan: &Arc<dyn ExecutionPlan>) -> Option<Arc<dyn ExecutionPlan>> {
            if plan.name() == "FilterExec" {
                return Some(Arc::clone(plan));
            }
            plan.children().iter().find_map(|c| find_filter(c))
        }
        let filter_a = find_filter(&plan_a).expect("t1 plan has a filter");
        let filter_b = find_filter(&plan_b).expect("t2 plan has a filter");

        let bytes_a = node_local_bytes(&filter_a, &fctx).ok().unwrap();
        let bytes_b = node_local_bytes(&filter_b, &fctx).ok().unwrap();
        assert_eq!(
            bytes_a, bytes_b,
            "identical filters over same-schema children must have identical node-local bytes"
        );

        // While the subtree fingerprints still differ via child fingerprints.
        let fps_a = fingerprints_of(&filter_a, &fctx);
        let fps_b = fingerprints_of(&filter_b, &fctx);
        assert_ne!(
            fps_a[0].1, fps_b[0].1,
            "different data must yield different subtrees"
        );
    }

    #[test]
    fn wide_hasher_is_deterministic_and_order_sensitive() {
        let mut a = WideHasher::new();
        a.write(b"hello");
        a.hash_value(&42u64);
        let mut b = WideHasher::new();
        b.write(b"hello");
        b.hash_value(&42u64);
        assert_eq!(a.finish(), b.finish());

        let mut c = WideHasher::new();
        c.hash_value(&42u64);
        c.write(b"hello");
        assert_ne!(a.finish(), c.finish());
    }

    #[test]
    fn config_fingerprint_covers_the_documented_subset() {
        let base = ConfigOptions::default();
        let base_fp = config_fingerprint(&base);

        let mut changed = ConfigOptions::default();
        changed.execution.target_partitions = base.execution.target_partitions + 1;
        assert_ne!(base_fp, config_fingerprint(&changed));

        let mut changed = ConfigOptions::default();
        changed.execution.batch_size += 1;
        assert_ne!(base_fp, config_fingerprint(&changed));

        let mut changed = ConfigOptions::default();
        changed.execution.time_zone = Some("America/New_York".to_string());
        assert_ne!(base_fp, config_fingerprint(&changed));

        let mut changed = ConfigOptions::default();
        changed.optimizer.enable_dynamic_filter_pushdown =
            !base.optimizer.enable_dynamic_filter_pushdown;
        assert_ne!(base_fp, config_fingerprint(&changed));

        assert_eq!(base_fp, config_fingerprint(&ConfigOptions::default()));
    }
}
