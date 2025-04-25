use std::f32::consts::PI;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::Arc;

use arrow::array::{ArrayRef, Float32Array, RecordBatch};
use arrow::datatypes::{DataType, Field, Schema};
use rstest::rstest;

mod utils;
use avenger_lang::ast::AvengerFile;
use avenger_lang::error::AvengerLangError;
use avenger_lang::parser::AvengerParser;
use avenger_lang::task_graph::runtime::TaskGraphRuntime;
use avenger_lang::task_graph::task_graph::TaskGraph;
use datafusion::scalar::ScalarValue;
use utils::assert_runtime_image_equal;
use anyhow::Result;

#[rstest]
// #[case("arcs/simple")]
// #[case("arcs/simple2")]
#[case("components/custom_component")]
#[tokio::test]
async fn test_baselines(#[case] path: &str) -> Result<()> {
    let file = load_file(&format!("{path}.avgr"))?;
    let runtime = TaskGraphRuntime::new();
    let scene_graph = runtime.evaluate_file(&file).await?;
    assert_runtime_image_equal(&scene_graph, path, 2.0, true).await?;
    Ok(())
}

fn load_file(name: &str) -> Result<AvengerFile> {
    let path = Path::new("tests/baselines").join(name);
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut contents = String::new();
    reader.read_to_string(&mut contents)?;
    let file = AvengerParser::parse_single_file(&contents)?;
    Ok(file)
}
