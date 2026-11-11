//! The registered function of a mark, and the binding of its calls.

use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use datafusion::{
    arrow::datatypes::{DataType, Field, FieldRef},
    common::{
        internal_err, plan_datafusion_err, plan_err,
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
        Column, DFSchema, Result,
    },
    logical_expr::{
        expr::{AggregateFunction, ScalarFunction},
        function::{AccumulatorArgs, AggregateFunctionSimplification},
        simplify::SimplifyContext,
        utils::AggregateOrderSensitivity,
        Accumulator, AggregateUDF, AggregateUDFImpl, DocSection, Documentation,
        DocumentationBuilder, Expr, ExprSchemable, Signature, Volatility,
    },
};

use super::instance::MarkInstance;
use avenger_chart::chart_field;
use avenger_chart::functions::{Arg, ArgError, ChannelArg, Kind, Mark, Reason, COORD};

/// The documentation section of the mark functions.
const SECTION: DocSection = DocSection {
    include: true,
    label: "Chart Functions",
    description: Some("Functions that build avenger charts."),
};

/// The registered function of a mark: its signature, its parameter names, its documentation,
/// and its return field. Its `simplify` hook binds each call to a [`MarkInstance`], so the
/// declaration itself never runs.
pub(crate) struct MarkDeclaration {
    spec: &'static Mark,
    signature: Signature,
    documentation: Documentation,
}

impl MarkDeclaration {
    pub(crate) fn new(spec: &'static Mark) -> Self {
        let names: Vec<&str> = (spec.channels.iter().map(|channel| channel.name))
            .chain(spec.properties.iter().map(|property| property.name))
            .chain(spec.options.iter().map(|option| option.name))
            .chain([COORD])
            .collect();
        let signature = Signature::user_defined(Volatility::Immutable)
            .with_parameter_names(names.clone())
            .expect("the parameter names of a mark are distinct");
        let syntax: Vec<String> = (names.iter())
            .map(|name| format!("{name} => expression"))
            .collect();
        let mut documentation = DocumentationBuilder::new_with_details(
            SECTION,
            spec.doc,
            format!("{}({})", spec.name, syntax.join(", ")),
        );
        for channel in spec.channels {
            documentation = documentation.with_argument(channel.name, channel.doc);
        }
        for property in spec.properties {
            documentation = documentation.with_argument(property.name, property.doc);
        }
        for option in spec.options {
            documentation = documentation.with_argument(option.name, option.doc);
        }
        documentation = documentation.with_argument(
            COORD,
            "The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.",
        );
        Self {
            spec,
            signature,
            documentation: documentation.build(),
        }
    }
}

impl std::fmt::Debug for MarkDeclaration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarkDeclaration")
            .field("name", &self.spec.name)
            .finish_non_exhaustive()
    }
}

impl PartialEq for MarkDeclaration {
    fn eq(&self, other: &Self) -> bool {
        self.spec.name == other.spec.name
    }
}

impl Eq for MarkDeclaration {}

impl Hash for MarkDeclaration {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.spec.name.hash(state);
    }
}

impl AggregateUDFImpl for MarkDeclaration {
    fn name(&self) -> &str {
        self.spec.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn coerce_types(&self, arg_types: &[DataType]) -> Result<Vec<DataType>> {
        let spec = self.spec;
        let count = spec.channels.len() + spec.properties.len() + spec.options.len() + 1;
        let name = spec.name;
        if arg_types.len() > count {
            return plan_err!(
                "{name}() takes {count} arguments at most, not {}",
                arg_types.len()
            );
        }
        Ok(arg_types.to_vec())
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::BinaryView)
    }

    fn return_field(&self, _arg_fields: &[FieldRef]) -> Result<FieldRef> {
        Ok(Arc::new(chart_field(self.spec.name, true)))
    }

    fn accumulator(&self, _acc_args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        internal_err!(
            "{}() runs only after the simplifier binds its call, which a session that skips \
             failed optimizer rules can leave unbound",
            self.spec.name
        )
    }

    fn order_sensitivity(&self) -> AggregateOrderSensitivity {
        AggregateOrderSensitivity::Beneficial
    }

    fn simplify(&self) -> Option<AggregateFunctionSimplification> {
        let spec = self.spec;
        Some(Box::new(move |call, context| bind(spec, call, context)))
    }

    fn documentation(&self) -> Option<&Documentation> {
        Some(&self.documentation)
    }
}

/// The call `call` of the mark `spec`, bound to an instance that takes only its columns.
fn bind(spec: &'static Mark, call: AggregateFunction, context: &SimplifyContext) -> Result<Expr> {
    let name = spec.name;
    let params = call.params;
    if params.distinct {
        return plan_err!("{name}() takes no DISTINCT");
    }
    if params.null_treatment.is_some() {
        return plan_err!("{name}() takes no IGNORE NULLS or RESPECT NULLS");
    }
    let schema = context.schema();
    let failed = |error: ArgError| match error.reason {
        Reason::Mark(_) => plan_datafusion_err!("{error}"),
        _ => plan_datafusion_err!("{name}(): {error}"),
    };
    let mut mark = spec.call();
    let mut data: Vec<Expr> = Vec::new();
    let mut fields: Vec<FieldRef> = Vec::new();
    let mut args = params.args.into_iter();
    let channels: Vec<Expr> = args.by_ref().take(spec.channels.len()).collect();
    let properties: Vec<Expr> = args.by_ref().take(spec.properties.len()).collect();
    let options: Vec<Expr> = args.by_ref().take(spec.options.len()).collect();
    let coord = args.next();
    for (declared, argument) in spec.channels.iter().zip(channels) {
        let (argument, explicit, options) = match kind_argument(argument) {
            Ok((kind, inner, options)) => (inner, Some(kind), options),
            Err(argument) => (argument, None, Vec::new()),
        };
        let options = constants(name, &format!("an option of `{}`", declared.name), options)?;
        if let Some(inner) = contained_kind(&argument) {
            return plan_err!(
                "{inner}() is the whole value of a channel of {name}(), and `{}` holds one inside \
                 an expression",
                declared.name
            );
        }
        // A bare value takes the default kind of its channel, which the mark call gives it.
        let argument = match argument {
            Expr::Literal(value, _) if value.is_null() && explicit.is_none() => continue,
            Expr::Literal(value, _) if value.data_type() == DataType::Null => {
                let kind = explicit
                    .expect("a bare NULL leaves its channel unset")
                    .name();
                return plan_err!(
                    "{kind}(NULL) of `{}` has no type: cast the NULL, or leave out the channel",
                    declared.name
                );
            }
            Expr::Literal(value, _) => ChannelArg::Value(value.to_array_of_size(1)?),
            argument => {
                let column = match data.iter().position(|known| *known == argument) {
                    Some(index) => fields[index].name().clone(),
                    None => {
                        let column = column_name(&argument);
                        fields.push(data_field(&argument, &column, schema)?);
                        data.push(argument);
                        column
                    }
                };
                ChannelArg::Field(column)
            }
        };
        mark.channel(declared.name, argument, explicit, &options)
            .map_err(failed)?;
    }
    let properties = constants(name, "a property", properties)?;
    mark.properties(&properties).map_err(failed)?;
    let options = constants(name, "an option of its plot or its layer", options)?;
    mark.options(&options).map_err(failed)?;
    let coord = constants(name, "`coord`", coord.into_iter().collect())?;
    if let Some(coord) = coord.into_iter().next().flatten() {
        mark.coord(&coord).map_err(failed)?;
    }
    let template = mark.bind().map_err(failed)?;
    let instance = MarkInstance::new(template, fields, &params.order_by);
    Ok(Expr::AggregateFunction(AggregateFunction::new_udf(
        Arc::new(AggregateUDF::new_from_impl(instance)),
        data,
        false,
        params.filter,
        params.order_by,
        None,
    )))
}

/// The kind, the value, and the options of `expr` when it is a call of `scaled` or `direct`, or
/// `expr` itself.
fn kind_argument(expr: Expr) -> std::result::Result<(Kind, Expr, Vec<Expr>), Expr> {
    match expr {
        Expr::ScalarFunction(ScalarFunction { func, mut args }) if !args.is_empty() => {
            match Kind::of(func.name()) {
                Some(kind) => {
                    let value = args.remove(0);
                    Ok((kind, value, args))
                }
                None => Err(Expr::ScalarFunction(ScalarFunction { func, args })),
            }
        }
        expr => Err(expr),
    }
}

/// The arguments of `exprs`, which are literals after folding, as the options of `what` of the
/// mark `name`: `None` for a `NULL`, which leaves its parameter unset.
fn constants(name: &str, what: &str, exprs: Vec<Expr>) -> Result<Vec<Option<Arg>>> {
    (exprs.into_iter())
        .map(|expr| match expr {
            Expr::Literal(value, _) => super::arg(&value),
            other => plan_err!(
                "{name}() needs a plan-time constant for {what}, not `{}`",
                other.human_display()
            ),
        })
        .collect()
}

/// The name of the first call of `scaled` or `direct` in `expr`.
fn contained_kind(expr: &Expr) -> Option<String> {
    let mut found = None;
    expr.apply(|expr| {
        if let Expr::ScalarFunction(ScalarFunction { func, .. }) = expr {
            if Kind::of(func.name()).is_some() {
                found = Some(func.name().to_string());
                return Ok(TreeNodeRecursion::Stop);
            }
        }
        Ok(TreeNodeRecursion::Continue)
    })
    .expect("the walk does not fail");
    found
}

/// The name of the layer table's column of the data argument `argument`: the name of a column
/// reference, or the display of another expression without the qualifiers of its columns, such as
/// `body_mass * 2`, which axis and legend titles show.
pub(super) fn column_name(argument: &Expr) -> String {
    if let Expr::Column(column) = argument {
        return column.name.clone();
    }
    let unqualified = argument
        .clone()
        .transform(|expr| match expr {
            Expr::Column(column) if column.relation.is_some() => Ok(Transformed::yes(
                Expr::Column(Column::new_unqualified(column.name)),
            )),
            expr => Ok(Transformed::no(expr)),
        })
        .expect("the rewrite does not fail")
        .data;
    let name = unqualified.human_display().to_string();
    name
}

/// The field of the layer table's column `column`, of the data argument `argument`: its type,
/// its nullability, and its metadata.
fn data_field(argument: &Expr, column: &str, schema: &DFSchema) -> Result<FieldRef> {
    let (_, field) = argument.to_field(schema)?;
    Ok(Arc::new(
        Field::new(column, field.data_type().clone(), field.is_nullable())
            .with_metadata(field.metadata().clone()),
    ))
}
