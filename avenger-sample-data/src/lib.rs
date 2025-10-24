use std::path::{Path, PathBuf};

pub fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data")
}

pub fn iris_path() -> String {
    data_dir()
        .join("iris.parquet")
        .to_str()
        .unwrap()
        .to_string()
}

pub fn stocks_path() -> String {
    data_dir()
        .join("stocks.parquet")
        .to_str()
        .unwrap()
        .to_string()
}

pub fn movies_path() -> String {
    data_dir()
        .join("movies.parquet")
        .to_str()
        .unwrap()
        .to_string()
}

pub fn seattle_weather_path() -> String {
    data_dir()
        .join("seattle-weather.parquet")
        .to_str()
        .unwrap()
        .to_string()
}

pub fn cars_path() -> String {
    data_dir()
        .join("cars.parquet")
        .to_str()
        .unwrap()
        .to_string()
}

pub fn barley_path() -> String {
    data_dir()
        .join("barley.parquet")
        .to_str()
        .unwrap()
        .to_string()
}

pub fn airports_path() -> String {
    data_dir()
        .join("airports.parquet")
        .to_str()
        .unwrap()
        .to_string()
}

pub fn co2_concentration_path() -> String {
    data_dir()
        .join("co2-concentration.parquet")
        .to_str()
        .unwrap()
        .to_string()
}

pub fn unemployment_path() -> String {
    data_dir()
        .join("unemployment.parquet")
        .to_str()
        .unwrap()
        .to_string()
}
