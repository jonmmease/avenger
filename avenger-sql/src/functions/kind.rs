//! The channel functions, such as `x(value, <options>)`, and the markers `datum(value)` and
//! `value(value)`: each returns its value, and a mark function reads a channel argument that
//! calls them as the encoding that they give.

use std::any::Any;

use datafusion::{
    arrow::datatypes::{DataType, FieldRef},
    common::Result,
    logical_expr::{
        expr::ScalarFunction, ColumnarValue, Expr, ReturnFieldArgs, ScalarFunctionArgs,
        ScalarUDFImpl, Signature, Volatility,
    },
};

use avenger_chart::functions::{ChannelFunction, Kind};

/// `datum(value)` or `value(value)`, whose result is `value`. A mark function reads a channel
/// argument that calls one of them, alone or as the value of a channel function, as an encoding of
/// its kind.
///
/// It is volatile, so the simplifier does not fold a call of a constant into the constant before
/// a mark function binds its call.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct KindFunction {
    kind: Kind,
    signature: Signature,
}

impl KindFunction {
    pub(crate) fn new(kind: Kind) -> Self {
        Self {
            kind,
            signature: Signature::any(1, Volatility::Volatile),
        }
    }
}

impl ScalarUDFImpl for KindFunction {
    fn name(&self) -> &str {
        self.kind.name()
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn coerce_types(&self, arg_types: &[DataType]) -> Result<Vec<DataType>> {
        Ok(arg_types.to_vec())
    }

    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        Ok(arg_types[0].clone())
    }

    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        Ok(args.arg_fields[0].clone())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        Ok(first(args))
    }
}

/// The channel function of a channel with a scale, `name(value, <options>)`, whose result is
/// `value`, and whose arguments after the value are its options, in the order of the function's
/// options. A mark function reads a channel argument that calls it as an encoding with those
/// options.
///
/// It is volatile, as [`KindFunction`] is.
#[derive(Debug)]
pub(crate) struct ChannelUdf {
    function: &'static ChannelFunction,
    signature: Signature,
}

impl ChannelUdf {
    pub(crate) fn new(function: &'static ChannelFunction) -> Self {
        Self {
            function,
            signature: Signature::user_defined(Volatility::Volatile),
        }
    }
}

impl PartialEq for ChannelUdf {
    fn eq(&self, other: &Self) -> bool {
        self.function.name == other.function.name
    }
}

impl Eq for ChannelUdf {}

impl std::hash::Hash for ChannelUdf {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.function.name.hash(state);
    }
}

impl ScalarUDFImpl for ChannelUdf {
    fn name(&self) -> &str {
        self.function.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn coerce_types(&self, arg_types: &[DataType]) -> Result<Vec<DataType>> {
        Ok(arg_types.to_vec())
    }

    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        Ok(arg_types[0].clone())
    }

    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        Ok(args.arg_fields[0].clone())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        Ok(first(args))
    }
}

/// The value of a call of a channel function or a marker: its first argument.
fn first(args: ScalarFunctionArgs) -> ColumnarValue {
    args.args
        .into_iter()
        .next()
        .expect("a channel function takes its value first")
}

/// A channel argument of a mark as its calls give it: the channel function that wraps it with
/// its options, the kind that `datum` or `value` gives, and the value inside them.
pub(crate) struct Wrapped<'e> {
    pub function: Option<(&'static ChannelFunction, &'e [Expr])>,
    pub kind: Option<Kind>,
    pub value: &'e Expr,
}

/// The channel function of the call `expr`, when it calls one.
pub(crate) fn channel_function(expr: &Expr) -> Option<&'static ChannelFunction> {
    match expr {
        Expr::ScalarFunction(ScalarFunction { func, args }) if !args.is_empty() => {
            let inner: &dyn Any = func.inner().as_ref();
            inner.downcast_ref::<ChannelUdf>().map(|udf| udf.function)
        }
        _ => None,
    }
}

/// The kind that the call `expr` gives, when it calls `datum` or `value`.
pub(crate) fn marker(expr: &Expr) -> Option<Kind> {
    match expr {
        Expr::ScalarFunction(ScalarFunction { func, args }) if !args.is_empty() => {
            let inner: &dyn Any = func.inner().as_ref();
            inner.downcast_ref::<KindFunction>().map(|udf| udf.kind)
        }
        _ => None,
    }
}

/// Whether `expr` calls a channel function, `datum`, or `value`.
pub(crate) fn wraps(expr: &Expr) -> bool {
    channel_function(expr).is_some() || marker(expr).is_some()
}

/// The parts of the channel argument `expr`: a channel function around it, then `datum` or
/// `value` inside that, then the value.
pub(crate) fn unwrap(expr: &Expr) -> Wrapped<'_> {
    let (function, expr) = match (channel_function(expr), expr) {
        (Some(function), Expr::ScalarFunction(call)) => {
            (Some((function, &call.args[1..])), &call.args[0])
        }
        _ => (None, expr),
    };
    let (kind, value) = match (marker(expr), expr) {
        (Some(kind), Expr::ScalarFunction(call)) => (Some(kind), &call.args[0]),
        _ => (None, expr),
    };
    Wrapped {
        function,
        kind,
        value,
    }
}

/// The channel argument `expr` with `f` applied to its value inside its channel function and its
/// marker.
pub(crate) fn map_value(expr: &Expr, f: &mut impl FnMut(&Expr) -> Expr) -> Expr {
    match expr {
        Expr::ScalarFunction(ScalarFunction { func, args }) if wraps(expr) => {
            let mut args = args.clone();
            args[0] = map_value(&args[0], f);
            Expr::ScalarFunction(ScalarFunction {
                func: func.clone(),
                args,
            })
        }
        expr => f(expr),
    }
}

/// `restyle(default, predicate, value, ...)`, whose result is `default`: the channel argument
/// that the pass plans for a `CASE` or an `IF` over selections, which a mark function reads as
/// a channel of the value `default` with a conditional value for each `predicate`, an encoded
/// [`avenger_chart::proto::Predicate`], whose rows draw its `value`. The renderer tests the
/// predicates, so no query reads the selections.
///
/// It is volatile, as [`KindFunction`] is.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct RestyleFunction {
    signature: Signature,
}

impl RestyleFunction {
    pub(crate) fn new() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Volatile),
        }
    }
}

impl ScalarUDFImpl for RestyleFunction {
    fn name(&self) -> &str {
        "restyle"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn coerce_types(&self, arg_types: &[DataType]) -> Result<Vec<DataType>> {
        Ok(arg_types.to_vec())
    }

    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        Ok(arg_types[0].clone())
    }

    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        Ok(args.arg_fields[0].clone())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        Ok(first(args))
    }
}

/// The call of `restyle` with the value `default` and, for each condition, its encoded
/// predicate and its value.
pub(crate) fn restyle(default: Expr, conditions: Vec<(Vec<u8>, Expr)>) -> Expr {
    use datafusion::{common::ScalarValue, logical_expr::ScalarUDF};
    let mut args = vec![default];
    for (predicate, value) in conditions {
        args.push(Expr::Literal(ScalarValue::Binary(Some(predicate)), None));
        args.push(value);
    }
    let udf = std::sync::Arc::new(ScalarUDF::new_from_impl(RestyleFunction::new()));
    Expr::ScalarFunction(ScalarFunction::new_udf(udf, args))
}

/// A condition of a restyle: its encoded predicate, and the value of the rows that it selects.
pub(crate) type Restyle<'e> = (&'e [u8], &'e Expr);

/// The value and the conditions of `expr`, when it calls `restyle`.
pub(crate) fn restyled(expr: &Expr) -> Option<(&Expr, Vec<Restyle<'_>>)> {
    use datafusion::common::ScalarValue;
    let Expr::ScalarFunction(ScalarFunction { func, args }) = expr else {
        return None;
    };
    let inner: &dyn Any = func.inner().as_ref();
    inner.downcast_ref::<RestyleFunction>()?;
    let conditions = (args[1..].chunks(2))
        .map(|pair| match &pair[0] {
            Expr::Literal(ScalarValue::Binary(Some(bytes)), _) => {
                Some((bytes.as_slice(), &pair[1]))
            }
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    Some((&args[0], conditions))
}
