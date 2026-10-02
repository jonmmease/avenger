#![doc = include_str!("../README.md")]

mod codec;
mod expr;
mod spec;
mod udf;

pub use avenger_scales;
pub use codec::ScaleExtensionCodec;
pub use datafusion;
pub use expr::{list_literal, options_literal, scale_expr};
pub use spec::{BuiltinScale, ScaleSpec};
pub use udf::{create_scale_udf, ScaleUDF, SCALE_FUNCTION_NAME, SCALE_FUNCTION_VERSION};
