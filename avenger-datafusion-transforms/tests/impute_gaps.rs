//! `impute` with `fill_gaps` and `partition_by`: the keys of a sequence that no row has, for
//! time units, bins, dates, intervals, and numbers, in partitions that impute separately.
//!
//! Each case builds an [`ImputeCall`] from the names of its columns and the SQL of its
//! constants over the schema of its input query, plans it with [`impute::plan`], and registers
//! the plan as a view, which DataFusion's SQL reads. The keys of time units and bins come from
//! views of [`timeunit::plan`] and [`bin::plan`].

mod common;

use common::session;

use avenger_datafusion_transforms::bin::{self, BinCall, BinOptions};
use avenger_datafusion_transforms::impute::{self, ImputeCall, Named};
use avenger_datafusion_transforms::stack::{self, StackOffset};
use avenger_datafusion_transforms::table_function::{ArgAt, ArgError};
use avenger_datafusion_transforms::timeunit::{
    self, TimeUnitCall, TimeUnitOptions, Unit, WeekStart,
};
use datafusion::arrow::array::Array;
use datafusion::arrow::util::display::array_value_to_string;
use datafusion::common::{Column, DFSchema, DataFusionError, Result};
use datafusion::prelude::{SessionContext, col};

/// A named argument of a call, in DataFusion's SQL over the schema of the call's input.
#[derive(Clone, Copy)]
enum Arg<'a> {
    /// A constant, such as `true` or `'month'`.
    Value(&'a str),
    /// A list of constants, such as `[DATE '2024-03-01']`.
    List(&'a [&'a str]),
    /// A column name, of `flag`.
    Name(&'a str),
    /// Columns of the input, of `partition_by`.
    Columns(&'a [&'a str]),
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
fn input_column(schema: &DFSchema, name: &str) -> Result<Column> {
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
                .map(|n| input_column(schema, n))
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
                Arg::Columns(names) => Named::Columns(columns(names)?),
            };
            named.push((name.to_string(), arg));
        }
        Ok(ImputeCall {
            key: input_column(schema, self.key)?,
            groups: columns(self.groups)?,
            value: input_column(schema, self.value)?,
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

    /// The error of running the call over the query `input`.
    async fn run_error(&self, ctx: &SessionContext, input: &str) -> Result<String> {
        self.view(ctx, "imputed", input).await?;
        match ctx.sql("SELECT * FROM imputed").await?.collect().await {
            Ok(_) => panic!("no error: {input}"),
            Err(e) => Ok(e.to_string()),
        }
    }
}

/// A call without groups.
fn ungrouped<'a>(key: &'a str, value: &'a str, named: &'a [(&'a str, Arg<'a>)]) -> Call<'a> {
    Call {
        key,
        groups: &[],
        value,
        named,
    }
}

/// The named argument `value => 0`.
const VALUE: (&str, Arg) = ("value", Arg::Value("0"));

/// Runs the statement `sql`.
async fn exec(ctx: &SessionContext, sql: &str) -> Result<()> {
    ctx.sql(sql).await?.collect().await?;
    Ok(())
}

/// The options of `timeunit` with the unit `text`.
fn unit(text: &str) -> TimeUnitOptions {
    TimeUnitOptions {
        unit: Some(Unit::parse(text).expect("a unit")),
        ..Default::default()
    }
}

/// Registers as the view `name` the query `input` with the column `u`, the time unit of its
/// column `t` by `options`.
async fn time_units(
    ctx: &SessionContext,
    name: &str,
    input: &str,
    options: TimeUnitOptions,
) -> Result<()> {
    let call = TimeUnitCall {
        value: col("t"),
        partition: vec![],
        options,
    };
    common::view(ctx, name, input, |plan| {
        timeunit::plan(plan, call, Column::from_name("u"))
    })
    .await
}

/// Registers as the view `name` the query `input` with the column `b`, the bin of its column
/// `x` by `options` in the window partitions of the columns `partition`.
async fn bins_of(
    ctx: &SessionContext,
    name: &str,
    input: &str,
    partition: &[&str],
    options: BinOptions,
) -> Result<()> {
    let call = BinCall {
        value: col("x"),
        partition: partition.iter().map(|c| col(*c)).collect(),
        options,
    };
    common::view(ctx, name, input, |plan| {
        bin::plan(plan, call, Column::from_name("b"))
    })
    .await
}

/// The rows of a query as strings, sorted.
async fn rows(ctx: &SessionContext, sql: &str) -> Result<Vec<Vec<String>>> {
    let mut out = Vec::new();
    for b in ctx.sql(sql).await?.collect().await? {
        for i in 0..b.num_rows() {
            out.push(
                b.columns()
                    .iter()
                    .map(|c| {
                        if c.is_null(i) {
                            "NULL".to_string()
                        } else {
                            array_value_to_string(c, i).unwrap()
                        }
                    })
                    .collect(),
            );
        }
    }
    out.sort();
    Ok(out)
}

/// The values of the first column of a query, sorted.
async fn column(ctx: &SessionContext, sql: &str) -> Result<Vec<String>> {
    Ok(rows(ctx, sql)
        .await?
        .into_iter()
        .map(|r| r[0].clone())
        .collect())
}

/// The rows that `sql` reads from the view `imputed` of `call` over the query `input`.
async fn imputed(
    ctx: &SessionContext,
    input: &str,
    call: Call<'_>,
    sql: &str,
) -> Result<Vec<Vec<String>>> {
    call.view(ctx, "imputed", input).await?;
    rows(ctx, sql).await
}

/// The values of the first column that `sql` reads from the view `imputed` of `call` over the
/// query `input`.
async fn imputed_column(
    ctx: &SessionContext,
    input: &str,
    call: Call<'_>,
    sql: &str,
) -> Result<Vec<String>> {
    call.view(ctx, "imputed", input).await?;
    column(ctx, sql).await
}

/// The tables `sales` and `obs`, and the view `sales_months` of `sales` with the column `u`,
/// the month of `t`.
async fn context() -> Result<SessionContext> {
    let ctx = session();
    exec(
        &ctx,
        "CREATE TABLE sales (t TIMESTAMP, d DATE, series VARCHAR, units DOUBLE) AS VALUES
         (TIMESTAMP '2024-01-05 10:00:00', DATE '2024-01-01', 'A', 5.0),
         (TIMESTAMP '2024-02-10 10:00:00', DATE '2024-02-01', 'A', 10.0),
         (TIMESTAMP '2024-04-20 10:00:00', DATE '2024-04-01', 'A', 20.0),
         (TIMESTAMP '2024-01-07 10:00:00', DATE '2024-01-01', 'B', 7.0),
         (TIMESTAMP '2024-04-02 10:00:00', DATE '2024-04-01', 'B', 3.0)",
    )
    .await?;
    exec(
        &ctx,
        "CREATE TABLE obs (g BIGINT, x DOUBLE) AS VALUES
         (1, 0.0), (1, 10.0), (1, 100.0),
         (2, 0.0), (2, 0.5), (2, 3.0),
         (NULL, 0.0), (NULL, 40.0)",
    )
    .await?;
    time_units(
        &ctx,
        "sales_months",
        "SELECT series, t, units FROM sales",
        unit("month"),
    )
    .await?;
    Ok(ctx)
}

/// The added keys keep the type of the unit: a dictionary from `timeunit`, and a string from a
/// key that a query builds, which fills as a key of `timeunit` does.
#[tokio::test]
async fn unit_types() -> Result<()> {
    let ctx = context().await?;
    let inputs = [
        (
            "SELECT u, units FROM sales_months WHERE series = 'A'",
            "Dictionary(Int32, Utf8)",
        ),
        (
            "SELECT named_struct('start', date_trunc('month', t), 'end', date_trunc('month', t) + INTERVAL '1 month', 'unit', 'month') AS u, units FROM sales WHERE series = 'A'",
            "Utf8",
        ),
    ];
    for (input, unit_type) in inputs {
        let got = imputed(
            &ctx,
            input,
            ungrouped(
                "u",
                "units",
                &[
                    ("value", Arg::Value("0")),
                    ("fill_gaps", Arg::Value("true")),
                ],
            ),
            "SELECT u['start'], u['unit'], arrow_typeof(u['unit']) FROM imputed",
        )
        .await?;
        let want: Vec<Vec<String>> = ["01", "02", "03", "04"]
            .iter()
            .map(|m| {
                vec![
                    format!("2024-{m}-01T00:00:00"),
                    "month".into(),
                    unit_type.into(),
                ]
            })
            .collect();
        assert_eq!(got, want, "{input}");
    }
    Ok(())
}

/// Months of `timeunit` per series: every series gets a row at every month from January to
/// April, March too, which no series has, so that each layer of a stack has every month.
#[tokio::test]
async fn time_unit_months() -> Result<()> {
    let ctx = context().await?;
    let monthly = "SELECT series, u, sum(units) AS units FROM sales_months GROUP BY series, u";
    let filled = Call {
        key: "u",
        groups: &["series"],
        value: "units",
        named: &[
            ("value", Arg::Value("0")),
            ("fill_gaps", Arg::Value("true")),
        ],
    };
    let got = imputed(
        &ctx,
        monthly,
        filled,
        "SELECT series, u['start'] AS month, units FROM imputed",
    )
    .await?;
    let want = [
        ["A", "2024-01-01T00:00:00", "5.0"],
        ["A", "2024-02-01T00:00:00", "10.0"],
        ["A", "2024-03-01T00:00:00", "0.0"],
        ["A", "2024-04-01T00:00:00", "20.0"],
        ["B", "2024-01-01T00:00:00", "7.0"],
        ["B", "2024-02-01T00:00:00", "0.0"],
        ["B", "2024-03-01T00:00:00", "0.0"],
        ["B", "2024-04-01T00:00:00", "3.0"],
    ];
    assert_eq!(got, want.map(|r| r.map(String::from).to_vec()));

    // The added keys are whole time units: the end of March and the unit name.
    let ends = column(
        &ctx,
        "SELECT u['end'] || ' ' || u['unit'] FROM imputed WHERE u['start'] = TIMESTAMP '2024-03-01'",
    )
    .await?;
    assert_eq!(
        ends,
        ["2024-04-01T00:00:00 month", "2024-04-01T00:00:00 month"]
    );

    // Stacked: each month has a layer of each series.
    ctx.register_udwf(stack::window(StackOffset::Zero).as_ref().clone());
    let stacked = rows(
        &ctx,
        "SELECT u['start'] AS month, count(*) FROM (SELECT u, stack(units) OVER (PARTITION BY u ORDER BY series) AS y
         FROM imputed) GROUP BY u ORDER BY 1",
    )
    .await?;
    assert_eq!(stacked.len(), 4);
    assert!(stacked.iter().all(|r| r[1] == "2"));

    // A sequence: all of 2024's first half, by the key's own unit.
    let half = imputed_column(
        &ctx,
        monthly,
        Call {
            named: &[
                ("value", Arg::Value("0")),
                (
                    "fill_gaps",
                    Arg::Value(
                        "named_struct('start', TIMESTAMP '2024-01-01', 'end', TIMESTAMP '2024-07-01')",
                    ),
                ),
            ],
            ..filled
        },
        "SELECT DISTINCT u['start'] FROM imputed",
    )
    .await?;
    assert_eq!(half.len(), 6);
    Ok(())
}

/// A DATE key names its unit.
#[tokio::test]
async fn date_keys() -> Result<()> {
    let ctx = context().await?;
    let input = "SELECT d, series, sum(units) AS units FROM sales GROUP BY d, series";
    fn call<'a>(named: &'a [(&'a str, Arg<'a>)]) -> Call<'a> {
        Call {
            key: "d",
            groups: &["series"],
            value: "units",
            named,
        }
    }
    let month = ("fill_gaps", Arg::Value("'month'"));
    let value = ("value", Arg::Value("0"));
    let got = imputed_column(
        &ctx,
        input,
        call(&[value, month]),
        "SELECT DISTINCT d FROM imputed",
    )
    .await?;
    assert_eq!(
        got,
        ["2024-01-01", "2024-02-01", "2024-03-01", "2024-04-01"]
    );

    // With the end of the year, half-open: 2024-12-31 ends with December.
    let got = imputed_column(
        &ctx,
        input,
        call(&[
            value,
            (
                "fill_gaps",
                Arg::Value("named_struct('end', DATE '2024-12-31', 'step', 'month')"),
            ),
        ]),
        "SELECT DISTINCT d FROM imputed",
    )
    .await?;
    assert_eq!(got.len(), 12);
    assert_eq!(got.last().unwrap(), "2024-12-01");

    // Two series of four months each, and the added rows are flagged.
    let n = imputed(
        &ctx,
        input,
        call(&[value, month, ("flag", Arg::Name("imputed"))]),
        "SELECT imputed, count(*) FROM imputed GROUP BY imputed",
    )
    .await?;
    assert_eq!(
        n,
        [["false", "5"], ["true", "3"]].map(|r| r.map(String::from).to_vec())
    );
    Ok(())
}

/// Bins: the empty bins between the bins of the rows, and with the bins of `bin_params`, the
/// empty bins at the ends of the mesh, with the keys that `bin` gives values in them.
#[tokio::test]
async fn bins() -> Result<()> {
    let ctx = session();
    exec(
        &ctx,
        "CREATE TABLE v (x DOUBLE) AS VALUES (5.0), (25.0), (26.0), (55.0)",
    )
    .await?;
    let options = BinOptions {
        extent: Some([0.0, 100.0]),
        maxbins: 10.0,
        ..Default::default()
    };
    bins_of(&ctx, "v_bins", "SELECT x FROM v", &[], options.clone()).await?;
    let hist = "SELECT b, count(*) AS n FROM v_bins GROUP BY b";
    let starts = |fill| {
        let ctx = &ctx;
        async move {
            let named = [("value", Arg::Value("0")), ("fill_gaps", Arg::Value(fill))];
            imputed_column(
                ctx,
                hist,
                ungrouped("b", "n", &named),
                "SELECT b['start'] FROM imputed ORDER BY 1",
            )
            .await
        }
    };
    let between: Vec<String> = starts("true").await?;
    assert_eq!(between, ["0.0", "10.0", "20.0", "30.0", "40.0", "50.0"]);

    // The complete mesh, and the bits of each key are those of `bin`.
    let p = bin::bin_params(
        0.0,
        100.0,
        &BinOptions {
            maxbins: 10.0,
            ..Default::default()
        },
    );
    let mesh_params = format!(
        "named_struct('start', {:?}, 'end', {:?}, 'step', {:?})",
        p.start, p.end, p.step
    );
    let mesh = imputed(
        &ctx,
        hist,
        ungrouped(
            "b",
            "n",
            &[
                ("value", Arg::Value("0")),
                ("fill_gaps", Arg::Value(&mesh_params)),
            ],
        ),
        "SELECT b['start'], b['end'] FROM imputed",
    )
    .await?;
    bins_of(
        &ctx,
        "mesh_bins",
        "SELECT CAST(value AS DOUBLE) * 10 + 5 AS x FROM generate_series(0, 9)",
        &[],
        options,
    )
    .await?;
    let binned = rows(&ctx, "SELECT DISTINCT b['start'], b['end'] FROM mesh_bins").await?;
    assert_eq!(mesh, binned);
    assert_eq!(mesh.len(), 10);

    // A range narrower than the data: keys outside it stay, and no gaps are filled there.
    let narrow = starts("named_struct('start', 20.0, 'end', 40.0)").await?;
    assert_eq!(narrow, ["0.0", "20.0", "30.0", "50.0"]);
    // Only an end: from the least key.
    let to = starts("named_struct('end', 80.0)").await?;
    assert_eq!(to.len(), 8);
    Ok(())
}

/// Partitions: each partition has its own bins, its own gaps, and its own groups. The input
/// runs in several DataFusion partitions, and a NULL partition column is one partition.
#[tokio::test]
async fn partitions() -> Result<()> {
    let ctx = context().await?;
    exec(&ctx, "SET datafusion.execution.target_partitions = 4").await?;
    let options = BinOptions {
        maxbins: 10.0,
        ..Default::default()
    };
    bins_of(&ctx, "obs_bins", "SELECT g, x FROM obs", &["g"], options).await?;
    let hist = "SELECT g, b, count(*) AS n FROM obs_bins GROUP BY g, b";
    let value = ("value", Arg::Value("0"));
    let fill = ("fill_gaps", Arg::Value("true"));
    let by_g = ("partition_by", Arg::Columns(&["g"]));
    let got = imputed(
        &ctx,
        hist,
        ungrouped("b", "n", &[value, fill, by_g]),
        "SELECT g, count(*), sum(n), min(b['end'] - b['start']) FROM imputed GROUP BY g",
    )
    .await?;
    // g = 1: bins 0 to 90 by 10; g = 2: bins 0 to 2.5 by 0.5; NULL: bins 0 to 35 by 5.
    let want = [
        ["1", "10", "3", "10.0"],
        ["2", "6", "3", "0.5"],
        ["NULL", "8", "2", "5.0"],
    ];
    assert_eq!(got, want.map(|r| r.map(String::from).to_vec()));

    // The partitions run apart: the input is hash partitioned on the partition column.
    let plan = rows(&ctx, "EXPLAIN SELECT * FROM imputed").await?;
    let plan: String = plan.iter().map(|r| r.join(" ")).collect();
    assert!(
        plan.contains("RepartitionExec: partitioning=Hash([g@"),
        "{plan}"
    );

    // Without the partitions, the bins of the partitions differ in width.
    let e = ungrouped("b", "n", &[value, fill])
        .run_error(&ctx, hist)
        .await?;
    assert!(
        e.contains("intervals of one width") && e.contains("partition_by"),
        "{e}"
    );

    // A series that does not occur in a partition gets no rows in it.
    let input = "SELECT d, series, sum(units) AS units, CASE WHEN series = 'A' THEN 'left' ELSE 'right' END AS facet FROM sales GROUP BY d, series";
    let got = imputed(
        &ctx,
        input,
        Call {
            key: "d",
            groups: &["series"],
            value: "units",
            named: &[
                value,
                ("fill_gaps", Arg::Value("'month'")),
                ("partition_by", Arg::Columns(&["facet"])),
            ],
        },
        "SELECT facet, series, count(*) FROM imputed GROUP BY facet, series",
    )
    .await?;
    assert_eq!(
        got,
        [["left", "A", "4"], ["right", "B", "4"]].map(|r| r.map(String::from).to_vec())
    );
    Ok(())
}

/// Calendars: hours on a day of New York with a repeated hour, Sunday weeks and fiscal
/// quarters from the options of `timeunit`.
#[tokio::test]
async fn calendars() -> Result<()> {
    let ctx = session();
    exec(&ctx,
        "CREATE TABLE ts AS SELECT arrow_cast(column1, 'Timestamp(Millisecond, Some(\"America/New_York\"))') AS t FROM (VALUES
         (TIMESTAMP '2024-11-03 04:30:00'), (TIMESTAMP '2024-11-04 04:30:00'))",
    )
    .await?;
    let named = [
        ("value", Arg::Value("0")),
        ("fill_gaps", Arg::Value("true")),
    ];
    let call = ungrouped("u", "n", &named);
    let starts = "SELECT u['start'] FROM imputed";
    time_units(&ctx, "ts_hours", "SELECT t FROM ts", unit("hour")).await?;
    let hours = imputed_column(&ctx, "SELECT u, 1 AS n FROM ts_hours", call, starts).await?;
    // 00:00 to 23:00 local, with 01:00 two times.
    assert_eq!(hours.len(), 25);

    exec(
        &ctx,
        "CREATE TABLE days (t TIMESTAMP) AS VALUES (TIMESTAMP '2024-03-05'), (TIMESTAMP '2024-03-26'),
         (TIMESTAMP '2024-10-15'), (TIMESTAMP '2025-06-01')",
    )
    .await?;
    time_units(
        &ctx,
        "days_weeks",
        "SELECT t FROM days WHERE t < TIMESTAMP '2024-04-01'",
        TimeUnitOptions {
            week_start: WeekStart::Sunday,
            ..unit("week")
        },
    )
    .await?;
    let weeks = imputed_column(&ctx, "SELECT u, 1 AS n FROM days_weeks", call, starts).await?;
    assert_eq!(
        weeks,
        [
            "2024-03-03T00:00:00",
            "2024-03-10T00:00:00",
            "2024-03-17T00:00:00",
            "2024-03-24T00:00:00"
        ]
    );
    time_units(
        &ctx,
        "days_quarters",
        "SELECT t FROM days WHERE t >= TIMESTAMP '2024-10-01'",
        TimeUnitOptions {
            fiscal_year_start: Some(10),
            ..unit("fiscal_quarter")
        },
    )
    .await?;
    let quarters =
        imputed_column(&ctx, "SELECT u, 1 AS n FROM days_quarters", call, starts).await?;
    assert_eq!(
        quarters,
        [
            "2024-10-01T00:00:00",
            "2025-01-01T00:00:00",
            "2025-04-01T00:00:00"
        ]
    );
    Ok(())
}

/// Intervals of dates that a query builds, and numbers with a step.
#[tokio::test]
async fn intervals_and_numbers() -> Result<()> {
    let ctx = session();
    exec(&ctx, "CREATE TABLE e (t TIMESTAMP, year BIGINT) AS VALUES (TIMESTAMP '2024-01-01 05:00:00', 2001), (TIMESTAMP '2024-01-04 07:00:00', 2004)")
        .await?;
    let days = "SELECT named_struct('start', date_bin(INTERVAL '1 day', t), 'end', date_bin(INTERVAL '1 day', t) + INTERVAL '1 day') AS k, 1 AS n FROM e";
    let starts = "SELECT k['start'] FROM imputed";
    let value = ("value", Arg::Value("0"));
    let got = imputed_column(
        &ctx,
        days,
        ungrouped("k", "n", &[value, ("fill_gaps", Arg::Value("true"))]),
        starts,
    )
    .await?;
    assert_eq!(got.len(), 4);
    let got = imputed_column(
        &ctx,
        days,
        ungrouped(
            "k",
            "n",
            &[
                value,
                (
                    "fill_gaps",
                    Arg::Value(
                        "named_struct('end', TIMESTAMP '2024-01-08', 'step', INTERVAL '1 day')",
                    ),
                ),
            ],
        ),
        starts,
    )
    .await?;
    assert_eq!(got.len(), 7);
    let years = imputed_column(
        &ctx,
        "SELECT year, 1 AS n FROM e",
        ungrouped(
            "year",
            "n",
            &[value, ("fill_gaps", Arg::Value("named_struct('step', 1)"))],
        ),
        "SELECT year FROM imputed",
    )
    .await?;
    assert_eq!(years, ["2001", "2002", "2003", "2004"]);
    Ok(())
}

/// The errors of `fill_gaps` and `partition_by`: those of the arguments, at their arguments,
/// then those of the keys of a partition, as the call runs.
#[tokio::test]
async fn errors() -> Result<()> {
    let ctx = context().await?;
    exec(&ctx, "CREATE TABLE mid (d DATE, n BIGINT) AS VALUES (DATE '2024-03-15', 1), (DATE '2024-05-01', 2)").await?;
    let mid = "SELECT * FROM mid";
    // The input, the call, the place of the error, and a part of its message.
    let plan_errors: [(&str, Call, ArgAt, &str); 9] = [
        (
            mid,
            ungrouped(
                "d",
                "n",
                &[VALUE, ("fill_gaps", Arg::Value("'fiscal_quarter'"))],
            ),
            ArgAt::Named(1),
            "needs a fiscal year start",
        ),
        (
            mid,
            ungrouped(
                "d",
                "n",
                &[VALUE, ("fill_gaps", Arg::Value("'month_of_year'"))],
            ),
            ArgAt::Named(1),
            "does not apply to a date key",
        ),
        (
            mid,
            ungrouped("d", "n", &[VALUE, ("fill_gaps", Arg::Value("'hour'"))]),
            ArgAt::Named(1),
            "shorter than one day",
        ),
        (
            mid,
            ungrouped("d", "n", &[VALUE, ("fill_gaps", Arg::Value("true"))]),
            ArgAt::Named(1),
            "needs a unit",
        ),
        (
            mid,
            ungrouped(
                "n",
                "d",
                &[
                    ("value", Arg::Value("DATE '2024-01-01'")),
                    ("fill_gaps", Arg::Value("true")),
                ],
            ),
            ArgAt::Named(1),
            "needs a step",
        ),
        (
            "SELECT series, units FROM sales",
            ungrouped(
                "series",
                "units",
                &[VALUE, ("fill_gaps", Arg::Value("true"))],
            ),
            ArgAt::Named(1),
            "fills the gaps of bins",
        ),
        (
            mid,
            ungrouped(
                "d",
                "n",
                &[
                    VALUE,
                    ("fill_gaps", Arg::Value("'month'")),
                    ("keys", Arg::List(&["DATE '2024-03-01'"])),
                ],
            ),
            ArgAt::Named(1),
            "`keys` replaces",
        ),
        (
            mid,
            ungrouped(
                "d",
                "n",
                &[
                    VALUE,
                    (
                        "fill_gaps",
                        Arg::Value("named_struct('stop', DATE '2024-12-01', 'step', 'month')"),
                    ),
                ],
            ),
            ArgAt::Named(1),
            "Unknown field `stop`",
        ),
        (
            "SELECT * FROM sales",
            Call {
                key: "d",
                groups: &["series"],
                value: "units",
                named: &[VALUE, ("partition_by", Arg::Columns(&["series"]))],
            },
            ArgAt::Named(1),
            "is also a column of another argument",
        ),
    ];
    for (input, call, at, message) in plan_errors {
        let e = call.error(&ctx, input).await?;
        assert!(
            e.at == at && e.message.contains(message),
            "{message}: {e:?}"
        );
    }

    time_units(&ctx, "sales_days", "SELECT t FROM sales", unit("day")).await?;
    exec(&ctx, "CREATE TABLE far (t TIMESTAMP, n BIGINT) AS VALUES (TIMESTAMP '2020-01-01', 1), (TIMESTAMP '2024-01-01', 1)").await?;
    let run_errors: [(&str, Call, &str); 6] = [
        (
            mid,
            ungrouped("d", "n", &[VALUE, ("fill_gaps", Arg::Value("'month'"))]),
            "is not the start of a 'month'",
        ),
        (
            "SELECT named_struct('start', TIMESTAMP '2024-03-03', 'end', TIMESTAMP '2024-03-10', 'unit', 'week') AS k, 1 AS n",
            ungrouped("k", "n", &[VALUE, ("fill_gaps", Arg::Value("true"))]),
            "is not a unit 'week' of its calendar options",
        ),
        (
            "SELECT u, units FROM sales_months",
            ungrouped("u", "units", &[VALUE, ("fill_gaps", Arg::Value("'day'"))]),
            "differs from the unit 'month'",
        ),
        (
            "SELECT u, 1 AS n FROM sales_months UNION ALL SELECT u, 1 AS n FROM sales_days",
            ungrouped("u", "n", &[VALUE, ("fill_gaps", Arg::Value("true"))]),
            "the keys have the units",
        ),
        (
            mid,
            ungrouped(
                "d",
                "n",
                &[
                    VALUE,
                    (
                        "fill_gaps",
                        Arg::Value(
                            "named_struct('start', DATE '2024-01-01', 'end', DATE '2026-01-01', 'step', 'month')",
                        ),
                    ),
                ],
            ),
            "is not the start of a 'month'",
        ),
        // The limit of positions.
        (
            "SELECT * FROM far",
            ungrouped("t", "n", &[VALUE, ("fill_gaps", Arg::Value("'second'"))]),
            "more than 1000000",
        ),
    ];
    for (input, call, message) in run_errors {
        let e = call.run_error(&ctx, input).await?;
        assert!(e.contains(message), "{input}: {e}");
    }
    Ok(())
}
