//! The functions of `MATCH_RECOGNIZE`: `PREV` and `NEXT` of its `DEFINE`, and `FIRST`, `LAST`,
//! `MATCH_NUMBER`, `MATCH_ROW_NUMBER` and `CLASSIFIER` of its `MEASURES`.
//!
//! The pass allows each only in its clause. A call of `PREV`, `NEXT` or the functions of a match
//! plans as a marker, a function of the name that [`marker`] gives, which the pass replaces: `PREV`
//! and `NEXT` with `LAG` and `LEAD` over the clause's partition and order, and the functions of a
//! match with the columns that the clause's node computes. A marker never runs.

use std::sync::{Arc, LazyLock};

use datafusion::{
    arrow::datatypes::DataType,
    common::{plan_err, Result, ScalarValue},
    functions_aggregate::first_last::{first_value_udaf, last_value_udaf},
    logical_expr::{
        ColumnarValue, Expr, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};

use super::{Function, Planned};

/// The rows of the examples: the prices of a day each, by `id`.
const PRICES: &str = "FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 9), STRUCT(4, 12), STRUCT(5, 11)]) |> SELECT *";

/// The marker of `PREV`, `NEXT`, `MATCH_NUMBER`, `MATCH_ROW_NUMBER` or `CLASSIFIER`, which the
/// pass replaces: `$prev`, `$next`, `$match_number`, `$match_row_number` or `$classifier`.
pub(crate) fn marker(function: &str) -> String {
    format!("${}", function.to_ascii_lowercase())
}

/// The arguments of `expr` when it is a call of the marker of `function`.
pub(crate) fn marker_args<'e>(expr: &'e Expr, function: &str) -> Option<&'e [Expr]> {
    match expr {
        Expr::ScalarFunction(call) if call.func.name() == marker(function) => Some(&call.args),
        _ => None,
    }
}

pub(super) fn functions() -> Vec<Function> {
    vec![
        navigation("PREV", "before", "LAG")
            .example(&format!("{PRICES} |> MATCH_RECOGNIZE(ORDER BY id MEASURES MIN(id) AS first_id, COUNT(*) AS n PATTERN (up+) DEFINE up AS price > PREV(price))"), "first_id | n\n3 | 2"),
        navigation("NEXT", "after", "LEAD")
            .example(&format!("{PRICES} |> MATCH_RECOGNIZE(ORDER BY id MEASURES MIN(id) AS peak_id PATTERN (peak) DEFINE peak AS price > PREV(price) AND price > NEXT(price))"), "peak_id\n4"),
        end("FIRST", "first", first_value_udaf)
            .example(&format!("{PRICES} |> MATCH_RECOGNIZE(ORDER BY id MEASURES FIRST(price) AS first_price, LAST(price) AS last_price PATTERN (up+) DEFINE up AS price > PREV(price))"), "first_price | last_price\n9 | 12"),
        end("LAST", "last", last_value_udaf)
            .example(&format!("{PRICES} |> MATCH_RECOGNIZE(ORDER BY id MEASURES FIRST(dn.price) AS low, LAST(up.price) AS high PATTERN (dn up+) DEFINE dn AS price < PREV(price), up AS price > PREV(price))"), "low | high\n8 | 12"),
        of_match("MATCH_NUMBER", "Returns the number of the match in its partition, from 1.", DataType::Int64)
            .description("Matches take their numbers in the order of their first rows, and the numbers count empty matches and overlapping matches. A measure reads it outside aggregates, since it is the same for every row of a match.")
            .example(&format!("{PRICES} |> MATCH_RECOGNIZE(ORDER BY id MEASURES MATCH_NUMBER() AS m, MIN(id) AS first_id PATTERN (x x) DEFINE x AS TRUE)"), "m | first_id\n1 | 1\n2 | 3"),
        of_match("MATCH_ROW_NUMBER", "Returns the number of the row in its partition, from 1, in the order of `MATCH_RECOGNIZE`.", DataType::Int64)
            .description("A measure reads it in aggregates only, since each row of a match has its own. The number counts the rows of the partition, so the rows of a second match can be 3 and 4.")
            .note("GoogleSQL's docs number the rows of each match from 1. avenger-sql numbers them in the partition, as GoogleSQL's `execute_query` does.")
            .example(&format!("{PRICES} |> MATCH_RECOGNIZE(ORDER BY id MEASURES ARRAY_AGG(MATCH_ROW_NUMBER()) AS row_numbers PATTERN (x x) DEFINE x AS TRUE)"), "row_numbers\n[1, 2]\n[3, 4]"),
        of_match("CLASSIFIER", "Returns the pattern variable that the row matched, as `DEFINE` spells it.", DataType::Utf8)
            .description("A measure reads it in aggregates only, since each row of a match has its own.")
            .example(&format!("{PRICES} |> MATCH_RECOGNIZE(ORDER BY id MEASURES STRING_AGG(CLASSIFIER(), ',') AS variables PATTERN (dn up+) DEFINE dn AS price < PREV(price), up AS price > PREV(price))"), "variables\n'dn,up,up'"),
    ]
}

/// `PREV` or `NEXT`, which read the row `side` the row, as `window` does.
fn navigation(name: &str, side: &str, window: &str) -> Function {
    let marker_name = marker(name);
    Function::scalar(name, &format!("Returns a value of the row that is a number of rows {side} the row, in a `DEFINE` of `MATCH_RECOGNIZE`."))
        .description(&format!("Reads the rows of the partition in the order of `MATCH_RECOGNIZE`, whatever the matches, as `{window}` does over that partition and order. Returns `NULL` when no row is that far {side} the row."))
        .param("value", "The value of the row, which can't call `PREV` or `NEXT`.")
        .param("offset", "How many rows to read, an `INT64` literal or query parameter that is not negative or `NULL`. 0 reads the row. When omitted, 1.")
        .form("(value: T[, offset: INT64]) -> T where T: ANY", move |call| {
            let offset = (call.value(1).cloned())
                .unwrap_or_else(|| Expr::Literal(ScalarValue::Int64(Some(1)), None));
            Ok(Planned::Scalar(marker_udf(&marker_name).call(vec![call.arg(0), offset])))
        })
}

/// `FIRST` or `LAST`, the value of the `position` row of the match that isn't `NULL`.
fn end(
    name: &str,
    position: &str,
    udaf: fn() -> Arc<datafusion::logical_expr::AggregateUDF>,
) -> Function {
    Function::aggregate(name, &format!("Returns the value of the {position} row of the match, or of a pattern variable's rows, in a `MEASURES` of `MATCH_RECOGNIZE`."))
        .description("Reads the rows in the order of `MATCH_RECOGNIZE`, and leaves out the `NULL` values, as GoogleSQL's `execute_query` does. Returns `NULL` when no row has a value that isn't `NULL`, as for a variable without rows.")
        .param("value", &format!("The value of each row: of the match's rows, or of a pattern variable's rows, as `{name}(v.price)`."))
        .form("(value: T) -> T where T: ANY", move |call| {
            Ok(Planned::Aggregate {
                func: udaf(),
                args: vec![call.arg(0)],
                filter: None,
                result: None,
            })
        })
        .without_window()
}

/// `MATCH_NUMBER`, `MATCH_ROW_NUMBER` or `CLASSIFIER`, a value of the match or of its row of the
/// type `result`.
fn of_match(name: &str, summary: &str, result: DataType) -> Function {
    let marker_name = marker(name);
    let declaration = match result {
        DataType::Utf8 => "() -> STRING",
        _ => "() -> INT64",
    };
    Function::scalar(name, summary).form(declaration, move |_| {
        Ok(Planned::Scalar(
            marker_udf(&marker_name).call(vec![Expr::Literal(scalar_of(&result), None)]),
        ))
    })
}

/// A `NULL` of `ty`, which gives a marker without arguments its type.
fn scalar_of(ty: &DataType) -> ScalarValue {
    ScalarValue::try_from(ty).unwrap_or(ScalarValue::Null)
}

/// The marker of the name `name`, whose type is the type of its first argument.
fn marker_udf(name: &str) -> Arc<ScalarUDF> {
    static MARKERS: LazyLock<std::sync::Mutex<Vec<Arc<ScalarUDF>>>> =
        LazyLock::new(Default::default);
    let mut markers = MARKERS.lock().expect("the markers");
    if let Some(udf) = markers.iter().find(|udf| udf.name() == name) {
        return udf.clone();
    }
    let udf = Arc::new(ScalarUDF::new_from_impl(Marker {
        name: name.to_string(),
        signature: Signature::variadic_any(Volatility::Immutable),
    }));
    markers.push(udf.clone());
    udf
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct Marker {
    name: String,
    signature: Signature,
}

impl ScalarUDFImpl for Marker {
    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        Ok(arg_types.first().cloned().unwrap_or(DataType::Null))
    }

    fn invoke_with_args(&self, _args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        plan_err!(
            "{} runs only in MATCH_RECOGNIZE, which replaces it",
            self.name
        )
    }
}
