//! `lump`, the string form: a window function whose `by` and `keep` arguments are SQL text.
//!
//! ```sql
//! SELECT *, lump(port, 'n', 3, 'by', 'sum(tons)') OVER (PARTITION BY region) AS l FROM catches
//! SELECT *, lump(weather, 'keep', 'count(*) >= 0.05 * sum(count(*)) OVER ()',
//!                'other', 'Under 5%') OVER () AS l FROM seattle_weather
//! ```
//!
//! The options follow the category as pairs of a name and a constant:
//!
//! - `'n', k`: keep the `k` categories with the largest weights, ranked by weight
//!   descending with NULLs last, then by category ascending with NULLs last.
//! - `'keep', '<condition>'`: keep the categories for which the condition is true. It is
//!   evaluated once for each category, over the rows of the category in the window
//!   partition. It can use aggregates of the category's rows (`count(*)`), aggregates over
//!   the categories (`sum(count(*))`, which needs no `OVER`), and windows over the
//!   categories (`sum(count(*)) OVER (ORDER BY count(*) DESC ROWS UNBOUNDED PRECEDING)`).
//!   Every window runs within the lump's partition, so a `PARTITION BY` in `keep` is an
//!   error. A NULL condition drops the category.
//! - `'by', '<aggregate>'`: the weight of each category; `count(*)` by default.
//! - `'other', 'label'`: the label of the Other group; `'Other'` by default.
//!
//! The result is `struct<value, other, weight>`: for a kept category, the category and its
//! weight; for the Other group, the label and the weight of all rows of the dropped
//! categories. The weight is always DOUBLE, because the type of a window function is fixed
//! before anything parses the `by` string.
//!
//! The strings can read the rows of the window's input as the table `__input`, for example
//! in a subquery, which reads all of the input, not only the partition.
//!
//! [`LumpRewrite`] plans the strings and replaces the window with a grouped plan, which reads
//! the input two times, so a lump with `'keep'` or `'by'` cannot read a volatile input.
//! Without the rule, the window's own evaluator handles only `'n'` with the default `by`.
//! [`LumpChartFusion`] computes an aggregate grouped by a lump, as a chart does, from one
//! aggregate of the input.

use std::collections::HashMap;
use std::sync::Arc;

use avenger_datafusion_aggregate_state::Family;
use datafusion::arrow::array::{
    Array, ArrayRef, Float64Array, StringArray, StructArray, UInt32Array,
};
use datafusion::arrow::compute::take;
use datafusion::arrow::datatypes::{DataType, Field, FieldRef, Fields, SchemaRef};
use datafusion::arrow::row::{RowConverter, SortField};
use datafusion::common::config::ConfigOptions;
use datafusion::common::tree_node::{Transformed, TransformedResult};
use datafusion::common::{
    Column, Result, ScalarValue, TableReference, plan_datafusion_err, plan_err,
};
use datafusion::logical_expr::expr::WindowFunctionParams;
use datafusion::logical_expr::function::{PartitionEvaluatorArgs, WindowUDFFieldArgs};
use datafusion::logical_expr::planner::{ContextProvider, ExprPlanner};
use datafusion::logical_expr::registry::FunctionRegistry;
use datafusion::logical_expr::{
    AggregateUDF, Cast, Expr, HigherOrderUDF, LogicalPlan, LogicalPlanBuilder, LogicalTableSource,
    PartitionEvaluator, ScalarUDF, Signature, SubqueryAlias, TableSource, Volatility, Window,
    WindowUDF, WindowUDFImpl,
};
use datafusion::optimizer::analyzer::Analyzer;
use datafusion::optimizer::{ApplyOrder, OptimizerConfig, OptimizerRule};
use datafusion::physical_expr::expressions::Literal;
use datafusion::sql::parser::DFParser;
use datafusion::sql::planner::{ParserOptions, SqlToRel};
use datafusion::sql::sqlparser::ast::{
    Expr as SqlExpr, Ident, Visit, VisitMut, Visitor, VisitorMut, WindowSpec, WindowType,
};
use datafusion::sql::sqlparser::dialect::dialect_from_str;
use datafusion::sql::sqlparser::parser::Parser;
use datafusion::sql::sqlparser::tokenizer::Token;
use std::ops::ControlFlow;

// ---------------------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct LumpOptions {
    pub n: Option<i64>,
    pub keep: Option<String>,
    pub by: Option<String>,
    pub other: String,
}

impl LumpOptions {
    pub fn parse(pairs: &[ScalarValue]) -> Result<Self> {
        if !pairs.len().is_multiple_of(2) {
            return plan_err!(
                "lump options must be pairs of a name and a value, but the last name has no value"
            );
        }
        let mut o = LumpOptions {
            n: None,
            keep: None,
            by: None,
            other: "Other".into(),
        };
        let mut seen = Vec::new();
        for pair in pairs.chunks(2) {
            let Some(Some(name)) = pair[0].try_as_str() else {
                return plan_err!("a lump option name must be a string, not {}", pair[0]);
            };
            if seen.contains(&name) {
                return plan_err!("the lump option '{name}' is given twice");
            }
            seen.push(name);
            let v = &pair[1];
            let text = |what: &str| -> Result<String> {
                match v.try_as_str() {
                    Some(Some(s)) => Ok(s.to_string()),
                    _ => plan_err!("the lump option '{what}' must be a string, not {v}"),
                }
            };
            match name {
                "n" => {
                    let n = match v.cast_to(&DataType::Int64) {
                        Ok(ScalarValue::Int64(Some(n))) if v.data_type().is_integer() => n,
                        _ => {
                            return plan_err!(
                                "the lump option 'n' must be a positive integer, not {v}"
                            );
                        }
                    };
                    if n < 1 {
                        return plan_err!(
                            "the lump option 'n' must be a positive integer, not {n}"
                        );
                    }
                    o.n = Some(n);
                }
                "keep" => o.keep = Some(text("keep")?),
                "by" => o.by = Some(text("by")?),
                "other" => {
                    if let ScalarValue::Boolean(Some(false)) = v {
                        return plan_err!(
                            "lump cannot remove rows, so 'other' cannot be false; \
                             filter the dropped categories with `WHERE <lump>.other IS NULL`"
                        );
                    }
                    o.other = text("other")?;
                }
                _ => {
                    return plan_err!(
                        "unknown lump option '{name}': the options are n, keep, by, other"
                    );
                }
            }
        }
        if o.n.is_some() == o.keep.is_some() {
            return plan_err!("lump needs exactly one of the options 'n' and 'keep'");
        }
        Ok(o)
    }
}

/// The fields of the lump struct.
pub fn lump_fields(category: &DataType) -> Fields {
    Fields::from(vec![
        Field::new("value", category.clone(), true),
        Field::new("other", DataType::Utf8, true),
        Field::new("weight", DataType::Float64, true),
    ])
}

// ---------------------------------------------------------------------------------------
// The window function and its evaluator
// ---------------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct LumpWindow {
    signature: Signature,
}

impl Default for LumpWindow {
    fn default() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
}

impl WindowUDFImpl for LumpWindow {
    fn name(&self) -> &str {
        "lump"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn partition_evaluator(
        &self,
        args: PartitionEvaluatorArgs,
    ) -> Result<Box<dyn PartitionEvaluator>> {
        let options = args.input_exprs()[1..]
            .iter()
            .map(|e| {
                e.downcast_ref::<Literal>()
                    .map(|l| l.value().clone())
                    .ok_or_else(|| plan_datafusion_err!("lump options must be constants, not {e}"))
            })
            .collect::<Result<Vec<_>>>()?;
        let o = LumpOptions::parse(&options)?;
        let simple = o.keep.is_none()
            && o.by
                .as_deref()
                .is_none_or(|b| b.trim().eq_ignore_ascii_case("count(*)"));
        if !simple {
            return plan_err!(
                "lump with a 'keep' or a 'by' string needs the lump rewrite rule, which plans the strings"
            );
        }
        Ok(Box::new(LumpEvaluator {
            n: o.n.unwrap() as usize,
            other: o.other,
        }))
    }
    fn field(&self, args: WindowUDFFieldArgs) -> Result<FieldRef> {
        let Some(category) = args.input_fields().first() else {
            return plan_err!("lump needs a category");
        };
        let t = category.data_type();
        if matches!(
            t,
            DataType::List(_)
                | DataType::LargeList(_)
                | DataType::FixedSizeList(_, _)
                | DataType::Map(_, _)
        ) {
            return plan_err!("a lump category cannot be a list or a map, not {t}");
        }
        Ok(Arc::new(Field::new(
            args.name(),
            DataType::Struct(lump_fields(t)),
            false,
        )))
    }
}

/// `'n'` with the weight `count(*)`, over a whole partition in memory.
#[derive(Debug)]
struct LumpEvaluator {
    n: usize,
    other: String,
}

impl PartitionEvaluator for LumpEvaluator {
    fn evaluate_all(&mut self, values: &[ArrayRef], num_rows: usize) -> Result<ArrayRef> {
        let category = &values[0];
        // Ascending with NULLs last, so that comparing the row bytes orders the categories as
        // the tie break of the ranking does.
        let conv = RowConverter::new(vec![SortField::new_with_options(
            category.data_type().clone(),
            datafusion::arrow::compute::SortOptions {
                descending: false,
                nulls_first: false,
            },
        )])?;
        let rows = conv.convert_columns(std::slice::from_ref(category))?;
        let mut index: HashMap<&[u8], usize> = HashMap::new();
        let mut first_row: Vec<usize> = Vec::new();
        let mut counts: Vec<f64> = Vec::new();
        let mut row_cat = Vec::with_capacity(num_rows);
        for i in 0..num_rows {
            let c = *index.entry(rows.row(i).data()).or_insert_with(|| {
                first_row.push(i);
                counts.push(0.0);
                counts.len() - 1
            });
            counts[c] += 1.0;
            row_cat.push(c);
        }
        let mut order: Vec<usize> = (0..counts.len()).collect();
        order.sort_by(|&a, &b| {
            counts[b]
                .total_cmp(&counts[a])
                .then_with(|| rows.row(first_row[a]).cmp(&rows.row(first_row[b])))
        });
        let mut kept = vec![false; counts.len()];
        for &c in order.iter().take(self.n) {
            kept[c] = true;
        }
        let other_weight: f64 = (0..counts.len())
            .filter(|&c| !kept[c])
            .map(|c| counts[c])
            .sum();

        let value_idx: UInt32Array = row_cat
            .iter()
            .enumerate()
            .map(|(i, &c)| kept[c].then_some(i as u32))
            .collect();
        let value = take(category, &value_idx, None)?;
        let other: StringArray = row_cat
            .iter()
            .map(|&c| (!kept[c]).then_some(self.other.as_str()))
            .collect();
        let weight: Float64Array = row_cat
            .iter()
            .map(|&c| Some(if kept[c] { counts[c] } else { other_weight }))
            .collect();
        Ok(Arc::new(StructArray::new(
            lump_fields(category.data_type()),
            vec![value, Arc::new(other), Arc::new(weight)],
            None,
        )))
    }
}

// ---------------------------------------------------------------------------------------
// Planning the strings
// ---------------------------------------------------------------------------------------

/// A context provider for the generated SQL: the session's functions and one table,
/// `__input`.
struct Provider<'a> {
    /// The input's own name in the generated SQL.
    name: &'a str,
    registry: &'a dyn FunctionRegistry,
    options: Arc<ConfigOptions>,
    planners: Vec<Arc<dyn ExprPlanner>>,
    input: Arc<dyn TableSource>,
}

impl ContextProvider for Provider<'_> {
    fn get_table_source(&self, name: TableReference) -> Result<Arc<dyn TableSource>> {
        if name.table() == INPUT || name.table() == self.name {
            Ok(self.input.clone())
        } else {
            plan_err!(
                "the strings of lump can read only the rows of the window, not the table {name}"
            )
        }
    }
    fn get_expr_planners(&self) -> &[Arc<dyn ExprPlanner>] {
        &self.planners
    }
    fn get_function_meta(&self, name: &str) -> Option<Arc<ScalarUDF>> {
        self.registry.udf(name).ok()
    }
    fn get_higher_order_meta(&self, name: &str) -> Option<Arc<HigherOrderUDF>> {
        self.registry.higher_order_function(name).ok()
    }
    fn get_aggregate_meta(&self, name: &str) -> Option<Arc<AggregateUDF>> {
        self.registry.udaf(name).ok()
    }
    fn get_window_meta(&self, name: &str) -> Option<Arc<WindowUDF>> {
        self.registry.udwf(name).ok()
    }
    fn get_variable_type(&self, _: &[String]) -> Option<DataType> {
        None
    }
    fn options(&self) -> &ConfigOptions {
        &self.options
    }
    fn udf_names(&self) -> Vec<String> {
        self.registry.udfs().into_iter().collect()
    }
    fn higher_order_function_names(&self) -> Vec<String> {
        self.registry
            .higher_order_function_names()
            .into_iter()
            .collect()
    }
    fn udaf_names(&self) -> Vec<String> {
        self.registry.udafs().into_iter().collect()
    }
    fn udwf_names(&self) -> Vec<String> {
        self.registry.udwfs().into_iter().collect()
    }
}

const INPUT: &str = "__input";

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Parses an expression string of an option, for errors that name the option.
fn parse_expr(dialect: &str, what: &str, text: &str) -> Result<SqlExpr> {
    let dialect = dialect_from_str(dialect)
        .unwrap_or_else(|| Box::new(datafusion::sql::sqlparser::dialect::GenericDialect {}));
    Parser::new(dialect.as_ref())
        .try_with_sql(text)
        .and_then(|mut p| {
            let e = p.parse_expr()?;
            p.expect_token(&Token::EOF)?;
            Ok(e)
        })
        .map_err(|e| plan_datafusion_err!("the lump option '{what}' is not an expression: {e}"))
}

/// Whether an expression has a window function (a call with `OVER`).
fn has_window(expr: &SqlExpr) -> bool {
    datafusion::sql::sqlparser::ast::visit_expressions(expr, |e| match e {
        SqlExpr::Function(f) if f.over.is_some() => ControlFlow::Break(()),
        _ => ControlFlow::Continue(()),
    })
    .is_break()
}

/// Whether a call is an aggregate: a registered aggregate function without `OVER`.
fn is_aggregate(
    registry: &dyn FunctionRegistry,
    f: &datafusion::sql::sqlparser::ast::Function,
) -> bool {
    f.over.is_none()
        && registry
            .udaf(&f.name.to_string().to_ascii_lowercase())
            .is_ok()
}

/// The deepest nesting of aggregates in an expression: 1 for `count(*)`, 2 for
/// `sum(count(*))`. Windows (calls with `OVER`) do not count.
fn aggregate_depth(registry: &dyn FunctionRegistry, expr: &SqlExpr) -> usize {
    struct Depth<'a> {
        registry: &'a dyn FunctionRegistry,
        stack: Vec<bool>,
        depth: usize,
        max: usize,
    }
    impl Visitor for Depth<'_> {
        type Break = ();
        fn pre_visit_expr(&mut self, expr: &SqlExpr) -> ControlFlow<()> {
            let agg = matches!(expr, SqlExpr::Function(f) if is_aggregate(self.registry, f));
            if agg {
                self.depth += 1;
                self.max = self.max.max(self.depth);
            }
            self.stack.push(agg);
            ControlFlow::Continue(())
        }
        fn post_visit_expr(&mut self, _: &SqlExpr) -> ControlFlow<()> {
            if self.stack.pop() == Some(true) {
                self.depth -= 1;
            }
            ControlFlow::Continue(())
        }
    }
    let mut d = Depth {
        registry,
        stack: Vec::new(),
        depth: 0,
        max: 0,
    };
    let _ = Visit::visit(expr, &mut d);
    d.max
}

/// Scopes the windows of `keep` to the categories of the lump's partition:
///
/// - A window gets the partition keys of the lump. An explicit `PARTITION BY` is an error.
/// - An aggregate of an aggregate without `OVER`, such as `sum(count(*))`, is a window
///   over the categories: it gets `OVER (PARTITION BY <keys>)`.
/// - Three levels of aggregates are an error.
struct KeepScope<'a> {
    keys: Vec<SqlExpr>,
    registry: &'a dyn FunctionRegistry,
}

impl VisitorMut for KeepScope<'_> {
    type Break = String;
    fn pre_visit_expr(&mut self, expr: &mut SqlExpr) -> ControlFlow<Self::Break> {
        let depth = aggregate_depth(self.registry, expr);
        let SqlExpr::Function(f) = expr else {
            return ControlFlow::Continue(());
        };
        if is_aggregate(self.registry, f) {
            match depth {
                0 | 1 => {}
                2 => {
                    f.over = Some(WindowType::WindowSpec(WindowSpec {
                        window_name: None,
                        partition_by: self.keys.clone(),
                        order_by: vec![],
                        window_frame: None,
                    }));
                }
                _ => {
                    return ControlFlow::Break(format!(
                        "`{f}` nests three aggregates; the 'keep' of lump allows an aggregate of each \
                         category's rows (such as count(*)) and an aggregate over the categories (such as sum(count(*)))"
                    ));
                }
            }
            return ControlFlow::Continue(());
        }
        match &mut f.over {
            Some(WindowType::WindowSpec(spec)) => {
                if !spec.partition_by.is_empty() {
                    return ControlFlow::Break(
                        "a window in the 'keep' of lump already runs over the categories of the lump's \
                         partition; remove its PARTITION BY"
                            .into(),
                    );
                }
                if spec.window_name.is_some() {
                    return ControlFlow::Break(
                        "named windows are not supported in the 'keep' of lump".into(),
                    );
                }
                spec.partition_by = self.keys.clone();
            }
            Some(WindowType::NamedWindow(_)) => {
                return ControlFlow::Break(
                    "named windows are not supported in the 'keep' of lump".into(),
                );
            }
            None => {}
        }
        ControlFlow::Continue(())
    }
}

/// The aggregate that combines the weights of categories into the weight of the Other
/// group, when `by` is one that combines: `count` and `sum` add, `min` and `max` combine.
fn combining(by: &SqlExpr) -> Option<&'static str> {
    let SqlExpr::Function(f) = by else {
        return None;
    };
    if f.over.is_some() || f.filter.is_some() {
        return None;
    }
    if let datafusion::sql::sqlparser::ast::FunctionArguments::List(list) = &f.args
        && list.duplicate_treatment.is_some()
    {
        return None;
    }
    match f.name.to_string().to_ascii_lowercase().as_str() {
        "count" | "sum" => Some("sum"),
        "min" => Some("min"),
        "max" => Some("max"),
        _ => None,
    }
}

/// Replaces `lump(...) OVER (...)` with a grouped plan built from its strings: the decision
/// of each category, joined back to the rows on the partition keys and the category.
#[derive(Debug, Default)]
pub struct LumpRewrite;

/// The parameters of a window node whose only function is one `lump` call.
fn lump_call(window: &Window) -> Option<&WindowFunctionParams> {
    let [expr] = &window.window_expr[..] else {
        return None;
    };
    let wf = crate::window_call(expr).filter(|_| crate::is_call::<LumpWindow>(expr))?;
    (!wf.params.args.is_empty()).then_some(&wf.params)
}

/// What the lump rules share: the input with the category and the partition keys as
/// columns, and the SQL of the decisions, `decided` and `decisions`, over the table
/// `__input`.
struct LumpPlan<'a> {
    /// The name of the input in the generated SQL, which no other lump rewrite uses, so that
    /// a lump over a lump does not have two relations of one name.
    input: String,
    registry: &'a dyn FunctionRegistry,
    options: Arc<ConfigOptions>,
    dialect: String,
    /// The number of columns of the window's input.
    n_in: usize,
    category_type: DataType,
    /// `__key_0`, ...: the partition keys as columns of `__input`.
    keys: Vec<String>,
    label: String,
    /// `by` and the condition of keep (from `'n'` or `'keep'`, with its windows scoped), as
    /// SQL over the columns of `__input`.
    by: String,
    keep: String,
    /// The names that the category and the partition keys group by: their column names
    /// when they are columns, else `__cat` and `__key_i`.
    key_groups: Vec<String>,
    cat_group: String,
    decided: String,
    decisions: String,
    projected: LogicalPlan,
    table_schema: SchemaRef,
}

impl<'a> LumpPlan<'a> {
    fn new(
        window: &Window,
        params: &WindowFunctionParams,
        config: &'a dyn OptimizerConfig,
    ) -> Result<Self> {
        crate::whole_partition("lump", params)?;
        let literals = params.args[1..]
            .iter()
            .map(|e| match e {
                Expr::Literal(v, _) => Ok(v.clone()),
                e => plan_err!("lump options must be constants, not {e}"),
            })
            .collect::<Result<Vec<_>>>()?;
        let o = LumpOptions::parse(&literals)?;
        let registry = config.function_registry().ok_or_else(|| {
            plan_datafusion_err!("the lump rule needs the session's functions to plan its strings")
        })?;
        let options = config.options();
        let dialect = options.sql_parser.dialect.to_string();
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let input_name = format!(
            "{INPUT}_{}",
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );

        // The input, with the category and the partition keys as columns.
        let input = window.input.as_ref().clone();
        let n_in = input.schema().fields().len();
        let category = params.args[0].clone();
        let DataType::Struct(fields) = window.schema.field(n_in).data_type() else {
            unreachable!()
        };
        let category_type = fields[0].data_type().clone();
        let mut columns: Vec<Expr> = input
            .schema()
            .columns()
            .into_iter()
            .map(Expr::Column)
            .collect();
        columns.push(category.clone().alias("__cat"));
        for (i, k) in params.partition_by.iter().enumerate() {
            columns.push(k.clone().alias(format!("__key_{i}")));
        }
        let projected = LogicalPlanBuilder::from(input).project(columns)?.build()?;
        let names: Vec<String> = projected
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect();
        if (1..names.len()).any(|i| names[..i].contains(&names[i])) {
            return plan_err!("lump cannot read an input with two columns of the same name");
        }
        let table_schema: SchemaRef = Arc::new(projected.schema().as_arrow().clone());

        // A category or key that is a column groups by its name, so that `keep` can refer
        // to it.
        let group_name = |e: &Expr, alias: &str| match e {
            Expr::Column(c) => quote(&c.name),
            _ => quote(alias),
        };
        let keys: Vec<String> = (0..params.partition_by.len())
            .map(|i| format!("__key_{i}"))
            .collect();
        let key_groups: Vec<String> = params
            .partition_by
            .iter()
            .zip(&keys)
            .map(|(e, k)| group_name(e, k))
            .collect();
        let cat_group = group_name(&category, "__cat");
        let by_text = o.by.clone().unwrap_or_else(|| "count(*)".into());
        let by_ast = parse_expr(&dialect, "by", &by_text)?;
        if has_window(&by_ast) {
            return plan_err!(
                "the 'by' of lump is the weight of one category, so it cannot have a window: {by_text}"
            );
        }
        if aggregate_depth(registry, &by_ast) > 1 {
            return plan_err!(
                "the 'by' of lump is the weight of one category, so it cannot be an aggregate of aggregates: {by_text}"
            );
        }
        let by = by_ast.to_string();
        let part = if keys.is_empty() {
            String::new()
        } else {
            format!(
                "PARTITION BY {}",
                keys.iter().map(|k| quote(k)).collect::<Vec<_>>().join(", ")
            )
        };
        let keep = match (&o.n, &o.keep) {
            (Some(n), None) => format!(
                "row_number() OVER ({} ORDER BY ({by}) DESC NULLS LAST, {cat_group} ASC NULLS LAST) <= {n}",
                if key_groups.is_empty() {
                    String::new()
                } else {
                    format!("PARTITION BY {}", key_groups.join(", "))
                }
            ),
            (None, Some(keep)) => {
                let mut ast = parse_expr(&dialect, "keep", keep)?;
                let key_exprs = key_groups
                    .iter()
                    .map(|k| SqlExpr::Identifier(Ident::with_quote('"', k.trim_matches('"'))))
                    .collect();
                if let ControlFlow::Break(e) = VisitMut::visit(
                    &mut ast,
                    &mut KeepScope {
                        keys: key_exprs,
                        registry,
                    },
                ) {
                    return plan_err!("{e}");
                }
                ast.to_string()
            }
            _ => unreachable!("checked by the options"),
        };
        let select_keys: String = key_groups
            .iter()
            .zip(&keys)
            .map(|(g, k)| format!("{g} AS {}, ", quote(k)))
            .collect();
        let group_by: Vec<String> = key_groups
            .iter()
            .cloned()
            .chain([cat_group.clone()])
            .collect();
        let decided = format!(
            "SELECT {select_keys}{cat_group} AS \"__cat\", CAST(({by}) AS DOUBLE) AS \"__weight\", coalesce(({keep}), false) AS \"__keep\" \
             FROM {input_name} GROUP BY {}",
            group_by.join(", ")
        );
        let mut plan = Self {
            input: input_name.clone(),
            registry,
            options: options.clone(),
            dialect,
            n_in,
            category_type,
            keys,
            label: o.other.replace('\'', "''"),
            by: by.clone(),
            keep: keep.clone(),
            key_groups: key_groups.clone(),
            cat_group: cat_group.clone(),
            decided,
            decisions: String::new(),
            projected,
            table_schema,
        };
        if o.keep.is_some() {
            let probe = format!(
                "SELECT ({keep}) AS k FROM {input_name} GROUP BY {}",
                group_by.join(", ")
            );
            let probe = plan.to_rel(&probe)?;
            let dt = probe.schema().field(0).data_type().clone();
            if dt != DataType::Boolean && dt != DataType::Null {
                return plan_err!(
                    "the 'keep' of lump must be a condition, not an expression of type {dt}"
                );
            }
        }
        plan.decisions = match combining(&by_ast) {
            Some(agg) => format!(
                "SELECT *, {agg}(CASE WHEN \"__keep\" THEN NULL ELSE \"__weight\" END) OVER ({part}) AS \"__other\" FROM decided"
            ),
            None => {
                let k = &plan.keys;
                let ikeys: String = k.iter().map(|k| format!("i.{}, ", quote(k))).collect();
                let igroup = if k.is_empty() {
                    String::new()
                } else {
                    format!(
                        " GROUP BY {}",
                        k.iter()
                            .map(|k| format!("i.{}", quote(k)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                format!(
                    "SELECT d.*, o.\"__other\" FROM decided d LEFT JOIN \
                     (SELECT {ikeys}CAST(({by}) AS DOUBLE) AS \"__other\" FROM {input_name} i JOIN decided dd ON {} \
                      WHERE NOT dd.\"__keep\"{igroup}) o ON {}",
                    plan.on("i", "dd", true),
                    plan.on("d", "o", false)
                )
            }
        };
        Ok(plan)
    }

    /// The null-safe join condition on the partition keys, and the category if `with_cat`.
    fn on(&self, a: &str, b: &str, with_cat: bool) -> String {
        let mut c: Vec<String> = self
            .keys
            .iter()
            .map(|k| format!("({a}.{k} IS NOT DISTINCT FROM {b}.{k})", k = quote(k)))
            .collect();
        if with_cat {
            c.push(format!(
                "({a}.\"__cat\" IS NOT DISTINCT FROM {b}.\"__cat\")"
            ));
        }
        if c.is_empty() {
            "true".into()
        } else {
            c.join(" AND ")
        }
    }

    /// The CTEs of the decisions, to put before a final `SELECT`.
    fn with(&self) -> String {
        format!(
            "WITH decided AS ({}), decisions AS ({}) ",
            self.decided, self.decisions
        )
    }

    /// The three fields of the lump, for rows of `__cat` joined to the decisions as `d`.
    fn fields_sql(&self, cat: &str) -> String {
        format!(
            "CASE WHEN d.\"__keep\" THEN {cat} END AS \"__lvalue\", \
             CASE WHEN d.\"__keep\" THEN NULL ELSE '{label}' END AS \"__lother\", \
             CASE WHEN d.\"__keep\" THEN d.\"__weight\" ELSE d.\"__other\" END AS \"__lweight\"",
            label = self.label
        )
    }

    /// Plans SQL over `__input`, without the input.
    fn to_rel(&self, sql: &str) -> Result<LogicalPlan> {
        let provider = Provider {
            name: &self.input,
            registry: self.registry,
            options: self.options.clone(),
            planners: self.registry.expr_planners(),
            input: Arc::new(LogicalTableSource::new(self.table_schema.clone())),
        };
        let dialect = dialect_from_str(&self.dialect)
            .unwrap_or_else(|| Box::new(datafusion::sql::sqlparser::dialect::GenericDialect {}));
        let mut statements = DFParser::parse_sql_with_dialect(sql, dialect.as_ref())?;
        let statement = statements
            .pop_front()
            .ok_or_else(|| plan_datafusion_err!("no statement"))?;
        let planner =
            SqlToRel::new_with_options(&provider, ParserOptions::from(&self.options.sql_parser));
        planner.statement_to_plan(statement).map_err(|e| {
            plan_datafusion_err!(
                "lump could not plan its 'by' or 'keep': {}",
                e.strip_backtrace()
            )
        })
    }

    /// Plans SQL over `__input`, then puts the real input in its place.
    fn plan(&self, sql: &str) -> Result<LogicalPlan> {
        let generated = self
            .to_rel(sql)?
            .transform_up_with_subqueries(|p| match &p {
                LogicalPlan::TableScan(scan) if self.reads(&scan.table_name) => Ok(
                    Transformed::yes(LogicalPlan::SubqueryAlias(SubqueryAlias::try_new(
                        Arc::new(self.projected.clone()),
                        scan.table_name.clone(),
                    )?)),
                ),
                _ => Ok(Transformed::no(p)),
            })
            .data()?;
        Analyzer::new().execute_and_check(generated, &self.options, |_, _| {})
    }

    /// Whether a table of the generated SQL is the input: `__input` in the strings of the
    /// options, and the input's own name in the SQL of the rule.
    fn reads(&self, table: &TableReference) -> bool {
        table.table() == INPUT || table.table() == self.input
    }

    /// The lump struct from the columns `__lvalue`, `__lother`, and `__lweight` of `schema`.
    fn struct_expr(&self, schema: &datafusion::common::DFSchema) -> Result<Expr> {
        let col = |name: &str| -> Result<Expr> {
            Ok(Expr::Column(
                schema
                    .columns_with_unqualified_name(name)
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        plan_datafusion_err!("lump: the generated plan has no column {name}")
                    })?,
            ))
        };
        let parts = vec![
            Expr::Cast(Cast::new(
                Box::new(col("__lvalue")?),
                self.category_type.clone(),
            )),
            Expr::Cast(Cast::new(Box::new(col("__lother")?), DataType::Utf8)),
            Expr::Cast(Cast::new(Box::new(col("__lweight")?), DataType::Float64)),
        ];
        Ok(crate::make_struct_udf(lump_fields(&self.category_type)).call(parts))
    }
}

impl OptimizerRule for LumpRewrite {
    fn name(&self) -> &str {
        "lump_rewrite"
    }

    fn apply_order(&self) -> Option<ApplyOrder> {
        Some(ApplyOrder::BottomUp)
    }

    fn rewrite(
        &self,
        plan: LogicalPlan,
        config: &dyn OptimizerConfig,
    ) -> Result<Transformed<LogicalPlan>> {
        let LogicalPlan::Window(window) = &plan else {
            return Ok(Transformed::no(plan));
        };
        // A window node with other calls: the first lump in a window node of its own, under
        // the others, which may be lumps too.
        let first = window
            .window_expr
            .iter()
            .position(crate::is_call::<LumpWindow>);
        if let Some(split) = crate::split_window(window, |i, _| Some(i) == first)? {
            let lower = self.rewrite(split.lower.clone(), config)?.data;
            return Ok(Transformed::yes(
                split.rebuild(lower, |upper| Ok(self.rewrite(upper, config)?.data))?,
            ));
        }
        let Some(params) = lump_call(window) else {
            return Ok(Transformed::no(plan));
        };
        // The rewrite reads the input twice: for the decisions, and for the rows. The window
        // evaluator reads it once, but takes only 'n' with the default weight.
        if crate::is_volatile(&window.input)?
            || params
                .args
                .iter()
                .chain(&params.partition_by)
                .any(Expr::is_volatile)
        {
            let o = LumpOptions::parse(&crate::literals(&params.args[1..]).unwrap_or_default())?;
            if o.keep.is_none() && o.by.is_none() {
                return Ok(Transformed::no(plan));
            }
            return plan_err!(
                "lump with 'keep' or 'by' reads its input two times, so the input cannot have a volatile function such as random()"
            );
        }
        let lp = LumpPlan::new(window, params, config)?;
        let sql = format!(
            "{} SELECT {input}.*, {} FROM {input} JOIN decisions d ON {}",
            lp.with(),
            lp.fields_sql(&format!("{input}.\"__cat\"", input = lp.input)),
            lp.on(&lp.input, "d", true),
            input = lp.input
        );
        let generated = lp.plan(&sql)?;

        // The output of the window: its input columns, then the lump struct.
        let out = generated.schema().clone();
        let mut exprs: Vec<Expr> = Vec::with_capacity(lp.n_in + 1);
        for i in 0..lp.n_in {
            let (q, f) = window.schema.qualified_field(i);
            let (oq, of) = out.qualified_field(i);
            exprs.push(Expr::Column(Column::from((oq, of))).alias_qualified(q.cloned(), f.name()));
        }
        let (q, f) = window.schema.qualified_field(lp.n_in);
        exprs.push(lp.struct_expr(&out)?.alias_qualified(q.cloned(), f.name()));
        Ok(Transformed::yes(
            LogicalPlanBuilder::from(generated)
                .project(exprs)?
                .build()?,
        ))
    }
}

// ---------------------------------------------------------------------------------------
// The chart fusion rule
// ---------------------------------------------------------------------------------------

/// What a column of a plan above a lump window comes from.
#[derive(Debug, Clone, PartialEq)]
enum Source {
    /// The lump.
    Lump,
    /// A column of the window's input.
    Input(usize),
}

/// For each column of `plan`, its source, through aliases and projections of columns, down
/// to a lump window.
fn sources(plan: &LogicalPlan) -> Option<(Vec<Option<Source>>, &Window)> {
    match plan {
        LogicalPlan::Window(w) => {
            lump_call(w)?;
            let n_in = w.input.schema().fields().len();
            Some((
                (0..=n_in)
                    .map(|j| {
                        Some(if j == n_in {
                            Source::Lump
                        } else {
                            Source::Input(j)
                        })
                    })
                    .collect(),
                w,
            ))
        }
        LogicalPlan::SubqueryAlias(a) => sources(&a.input),
        LogicalPlan::Projection(p) => {
            let (inner, w) = sources(&p.input)?;
            let out = p
                .expr
                .iter()
                .map(|e| {
                    let e = match e {
                        Expr::Alias(a) => a.expr.as_ref(),
                        e => e,
                    };
                    match e {
                        Expr::Column(c) => p
                            .input
                            .schema()
                            .index_of_column(c)
                            .ok()
                            .and_then(|i| inner[i].clone()),
                        _ => None,
                    }
                })
                .collect();
            Some((out, w))
        }
        _ => None,
    }
}

/// The aggregate-state family of an aggregate function name, as DataFusion and SQL name it.
fn family(name: &str) -> Option<Family> {
    Some(match name.to_ascii_lowercase().as_str() {
        "count" => Family::Count,
        "sum" => Family::Sum,
        "min" => Family::Min,
        "max" => Family::Max,
        "avg" | "mean" => Family::Avg,
        "var" | "variance" | "var_samp" | "var_sample" => Family::VarSamp,
        "var_pop" | "var_population" => Family::VarPop,
        "stddev" | "stddev_samp" => Family::StddevSamp,
        "stddev_pop" => Family::StddevPop,
        _ => return None,
    })
}

/// Rewrites every aggregate of an aggregate-state family in an expression, `f(args)`, to
/// `f_merge("<prefix>i")`, and collects `f_state(args)` for each. Another aggregate, or an
/// aggregate with DISTINCT, FILTER, or WITHIN GROUP, cannot merge states: the rewrite fails.
struct ToStates<'a> {
    registry: &'a dyn FunctionRegistry,
    prefix: &'a str,
    states: Vec<(Family, String)>,
}

impl VisitorMut for ToStates<'_> {
    type Break = ();
    fn pre_visit_expr(&mut self, expr: &mut SqlExpr) -> ControlFlow<()> {
        // A subquery reads `__input` again, which `fine` does not hold.
        if matches!(
            expr,
            SqlExpr::Subquery(_) | SqlExpr::Exists { .. } | SqlExpr::InSubquery { .. }
        ) {
            return ControlFlow::Break(());
        }
        let SqlExpr::Function(f) = expr else {
            return ControlFlow::Continue(());
        };
        if f.over.is_some() {
            return ControlFlow::Continue(());
        }
        let name = f.name.to_string();
        let Some(fam) = family(&name) else {
            return if self.registry.udaf(&name.to_ascii_lowercase()).is_ok() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            };
        };
        use datafusion::sql::sqlparser::ast::{
            FunctionArg as FA, FunctionArgExpr as A, FunctionArguments,
        };
        let FunctionArguments::List(list) = &f.args else {
            return ControlFlow::Break(());
        };
        if list.duplicate_treatment.is_some()
            || f.filter.is_some()
            || !f.within_group.is_empty()
            || !list.clauses.is_empty()
        {
            return ControlFlow::Break(());
        }
        let args: Vec<String> = list
            .args
            .iter()
            .filter_map(|a| match a {
                FA::Unnamed(A::Wildcard) => None,
                a => Some(a.to_string()),
            })
            .collect();
        let i = self.states.len();
        self.states
            .push((fam, format!("{}_state({})", fam.name(), args.join(", "))));
        let text = format!("{}_merge(\"{}{i}\")", fam.name(), self.prefix);
        let Ok(merged) = Parser::new(&datafusion::sql::sqlparser::dialect::GenericDialect {})
            .try_with_sql(&text)
            .and_then(|mut p| p.parse_expr())
        else {
            return ControlFlow::Break(());
        };
        *expr = merged;
        ControlFlow::Continue(())
    }
}

/// An expression in one form, so that equal states compare equal: identifiers quoted, and
/// unquoted ones normalized to lower case if the session normalizes identifiers.
fn canonical(dialect: &str, sql: &str, normalize: bool) -> String {
    struct Quote(bool);
    impl VisitorMut for Quote {
        type Break = ();
        fn pre_visit_expr(&mut self, expr: &mut SqlExpr) -> ControlFlow<()> {
            let normalize = self.0;
            let fix = |i: &mut Ident| {
                if i.quote_style.is_none() {
                    if normalize {
                        i.value = i.value.to_ascii_lowercase();
                    }
                    i.quote_style = Some('"');
                }
            };
            match expr {
                SqlExpr::Identifier(i) => fix(i),
                SqlExpr::CompoundIdentifier(ids) => ids.iter_mut().for_each(fix),
                _ => {}
            }
            ControlFlow::Continue(())
        }
    }
    match parse_expr(dialect, "", sql) {
        Ok(mut ast) => {
            let _ = VisitMut::visit(&mut ast, &mut Quote(normalize));
            ast.to_string()
        }
        Err(_) => sql.to_string(),
    }
}

/// `text` with its aggregates as merges of states, and the states; None if an aggregate
/// cannot merge states.
fn to_states(
    registry: &dyn FunctionRegistry,
    dialect: &str,
    text: &str,
    prefix: &str,
) -> Option<(String, Vec<(Family, String)>)> {
    let mut ast = parse_expr(dialect, "", text).ok()?;
    let mut v = ToStates {
        registry,
        prefix,
        states: Vec::new(),
    };
    if let ControlFlow::Break(()) = VisitMut::visit(&mut ast, &mut v) {
        return None;
    }
    Some((ast.to_string(), v.states))
}

/// Computes `GROUP BY <lump>, ...` with aggregates of the aggregate-state families
/// (`count`, `sum`, `min`, `max`, `avg`, `var_samp`, `var_pop`, `stddev_samp`,
/// `stddev_pop`) of columns, from one aggregate of the window's input, `fine`, grouped by
/// the partition keys, the category, and the other group columns. `fine` holds a state of
/// each of the chart's aggregates and of each aggregate of `by` and `keep`:
///
/// - the decisions merge the states of `by` and `keep` for each category;
/// - the weight of the Other group merges the states of `by` of the dropped categories;
/// - the chart merges the states of its aggregates for each lump value.
///
/// So the input is read once. When `by` or `keep` has an aggregate without a state (such
/// as `median`), the decisions read the input again, as in `LumpRewrite`.
#[derive(Debug, Default)]
pub struct LumpChartFusion;

impl OptimizerRule for LumpChartFusion {
    fn name(&self) -> &str {
        "lump_chart_fusion"
    }

    fn apply_order(&self) -> Option<ApplyOrder> {
        Some(ApplyOrder::TopDown)
    }

    fn rewrite(
        &self,
        plan: LogicalPlan,
        config: &dyn OptimizerConfig,
    ) -> Result<Transformed<LogicalPlan>> {
        // Where the fused plan fails, the lump rule plans the lump, or reports its error.
        match self.fuse(plan.clone(), config) {
            Ok(t) => Ok(t),
            Err(_) => Ok(Transformed::no(plan)),
        }
    }
}

impl LumpChartFusion {
    fn fuse(
        &self,
        plan: LogicalPlan,
        config: &dyn OptimizerConfig,
    ) -> Result<Transformed<LogicalPlan>> {
        let LogicalPlan::Aggregate(agg) = &plan else {
            return Ok(Transformed::no(plan));
        };
        let Some((map, window)) = sources(&agg.input) else {
            return Ok(Transformed::no(plan));
        };
        if crate::is_volatile(&window.input)? {
            return Ok(Transformed::no(plan));
        }
        let input_schema = agg.input.schema();
        let source = |e: &Expr| -> Option<Source> {
            match e {
                Expr::Column(c) => input_schema
                    .index_of_column(c)
                    .ok()
                    .and_then(|i| map[i].clone()),
                _ => None,
            }
        };
        let window_input = window.input.schema();
        let name = |j: usize| quote(window_input.field(j).name());

        // The groups: one lump, and columns of the input.
        let mut groups: Vec<Source> = Vec::new();
        for g in &agg.group_expr {
            match source(g) {
                Some(src) => groups.push(src),
                None => return Ok(Transformed::no(plan)),
            }
        }
        if groups.iter().filter(|g| **g == Source::Lump).count() != 1 {
            return Ok(Transformed::no(plan));
        }
        // The chart's aggregates: a family's aggregate of a column, or count(*).
        let mut chart: Vec<(Family, Option<String>)> = Vec::new();
        for a in &agg.aggr_expr {
            let a = match a {
                Expr::Alias(x) => x.expr.as_ref(),
                e => e,
            };
            let Expr::AggregateFunction(f) = a else {
                return Ok(Transformed::no(plan));
            };
            let p = &f.params;
            if p.distinct || p.filter.is_some() || !p.order_by.is_empty() || p.args.len() != 1 {
                return Ok(Transformed::no(plan));
            }
            let Some(fam) = family(f.func.name()) else {
                return Ok(Transformed::no(plan));
            };
            let column = match source(&p.args[0]) {
                Some(Source::Input(j)) => Some(name(j)),
                Some(Source::Lump) => return Ok(Transformed::no(plan)),
                None if fam == Family::Count
                    && matches!(&p.args[0], Expr::Literal(v, _) if !v.is_null()) =>
                {
                    None
                }
                None => return Ok(Transformed::no(plan)),
            };
            chart.push((fam, column));
        }

        let Some(params) = lump_call(window) else {
            return Ok(Transformed::no(plan));
        };
        let lp = LumpPlan::new(window, params, config)?;
        let input = lp.input.clone();
        let registry = lp.registry;
        let other_groups: Vec<String> = groups
            .iter()
            .filter_map(|g| match g {
                Source::Input(j) => Some(name(*j)),
                Source::Lump => None,
            })
            .collect();
        // The groups of `fine`: the keys and the category as columns of `__input`, and the
        // chart's other groups. The names that `by` and `keep` use for the keys and the
        // category are aliases on the small table `fine`, not groups over every row.
        let mut fine_groups: Vec<String> = lp.keys.iter().map(|k| quote(k)).collect();
        fine_groups.push("\"__cat\"".into());
        for g in &other_groups {
            if !fine_groups.contains(g) {
                fine_groups.push(g.clone());
            }
        }
        let mut decision_groups: Vec<String> = lp.keys.iter().map(|k| quote(k)).collect();
        decision_groups.push("\"__cat\"".into());
        let mut aliases = String::new();
        for (g, col) in lp
            .key_groups
            .iter()
            .zip(lp.keys.iter().map(|k| quote(k)))
            .chain([(&lp.cat_group, "\"__cat\"".to_string())])
        {
            if !decision_groups.contains(g) {
                decision_groups.push(g.clone());
                if !fine_groups.contains(g) {
                    aliases.push_str(&format!(", {col} AS {g}"));
                }
            }
        }

        // Each distinct state is one column of `fine`, shared by the chart, `by`, and `keep`.
        let mut state_cols: Vec<(String, String)> = Vec::new();
        let dialect = lp.dialect.clone();
        let normalize = lp.options.sql_parser.enable_ident_normalization;
        let alias_of = |sql: String, cols: &mut Vec<(String, String)>| -> String {
            let sql = canonical(&dialect, &sql, normalize);
            if let Some((_, a)) = cols.iter().find(|(s, _)| *s == sql) {
                return a.clone();
            }
            let a = format!("__s{}", cols.len());
            cols.push((sql, a.clone()));
            a
        };
        let chart_cols: Vec<String> = chart
            .iter()
            .map(|(fam, col)| {
                alias_of(
                    format!("{}_state({})", fam.name(), col.as_deref().unwrap_or("")),
                    &mut state_cols,
                )
            })
            .collect();
        // Replaces the placeholders `"<prefix>i"` of `to_states` with the state columns.
        let bind = |text: &str,
                    prefix: &str,
                    sts: &[(Family, String)],
                    cols: &mut Vec<(String, String)>|
         -> (String, Vec<(Family, String)>) {
            let mut text = text.to_string();
            let mut bound = Vec::new();
            for (i, (fam, st)) in sts.iter().enumerate().rev() {
                let a = alias_of(st.clone(), cols);
                text = text.replace(&format!("\"{prefix}{i}\""), &format!("\"{a}\""));
                bound.push((*fam, a));
            }
            bound.reverse();
            (text, bound)
        };
        let part = if lp.keys.is_empty() {
            String::new()
        } else {
            format!(
                "PARTITION BY {}",
                lp.keys
                    .iter()
                    .map(|k| quote(k))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        // `by` over the dropped categories of each partition: each merge of a state column
        // as a window over the categories, filtered to the dropped ones. A window reads the
        // decisions once, where a grouped subquery and a join would read them twice.
        let other_window = |text: &str, cols: &[(Family, String)]| -> String {
            let mut text = text.to_string();
            for (fam, col) in cols {
                let call = format!("{}_merge(\"{col}\")", fam.name());
                text = text.replace(
                    &call,
                    &format!("{call} FILTER (WHERE NOT \"__keep\") OVER ({part})"),
                );
            }
            text
        };
        let finals = |prefix: &str| -> String {
            chart
                .iter()
                .enumerate()
                .map(|(k, (fam, _))| {
                    format!(
                        "{}_merge({prefix}.\"{}\") AS \"__a{k}\"",
                        fam.name(),
                        chart_cols[k]
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        let by_states = to_states(registry, &lp.dialect, &lp.by, "__sb");

        let sql = if other_groups.is_empty() {
            // One row per category: one aggregate of the input holds the decisions and the
            // states of the chart, and the chart merges them by the lump's fields. The input
            // is read once (twice when `by` has an aggregate without a state).
            let other = by_states.map(|(by, sts)| {
                let (by, bound) = bind(&by, "__sb", &sts, &mut state_cols);
                other_window(&by, &bound)
            });
            let states: Vec<String> = state_cols
                .iter()
                .map(|(sql, a)| format!(", {sql} AS \"{a}\""))
                .collect();
            let select_keys: String = lp
                .key_groups
                .iter()
                .zip(&lp.keys)
                .map(|(g, k)| format!("{g} AS {}, ", quote(k)))
                .collect();
            let groups: Vec<String> = lp
                .key_groups
                .iter()
                .cloned()
                .chain([lp.cat_group.clone()])
                .collect();
            let decided = format!(
                "SELECT {select_keys}{cat} AS \"__cat\", CAST(({by}) AS DOUBLE) AS \"__weight\", \
                 coalesce(({keep}), false) AS \"__keep\"{states} FROM {input} GROUP BY {groups}",
                cat = lp.cat_group,
                by = lp.by,
                keep = lp.keep,
                states = states.concat(),
                groups = groups.join(", "),
            );
            let decisions = match other {
                Some(other) => {
                    format!("SELECT *, CAST(({other}) AS DOUBLE) AS \"__other\" FROM decided")
                }
                None => lp.decisions.clone(),
            };
            format!(
                "WITH decided AS ({decided}), decisions AS ({decisions}) \
                 SELECT {}, {} FROM decisions d GROUP BY 1, 2, 3",
                lp.fields_sql("d.\"__cat\""),
                finals("d"),
            )
        } else {
            // Rows by category and the other groups: `fine` holds the states, the decisions
            // merge them for each category, and the chart merges them by the lump's fields and
            // the other groups. `fine` is read twice, by the decisions and by the chart.
            let keep_states = to_states(registry, &lp.dialect, &lp.keep, "__sk");
            let fused = match (by_states, keep_states) {
                (Some((by, by_sts)), Some((keep, keep_sts))) => {
                    let (by, by_bound) = bind(&by, "__sb", &by_sts, &mut state_cols);
                    let (keep, _) = bind(&keep, "__sk", &keep_sts, &mut state_cols);
                    Some((by, by_bound, keep))
                }
                _ => None,
            };
            let states: Vec<String> = state_cols
                .iter()
                .map(|(sql, a)| format!("{sql} AS \"{a}\""))
                .collect();
            let with = match fused {
                Some((by, by_bound, keep)) => {
                    // Each state of `by` merged for each category, for the Other group.
                    let mut merged: Vec<String> = Vec::new();
                    let mut m_cols: Vec<(Family, String)> = Vec::new();
                    let mut by_other = by.clone();
                    for (fam, a) in &by_bound {
                        let m = format!("__m{a}");
                        let col = format!(", {}_merge_state(\"{a}\") AS \"{m}\"", fam.name());
                        if !merged.contains(&col) {
                            merged.push(col);
                            m_cols.push((*fam, m.clone()));
                        }
                        by_other = by_other.replace(&format!("\"{a}\""), &format!("\"{m}\""));
                    }
                    format!(
                        "WITH fine AS (SELECT {fg}, {states} FROM {input} GROUP BY {fg}), \
                         named AS (SELECT *{aliases} FROM fine), \
                         decided AS (SELECT {dg}, CAST(({by}) AS DOUBLE) AS \"__weight\", coalesce(({keep}), false) AS \"__keep\"{merged} \
                                     FROM named GROUP BY {dg}), \
                         decisions AS (SELECT *, CAST(({other}) AS DOUBLE) AS \"__other\" FROM decided)",
                        fg = fine_groups.join(", "),
                        states = states.join(", "),
                        dg = decision_groups.join(", "),
                        merged = merged.concat(),
                        other = other_window(&by_other, &m_cols),
                    )
                }
                // `by` or `keep` has an aggregate without a state: the decisions read the input.
                None => format!(
                    "{}, fine AS (SELECT {fg}, {states} FROM {input} GROUP BY {fg})",
                    lp.with().trim_end(),
                    fg = fine_groups.join(", "),
                    states = states.join(", "),
                ),
            };
            let g_select: String = other_groups
                .iter()
                .enumerate()
                .map(|(j, g)| format!(", f.{g} AS \"__g{j}\""))
                .collect();
            let group_by: String = (1..=3 + other_groups.len())
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{with} SELECT {}{g_select}, {} FROM fine f JOIN decisions d ON {} GROUP BY {group_by}",
                lp.fields_sql("f.\"__cat\""),
                finals("f"),
                lp.on("f", "d", true),
            )
        };
        let generated = lp.plan(&sql)?;

        // The output of the aggregate, with its names and types.
        let out = generated.schema().clone();
        let col = |n: &str| {
            Expr::Column(
                out.columns_with_unqualified_name(n)
                    .into_iter()
                    .next()
                    .unwrap(),
            )
        };
        let mut exprs = Vec::new();
        let mut j = 0;
        for (i, g) in groups.iter().enumerate() {
            let (q, f) = agg.schema.qualified_field(i);
            let e = match g {
                Source::Lump => lp.struct_expr(&out)?,
                Source::Input(_) => {
                    j += 1;
                    Expr::Cast(Cast::new(
                        Box::new(col(&format!("__g{}", j - 1))),
                        f.data_type().clone(),
                    ))
                }
            };
            exprs.push(e.alias_qualified(q.cloned(), f.name()));
        }
        for k in 0..chart.len() {
            let (q, f) = agg.schema.qualified_field(groups.len() + k);
            let e = Expr::Cast(Cast::new(
                Box::new(col(&format!("__a{k}"))),
                f.data_type().clone(),
            ));
            exprs.push(e.alias_qualified(q.cloned(), f.name()));
        }
        Ok(Transformed::yes(
            LogicalPlanBuilder::from(generated)
                .project(exprs)?
                .build()?,
        ))
    }
}

/// The `lump` window function.
pub fn lump_udwf() -> WindowUDF {
    WindowUDF::new_from_impl(LumpWindow::default())
}
