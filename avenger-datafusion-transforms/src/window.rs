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

use crate::bin;
use crate::lump::{self, LumpCall};
use crate::timeunit;
use crate::utils::{PartitionValue, partition_values};

/// The transform of a placeholder window.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) enum Transform {
    /// `bin`, whose second argument is the struct of its options, and whether the call gives an
    /// extent.
    Bin {
        gives_extent: bool,
    },
    /// `timeunit`, whose second argument is the struct of its options, and whether the call
    /// gives a unit.
    TimeUnit {
        gives_unit: bool,
    },
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
        signature: Signature::variadic_any(Volatility::Immutable),
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
            Transform::Bin { .. } => bin::result_field(args.name(), input),
            Transform::TimeUnit { .. } => timeunit::window_field(args.name(), input, None),
            Transform::Lump(_) => Ok(Arc::new(Field::new(
                args.name(),
                DataType::Struct(lump::lump_fields(input.data_type())),
                true,
            ))),
        }
    }
}

/// The argument at `index` of the placeholder window `function`, such as the struct of its
/// options.
fn argument(function: &WindowFunction, index: usize) -> Result<Expr> {
    match function.params.args.get(index) {
        Some(arg) => Ok(arg.clone()),
        None => plan_err!("{} needs its options", function.fun.name()),
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
        /// The struct of the options.
        options: Expr,
        gives_extent: bool,
    },
    TimeUnit {
        value: Expr,
        /// The struct of the options.
        options: Expr,
        gives_unit: bool,
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
            // The window's own category and partition, which a caller may have rebased onto the
            // columns of an earlier window, such as a `timeunit` that the category reads.
            Transform::Lump(call) => {
                let mut call = call.as_ref().clone();
                call.category = value;
                call.partition = params.partition_by.clone();
                plan = lump::plan(plan, call, output)?;
                continue;
            }
            Transform::Bin { gives_extent } => PartitionedCall::Bin {
                value,
                options: argument(function, 1)?,
                gives_extent: *gives_extent,
            },
            Transform::TimeUnit { gives_unit } => PartitionedCall::TimeUnit {
                value,
                options: argument(function, 1)?,
                gives_unit: *gives_unit,
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
            PartitionedCall::Bin {
                value,
                options,
                gives_extent,
            } => bin::output(value, options, gives_extent, i)?,
            PartitionedCall::TimeUnit {
                value,
                options,
                gives_unit,
            } => timeunit::output(value, options, gives_unit, i),
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
