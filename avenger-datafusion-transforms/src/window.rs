//! The placeholder windows of `bin`, `timeunit`, and `lump`, and [`plan_windows`], which plans
//! them onto the input of a projection.
//!
//! `bin` and `timeunit` read a value of each window partition: the extent of its values, or the
//! unit that `timeunit` chooses for them. The calls over one partition share one aggregate of the
//! partitions, joined back to their rows, or without a partition, scalar subqueries.

use std::any::Any;
use std::sync::Arc;

use datafusion::arrow::datatypes::{DataType, Field, FieldRef};
use datafusion::common::{Column, Result, plan_err};
use datafusion::logical_expr::expr::WindowFunction;
use datafusion::logical_expr::function::{PartitionEvaluatorArgs, WindowUDFFieldArgs};
use datafusion::logical_expr::{
    Expr, LogicalPlan, LogicalPlanBuilder, PartitionEvaluator, Signature, Volatility,
    WindowFunctionDefinition, WindowUDF, WindowUDFImpl,
};

use crate::bin::{self, BinOptions};
use crate::lump::{self, LumpCall};
use crate::timeunit::{self, TimeUnitOptions};
use crate::utils::{PartitionValue, partition_values};

/// The transform of a placeholder window.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) enum Transform {
    Bin(BinOptions),
    TimeUnit(TimeUnitOptions),
    Lump(Box<LumpCall>),
}

/// A placeholder window function, which [`plan_windows`] plans. Its name tells apart the calls
/// of one value with different options, as a window node names its columns by their calls.
#[derive(Debug, PartialEq, Eq, Hash)]
struct Placeholder {
    name: String,
    transform: Transform,
    signature: Signature,
}

/// The placeholder window function `name` of `transform`, which takes the value of the call.
pub(crate) fn placeholder(name: String, transform: Transform) -> Arc<WindowUDF> {
    Arc::new(WindowUDF::new_from_impl(Placeholder {
        name,
        transform,
        signature: Signature::any(1, Volatility::Immutable),
    }))
}

impl WindowUDFImpl for Placeholder {
    fn name(&self) -> &str {
        &self.name
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn partition_evaluator(
        &self,
        _: PartitionEvaluatorArgs,
    ) -> Result<Box<dyn PartitionEvaluator>> {
        plan_err!(
            "{} runs only in a projection whose windows plan through plan_windows",
            self.name
        )
    }
    fn field(&self, args: WindowUDFFieldArgs) -> Result<FieldRef> {
        let Some(input) = args.input_fields().first() else {
            return plan_err!("{} needs a value", self.name);
        };
        match &self.transform {
            Transform::Bin(_) => bin::result_field(args.name(), input),
            Transform::TimeUnit(options) => timeunit::window_field(args.name(), input, options),
            Transform::Lump(_) => Ok(Arc::new(Field::new(
                args.name(),
                DataType::Struct(lump::lump_fields(input.data_type())),
                true,
            ))),
        }
    }
}

/// The transform of the placeholder window `expr`, and its window function.
fn placeholder_of(expr: &Expr) -> Option<(&Transform, &WindowFunction)> {
    let Expr::WindowFunction(function) = expr else {
        return None;
    };
    let WindowFunctionDefinition::WindowUDF(udwf) = &function.fun else {
        return None;
    };
    let inner: &dyn Any = udwf.inner().as_ref();
    let placeholder = inner.downcast_ref::<Placeholder>()?;
    Some((&placeholder.transform, function))
}

/// The call of `bin` or `timeunit` that a value of each window partition serves.
pub(crate) enum PartitionedCall {
    Bin {
        value: Expr,
        options: BinOptions,
    },
    TimeUnit {
        value: Expr,
        options: TimeUnitOptions,
    },
}

/// The result of a call of [`PartitionedCall`]: an expression of the row, or an expression of
/// the row and of a value of the row's partition.
pub(crate) enum Output {
    Row(Expr),
    Partition(PartitionValue, Box<dyn FnOnce(Expr) -> Expr>),
}

/// The calls of `bin` and `timeunit` over one partition, with their output columns.
struct Group {
    partition: Vec<Expr>,
    calls: Vec<(PartitionedCall, Column)>,
}

/// `input` with the column of each placeholder of `windows`, and the other windows of
/// `windows`, for a window node over the result. The column of a placeholder is named after the
/// window, as a window node names its columns, so a projection over both reads each window as
/// the column of its schema name.
pub fn plan_windows(input: LogicalPlan, windows: Vec<Expr>) -> Result<(LogicalPlan, Vec<Expr>)> {
    let mut plan = input;
    let mut rest = Vec::new();
    // The calls of `bin` and `timeunit` by their partition, which one aggregate serves.
    let mut groups: Vec<Group> = Vec::new();
    for window in windows {
        let Some((transform, function)) = placeholder_of(&window) else {
            rest.push(window);
            continue;
        };
        let output = Column::from_name(window.schema_name().to_string());
        let params = &function.params;
        let whole = params.window_frame.start_bound.is_unbounded()
            && params.window_frame.end_bound.is_unbounded();
        if !params.order_by.is_empty() || !whole {
            return plan_err!(
                "the window of {} is its whole partition, so it takes no ORDER BY or frame",
                function.fun.name()
            );
        }
        let value = function.params.args[0].clone();
        let call = match transform {
            Transform::Lump(call) => {
                plan = lump::plan(plan, call.as_ref().clone(), output)?;
                continue;
            }
            Transform::Bin(options) => PartitionedCall::Bin {
                value,
                options: options.clone(),
            },
            Transform::TimeUnit(options) => PartitionedCall::TimeUnit {
                value,
                options: options.clone(),
            },
        };
        let partition = &function.params.partition_by;
        match groups
            .iter_mut()
            .find(|group| &group.partition == partition)
        {
            Some(group) => group.calls.push((call, output)),
            None => groups.push(Group {
                partition: partition.clone(),
                calls: vec![(call, output)],
            }),
        }
    }
    for group in groups {
        plan = plan_partitioned(plan, &group.partition, group.calls)?;
    }
    Ok((plan, rest))
}

/// `input` with the column of each of `calls` over the window partitions of `partition`, after
/// the columns of `input`.
pub(crate) fn plan_partitioned(
    input: LogicalPlan,
    partition: &[Expr],
    calls: Vec<(PartitionedCall, Column)>,
) -> Result<LogicalPlan> {
    let columns: Vec<Expr> = (input.schema().columns().into_iter())
        .map(Expr::Column)
        .collect();
    // The partition as columns: an expression that is not a column is projected first.
    let mut keys = Vec::with_capacity(partition.len());
    let mut projected = Vec::new();
    for (k, expr) in partition.iter().enumerate() {
        match expr {
            Expr::Column(column) => keys.push(column.clone()),
            expr => {
                let name = format!("__partition_{k}");
                projected.push(expr.clone().alias(&name));
                keys.push(Column::from_name(name));
            }
        }
    }
    let input = match projected.is_empty() {
        true => input,
        false => LogicalPlanBuilder::from(input)
            .project(columns.iter().cloned().chain(projected))?
            .build()?,
    };

    let mut values = Vec::new();
    let mut outputs = Vec::with_capacity(calls.len());
    for (i, (call, output)) in calls.into_iter().enumerate() {
        let result = match call {
            PartitionedCall::Bin { value, options } => bin::output(value, options, i)?,
            PartitionedCall::TimeUnit { value, options } => timeunit::output(value, options, i),
        };
        match result {
            Output::Row(expr) => outputs.push((Ok(expr), output)),
            Output::Partition(value, finish) => {
                values.push(value);
                outputs.push((Err(finish), output));
            }
        }
    }
    let (builder, mut of_partitions) = partition_values(input, &keys, values, "partition")?;
    of_partitions.reverse();
    let mut exprs = columns;
    for (expr, output) in outputs {
        let expr = match expr {
            Ok(expr) => expr,
            Err(finish) => finish(of_partitions.pop().expect("a value of each partition")),
        };
        exprs.push(expr.alias_qualified(output.relation, output.name));
    }
    builder.project(exprs)?.build()
}
