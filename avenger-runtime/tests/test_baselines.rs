use std::f32::consts::PI;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{ArrayRef, Float32Array, RecordBatch};
use arrow::datatypes::{DataType, Field, Schema};
use avenger_lang::loader::{AvengerFilesystemLoader, AvengerLoader};
use rstest::rstest;

mod utils;
use anyhow::Result;
use datafusion::scalar::ScalarValue;
use utils::assert_runtime_image_equal;

#[cfg(test)]
mod tests {
    use super::*;
    #[rstest]
    // #[case("arcs/simple")]
    #[case("scale/iris")]
    // #[case("components/custom_component")]
    // #[case("scale/simple")]
    #[tokio::test]
    async fn test_baselines(#[case] path: &str) -> Result<()> {
        use avenger_lang::imports::load_main_component_file;
        use avenger_runtime::{cache::RuntimeCacheConfig, runtime::TaskGraphRuntime};

        let file_ast = load_main_component_file(
            PathBuf::from(format!("tests/baselines/{}", path)).join("App.avgr"),
            true,
        )?;
        println!("{}", file_ast);
        let runtime = Arc::new(TaskGraphRuntime::new(RuntimeCacheConfig::default()));
        let scene_graph = runtime.evaluate_file(&file_ast).await?;
        assert_runtime_image_equal(&scene_graph, path, 2.0, true).await?;
        Ok(())
    }
}