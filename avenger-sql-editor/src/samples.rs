//! The sample tables of the editor: six CSV files and three Arrow IPC streams of geographic data in
//! the binary, which `register` adds to a session as the tables of the schema `samples`, such as
//! `samples.penguins`. The example queries read them, and a table that the user adds keeps its own
//! name beside them.

use std::{io::Cursor, sync::Arc};

use arrow::{
    array::{Array, ArrayRef, AsArray, Float64Array, RecordBatch},
    datatypes::{DataType, Field, Float64Type, Schema},
    ipc::reader::StreamReader,
};
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

/// The file of a sample table.
#[derive(Clone, Copy, Debug)]
pub enum File {
    /// The text of a CSV file.
    Csv(&'static [u8]),
    /// An Arrow IPC stream, whose GeoArrow geometries keep their extension types. A column of
    /// points becomes the columns `longitude` and `latitude`, which `mark_geo_symbol` takes.
    Arrow(&'static [u8]),
}

/// The name and the file of each sample table. `samples/README.md` gives the source and the terms
/// of each file. The geographic files are the world data of avenger-guides-geo's examples, which
/// Git LFS stores.
pub const TABLES: [(&str, File); 9] = [
    (
        "penguins",
        File::Csv(include_bytes!("../samples/penguins.csv")),
    ),
    ("cars", File::Csv(include_bytes!("../samples/cars.csv"))),
    ("movies", File::Csv(include_bytes!("../samples/movies.csv"))),
    ("iris", File::Csv(include_bytes!("../samples/iris.csv"))),
    (
        "seattle_weather",
        File::Csv(include_bytes!("../samples/seattle_weather.csv")),
    ),
    ("sales", File::Csv(include_bytes!("../samples/sales.csv"))),
    (
        "earthquakes",
        File::Arrow(include_bytes!(
            "../../avenger-guides-geo/examples/data/earthquakes.arrows"
        )),
    ),
    (
        "countries",
        File::Arrow(include_bytes!(
            "../../avenger-guides-geo/examples/data/countries.arrows"
        )),
    ),
    (
        "plate_boundaries",
        File::Arrow(include_bytes!(
            "../../avenger-guides-geo/examples/data/plate_boundaries.arrows"
        )),
    ),
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
/// sample file, and returns the tables. DataFusion's CSV reader reads each CSV file, so a sample
/// table has the column types that the same file registered from disk has.
pub async fn register(ctx: &SessionContext) -> Result<Vec<Sample>> {
    let store = InMemory::new();
    for (name, file) in TABLES {
        if let File::Csv(text) = file {
            let path = Path::from(format!("{name}.csv"));
            store.put(&path, PutPayload::from_static(text)).await?;
        }
    }
    let url = ObjectStoreUrl::parse(STORE)?;
    ctx.register_object_store(url.as_ref(), Arc::new(store));
    let catalog_name = ctx.state().config_options().catalog.default_catalog.clone();
    let catalog = ctx
        .catalog(&catalog_name)
        .expect("a session has its default catalog");
    catalog.register_schema(SCHEMA, Arc::new(MemorySchemaProvider::new()))?;
    let mut samples = Vec::with_capacity(TABLES.len());
    for (name, file) in TABLES {
        let (schema, batches) = match file {
            File::Csv(_) => {
                let frame = ctx
                    .read_csv(format!("{STORE}/{name}.csv"), CsvReadOptions::new())
                    .await?;
                let schema = frame.schema().inner().clone();
                (schema, frame.collect().await?)
            }
            File::Arrow(bytes) => {
                let reader = StreamReader::try_new(Cursor::new(bytes), None)?;
                let batches = reader
                    .map(|batch| Ok(points_as_positions(batch?)?))
                    .collect::<Result<Vec<_>>>()?;
                let schema = match batches.first() {
                    Some(batch) => batch.schema(),
                    None => Arc::new(Schema::empty()),
                };
                (schema, batches)
            }
        };
        let columns = (schema.fields().iter())
            .map(|field| (field.name().clone(), field.data_type().clone()))
            .collect();
        let table = MemTable::try_new(schema, vec![batches])?;
        ctx.register_table(TableReference::partial(SCHEMA, name), Arc::new(table))?;
        samples.push(Sample {
            name: name.to_string(),
            columns,
        });
    }
    Ok(samples)
}

/// `batch` with the columns `longitude` and `latitude` in place of each column of GeoArrow points,
/// whose coordinates interleave.
fn points_as_positions(batch: RecordBatch) -> arrow::error::Result<RecordBatch> {
    let schema = batch.schema();
    let mut fields = Vec::new();
    let mut columns = Vec::new();
    for (field, column) in schema.fields().iter().zip(batch.columns()) {
        let point = field.extension_type_name() == Some("geoarrow.point");
        match column.as_fixed_size_list_opt() {
            Some(list) if point && list.value_length() == 2 => {
                let coordinates = list.values().as_primitive::<Float64Type>().values();
                for (axis, name) in ["longitude", "latitude"].into_iter().enumerate() {
                    let values = coordinates.iter().skip(axis).step_by(2).copied();
                    let values = Float64Array::new(values.collect(), list.nulls().cloned());
                    fields.push(Field::new(name, DataType::Float64, field.is_nullable()));
                    columns.push(Arc::new(values) as ArrayRef);
                }
            }
            _ => {
                fields.push(field.as_ref().clone());
                columns.push(column.clone());
            }
        }
    }
    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)
}
