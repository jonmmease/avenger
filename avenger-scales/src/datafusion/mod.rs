//! Scales and domain summaries in DataFusion queries.
//!
//! [`summary_expr`] is an aggregate that summarizes values for fitting a scale's domain, and
//! [`fit_expr`] fits the scale's domain to a summary, returning the fitted configuration as a
//! struct. [`scale_expr`] applies a scale with a configuration from an expression: a fitted
//! configuration in the same plan, or a scale fitted in Rust through [`config_expr`]. So one
//! plan can go from rows to pixels. [`config_from_arrow`] decodes a fitted configuration, for
//! drawing a scale's axis, and summaries decode with
//! [`DomainSummary::from_arrow`](crate::scales::domain_summary::DomainSummary::from_arrow) for
//! merging and fitting in Rust.
//!
//! The functions hold their scale, so they are built in Rust rather than registered for SQL.
//! [`ScaleExtensionCodec`] serializes plans that hold them. Scales serialize through typetag, as
//! [`ScaleImpl`] describes, so plans decode without registering their scales.

mod codec;
mod config;
mod expr;
mod fit;
mod scale;
mod summary;

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use datafusion::common::{DataFusionError, Result};

use crate::scales::ScaleImpl;

pub use codec::ScaleExtensionCodec;
pub use expr::{config_expr, config_from_arrow, fit_expr, scale_expr, summary_expr};

/// The version of the payload in which [`ScaleExtensionCodec`] serializes a scale.
pub(crate) const SCALE_PAYLOAD_VERSION: &str = "avenger-scales/1";

/// A scale with its serialized form, which identifies it in plans.
#[derive(Debug, Clone)]
pub(crate) struct SerializedScale {
    scale: Arc<dyn ScaleImpl>,
    json: Vec<u8>,
}

impl SerializedScale {
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

impl PartialEq for SerializedScale {
    fn eq(&self, other: &Self) -> bool {
        self.json == other.json
    }
}

impl Eq for SerializedScale {}

impl Hash for SerializedScale {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.json.hash(state);
    }
}

pub(crate) fn external(error: impl std::error::Error + Send + Sync + 'static) -> DataFusionError {
    DataFusionError::External(Box::new(error))
}
