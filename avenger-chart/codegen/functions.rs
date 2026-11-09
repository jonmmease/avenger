//! The generation of `functions.rs`, behind the `datafusion` feature: the table of the mark
//! functions, one for each mark of the Cartesian and the polar layers.
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
//! The marks of geo layers have no functions until the constructors give their plots the `Geo`
//! options that a geo plot requires.

use std::{collections::BTreeMap, fmt::Write};

use prost_types::FileDescriptorSet;

use super::{annotations::Annotations, builders::Comments, camel, snake, Messages, KINDS, PACKAGE};

/// The coordinate systems whose marks have functions.
const KINDS_WITH_FUNCTIONS: [&str; 2] = ["Cartesian", "Polar"];

/// The code of `functions.rs`: `MARKS`, a `MarkSpec` for each mark function.
pub(super) fn functions(
    set: &FileDescriptorSet,
    messages: &Messages,
    annotations: &Annotations,
) -> Result<String, String> {
    let comments = Comments::new(set);
    let channel_comments = channel_comments(set);
    let encoding = format!("{PACKAGE}.Encoding");
    let mut names = Vec::new();
    let mut code = String::from("pub(crate) static MARKS: &[MarkSpec] = &[\n");
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
            writeln!(code, "    MarkSpec {{").unwrap();
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
                "        mark: || LayerMark::{kind}({module}::Mark::{}(Default::default())),",
                camel(member.name())
            )
            .unwrap();
            writeln!(code, "        channels: &[").unwrap();
            for (optional, _, index) in channels {
                let field = &mark.field[index];
                writeln!(
                    code,
                    "            ChannelSpec {{ name: {:?}, channel: crate::proto::Channel::{}, required: {}, doc: {} }},",
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
            code.push_str("        ],\n    },\n");
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

/// `comment`, a comment of a `.proto` file, as paragraphs: the lines of each paragraph, which
/// the file wraps, joined by spaces, and the paragraphs by a blank line.
fn clean(comment: &str) -> String {
    let lines: Vec<&str> = comment.lines().map(str::trim).collect();
    let paragraphs: Vec<String> = (lines.split(|line| line.is_empty()))
        .filter(|paragraph| !paragraph.is_empty())
        .map(|paragraph| paragraph.join(" "))
        .collect();
    paragraphs.join("\n\n")
}
