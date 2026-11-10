//! The generation of `mark_functions.rs`, behind the `functions` feature: the table of the mark
//! functions, one for each mark of each layer.
//!
//! The rules, for the mark message `FooMark` of a layer:
//!
//! - the function's name is the message's name without `Mark`, in snake case: `bar` for
//!   `BarMark`, and `polar_bar` for `PolarBarMark`
//! - its channels, which are its parameters, are the `Encoding` fields of the message outside a
//!   oneof: the required channels first, then the others, each in the order of the field numbers
//! - its documentation is the comment of the message, and each channel's is the comment of its
//!   field, or of its value of the `Channel` enum, which a field without a comment of its own
//!   means
//!
//! - its properties, the parameters after its channels, are the parameters of the mark message in
//!   the parameter model, which applies them with its generated `apply_all_` function
//!
//! A mark function also takes `coord`, the options of its plot, last, which the geo marks require.

use std::{collections::BTreeMap, fmt::Write};

use prost_types::FileDescriptorSet;

use super::{
    annotations::Annotations,
    builders::{clean, Comments},
    camel, snake, Messages, KINDS, PACKAGE,
};

/// The coordinate systems whose marks have functions.
const KINDS_WITH_FUNCTIONS: [&str; 3] = ["Cartesian", "Polar", "Geo"];

/// The code of `mark_functions.rs`: `MARKS`, a `Mark` for each mark function.
pub(super) fn mark_functions(
    set: &FileDescriptorSet,
    messages: &Messages,
    annotations: &Annotations,
) -> Result<String, String> {
    let comments = Comments::new(set);
    let channel_comments = channel_comments(set);
    let encoding = format!("{PACKAGE}.Encoding");
    let mut names = Vec::new();
    let mut code = String::from("pub(crate) static MARKS: &[Mark] = &[\n");
    for (_, layer_path, layer) in &messages.all {
        let Some(oneof) = (layer.oneof_decl.iter()).position(|oneof| oneof.name() == "mark") else {
            continue;
        };
        let Some(&(_, kind)) = KINDS
            .iter()
            .find(|(prefix, _)| layer.name().starts_with(prefix))
        else {
            return Err(format!("`{}` names its coordinate system", layer.name()));
        };
        if !KINDS_WITH_FUNCTIONS.contains(&kind) {
            continue;
        }
        let module = format!(
            "{}::{}",
            layer_path.rsplit_once("::").expect("a nested path").0,
            snake(layer.name())
        );
        for member in (layer.field.iter()).filter(|field| field.oneof_index == Some(oneof as i32)) {
            let (mark_name, _, mark) = messages
                .get(member.type_name())
                .ok_or_else(|| format!("`{}.{}` is a message", layer.name(), member.name()))?;
            let base = (mark.name().strip_suffix("Mark"))
                .ok_or_else(|| format!("the mark `{}` ends with `Mark`", mark.name()))?;
            let name = snake(base);
            if names.contains(&name) {
                return Err(format!("two marks take the function name `{name}`"));
            }
            names.push(name.clone());
            let mut channels: Vec<(bool, i32, usize)> = (mark.field.iter().enumerate())
                .filter(|(_, field)| field.type_name() == encoding)
                .filter(|(_, field)| field.oneof_index.is_none() || field.proto3_optional())
                .map(|(index, field)| {
                    let required = annotations.is_required(mark_name, field.name());
                    (!required, field.number(), index)
                })
                .collect();
            channels.sort();
            let doc = |comment: Option<&String>| {
                format!("{:?}", comment.map_or(String::new(), |c| clean(c)))
            };
            writeln!(code, "    Mark {{").unwrap();
            writeln!(code, "        name: {name:?},").unwrap();
            writeln!(code, "        coord: crate::CoordKind::{kind},").unwrap();
            writeln!(
                code,
                "        doc: {},",
                doc(comments.messages.get(mark_name.as_str()))
            )
            .unwrap();
            writeln!(
                code,
                "        mark: || call::LayerMark::{kind}({module}::Mark::{}(Default::default())),",
                camel(member.name())
            )
            .unwrap();
            writeln!(code, "        channels: &[").unwrap();
            for (optional, _, index) in channels {
                let field = &mark.field[index];
                writeln!(
                    code,
                    "            Channel {{ name: {:?}, channel: crate::proto::Channel::{}, required: {}, doc: {} }},",
                    field.name(),
                    camel(field.name()),
                    !optional,
                    doc(comments
                        .fields
                        .get(&(mark_name.clone(), index))
                        .or_else(|| channel_comments.get(field.name())))
                )
                .unwrap();
            }
            code.push_str("        ],\n");
            // The properties, which the parameter model of the mark message gives.
            let function = snake(mark.name());
            writeln!(
                code,
                "        properties: generated::PARAMS_{},",
                function.to_uppercase()
            )
            .unwrap();
            writeln!(
                code,
                "        apply_properties: |mark, parts, args| {{\n            let call::LayerMark::{kind}({module}::Mark::{}(message)) = mark else {{\n                unreachable!(\"the mark of its function\")\n            }};\n            let builder = crate::builder::Builder {{\n                message: std::mem::take(message),\n                parts: std::mem::take(parts),\n            }};\n            let builder = generated::apply_all_{function}(builder, args)?;\n            *message = builder.message;\n            *parts = builder.parts;\n            Ok(())\n        }},",
                camel(member.name())
            )
            .unwrap();
            code.push_str("    },\n");
        }
    }
    code.push_str("];\n");
    Ok(code)
}

/// The comment of each value of the `Channel` enum, by the name of the channel, such as `x`.
fn channel_comments(set: &FileDescriptorSet) -> BTreeMap<String, String> {
    let mut comments = BTreeMap::new();
    for file in &set.file {
        let Some(index) = (file.enum_type.iter()).position(|e| e.name() == "Channel") else {
            continue;
        };
        let values = &file.enum_type[index].value;
        let locations = file.source_code_info.iter().flat_map(|info| &info.location);
        for location in locations {
            if let [5, enumeration, 2, value] = location.path.as_slice() {
                if *enumeration as usize == index && !location.leading_comments().is_empty() {
                    let name = values[*value as usize].name();
                    let name = name.strip_prefix("CHANNEL_").unwrap_or(name).to_lowercase();
                    comments.insert(name, location.leading_comments().to_string());
                }
            }
        }
    }
    comments
}
