#![deny(missing_docs)]

//! Physical-plan result caching for DataFusion execution.
//!
//! This crate caches the results of physical-plan subtrees across repeated
//! physical planning requests on a long-lived [`SessionContext`]. After
//! DataFusion's physical optimization, [`EvaluationCachePlanner`] walks the
//! plan bottom-up computing a safety-gated fingerprint per subtree, replaces
//! subtrees whose fingerprints have committed cache entries with
//! `CacheReadExec`, and wraps admitted cache misses with `CacheWriteExec` — a
//! tee that stages record batches during normal execution and commits the
//! entry atomically when every partition completes.
//!
//! The pipeline is:
//!
//! ```text
//! LogicalPlan -> logical opt -> physical planning -> physical optimization
//!   -> EvaluationCachePlanner (this crate)
//!        - bottom-up subtree fingerprints (proto-bytes hash, safety-gated)
//!        - non-overlapping hit frontier  -> CacheReadExec
//!        - separate write frontier       -> CacheWriteExec (tee + commit)
//!   -> execute (unchanged semantics)
//! ```
//!
//! The cache lives in memory, is byte-budgeted with least-recently-used
//! eviction, and admits an entry only after its fingerprint has been observed
//! repeatedly. Subtrees containing runtime-mutated expressions (dynamic
//! filters), volatile expressions, or unbounded sources are excluded from
//! both the read and the write frontier.
//!
//! Design document: `avenger-chart/docs/future-work/physical-plan-evaluation-cache.md`.
//!
//! [`SessionContext`]: datafusion::prelude::SessionContext
//! [`EvaluationCachePlanner`]: crate::planner::EvaluationCachePlanner

pub mod clock;
pub mod config;
pub mod exec;
pub mod fingerprint;
pub mod planner;
pub mod store;

pub use clock::{CacheClock, StdClock};
pub use config::{CacheMetricsSnapshot, EvaluationCacheConfig};
pub use exec::{CacheReadExec, CacheWriteExec};
pub use fingerprint::{
    CacheVersion, CacheVersionProvider, ExclusionReason, FingerprintContext, FingerprintOutcome,
    PhysicalCacheKey, PhysicalPlanFingerprinter, PlanFingerprint, ProtoFingerprinter,
};
pub use planner::EvaluationCachePlanner;
pub use store::{
    AdmissionDecision, CacheCandidate, CacheEntryRef, CacheLookup, CacheWaitHandle,
    CacheWriteHandle, DeclineReason, EvaluationCache, PlanObservation,
};
