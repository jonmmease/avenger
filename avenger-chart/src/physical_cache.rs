//! Blessed wiring for the physical-plan result cache
//! ([`avenger_datafusion_cache`]).
//!
//! The cache reuses executed physical-plan subtree results across the many
//! DataFusion queries a chart session issues (domain inference, facet
//! slots, mark data, measurement passes) and across repeated evaluations
//! (parameter changes, hot reload). Charts build every evaluation-time
//! `DataFrame` from the live context's state, so installing the cache rule
//! at context build time covers everything a `PlotSession` or
//! `CompiledPlot::evaluate` runs — no chart-side changes are needed for the
//! mechanics, only for policy (gesture hints) and observability (metrics).
//!
//! Install the rule BEFORE any DataFrame exists: DataFrames snapshot the
//! session state at construction, so a host that pre-builds DataFrames from
//! an uncached state and evaluates them later bypasses the cache silently.
//!
//! Store versioning note: scoped store and selection tables need no
//! [`CacheVersionProvider`](avenger_datafusion_cache::CacheVersionProvider)
//! registration. Store batches embed their revision as a column
//! (`__avenger_store_revision`), so the cache's built-in content-hash
//! versioning of memory leaves incorporates the revision by construction:
//! every store patch changes the scanned bytes, which changes the
//! fingerprint. Store provenance is erased at the physical layer (plain
//! memory scans), so a resolver could not attribute scans to stores anyway.
//!
//! UDF identity note: fingerprints identify UDFs and UDAFs by NAME (plus
//! arguments and return type). Avenger registers its scale and transform
//! functions once per process with stable semantics per build, which is
//! exactly the registration discipline the cache requires. Re-registering
//! a DIFFERENT function under an unchanged name mid-session is unsupported
//! while the cache is enabled — call [`EvaluationCache::clear`] if you must.

use std::sync::{Arc, OnceLock};

use avenger_datafusion_cache::EvaluationCachePlanner;
pub use avenger_datafusion_cache::{CacheMetricsSnapshot, EvaluationCache, EvaluationCacheConfig};
use datafusion::execution::session_state::SessionStateBuilder;
use datafusion::prelude::SessionContext;

/// Name of the environment kill switch. `AVENGER_PHYSICAL_CACHE=0` makes
/// [`cached_session_context`] return a plain, rule-free context (strictly
/// zero overhead); any other value or unset leaves the cache enabled for
/// hosts that opted in by calling the helper.
pub const PHYSICAL_CACHE_ENV: &str = "AVENGER_PHYSICAL_CACHE";

fn kill_switch_from(value: Option<&str>) -> bool {
    matches!(value, Some("0"))
}

/// Whether `AVENGER_PHYSICAL_CACHE=0` disables the cache (read once).
pub fn physical_cache_disabled_by_env() -> bool {
    static DISABLED: OnceLock<bool> = OnceLock::new();
    *DISABLED.get_or_init(|| kill_switch_from(std::env::var(PHYSICAL_CACHE_ENV).ok().as_deref()))
}

/// Install the physical result cache on a session-state builder.
///
/// Appends the cache planner as the LAST physical optimizer rule (callers
/// must not add further rules after this — the planner's substitutions are
/// not re-validated) and stores the cache as a [`SessionConfig`] extension
/// so session-side features and [`physical_cache_from_ctx`] can discover it.
///
/// This is the composable primitive; most hosts want
/// [`cached_session_context`].
///
/// [`SessionConfig`]: datafusion::prelude::SessionConfig
pub fn install_physical_cache(
    mut builder: SessionStateBuilder,
    config: EvaluationCacheConfig,
) -> (SessionStateBuilder, Arc<EvaluationCache>) {
    let cache = EvaluationCache::new(config);
    let planner = EvaluationCachePlanner::new(Arc::clone(&cache));

    let config_slot = builder.config();
    let session_config = config_slot.take().unwrap_or_default();
    *config_slot = Some(session_config.with_extension(Arc::clone(&cache)));

    let builder = builder.with_physical_optimizer_rule(Arc::new(planner));
    (builder, cache)
}

/// Build a `SessionContext` with default features and the physical result
/// cache installed, honoring the [`PHYSICAL_CACHE_ENV`] kill switch.
///
/// When the kill switch disables the cache, the returned context has NO
/// cache rule at all (not a dormant one) and the returned cache handle is a
/// detached, never-consulted instance whose metrics stay zero;
/// [`physical_cache_from_ctx`] returns `None` for such a context.
pub fn cached_session_context(
    config: EvaluationCacheConfig,
) -> (SessionContext, Arc<EvaluationCache>) {
    if physical_cache_disabled_by_env() {
        return (
            SessionContext::new(),
            EvaluationCache::new(EvaluationCacheConfig {
                enabled: false,
                ..Default::default()
            }),
        );
    }
    let (builder, cache) =
        install_physical_cache(SessionStateBuilder::new().with_default_features(), config);
    (SessionContext::new_with_state(builder.build()), cache)
}

/// The cache installed on `ctx`'s session config, if any.
///
/// Session-side features (evaluation metrics deltas, the preview-mode
/// observe-only hint) use this lookup; absence means every cache-aware
/// feature silently no-ops.
pub fn physical_cache_from_ctx(ctx: &SessionContext) -> Option<Arc<EvaluationCache>> {
    ctx.state().config().get_extension::<EvaluationCache>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kill_switch_only_zero_disables() {
        assert!(kill_switch_from(Some("0")));
        assert!(!kill_switch_from(Some("1")));
        assert!(!kill_switch_from(Some("")));
        assert!(!kill_switch_from(Some("false")));
        assert!(!kill_switch_from(None));
    }

    #[test]
    fn install_preserves_existing_session_config() {
        use datafusion::prelude::SessionConfig;
        // A pre-set config option must survive extension injection.
        let session_config = SessionConfig::new().with_batch_size(1234);
        let builder = SessionStateBuilder::new()
            .with_default_features()
            .with_config(session_config);
        let (builder, cache) = install_physical_cache(builder, EvaluationCacheConfig::default());
        let ctx = SessionContext::new_with_state(builder.build());
        assert_eq!(ctx.state().config().batch_size(), 1234);
        let found = physical_cache_from_ctx(&ctx).expect("extension installed");
        assert!(Arc::ptr_eq(&found, &cache));
    }
}
