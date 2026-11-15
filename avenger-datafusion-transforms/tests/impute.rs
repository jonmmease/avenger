//! `impute` against reference SQL: the key values, the groups, the join of each group to the
//! key values, an anti join to the input, the imputed value, and a `UNION ALL` with the input.
//! Then the errors of its arguments, at their arguments, and its edge cases.
//!
//! Each case builds an [`ImputeCall`] from the names of its columns and the SQL of its
//! constants over the schema of its input query, plans it with [`impute::plan`], and registers
//! the plan as a view, which DataFusion's SQL reads.

mod common;

use common::session;

use avenger_datafusion_transforms::impute::{self, ImputeCall, Named};
use avenger_datafusion_transforms::table_function::{ArgAt, ArgError};
use datafusion::arrow::array::{Array, ArrayRef};
use datafusion::arrow::datatypes::DataType;
use datafusion::arrow::util::display::array_value_to_string;
use datafusion::common::{Column, DFSchema, DataFusionError, Result};
use datafusion::prelude::SessionContext;

/// A named argument of a call, in DataFusion's SQL over the schema of the call's input.
#[derive(Clone, Copy)]
enum Arg<'a> {
    /// A constant, such as `0` or `'avg'`.
    Value(&'a str),
    /// A list of constants, such as `[-1, 1]`.
    List(&'a [&'a str]),
    /// A column name, of `flag`.
    Name(&'a str),
}

/// A call of `impute`: the names of its key, group, and value columns, and its named
/// arguments.
#[derive(Clone, Copy)]
struct Call<'a> {
    key: &'a str,
    groups: &'a [&'a str],
    value: &'a str,
    named: &'a [(&'a str, Arg<'a>)],
}

/// The column `name` of `schema`, with its qualifier.
fn column(schema: &DFSchema, name: &str) -> Result<Column> {
    let (qualifier, field) = schema.qualified_field_with_unqualified_name(name)?;
    Ok(Column::new(qualifier.cloned(), field.name()))
}

impl Call<'_> {
    /// The call over the query `input`, with the columns of the input and the constants as
    /// expressions over its schema.
    async fn over(&self, ctx: &SessionContext, input: &str) -> Result<ImputeCall> {
        let plan = common::input(ctx, input).await?;
        let schema = plan.schema();
        let columns = |names: &[&str]| {
            (names.iter())
                .map(|n| column(schema, n))
                .collect::<Result<Vec<_>>>()
        };
        let mut named = Vec::new();
        for (name, arg) in self.named {
            let arg = match *arg {
                Arg::Value(sql) => Named::Value(common::expr(ctx, input, sql).await?),
                Arg::List(list) => {
                    let mut exprs = Vec::new();
                    for sql in list {
                        exprs.push(common::expr(ctx, input, sql).await?);
                    }
                    Named::List(exprs)
                }
                Arg::Name(name) => Named::Name(name.to_string()),
            };
            named.push((name.to_string(), arg));
        }
        Ok(ImputeCall {
            key: column(schema, self.key)?,
            groups: columns(self.groups)?,
            value: column(schema, self.value)?,
            named,
        })
    }

    /// Registers as the view `name` the plan of the call over the query `input`.
    async fn view(&self, ctx: &SessionContext, name: &str, input: &str) -> Result<()> {
        let call = self.over(ctx, input).await?;
        common::view(ctx, name, input, |plan| {
            impute::plan(plan, call).map_err(|e| DataFusionError::Plan(e.message))
        })
        .await
    }

    /// The error of the plan of the call over the query `input`.
    async fn error(&self, ctx: &SessionContext, input: &str) -> Result<ArgError> {
        let call = self.over(ctx, input).await?;
        match impute::plan(common::input(ctx, input).await?, call) {
            Ok(_) => panic!("no error: {input}"),
            Err(e) => Ok(e),
        }
    }
}

async fn context() -> Result<SessionContext> {
    let ctx = session();
    ctx.sql(
        "CREATE TABLE s (month BIGINT, region VARCHAR, product VARCHAR, units DOUBLE, note VARCHAR, iunits BIGINT) AS VALUES
         (1, 'North', 'A', 10.0, 'a', 10),
         (2, 'North', 'A', 12.5, NULL, 12),
         (4, 'North', 'A', 7.0, 'b', 7),
         (1, 'North', 'B', 3.0, NULL, 3),
         (3, 'North', 'B', 4.5, NULL, 4),
         (3, 'North', 'B', 1.0, 'duplicate key', 1),
         (2, 'South', 'A', 8.0, NULL, 8),
         (5, 'South', 'A', NULL, 'null measure', NULL),
         (1, 'South', 'B', 6.0, NULL, 6),
         (6, 'South', 'B', 2.5, NULL, 2),
         (2, NULL, 'A', 9.0, 'null region', 9),
         (NULL, 'North', 'A', 11.0, 'null key', 11),
         (4, 'East', 'B', 5.5, NULL, 5)",
    )
    .await?
    .collect()
    .await?;
    Ok(ctx)
}

enum Fill {
    Value(&'static str),
    Stat(&'static str, Option<(Option<i64>, Option<i64>)>),
}

struct Case {
    name: &'static str,
    input: &'static str,
    call: Call<'static>,
    fill: Fill,
}

impl Case {
    /// The constants of the named list argument `name`, as the SQL of their list.
    fn list(&self, name: &str) -> Option<String> {
        self.call.named.iter().find_map(|(n, arg)| match arg {
            Arg::List(list) if *n == name => Some(list.join(", ")),
            _ => None,
        })
    }

    fn flag(&self) -> Option<&'static str> {
        self.call.named.iter().find_map(|(_, arg)| match arg {
            Arg::Name(name) => Some(*name),
            _ => None,
        })
    }
}

const ALL: &str = "SELECT * FROM s";
const AGG: &str = "SELECT month, region, sum(units) AS units FROM s WHERE month IS NOT NULL GROUP BY month, region";

fn cases() -> Vec<Case> {
    let c = |name, input, groups, value, named, fill| Case {
        name,
        input,
        call: Call {
            key: "month",
            groups,
            value,
            named,
        },
        fill,
    };
    use Arg::{List, Name, Value};
    vec![
        c(
            "value 0, groups [region]",
            ALL,
            &["region"],
            "units",
            &[("value", Value("0"))],
            Fill::Value("0"),
        ),
        c(
            "two groups and a flag",
            ALL,
            &["region", "product"],
            "units",
            &[("value", Value("0")), ("flag", Name("imputed"))],
            Fill::Value("0"),
        ),
        c(
            "no groups",
            ALL,
            &[],
            "units",
            &[("value", Value("-1"))],
            Fill::Value("-1"),
        ),
        c(
            "value NULL",
            ALL,
            &["region"],
            "units",
            &[("value", Value("NULL")), ("flag", Name("imputed"))],
            Fill::Value("NULL"),
        ),
        c(
            "avg",
            ALL,
            &["region"],
            "units",
            &[("method", Value("'avg'"))],
            Fill::Stat("avg", None),
        ),
        c(
            "median",
            ALL,
            &["region", "product"],
            "units",
            &[("method", Value("'median'"))],
            Fill::Stat("median", None),
        ),
        c(
            "min",
            ALL,
            &["region"],
            "units",
            &[("method", Value("'min'"))],
            Fill::Stat("min", None),
        ),
        c(
            "max",
            ALL,
            &["product"],
            "units",
            &[("method", Value("'max'"))],
            Fill::Stat("max", None),
        ),
        c(
            "keys",
            ALL,
            &["region"],
            "units",
            &[("value", Value("0")), ("keys", List(&["1", "2", "3", "4"]))],
            Fill::Value("0"),
        ),
        c(
            "extra keys",
            ALL,
            &["region"],
            "units",
            &[
                ("value", Value("0")),
                ("extra_keys", List(&["7", "2", "9"])),
            ],
            Fill::Value("0"),
        ),
        c(
            "keys and extra keys",
            ALL,
            &["region"],
            "units",
            &[
                ("value", Value("0")),
                ("keys", List(&["3", "1"])),
                ("extra_keys", List(&["8", "1", "5"])),
            ],
            Fill::Value("0"),
        ),
        c(
            "integer value column",
            ALL,
            &["region"],
            "iunits",
            &[("value", Value("0"))],
            Fill::Value("0"),
        ),
        c(
            "frame avg [-1, 1]",
            AGG,
            &["region"],
            "units",
            &[("method", Value("'avg'")), ("frame", List(&["-1", "1"]))],
            Fill::Stat("avg", Some((Some(-1), Some(1)))),
        ),
        c(
            "frame max [NULL, 0]",
            AGG,
            &["region"],
            "units",
            &[("method", Value("'max'")), ("frame", List(&["NULL", "0"]))],
            Fill::Stat("max", Some((None, Some(0)))),
        ),
        c(
            "frame median [-2, 2], keys",
            AGG,
            &["region"],
            "units",
            &[
                ("method", Value("'median'")),
                ("frame", List(&["-2", "2"])),
                ("keys", List(&["6", "5", "4", "3", "2", "1"])),
            ],
            Fill::Stat("median", Some((Some(-2), Some(2)))),
        ),
        c(
            "frame min [1, NULL], no groups",
            AGG,
            &[],
            "units",
            &[("method", Value("'min'")), ("frame", List(&["1", "NULL"]))],
            Fill::Stat("min", Some((Some(1), None))),
        ),
        c(
            "avg of integers is a DOUBLE",
            ALL,
            &["region"],
            "iunits",
            &[("method", Value("'avg'"))],
            Fill::Stat("avg", None),
        ),
        c(
            "median of integers stays BIGINT",
            ALL,
            &["region"],
            "iunits",
            &[("method", Value("'median'"))],
            Fill::Stat("median", None),
        ),
        c(
            "min of strings",
            ALL,
            &["region"],
            "note",
            &[("method", Value("'min'"))],
            Fill::Stat("min", None),
        ),
        c(
            "frame avg of integers",
            ALL,
            &["region"],
            "iunits",
            &[("method", Value("'avg'")), ("frame", List(&["-1", "1"]))],
            Fill::Stat("avg", Some((Some(-1), Some(1)))),
        ),
        c(
            "value 0.5 widens integers to DOUBLE",
            ALL,
            &["region"],
            "iunits",
            &[("value", Value("0.5"))],
            Fill::Value("0.5"),
        ),
        c(
            "subquery input",
            "SELECT month, product, units FROM s WHERE region = 'North'",
            &["product"],
            "units",
            &[("value", Value("0"))],
            Fill::Value("0"),
        ),
    ]
}

fn frame_bound(b: Option<i64>, start: bool) -> String {
    match b {
        None if start => "UNBOUNDED PRECEDING".into(),
        None => "UNBOUNDED FOLLOWING".into(),
        Some(0) => "CURRENT ROW".into(),
        Some(b) if b < 0 => format!("{} PRECEDING", -b),
        Some(b) => format!("{b} FOLLOWING"),
    }
}

/// The reference SQL of a case.
/// `out` is the type of the value column in the result: the type of `COALESCE` of the column and
/// the fill.
fn reference(c: &Case, columns: &[(String, DataType)], out: &str) -> String {
    let q = |n: &str| format!("\"{n}\"");
    let key = q(c.call.key);
    let val_type = &columns.iter().find(|(n, _)| n == c.call.value).unwrap().1;
    let gs: Vec<String> = c.call.groups.iter().map(|g| q(g)).collect();
    let keyvals = match (c.list("keys"), c.list("extra_keys")) {
        (Some(k), Some(e)) => format!(
            "SELECT DISTINCT __k FROM (SELECT unnest(make_array({k})) AS __k UNION ALL SELECT unnest(make_array({e})))"
        ),
        (Some(k), None) => {
            format!("SELECT DISTINCT __k FROM (SELECT unnest(make_array({k})) AS __k)")
        }
        (None, Some(e)) => format!(
            "SELECT DISTINCT __k FROM (SELECT {key} AS __k FROM input WHERE {key} IS NOT NULL UNION ALL SELECT unnest(make_array({e})))"
        ),
        (None, None) => format!("SELECT DISTINCT {key} AS __k FROM input WHERE {key} IS NOT NULL"),
    };
    let grp = if gs.is_empty() {
        "SELECT DISTINCT 1 AS __one FROM input".to_string()
    } else {
        format!("SELECT DISTINCT {} FROM input", gs.join(", "))
    };
    let on_groups = |a: &str, b: &str| -> String {
        gs.iter()
            .map(|g| format!("({a}.{g} IS NOT DISTINCT FROM {b}.{g}) AND "))
            .collect()
    };
    let present_groups: String = gs.iter().map(|g| format!("{g}, ")).collect();
    let mut sql = format!(
        "WITH input AS (SELECT * FROM ({input})),\n\
         keyvals AS ({keyvals}),\n\
         grp AS ({grp}),\n\
         cand AS (SELECT grp.*, keyvals.__k FROM grp CROSS JOIN keyvals),\n\
         present AS (SELECT DISTINCT {present_groups}{key} AS __k, true AS __present FROM input),\n\
         added AS (SELECT cand.* FROM cand LEFT JOIN present p ON {on}cand.__k = p.__k WHERE p.__present IS NULL)",
        input = c.input,
        on = on_groups("cand", "p"),
    );
    // The value of each added row, and what the added rows are selected from.
    let (imputed, from) = match &c.fill {
        Fill::Value(v) => (format!("arrow_cast({v}, '{out}')"), "added".to_string()),
        Fill::Stat(agg, None) => {
            let by = if gs.is_empty() {
                String::new()
            } else {
                format!(" GROUP BY {}", gs.join(", "))
            };
            sql += &format!(
                ",\nst AS (SELECT {present_groups}{agg}({v}) AS __imp FROM input{by})",
                v = q(c.call.value)
            );
            let join = if gs.is_empty() {
                "added CROSS JOIN st".to_string()
            } else {
                let on: String = on_groups("added", "st");
                format!("added LEFT JOIN st ON {}", on.trim_end_matches(" AND "))
            };
            (format!("arrow_cast(st.__imp, '{out}')"), join)
        }
        Fill::Stat(agg, Some((lo, hi))) => {
            let order = match c.list("keys") {
                Some(k) => format!("array_position(make_array({k}), __k) ASC NULLS LAST, __k"),
                None => "__k".to_string(),
            };
            let part = if gs.is_empty() {
                String::new()
            } else {
                format!("PARTITION BY {} ", gs.join(", "))
            };
            let gsel: String = gs.iter().map(|g| format!("i.{g}, ")).collect();
            let gadd: String = gs.iter().map(|g| format!("{g}, ")).collect();
            sql += &format!(
                ",\ncombined AS (SELECT {gsel}i.{key} AS __k, i.{v} AS __v, false AS __added FROM input i JOIN keyvals k ON i.{key} = k.__k\n\
                   UNION ALL SELECT {gadd}__k, arrow_cast(NULL, '{val_type}'), true FROM added),\n\
                 framed AS (SELECT *, {agg}(__v) OVER ({part}ORDER BY {order} ROWS BETWEEN {lo} AND {hi}) AS __imp FROM combined)",
                v = q(c.call.value),
                lo = frame_bound(*lo, true),
                hi = frame_bound(*hi, false),
            );
            (
                format!("arrow_cast(__imp, '{out}')"),
                "framed WHERE __added".to_string(),
            )
        }
    };
    let flag_in = c
        .flag()
        .map_or(String::new(), |f| format!(", false AS {f}"));
    let flag_add = c.flag().map_or(String::new(), |f| format!(", true AS {f}"));
    let cols_in: Vec<String> = columns
        .iter()
        .map(|(n, _)| {
            if n == c.call.value {
                format!("arrow_cast({}, '{out}') AS {}", q(n), q(n))
            } else {
                q(n)
            }
        })
        .collect();
    let cols_add: Vec<String> = columns
        .iter()
        .map(|(n, t)| {
            if n == c.call.key {
                format!("__k AS {}", q(n))
            } else if c.call.groups.contains(&n.as_str()) {
                let qualifier =
                    if from.starts_with("added LEFT JOIN") || from.starts_with("added CROSS") {
                        "added."
                    } else {
                        ""
                    };
                format!("{qualifier}{}", q(n))
            } else if n == c.call.value {
                format!("{imputed} AS {}", q(n))
            } else {
                format!("arrow_cast(NULL, '{t}') AS {}", q(n))
            }
        })
        .collect();
    sql += &format!(
        "\nSELECT {}{flag_in} FROM input\nUNION ALL\nSELECT {}{flag_add} FROM {from}",
        cols_in.join(", "),
        cols_add.join(", ")
    );
    sql
}

/// The rows as sorted strings, and the column names and types.
async fn rows(ctx: &SessionContext, sql: &str) -> Result<(Vec<String>, Vec<Vec<String>>)> {
    let df = ctx.sql(sql).await?;
    let schema: Vec<String> = df
        .schema()
        .fields()
        .iter()
        .map(|f| format!("{} {}", f.name(), f.data_type()))
        .collect();
    let mut out = Vec::new();
    for b in df.collect().await? {
        for i in 0..b.num_rows() {
            out.push(
                b.columns()
                    .iter()
                    .map(|c: &ArrayRef| {
                        if c.is_null(i) {
                            "NULL".to_string()
                        } else {
                            array_value_to_string(c, i).unwrap()
                        }
                    })
                    .collect::<Vec<_>>(),
            );
        }
    }
    out.sort();
    Ok((schema, out))
}

#[tokio::test]
async fn reference_sql() -> Result<()> {
    let ctx = context().await?;
    for c in cases() {
        let probe = ctx
            .sql(&format!("SELECT * FROM ({}) LIMIT 0", c.input))
            .await?;
        let columns: Vec<(String, DataType)> = probe
            .schema()
            .fields()
            .iter()
            .map(|f| (f.name().clone(), f.data_type().clone()))
            .collect();
        // The type of `COALESCE` of the value column and the fill.
        let fill = match &c.fill {
            Fill::Value(v) => v.to_string(),
            Fill::Stat(agg, _) => {
                format!("(SELECT {agg}({v}) FROM ({}))", c.input, v = c.call.value)
            }
        };
        let typed = rows(
            &ctx,
            &format!(
                "SELECT arrow_typeof(coalesce({}, {fill})) FROM ({}) LIMIT 1",
                c.call.value, c.input
            ),
        )
        .await?;
        let reference_sql = reference(&c, &columns, &typed.1[0][0]);
        c.call.view(&ctx, "imputed", c.input).await?;
        let ours = rows(&ctx, "SELECT * FROM imputed").await?;
        let expected = rows(&ctx, &reference_sql).await?;
        assert_eq!(ours, expected, "{}", c.name);
    }
    Ok(())
}

#[tokio::test]
async fn errors() -> Result<()> {
    let ctx = context().await?;
    use Arg::{List, Name, Value};
    let call = |groups, value, named| Call {
        key: "month",
        groups,
        value,
        named,
    };
    // The call, the place of the error, and the start of its message.
    let errors: [(Call, ArgAt, &str); 14] = [
        (
            call(&["region"], "units", &[]),
            ArgAt::Call,
            "Impute needs exactly one of `value` and `method`; found neither",
        ),
        (
            call(
                &["region"],
                "units",
                &[("value", Value("0")), ("method", Value("'avg'"))],
            ),
            ArgAt::Named(1),
            "Impute needs exactly one of `value` and `method`; found both",
        ),
        (
            call(&["region"], "units", &[("method", Value("'linear'"))]),
            ArgAt::Named(0),
            "`method` must be 'avg', 'median', 'min', or 'max'",
        ),
        (
            call(
                &["region"],
                "units",
                &[("value", Value("0")), ("frame", List(&["-1", "1"]))],
            ),
            ArgAt::Named(1),
            "`frame` needs a method",
        ),
        (
            call(
                &["region"],
                "units",
                &[("method", Value("'avg'")), ("frame", List(&["2", "1"]))],
            ),
            ArgAt::Named(1),
            "The start of `frame`, 2, is after its end, 1",
        ),
        (
            call(
                &["region"],
                "units",
                &[("value", Value("0")), ("fill", Value("1"))],
            ),
            ArgAt::Named(1),
            "Unknown argument `fill` of impute",
        ),
        (
            call(&["region"], "units", &[("value", Value("'zero'"))]),
            ArgAt::Named(0),
            "`value` 'zero' does not coerce to the type of units",
        ),
        (
            call(
                &["region"],
                "units",
                &[("value", Value("0")), ("extra_keys", List(&["'a'"]))],
            ),
            ArgAt::NamedElement(1, 0),
            "The value 'a' of `extra_keys` does not coerce",
        ),
        (
            call(
                &["region"],
                "units",
                &[("value", Value("0")), ("flag", Name("note"))],
            ),
            ArgAt::Named(1),
            "The flag column `note` is already a column of the input",
        ),
        (
            call(&["region"], "note", &[("method", Value("'avg'"))]),
            ArgAt::Named(0),
            "`method => 'avg'` does not apply to note",
        ),
        (
            call(&["month"], "units", &[("value", Value("0"))]),
            ArgAt::Element(1, 0),
            "The key month of impute is also a group column",
        ),
        (
            call(
                &["region"],
                "units",
                &[("value", Value("0")), ("keys", List(&["1", "NULL"]))],
            ),
            ArgAt::NamedElement(1, 1),
            "A value of `keys` cannot be NULL",
        ),
        (
            call(
                &["region"],
                "units",
                &[("value", Value("0")), ("keys", List(&["1.5"]))],
            ),
            ArgAt::NamedElement(1, 0),
            "The value 1.5 of `keys` does not coerce",
        ),
        (
            call(
                &["region"],
                "units",
                &[
                    ("method", Value("'avg'")),
                    ("frame", List(&["0", "18446744073709551615"])),
                ],
            ),
            ArgAt::NamedElement(1, 1),
            "A bound of `frame` must fit in an INT64",
        ),
    ];
    for (call, at, message) in errors {
        let e = call.error(&ctx, ALL).await?;
        assert!(
            e.at == at && e.message.starts_with(message),
            "{message}: {e:?}"
        );
    }
    Ok(())
}

/// The rows of the view of `call` over the query `input`, read by `sql`, sorted.
async fn imputed(
    ctx: &SessionContext,
    input: &str,
    call: Call<'_>,
    sql: &str,
) -> Result<Vec<Vec<String>>> {
    call.view(ctx, "imputed", input).await?;
    Ok(rows(ctx, sql).await?.1)
}

#[tokio::test]
async fn edge_cases() -> Result<()> {
    let ctx = context().await?;
    use Arg::{List, Value};
    fn call<'a>(named: &'a [(&'a str, Arg<'a>)]) -> Call<'a> {
        Call {
            key: "month",
            groups: &["region"],
            value: "units",
            named,
        }
    }
    let all = "SELECT * FROM imputed";
    // A constant that needs type coercion.
    let a = imputed(&ctx, ALL, call(&[("value", Value("1 + 0.5"))]), all).await?;
    assert!(a.iter().any(|r| r[3] == "1.5"));
    // A frame of the whole group is no frame.
    let keys = ("keys", List(&["1", "2", "3"]));
    let no_frame = imputed(&ctx, ALL, call(&[("method", Value("'avg'")), keys]), all).await?;
    let whole = imputed(
        &ctx,
        ALL,
        call(&[
            ("method", Value("'avg'")),
            keys,
            ("frame", List(&["NULL", "NULL"])),
        ]),
        all,
    )
    .await?;
    assert_eq!(no_frame, whole);
    // A frame bound past the end of every group.
    let wide = imputed(
        &ctx,
        ALL,
        call(&[
            ("method", Value("'max'")),
            ("frame", List(&["0", "9223372036854775807"])),
        ]),
        all,
    )
    .await?;
    let open = imputed(
        &ctx,
        ALL,
        call(&[("method", Value("'max'")), ("frame", List(&["0", "NULL"]))]),
        all,
    )
    .await?;
    assert_eq!(wide, open);
    // An empty input.
    let empty = imputed(
        &ctx,
        "SELECT * FROM s WHERE false",
        call(&[("value", Value("0"))]),
        all,
    )
    .await?;
    assert!(empty.is_empty());
    // A struct key, as the result of timeunit or bin, sorted in key order.
    let input = "SELECT named_struct('m', month) AS k, region AS g, units AS v FROM s WHERE month IS NOT NULL";
    let call = Call {
        key: "k",
        groups: &["g"],
        value: "v",
        named: &[("value", Value("0"))],
    };
    // 12 rows, and the missing months of 4 regions: 2, 2, 5, and 5.
    let all = imputed(&ctx, input, call, "SELECT k['m'] AS m, g, v FROM imputed").await?;
    assert_eq!(all.len(), 26);
    let east: Vec<&str> = all
        .iter()
        .filter(|r| r[1] == "East")
        .map(|r| r[0].as_str())
        .collect();
    assert_eq!(east, ["1", "2", "3", "4", "5", "6"]);
    Ok(())
}
