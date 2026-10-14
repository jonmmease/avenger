//! Scales and domain summaries in DataFusion queries.
//!
//! [`scale_expr`] applies a scale to values in a query, with its domain, range, and options given
//! by expressions. [`summary_expr`] is an aggregate that summarizes values for fitting a scale's
//! domain, and returns the summary in the Arrow form of
//! [`DomainSummary::to_arrow`](crate::scales::domain_summary::DomainSummary::to_arrow). The fit
//! stays in Rust: decode the summary with
//! [`DomainSummary::from_arrow`](crate::scales::domain_summary::DomainSummary::from_arrow), merge
//! it with other summaries, and fit the scale's domain to it.
//!
//! Both functions hold their scale, so they are built in Rust rather than registered for SQL.
//! [`ScaleExtensionCodec`] serializes plans that hold them. Scales serialize through typetag, as
//! [`ScaleImpl`] describes, so plans decode without registering their scales.

mod codec;
mod expr;
mod scale;
mod summary;

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use datafusion::common::{DataFusionError, Result};

use crate::scales::ScaleImpl;

pub use codec::ScaleExtensionCodec;
pub use expr::{list_literal, options_literal, scale_expr, summary_expr};

/// The version of the payload in which [`ScaleExtensionCodec`] serializes a scale.
pub const SCALE_PAYLOAD_VERSION: &str = "avenger-scales/1";

/// A scale with its serialized form, which identifies it in plans.
#[derive(Debug, Clone)]
pub(crate) struct PlannedScale {
    scale: Arc<dyn ScaleImpl>,
    json: Vec<u8>,
}

impl PlannedScale {
    pub(crate) fn new(scale: &dyn ScaleImpl) -> Result<Self> {
        let mut json = serde_json::to_value(scale).map_err(external)?;
        json.sort_all_objects();
        Self::from_json(&json)
    }

    /// The scale decoded from its serialized form, so that a plan runs the same scale before and
    /// after serialization.
    pub(crate) fn from_json(json: &serde_json::Value) -> Result<Self> {
        let scale: Box<dyn ScaleImpl> = serde_json::from_value(json.clone()).map_err(external)?;
        Ok(Self {
            scale: Arc::from(scale),
            json: serde_json::to_vec(json).map_err(external)?,
        })
    }

    pub(crate) fn scale(&self) -> &Arc<dyn ScaleImpl> {
        &self.scale
    }

    pub(crate) fn json(&self) -> &[u8] {
        &self.json
    }
}

impl PartialEq for PlannedScale {
    fn eq(&self, other: &Self) -> bool {
        self.json == other.json
    }
}

impl Eq for PlannedScale {}

impl Hash for PlannedScale {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.json.hash(state);
    }
}

pub(crate) fn external(error: impl std::error::Error + Send + Sync + 'static) -> DataFusionError {
    DataFusionError::External(Box::new(error))
}
