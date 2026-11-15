//! Data transforms for DataFusion: the Vega-style `bin`, `stack`, `timeunit`, and `impute`, and
//! `lump`, which keeps the top categories and lumps the rest into an Other group. A front end,
//! such as avenger-sql, builds a typed call of a transform from its planned arguments, and the
//! crate plans the call.
//!
//! - `bin`, `timeunit`, and `lump` are expressions of a projection, over the window partitions
//!   of their `OVER` clause. A front end writes each call as a placeholder window, from
//!   [`bin::window`], [`timeunit::window`], or [`lump::window`], and plans the windows of the
//!   projection through [`plan_windows`], which plans each placeholder onto the input. `bin` and
//!   `timeunit` plan as an aggregate of each partition, joined back to its rows, and `lump` as
//!   a grouped plan of the categories.
//! - `stack` is the window function of [`stack::window`].
//! - `impute` is a table function over an input relation, which [`impute::plan`] plans.
//!
//! `bin`, `timeunit`, and `lump` read their input more than once, so their plans read it through
//! a shared node of avenger-datafusion-shared, which runs it once. A query planner plans these
//! nodes with its `SharedExtensionPlanner` and `impute` with
//! [`table_function::TableFunctionExtensionPlanner`], and a session runs its `InlineShared`,
//! which computes cheap inputs again in place of sharing them.
//!
//! The options of a call are Rust values, which their `check` methods check: an
//! [`OptionError`] names the option, so the front end places the error at its argument.
//!
//! Each module describes its transform: [`bin`], [`stack`], [`timeunit`], [`impute`], and
//! [`lump`]. [`table_function`] holds the machinery of table functions over an input relation,
//! such as `impute`. [`StructGroupSplit`] groups an aggregate by the fields of a struct key, such
//! as a bin, then makes the struct again above the aggregate.

pub mod bin;
pub mod impute;
pub mod lump;
pub mod stack;
pub mod table_function;
pub mod timeunit;
mod utils;
mod window;

pub use utils::StructGroupSplit;
pub use window::plan_windows;

use std::fmt;

use datafusion::common::DataFusionError;

/// An error of the value of an option of a call, such as a `maxbins` of 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionError {
    /// The name of the option, such as `maxbins`.
    pub option: &'static str,
    /// The error, as a sentence.
    pub message: String,
}

impl OptionError {
    pub(crate) fn new(option: &'static str, message: impl Into<String>) -> Self {
        Self {
            option,
            message: message.into(),
        }
    }
}

impl fmt::Display for OptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for OptionError {}

impl From<OptionError> for DataFusionError {
    fn from(error: OptionError) -> Self {
        DataFusionError::Plan(error.message)
    }
}
