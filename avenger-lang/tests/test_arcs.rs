use std::f32::consts::PI;
use std::sync::Arc;

use arrow::array::{ArrayRef, Float32Array, RecordBatch};
use arrow::datatypes::{DataType, Field, Schema};

mod utils;
use avenger_lang::error::AvengerLangError;
use avenger_lang::parser::AvengerParser;
use avenger_lang::task_graph::runtime::TaskGraphRuntime;
use avenger_lang::task_graph::task_graph::TaskGraph;
use datafusion::scalar::ScalarValue;
use utils::assert_runtime_image_equal;
use anyhow::Result;

#[tokio::test]
async fn test_arcs() -> Result<()> {
    
    let src = r#"
    width := 200;
    height := 200;
    
    dataset data_0: SELECT * FROM (VALUES 
            (1, 'red'),
            (2, 'green'),
            (3, 'blue')
        ) foo("a", "b");

    val pi: 3.14159;
    
    comp g1: Group {
        x := 20;
        y := 20;
    
        comp mark1: Arc {
            data := SELECT * FROM @data_0;
            x := ("a" - 1) * 40;
            y := ("a" - 1) * 40 + 50;

            start_angle := "a" * @pi / 8.0;
            end_angle := @start_angle + @pi / 4.0;

            inner_radius := 30;
            outer_radius := 50;
            fill := "b";
            stroke_width := 4;
            stroke := 'black';

            clip := false;
            zindex := 1 + 2;
        }
    }
    "#;

    let file = AvengerParser::parse_single_file(src)?;
    let task_graph = Arc::new(TaskGraph::try_from(&file)?);
    let runtime = TaskGraphRuntime::new();
    let scene_graph = runtime.evaluate_file(&file).await?;

    assert_runtime_image_equal(&scene_graph, "test_arcs", 2.0, true).await?;
    Ok(())
}