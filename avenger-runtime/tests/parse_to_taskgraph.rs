use std::sync::Arc;

use avenger_lang::{ast::AvengerFile, parser::AvengerParser};
use avenger_runtime::{cache::RuntimeCacheConfig, context::TaskEvaluationContext, error::AvengerRuntimeError, runtime::TaskGraphRuntime, task_graph::TaskGraph, value::{ArrowTable, TaskDataset}, variable::Variable};


fn parse_file(src: &str) -> Result<AvengerFile, AvengerRuntimeError> {
    let mut parser = AvengerParser::new(src, "Foo", "/path/to/components")?;
    let file = parser.parse()?;
    Ok(file)
}

#[tokio::test]
async fn test_parse_file_to_taskgraph() -> Result<(), AvengerRuntimeError> {
    let src = r#"
    // This is a comment
    in val<int> my_val: 1 + 23;
    dataset my_dataset: select @my_val * 2 as my_val_2;
    out expr my_expr: @my_val + "some_col";
    out dataset my_dataset2:
        with a as (select 23 as "some_col")
        select @my_expr * 3 as my_val_3 from a;

    dataset my_dataset3: select my_val_3 * 2 as another_col from @my_dataset2;
    "#;

    // Create a new parser with the tokens and parse the file
    let file_ast = parse_file(src)?;
    // println!("{:#?}", file);

    // Build Task Graph from parsed file
    let task_graph = Arc::new(TaskGraph::from_file(&file_ast)?);
    // println!("{:#?}", task_graph);

    // Evaluate the task graph
    let my_val = Variable::new(vec!["my_val".to_string()]);
    let my_expr = Variable::new(vec!["my_expr".to_string()]);
    let my_dataset = Variable::new(vec!["my_dataset".to_string()]);
    let my_dataset2 = Variable::new(vec!["my_dataset2".to_string()]);
    let my_dataset3 = Variable::new(vec!["my_dataset3".to_string()]);

    let cache_config = RuntimeCacheConfig::default();
    let runtime = Arc::new(TaskGraphRuntime::new(cache_config));

    // Eval root variable
    let vals = runtime
        .clone()
        .evaluate_variables(task_graph.clone(), &[my_val.clone()])
        .await?;

    let my_val_val = vals.get(&my_val).unwrap();
    println!("my_val: {:#?}", my_val_val);

    // Eval dataset variable
    let vals = runtime
        .clone()
        .evaluate_variables(task_graph.clone(), &[my_dataset.clone()])
        .await?;

    let my_dataset_val = vals.get(&my_dataset).unwrap().clone();
    let (my_dataset_val, task_value_context) = my_dataset_val.into_dataset().unwrap();

    let ctx = TaskEvaluationContext::new();
    ctx.register_task_value_context(&task_value_context).await?;

    let table = match my_dataset_val {
        TaskDataset::LogicalPlan(plan) => {
            let my_dataset_df = ctx.session_ctx().execute_logical_plan(plan.clone()).await?;
            ArrowTable::from_dataframe(my_dataset_df).await?
        }
        TaskDataset::ArrowTable(table) => table,
    };

    println!("my_dataset");
    table.show()?;

    // Eval expr variable
    let vals = runtime
        .clone()
        .evaluate_variables(task_graph.clone(), &[my_expr.clone()])
        .await?;

    let my_expr_val = vals.get(&my_expr).unwrap();
    println!("my_expr: {:#?}", my_expr_val);

    // Eval dataset2 variable
    let vals = runtime
        .clone()
        .evaluate_variables(task_graph.clone(), &[my_dataset2.clone()])
        .await?;

    let my_dataset2_val = vals.get(&my_dataset2).unwrap().clone();
    let (my_dataset2_val, task_value_context) = my_dataset2_val.into_dataset().unwrap();

    let ctx = TaskEvaluationContext::new();
    ctx.register_task_value_context(&task_value_context).await?;

    let table = match my_dataset2_val {
        TaskDataset::LogicalPlan(plan) => {
            let my_dataset2_df = ctx.session_ctx().execute_logical_plan(plan.clone()).await?;
            ArrowTable::from_dataframe(my_dataset2_df).await?
        }
        TaskDataset::ArrowTable(table) => table,
    };

    println!("my_dataset2");
    table.show()?;

    // Eval dataset3 variable
    let vals = runtime
        .clone()
        .evaluate_variables(task_graph.clone(), &[my_dataset3.clone()])
        .await?;

    let my_dataset3_val = vals.get(&my_dataset3).unwrap().clone();
    let (my_dataset3_val, task_value_context) = my_dataset3_val.into_dataset().unwrap();

    let ctx = TaskEvaluationContext::new();
    ctx.register_task_value_context(&task_value_context).await?;

    let table = match my_dataset3_val {
        TaskDataset::LogicalPlan(plan) => {
            let my_dataset3_df = ctx.session_ctx().execute_logical_plan(plan.clone()).await?;
            ArrowTable::from_dataframe(my_dataset3_df).await?
        }
        TaskDataset::ArrowTable(table) => table,
    };

    println!("my_dataset3");
    table.show()?;

    Ok(())
}

#[tokio::test]
async fn test_parse_file_to_taskgraph2() -> Result<(), AvengerRuntimeError> {
    let src = r#"
    // This is a comment
    in val<int> my_val: 1 + 23;

    val depends_on_nested_val: @foo.foo_val * 2;

    dataset upper_dataset: SELECT a, UPPER(b) as b FROM @foo.foo_dataset;

    comp foo: Arc {
        val foo_val: -@my_val;
        dataset foo_dataset: 
            SELECT * FROM 
            (VALUES (1, 'one'), (2, 'two'), (3, 'three')) foo("a", "b");

        // This should resolve to foo.foo_val.
        val bar_val: @foo_val * 2;
    }
    "#;

    // Build Task Graph from parsed file
    let file = parse_file(src)?;
    let task_graph = Arc::new(TaskGraph::from_file(&file)?);

    // Evaluate the task graph
    let my_val = Variable::new(vec!["my_val".to_string()]);
    let upper_dataset = Variable::new(vec!["upper_dataset".to_string()]);
    let foo_val = Variable::new(vec!["foo".to_string(), "foo_val".to_string()]);
    let depends_on_nested_val = Variable::new(vec!["depends_on_nested_val".to_string()]);
    let bar_val = Variable::new(vec!["foo".to_string(), "bar_val".to_string()]);

    let cache_config = RuntimeCacheConfig::default();
    let runtime = Arc::new(TaskGraphRuntime::new(cache_config));

    // Eval root variable
    let vals = runtime
        .clone()
        .evaluate_variables(
            task_graph.clone(),
            &[
                foo_val.clone(),
                depends_on_nested_val.clone(),
                upper_dataset.clone(),
                bar_val.clone(),
            ],
        )
        .await?;

    let foo_val_val = vals.get(&foo_val).unwrap();
    println!("foo_val: {:#?}", foo_val_val);

    let depends_on_nested_val_val = vals.get(&depends_on_nested_val).unwrap();
    println!("depends_on_nested_val: {:#?}", depends_on_nested_val_val);

    let upper_dataset_val = vals.get(&upper_dataset).unwrap();
    println!("upper_dataset: {:#?}", upper_dataset_val);

    let bar_val_val = vals.get(&bar_val).unwrap();
    println!("bar_val: {:#?}", bar_val_val);

    Ok(())
}

#[tokio::test]
async fn test_parse_file_to_scene_graph() -> Result<(), AvengerRuntimeError> {
    let src = r#"
    width := 440;
    height := 440;

    dataset data_0: SELECT * FROM (VALUES
            (1, 'red'),
            (2, 'green'),
            (3, 'blue')
        ) foo("a", "b");

    comp g1: Group {
        x := 20;
        y := 20;

        comp mark1: Rect {
            data := SELECT * FROM @data_0;
            x2 := @x + 10;
            x := "a" * 100;
            y := "a" * 10 + 10;
            y2 := 0;
            fill := "b";
            stroke_width := 4;
            stroke := 'black';
            clip := false;
            zindex := 1 + 2;
        }
    }
    "#;

    // Build Task Graph from parsed file
    let file = parse_file(src)?;

    let cache_config = RuntimeCacheConfig::default();
    let runtime = Arc::new(TaskGraphRuntime::new(cache_config));

    let scene_graph = runtime.evaluate_file(&file).await?;
    println!("scene_graph: {:#?}", scene_graph);

    Ok(())
}

// #[test]
// fn test_parse_sql_error() -> Result<(), AvengerLangError> {
//     match AvengerParser::parse_single_expr("2 + 3") {
//         Ok(ast) => {
//             println!("{:?}", ast);
//         }
//         Err(err) => {
//             println!("{:?}", err);
//         }
//     }

//     Ok(())
// }
