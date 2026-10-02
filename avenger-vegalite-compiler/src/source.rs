use crate::{error, spec::*, CompileError, Result, TableSnapshot};
use avenger_datafusion_dataflow::datafusion::arrow::{
    array::{new_null_array, ArrayRef, BooleanArray, Float64Array, StringArray, UInt64Array},
    datatypes::{DataType, Field, Schema},
    record_batch::{RecordBatch, RecordBatchOptions},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Seek, SeekFrom},
    path::Path,
    sync::Arc,
};

pub(crate) struct Source {
    pub snapshot: TableSnapshot,
    pub ordinal: String,
    pub prefix: String,
}

pub(crate) async fn load(
    spec: &UnitSpec,
    bindings: &BTreeMap<String, TableSnapshot>,
    base: &Path,
) -> Result<Source> {
    let referenced = referenced_fields(spec);
    let base = std::path::absolute(base).map_err(|e| CompileError::at("data.url", e))?;
    let snapshot = match &spec.data {
        Data::Named { name, format } => {
            if format.as_ref().and_then(|f| f.format_type).is_some() {
                return Err(error(
                    "data.format",
                    "format is not supported on a named table",
                ));
            }
            if let Some(table) = bindings.get(name) {
                table.clone()
            } else if let Some(rows) = spec.datasets.as_ref().and_then(|d| d.get(name)) {
                from_rows(rows, &referenced)?
            } else {
                return Err(error("data.name", format!("unknown dataset {name}")));
            }
        }
        Data::Inline { values, format, .. } => {
            if format
                .as_ref()
                .and_then(|f| f.format_type)
                .is_some_and(|f| f != FormatType::Json)
            {
                return Err(error(
                    "data.format",
                    "inline object rows require JSON format",
                ));
            }
            from_rows(values, &referenced)?
        }
        Data::Empty => from_rows(&[], &referenced)?,
        Data::Url { url, format, .. } => {
            let file = if url.starts_with("file:") {
                url::Url::parse(url)
                    .map_err(|e| CompileError::at("data.url", e))?
                    .to_file_path()
                    .map_err(|_| error("data.url", "invalid local file URL"))?
            } else if url.contains("://") || url.starts_with("data:") {
                return Err(error(
                    "data.url",
                    "only local files are supported; load remote data into a named TableSnapshot",
                ));
            } else {
                base.join(url)
            };
            let format = format
                .as_ref()
                .and_then(|f| f.format_type)
                .unwrap_or_else(|| {
                    match file
                        .extension()
                        .and_then(|x| x.to_str())
                        .map(str::to_ascii_lowercase)
                        .as_deref()
                    {
                        Some("csv") => FormatType::Csv,
                        Some("tsv") => FormatType::Tsv,
                        _ => FormatType::Json,
                    }
                });
            tokio::task::spawn_blocking(move || -> Result<TableSnapshot> {
                let mut reader = File::open(file).map_err(|e| CompileError::at("data.url", e))?;
                if format == FormatType::Json {
                    let rows: InlineDataset = serde_json::from_reader(reader)
                        .map_err(|e| CompileError::at("data.url", e))?;
                    from_rows(&rows, &referenced)
                } else {
                    use avenger_datafusion_dataflow::datafusion::arrow::csv::reader::{
                        Format, ReaderBuilder,
                    };
                    let delimiter = if format == FormatType::Tsv {
                        b'\t'
                    } else {
                        b','
                    };
                    let (schema, _) = Format::default()
                        .with_header(true)
                        .with_delimiter(delimiter)
                        .infer_schema(&mut reader, None)
                        .map_err(|e| CompileError::at("data.url", e))?;
                    reader
                        .seek(SeekFrom::Start(0))
                        .map_err(|e| CompileError::at("data.url", e))?;
                    let schema = Arc::new(schema);
                    let batches = ReaderBuilder::new(schema.clone())
                        .with_header(true)
                        .with_delimiter(delimiter)
                        .with_batch_size(8192)
                        .build(reader)
                        .map_err(|e| CompileError::at("data.url", e))?
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .map_err(|e| CompileError::at("data.url", e))?;
                    TableSnapshot::from_batches(schema, batches)
                        .map_err(|e| CompileError::at("data", e))
                }
            })
            .await
            .map_err(|e| CompileError::at("data.url", e))??
        }
    };
    let mut names: BTreeSet<String> = snapshot
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    for transform in spec.transform.iter().flatten() {
        match transform {
            Transform::Aggregate(t) => names.extend(t.aggregate.iter().map(|a| a.as_.clone())),
            Transform::Bin(t) => match &t.as_ {
                BinOutput::Name(n) => {
                    names.insert(n.clone());
                    names.insert(format!("{n}_end"));
                }
                BinOutput::Pair(pair) => names.extend(pair.clone()),
            },
            Transform::Filter(_) => {}
        }
    }
    let mut prefix = "__vl_".to_string();
    while names.iter().any(|n| n.starts_with(&prefix)) {
        prefix.push('_');
    }
    let ordinal = format!("{prefix}ordinal");
    let mut fields = snapshot
        .schema()
        .fields()
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    fields.push(Arc::new(Field::new(&ordinal, DataType::UInt64, false)));
    let schema = Arc::new(Schema::new(fields));
    let mut offset = 0u64;
    let batches = snapshot
        .batches()
        .iter()
        .map(|b| {
            let mut columns = b.columns().to_vec();
            columns.push(Arc::new(UInt64Array::from_iter_values(
                offset..offset + b.num_rows() as u64,
            )));
            offset += b.num_rows() as u64;
            RecordBatch::try_new(schema.clone(), columns).map_err(|e| CompileError::at("data", e))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Source {
        snapshot: TableSnapshot::from_batches(schema, batches)
            .map_err(|e| CompileError::at("data", e))?,
        ordinal,
        prefix,
    })
}

fn from_rows(
    rows: &[serde_json::Map<String, serde_json::Value>],
    referenced: &BTreeSet<String>,
) -> Result<TableSnapshot> {
    use serde_json::Value;
    let names: BTreeSet<_> = rows.iter().flat_map(|r| r.keys()).collect();
    let mut fields = vec![];
    let mut columns: Vec<ArrayRef> = vec![];
    'columns: for name in names {
        let values = rows
            .iter()
            .map(|r| r.get(name).unwrap_or(&Value::Null))
            .collect::<Vec<_>>();
        let mut ty = DataType::Null;
        for v in &values {
            let t = match v {
                Value::Null => continue,
                Value::Number(_) => DataType::Float64,
                Value::Bool(_) => DataType::Boolean,
                Value::String(_) => DataType::Utf8,
                _ if !referenced.contains(name) => continue 'columns,
                _ => {
                    return Err(error(
                        format!("data.values.{name}"),
                        "nested values are not supported",
                    ))
                }
            };
            if ty != DataType::Null && ty != t {
                if !referenced.contains(name) {
                    continue 'columns;
                }
                return Err(error(
                    format!("data.values.{name}"),
                    "mixed column types are not supported",
                ));
            }
            ty = t;
        }
        columns.push(match ty {
            DataType::Null => new_null_array(&ty, rows.len()),
            DataType::Float64 => {
                Arc::new(Float64Array::from_iter(values.iter().map(|v| v.as_f64())))
            }
            DataType::Boolean => {
                Arc::new(BooleanArray::from_iter(values.iter().map(|v| v.as_bool())))
            }
            _ => Arc::new(StringArray::from_iter(values.iter().map(|v| v.as_str()))),
        });
        fields.push(Field::new(name, ty, true));
    }
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new_with_options(
        schema.clone(),
        columns,
        &RecordBatchOptions::new().with_row_count(Some(rows.len())),
    )
    .map_err(|e| CompileError::at("data.values", e))?;
    TableSnapshot::from_batches(schema, vec![batch]).map_err(|e| CompileError::at("data.values", e))
}

fn referenced_fields(spec: &UnitSpec) -> BTreeSet<String> {
    let mut fields = Vec::new();
    if let Some(e) = &spec.encoding {
        for c in [&e.x, &e.y].into_iter().flatten() {
            fields.extend(c.field.iter().cloned());
        }
        for c in [&e.x2, &e.y2].into_iter().flatten() {
            fields.push(c.field.clone());
        }
    }
    for t in spec.transform.iter().flatten() {
        match t {
            Transform::Filter(t) => fields.push(t.filter.field.clone()),
            Transform::Bin(t) => fields.push(t.field.clone()),
            Transform::Aggregate(t) => {
                fields.extend(t.groupby.iter().flatten().cloned());
                fields.extend(t.aggregate.iter().filter_map(|a| a.field.clone()));
            }
        }
    }
    fields
        .into_iter()
        .map(|s| {
            let mut result = String::new();
            let mut chars = s.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    if let Some(c) = chars.next() {
                        result.push(c);
                    }
                } else {
                    result.push(c);
                }
            }
            result
        })
        .collect()
}
