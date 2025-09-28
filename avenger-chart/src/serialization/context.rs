//! SessionContext utilities for serialization
//!
//! This module provides utilities for creating SessionContext instances
//! with all necessary UDFs registered for deserializing LogicalPlans.

use datafusion::prelude::SessionContext;

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
