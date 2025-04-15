use std::sync::Arc;

use avenger_lang::{ast::AvengerFile, context::EvaluationContext, error::AvengerLangError, parser::AvengerParser, task_graph::{dependency::{Dependency, DependencyKind}, runtime::TaskGraphRuntime, task_graph::TaskGraph, value::{ArrowTable, TaskDataset}, variable::Variable}};


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

    let my_dataset_val = vals.get(&my_dataset).unwrap().clone();
    let (my_dataset_val, task_value_context) = my_dataset_val.into_dataset().unwrap();

    let ctx = EvaluationContext::new();
    ctx.register_task_value_context(&task_value_context).await?;

    let table = match my_dataset_val {
        TaskDataset::LogicalPlan(plan) => {
            let my_dataset_df = ctx.session_ctx().execute_logical_plan(plan.clone()).await?;
            ArrowTable::from_dataframe(my_dataset_df).await?
        }
        TaskDataset::ArrowTable(table) => {
            table
        }
    };

    println!("my_dataset");
    table.show()?;
    Ok(())
}

