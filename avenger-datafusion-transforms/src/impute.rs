//! `impute`: a table function that adds the rows that each group lacks.
//!
//! ```sql
//! SELECT * FROM impute((SELECT month, region, sum(units) AS units FROM sales GROUP BY 1, 2),
//!                      month, [region], units, value => 0, flag => imputed)
//! ```
//!
//! The first argument is the input: a subquery, a CTE, or a table. Then come the key column,
//! the group columns (a list, `[]`, or one column without brackets), and the value column. The named arguments are `value`, `method`
//! (`'avg'`, `'median'`, `'min'`, or `'max'`), `frame => [lo, hi]`, `keys => [...]`,
//! `extra_keys => [...]`, `fill_gaps`, `partition_by`, and `flag => name`.
//!
//! For each group (the distinct combinations of the group columns) and each key value, the
//! function adds a row when the group has no row with that key. The key values are `keys`,
//! or the distinct non-null keys of the input, plus `extra_keys`. Added rows have the key,
//! the group columns, and the imputed value; their other columns are NULL. Existing rows
//! stay as they are. `flag` adds a boolean column that is true in added rows.
//!
//! The imputed value is the constant `value` (`value => NULL` for NULL), or DataFusion's
//! aggregate function `method` of the values of the group, or of the rows of `frame` around
//! the added row in key order. The value column takes the type of `COALESCE` of the column
//! and the imputed value: `avg` of a BIGINT column makes it a DOUBLE, and so does
//! `value => 0.5`. A `value` that the column's type holds keeps the type.
//!
//! `fill_gaps` adds the gaps of the key sequence to the key values: the bins, time units,
//! dates, or numbers between the keys that no row has. Its value is a sequence
//! `{start, end, step}`, or a shorthand:
//!
//! - `true` for the struct of `bin` (`{start, end}`, whose width is the step) and of
//!   `timeunit` (`{start, end, unit}`, whose unit and calendar are the step).
//! - A unit, such as `'month'`, for a DATE or a timestamp key. Weeks start on Monday, and a
//!   fiscal unit needs a key from `timeunit`, which carries the fiscal year start.
//! - A struct with any of `start`, `end`, and `step`; a field that it leaves out comes from
//!   the keys. `bin_params(...)` is a sequence of the bins of one mesh. A number key needs a
//!   `step`.
//!
//! The range is half-open, `[start, end)`. Keys outside it stay key values, and impute fills no
//! gaps outside it. The intervals of a partition must have one width and one alignment, and the
//! time units one unit.
//!
//! `partition_by` names columns whose values impute separately: each partition has its own key
//! values, groups, and gaps.
//!
//! Added rows follow the input rows of their partition, by group in the order in which the
//! groups first occur, then in key order.
//!
//! `impute` is a [table function](crate::table_function): [`Impute`] binds its arguments, and
//! [`ImputeTransform`] adds the rows of each partition. It reads its input once, into memory.
//! The same result from SQL joins would read the input once for the keys, once for the
//! groups, and once for the rows: DataFusion does not share a subplan between readers, and the
//! input of `impute` is usually an aggregate.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use datafusion::arrow::array::{
    Array, ArrayRef, BooleanArray, RecordBatch, RecordBatchOptions, UInt32Array, new_null_array,
};
use datafusion::arrow::compute::{cast, concat, take};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use datafusion::arrow::row::{RowConverter, SortField};
use datafusion::common::{Column, DFSchema, DFSchemaRef, ExprSchema, Result, ScalarValue};
use datafusion::functions_aggregate::average::avg_udaf;
use datafusion::functions_aggregate::median::median_udaf;
use datafusion::functions_aggregate::min_max::{max_udaf, min_udaf};
use datafusion::logical_expr::AggregateUDF;
use datafusion::logical_expr::type_coercion::binary::{
    try_type_union_resolution, type_union_resolution,
};
use datafusion::logical_expr::type_coercion::functions::fields_with_udf;
use datafusion::physical_expr::PhysicalExpr;
use datafusion::physical_expr::aggregate::AggregateExprBuilder;
use datafusion::physical_expr::expressions::Column as PhysicalColumn;
use datafusion::sql::sqlparser::ast::{Expr as SqlExpr, Spanned, Value};
use datafusion::sql::sqlparser::tokenizer::Span as SqlSpan;

use crate::table_function::{
    Call, PartitionTransform, TableFunction, TableTransform, cast_exact, constant, err_at,
    row_bytes,
};

mod gaps;
pub use gaps::{GapKind, Gaps, Step};

// ---------------------------------------------------------------------------------------
// The fill
// ---------------------------------------------------------------------------------------

/// A statistic of the existing values of a group: DataFusion's aggregate function of the
/// name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Hash)]
pub enum Stat {
    Avg,
    Median,
    Min,
    Max,
}

impl Stat {
    pub fn name(self) -> &'static str {
        match self {
            Stat::Avg => "avg",
            Stat::Median => "median",
            Stat::Min => "min",
            Stat::Max => "max",
        }
    }

    fn udaf(self) -> Arc<AggregateUDF> {
        match self {
            Stat::Avg => avg_udaf(),
            Stat::Median => median_udaf(),
            Stat::Min => min_udaf(),
            Stat::Max => max_udaf(),
        }
    }

    /// The type of the values that the aggregate function reads, after DataFusion's coercion
    /// of the value column, and the type of its result.
    fn types(self, val: &Field) -> Result<(DataType, DataType)> {
        let udaf = self.udaf();
        let arg = fields_with_udf(&[Arc::new(val.clone())], udaf.as_ref())?.remove(0);
        let result = udaf.return_field(std::slice::from_ref(&arg))?;
        Ok((arg.data_type().clone(), result.data_type().clone()))
    }
}

/// The value of added rows.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Hash)]
pub enum Fill {
    /// `value => v`, of the type of the value column in the result.
    Value(ScalarValue),
    /// `method => 'avg'` and the others, over the whole group or over `frame => [lo, hi]`
    /// (None is an open end).
    Stat {
        stat: Stat,
        frame: Option<(Option<i64>, Option<i64>)>,
    },
}

impl Fill {
    /// The type of the value column in the result: the type of `COALESCE` of the column and
    /// the fill.
    fn value_type(&self, val: &Field) -> Result<DataType> {
        match self {
            Fill::Value(v) => Ok(v.data_type()),
            Fill::Stat { stat, .. } => {
                let (_, result) = stat.types(val)?;
                Ok(try_type_union_resolution(&[val.data_type().clone(), result])?.remove(0))
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// The transform
// ---------------------------------------------------------------------------------------

/// The transform of a call of `impute`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImputeTransform {
    pub key: Column,
    pub groupby: Vec<Column>,
    pub val: Column,
    pub fill: Fill,
    /// `keys => [...]`, of the type of the key.
    pub keys: Option<Vec<ScalarValue>>,
    /// `extra_keys => [...]`, of the type of the key.
    pub extra_keys: Vec<ScalarValue>,
    /// `fill_gaps => ...`.
    pub gaps: Option<Gaps>,
    pub flag: Option<String>,
}

/// The input columns, then the flag. Added rows are NULL in the columns other than the
/// key, the groups, and the partition columns, so those columns become nullable, and the value column takes the type
/// of [`Fill::value_type`].
fn output_schema(
    input: &DFSchema,
    key: &Column,
    groupby: &[Column],
    (val, fill): (&Column, &Fill),
    flag: Option<&str>,
) -> Result<DFSchemaRef> {
    let key_i = input.index_of_column(key)?;
    let val_i = input.index_of_column(val)?;
    let val_type = fill.value_type(input.field(val_i))?;
    let group_i = groupby
        .iter()
        .map(|c| input.index_of_column(c))
        .collect::<Result<HashSet<_>>>()?;
    let mut fields: Vec<_> = input
        .iter()
        .enumerate()
        .map(|(i, (q, f))| {
            let keep = i == key_i || group_i.contains(&i);
            let f = if keep {
                f.clone()
            } else if i == val_i {
                Arc::new(
                    f.as_ref()
                        .clone()
                        .with_data_type(val_type.clone())
                        .with_nullable(true),
                )
            } else {
                Arc::new(f.as_ref().clone().with_nullable(true))
            };
            (q.cloned(), f)
        })
        .collect();
    if let Some(flag) = flag {
        fields.push((None, Arc::new(Field::new(flag, DataType::Boolean, false))));
    }
    Ok(Arc::new(DFSchema::new_with_metadata(
        fields,
        input.metadata().clone(),
    )?))
}

impl TableTransform for ImputeTransform {
    fn name(&self) -> &'static str {
        "Impute"
    }
    fn columns(&self) -> Vec<Column> {
        std::iter::once(self.key.clone())
            .chain(self.groupby.iter().cloned())
            .chain([self.val.clone()])
            .collect()
    }
    fn with_columns(&self, columns: Vec<Column>) -> Result<Arc<dyn TableTransform>> {
        let g = self.groupby.len();
        Ok(Arc::new(Self {
            key: columns[0].clone(),
            groupby: columns[1..1 + g].to_vec(),
            val: columns[1 + g].clone(),
            ..self.clone()
        }))
    }
    fn schema(&self, input: &DFSchema, partition_by: &[Column]) -> Result<DFSchemaRef> {
        let carried: Vec<Column> = self.groupby.iter().chain(partition_by).cloned().collect();
        output_schema(
            input,
            &self.key,
            &carried,
            (&self.val, &self.fill),
            self.flag.as_deref(),
        )
    }
    fn partition_transform(&self, input: &DFSchema) -> Result<Arc<dyn PartitionTransform>> {
        Ok(Arc::new(Spec {
            key: input.index_of_column(&self.key)?,
            groups: self
                .groupby
                .iter()
                .map(|c| input.index_of_column(c))
                .collect::<Result<Vec<_>>>()?,
            val: input.index_of_column(&self.val)?,
            fill: self.fill.clone(),
            keys: self.keys.clone(),
            extra_keys: self.extra_keys.clone(),
            gaps: self.gaps.clone(),
        }))
    }
    fn fmt_for_explain(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let groups: Vec<_> = self.groupby.iter().map(|c| c.to_string()).collect();
        write!(
            f,
            "key={}, groupby=[{}], val={}, fill={:?}",
            self.key,
            groups.join(", "),
            self.val,
            self.fill
        )?;
        if let Some(keys) = &self.keys {
            write!(f, ", keys={}", keys.len())?;
        }
        if !self.extra_keys.is_empty() {
            write!(f, ", extra_keys={}", self.extra_keys.len())?;
        }
        if let Some(gaps) = &self.gaps {
            write!(f, ", fill_gaps={gaps:?}")?;
        }
        if let Some(flag) = &self.flag {
            write!(f, ", flag={flag}")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------
// The arguments
// ---------------------------------------------------------------------------------------

/// The table function `impute`.
#[derive(Debug)]
pub struct Impute;

impl TableFunction for Impute {
    fn name(&self) -> &'static str {
        "impute"
    }
    fn positional(&self) -> &'static [&'static str] {
        &["the key", "the groups", "the value column"]
    }
    fn named(&self) -> &'static [&'static str] {
        &[
            "value",
            "method",
            "frame",
            "keys",
            "extra_keys",
            "fill_gaps",
            "flag",
        ]
    }
    fn plan(&self, call: Call<'_>) -> Result<Arc<dyn TableTransform>> {
        plan_impute(call)
    }
}

fn plan_impute(mut call: Call<'_>) -> Result<Arc<dyn TableTransform>> {
    let schema = call.schema();
    let mut positional = std::mem::take(&mut call.positional).into_iter();
    let key = call.column("key", positional.next().expect("the key"))?;
    let mut groupby = Vec::new();
    for (c, span) in call.columns("group", positional.next().expect("the groups"))? {
        if c == key {
            return Err(err_at(
                span,
                format!("the key {c} of impute is also a group column"),
            ));
        }
        if groupby.contains(&c) {
            return Err(err_at(
                span,
                format!("the group column {c} of impute is given twice"),
            ));
        }
        groupby.push(c);
    }
    let val_expr = positional.next().expect("the value column");
    let val_span = val_expr.span();
    let val = call.column("value column", val_expr)?;
    if val == key || groupby.contains(&val) {
        return Err(err_at(
            val_span,
            format!("the value column {val} of impute is also the key or a group column"),
        ));
    }
    let key_type = schema.field_from_column(&key)?.data_type().clone();
    let val_type = schema.field_from_column(&val)?.data_type().clone();
    let call_span = call.span;

    // The named arguments.
    let mut value = None;
    let mut method: Option<(String, SqlSpan)> = None;
    let mut frame = None;
    let mut keys = None;
    let mut extra_keys = Vec::new();
    let mut gaps: Option<(Gaps, SqlSpan)> = None;
    let mut flag = None;
    for (name, e) in std::mem::take(&mut call.named) {
        let span = e.span();
        match name.as_str() {
            "value" => {
                let v = call.constant("`value`", e)?;
                // A value that the column's type holds keeps the type. A number that a numeric
                // column's type does not hold widens the column, as `COALESCE` of the column
                // and the value would.
                let cast = cast_exact(&v, &val_type).or_else(|| {
                    if !(val_type.is_numeric() && v.data_type().is_numeric()) {
                        return None;
                    }
                    let to = type_union_resolution(&[val_type.clone(), v.data_type()])?;
                    cast_exact(&v, &to)
                });
                let Some(cast) = cast else {
                    let shown = if v.data_type().is_null() || v.try_as_str().is_none() {
                        v.to_string()
                    } else {
                        format!("'{v}'")
                    };
                    return Err(err_at(
                        span,
                        format!("`value` {shown} does not coerce to the type of {val}, {val_type}"),
                    ));
                };
                value = Some((cast, span));
            }
            "method" => {
                let SqlExpr::Value(v) = &e else {
                    return Err(err_at(
                        span,
                        "`method` must be 'avg', 'median', 'min', or 'max'",
                    ));
                };
                let Value::SingleQuotedString(s) = &v.value else {
                    return Err(err_at(
                        span,
                        format!("`method` must be 'avg', 'median', 'min', or 'max'; found {e}"),
                    ));
                };
                method = Some((s.clone(), span));
            }
            "frame" => {
                let SqlExpr::Array(a) = e else {
                    return Err(err_at(
                        span,
                        "`frame` must be a list of two bounds, such as [-2, 2]",
                    ));
                };
                if a.elem.len() != 2 {
                    return Err(err_at(
                        span,
                        "`frame` must be a list of two bounds, such as [-2, 2]",
                    ));
                }
                let mut bounds = [None, None];
                for (i, b) in a.elem.into_iter().enumerate() {
                    let bspan = b.span();
                    let expr = call.ctx.sql_to_expr(b, &DFSchema::empty()).map_err(|_| {
                        err_at(bspan, "a bound of `frame` must be an integer or NULL")
                    })?;
                    bounds[i] = match constant(&expr) {
                        Ok(v) if v.is_null() => None,
                        Ok(v) if v.data_type().is_integer() => match v.cast_to(&DataType::Int64) {
                            Ok(ScalarValue::Int64(Some(b))) => Some(b),
                            _ => {
                                return Err(err_at(
                                    bspan,
                                    "a bound of `frame` must fit in a BIGINT",
                                ));
                            }
                        },
                        _ => {
                            return Err(err_at(
                                bspan,
                                "a bound of `frame` must be an integer or NULL",
                            ));
                        }
                    };
                }
                if let [Some(lo), Some(hi)] = bounds
                    && lo > hi
                {
                    return Err(err_at(
                        span,
                        format!("the start of `frame`, {lo}, is after its end, {hi}"),
                    ));
                }
                frame = Some(((bounds[0], bounds[1]), span));
            }
            "keys" => keys = Some(call.constant_list("`keys`", e, &key_type, "the key")?),
            "extra_keys" => {
                extra_keys = call.constant_list("`extra_keys`", e, &key_type, "the key")?
            }
            "fill_gaps" => {
                let v = call.constant("`fill_gaps`", e)?;
                if let Some(g) = Gaps::plan(&v, &key_type).map_err(|m| err_at(span, m))? {
                    gaps = Some((g, span));
                }
            }
            "flag" => {
                let SqlExpr::Identifier(id) = e else {
                    return Err(err_at(
                        span,
                        "`flag` must be a column name, such as `flag => imputed`",
                    ));
                };
                let name = call.ctx.normalize_ident(id);
                if schema.fields().iter().any(|f| f.name() == &name) {
                    return Err(err_at(
                        span,
                        format!("the flag column `{name}` is already a column of the input"),
                    ));
                }
                flag = Some(name);
            }
            _ => unreachable!("checked above"),
        }
    }

    if let (Some((_, span)), Some(_)) = (&gaps, &keys) {
        return Err(err_at(
            *span,
            "`fill_gaps` adds to the key values of the rows, which `keys` replaces; give one of them",
        ));
    }
    let gaps = gaps.map(|(g, _)| g);
    let fill = match (value, method) {
        (Some(_), Some((_, span))) => {
            return Err(err_at(
                span,
                "impute needs exactly one of `value` and `method`; found both",
            ));
        }
        (None, None) => {
            return Err(err_at(
                call_span,
                "impute needs exactly one of `value` and `method`; found neither",
            ));
        }
        (Some((v, _)), None) => {
            if let Some((_, span)) = frame {
                return Err(err_at(
                    span,
                    "`frame` needs a method of 'avg', 'median', 'min', or 'max'; found `value`",
                ));
            }
            Fill::Value(v)
        }
        (None, Some((m, span))) => {
            let stat = match m.as_str() {
                "avg" => Stat::Avg,
                "median" => Stat::Median,
                "min" => Stat::Min,
                "max" => Stat::Max,
                _ => {
                    return Err(err_at(
                        span,
                        format!("`method` must be 'avg', 'median', 'min', or 'max'; found '{m}'"),
                    ));
                }
            };
            let fill = Fill::Stat {
                stat,
                frame: frame.map(|(f, _)| f),
            };
            if let Err(e) = fill.value_type(schema.field_from_column(&val)?) {
                let e = e.strip_backtrace();
                return Err(err_at(
                    span,
                    format!(
                        "`method => '{m}'` does not apply to {val}, {val_type}: {}",
                        e.lines().next().unwrap_or("")
                    ),
                ));
            }
            fill
        }
    };
    Ok(Arc::new(ImputeTransform {
        key,
        groupby,
        val,
        fill,
        keys,
        extra_keys,
        gaps,
        flag,
    }))
}

// ---------------------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------------------

/// What `impute` imputes in each partition: the columns, by index, and the arguments.
#[derive(Debug, Clone)]
struct Spec {
    key: usize,
    groups: Vec<usize>,
    val: usize,
    fill: Fill,
    keys: Option<Vec<ScalarValue>>,
    extra_keys: Vec<ScalarValue>,
    gaps: Option<Gaps>,
}

impl PartitionTransform for Spec {
    fn transform(
        &self,
        input: &RecordBatch,
        partitions: &[usize],
        schema: SchemaRef,
    ) -> Result<RecordBatch> {
        // The partition columns are constant within a partition, so they group with the
        // groups, and added rows take them from a row of their group.
        let groups: Vec<usize> = self.groups.iter().chain(partitions).copied().collect();
        let key_field = input.schema().field(self.key).clone();
        impute_batch(input, self, &groups, &key_field, schema)
    }
}

fn key_values(
    key: &ArrayRef,
    keys: Option<&[ScalarValue]>,
    extra: &[ScalarValue],
    gaps: Option<&Gaps>,
    key_field: &Field,
) -> Result<ArrayRef> {
    let conv = RowConverter::new(vec![SortField::new(key.data_type().clone())])?;
    let distinct = |a: &ArrayRef, skip: &HashSet<Vec<u8>>| -> Result<Vec<u32>> {
        let mut seen = skip.clone();
        let bytes = row_bytes(&conv, std::slice::from_ref(a))?;
        Ok((0..a.len())
            .filter(|&i| a.is_valid(i) && seen.insert(bytes[i].clone()))
            .map(|i| i as u32)
            .collect())
    };
    // Sorted in the order of the row format, which every key type has, structs too.
    let sorted = |a: ArrayRef| -> Result<ArrayRef> {
        let rows = conv.convert_columns(std::slice::from_ref(&a))?;
        let mut order: Vec<u32> = (0..a.len() as u32).collect();
        order.sort_by(|x, y| rows.row(*x as usize).cmp(&rows.row(*y as usize)));
        Ok(take(&a, &UInt32Array::from(order), None)?)
    };
    let to_array = |v: &[ScalarValue]| -> Result<ArrayRef> {
        if v.is_empty() {
            Ok(new_null_array(key.data_type(), 0))
        } else {
            ScalarValue::iter_to_array(v.iter().cloned())
        }
    };
    let extra = to_array(extra)?;
    match keys {
        Some(listed) => {
            let listed = to_array(listed)?;
            let listed_bytes: HashSet<_> = row_bytes(&conv, std::slice::from_ref(&listed))?
                .into_iter()
                .collect();
            let more = take(
                &extra,
                &UInt32Array::from(distinct(&extra, &listed_bytes)?),
                None,
            )?;
            Ok(concat(&[listed.as_ref(), sorted(more)?.as_ref()])?)
        }
        None => {
            let all = concat(&[key.as_ref(), extra.as_ref()])?;
            let observed = take(
                &all,
                &UInt32Array::from(distinct(&all, &HashSet::new())?),
                None,
            )?;
            let all = match gaps {
                Some(g) => concat(&[observed.as_ref(), g.keys(&observed, key_field)?.as_ref()])?,
                None => observed,
            };
            sorted(take(
                &all,
                &UInt32Array::from(distinct(&all, &HashSet::new())?),
                None,
            )?)
        }
    }
}

fn impute_batch(
    input: &RecordBatch,
    spec: &Spec,
    groups: &[usize],
    key_field: &Field,
    out_schema: SchemaRef,
) -> Result<RecordBatch> {
    let (key, val, fill) = (spec.key, spec.val, &spec.fill);
    let n = input.num_rows();
    let key_col = input.column(key);
    let domain = key_values(
        key_col,
        spec.keys.as_deref(),
        &spec.extra_keys,
        spec.gaps.as_ref(),
        key_field,
    )?;
    let key_conv = RowConverter::new(vec![SortField::new(key_col.data_type().clone())])?;
    let domain_index: HashMap<Vec<u8>, usize> =
        row_bytes(&key_conv, std::slice::from_ref(&domain))?
            .into_iter()
            .enumerate()
            .map(|(i, b)| (b, i))
            .collect();
    let row_key: Vec<Option<usize>> = row_bytes(&key_conv, std::slice::from_ref(key_col))?
        .iter()
        .enumerate()
        .map(|(i, b)| {
            key_col
                .is_valid(i)
                .then(|| domain_index.get(b).copied())
                .flatten()
        })
        .collect();

    // Groups in the order of their first row.
    let group_cols: Vec<ArrayRef> = groups.iter().map(|&i| input.column(i).clone()).collect();
    let row_group: Vec<usize>;
    let mut reps: Vec<usize> = Vec::new();
    if group_cols.is_empty() {
        row_group = vec![0; n];
        if n > 0 {
            reps.push(0);
        }
    } else {
        let fields = group_cols
            .iter()
            .map(|c| SortField::new(c.data_type().clone()))
            .collect();
        let conv = RowConverter::new(fields)?;
        let mut index: HashMap<Vec<u8>, usize> = HashMap::new();
        row_group = row_bytes(&conv, &group_cols)?
            .into_iter()
            .enumerate()
            .map(|(i, b)| {
                *index.entry(b).or_insert_with(|| {
                    reps.push(i);
                    reps.len() - 1
                })
            })
            .collect();
    }

    // The rows of each group at each key value.
    let mut cells: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for i in 0..n {
        if let Some(d) = row_key[i] {
            cells.entry((row_group[i], d)).or_default().push(i);
        }
    }

    // The added rows: each group, each missing key value, in key order.
    let mut added: Vec<(usize, usize)> = Vec::new();
    for g in 0..reps.len() {
        for d in 0..domain.len() {
            if !cells.contains_key(&(g, d)) {
                added.push((g, d));
            }
        }
    }
    let m = added.len();

    let val_col = input.column(val);
    let val_type = out_schema.field(val).data_type().clone();
    let imputed: ArrayRef = match fill {
        Fill::Value(v) => v.to_array_of_size(m)?,
        Fill::Stat { stat, frame } => {
            // DataFusion's aggregate function over the rows of a group or of a frame.
            let (arg_type, _) = stat.types(input.schema().field(val))?;
            let values = cast(val_col, &arg_type)?;
            let agg = AggregateExprBuilder::new(
                stat.udaf(),
                vec![Arc::new(PhysicalColumn::new("v", 0)) as Arc<dyn PhysicalExpr>],
            )
            .schema(Arc::new(Schema::new(vec![Field::new("v", arg_type, true)])))
            .alias(stat.name())
            .build()?;
            let of = |rows: Vec<u32>| -> Result<ScalarValue> {
                let mut acc = agg.create_accumulator()?;
                acc.update_batch(&[take(&values, &UInt32Array::from(rows), None)?])?;
                acc.evaluate()
            };
            let results: Vec<ScalarValue> = match frame {
                // A frame of the whole group is no frame.
                None | Some((None, None)) => {
                    let mut by_group: Vec<Vec<u32>> = vec![Vec::new(); reps.len()];
                    for (i, g) in row_group.iter().enumerate() {
                        by_group[*g].push(i as u32);
                    }
                    let per_group = by_group.into_iter().map(of).collect::<Result<Vec<_>>>()?;
                    added.iter().map(|&(g, _)| per_group[g].clone()).collect()
                }
                Some((lo, hi)) => {
                    // The positions of each group after imputation, in key order: the
                    // existing rows of each key value (in input order), or one added row.
                    let mut out = Vec::with_capacity(m);
                    for g in 0..reps.len() {
                        let mut slots: Vec<Option<usize>> = Vec::new();
                        for d in 0..domain.len() {
                            match cells.get(&(g, d)) {
                                Some(rows) => slots.extend(rows.iter().map(|&r| Some(r))),
                                None => slots.push(None),
                            }
                        }
                        let len = slots.len() as i64;
                        for (p, slot) in slots.iter().enumerate() {
                            if slot.is_some() {
                                continue;
                            }
                            let p = p as i64;
                            let start = lo.map_or(0, |lo| p.saturating_add(lo).max(0));
                            let end = hi.map_or(len - 1, |hi| p.saturating_add(hi).min(len - 1));
                            let rows = (start..=end.max(start - 1))
                                .filter_map(|q| slots[q as usize].map(|r| r as u32))
                                .collect();
                            out.push(of(rows)?);
                        }
                    }
                    out
                }
            };
            if results.is_empty() {
                new_null_array(&val_type, 0)
            } else {
                cast(&ScalarValue::iter_to_array(results)?, &val_type)?
            }
        }
    };

    let added_keys = UInt32Array::from(added.iter().map(|&(_, d)| d as u32).collect::<Vec<_>>());
    let added_reps = UInt32Array::from(
        added
            .iter()
            .map(|&(g, _)| reps[g] as u32)
            .collect::<Vec<_>>(),
    );
    let mut columns: Vec<ArrayRef> = Vec::with_capacity(out_schema.fields().len());
    for (i, col) in input.columns().iter().enumerate() {
        let tail: ArrayRef = if i == key {
            take(&domain, &added_keys, None)?
        } else if groups.contains(&i) {
            take(col, &added_reps, None)?
        } else if i == val {
            imputed.clone()
        } else {
            new_null_array(col.data_type(), m)
        };
        let col = if i == val {
            cast(col, &val_type)?
        } else {
            col.clone()
        };
        columns.push(concat(&[col.as_ref(), tail.as_ref()])?);
    }
    if out_schema.fields().len() > input.num_columns() {
        let flags: Vec<bool> = (0..n + m).map(|i| i >= n).collect();
        columns.push(Arc::new(BooleanArray::from(flags)));
    }
    Ok(RecordBatch::try_new_with_options(
        out_schema,
        columns,
        &RecordBatchOptions::new().with_row_count(Some(n + m)),
    )?)
}
