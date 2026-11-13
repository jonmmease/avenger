//! The node of `MATCH_RECOGNIZE` in DataFusion plans: inputs whose positions and variables SQL
//! computes, as a front end does, and measures that an aggregate over the node's output
//! computes, against the results of `execute_query` for the same GoogleSQL queries, which the
//! tests name by their cases in the research notes of `MATCH_RECOGNIZE`.

mod common;

use std::sync::Arc;

use avenger_datafusion_transforms::match_recognize::{
    MAX_INSTRUCTIONS, MatchRecognizeCall, Pattern, Skip, plan,
};
use datafusion::arrow::array::{ArrayRef, Int64Array, RecordBatch, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::json::writer::{JsonArray, WriterBuilder};
use datafusion::catalog::MemTable;
use datafusion::common::{Column, Result};
use datafusion::dataframe::DataFrame;
use datafusion::prelude::{SessionConfig, SessionContext};

// --- Patterns ---

fn var(index: usize) -> Pattern {
    Pattern::Variable(index)
}

fn cat(parts: Vec<Pattern>) -> Pattern {
    Pattern::Concat(parts)
}

fn repeat(body: Pattern, min: u32, max: Option<u32>, greedy: bool) -> Pattern {
    Pattern::Repeat {
        body: Box::new(body),
        min,
        max,
        greedy,
    }
}

// --- Data ---

fn ints(values: &[Option<i64>]) -> ArrayRef {
    Arc::new(Int64Array::from(values.to_vec()))
}

fn strings(values: &[Option<&str>]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

/// The table `ticks`: two symbols with interleaved ids, `TICKS` of the research notes.
fn ticks(ctx: &SessionContext) -> Result<()> {
    common::register(
        ctx,
        "ticks",
        vec![
            (
                "sym",
                strings(&[
                    Some("a"),
                    Some("b"),
                    Some("a"),
                    Some("b"),
                    Some("a"),
                    Some("b"),
                    Some("a"),
                ]),
            ),
            ("id", ints(&[1, 2, 3, 4, 5, 6, 7].map(Some))),
            ("price", ints(&[5, 50, 4, 40, 6, 45, 3].map(Some))),
        ],
    )
}

/// The table `flags`: 1 for up and 0 for down, `FLAGS` of the research notes.
fn flags(ctx: &SessionContext) -> Result<()> {
    common::register(
        ctx,
        "flags",
        vec![
            ("id", ints(&[1, 2, 3, 4, 5, 6, 7].map(Some))),
            ("f", ints(&[1, 1, 0, 1, 1, 1, 0].map(Some))),
        ],
    )
}

/// The table `nulls`: NULL ids, keys, and values, `NULLS` of the research notes.
fn nulls(ctx: &SessionContext) -> Result<()> {
    common::register(
        ctx,
        "nulls",
        vec![
            ("id", ints(&[Some(1), Some(2), Some(3), Some(4), None])),
            ("k", strings(&[Some("x"), Some("x"), None, None, Some("x")])),
            ("v", ints(&[Some(1), None, Some(3), Some(4), Some(5)])),
        ],
    )
}

// --- Running a call ---

/// The call of `pattern` over the columns `pos` and `variables`, by `partition_by`.
fn call(partition_by: &[&str], variables: &[&str], pattern: Pattern) -> MatchRecognizeCall {
    MatchRecognizeCall {
        partition_by: partition_by.iter().map(|c| Column::from_name(*c)).collect(),
        position: Column::from_name("pos"),
        variables: variables.iter().map(|c| Column::from_name(*c)).collect(),
        names: variables.iter().map(|c| c.to_string()).collect(),
        pattern,
        skip: Skip::PastLastRow,
        longest: false,
        match_number: "m".to_string(),
        classifier: "cls".to_string(),
    }
}

/// The rows of `measures` as JSON, a query of the table `matched`, which holds the output of
/// `call` over the query `input`.
async fn measures(
    ctx: &SessionContext,
    input: &str,
    call: MatchRecognizeCall,
    measures: &str,
) -> Result<String> {
    let input = ctx.sql(input).await?.into_unoptimized_plan();
    let matched = DataFrame::new(ctx.state(), plan(input, call)?);
    ctx.deregister_table("matched")?;
    ctx.register_table("matched", matched.into_view())?;
    let batches = ctx.sql(measures).await?.collect().await?;
    json(&batches)
}

fn json(batches: &[RecordBatch]) -> Result<String> {
    let mut writer = WriterBuilder::new()
        .with_explicit_nulls(true)
        .build::<_, JsonArray>(Vec::new());
    for batch in batches {
        writer.write(batch)?;
    }
    writer.finish()?;
    let out = String::from_utf8(writer.into_inner()).expect("JSON is UTF-8");
    // An empty result writes nothing, where `execute_query` gives `[]`.
    Ok(if out.is_empty() {
        "[]".to_string()
    } else {
        out
    })
}

fn session() -> SessionContext {
    common::session(true)
}

// --- Partitions ---

#[tokio::test]
async fn partitions_match_apart_in_their_order() -> Result<()> {
    let ctx = session();
    ticks(&ctx)?;
    // partition_by: PARTITION BY sym ORDER BY id, PATTERN (a b+), a AS TRUE,
    // b AS price < PREV(price).
    let out = measures(
        &ctx,
        "SELECT sym, id, price,
            ROW_NUMBER() OVER (PARTITION BY sym ORDER BY id NULLS FIRST) AS pos,
            TRUE AS a,
            price < LAG(price) OVER (PARTITION BY sym ORDER BY id NULLS FIRST) AS b
         FROM ticks",
        call(
            &["sym"],
            &["a", "b"],
            cat(vec![var(0), repeat(var(1), 1, None, true)]),
        ),
        "SELECT sym, MIN(id) AS s, MAX(id) AS e, m FROM matched GROUP BY sym, m ORDER BY sym, s",
    )
    .await?;
    assert_eq!(
        out,
        r#"[{"sym":"a","s":1,"e":3,"m":1},{"sym":"a","s":5,"e":7,"m":2},{"sym":"b","s":2,"e":4,"m":1}]"#
    );
    Ok(())
}

#[tokio::test]
async fn a_partition_split_across_batches_matches_as_one() -> Result<()> {
    let ctx = SessionContext::new_with_config(SessionConfig::new().with_target_partitions(4));
    avenger_datafusion_transforms::register(&ctx)?;
    // The rows of `ticks` in two batches of two partitions of the table.
    let schema = Arc::new(Schema::new(vec![
        Field::new("sym", DataType::Utf8, true),
        Field::new("id", DataType::Int64, true),
        Field::new("price", DataType::Int64, true),
    ]));
    let batch = |sym: &[&str], id: &[i64], price: &[i64]| {
        RecordBatch::try_new(
            schema.clone(),
            vec![
                strings(&sym.iter().map(|s| Some(*s)).collect::<Vec<_>>()),
                ints(&id.iter().map(|v| Some(*v)).collect::<Vec<_>>()),
                ints(&price.iter().map(|v| Some(*v)).collect::<Vec<_>>()),
            ],
        )
    };
    let first = batch(&["a", "b", "a"], &[5, 6, 7], &[6, 45, 3])?;
    let second = batch(&["a", "b", "a", "b"], &[1, 2, 3, 4], &[5, 50, 4, 40])?;
    ctx.register_table(
        "ticks",
        Arc::new(MemTable::try_new(schema, vec![vec![first], vec![second]])?),
    )?;
    let out = measures(
        &ctx,
        "SELECT sym, id, price,
            ROW_NUMBER() OVER (PARTITION BY sym ORDER BY id NULLS FIRST) AS pos,
            TRUE AS a,
            price < LAG(price) OVER (PARTITION BY sym ORDER BY id NULLS FIRST) AS b
         FROM ticks",
        call(
            &["sym"],
            &["a", "b"],
            cat(vec![var(0), repeat(var(1), 1, None, true)]),
        ),
        "SELECT sym, MIN(id) AS s, MAX(id) AS e, m FROM matched GROUP BY sym, m ORDER BY sym, s",
    )
    .await?;
    assert_eq!(
        out,
        r#"[{"sym":"a","s":1,"e":3,"m":1},{"sym":"a","s":5,"e":7,"m":2},{"sym":"b","s":2,"e":4,"m":1}]"#
    );
    Ok(())
}

#[tokio::test]
async fn a_null_key_is_a_partition() -> Result<()> {
    let ctx = session();
    nulls(&ctx)?;
    // partition_by_null_key: PARTITION BY k ORDER BY id, PATTERN (a+), a AS TRUE.
    let out = measures(
        &ctx,
        "SELECT id, k, v,
            ROW_NUMBER() OVER (PARTITION BY k ORDER BY id NULLS FIRST) AS pos,
            TRUE AS a
         FROM nulls",
        call(&["k"], &["a"], repeat(var(0), 1, None, true)),
        "SELECT k, COUNT(*) FILTER (WHERE cls IS NOT NULL) AS n,
            ARRAY_AGG(id ORDER BY pos) FILTER (WHERE cls IS NOT NULL) AS ids
         FROM matched GROUP BY k, m ORDER BY k NULLS FIRST",
    )
    .await?;
    assert_eq!(
        out,
        r#"[{"k":null,"n":2,"ids":[3,4]},{"k":"x","n":3,"ids":[null,1,2]}]"#
    );
    Ok(())
}

// --- Rows and their variables ---

#[tokio::test]
async fn each_row_carries_its_variable() -> Result<()> {
    let ctx = session();
    flags(&ctx)?;
    // greedy_star: ORDER BY id, PATTERN (u* x), u AS f = 1, x AS TRUE.
    let out = measures(
        &ctx,
        "SELECT id, f, ROW_NUMBER() OVER (ORDER BY id NULLS FIRST) AS pos,
            f = 1 AS u, TRUE AS x
         FROM flags",
        call(
            &[],
            &["u", "x"],
            cat(vec![repeat(var(0), 0, None, true), var(1)]),
        ),
        "SELECT ids, cls FROM (
            SELECT ARRAY_AGG(id ORDER BY pos) AS ids, STRING_AGG(cls, '' ORDER BY pos) AS cls, m
            FROM matched GROUP BY m
         ) ORDER BY m",
    )
    .await?;
    assert_eq!(
        out,
        r#"[{"ids":[1,2,3],"cls":"uux"},{"ids":[4,5,6,7],"cls":"uuux"}]"#
    );
    Ok(())
}

#[tokio::test]
async fn a_null_condition_is_false() -> Result<()> {
    let ctx = session();
    nulls(&ctx)?;
    // define_null_is_false: ORDER BY id, PATTERN (x+), x AS v > 0.
    let out = measures(
        &ctx,
        "SELECT id, k, v, ROW_NUMBER() OVER (ORDER BY id NULLS FIRST) AS pos, v > 0 AS x
         FROM nulls",
        call(&[], &["x"], repeat(var(0), 1, None, true)),
        "SELECT ids FROM (
            SELECT ARRAY_AGG(id ORDER BY pos) AS ids, m FROM matched GROUP BY m
         ) ORDER BY m",
    )
    .await?;
    assert_eq!(out, r#"[{"ids":[null,1]},{"ids":[3,4]}]"#);
    Ok(())
}

// --- Empty matches ---

#[tokio::test]
async fn an_empty_match_is_one_row_without_a_variable() -> Result<()> {
    let ctx = session();
    flags(&ctx)?;
    // star_yields_empty_matches: ORDER BY id, PATTERN (d*), d AS f = 0.
    let out = measures(
        &ctx,
        "SELECT id, f, ROW_NUMBER() OVER (ORDER BY id NULLS FIRST) AS pos, f = 0 AS d
         FROM flags",
        call(&[], &["d"], repeat(var(0), 0, None, true)),
        "SELECT COUNT(*) FILTER (WHERE cls IS NOT NULL) AS n, MIN(id) AS s, m
         FROM matched GROUP BY m ORDER BY m",
    )
    .await?;
    assert_eq!(
        out,
        r#"[{"n":0,"s":null,"m":1},{"n":0,"s":null,"m":2},{"n":1,"s":3,"m":3},{"n":0,"s":null,"m":4},{"n":0,"s":null,"m":5},{"n":0,"s":null,"m":6},{"n":1,"s":7,"m":7}]"#
    );
    Ok(())
}

#[tokio::test]
async fn an_empty_input_has_no_matches() -> Result<()> {
    let ctx = session();
    common::register(&ctx, "empty", vec![("id", ints(&[])), ("x", ints(&[]))])?;
    // empty_input_no_matches: ORDER BY id, PATTERN (a*), a AS TRUE.
    let out = measures(
        &ctx,
        "SELECT id, x, ROW_NUMBER() OVER (ORDER BY id NULLS FIRST) AS pos, TRUE AS a
         FROM empty",
        call(&[], &["a"], repeat(var(0), 0, None, true)),
        "SELECT COUNT(*) FILTER (WHERE cls IS NOT NULL) AS n FROM matched GROUP BY m",
    )
    .await?;
    assert_eq!(out, "[]");
    Ok(())
}

// --- Options ---

#[tokio::test]
async fn skip_to_next_row_overlaps_matches() -> Result<()> {
    let ctx = session();
    flags(&ctx)?;
    // skip_to_next_row_overlaps: ORDER BY id, AFTER MATCH SKIP TO NEXT ROW, PATTERN (u u),
    // u AS f = 1.
    let mut call = call(&[], &["u"], cat(vec![var(0), var(0)]));
    call.skip = Skip::ToNextRow;
    let out = measures(
        &ctx,
        "SELECT id, f, ROW_NUMBER() OVER (ORDER BY id NULLS FIRST) AS pos, f = 1 AS u
         FROM flags",
        call,
        "SELECT ARRAY_AGG(id ORDER BY pos) AS ids, m FROM matched GROUP BY m ORDER BY m",
    )
    .await?;
    assert_eq!(
        out,
        r#"[{"ids":[1,2],"m":1},{"ids":[4,5],"m":2},{"ids":[5,6],"m":3}]"#
    );
    Ok(())
}

#[tokio::test]
async fn the_longest_match_wins_with_use_longest_match() -> Result<()> {
    let ctx = session();
    flags(&ctx)?;
    // longest_match_option: ORDER BY id, PATTERN (u | u u), u AS f = 1,
    // OPTIONS (use_longest_match = TRUE).
    let mut call = call(
        &[],
        &["u"],
        Pattern::Alternation(vec![var(0), cat(vec![var(0), var(0)])]),
    );
    call.longest = true;
    let out = measures(
        &ctx,
        "SELECT id, f, ROW_NUMBER() OVER (ORDER BY id NULLS FIRST) AS pos, f = 1 AS u
         FROM flags",
        call,
        "SELECT ids FROM (
            SELECT ARRAY_AGG(id ORDER BY pos) AS ids, m FROM matched GROUP BY m
         ) ORDER BY m",
    )
    .await?;
    assert_eq!(out, r#"[{"ids":[1,2]},{"ids":[4,5]},{"ids":[6]}]"#);
    Ok(())
}

// --- The node ---

#[tokio::test]
async fn the_output_is_the_input_and_the_match_columns() -> Result<()> {
    let ctx = session();
    flags(&ctx)?;
    let input = (ctx
        .sql("SELECT id, f, ROW_NUMBER() OVER (ORDER BY id) AS pos, f = 1 AS u FROM flags"))
    .await?
    .into_unoptimized_plan();
    let node = plan(input, call(&[], &["u"], var(0)))?;
    let fields: Vec<String> = (node.schema().fields().iter())
        .map(|f| format!("{}: {} {}", f.name(), f.data_type(), f.is_nullable()))
        .collect();
    assert_eq!(
        fields,
        [
            "id: Int64 true",
            "f: Int64 true",
            "pos: UInt64 true",
            "m: Int64 false",
            "cls: Utf8 true"
        ]
    );
    let explain = node.display_indent().to_string();
    assert!(
        explain.starts_with(
            "MatchRecognize: pattern=Variable(0), variables=[u=u], position=pos, skip=PastLastRow, longest=false"
        ),
        "{explain}"
    );
    Ok(())
}

#[tokio::test]
async fn a_pattern_over_the_limit_is_a_plan_error() -> Result<()> {
    let ctx = session();
    flags(&ctx)?;
    let input = (ctx
        .sql("SELECT id, f, ROW_NUMBER() OVER (ORDER BY id) AS pos, f = 1 AS u FROM flags"))
    .await?
    .into_unoptimized_plan();
    let max = MAX_INSTRUCTIONS as u32;
    let pattern = repeat(var(0), max, Some(max), true);
    let error = plan(input, call(&[], &["u"], pattern)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("compiles to more than 100000 instructions"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
async fn a_variable_is_a_bool_column() -> Result<()> {
    let ctx = session();
    flags(&ctx)?;
    let input = (ctx.sql("SELECT id, f, ROW_NUMBER() OVER (ORDER BY id) AS pos FROM flags"))
        .await?
        .into_unoptimized_plan();
    let error = plan(input, call(&[], &["f"], var(0))).unwrap_err();
    assert!(error.to_string().contains("is a BOOL"), "{error}");
    Ok(())
}
