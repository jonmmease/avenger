//! The generation of `parameters.rs`, behind the `functions` feature: the code of the parameter
//! model, for each of its messages, which are the messages with constructors, the mark messages,
//! and `Encoding`. For each message, it generates its parameters with the inputs that each accepts,
//! a typed function that applies one argument to its builder, the conversion of a plain value or a
//! fragment to its builder, the conversion of a shorthand, which the first parameter of its
//! constructor takes, the member picking of each of its oneofs, and the names of the values of each
//! enum that a parameter takes. The constructors and the mark functions apply their arguments with
//! it.
//!
//! The generated code calls only the generated setters of the builders, so a function builds what
//! the builder builds for the same content.

use std::{collections::BTreeSet, fmt::Write};

use prost_types::FileDescriptorSet;

use super::{
    builders::{clean, method, rust_path, Comments},
    camel, ident,
    model::{Chain, Class, Input, MessageModel, Model, Node, ParamKind, Set, Shape},
    snake, PACKAGE,
};

/// The code of `parameters.rs`.
pub(super) fn parameters(
    set: &FileDescriptorSet,
    model: &Model,
    comments: &Comments,
) -> Result<String, String> {
    let mut code = String::new();
    let mut enums: BTreeSet<String> = BTreeSet::new();
    for message in &model.messages {
        emit_message(&mut code, message, model, comments, &mut enums)?;
    }
    for name in &enums {
        emit_enum(&mut code, set, comments, name)?;
    }
    Ok(code)
}

/// The Rust path of the builder of the message `name`.
pub(super) fn builder(name: &str) -> String {
    format!("{}Builder", rust_path(name))
}

/// A comment as one documentation string.
pub(super) fn doc(comment: Option<&String>) -> String {
    comment.map_or(String::new(), |c| clean(c))
}

/// The name of the constant of the parameters of `message`.
pub(super) fn params_const(message: &MessageModel) -> String {
    format!("PARAMS_{}", message.function.to_uppercase())
}

/// The function name of the message `name` in the model.
pub(super) fn function_of(model: &Model, name: &str) -> String {
    model.message(name).map_or_else(
        || snake(name.rsplit('.').next().unwrap_or(name)),
        |m| m.function.clone(),
    )
}

/// The name of the constructor of the oneof `oneof` of the message `message`, as the table of the
/// constructors names it, such as `scale.type`.
pub(super) fn oneof_constructor(message: &MessageModel, oneof: usize) -> String {
    format!(
        "{}.{}",
        message.function,
        message.message.oneof_decl[oneof].name()
    )
}

/// The type of the fragment of the constructor of a oneof, the full name of its message and the
/// oneof's name, such as `avenger.chart.v1.Scale.type`.
pub(super) fn oneof_fragment(message: &MessageModel, oneof: usize) -> String {
    format!(
        "{}.{}",
        message.name.trim_start_matches('.'),
        message.message.oneof_decl[oneof].name()
    )
}

/// The documentation of a parameter: the comment of its field or its oneof, or, without one, the
/// comment of its field's message, or for a oneof, the first sentence of the comment of each of its
/// members.
pub(super) fn param_doc(
    message: &MessageModel,
    kind: &ParamKind,
    comments: &Comments,
) -> Option<String> {
    let comment = match kind {
        ParamKind::Field(field, _) => {
            let index = (message.message.field.iter()).position(|f| f.name() == field.name());
            index.and_then(|index| comments.fields.get(&(message.name.clone(), index)))
        }
        ParamKind::Oneof(index, _) => comments.oneofs.get(&(message.name.clone(), *index)),
    };
    if let Some(comment) = comment {
        return Some(comment.clone());
    }
    match kind {
        ParamKind::Field(_, Shape::Message(name)) => comments.messages.get(name).cloned(),
        ParamKind::Oneof(_, members) => {
            let lines: Vec<String> = (members.iter())
                .filter_map(|(member, _)| {
                    let comment = member_comment(message, member, comments)?;
                    let first = comment
                        .split(". ")
                        .next()
                        .unwrap_or(&comment)
                        .trim_end_matches('.');
                    Some(format!("`{}`: {first}.", member.name()))
                })
                .collect();
            (!lines.is_empty()).then(|| format!("One of these:\n\n{}", lines.join("\n\n")))
        }
        _ => None,
    }
}

/// The inputs of a field parameter of `shape`, as the code of a slice of `Accept`s.
pub(super) fn field_accepts(shape: &Shape, model: &Model, enums: &mut BTreeSet<String>) -> String {
    if let Shape::Message(name) = shape {
        return accepts_const(model, name, None);
    }
    let code = shape_code(shape, model, enums);
    let mut accepts: Vec<String> = (shape.classes().into_iter())
        .map(|(class, _)| {
            format!(
                "Accept {{ class: Class::{}, chain: &[], shape: {code} }}",
                class.variant()
            )
        })
        .collect();
    // An array of a repeated message field also takes, for each element, a shorthand of the
    // message, as `tracks => [200, 400]` sets the length of each track.
    if let Shape::Repeated(item) = shape {
        if let Shape::Message(name) = item.as_ref() {
            for accept in element_accepts(model, name, enums) {
                if !accepts.contains(&accept) {
                    accepts.push(accept);
                }
            }
        }
    }
    format!("&[{}]", accepts.join(", "))
}

/// The code of one message: its parameters, the inputs of its message and its oneofs, its apply
/// functions, its conversions, and its oneofs.
fn emit_message(
    code: &mut String,
    message: &MessageModel,
    model: &Model,
    comments: &Comments,
    enums: &mut BTreeSet<String>,
) -> Result<(), String> {
    let function = &message.function;
    let builder = builder(&message.name);
    writeln!(
        code,
        "pub(crate) const {}: &[Param] = &[",
        params_const(message)
    )
    .unwrap();
    for param in &message.params {
        let comment = param_doc(message, &param.kind, comments);
        let accepts = match &param.kind {
            ParamKind::Field(_, shape) => field_accepts(shape, model, enums),
            ParamKind::Oneof(index, _) => accepts_const(model, &message.name, Some(*index)),
        };
        writeln!(
            code,
            "    Param {{ name: {:?}, doc: {:?}, required: {}, accepts: {accepts}, constructor: {:?}, positional: false }},",
            param.name,
            doc(comment.as_ref()),
            param.required,
            param.constructor,
        )
        .unwrap();
    }
    code.push_str("];\n\n");

    // The inputs of the message, of its shorthands, and of each of its oneofs.
    let mut accepts = chain_accepts(&message.bare(), model, enums);
    accepts.push(fragment_accept(
        message.name.trim_start_matches('.'),
        function,
    ));
    emit_accepts(code, &accepts_const(model, &message.name, None), &accepts);
    if message.has_shorthand() {
        let shorthand: Vec<&Chain> = message.shorthand.iter().collect();
        let accepts = chain_accepts(&shorthand, model, enums);
        emit_accepts(code, &shorthand_const(message), &accepts);
    }
    for param in &message.params {
        if let ParamKind::Oneof(oneof, _) = &param.kind {
            let values: Vec<&Chain> = model.oneof_values(&message.name, *oneof).iter().collect();
            let mut accepts = chain_accepts(&values, model, enums);
            emit_accepts(
                code,
                &format!("VALUES_{}_{oneof}", function.to_uppercase()),
                &accepts,
            );
            accepts.push(fragment_accept(
                &oneof_fragment(message, *oneof),
                &oneof_constructor(message, *oneof),
            ));
            emit_accepts(
                code,
                &accepts_const(model, &message.name, Some(*oneof)),
                &accepts,
            );
        }
    }

    // The application of the argument of each parameter.
    if message.params.is_empty() {
        writeln!(
            code,
            "pub(crate) fn apply_{function}(builder: {builder}, param: usize, arg: &Arg) -> Result<{builder}, ArgError> {{\n    let _ = (builder, arg);\n    unreachable!(\"`{}` has no parameter {{param}}\")\n}}\n",
            message.message.name()
        )
        .unwrap();
    } else {
        writeln!(
            code,
            "pub(crate) fn apply_{function}(builder: {builder}, param: usize, arg: &Arg) -> Result<{builder}, ArgError> {{\n    Ok(match param {{"
        )
        .unwrap();
        for (index, param) in message.params.iter().enumerate() {
            match &param.kind {
                ParamKind::Field(field, shape) => {
                    note_enums(shape, enums);
                    let setter = match (param.exclusive, shape) {
                        (Some(_), Shape::Message(_)) => format!("with_{}", field.name()),
                        _ => method(field.name()),
                    };
                    writeln!(
                        code,
                        "        {index} => builder.{setter}({}),",
                        conversion(shape, "arg", model)
                    )
                    .unwrap();
                }
                ParamKind::Oneof(oneof, members) => {
                    for (_, shape) in members {
                        note_enums(shape, enums);
                    }
                    writeln!(
                        code,
                        "        {index} => oneof_{function}_{oneof}(builder, arg)?,"
                    )
                    .unwrap();
                }
            }
        }
        writeln!(
            code,
            "        _ => unreachable!(\"a parameter of `{}`\"),\n    }})\n}}\n",
            message.message.name()
        )
        .unwrap();
    }

    // Every argument in the order of the parameters, after a check that a call gives at most one
    // of the parameters of each oneof whose members are parameters of their own. `filled` is the
    // parameter that the constructor's first parameter filled, which a call gives once and which
    // is then not missing.
    let mut exclusive = String::new();
    let oneofs: BTreeSet<usize> = message.params.iter().filter_map(|p| p.exclusive).collect();
    for oneof in oneofs {
        let indices: Vec<String> = (message.params.iter().enumerate())
            .filter(|(_, param)| param.exclusive == Some(oneof))
            .map(|(index, _)| index.to_string())
            .collect();
        writeln!(
            exclusive,
            "    let given: Vec<&'static str> = [{}].into_iter().filter(|index| matches!(args.get(*index), Some(Some(_))) || filled == Some(*index)).map(|index| {}[index].name).collect();\n    if let [first, second, ..] = given[..] {{\n        return Err(ArgError::exclusive(first).within(second));\n    }}",
            indices.join(", "),
            params_const(message)
        )
        .unwrap();
    }
    writeln!(
        code,
        "pub(crate) fn apply_given_{function}(mut builder: {builder}, args: &[Option<Arg>], filled: Option<usize>) -> Result<{builder}, ArgError> {{\n{exclusive}    for (index, arg) in args.iter().enumerate() {{\n        let param = &{params}[index];\n        match arg {{\n            Some(_) if filled == Some(index) => return Err(ArgError::twice(param.name)),\n            Some(arg) => builder = apply_{function}(builder, index, arg).map_err(|error| error.within(param.name))?,\n            None if param.required && filled != Some(index) => return Err(ArgError::required(param.name)),\n            None => {{}}\n        }}\n    }}\n    Ok(builder)\n}}\n",
        params = params_const(message)
    )
    .unwrap();
    writeln!(
        code,
        "pub(crate) fn apply_all_{function}(builder: {builder}, args: &[Option<Arg>]) -> Result<{builder}, ArgError> {{\n    apply_given_{function}(builder, args, None)\n}}\n"
    )
    .unwrap();

    // The oneofs.
    for param in &message.params {
        let ParamKind::Oneof(oneof, _) = &param.kind else {
            continue;
        };
        let node = Node::Oneof(message.name.clone(), *oneof);
        writeln!(
            code,
            "fn oneof_{function}_{oneof}(builder: {builder}, arg: &Arg) -> Result<{builder}, ArgError> {{"
        )
        .unwrap();
        let accepts = accepts_const(model, &message.name, Some(*oneof));
        let values: Vec<&Chain> = model.oneof_values(&message.name, *oneof).iter().collect();
        let oneof_ident = ident(message.message.oneof_decl[*oneof].name());
        let marker = oneof_fragment(message, *oneof);
        let fragment = format!(
            "            {marker:?} => {{\n                let mut fragment = fragment.clone();\n                fragment.r#type = {:?}.to_string();\n                let source = crate::builder::from_fragment::<{}>(&fragment).map_err(ArgError::invalid)?;\n                let mut builder = builder;\n                let own = builder.parts.adopt(\"\", source.message, source.parts);\n                builder.message.{oneof_ident} = own.{oneof_ident};\n                Ok(builder)\n            }}\n",
            message.name.trim_start_matches('.'),
            rust_path(&message.name)
        );
        emit_dispatch(
            code,
            model,
            &values,
            Some(&node),
            "builder",
            &accepts,
            &fragment,
        );
        code.push_str("}\n\n");
    }

    // The members that a string names, in each oneof of the message.
    for ((name, oneof), named) in &model.named {
        if *name != message.name {
            continue;
        }
        let members: Vec<(&prost_types::FieldDescriptorProto, &Shape)> = (message.params.iter())
            .flat_map(|param| match &param.kind {
                ParamKind::Oneof(index, members) if index == oneof => {
                    members.iter().map(|(m, s)| (*m, s)).collect::<Vec<_>>()
                }
                ParamKind::Field(member, shape) if param.exclusive == Some(*oneof) => {
                    vec![(*member, shape)]
                }
                _ => Vec::new(),
            })
            .collect();
        let names: Vec<String> = (members.iter())
            .filter(|(member, _)| named.iter().any(|n| n == member.name()))
            .map(|(member, _)| {
                let comment = member_comment(message, member, comments).unwrap_or_default();
                format!("Name {{ name: {:?}, doc: {comment:?} }}", member.name())
            })
            .collect();
        let names_const = names_const(model, &message.name, *oneof);
        writeln!(
            code,
            "pub(crate) const {names_const}: &[Name] = &[{}];\n",
            names.join(", ")
        )
        .unwrap();
        writeln!(
            code,
            "fn names_{function}_{oneof}(builder: {builder}, name: &str) -> Result<{builder}, ArgError> {{\n    Ok(match name {{"
        )
        .unwrap();
        for (member, shape) in &members {
            if let (true, Shape::Message(type_name)) =
                (named.iter().any(|n| n == member.name()), shape)
            {
                writeln!(
                    code,
                    "        {:?} => builder.with_{}({}::default()),",
                    member.name(),
                    member.name(),
                    self::builder(type_name)
                )
                .unwrap();
            }
        }
        writeln!(
            code,
            "        other => return Err(ArgError::member({names_const}, other)),\n    }})\n}}\n"
        )
        .unwrap();
    }

    // The conversion of a plain value or a fragment to the builder.
    let accepts = accepts_const(model, &message.name, None);
    writeln!(
        code,
        "#[allow(dead_code)]\npub(crate) fn to_{function}(arg: &Arg) -> Result<{builder}, ArgError> {{"
    )
    .unwrap();
    let fragment = format!(
        "            {:?} => Ok(crate::builder::from_fragment::<{}>(fragment).map_err(ArgError::invalid)?),\n",
        message.name.trim_start_matches('.'),
        rust_path(&message.name)
    );
    emit_dispatch(code, model, &message.bare(), None, "", &accepts, &fragment);
    code.push_str("}\n\n");

    // The conversion of a shorthand, which the first parameter of the constructor takes, onto a
    // builder, with the parameter that it fills.
    if message.has_shorthand() {
        writeln!(
            code,
            "#[allow(dead_code)]\npub(crate) fn lift_{function}(builder: {builder}, arg: &Arg) -> Result<({builder}, Option<usize>), ArgError> {{"
        )
        .unwrap();
        let shorthand: Vec<&Chain> = message.shorthand.iter().collect();
        let filled: Vec<Option<usize>> = message.filled.clone();
        emit_lift(
            code,
            model,
            message,
            &shorthand,
            &filled,
            &shorthand_const(message),
        );
        code.push_str("}\n\n");
    }
    Ok(())
}

/// The `Accept` of a fragment of the message or the oneof `message`, whose constructor is
/// `constructor`.
fn fragment_accept(message: &str, constructor: &str) -> String {
    format!(
        "Accept {{ class: Class::Fragment, chain: &[], shape: Shape::Message {{ message: {message:?}, constructor: {constructor:?} }} }}"
    )
}

/// The `Accept`s of `chains`, each with the names of the fields and members that its steps set,
/// innermost first.
fn chain_accepts(chains: &[&Chain], model: &Model, enums: &mut BTreeSet<String>) -> Vec<String> {
    let mut accepts = Vec::new();
    for chain in chains {
        let (steps, shape) = chain_shape(chain, model, enums);
        let accept = format!(
            "Accept {{ class: Class::{}, chain: &[{}], shape: {shape} }}",
            chain.class.variant(),
            steps.join(", ")
        );
        if !accepts.contains(&accept) {
            accepts.push(accept);
        }
    }
    accepts
}

/// The names of the steps of `chain`, innermost first, and the code of the shape of the value that
/// it reads.
fn chain_shape(
    chain: &Chain,
    model: &Model,
    enums: &mut BTreeSet<String>,
) -> (Vec<String>, String) {
    let mut steps = Vec::new();
    let mut shape = None;
    for edge in &chain.edges {
        match &edge.set {
            Set::Field(field) | Set::Member(field) => steps.push(format!("{:?}", field.name())),
            Set::Names(message, oneof) => {
                shape = Some(format!(
                    "Shape::Names({})",
                    names_const(model, message, *oneof)
                ));
            }
        }
        if shape.is_none() {
            if let Input::Class(input) = &edge.input {
                shape = Some(shape_code(input, model, enums));
            }
        }
    }
    (steps, shape.expect("a chain reads a value"))
}

/// The inputs of an array of the message `name` from values that convert to the message element
/// by element: one for each shape of the values, with the chain of the conversion.
fn element_accepts(model: &Model, name: &str, enums: &mut BTreeSet<String>) -> Vec<String> {
    let Some(message) = model.message(name) else {
        return Vec::new();
    };
    let mut accepts = Vec::new();
    for chain in message.bare() {
        let (steps, shape) = chain_shape(chain, model, enums);
        let accept = format!(
            "Accept {{ class: Class::Array, chain: &[{}], shape: Shape::Array(&{shape}) }}",
            steps.join(", ")
        );
        if !accepts.contains(&accept) {
            accepts.push(accept);
        }
    }
    accepts
}

/// The comment of the member `member` of a oneof of `message`, or of its message.
pub(super) fn member_comment(
    message: &MessageModel,
    member: &prost_types::FieldDescriptorProto,
    comments: &Comments,
) -> Option<String> {
    let index = (message.message.field.iter()).position(|f| f.name() == member.name())?;
    let own = comments.fields.get(&(message.name.clone(), index));
    let message_comment = || comments.messages.get(member.type_name());
    Some(clean(own.or_else(message_comment)?))
}

/// The name of the constant of the inputs of the message `name`, or of its oneof `oneof`.
pub(super) fn accepts_const(model: &Model, name: &str, oneof: Option<usize>) -> String {
    let function = function_of(model, name).to_uppercase();
    match oneof {
        Some(oneof) => format!("ACCEPTS_{function}_{oneof}"),
        None => format!("ACCEPTS_{function}"),
    }
}

/// The name of the constant of the shorthands of `message`.
pub(super) fn shorthand_const(message: &MessageModel) -> String {
    format!("SHORTHAND_{}", message.function.to_uppercase())
}

/// The name of the constant of the members that a string names, of the oneof `oneof` of the
/// message `name`.
fn names_const(model: &Model, name: &str, oneof: usize) -> String {
    format!("NAMES_{}_{oneof}", function_of(model, name).to_uppercase())
}

/// The name of the constant of the values of the enum `name`.
fn enum_const(name: &str) -> String {
    format!("ENUM_{}", snake(short(name)).to_uppercase())
}

/// The constant `name` of the inputs `accepts`.
fn emit_accepts(code: &mut String, name: &str, accepts: &[String]) {
    let lines: Vec<String> = accepts
        .iter()
        .map(|accept| format!("    {accept},"))
        .collect();
    writeln!(
        code,
        "pub(crate) const {name}: &[Accept] = &[\n{}\n];\n",
        lines.join("\n")
    )
    .unwrap();
}

/// The code of a `Shape` of `shape`.
pub(super) fn shape_code(shape: &Shape, model: &Model, enums: &mut BTreeSet<String>) -> String {
    match shape {
        Shape::Bool => "Shape::Bool".into(),
        Shape::Int(_) => "Shape::Int".into(),
        Shape::Float(_) => "Shape::Float".into(),
        Shape::Str => "Shape::Str".into(),
        Shape::Markup => "Shape::Markup".into(),
        Shape::FormatPattern => "Shape::FormatPattern".into(),
        Shape::Bytes => "Shape::Bytes".into(),
        Shape::Enum(name) => {
            enums.insert(name.clone());
            format!(
                "Shape::Enum {{ name: {:?}, values: {} }}",
                name.trim_start_matches('.'),
                enum_const(name)
            )
        }
        Shape::Value => "Shape::Value".into(),
        Shape::Message(name) => format!(
            "Shape::Message {{ message: {:?}, constructor: {:?} }}",
            name.trim_start_matches('.'),
            function_of(model, name)
        ),
        Shape::Repeated(item) => format!("Shape::Array(&{})", shape_code(item, model, enums)),
        Shape::Options => "Shape::Options".into(),
    }
}

/// Records the enums that `shape` holds.
fn note_enums(shape: &Shape, enums: &mut BTreeSet<String>) {
    match shape {
        Shape::Enum(name) => {
            enums.insert(name.clone());
        }
        Shape::Repeated(item) => note_enums(item, enums),
        _ => {}
    }
}

/// The body of a function that turns `arg` into its target through `chains`, or from a fragment
/// through the match arms `fragment`, whose error names the inputs of the constant `accepts`. An
/// edge whose target is `base_node` sets `base`, the builder that the function takes.
#[allow(clippy::too_many_arguments)]
fn emit_dispatch(
    code: &mut String,
    model: &Model,
    chains: &[&Chain],
    base_node: Option<&Node>,
    base: &str,
    accepts: &str,
    fragment: &str,
) {
    let rejected = format!("Err(ArgError::rejected(Expected::Inputs({accepts}), arg))");
    code.push_str("    match arg {\n");
    if !fragment.is_empty() {
        code.push_str("        Arg::Fragment(fragment) => match fragment.r#type.as_str() {\n");
        code.push_str(fragment);
        writeln!(code, "            _ => {rejected},\n        }},").unwrap();
    }
    let mut seen = BTreeSet::new();
    for chain in chains {
        if !seen.insert(chain.class) {
            continue;
        }
        writeln!(
            code,
            "        Arg::{}(..) => Ok({}),",
            arg_variant(chain.class),
            chain_code(chain, base_node, base, model)
        )
        .unwrap();
    }
    writeln!(code, "        _ => {rejected},\n    }}").unwrap();
}

/// The body of `lift_` of `message`: each shorthand of `chains` onto the builder, with the
/// parameter that it fills.
fn emit_lift(
    code: &mut String,
    model: &Model,
    message: &MessageModel,
    chains: &[&Chain],
    filled: &[Option<usize>],
    accepts: &str,
) {
    let node = Node::Message(message.name.clone());
    let rejected = format!("Err(ArgError::rejected(Expected::Inputs({accepts}), arg))");
    code.push_str("    match arg {\n");
    for (chain, filled) in chains.iter().zip(filled) {
        writeln!(
            code,
            "        Arg::{}(..) => Ok(({}, {filled:?})),",
            arg_variant(chain.class),
            chain_code(chain, Some(&node), "builder", model)
        )
        .unwrap();
    }
    writeln!(code, "        _ => {rejected},\n    }}").unwrap();
}

/// The variant of `Arg` of a class.
fn arg_variant(class: Class) -> &'static str {
    match class {
        Class::Bool => "Bool",
        Class::Int => "Int",
        Class::Float => "Float",
        Class::Str => "Str",
        Class::Array => "List",
        Class::Bytes => "Bytes",
        Class::Struct => "Struct",
        Class::Other => "Value",
    }
}

/// The expression of a chain: the argument, through each edge to its target. An edge whose target
/// is `base_node` sets `base`, and another edge a default builder of its message.
fn chain_code(chain: &Chain, base_node: Option<&Node>, base: &str, model: &Model) -> String {
    let mut expression: Option<String> = None;
    for edge in &chain.edges {
        let target = match (&edge.to, Some(&edge.to) == base_node) {
            (_, true) => base.to_string(),
            (Node::Message(name), false) => format!("{}::default()", builder(name)),
            (Node::Oneof(..), false) => unreachable!("a oneof's step sets the builder it takes"),
        };
        let inner = match &edge.input {
            Input::Class(shape) => conversion(shape, "arg", model),
            Input::Message => expression.take().expect("the step before"),
            Input::Name => String::new(),
        };
        expression = Some(match &edge.set {
            Set::Field(field) => format!("{target}.{}({inner})", method(field.name())),
            Set::Member(field) => match Shape::of(field) {
                Some(Shape::Message(_)) => format!("{target}.with_{}({inner})", field.name()),
                _ => format!("{target}.{}({inner})", method(field.name())),
            },
            Set::Names(message, oneof) => format!(
                "names_{}_{oneof}({target}, &arg.string()?)?",
                function_of(model, message)
            ),
        });
    }
    expression.expect("a chain of one step or more")
}

/// The expression that converts the argument `arg` to what a setter of `shape` takes.
pub(super) fn conversion(shape: &Shape, arg: &str, model: &Model) -> String {
    match shape {
        Shape::Bool => format!("{arg}.boolean()?"),
        Shape::Int(rust) => format!("{arg}.integer::<{rust}>()?"),
        Shape::Float("f32") => format!("{arg}.float32()?"),
        Shape::Float(_) => format!("{arg}.float64()?"),
        Shape::Str | Shape::Markup | Shape::FormatPattern => format!("{arg}.string()?"),
        Shape::Bytes => format!("{arg}.bytes()?"),
        Shape::Enum(name) => format!("enum_{}({arg})?", snake(short(name))),
        Shape::Value => format!("{arg}.value()?"),
        Shape::Message(name) => format!("to_{}({arg})?", function_of(model, name)),
        Shape::Repeated(item) => format!(
            "{arg}.items()?.into_iter().map(|item| Ok::<_, ArgError>({})).collect::<Result<Vec<_>, ArgError>>()?",
            conversion(item, "item", model)
        ),
        Shape::Options => format!("{arg}.options()?"),
    }
}

/// The short name of a full name.
fn short(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// The constant of the values of the enum `name`, and the function that reads a value from its
/// lowercase name.
fn emit_enum(
    code: &mut String,
    set: &FileDescriptorSet,
    comments: &Comments,
    name: &str,
) -> Result<(), String> {
    let short_name = short(name);
    let enumeration = (set.file.iter())
        .flat_map(|file| &file.enum_type)
        .find(|e| format!("{PACKAGE}.{}", e.name()) == name)
        .ok_or(format!("`{name}` is an enum"))?;
    let prefix = format!("{}_", snake(short_name).to_uppercase());
    let path = rust_path(name);
    let mut values = Vec::new();
    let mut arms = String::new();
    for (index, value) in enumeration.value.iter().enumerate() {
        if value.number() == 0 {
            continue;
        }
        let suffix = value.name().strip_prefix(&prefix).unwrap_or(value.name());
        let lower = suffix.to_lowercase();
        let comment = doc(comments.enum_values.get(&(name.to_string(), index)));
        values.push(format!("Name {{ name: {lower:?}, doc: {comment:?} }}"));
        writeln!(arms, "        {lower:?} => {path}::{},", camel(&lower)).unwrap();
    }
    let values_const = enum_const(name);
    writeln!(
        code,
        "pub(crate) const {values_const}: &[Name] = &[{}];\n",
        values.join(", ")
    )
    .unwrap();
    writeln!(
        code,
        "fn enum_{}(arg: &Arg) -> Result<{path}, ArgError> {{\n    let name = arg.string()?;\n    Ok(match name.as_str() {{\n{arms}        other => return Err(ArgError::value({values_const}, other)),\n    }})\n}}\n",
        snake(short_name)
    )
    .unwrap();
    Ok(())
}
