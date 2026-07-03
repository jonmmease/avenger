//! GeoJSON → DataFusion table loading (doc §7).
//!
//! Columns: `geometry` (Binary, ISO WKB, winding-normalized),
//! `bbox_xmin`/`bbox_ymin`/`bbox_xmax`/`bbox_ymax` (Float64 lon/lat
//! degrees), plus one column per feature property. Property typing:
//! all-numeric → Float64, all-boolean → Boolean, otherwise Utf8
//! (non-scalar JSON values are stringified).

use std::collections::BTreeSet;
use std::sync::Arc;

use avenger_chart_core::AvengerChartError;
use avenger_geo::ingest::{GeoFeature, geojson_to_features};
use datafusion::arrow::array::{
    ArrayRef, BinaryBuilder, BooleanBuilder, Float64Builder, StringBuilder,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use serde_json::Value as JsonValue;

const RESERVED_COLUMNS: [&str; 5] = [
    "geometry",
    "bbox_xmin",
    "bbox_ymin",
    "bbox_xmax",
    "bbox_ymax",
];

/// Convert a GeoJSON string into a RecordBatch.
pub fn geojson_to_record_batch(json: &str) -> Result<RecordBatch, AvengerChartError> {
    let features = geojson_to_features(json)
        .map_err(|err| AvengerChartError::InvalidArgument(format!("GeoJSON parse: {err}")))?;
    features_to_record_batch(&features)
}

/// Read a GeoJSON file and register it as `table_name`; returns the table
/// as a DataFrame.
pub async fn register_geojson(
    ctx: &SessionContext,
    table_name: &str,
    path: impl AsRef<std::path::Path>,
) -> Result<DataFrame, AvengerChartError> {
    let json = std::fs::read_to_string(path.as_ref()).map_err(|err| {
        AvengerChartError::InvalidArgument(format!(
            "read GeoJSON {}: {err}",
            path.as_ref().display()
        ))
    })?;
    let batch = geojson_to_record_batch(&json)?;
    ctx.register_batch(table_name, batch)
        .map_err(AvengerChartError::from)?;
    ctx.table(table_name).await.map_err(AvengerChartError::from)
}

fn features_to_record_batch(features: &[GeoFeature]) -> Result<RecordBatch, AvengerChartError> {
    let mut property_names: BTreeSet<String> = BTreeSet::new();
    for feature in features {
        for key in feature.properties.keys() {
            if RESERVED_COLUMNS.contains(&key.as_str()) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "GeoJSON property '{key}' collides with a reserved geo column name"
                )));
            }
            property_names.insert(key.clone());
        }
    }

    let mut fields = vec![
        Field::new("geometry", DataType::Binary, true),
        Field::new("bbox_xmin", DataType::Float64, true),
        Field::new("bbox_ymin", DataType::Float64, true),
        Field::new("bbox_xmax", DataType::Float64, true),
        Field::new("bbox_ymax", DataType::Float64, true),
    ];
    let mut columns: Vec<ArrayRef> = Vec::new();

    let mut geometry = BinaryBuilder::new();
    let mut bbox_builders = [
        Float64Builder::new(),
        Float64Builder::new(),
        Float64Builder::new(),
        Float64Builder::new(),
    ];
    for feature in features {
        match &feature.wkb {
            Some(bytes) => geometry.append_value(bytes),
            None => geometry.append_null(),
        }
        match feature.bbox {
            Some(bbox) => {
                for (builder, value) in bbox_builders.iter_mut().zip(bbox) {
                    builder.append_value(value);
                }
            }
            None => {
                for builder in &mut bbox_builders {
                    builder.append_null();
                }
            }
        }
    }
    columns.push(Arc::new(geometry.finish()));
    for mut builder in bbox_builders {
        columns.push(Arc::new(builder.finish()));
    }

    for name in &property_names {
        let values: Vec<Option<&JsonValue>> = features
            .iter()
            .map(|feature| feature.properties.get(name))
            .collect();
        let (field, array) = property_column(name, &values);
        fields.push(field);
        columns.push(array);
    }

    let schema = Arc::new(Schema::new(fields));
    RecordBatch::try_new(schema, columns).map_err(|err| {
        AvengerChartError::InternalError(format!("GeoJSON record batch assembly: {err}"))
    })
}

fn property_column(name: &str, values: &[Option<&JsonValue>]) -> (Field, ArrayRef) {
    let all_numeric = values.iter().all(|value| {
        matches!(
            value,
            None | Some(JsonValue::Null) | Some(JsonValue::Number(_))
        )
    });
    if all_numeric {
        let mut builder = Float64Builder::new();
        for value in values {
            match value {
                Some(JsonValue::Number(number)) => builder.append_option(number.as_f64()),
                _ => builder.append_null(),
            }
        }
        return (
            Field::new(name, DataType::Float64, true),
            Arc::new(builder.finish()),
        );
    }
    let all_boolean = values.iter().all(|value| {
        matches!(
            value,
            None | Some(JsonValue::Null) | Some(JsonValue::Bool(_))
        )
    });
    if all_boolean {
        let mut builder = BooleanBuilder::new();
        for value in values {
            match value {
                Some(JsonValue::Bool(flag)) => builder.append_value(*flag),
                _ => builder.append_null(),
            }
        }
        return (
            Field::new(name, DataType::Boolean, true),
            Arc::new(builder.finish()),
        );
    }
    let mut builder = StringBuilder::new();
    for value in values {
        match value {
            None | Some(JsonValue::Null) => builder.append_null(),
            Some(JsonValue::String(text)) => builder.append_value(text),
            Some(other) => builder.append_value(other.to_string()),
        }
    }
    (
        Field::new(name, DataType::Utf8, true),
        Arc::new(builder.finish()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn us_states_path() -> String {
        format!(
            "{}/../avenger-chart/tests/data/geo/us-states.json",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    #[tokio::test]
    async fn loads_us_states() {
        let ctx = SessionContext::new();
        let df = register_geojson(&ctx, "us_states", us_states_path())
            .await
            .expect("register");
        let batches = df.collect().await.expect("collect");
        let total: usize = batches.iter().map(|batch| batch.num_rows()).sum();
        assert_eq!(total, 52, "50 states + DC + PR");
        let schema = batches[0].schema();
        assert_eq!(schema.field(0).name(), "geometry");
        assert_eq!(schema.field(0).data_type(), &DataType::Binary);
        assert!(schema.field_with_name("density").is_ok());
        assert_eq!(
            schema.field_with_name("density").unwrap().data_type(),
            &DataType::Float64
        );
        assert!(schema.field_with_name("name").is_ok());
        // bbox sanity: this dataset draws Alaska "unwrapped" past the
        // antimeridian (xmin ≈ -188.9), so accept the extended range.
        let xmin = batches[0]
            .column_by_name("bbox_xmin")
            .unwrap()
            .as_any()
            .downcast_ref::<datafusion::arrow::array::Float64Array>()
            .unwrap();
        assert!(
            xmin.iter()
                .flatten()
                .all(|value| (-360.0..=180.0).contains(&value))
        );
    }

    #[tokio::test]
    async fn loads_countries_with_antimeridian_features() {
        let ctx = SessionContext::new();
        let path = format!(
            "{}/../avenger-chart/tests/data/geo/ne_110m_admin_0_countries.geojson",
            env!("CARGO_MANIFEST_DIR")
        );
        let df = register_geojson(&ctx, "countries", path)
            .await
            .expect("register");
        let batches = df.collect().await.expect("collect");
        let total: usize = batches.iter().map(|batch| batch.num_rows()).sum();
        assert_eq!(total, 177);
    }
}
