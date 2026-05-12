//! Helper module for loading test datasets
//!
//! All datasets are sourced from vega-datasets and stored as Parquet files
//! for efficient loading and guaranteed schema consistency.

use datafusion::dataframe::DataFrame;
use datafusion::error::DataFusionError;
use datafusion::prelude::*;
use std::path::PathBuf;

/// Get the absolute path to a test data file
fn test_data_path(filename: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let path: PathBuf = [manifest_dir, "tests", "data", filename].iter().collect();
    path.to_string_lossy().to_string()
}

/// Load the Seattle weather dataset
///
/// Daily weather observations from Seattle (2012-2015)
/// - Rows: ~1,460
/// - Columns: date, precipitation, temp_max, temp_min, wind, weather
pub async fn seattle_weather(ctx: &SessionContext) -> Result<DataFrame, DataFusionError> {
    ctx.read_parquet(
        &test_data_path("seattle-weather.parquet"),
        ParquetReadOptions::default(),
    )
    .await
}

/// Load the stocks dataset
///
/// Daily stock prices for AAPL, AMZN, GOOG, IBM, MSFT (2000-2010)
/// - Rows: ~560
/// - Columns: symbol, date, price
pub async fn stocks(ctx: &SessionContext) -> Result<DataFrame, DataFusionError> {
    ctx.read_parquet(
        &test_data_path("stocks.parquet"),
        ParquetReadOptions::default(),
    )
    .await
}

/// Load the Iris flower dataset
///
/// Classic dataset for classification (Fisher, 1936)
/// - Rows: 150
/// - Columns: sepal_length, sepal_width, petal_length, petal_width, species
pub async fn iris(ctx: &SessionContext) -> Result<DataFrame, DataFusionError> {
    ctx.read_parquet(
        &test_data_path("iris.parquet"),
        ParquetReadOptions::default(),
    )
    .await
}

/// Load the cars dataset
///
/// Automotive data from the 1970s-1980s
/// - Rows: 406
/// - Columns: Name, Miles_per_Gallon, Cylinders, Displacement, Horsepower, Weight, Acceleration, Year, Origin
pub async fn cars(ctx: &SessionContext) -> Result<DataFrame, DataFusionError> {
    ctx.read_parquet(
        &test_data_path("cars.parquet"),
        ParquetReadOptions::default(),
    )
    .await
}

/// Load the barley dataset
///
/// Agricultural yield data from Minnesota (1931-1932)
/// - Rows: 120
/// - Columns: yield, variety, year, site
pub async fn barley(ctx: &SessionContext) -> Result<DataFrame, DataFusionError> {
    ctx.read_parquet(
        &test_data_path("barley.parquet"),
        ParquetReadOptions::default(),
    )
    .await
}

/// Load the airports dataset
///
/// US airport locations and metadata
/// - Rows: 3,376
/// - Columns: iata, name, city, state, country, latitude, longitude
pub async fn airports(ctx: &SessionContext) -> Result<DataFrame, DataFusionError> {
    ctx.read_parquet(
        &test_data_path("airports.parquet"),
        ParquetReadOptions::default(),
    )
    .await
}

/// Load the CO2 concentration dataset
///
/// Atmospheric CO2 concentration measurements over time
/// - Rows: ~309
/// - Columns: Date, CO2 (parts per million)
pub async fn co2_concentration(ctx: &SessionContext) -> Result<DataFrame, DataFusionError> {
    ctx.read_parquet(
        &test_data_path("co2-concentration.parquet"),
        ParquetReadOptions::default(),
    )
    .await
}

/// Load the unemployment dataset
///
/// US unemployment rates by county over time
/// - Rows: ~3,200
/// - Columns: id (county FIPS), rate (by month)
pub async fn unemployment(ctx: &SessionContext) -> Result<DataFrame, DataFusionError> {
    ctx.read_parquet(
        &test_data_path("unemployment.parquet"),
        ParquetReadOptions::default(),
    )
    .await
}

/// Load the movies dataset
///
/// IMDB movie ratings and metadata
/// - Rows: 3,201
/// - Columns: Title, US Gross, Worldwide Gross, US DVD Sales, Production Budget, Release Date,
///   MPAA Rating, Running Time min, Distributor, Source, Major Genre, Creative Type, Director,
///   Rotten Tomatoes Rating, IMDB Rating, IMDB Votes
pub async fn movies(ctx: &SessionContext) -> Result<DataFrame, DataFusionError> {
    ctx.read_parquet(
        &test_data_path("movies.parquet"),
        ParquetReadOptions::default(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_can_load_all_datasets() {
        let ctx = SessionContext::new();

        // Test loading each dataset
        let datasets = vec![
            ("seattle-weather", seattle_weather(&ctx).await),
            ("stocks", stocks(&ctx).await),
            ("iris", iris(&ctx).await),
            ("cars", cars(&ctx).await),
            ("barley", barley(&ctx).await),
            ("airports", airports(&ctx).await),
            ("co2-concentration", co2_concentration(&ctx).await),
            ("unemployment", unemployment(&ctx).await),
            ("movies", movies(&ctx).await),
        ];

        for (name, result) in datasets {
            let df = result.unwrap_or_else(|_| panic!("Failed to load {}", name));
            let count = df
                .clone()
                .count()
                .await
                .unwrap_or_else(|_| panic!("Failed to count rows in {}", name));
            println!("{}: {} rows", name, count);
            assert!(count > 0, "{} should have rows", name);
        }
    }
}
