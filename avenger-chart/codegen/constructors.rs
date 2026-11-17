//! The generation of `constructors.rs`, behind the `functions` feature: the table of the
//! constructors and the function that builds the fragment of each from its arguments, through the
//! code of the parameter model in `parameters.rs`.
//!
//! The constructors are of two kinds:
//!
//! - one for each message of the option tree with fields, except the marks, which builds the
//!   fragment of its message. Its parameters are those of its message, after its first parameter
//!   when its message has shorthands: a parameter `value` that takes every shorthand, or the one
//!   lifted field itself when it is a field of plain values.
//! - one for each oneof of a message of the model that is a parameter of its own, which builds the
//!   fragment of its message with only the oneof set, under the type of the oneof, such as
//!   `avenger.chart.v1.Scale.type`. Its parameters are a first parameter `value` that takes the
//!   values of the oneof, then its members with fields, of which a call gives one.
//!
//! A constructor's name is the name of its message in snake case, such as `log_scale`, or of its
//! message and its oneof, such as `scale.type`. The names are unique and stand for the
//! constructors in a plan: a binding names a constructor after the place that it fills, such as
//! `log` in `type => log(base => 2)`.

use std::{collections::BTreeSet, fmt::Write};

use super::{
    builders::Comments,
    model::{Chain, MessageModel, Model, ParamKind, Set, Shape},
    parameters::{
        builder, conversion, doc, field_accepts, member_comment, oneof_constructor, oneof_fragment,
        param_doc, params_const, shorthand_const,
    },
};

/// The code of `constructors.rs`, where `constructors` names the messages with constructors.
pub(super) fn constructors(
    model: &Model,
    comments: &Comments,
    constructors: &[String],
    lifts: &[String],
) -> Result<String, String> {
    let mut code = String::new();
    let mut table = Vec::new();
    let mut enums = BTreeSet::new();
    for name in constructors {
        let message = model.message(name).ok_or(format!("`{name}` has a model"))?;
        let function = &message.function;
        let params = format!("CTOR_{}", function.to_uppercase());
        emit_message_constructor(&mut code, message, &params)?;
        // The parameters that a first parameter of its own fills.
        let mut fills: Vec<String> = Vec::new();
        if message.merged.is_none() {
            for index in message.filled.iter().flatten() {
                let name = format!("{:?}", message.params[*index].name);
                if !fills.contains(&name) {
                    fills.push(name);
                }
            }
        }
        table.push(format!(
            "    Constructor {{ name: {function:?}, message: {:?}, oneof: None, doc: {:?}, params: {params}, fills: &[{}], build: build_{function}, convert: convert_{function} }},",
            name.trim_start_matches('.'),
            doc(comments.messages.get(name)),
            fills.join(", "),
        ));
    }
    for message in &model.messages {
        for param in &message.params {
            let ParamKind::Oneof(oneof, members) = &param.kind else {
                continue;
            };
            let function = format!("{}_oneof_{oneof}", message.function);
            let params = format!("CTOR_{}", function.to_uppercase());
            emit_oneof_constructor(
                &mut code, model, comments, message, *oneof, members, &function, &params,
                &mut enums,
            )?;
            table.push(format!(
                "    Constructor {{ name: {:?}, message: {:?}, oneof: Some({:?}), doc: {:?}, params: {params}, fills: &[], build: build_{function}, convert: convert_{function} }},",
                oneof_constructor(message, *oneof),
                message.name.trim_start_matches('.'),
                param.name,
                doc(param_doc(message, &param.kind, comments).as_ref()),
            ));
        }
    }
    let mut out = String::from("pub(crate) static CONSTRUCTORS: &[Constructor] = &[\n");
    for line in table {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("];\n\n");
    let lifts: Vec<String> = lifts.iter().map(|lift| format!("{lift:?}")).collect();
    writeln!(
        out,
        "pub(crate) const LIFTS: &[&str] = &[{}];\n",
        lifts.join(", ")
    )
    .unwrap();
    out.push_str(&code);
    Ok(out)
}

/// The words of the shorthands of `message`, for the documentation of its first parameter.
fn shorthand_doc(message: &MessageModel) -> String {
    let parts: Vec<String> = (message.shorthand.iter())
        .map(|chain| format!("{} fills {}", class_word(chain), chain_target(chain)))
        .collect();
    format!(
        "The shorthand of the constructor, which the value of its parameter alone takes: {}.",
        parts.join(", ")
    )
}

/// A class of a chain in words.
fn class_word(chain: &Chain) -> &'static str {
    use super::model::Class;
    match chain.class {
        Class::Bool => "a Boolean",
        Class::Int => "an integer",
        Class::Float => "a number",
        Class::Str => "a string",
        Class::Array => "an array",
        Class::Bytes => "bytes",
        Class::Struct => "a struct",
        Class::Other => "another constant",
    }
}

/// What a chain sets, in words: the field or the member of its last step, or a member that the
/// string names.
fn chain_target(chain: &Chain) -> String {
    match &chain.edges.last().expect("a step").set {
        Set::Field(field) | Set::Member(field) => format!("`{}`", field.name()),
        Set::Names(..) => "the member that it names".to_string(),
    }
}

/// The parameters and the build of the constructor of `message`.
fn emit_message_constructor(
    code: &mut String,
    message: &MessageModel,
    params: &str,
) -> Result<(), String> {
    let function = &message.function;
    let own = params_const(message);
    // The parameters of the constructor, each with its index among the message's parameters, or
    // `None` for the parameter of the shorthands.
    let mut items: Vec<(String, Option<usize>)> = Vec::new();
    match (message.has_shorthand(), message.merged) {
        (true, Some(index)) => {
            items.push((format!("Param {{ positional: true, ..{own}[{index}] }}"), Some(index)));
        }
        (true, None) => items.push((
            format!(
                "Param {{ name: \"value\", doc: {:?}, required: false, accepts: {}, constructor: None, positional: true }}",
                shorthand_doc(message),
                shorthand_const(message)
            ),
            None,
        )),
        (false, _) => {}
    }
    for index in 0..message.params.len() {
        if message.merged == Some(index) && message.has_shorthand() {
            continue;
        }
        items.push((format!("{own}[{index}]"), Some(index)));
    }
    let lines: Vec<String> = items
        .iter()
        .map(|(item, _)| format!("    {item},"))
        .collect();
    writeln!(
        code,
        "pub(crate) const {params}: &[Param] = &[\n{}\n];\n",
        lines.join("\n")
    )
    .unwrap();
    let map: Vec<String> = (items.iter())
        .map(|(_, index)| index.map_or("usize::MAX".to_string(), |index| index.to_string()))
        .collect();
    let lift = match message.has_shorthand() && message.merged.is_none() {
        true => format!(
            "    let (builder, filled) = match shorthand {{\n        Some(arg) => lift_{function}(builder, arg).map_err(|error| error.within(\"value\"))?,\n        None => (builder, None),\n    }};\n"
        ),
        false => "    let _ = shorthand;\n    let filled = None;\n".to_string(),
    };
    writeln!(
        code,
        "fn convert_{function}(arg: &Arg) -> Result<crate::proto::Fragment, ArgError> {{\n    to_{function}(arg)?.to_fragment().map_err(ArgError::invalid)\n}}\n\nfn build_{function}(args: &[Option<Arg>]) -> Result<crate::proto::Fragment, ArgError> {{\n    const MAP: &[usize] = &[{}];\n    let mut ordered: Vec<Option<Arg>> = vec![None; {own}.len()];\n    let mut shorthand = None;\n    for (index, arg) in args.iter().enumerate() {{\n        match MAP[index] {{\n            usize::MAX => shorthand = arg.as_ref(),\n            own => ordered[own] = arg.clone(),\n        }}\n    }}\n    let builder = {}::default();\n{lift}    let builder = apply_given_{function}(builder, &ordered, filled)?;\n    builder.to_fragment().map_err(ArgError::invalid)\n}}\n",
        map.join(", "),
        builder(&message.name)
    )
    .unwrap();
    Ok(())
}

/// The parameters and the build of the constructor of the oneof `oneof` of `message`.
#[allow(clippy::too_many_arguments)]
fn emit_oneof_constructor(
    code: &mut String,
    model: &Model,
    comments: &Comments,
    message: &MessageModel,
    oneof: usize,
    members: &[(&prost_types::FieldDescriptorProto, Shape)],
    function: &str,
    params: &str,
    enums: &mut BTreeSet<String>,
) -> Result<(), String> {
    let values = !model.oneof_values(&message.name, oneof).is_empty();
    let builder = builder(&message.name);
    let mut items = Vec::new();
    let mut arms = Vec::new();
    if values {
        items.push(format!(
            "Param {{ name: \"value\", doc: \"A value of the choice: a value of a member, or the name of a member without required content.\", required: false, accepts: VALUES_{}_{oneof}, constructor: None, positional: true }}",
            message.function.to_uppercase()
        ));
        arms.push(format!(
            "            0 => oneof_{}_{oneof}(builder, arg).map_err(within)?,",
            message.function
        ));
    }
    for (member, shape) in members {
        // A member without fields has no constructor, and the first parameter names it.
        if let Shape::Message(name) = shape {
            let fieldless = model
                .message(name)
                .is_none_or(|member| member.message.field.is_empty());
            if fieldless {
                continue;
            }
        }
        let comment = member_comment(message, member, comments).unwrap_or_default();
        let constructor = matches!(shape, Shape::Message(_)).then(|| member.name().to_string());
        items.push(format!(
            "Param {{ name: {:?}, doc: {comment:?}, required: false, accepts: {}, constructor: {constructor:?}, positional: false }}",
            member.name(),
            field_accepts(shape, model, enums)
        ));
        let setter = match shape {
            Shape::Message(_) => format!("with_{}", member.name()),
            _ => super::builders::method(member.name()),
        };
        arms.push(format!(
            "            {} => {{\n                let apply = |builder: {builder}| -> Result<{builder}, ArgError> {{ Ok(builder.{setter}({})) }};\n                apply(builder).map_err(within)?\n            }}",
            arms.len(),
            conversion(shape, "arg", model)
        ));
    }
    let lines: Vec<String> = items.iter().map(|item| format!("    {item},")).collect();
    writeln!(
        code,
        "pub(crate) const {params}: &[Param] = &[\n{}\n];\n",
        lines.join("\n")
    )
    .unwrap();
    writeln!(
        code,
        "fn convert_{function}(arg: &Arg) -> Result<crate::proto::Fragment, ArgError> {{\n    let builder = oneof_{}_{oneof}({builder}::default(), arg)?;\n    let mut fragment = builder.to_fragment().map_err(ArgError::invalid)?;\n    fragment.r#type = {:?}.to_string();\n    Ok(fragment)\n}}\n",
        message.function,
        oneof_fragment(message, oneof)
    )
    .unwrap();
    writeln!(
        code,
        "fn build_{function}(args: &[Option<Arg>]) -> Result<crate::proto::Fragment, ArgError> {{\n    let given: Vec<&'static str> = (args.iter().enumerate()).filter(|(_, arg)| arg.is_some()).map(|(index, _)| {params}[index].name).collect();\n    if let [first, second, ..] = given[..] {{\n        return Err(ArgError::exclusive(first).within(second));\n    }}\n    let mut builder = {builder}::default();\n    for (index, arg) in args.iter().enumerate() {{\n        let Some(arg) = arg else {{\n            continue;\n        }};\n        let within = |error: ArgError| error.within({params}[index].name);\n        builder = match index {{\n{}\n            _ => unreachable!(\"a parameter of the constructor of a oneof\"),\n        }};\n    }}\n    let mut fragment = builder.to_fragment().map_err(ArgError::invalid)?;\n    fragment.r#type = {:?}.to_string();\n    Ok(fragment)\n}}\n",
        arms.join("\n"),
        oneof_fragment(message, oneof)
    )
    .unwrap();
    Ok(())
}
