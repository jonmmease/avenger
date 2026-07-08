use std::{
    error::Error,
    fs::{self, File},
    path::{Path, PathBuf},
};

use arrow::{compute::concat_batches, ipc::writer::FileWriter, record_batch::RecordBatch};
use datafusion::{
    dataframe::DataFrame,
    error::{DataFusionError, Result as DataFusionResult},
    prelude::{ParquetReadOptions, SessionContext, col, lit},
};

const TAXI_ROWS: usize = 1_000_000;
const TAXI_BATCH_ROWS: usize = 8192;
const TAXI_X_MIN: f64 = -8_242_500.0;
const TAXI_X_MAX: f64 = -8_226_500.0;
const TAXI_Y_MIN: f64 = 4_968_000.0;
const TAXI_Y_MAX: f64 = 4_983_000.0;

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    runtime.block_on(async_main())
}

async fn async_main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let input = std::env::var_os("AVENGER_TAXI_PARQUET")
        .map(PathBuf::from)
        .unwrap_or_else(default_input_path);
    let output = std::env::var_os("AVENGER_TAXI_ARROW")
        .map(PathBuf::from)
        .unwrap_or_else(default_output_path);

    if !input.exists() {
        return Err(format!("taxi parquet not found at {}", input.display()).into());
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    let ctx = SessionContext::new();
    let df = taxi_dataframe(&ctx, &input).await?;
    let batches = rechunk_record_batches(df.collect().await?, TAXI_BATCH_ROWS)?;
    let rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
    if rows == 0 {
        return Err("taxi query produced no rows".into());
    }

    write_arrow_file(&output, &batches)?;

    println!(
        "wrote {rows} point rows in {} batches to {}",
        batches.len(),
        output.display()
    );
    Ok(())
}

async fn taxi_dataframe(ctx: &SessionContext, path: &Path) -> DataFusionResult<DataFrame> {
    ctx.read_parquet(
        path.to_str()
            .ok_or_else(|| DataFusionError::Execution("taxi path is not UTF-8".to_string()))?,
        ParquetReadOptions::default(),
    )
    .await?
    .filter(
        col("pickup_x")
            .gt_eq(lit(TAXI_X_MIN))
            .and(col("pickup_x").lt_eq(lit(TAXI_X_MAX)))
            .and(col("pickup_y").gt_eq(lit(TAXI_Y_MIN)))
            .and(col("pickup_y").lt_eq(lit(TAXI_Y_MAX))),
    )?
    .select_columns(&["pickup_x", "pickup_y"])?
    .limit(0, Some(TAXI_ROWS))
}

fn rechunk_record_batches(
    batches: Vec<RecordBatch>,
    target_rows: usize,
) -> DataFusionResult<Vec<RecordBatch>> {
    if target_rows == 0 {
        return Err(DataFusionError::Execution(
            "target_rows must be greater than zero".to_string(),
        ));
    }
    let Some(schema) = batches.first().map(RecordBatch::schema) else {
        return Ok(Vec::new());
    };
    let mut output = Vec::new();
    let mut pending = Vec::new();
    let mut pending_rows = 0usize;

    for batch in batches {
        let mut offset = 0usize;
        while offset < batch.num_rows() {
            let available = batch.num_rows() - offset;
            let needed = target_rows - pending_rows;
            let take = available.min(needed);
            pending.push(batch.slice(offset, take));
            pending_rows += take;
            offset += take;

            if pending_rows == target_rows {
                output.push(concat_batches(&schema, pending.iter())?);
                pending.clear();
                pending_rows = 0;
            }
        }
    }
    if !pending.is_empty() {
        output.push(concat_batches(&schema, pending.iter())?);
    }
    Ok(output)
}

fn default_input_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scratch/data/nyc_taxi_wide.parquet")
}

fn default_output_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/nyc_taxi_1m.arrow")
}

fn write_arrow_file(
    path: &Path,
    batches: &[RecordBatch],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let Some(schema) = batches.first().map(RecordBatch::schema) else {
        return Err(format!("cannot write empty Arrow file to {}", path.display()).into());
    };
    let file = File::create(path)?;
    let mut writer = FileWriter::try_new(file, schema.as_ref())?;
    for batch in batches {
        writer.write(batch)?;
    }
    writer.finish()?;
    Ok(())
}
