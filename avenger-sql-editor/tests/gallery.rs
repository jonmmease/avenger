//! The examples of the gallery: how `examples.md` reads, and that every example runs against the
//! sample tables and draws each of its charts at the size of a thumbnail.

use avenger_sql_editor::charts::{cascade, engine, StaticChart};
use avenger_sql_editor::gallery::{examples, parse, Example};
use avenger_sql_editor::{query, samples};

/// The size of a thumbnail's evaluation, in logical pixels.
const THUMBNAIL: [f32; 2] = [480.0, 300.0];

#[test]
fn a_heading_its_prose_and_its_query_are_an_example() {
    let text = "\
# Examples

An introduction, which is no description.

```sql
SELECT 'not an example'
```

## Bars

The prose of a section.

### Counts

The number of each species.
A line of the same paragraph.

A second paragraph.

```bash
echo 'not the query'
```

```sql
FROM samples.penguins
|> AGGREGATE mark_bar(x => species, y => count(*))
```

Prose after the query.

```sql
SELECT 'a second block'
```

### Without a query

Only prose.

## Points

### Bills

Bill depth against bill length.

```sql
FROM samples.penguins |> AGGREGATE mark_symbol(x => bill_length_mm, y => bill_depth_mm)
```
";
    assert_eq!(
        parse(text),
        [
            Example {
                section: "Bars".into(),
                title: "Counts".into(),
                description: "The number of each species. A line of the same paragraph.\n\nA second paragraph."
                    .into(),
                sql: "FROM samples.penguins\n|> AGGREGATE mark_bar(x => species, y => count(*))".into(),
            },
            Example {
                section: "Points".into(),
                title: "Bills".into(),
                description: "Bill depth against bill length.".into(),
                sql: "FROM samples.penguins |> AGGREGATE mark_symbol(x => bill_length_mm, y => bill_depth_mm)"
                    .into(),
            },
        ]
    );
}

#[test]
fn each_heading_of_the_examples_is_an_example_with_a_description() {
    let text = include_str!("../examples.md");
    let headings = text.lines().filter(|line| line.starts_with("### ")).count();
    assert_eq!(examples().len(), headings);
    for example in examples() {
        let name = format!("{} / {}", example.section, example.title);
        assert!(!example.section.is_empty(), "{name}");
        assert!(!example.description.is_empty(), "{name}");
        assert!(!example.description.ends_with(':'), "{name}");
        // A sample table reads by its schema, as `samples.penguins`.
        for (table, _) in samples::TABLES {
            assert!(!example.sql.contains(&format!("FROM {table}")), "{name}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_example_runs_and_draws_its_charts() {
    let ctx = query::session().unwrap();
    samples::register(&ctx).await.unwrap();
    let (cascade, engine) = (cascade(), engine());
    let mut failures = Vec::new();
    for example in examples() {
        let name = format!("{} / {}", example.section, example.title);
        let outcome = match query::run(&ctx, &example.sql).await {
            Ok(outcome) => outcome,
            Err(error) => {
                failures.push(format!("{name}: {}", error.message));
                continue;
            }
        };
        let result = &outcome.result;
        let mut charts = 0;
        for row in 0..result.num_rows() {
            for column in result.chart_columns() {
                let Some(chart) = result.chart(row, column) else {
                    continue;
                };
                charts += 1;
                let scene = StaticChart::new(chart.clone(), cascade.clone(), engine.clone())
                    .and_then(|mut chart| chart.scene(THUMBNAIL));
                if let Err(error) = scene {
                    failures.push(format!("{name}, row {row}, column {column}: {error}"));
                }
            }
        }
        if charts == 0 {
            failures.push(format!("{name}: no chart"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The lifted form of each example lifts to itself, with nothing more to write.
#[tokio::test(flavor = "multi_thread")]
async fn every_example_lifts() {
    let ctx = query::session().unwrap();
    samples::register(&ctx).await.unwrap();
    let mut failures = Vec::new();
    for example in examples() {
        let name = format!("{} / {}", example.section, example.title);
        let lifted = match avenger_sql::lift(&ctx, &example.sql).await {
            Ok(lifted) => lifted,
            Err(error) => {
                failures.push(format!("{name}: {error}"));
                continue;
            }
        };
        match avenger_sql::lift(&ctx, &lifted.text).await {
            Ok(again) if again.text == lifted.text && again.generated.is_empty() => {}
            Ok(again) => failures.push(format!("{name}: lifts again to\n{}", again.text)),
            Err(error) => failures.push(format!("{name}: its lifting fails: {error}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_example_is_in_the_style() {
    for example in examples() {
        assert_eq!(
            avenger_sql_syntax::format::format(&example.sql, avenger_sql_syntax::Role::Script),
            example.sql,
            "{} is not formatted",
            example.title
        );
    }
}
