//! Scale type specifications for compile-time type safety

use std::{collections::HashMap, sync::Arc};

use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};
use serde::{Deserialize, Serialize};

/// Marker trait for scale types
#[typetag::serde(tag = "type")]
pub trait ScaleSpec: std::fmt::Debug + Send + Sync + 'static {
    /// Clone this scale spec into a new boxed instance
    fn clone_box(&self) -> Box<dyn ScaleSpec>;

    /// Create the scale implementation for this type
    fn create_impl(&self) -> Arc<dyn ScaleImpl>;

    /// Get the name of this scale type
    fn name(&self) -> &'static str;

    /// Get default options for this scale type
    /// Returns a map of option name to scalar value
    fn default_options(&self) -> HashMap<String, avenger_scales::scalar::Scalar> {
        HashMap::new()
    }

    /// Get the domain kind for this scale type
    fn domain_kind(&self) -> DomainKind {
        self.create_impl().domain_kind()
    }

    /// Get the range kind for this scale type
    fn range_kind(&self) -> RangeKind {
        self.create_impl().range_kind()
    }
}

// Implement Clone for Box<dyn ScaleSpec>
impl Clone for Box<dyn ScaleSpec> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// Auto scale marker type (for automatic type inference)
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Auto;

// ===== ScaleSpec implementations =====

#[typetag::serde]
impl ScaleSpec for Auto {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(*self)
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        unimplemented!("Auto scale type does not have a direct implementation");
    }

    fn name(&self) -> &'static str {
        "auto"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_scale_is_named_auto() {
        let auto = Auto;
        assert_eq!(auto.name(), "auto");
    }
}
