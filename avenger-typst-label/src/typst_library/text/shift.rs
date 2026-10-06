//! Ported from crates/typst-library/src/text/shift.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: a partial port so far. The fallback script metrics are here, which font metrics
//! read; the text library adds the sub- and superscript elements.

use crate::typst_library::layout::Em;
use crate::typst_library::text::ScriptMetrics;

pub static DEFAULT_SUBSCRIPT_METRICS: ScriptMetrics = ScriptMetrics {
    width: Em::new(0.6),
    height: Em::new(0.6),
    horizontal_offset: Em::zero(),
    vertical_offset: Em::new(-0.2),
};

pub static DEFAULT_SUPERSCRIPT_METRICS: ScriptMetrics = ScriptMetrics {
    width: Em::new(0.6),
    height: Em::new(0.6),
    horizontal_offset: Em::zero(),
    vertical_offset: Em::new(0.5),
};
