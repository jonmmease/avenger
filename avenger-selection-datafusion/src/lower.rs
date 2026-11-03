//! The lowering of a `selected` call to one row test over the rows of its input, for a bound
//! state: avenger-selection's prepared `selected` test, which a renderer runs alike.

use std::{any::Any, sync::Arc};

use avenger_selection::{SelectedOptions, SelectionState};
use datafusion::{
    arrow::{array::AsArray, compute::cast, datatypes::DataType},
    common::{plan_err, Column, DFSchema, Result, ScalarValue},
    logical_expr::{lit, simplify::SimplifyContext, Expr, ScalarUDF},
    optimizer::simplify_expressions::ExprSimplifier,
};

use crate::functions::{external, SelectedFunction, SelectionMatches};

/// The arguments of a `selected` call with its constant options folded, such as an array
/// literal, which DataFusion plans as a call. The state and the partition stay as they are.
pub(crate) fn fold_options(args: &[Expr], schema: &DFSchema) -> Result<Vec<Expr>> {
    let simplifier = ExprSimplifier::new(
        SimplifyContext::builder()
            .with_schema(Arc::new(schema.clone()))
            .build(),
    );
    (args.iter())
        .enumerate()
        .map(|(index, arg)| match (index, arg) {
            (0 | 3, _) | (_, Expr::Literal(..) | Expr::Placeholder(_)) => Ok(arg.clone()),
            _ => simplifier.simplify(arg.clone()),
        })
        .collect()
}

/// Whether `func` is `selected`.
pub(crate) fn is_selected(func: &ScalarUDF) -> bool {
    (func.inner().as_ref() as &dyn Any).is::<SelectedFunction>()
}

/// The arguments of a `selected` call whose state is bound.
pub(crate) struct Call {
    pub(crate) state: SelectionState,
    pub(crate) options: Options,
}

/// The arguments of a `selected` call after its state: the options of its test, and the struct of
/// the row's partition columns that a faceted owner's test reads.
pub(crate) struct Options {
    pub(crate) selected: SelectedOptions,
    pub(crate) partition: Option<Expr>,
}

/// The literal of `argument`, or a plan error that names `name`.
fn literal<'a>(argument: Option<&'a Expr>, name: &str) -> Result<Option<&'a ScalarValue>> {
    match argument {
        None => Ok(None),
        Some(Expr::Literal(value, _)) if value.is_null() => Ok(None),
        Some(Expr::Literal(value, _)) => Ok(Some(value)),
        Some(other) => plan_err!("the argument `{name}` of `selected` is a constant, not {other}"),
    }
}

fn string(value: &ScalarValue, name: &str) -> Result<String> {
    match value.try_as_str() {
        Some(Some(text)) => Ok(text.to_string()),
        _ => plan_err!("the argument `{name}` of `selected` is a string, not {value}"),
    }
}

impl Call {
    /// The call of the state `state`, with the options of the arguments after the first.
    pub(crate) fn new(state: SelectionState, args: &[Expr]) -> Result<Self> {
        Ok(Self {
            state,
            options: Options::parse(args)?,
        })
    }

    /// The call's row test over the rows of `schema`, or its answer for every row when the test
    /// reads no row, such as for an empty selection.
    pub(crate) fn lower(&self, schema: &DFSchema) -> Result<Expr> {
        let types = |name: &str| {
            (schema.qualified_field_with_unqualified_name(name).ok())
                .map(|(_, field)| field.data_type().clone())
        };
        let prepared = (self.state)
            .prepare_selected(&self.options.selected, types)
            .map_err(external)?;
        if let Some(answer) = prepared.constant() {
            return Ok(lit(answer));
        }
        let mut args = (prepared.columns().into_iter())
            .map(|name| column(name, schema))
            .collect::<Result<Vec<_>>>()?;
        let partition = (self.options.partition.as_ref()).filter(|_| self.options.selected.faceted);
        args.extend(partition.cloned());
        Ok(ScalarUDF::from(SelectionMatches::new(prepared, partition.is_some())).call(args))
    }
}

impl Options {
    /// The options of the arguments `args` of a call, from its second argument.
    pub(crate) fn parse(args: &[Expr]) -> Result<Self> {
        let exclude_own = match literal(args.get(1), "exclude_own")? {
            None => false,
            Some(ScalarValue::Boolean(Some(exclude))) => *exclude,
            Some(other) => {
                return plan_err!(
                    "the argument `exclude_own` of `selected` is a boolean, not {other}"
                )
            }
        };
        let owner = (literal(args.get(2), "owner")?)
            .map(|owner| string(owner, "owner"))
            .transpose()?;
        let partition = match args.get(3) {
            Some(Expr::Literal(value, _)) if value.is_null() => None,
            other => other.cloned(),
        };
        let empty_selects_none = match literal(args.get(4), "empty")? {
            None => false,
            Some(empty) => match string(empty, "empty")?.as_str() {
                "all" => false,
                "none" => true,
                other => {
                    return plan_err!(
                        "the argument `empty` of `selected` is 'all' or 'none', not '{other}'"
                    )
                }
            },
        };
        let without = match literal(args.get(5), "without")? {
            None => Vec::new(),
            Some(ScalarValue::List(list)) => {
                let owners = cast(&list.value(0), &DataType::Utf8)?;
                (owners.as_string::<i32>().iter())
                    .flatten()
                    .map(str::to_string)
                    .collect()
            }
            Some(other) => {
                return plan_err!(
                    "the argument `without` of `selected` is a list of owners, not {other}"
                )
            }
        };
        let exclude_own = match (exclude_own, owner) {
            (false, _) => Vec::new(),
            (true, Some(owner)) => vec![owner],
            (true, None) => {
                return plan_err!("`selected` with `exclude_own` needs the `owner` of the rows")
            }
        };
        Ok(Self {
            selected: SelectedOptions {
                faceted: !exclude_own.is_empty() && partition.is_some(),
                exclude_own,
                empty_selects_none,
                without,
            },
            partition,
        })
    }
}

/// The column `name` of `schema`.
pub(crate) fn column(name: &str, schema: &DFSchema) -> Result<Expr> {
    match schema.qualified_field_with_unqualified_name(name) {
        Ok((qualifier, field)) => Ok(Expr::Column(Column::new(qualifier.cloned(), field.name()))),
        Err(_) => plan_err!("a selection tests the column `{name}`, which the input does not have"),
    }
}
