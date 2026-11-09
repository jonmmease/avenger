//! The generation of `constructors.rs`, behind the `datafusion` feature: from the parameter model,
//! the table of the constructors, and for each message of the model a typed function that applies
//! one argument to its builder, the conversions of arguments to its builder, the member picking of
//! each of its oneofs, and the names of the values of each enum that a parameter takes.
//!
//! The generated code calls only the generated setters of the builders, so a constructor builds
//! what the builder builds for the same content. It also holds `SURFACE`, the text of every
//! constructor's parameters and the arguments that each takes, which a test compares with its
//! snapshot.

use std::{collections::BTreeSet, fmt::Write};

use prost_types::FileDescriptorSet;

use super::{
    builders::{method, rust_path, Comments},
    camel,
    functions::clean,
    model::{Class, Edge, Input, MessageModel, Model, Node, ParamKind, Set, Shape, Source},
    snake, PACKAGE,
};

/// The code of `constructors.rs`, where `constructors` names the messages with constructors.
pub(super) fn constructors(
    set: &FileDescriptorSet,
    model: &Model,
    comments: &Comments,
    constructors: &[String],
) -> Result<String, String> {
    let mut code = String::new();
    let mut surface = String::new();
    let mut enums: BTreeSet<String> = BTreeSet::new();
    for message in &model.messages {
        emit_message(&mut code, message, model, comments, &mut enums)?;
    }
    code.push_str("pub(crate) static CONSTRUCTORS: &[ConstructorSpec] = &[\n");
    for name in constructors {
        let message = model.message(name).ok_or(format!("`{name}` has a model"))?;
        let function = &message.function;
        writeln!(
            code,
            "    ConstructorSpec {{ name: {function:?}, message: {:?}, doc: {:?}, params: {}, build: build_{function} }},",
            name.trim_start_matches('.'),
            doc(comments.messages.get(name)),
            params_const(message)
        )
        .unwrap();
        describe(&mut surface, message, model);
    }
    code.push_str("];\n\n");
    for name in constructors {
        let message = model.message(name).expect("a model");
        let function = &message.function;
        writeln!(
            code,
            "fn build_{function}(args: &[Option<Arg>]) -> Result<crate::proto::Fragment, String> {{\n    let builder = apply_all_{function}({}::default(), args)?;\n    builder.to_fragment().map_err(|error| error.to_string())\n}}\n",
            builder(name)
        )
        .unwrap();
    }
    for name in &enums {
        emit_enum(&mut code, set, name)?;
    }
    writeln!(code, "pub(crate) const SURFACE: &str = {surface:?};").unwrap();
    Ok(code)
}

/// The Rust path of the builder of the message `name`.
fn builder(name: &str) -> String {
    format!("{}Builder", rust_path(name))
}

/// A comment as one documentation string.
fn doc(comment: Option<&String>) -> String {
    comment.map_or(String::new(), |c| clean(c))
}

/// The name of the constant of the parameters of `message`.
fn params_const(message: &MessageModel) -> String {
    format!("PARAMS_{}", message.function.to_uppercase())
}

/// The function name of the message `name` in the model.
fn function_of(model: &Model, name: &str) -> String {
    model.message(name).map_or_else(
        || snake(name.rsplit('.').next().unwrap_or(name)),
        |m| m.function.clone(),
    )
}

/// The code of one message: its parameters, its apply functions, its conversion, and its oneofs.
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
        "pub(crate) const {}: &[ParamSpec] = &[",
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
        writeln!(
            code,
            "    ParamSpec {{ name: {:?}, doc: {:?}, required: {} }},",
            param.name,
            doc(comment),
            param.required
        )
        .unwrap();
    }
    code.push_str("];\n\n");

    // The application of the argument of each parameter.
    if message.params.is_empty() {
        writeln!(
            code,
            "pub(crate) fn apply_{function}(builder: {builder}, param: usize, arg: &Arg) -> Result<{builder}, String> {{\n    let _ = (builder, arg);\n    unreachable!(\"`{}` has no parameter {{param}}\")\n}}\n",
            message.message.name()
        )
        .unwrap();
    } else {
        writeln!(
        code,
        "pub(crate) fn apply_{function}(builder: {builder}, param: usize, arg: &Arg) -> Result<{builder}, String> {{\n    Ok(match param {{"
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
        "pub(crate) fn apply_all_{function}(mut builder: {builder}, args: &[Option<Arg>]) -> Result<{builder}, String> {{\n    for (index, arg) in args.iter().enumerate() {{\n        let param = &{}[index];\n        match arg {{\n            Some(arg) => builder = apply_{function}(builder, index, arg).map_err(|error| format!(\"`{{}}` {{error}}\", param.name))?,\n            None if param.required => return Err(format!(\"`{{}}` is required\", param.name)),\n            None => {{}}\n        }}\n    }}\n    Ok(builder)\n}}\n",
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
            "fn oneof_{function}_{oneof}(builder: {builder}, arg: &Arg) -> Result<{builder}, String> {{"
        )
        .unwrap();
        emit_dispatch(code, model, &node, "builder", &format!("`{}`", param.name));
        code.push_str("}\n\n");
        // The members that a string names.
        let names_edge = (model.edges.iter()).any(|edge| {
            matches!(&edge.set, Set::Names(name, index) if *name == message.name && index == oneof)
        });
        if !names_edge {
            continue;
        }
        let named: Vec<&str> = (members.iter())
            .filter(|(_, shape)| matches!(shape, Shape::Message(_)))
            .map(|(member, _)| member.name())
            .collect();
        writeln!(
            code,
            "fn names_{function}_{oneof}(builder: {builder}, name: &str) -> Result<{builder}, String> {{\n    Ok(match name {{"
        )
        .unwrap();
        for (member, shape) in members {
            if let Shape::Message(type_name) = shape {
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
            "        other => return Err(format!(\"takes one of {}, not `{{other}}`\")),\n    }})\n}}\n",
            named.join(", ")
        )
        .unwrap();
    }

    // The conversion of an argument to the builder.
    let node = Node::Message(message.name.clone());
    writeln!(
        code,
        "#[allow(dead_code)]\npub(crate) fn to_{function}(arg: &Arg) -> Result<{builder}, String> {{"
    )
    .unwrap();
    emit_dispatch(
        code,
        model,
        &node,
        "",
        &format!("a `{}`", message.message.name()),
    );
    code.push_str("}\n\n");
    Ok(())
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

/// The body of a function that turns `arg` into the target of `node`, on `base` for a oneof.
fn emit_dispatch(code: &mut String, model: &Model, node: &Node, base: &str, what: &str) {
    let accepted = model.accepted(node);
    code.push_str("    match arg.class() {\n");
    let fragments: Vec<&(Source, Vec<&Edge>)> = (accepted.iter())
        .filter(|(source, _)| matches!(source, Source::Fragment(_)))
        .collect();
    if !fragments.is_empty() {
        code.push_str("        Class::Fragment(fragment) => match fragment.r#type.as_str() {\n");
        for (source, chain) in &fragments {
            let Source::Fragment(name) = source else {
                unreachable!()
            };
            let start = format!(
                "crate::builder::from_fragment::<{}>(fragment).map_err(|error| error.to_string())?",
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
        writeln!(
            code,
            "            other => Err(format!(\"takes {what}, not a `{{}}`\", other.rsplit('.').next().unwrap_or(other))),\n        }},"
        )
        .unwrap();
    }
    for (source, chain) in &accepted {
        let Source::Class(class) = source else {
            continue;
        };
        writeln!(
            code,
            "        Class::{} => Ok({}),",
            class.variant(),
            chain_code(chain, None, base, model)
        )
        .unwrap();
    }
    writeln!(
        code,
        "        other => Err(format!(\"takes {what}, not {{}}\", other.describe())),\n    }}"
    )
    .unwrap();
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
            "{arg}.items()?.iter().map(|item| Ok::<_, String>({})).collect::<Result<Vec<_>, String>>()?",
            conversion(item, "item", model)
        ),
    }
}

/// The short name of a full name.
fn short(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// The function that reads a value of the enum `name` from its lowercase name.
fn emit_enum(code: &mut String, set: &FileDescriptorSet, name: &str) -> Result<(), String> {
    let short_name = short(name);
    let enumeration = (set.file.iter())
        .flat_map(|file| &file.enum_type)
        .find(|e| format!("{PACKAGE}.{}", e.name()) == name)
        .ok_or(format!("`{name}` is an enum"))?;
    let prefix = format!("{}_", snake(short_name).to_uppercase());
    let path = rust_path(name);
    let mut names = Vec::new();
    writeln!(
        code,
        "fn enum_{}(arg: &Arg) -> Result<{path}, String> {{\n    let name = arg.string()?;\n    Ok(match name.as_str() {{",
        snake(short_name)
    )
    .unwrap();
    for value in &enumeration.value {
        if value.number() == 0 {
            continue;
        }
        let suffix = value.name().strip_prefix(&prefix).unwrap_or(value.name());
        let lower = suffix.to_lowercase();
        writeln!(code, "        {lower:?} => {path}::{},", camel(&lower)).unwrap();
        names.push(lower);
    }
    writeln!(
        code,
        "        other => return Err(format!(\"takes {}, not '{{other}}'\")),\n    }})\n}}\n",
        names.join(", ")
    )
    .unwrap();
    Ok(())
}

/// The text of the parameters of the constructor of `message` and what each takes.
fn describe(surface: &mut String, message: &MessageModel, model: &Model) {
    let params: Vec<&str> = message.params.iter().map(|p| p.name.as_str()).collect();
    writeln!(surface, "{}({})", message.function, params.join(", ")).unwrap();
    for param in &message.params {
        let required = if param.required { " (required)" } else { "" };
        let takes = match &param.kind {
            ParamKind::Field(_, Shape::Message(name)) => {
                sources(model, &Node::Message(name.clone()))
            }
            ParamKind::Field(_, shape) => describe_shape(shape),
            ParamKind::Oneof(index, _) => {
                sources(model, &Node::Oneof(message.name.clone(), *index))
            }
        };
        writeln!(surface, "  {}{required}: {takes}", param.name).unwrap();
    }
}

/// The text of what a field of `shape` takes.
fn describe_shape(shape: &Shape) -> String {
    match shape {
        Shape::Bool => "Boolean".into(),
        Shape::Int(_) => "integer".into(),
        Shape::Float(_) => "number".into(),
        Shape::Str => "string".into(),
        Shape::Bytes => "bytes".into(),
        Shape::Enum(name) => format!("enum {}", short(name)),
        Shape::Value => "any constant".into(),
        Shape::Message(name) => short(name).into(),
        Shape::Repeated(item) => format!("array of {}", describe_shape(item)),
    }
}

/// The text of the sources that reach `node`, each with its chain.
fn sources(model: &Model, node: &Node) -> String {
    let mut parts = Vec::new();
    for (source, chain) in model.accepted(node) {
        let mut steps: Vec<String> = Vec::new();
        for edge in &chain {
            let step = match &edge.set {
                Set::Field(field) | Set::Member(field) => field.name().to_string(),
                Set::Names(..) => "a member's name".to_string(),
            };
            steps.push(step);
        }
        let start = match &source {
            Source::Fragment(name) => short(name).to_string(),
            Source::Class(class) => match class {
                Class::Bool => "Boolean".into(),
                Class::Int => "integer".into(),
                Class::Float => "float".into(),
                Class::Str => "string".into(),
                Class::Array => "array".into(),
                Class::Bytes => "bytes".into(),
                Class::Other => "other".into(),
            },
        };
        parts.push(match steps.is_empty() {
            true => start,
            false => format!("{start} -> {}", steps.join(" -> ")),
        });
    }
    parts.join("; ")
}
