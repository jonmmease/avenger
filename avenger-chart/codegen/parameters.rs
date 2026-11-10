//! The generation of `parameters.rs`, behind the `functions` feature: the code of the parameter
//! model, for each of its messages, which are the messages with constructors, the mark messages,
//! and `Encoding`. For each message, it generates its parameters with the inputs that each accepts,
//! a typed function that applies one argument to its builder, the conversions of arguments to its
//! builder, the member picking of each of its oneofs, and the names of the values of each enum that
//! a parameter takes. The constructors and the mark functions apply their arguments with it.
//!
//! The generated code calls only the generated setters of the builders, so a function builds what
//! the builder builds for the same content.

use std::{collections::BTreeSet, fmt::Write};

use prost_types::FileDescriptorSet;

use super::{
    builders::{clean, method, rust_path, Comments},
    camel,
    model::{Class, Edge, Input, MessageModel, Model, Node, ParamKind, Set, Shape, Source},
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

/// The code of one message: its parameters, the inputs of its message and its oneofs, its apply
/// functions, its conversion, and its oneofs.
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
        let comment = match &param.kind {
            ParamKind::Field(field, _) => {
                let index = message
                    .message
                    .field
                    .iter()
                    .position(|f| f.name() == field.name());
                index.and_then(|index| comments.fields.get(&(message.name.clone(), index)))
            }
            ParamKind::Oneof(index, _) => comments.oneofs.get(&(message.name.clone(), *index)),
        };
        // A parameter without a comment of its own takes the comment of its field's message, or,
        // for a oneof, the first sentence of the comment of each of its members.
        let fallback = match (&param.kind, comment) {
            (_, Some(_)) => None,
            (ParamKind::Field(_, Shape::Message(name)), None) => {
                comments.messages.get(name).cloned()
            }
            (ParamKind::Oneof(_, members), None) => {
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
        };
        let comment = comment.or(fallback.as_ref());
        let accepts = match &param.kind {
            ParamKind::Field(_, Shape::Message(name)) => accepts_const(model, name, None),
            ParamKind::Field(_, shape) => {
                let shape = shape_code(shape, model, enums);
                let accepts: Vec<String> = (shape_classes(&param.kind))
                    .into_iter()
                    .map(|class| {
                        format!(
                            "Accept {{ class: Class::{}, chain: &[], shape: {shape} }}",
                            class.variant()
                        )
                    })
                    .collect();
                format!("&[{}]", accepts.join(", "))
            }
            ParamKind::Oneof(index, _) => accepts_const(model, &message.name, Some(*index)),
        };
        writeln!(
            code,
            "    Param {{ name: {:?}, doc: {:?}, required: {}, accepts: {accepts} }},",
            param.name,
            doc(comment),
            param.required
        )
        .unwrap();
    }
    code.push_str("];\n\n");

    // The inputs of the message, and of each of its oneofs.
    let node = Node::Message(message.name.clone());
    emit_accepts(
        code,
        model,
        &node,
        &accepts_const(model, &message.name, None),
        enums,
    );
    for param in &message.params {
        if let ParamKind::Oneof(oneof, _) = &param.kind {
            let node = Node::Oneof(message.name.clone(), *oneof);
            let name = accepts_const(model, &message.name, Some(*oneof));
            emit_accepts(code, model, &node, &name, enums);
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
                    writeln!(
                        code,
                        "        {index} => builder.{}({}),",
                        method(field.name()),
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

    // Every argument in the order of the parameters.
    writeln!(
        code,
        "pub(crate) fn apply_all_{function}(mut builder: {builder}, args: &[Option<Arg>]) -> Result<{builder}, ArgError> {{\n    for (index, arg) in args.iter().enumerate() {{\n        let param = &{}[index];\n        match arg {{\n            Some(arg) => builder = apply_{function}(builder, index, arg).map_err(|error| error.within(param.name))?,\n            None if param.required => return Err(ArgError::required(param.name)),\n            None => {{}}\n        }}\n    }}\n    Ok(builder)\n}}\n",
        params_const(message)
    )
    .unwrap();

    // The oneofs.
    for param in &message.params {
        let ParamKind::Oneof(oneof, members) = &param.kind else {
            continue;
        };
        let node = Node::Oneof(message.name.clone(), *oneof);
        writeln!(
            code,
            "fn oneof_{function}_{oneof}(builder: {builder}, arg: &Arg) -> Result<{builder}, ArgError> {{"
        )
        .unwrap();
        let accepts = accepts_const(model, &message.name, Some(*oneof));
        emit_dispatch(code, model, &node, "builder", &accepts);
        code.push_str("}\n\n");
        // The members that a string names.
        let Some(named) = model.named.get(&(message.name.clone(), *oneof)) else {
            continue;
        };
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
        for (member, shape) in members {
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

    // The conversion of an argument to the builder.
    let node = Node::Message(message.name.clone());
    writeln!(
        code,
        "#[allow(dead_code)]\npub(crate) fn to_{function}(arg: &Arg) -> Result<{builder}, ArgError> {{"
    )
    .unwrap();
    let accepts = accepts_const(model, &message.name, None);
    emit_dispatch(code, model, &node, "", &accepts);
    code.push_str("}\n\n");
    Ok(())
}

/// The comment of the member `member` of a oneof of `message`, or of its message.
fn member_comment(
    message: &MessageModel,
    member: &prost_types::FieldDescriptorProto,
    comments: &Comments,
) -> Option<String> {
    let index = (message.message.field.iter()).position(|f| f.name() == member.name())?;
    let own = comments.fields.get(&(message.name.clone(), index));
    let message_comment = || comments.messages.get(member.type_name());
    Some(clean(own.or_else(message_comment)?))
}

/// The classes of the arguments of a field parameter.
fn shape_classes(kind: &ParamKind) -> Vec<Class> {
    match kind {
        ParamKind::Field(_, shape) => shape
            .classes()
            .into_iter()
            .map(|(class, _)| class)
            .collect(),
        ParamKind::Oneof(..) => Vec::new(),
    }
}

/// The name of the constant of the inputs of the message `name`, or of its oneof `oneof`.
fn accepts_const(model: &Model, name: &str, oneof: Option<usize>) -> String {
    let function = function_of(model, name).to_uppercase();
    match oneof {
        Some(oneof) => format!("ACCEPTS_{function}_{oneof}"),
        None => format!("ACCEPTS_{function}"),
    }
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

/// The constant `name` of the inputs that reach `node`, each with its chain.
fn emit_accepts(
    code: &mut String,
    model: &Model,
    node: &Node,
    name: &str,
    enums: &mut BTreeSet<String>,
) {
    let mut accepts = Vec::new();
    for (source, chain) in model.accepted(node) {
        let mut steps: Vec<String> = Vec::new();
        let mut shape = None;
        for edge in &chain {
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
                if let Input::Class(_, input) = &edge.input {
                    shape = Some(shape_code(input, model, enums));
                }
            }
        }
        let (class, shape) = match &source {
            Source::Fragment(message) => (
                "Fragment".to_string(),
                shape_code(&Shape::Message(message.clone()), model, enums),
            ),
            Source::Class(class) => (
                class.variant().to_string(),
                shape.expect("a chain from a class reads the class"),
            ),
        };
        accepts.push(format!(
            "    Accept {{ class: Class::{class}, chain: &[{}], shape: {shape} }},",
            steps.join(", ")
        ));
    }
    writeln!(
        code,
        "pub(crate) const {name}: &[Accept] = &[\n{}\n];\n",
        accepts.join("\n")
    )
    .unwrap();
}

/// The code of a `Shape` of `shape`.
fn shape_code(shape: &Shape, model: &Model, enums: &mut BTreeSet<String>) -> String {
    match shape {
        Shape::Bool => "Shape::Bool".into(),
        Shape::Int(_) => "Shape::Int".into(),
        Shape::Float(_) => "Shape::Float".into(),
        Shape::Str => "Shape::Str".into(),
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

/// The body of a function that turns `arg` into the target of `node`, on `base` for a oneof, whose
/// error names the inputs of the constant `accepts`.
fn emit_dispatch(code: &mut String, model: &Model, node: &Node, base: &str, accepts: &str) {
    let accepted = model.accepted(node);
    let rejected = format!("Err(ArgError::rejected(Expected::Inputs({accepts}), arg))");
    code.push_str("    match arg {\n");
    let fragments: Vec<&(Source, Vec<&Edge>)> = (accepted.iter())
        .filter(|(source, _)| matches!(source, Source::Fragment(_)))
        .collect();
    if !fragments.is_empty() {
        code.push_str("        Arg::Fragment(fragment) => match fragment.r#type.as_str() {\n");
        for (source, chain) in &fragments {
            let Source::Fragment(name) = source else {
                unreachable!()
            };
            let start = format!(
                "crate::builder::from_fragment::<{}>(fragment).map_err(ArgError::invalid)?",
                rust_path(name)
            );
            writeln!(
                code,
                "            {:?} => Ok({}),",
                name.trim_start_matches('.'),
                chain_code(chain, Some(start), base, model)
            )
            .unwrap();
        }
        writeln!(code, "            _ => {rejected},\n        }},").unwrap();
    }
    for (source, chain) in &accepted {
        let Source::Class(class) = source else {
            continue;
        };
        writeln!(
            code,
            "        Arg::{}(..) => Ok({}),",
            arg_variant(*class),
            chain_code(chain, None, base, model)
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
        Class::Other => "Value",
    }
}

/// The expression of a chain: `start`, the builder of a fragment, or the argument, through each
/// edge to its target, which is `base` for a oneof.
fn chain_code(chain: &[&Edge], start: Option<String>, base: &str, model: &Model) -> String {
    let mut expression = match (start, chain.is_empty()) {
        (Some(start), true) => return start,
        (start, _) => start,
    };
    for edge in chain {
        let target = match &edge.to {
            Node::Message(name) => format!("{}::default()", builder(name)),
            Node::Oneof(..) => base.to_string(),
        };
        let inner = match &edge.input {
            Input::Class(_, shape) => conversion(shape, "arg", model),
            Input::Message(_) => expression.take().expect("the step before"),
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
    expression.expect("a chain or a start")
}

/// The expression that converts the argument `arg` to what a setter of `shape` takes.
fn conversion(shape: &Shape, arg: &str, model: &Model) -> String {
    match shape {
        Shape::Bool => format!("{arg}.boolean()?"),
        Shape::Int(rust) => format!("{arg}.integer::<{rust}>()?"),
        Shape::Float("f32") => format!("{arg}.float32()?"),
        Shape::Float(_) => format!("{arg}.float64()?"),
        Shape::Str => format!("{arg}.string()?"),
        Shape::Bytes => format!("{arg}.bytes()?"),
        Shape::Enum(name) => format!("enum_{}({arg})?", snake(short(name))),
        Shape::Value => format!("{arg}.value()?"),
        Shape::Message(name) => format!("to_{}({arg})?", function_of(model, name)),
        Shape::Repeated(item) => format!(
            "{arg}.items()?.into_iter().map(|item| Ok::<_, ArgError>({})).collect::<Result<Vec<_>, ArgError>>()?",
            conversion(item, "item", model)
        ),
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
