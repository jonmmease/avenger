//! SessionContext utilities for serialization
//!
//! This module provides utilities for creating SessionContext instances
//! with all necessary UDFs registered for deserializing LogicalPlans.

use datafusion::prelude::SessionContext;

/// Create a SessionContext with all necessary UDFs registered for avenger-chart
///
/// This context can be used to deserialize LogicalPlans that contain
/// scale transformations and other custom functions.
pub fn create_context_with_udfs() -> SessionContext {
    let ctx = SessionContext::new();

    // For now, we don't register any UDFs because scale UDFs are created
    // dynamically with specific scale configurations.
    //
    // TODO: This is a limitation - we need to either:
    // 1. Store scale configurations alongside the LogicalPlan
    // 2. Apply scales after deserialization rather than in the plan
    // 3. Create a registry of all scale UDFs used in the plan

    ctx
}

/// Information needed to recreate UDFs for a serialized plan
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UdfRegistry {
    // TODO: Add fields to store scale configurations and other UDF metadata
    // For now this is a placeholder
}

impl UdfRegistry {
    pub fn new() -> Self {
        Self {}
    }

    /// Register all UDFs from this registry with the given context
    pub fn register_with_context(&self, _ctx: &SessionContext) {
        // TODO: Implement UDF registration
    }
}