//! The sample project, which the editor embeds and opens when it starts: a data script that loads
//! the sample tables in the schema `samples`, such as `samples.penguins`, from six CSV files and
//! three Arrow IPC streams of geographic data, and a chart script for each example of the
//! gallery. The files are those of `samples/`, which `build.rs` lists, with the Arrow IPC streams
//! of avenger-guides-geo's examples through the link `data/files/geo`.

use avenger_sql_project::{FileSource, SourceError};
use datafusion::object_store::PutPayload;

/// The name of the sample project, the host of its files' URLs, as `memory://samples/`.
pub const NAME: &str = "samples";

/// The files of the sample project, each a path relative to its root and its bytes, in the order
/// of their paths.
pub const FILES: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/samples.rs"));

/// The files of the sample project in an embedded file source.
pub async fn source() -> Result<FileSource, SourceError> {
    let files = (FILES.iter()).map(|&(path, bytes)| (path, PutPayload::from_static(bytes)));
    FileSource::embedded(NAME, files).await
}
