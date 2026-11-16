//! The sample tables of the editor: six CSV files in the binary, which `register` adds to a
//! session as the tables of the schema `samples`, such as `samples.penguins`. The example queries
//! read them, and a table that the user adds keeps its own name beside them.

use std::sync::Arc;

use arrow::datatypes::DataType;
use datafusion::{
    catalog::MemorySchemaProvider,
    common::{Result, TableReference},
    datasource::MemTable,
    execution::object_store::ObjectStoreUrl,
    object_store::{memory::InMemory, path::Path, ObjectStoreExt, PutPayload},
    prelude::{CsvReadOptions, SessionContext},
};

/// The schema of the sample tables.
pub const SCHEMA: &str = "samples";

/// The name and the CSV text of each sample table. `samples/README.md` gives the source and the
/// terms of each file.
pub const TABLES: [(&str, &[u8]); 6] = [
    ("penguins", include_bytes!("../samples/penguins.csv")),
    ("cars", include_bytes!("../samples/cars.csv")),
    ("movies", include_bytes!("../samples/movies.csv")),
    ("iris", include_bytes!("../samples/iris.csv")),
    (
        "seattle_weather",
        include_bytes!("../samples/seattle_weather.csv"),
    ),
    ("sales", include_bytes!("../samples/sales.csv")),
];

/// A sample table: its name in the schema `samples`, and the names and types of its columns.
#[derive(Clone, Debug)]
pub struct Sample {
    pub name: String,
    pub columns: Vec<(String, DataType)>,
}

/// The address of the object store that holds the sample files while `register` reads them.
const STORE: &str = "samples://files";

/// Adds the schema `samples` to the default catalog of `ctx`, with a table in memory for each
/// sample file, and returns the tables. DataFusion's CSV reader reads each file, so a sample
/// table has the column types that the same file registered from disk has.
pub async fn register(ctx: &SessionContext) -> Result<Vec<Sample>> {
    let store = InMemory::new();
    for (name, text) in TABLES {
        let path = Path::from(format!("{name}.csv"));
        store.put(&path, PutPayload::from_static(text)).await?;
    }
    let url = ObjectStoreUrl::parse(STORE)?;
    ctx.register_object_store(url.as_ref(), Arc::new(store));
    let catalog_name = ctx.state().config_options().catalog.default_catalog.clone();
    let catalog = ctx
        .catalog(&catalog_name)
        .expect("a session has its default catalog");
    catalog.register_schema(SCHEMA, Arc::new(MemorySchemaProvider::new()))?;
    let mut samples = Vec::with_capacity(TABLES.len());
    for (name, _) in TABLES {
        let frame = ctx
            .read_csv(format!("{STORE}/{name}.csv"), CsvReadOptions::new())
            .await?;
        let schema = frame.schema().inner().clone();
        let columns = (schema.fields().iter())
            .map(|field| (field.name().clone(), field.data_type().clone()))
            .collect();
        let batches = frame.collect().await?;
        let table = MemTable::try_new(schema, vec![batches])?;
        ctx.register_table(TableReference::partial(SCHEMA, name), Arc::new(table))?;
        samples.push(Sample {
            name: name.to_string(),
            columns,
        });
    }
    Ok(samples)
}
