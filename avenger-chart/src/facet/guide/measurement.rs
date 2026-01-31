//! Measurement utilities for facet guides
//!
//! This module provides types for asynchronous overflow measurement.

use crate::error::AvengerChartError;
use crate::guide::OverflowSpaceRequirement;
use std::future::Future;
use std::pin::Pin;

/// Type alias for async measure overflow functions
pub type AsyncMeasureOverflowFn = Box<
    dyn Fn() -> Pin<
            Box<dyn Future<Output = Result<OverflowSpaceRequirement, AvengerChartError>> + Send>,
        > + Send
        + Sync,
>;
