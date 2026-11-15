//! avenger's own functions: the marks, constructors, `scaled`, and `direct` of avenger-chart, the
//! window transforms `bin`, `stack`, and `timeunit`, `lump`, and the table function `impute`.
//!
//! A mark is an aggregate that builds a chart of the rows of its group, as in
//! `|> AGGREGATE bar(x => species, y => n) AS chart`, and takes each channel as a parameter. A
//! constructor returns the fragment of its message. The transforms take their options as named
//! arguments, as in `bin(x, maxbins => 30)`, which must be constants: the form of each builds the
//! typed options of its call and checks them, so an error of an option is at its argument. The
//! docs of the marks and the constructors are avenger-chart's, and the forms of the transforms
//! declare their options, with their docs. The pass plans the calls of `lump` and `impute` itself,
//! since their arguments read other scopes than a row: the library holds their docs and forms.
//!
//! A function or a parameter whose name is a reserved word of GoogleSQL takes the name with a
//! trailing underscore, as the constructor `select` is `select_` and the parameter `range` of
//! `scale` is `range_`, so a call can name it: GoogleSQL takes no reserved word as the name of a
//! function or an argument. Each binding of avenger-chart's functions follows this rule for the
//! reserved words of its language.

use std::sync::Arc;

use avenger_chart::functions::{
    constructors, encoding_options, marks, Accept as ChartAccept, Kind, Shape, COORD,
};
use avenger_datafusion_transforms::{
    bin::{self, BinOptions},
    stack::{self, StackOffset},
    table_function::constant,
    timeunit::{self, TimeUnitOptions, Unit, WeekStart},
    OptionError,
};
use datafusion::{
    arrow::{
        array::{Array, AsArray},
        datatypes::Float64Type,
    },
    common::{DataFusionError, ScalarValue},
    logical_expr::{AggregateUDF, Expr, ScalarUDF, WindowFunctionDefinition, WindowUDF},
};

use super::{
    form::{Accept, Arity, FormArg, Signature, Ty},
    null, Function, Need, Planned,
};
use crate::functions::{
    constructor::ConstructorFunction, declaration::MarkDeclaration, kind::KindFunction,
};

/// The doc of the parameter `coord` of a mark.
const COORD_DOC: &str =
    "The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.";

pub(super) fn functions() -> Vec<Function> {
    let mut functions = Vec::new();
    for mark in marks() {
        let udaf = Arc::new(AggregateUDF::new_from_impl(MarkDeclaration::new(mark)));
        let mut function = Function::aggregate(&sql_name(mark.name), summary(mark.doc))
            .description(mark.doc)
            .order_by();
        function.clauses.over = super::Over::Forbidden;
        function.clauses.having = false;
        let mut args = Vec::new();
        for channel in mark.channels {
            function = function.param(&sql_name(channel.name), channel.doc);
            args.push(any(channel.name, channel.required));
        }
        for property in mark.properties {
            function = function.param(&sql_name(property.name), property.doc);
            args.push(arg(
                property.name,
                ty_of(property.accepts),
                property.required,
            ));
        }
        function = function.param(&sql_name(COORD), COORD_DOC);
        args.push(any(COORD, false));
        let signature = Signature {
            args,
            result: Ty::Bytes,
            vars: Vec::new(),
        };
        functions.push(function.with_signature(signature, move |call| {
            Ok(Planned::Aggregate {
                func: udaf.clone(),
                args: positional(call.values()),
                filter: None,
                result: None,
            })
        }));
    }
    for constructor in constructors() {
        let udf = Arc::new(ScalarUDF::new_from_impl(ConstructorFunction::new(
            constructor,
        )));
        let doc = match constructor.doc {
            "" => format!("A fragment of the message `{}`.", constructor.message),
            doc => doc.to_string(),
        };
        let mut function =
            Function::scalar(&sql_name(constructor.name), summary(&doc)).description(&doc);
        let mut args = Vec::new();
        for param in constructor.params {
            function = function.param(&sql_name(param.name), param.doc);
            args.push(arg(param.name, ty_of(param.accepts), param.required));
        }
        let signature = Signature {
            args,
            result: Ty::Bytes,
            vars: Vec::new(),
        };
        functions.push(function.with_signature(signature, move |call| {
            Ok(udf.call(positional(call.values())))
        }));
    }
    functions.push(scaled());
    functions.push(direct());
    functions.push(bin());
    functions.push(stack());
    functions.push(timeunit());
    functions.push(lump());
    functions.push(impute());
    functions
}

/// The first sentence of `doc`, for a summary.
fn summary(doc: &str) -> &str {
    match doc.find(". ") {
        Some(end) => &doc[..=end],
        None => doc.trim_end(),
    }
}

/// The name of the function or the parameter `name` in SQL: `name`, or with a trailing
/// underscore when it is a reserved word of GoogleSQL.
fn sql_name(name: &str) -> String {
    match avenger_sql_syntax::is_reserved(name) {
        true => format!("{name}_"),
        false => name.to_string(),
    }
}

/// The argument of a form for the parameter `name`.
fn arg(name: &str, accept: Accept, required: bool) -> FormArg {
    FormArg {
        param: sql_name(name),
        accept,
        arity: match required {
            true => Arity::Required,
            false => Arity::Optional,
        },
        named_only: false,
    }
}

/// The argument of a channel, which takes a column, an expression of the rows, or a value.
fn any(name: &str, required: bool) -> FormArg {
    arg(name, Accept::Value(Ty::Any), required)
}

/// What a parameter of avenger-chart takes: the type of its one shape, the names of its one enum,
/// or any value when it takes several shapes, as a oneof does.
fn ty_of(accepts: &[ChartAccept]) -> Accept {
    match accepts {
        [accept] => match &accept.shape {
            Shape::Enum { values, .. } | Shape::Names(values) => {
                Accept::OneOf(values.iter().map(|value| value.name.to_string()).collect())
            }
            shape => Accept::Value(shape_ty(shape)),
        },
        _ => Accept::Value(Ty::Any),
    }
}

fn shape_ty(shape: &Shape) -> Ty {
    match shape {
        Shape::Bool => Ty::Bool,
        Shape::Int => Ty::Int64,
        Shape::Float => Ty::Float64,
        Shape::Str => Ty::String,
        Shape::Bytes | Shape::Message { .. } => Ty::Bytes,
        Shape::Array(element) => Ty::Array(Box::new(shape_ty(element))),
        Shape::Enum { .. } | Shape::Names(_) => Ty::String,
        Shape::Value => Ty::Any,
    }
}

/// The values of the parameters in their order, with `NULL` for an omitted one before the last
/// that the call gives, as the functions of avenger-chart take them.
fn positional(values: Vec<Option<Expr>>) -> Vec<Expr> {
    let given = values
        .iter()
        .rposition(|value| value.is_some())
        .map_or(0, |last| last + 1);
    values
        .into_iter()
        .take(given)
        .map(|value| value.unwrap_or_else(null))
        .collect()
}

fn scaled() -> Function {
    let udf = Arc::new(ScalarUDF::new_from_shared_impl(Arc::new(
        KindFunction::new(Kind::Scaled),
    )));
    let mut function = Function::scalar(
        Kind::Scaled.name(),
        "Gives a channel of a mark a value that passes through the channel's scale.",
    )
    .description(
        "Use `scaled` in an argument of a mark function, as in `color => scaled('Adelie')`, the \
         color that the color scale gives the category `Adelie`. A channel argument without \
         `scaled` or `direct` passes through the scale when the channel has a scale, such as \
         `color` or `y`, and is drawn as it is otherwise, such as `text`. A constant on a \
         position passes through the scale, as `y => 0` does for a reference line at zero. \
         `scaled` makes the scaled kind explicit, and its options set the channel's encoding.",
    )
    .param("value", "The value of the channel.");
    let mut args = vec![arg("value", Accept::Value(Ty::Var("T".to_string())), true)];
    for option in encoding_options() {
        function = function.param(&sql_name(option.name), option.doc);
        args.push(FormArg {
            named_only: true,
            ..arg(option.name, ty_of(option.accepts), false)
        });
    }
    let signature = Signature {
        args,
        result: Ty::Var("T".to_string()),
        vars: vec![("T".to_string(), Vec::new())],
    };
    function
        .with_signature(signature, move |call| {
            Ok(udf.call(positional(call.values())))
        })
        .example(
            "FROM penguins |> AGGREGATE AVG(body_mass_g) AS mass GROUP BY species \
             |> AGGREGATE bar(x => species, y => scaled(mass, title => 'Body mass (g)')) AS chart",
            "The axis of `y` takes the title `Body mass (g)`.",
        )
}

fn direct() -> Function {
    let udf = Arc::new(ScalarUDF::new_from_shared_impl(Arc::new(
        KindFunction::new(Kind::Direct),
    )));
    Function::scalar(
        Kind::Direct.name(),
        "Gives a channel of a mark a value that the mark draws as it is, without the channel's \
         scale.",
    )
    .description(
        "Use `direct` in an argument of a mark function, as in `fill => direct(color)` for a \
         column of CSS colors. On a position, a direct value is a fraction of the plot: \
         `x => direct(0.5)` is the middle of the plot's width, and `y => direct(0)` its bottom \
         edge, at any size of the plot.",
    )
    .param("value", "The value of the channel.")
    .form("(value: T) -> T where T: ANY", move |call| {
        Ok(udf.call(vec![call.arg(0)]))
    })
    .example(
        "FROM penguins |> AGGREGATE AVG(body_mass_g) AS mass GROUP BY species \
         |> AGGREGATE bar(x => species, y => mass, fill => direct('steelblue')) AS chart",
        "Every bar is steel blue, and the chart has no color legend.",
    )
}

fn bin() -> Function {
    transform(
        Function::window(
            "bin",
            "Divides the range of numbers into bins of a round step, and returns the bounds of \
             the bin of each row.",
            Need::Forbidden,
            false,
        )
        .description(
            "`bin` computes over each partition of its `OVER` clause, which takes no `ORDER BY` \
             or frame: the bins of the rows of a partition share one step and one start. `OVER \
             ()` bins all the rows together. Without options, `bin` chooses the step so the \
             range holds at most 20 bins, from steps of a power of 10 divided by 5 or 2. The \
             bounds hold the values from `start` up to, but not including, `end`, and are `NULL` \
             for a value without a bin.",
        )
        .param(
            "value",
            "The number of each row. A `NULL`, `NaN` or infinite value has no bin.",
        )
        .param("maxbins", "The greatest number of bins. When omitted, 20.")
        .param("base", "The number base of the steps. When omitted, 10.")
        .param(
            "divide",
            "The factors by which a step can be divided to give more bins. When omitted, `[5, 2]`.",
        )
        .param(
            "extent",
            "The range to bin, as `[low, high]`. A value outside it has no bin. When omitted, the \
             range of the finite values of the partition.",
        )
        .param(
            "span",
            "The length of the range that chooses the step, in place of the length of the extent.",
        )
        .param(
            "step",
            "The step of the bins, in place of the step that `bin` chooses.",
        )
        .param(
            "steps",
            "The steps that `bin` chooses from: the smallest that gives at most `maxbins` bins.",
        )
        .param("minstep", "The smallest step. When omitted, 0.")
        .param(
            "nice",
            "Whether the first bin starts at a whole multiple of the step. When omitted, `TRUE`. \
             With `FALSE`, the first bin starts at the low end of the range.",
        )
        .param("anchor", "A value at which a bin starts."),
        "(value: T[, maxbins => FLOAT64][, base => FLOAT64][, divide => ARRAY<FLOAT64>][, extent \
         => ARRAY<FLOAT64>][, span => FLOAT64][, step => FLOAT64][, steps => ARRAY<FLOAT64>][, \
         minstep => FLOAT64][, nice => BOOL][, anchor => FLOAT64]) -> STRUCT<start FLOAT64, end \
         FLOAT64> where T: INT64 | NUMERIC | BIGNUMERIC | FLOAT64",
        bin_window,
    )
    .example(
        "FROM UNNEST([1.0, 7.0, 12.0, 18.0, 25.0]) AS x |> EXTEND bin(x, maxbins => 3) OVER () \
         AS b |> ORDER BY x",
        "x | b\n1.0 | STRUCT(0.0 AS start, 10.0 AS end)\n7.0 | STRUCT(0.0 AS start, 10.0 AS \
         end)\n12.0 | STRUCT(10.0 AS start, 20.0 AS end)\n18.0 | STRUCT(10.0 AS start, 20.0 AS \
         end)\n25.0 | STRUCT(20.0 AS start, 30.0 AS end)",
    )
    .example(
        "FROM UNNEST([1.0, 7.0, 12.0]) AS x |> EXTEND bin(x, extent => [0, 10], step => 5) OVER \
         () AS b |> ORDER BY x",
        "x | b\n1.0 | STRUCT(0.0 AS start, 5.0 AS end)\n7.0 | STRUCT(5.0 AS start, 10.0 AS \
         end)\n12.0 | NULL",
    )
}

fn stack() -> Function {
    transform(
        Function::window(
            "stack",
            "Stacks numbers, and returns the bounds of the layer of each row, as for the \
             segments of a stacked bar.",
            Need::Required,
            false,
        )
        .description(
            "`PARTITION BY` gives the stacks, and `ORDER BY` the order of the layers in each \
             stack, from the bottom. The bounds are `NULL` for a value without a layer.",
        )
        .param(
            "value",
            "The number of each row. A `NULL` or `NaN` value has no layer and adds nothing to \
             its stack.",
        )
        .param(
            "offset",
            "Where each stack starts: `'zero'`, where positive values stack up from zero and \
             negative values down from zero, `'normalize'`, where the absolute values stack from 0 \
             to 1 as fractions of their stack's total, or `'center'`, where the absolute values \
             stack centered on zero. When omitted, `'zero'`.",
        ),
        "(value: T[, offset => ONEOF('zero', 'normalize', 'center')]) -> STRUCT<start FLOAT64, end \
         FLOAT64> where T: INT64 | NUMERIC | BIGNUMERIC | FLOAT64",
        stack_window,
    )
    .example(
        "FROM sales |> EXTEND stack(units) OVER (PARTITION BY region ORDER BY product) AS s |> \
         ORDER BY region, product",
        "region | product | units | s\neast | a | 3 | STRUCT(0.0 AS start, 3.0 AS end)\neast | b \
         | 5 | STRUCT(3.0 AS start, 8.0 AS end)\nwest | a | 2 | STRUCT(0.0 AS start, 2.0 AS \
         end)\nwest | b | 4 | STRUCT(2.0 AS start, 6.0 AS end)",
    )
}

fn timeunit() -> Function {
    transform(
        Function::window(
            "timeunit",
            "Returns the calendar unit, such as the month or the quarter, that holds each date \
             or instant.",
            Need::Forbidden,
            false,
        )
        .description(
            "`timeunit` takes no `ORDER BY` or frame in its `OVER` clause. Without a `unit`, it \
             chooses one unit for each partition. With a `unit`, the partitions don't matter, so \
             `OVER ()` serves. A count steps evenly across the longer units, so `'7 days'` are \
             weeks, and a count that divides a longer unit, such as `'15 minutes'` or `'3 \
             months'`, starts again at each start of the longer unit. The result holds the start \
             of the unit, the start of the next unit, and the name of the unit, such as \
             `'month'` or `'3 months'`.",
        )
        .param(
            "value",
            "The `DATE`, `DATETIME` or `TIMESTAMP` of each row. A `TIMESTAMP` takes its units in \
             the civil time of its time zone.",
        )
        .param(
            "unit",
            "The unit, such as `'month'`, which holds a span of time and takes a count and a \
             plural, as in `'15 minutes'` or `'3 months'`, or a collapsing unit, such as \
             `'month_of_year'`, which maps each value into one reference year. The fiscal units \
             `'fiscal_year'` and `'fiscal_quarter'` need `fiscal_year_start`. When omitted, \
             `timeunit` chooses one unit for each partition.",
        )
        .param(
            "min_unit",
            "The shortest unit that `timeunit` can choose. When omitted, the shortest is the \
             resolution of the data, such as `'month'` for values that are all first days of \
             months.",
        )
        .param(
            "maxbins",
            "The greatest number of units that a chosen unit gives the partition. When omitted, \
             20.",
        )
        .param(
            "week_start",
            "The first day of a week. When omitted, `'monday'`.",
        )
        .param(
            "fiscal_year_start",
            "The number of the month, from 1 to 12, in which a fiscal year starts, such as 10 for \
             October.",
        ),
        "(value: T[, unit => STRING][, min_unit => STRING][, maxbins => INT64][, week_start => \
         ONEOF('monday', 'sunday')][, fiscal_year_start => INT64]) -> STRUCT<start T, end T, unit \
         STRING> where T: DATE | DATETIME | TIMESTAMP",
        timeunit_window,
    )
    .example(
        "FROM UNNEST([DATE '2024-01-15', DATE '2024-02-03', DATE '2024-02-28']) AS day |> \
         EXTEND timeunit(day, unit => 'month') OVER () AS u |> ORDER BY day",
        "day | u\nDATE '2024-01-15' | STRUCT(DATE '2024-01-01' AS start, DATE '2024-02-01' AS \
         end, 'month' AS unit)\nDATE '2024-02-03' | STRUCT(DATE '2024-02-01' AS start, DATE \
         '2024-03-01' AS end, 'month' AS unit)\nDATE '2024-02-28' | STRUCT(DATE '2024-02-01' AS \
         start, DATE '2024-03-01' AS end, 'month' AS unit)",
    )
}

/// The mapping of a function that the pass plans itself.
fn planned_by_the_pass(_: &super::Call) -> Result<Planned, super::CallError> {
    Err(super::CallError::call(
        "the pass plans this function itself",
    ))
}

fn lump() -> Function {
    Function::window(
        "lump",
        "Keeps the heaviest categories of each partition, or the categories that a condition \
         keeps, and lumps the rest into an Other group.",
        Need::Forbidden,
        false,
    )
    .description(
        "`lump` decides each category of each partition of its `OVER` clause, which takes no \
         `ORDER BY` or frame, and returns the lump of each row's category: \
         `STRUCT<value, other, weight, category_count>`. For a kept category, `value` is the \
         category, `other` is `NULL`, `weight` is its weight, and `category_count` is 1. For the \
         Other group, `value` is `NULL`, `other` is the label, `weight` is the weight of all \
         the rows of the lumped categories, and `category_count` is their number, for a label \
         such as \"Other (12)\". A call takes exactly one of `n` and `keep`. The plan groups the \
         rows by the partition and the category and joins the decisions back to the rows, so \
         it reads its input more than once.",
    )
    .param("category", "The category of each row.")
    .param(
        "weight",
        "The weight of each category, an aggregate of the category's rows, such as \
         `SUM(tons)`. When omitted, the weight is `COUNT(*)`. It orders the window `ranked` of \
         `keep`: the categories from the heaviest, and by category on a tie.",
    )
    .param(
        "n",
        "The number of categories to keep in each partition, the first of the window `ranked`.",
    )
    .param(
        "keep",
        "The condition that keeps a category, evaluated once for each category. Like a \
         `HAVING` clause it compares aggregates of the category's rows, as `SUM(tons) > 1000`, \
         and like `QUALIFY` it can use windows over the categories of the partition, as \
         `COUNT(*) >= 0.05 * SUM(COUNT(*)) OVER ()` or `ROW_NUMBER() OVER ranked <= 5`. A \
         window in `keep` takes no `PARTITION BY`, and the NULL category isn't among the \
         categories of its windows. A `NULL` condition lumps the category.",
    )
    .param(
        "other",
        "The label of the Other group. When omitted, the label is `'Other'`.",
    )
    .param(
        "null_category",
        "What happens to the `NULL` category, which `n` and `keep` don't decide: `'keep'` \
         keeps it as a category of its own, `'other'` lumps it into the Other group, and \
         `'drop'` gives its rows a `NULL` lump. When omitted, it is `'keep'`.",
    )
    .form(
        "(category: ANY[, weight => ANY][, n => CONST INT64][, keep => BOOL][, other => CONST \
         STRING][, null_category => ONEOF('keep', 'other', 'drop')]) -> STRUCT<value ANY, other \
         STRING, weight FLOAT64, category_count INT64>",
        planned_by_the_pass,
    )
    .example(
        "FROM UNNEST(['a', 'a', 'a', 'b', 'b', 'c']) AS port |> EXTEND lump(port, n => 1) OVER \
         () AS l |> SELECT DISTINCT port, l.value AS value, l.other AS other, l.category_count \
         AS categories",
        "port | value | other | categories\n'a' | 'a' | NULL | 1\n'b' | NULL | 'Other' | \
         2\n'c' | NULL | 'Other' | 2",
    )
    .example(
        "FROM UNNEST([STRUCT('a' AS port, 5.0 AS tons), STRUCT('b', 1.0), STRUCT('c', 4.0)]) \
         |> EXTEND lump(port, weight => SUM(tons), keep => SUM(tons) >= 0.25 * SUM(SUM(tons)) \
         OVER ()) OVER () AS l |> SELECT port, l.value AS value, l.weight AS weight",
        "port | value | weight\n'a' | 'a' | 5.0\n'b' | NULL | 1.0\n'c' | 'c' | 4.0",
    )
}

/// The arguments of `impute` after its input, which a pipe `CALL` passes.
const IMPUTE_ARGS: &str = "key: ANY, group_columns: ARRAY<ANY>, column: ANY[, value => ANY][, \
     method => ONEOF('avg', 'median', 'min', 'max')][, frame => ARRAY<INT64>][, keys => \
     ARRAY<ANY>][, extra_keys => ARRAY<ANY>][, fill_gaps => ANY][, flag => CONST STRING][, \
     partition_by => ARRAY<ANY>]";

fn impute() -> Function {
    Function::table(
        "impute",
        "Adds the rows that each group lacks, one for each key that the group has no row of, \
         with an imputed value.",
    )
    .description(
        "A pipe `CALL impute(...)` imputes the rows of its input, and `FROM impute(TABLE t, \
         ...)` the rows of a table. For each group, the distinct combination of the group \
         columns, and each key, `impute` adds a row when the group has no row of the key. The \
         keys are `keys`, or the distinct keys of the input that aren't `NULL`, and \
         `extra_keys`. An added row holds the key, the group columns, the partition columns, \
         and the imputed value, and `NULL` in its other columns; the rows of the input stay as \
         they are. The added rows follow the rows of their partition, by group in the order of \
         the groups' first rows, then by key. `impute` reads its input once, into memory.",
    )
    .param("input", "In `FROM`, the table to impute, as `TABLE name`.")
    .param("key", "The column of the keys, such as a month.")
    .param(
        "group_columns",
        "The group columns, as an array of columns, such as `[region]`, or `[]` for one group.",
    )
    .param("column", "The column of the values to impute.")
    .param(
        "value",
        "The value of the added rows. The column takes the common type of its values and \
         `value`. A call takes exactly one of `value` and `method`.",
    )
    .param(
        "method",
        "The aggregate of the group's values that gives the value of an added row: `'avg'`, \
         `'median'`, `'min'` or `'max'`. The column takes the type of the aggregate.",
    )
    .param(
        "frame",
        "The rows of the group around an added row, in key order, whose values `method` \
         aggregates, as `[-2, 2]`: the bounds before and after the row, `NULL` for an \
         unbounded side. When omitted, `method` aggregates the whole group.",
    )
    .param(
        "keys",
        "The keys, in place of the keys of the input, as an array of constants.",
    )
    .param(
        "extra_keys",
        "Keys to add to the keys of the input, as an array of constants.",
    )
    .param(
        "fill_gaps",
        "Adds the gaps of the sequence of the keys to the keys: the bins, time units, dates or \
         numbers between the keys that no row has. `TRUE` fills the gaps of a key from `bin` \
         or `timeunit`; a unit, such as `'month'`, those of a `DATE` or `TIMESTAMP` key; and a \
         struct with any of `start`, `end` and `step` a sequence of its own, whose missing \
         fields come from the keys. A number key needs a `step`. The range is half-open, from \
         `start` up to `end`.",
    )
    .param(
        "flag",
        "The name of a `BOOL` column to add, `TRUE` in the added rows, such as `'imputed'`.",
    )
    .param(
        "partition_by",
        "The columns whose values `impute` imputes apart, as an array of columns: each \
         partition has its own keys, groups and gaps.",
    )
    .form(&format!("({IMPUTE_ARGS}) -> TABLE"), planned_by_the_pass)
    .form(
        &format!("(input: TABLE, {IMPUTE_ARGS}) -> TABLE"),
        planned_by_the_pass,
    )
    .example(
        "FROM UNNEST([STRUCT('a' AS g, 1 AS k, 10 AS v), STRUCT('a', 2, 20), STRUCT('b', 1, \
         5)]) |> SELECT * |> CALL impute(k, [g], v, value => 0)",
        "g | k | v\n'a' | 1 | 10\n'a' | 2 | 20\n'b' | 1 | 5\n'b' | 2 | 0",
    )
}

/// The window transform `function` of the form `declaration`, whose options are named arguments
/// after its value. `window` builds the window function of a call from its options.
fn transform(
    function: Function,
    declaration: &str,
    window: fn(&Options) -> Result<Arc<WindowUDF>, super::CallError>,
) -> Function {
    let signature = Signature::parse(declaration).expect("the form of a transform");
    let names: Vec<String> = (signature.args.iter())
        .map(|arg| arg.param.clone())
        .collect();
    function.with_signature(signature, move |call| {
        let options = Options {
            call,
            names: &names,
        };
        Ok(Planned::Window {
            func: WindowFunctionDefinition::WindowUDF(window(&options)?),
            args: vec![call.arg(0)],
            result: None,
        })
    })
}

/// The options of a call of a window transform, the constants of its named arguments.
struct Options<'c, 'a> {
    call: &'c super::Call<'a>,
    /// The names of the parameters of the form, the value's first.
    names: &'c [String],
}

impl Options<'_, '_> {
    fn index(&self, name: &str) -> usize {
        (self.names.iter())
            .position(|param| param == name)
            .expect("an option of the form")
    }

    /// The constant of the option `name`, when the call gives it.
    fn value(&self, name: &str) -> Result<Option<ScalarValue>, super::CallError> {
        let index = self.index(name);
        let Some(expr) = self.call.value(index) else {
            return Ok(None);
        };
        match constant(expr) {
            Ok(value) if value.is_null() => Err(self
                .call
                .error_at(index, format!("`{name}` takes a value, not NULL"))),
            Ok(value) => Ok(Some(value)),
            Err(_) => Err(self
                .call
                .error_at(index, format!("`{name}` takes a constant"))),
        }
    }

    fn number(&self, name: &str) -> Result<Option<f64>, super::CallError> {
        Ok(self.value(name)?.and_then(|value| match value {
            ScalarValue::Float64(number) => number,
            _ => None,
        }))
    }

    fn integer(&self, name: &str) -> Result<Option<i64>, super::CallError> {
        Ok(self.value(name)?.and_then(|value| match value {
            ScalarValue::Int64(number) => number,
            _ => None,
        }))
    }

    fn text(&self, name: &str) -> Result<Option<String>, super::CallError> {
        Ok(self
            .value(name)?
            .and_then(|value| value.try_as_str().flatten().map(str::to_string)))
    }

    fn boolean(&self, name: &str) -> Result<Option<bool>, super::CallError> {
        Ok(self.value(name)?.and_then(|value| match value {
            ScalarValue::Boolean(value) => value,
            _ => None,
        }))
    }

    fn numbers(&self, name: &str) -> Result<Option<Vec<f64>>, super::CallError> {
        let Some(ScalarValue::List(list)) = self.value(name)? else {
            return Ok(None);
        };
        let numbers = list.value(0);
        if numbers.null_count() > 0 {
            return Err(self.call.error_at(
                self.index(name),
                format!("`{name}` takes numbers, not NULL"),
            ));
        }
        Ok(Some(
            numbers.as_primitive::<Float64Type>().values().to_vec(),
        ))
    }

    /// The unit of the option `name`, when the call gives it.
    fn unit(&self, name: &str) -> Result<Option<Unit>, super::CallError> {
        let Some(text) = self.text(name)? else {
            return Ok(None);
        };
        match Unit::parse(&text) {
            Ok(unit) => Ok(Some(unit)),
            Err(DataFusionError::Plan(message)) => {
                let mut chars = message.chars();
                let message: String = (chars.next().into_iter())
                    .flat_map(char::to_uppercase)
                    .chain(chars)
                    .collect();
                Err(self.call.error_at(self.index(name), message))
            }
            Err(error) => Err(error.into()),
        }
    }

    /// An error of an option, at its argument.
    fn error(&self, error: OptionError) -> super::CallError {
        self.call.error_at(self.index(error.option), error.message)
    }
}

/// The window of a call of `bin`.
fn bin_window(o: &Options) -> Result<Arc<WindowUDF>, super::CallError> {
    let mut options = BinOptions::default();
    if let Some(maxbins) = o.number("maxbins")? {
        options.maxbins = maxbins;
    }
    if let Some(base) = o.number("base")? {
        options.base = base;
    }
    if let Some(divide) = o.numbers("divide")? {
        options.divide = divide;
    }
    if let Some(extent) = o.numbers("extent")? {
        let [low, high] = extent[..] else {
            return Err(o.call.error_at(
                o.index("extent"),
                "`extent` takes two numbers, as `[low, high]`",
            ));
        };
        options.extent = Some([low, high]);
    }
    options.span = o.number("span")?;
    options.step = o.number("step")?;
    options.steps = o.numbers("steps")?;
    if let Some(minstep) = o.number("minstep")? {
        options.minstep = minstep;
    }
    if let Some(nice) = o.boolean("nice")? {
        options.nice = nice;
    }
    options.anchor = o.number("anchor")?;
    options.check().map_err(|error| o.error(error))?;
    Ok(bin::window(options))
}

/// The window of a call of `stack`.
fn stack_window(o: &Options) -> Result<Arc<WindowUDF>, super::CallError> {
    let offset = match o.text("offset")? {
        // The form takes only the names of offsets.
        Some(name) => StackOffset::from_name(&name).expect("an offset of the form"),
        None => StackOffset::Zero,
    };
    Ok(stack::window(offset))
}

/// The window of a call of `timeunit`.
fn timeunit_window(o: &Options) -> Result<Arc<WindowUDF>, super::CallError> {
    let week_start = match o.text("week_start")? {
        // The form takes only the names of days.
        Some(name) => WeekStart::from_name(&name).expect("a week start of the form"),
        None => WeekStart::default(),
    };
    let options = TimeUnitOptions {
        unit: o.unit("unit")?,
        maxbins: o.integer("maxbins")?,
        min_unit: o.unit("min_unit")?,
        week_start,
        fiscal_year_start: o.integer("fiscal_year_start")?,
    };
    options.check().map_err(|error| o.error(error))?;
    let value = o.call.type_of(&o.call.arg(0))?;
    options.check_type(&value).map_err(|error| o.error(error))?;
    Ok(timeunit::window(options))
}
