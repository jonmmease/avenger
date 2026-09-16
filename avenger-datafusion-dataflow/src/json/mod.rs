//! JSON authoring and query adapters. Definitions lower to the native Dataflow API.
mod load;
mod request;
mod sources;
mod spec;
mod values;
pub use sources::{AssetBindings, FileSourceResolver, SourceResolver};
pub use spec::*;
fn invalid(message: impl std::fmt::Display) -> crate::Error {
    crate::Error::Json(message.to_string())
}
/// JSON Schema for the supported definition grammar.
pub fn dataflow_spec_schema() -> schemars::Schema {
    schemars::schema_for!(DataflowSpec)
}
/// JSON Schema for the supported request grammar.
pub fn query_request_schema() -> schemars::Schema {
    schemars::schema_for!(QueryRequest)
}
