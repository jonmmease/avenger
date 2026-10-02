#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

pub mod clock;
pub mod config;
pub mod exec;
pub mod fingerprint;
pub mod planner;
pub mod prepared;
mod reusable_planning;
mod runtime_parameters;
pub mod store;

pub use clock::{CacheClock, StdClock};
pub use config::{CacheMetricsSnapshot, EvaluationCacheConfig};
pub use exec::{CacheReadExec, CacheWriteExec};
pub use fingerprint::{
    CacheVersion, CacheVersionProvider, ExclusionReason, FingerprintContext, FingerprintOutcome,
    PhysicalCacheKey, PhysicalPlanFingerprinter, PlanFingerprint, ProtoFingerprinter,
};
pub use planner::EvaluationCachePlanner;
pub use prepared::{
    ExactResultBypassReason, ExactResultDisposition, PhysicalPlanBypassReason,
    PhysicalPlanDisposition, PreparedPlanBypassReason, PreparedPlanCache, PreparedPlanCacheConfig,
    PreparedPlanCacheMetricsSnapshot, PreparedPlanCollectMetrics, PreparedPlanCollectOutput,
    PreparedPlanDisposition, logical_expr_is_immutable, logical_plan_is_immutable,
};
pub use store::{
    AdmissionDecision, CacheCandidate, CacheEntryRef, CacheLookup, CacheWaitHandle,
    CacheWriteHandle, DeclineReason, EvaluationCache, PlanObservation,
};
