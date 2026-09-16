//! Regenerate the published version 1 JSON schemas.
use avenger_datafusion_dataflow::json::{dataflow_spec_schema, query_request_schema};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas");
    std::fs::create_dir_all(&directory)?;
    std::fs::write(
        directory.join("dataflow-spec-v1.schema.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&dataflow_spec_schema())?
        ),
    )?;
    std::fs::write(
        directory.join("query-request-v1.schema.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&query_request_schema())?
        ),
    )?;
    Ok(())
}
