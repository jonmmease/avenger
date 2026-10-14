use anyhow::{Context, Result, ensure};
use arrow::{
    compute::concat_batches,
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use avenger_datafusion_dataflow::{TableSnapshot, TableStore};
use datafusion::parquet::arrow::{ProjectionMask, arrow_reader::ParquetRecordBatchReaderBuilder};
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

pub fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("arr_delay", DataType::Int32, false),
        Field::new("distance", DataType::Int32, false),
        Field::new("scheduled_minute", DataType::UInt16, false),
        Field::new("carrier", DataType::Utf8, false),
    ]))
}

pub struct Replay {
    files: Vec<PathBuf>,
    batch_rows: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    file: usize,
    offset: usize,
    pub batches: usize,
}

pub struct Appended {
    pub snapshot: TableSnapshot,
    pub next: Cursor,
    pub period: String,
    pub append_ms: f64,
}

impl Replay {
    pub fn open(path: &Path, batch_rows: usize) -> Result<Arc<Self>> {
        ensure!(batch_rows > 0, "batch size must be positive");
        let mut files = if path.is_dir() {
            std::fs::read_dir(path)?
                .map(|entry| Ok(entry?.path()))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .filter(|p| p.extension().is_some_and(|ext| ext == "parquet"))
                .collect()
        } else {
            vec![path.to_owned()]
        };
        files.sort();
        ensure!(!files.is_empty(), "no Parquet files in {}", path.display());
        let replay = Arc::new(Self { files, batch_rows });
        replay.reader(0)?;
        Ok(replay)
    }

    fn reader(&self, file: usize) -> Result<ParquetRecordBatchReaderBuilder<File>> {
        let path = &self.files[file];
        let reader = ParquetRecordBatchReaderBuilder::try_new(
            File::open(path).with_context(|| format!("open {}", path.display()))?,
        )?;
        let required = schema();
        for expected in required.fields() {
            let field = reader
                .schema()
                .field_with_name(expected.name())
                .with_context(|| format!("{} requires {}", path.display(), expected.name()))?;
            ensure!(
                field.data_type() == expected.data_type(),
                "{}: {} must have type {}",
                path.display(),
                expected.name(),
                expected.data_type()
            );
        }
        let columns = required
            .fields()
            .iter()
            .map(|field| reader.schema().index_of(field.name()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let projection = ProjectionMask::roots(reader.parquet_schema(), columns);
        Ok(reader.with_projection(projection))
    }

    // An explicit file/row cursor makes decoding repeatable. Only one bounded
    // batch is decoded at a time, and no mutable reader survives a canceled task.
    fn read(&self, mut cursor: Cursor) -> Result<Option<(RecordBatch, Cursor, String)>> {
        while cursor.file < self.files.len() {
            let reader = self
                .reader(cursor.file)?
                .with_offset(cursor.offset)
                .with_limit(self.batch_rows)
                .with_batch_size(self.batch_rows)
                .build()?;
            let mut batches = vec![];
            let required = schema();
            for batch in reader {
                let batch = batch?;
                let columns = required
                    .fields()
                    .iter()
                    .map(|field| {
                        let column = batch
                            .column_by_name(field.name())
                            .context("projected replay column")?;
                        ensure!(
                            column.null_count() == 0,
                            "{} contains null values",
                            field.name()
                        );
                        Ok(column.clone())
                    })
                    .collect::<Result<Vec<_>>>()?;
                batches.push(RecordBatch::try_new(required.clone(), columns)?);
            }
            let batch = concat_batches(&required, &batches)?;
            if batch.num_rows() == 0 {
                cursor.file += 1;
                cursor.offset = 0;
                continue;
            }
            let period = self.files[cursor.file]
                .file_stem()
                .context("Parquet filename")?
                .to_string_lossy()
                .into_owned();
            cursor.offset += batch.num_rows();
            cursor.batches += 1;
            return Ok(Some((batch, cursor, period)));
        }
        Ok(None)
    }

    pub async fn append(
        self: Arc<Self>,
        cursor: Cursor,
        store: TableStore,
    ) -> Result<Option<Appended>> {
        let decoded = tokio::task::spawn_blocking(move || self.read(cursor)).await??;
        let Some((batch, next, period)) = decoded else {
            return Ok(None);
        };
        let started = Instant::now();
        let snapshot = store.append_batch(batch)?;
        Ok(Some(Appended {
            snapshot,
            next,
            period,
            append_ms: started.elapsed().as_secs_f64() * 1000.,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Int32Array, StringArray, UInt16Array};
    use datafusion::parquet::arrow::ArrowWriter;

    pub fn batch(values: &[i32]) -> RecordBatch {
        RecordBatch::try_new(
            schema(),
            vec![
                Arc::new(Int32Array::from(values.to_vec())),
                Arc::new(Int32Array::from(vec![500; values.len()])),
                Arc::new(UInt16Array::from(vec![600; values.len()])),
                Arc::new(StringArray::from(vec!["AA"; values.len()])),
            ],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn replay_preserves_file_order_partial_batches_and_captured_history() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for (name, values) in [("2016-02", vec![4, 5]), ("2016-01", vec![1, 2, 3])] {
            let mut writer = ArrowWriter::try_new(
                File::create(dir.path().join(format!("{name}.parquet")))?,
                schema(),
                None,
            )?;
            writer.write(&batch(&values))?;
            writer.close()?;
        }
        let replay = Replay::open(dir.path(), 2)?;
        let store = TableStore::new(TableSnapshot::empty(schema()));
        let mut cursor = Cursor::default();
        let mut prefixes = vec![];
        while let Some(result) = replay.clone().append(cursor, store.clone()).await? {
            cursor = result.next;
            prefixes.push(result.snapshot);
        }
        assert_eq!(
            prefixes
                .iter()
                .map(TableSnapshot::num_rows)
                .collect::<Vec<_>>(),
            [2, 3, 5]
        );
        assert_eq!(cursor.batches, 3);
        let rows = prefixes[2]
            .batch_iter()
            .flat_map(|batch| {
                batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int32Array>()
                    .unwrap()
                    .values()
                    .to_vec()
            })
            .collect::<Vec<_>>();
        assert_eq!(rows, [1, 2, 3, 4, 5]);
        let restarted = TableStore::new(TableSnapshot::empty(schema()));
        let first = replay.append(Cursor::default(), restarted).await?.unwrap();
        assert_eq!(first.snapshot.num_rows(), 2);
        assert_ne!(first.snapshot.id(), prefixes[0].id());
        Ok(())
    }
}
