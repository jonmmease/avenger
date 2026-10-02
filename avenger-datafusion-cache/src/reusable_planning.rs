//! Configuration marker for reusable physical-prototype planning.

use datafusion_common::{config::ConfigExtension, extensions_options};

extensions_options! {
    /// Internal physical-planning profile used for reusable prototypes.
    pub struct ReusablePlanPlanning {
        /// Bypass result-cache rewriting while constructing a prototype.
        pub enabled: bool, default = false
    }
}

impl ConfigExtension for ReusablePlanPlanning {
    const PREFIX: &'static str = "avenger_reusable_plan";
}
