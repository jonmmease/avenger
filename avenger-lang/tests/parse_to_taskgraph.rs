use std::sync::Arc;

use avenger_lang::{ast::AvengerFile, context::EvaluationContext, error::AvengerLangError, parser::AvengerParser, task_graph::{dependency::{Dependency, DependencyKind}, runtime::TaskGraphRuntime, task_graph::TaskGraph, variable::Variable}};


#[tokio::test]
async fn test_parse_file_to_taskgraph() -> Result<(), AvengerLangError> {
    let src = r#"
    // This is a comment
    in val<int> my_val: 1 + 23;
    dataset my_dataset: select @my_val * 2 as my_val_2;
    out expr my_expr: @my_val + 1;
    "#;
    
    // Create a new parser with the tokens and parse the file
    let parser = AvengerParser::new();
    let tokens = parser.tokenize(src).unwrap();
    let mut parser = parser.with_tokens_with_locations(tokens);
    let file = parser.parse().unwrap();
    println!("{:#?}", file);

    // Build Task Graph from parsed file
    let task_graph = Arc::new(TaskGraph::try_from(file)?);
    println!("{:#?}", task_graph);

    // Evaluate the task graph
    let my_val = Variable::new("my_val");
    let my_expr = Variable::new("my_expr");
    let my_dataset = Variable::new("my_dataset");

    let runtime = TaskGraphRuntime::new();

    // Eval root variable
    let vals = runtime.evaluate_variables(
        task_graph.clone(), &[my_val.clone()]
    ).await?;

    let my_val_val = vals.get(&my_val).unwrap();
    println!("my_val: {:#?}", my_val_val);

    // Eval dataset variable
    let vals = runtime.evaluate_variables(
        task_graph.clone(), &[my_dataset.clone()]
    ).await?;

    let my_dataset_val = vals.get(&my_dataset).unwrap();
    let my_dataset_val = my_dataset_val.as_dataset().unwrap();
    let ctx = EvaluationContext::new();
    let my_dataset_df = ctx.session_ctx().execute_logical_plan(my_dataset_val.clone()).await?;

    println!("my_dataset");
    my_dataset_df.show().await?;
    Ok(())
}

