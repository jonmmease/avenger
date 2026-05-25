//! Compatibility re-export for `Maybe` utilities.
//!
//! The implementation now lives under `avenger-chart-core` as part of the crate-split
//! migration. Keep this module behavior-free so existing public paths continue
//! to work while internal imports move to the new owner.

pub use avenger_chart_core::maybe::*;
