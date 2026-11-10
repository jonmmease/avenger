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
//! A mark function also takes the options of its plot that concern no other layer and of its
//! layer (`mark_options.rs`), without an option whose name a field of its mark has, such as the
//! `width` channel of `text`, then `coord`, the options of its plot's coordinate system, last,
//! which the geo marks require.

use std::{collections::BTreeMap, fmt::Write};

use prost_types::FileDescriptorSet;

use super::{
    annotations::Annotations,
    builders::{clean, Comments},
    camel,
    model::Model,
    options::MARK_LAYER,
    parameters::builder,
    snake, Messages, KINDS, PACKAGE,
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
            // The options of the plot and the layer, without those whose names a field of the
            // mark has, such as the `width` channel of `text`, which wins.
            let options: Vec<String> = (PLOT_OPTIONS.iter().chain(MARK_LAYER.1))
                .enumerate()
                .filter(|(_, option)| !mark.field.iter().any(|field| field.name() == **option))
                .map(|(index, _)| format!("generated::PARAMS_MARK_OPTIONS[{index}]"))
                .collect();
            writeln!(code, "        options: &[{}],", options.join(", ")).unwrap();
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

/// The options of a mark function's plot that concern no other layer, which a mark call takes
/// after its properties, in this order, before the options of its layer, `MARK_LAYER`'s.
const PLOT_OPTIONS: [&str; 5] = ["title", "attribution", "tools", "width", "height"];

/// The code of `mark_options.rs`: `PARAMS_MARK_OPTIONS`, the options of every mark function, which
/// are parameters of its plot and of its layer in the parameter model, and
/// `apply_all_mark_options`, which applies their arguments to the builders of the plot and the
/// layer.
pub(super) fn mark_options(model: &Model) -> Result<String, String> {
    let node = |name: &str| {
        let full = format!("{PACKAGE}.{name}");
        model.message(&full).ok_or(format!("`{name}` has a model"))
    };
    let plot = node("Plot")?;
    let layer = node(MARK_LAYER.0)?;
    let mut items = Vec::new();
    let mut arms = Vec::new();
    let options = (PLOT_OPTIONS.iter().map(|name| (plot, *name)))
        .chain(MARK_LAYER.1.iter().map(|name| (layer, *name)));
    for (index, (message, name)) in options.enumerate() {
        let param = (message.params.iter())
            .position(|param| param.name == name)
            .ok_or(format!("`{}` has no parameter `{name}`", message.name))?;
        items.push(format!(
            "PARAMS_{}[{param}]",
            message.function.to_uppercase()
        ));
        let target = match std::ptr::eq(message, plot) {
            true => "plot",
            false => "layer",
        };
        arms.push(format!(
            "            {index} => {target} = apply_{}({target}, {param}, arg).map_err(within)?,",
            message.function
        ));
    }
    let plot_builder = builder(&plot.name);
    let layer_builder = builder(&layer.name);
    let mut code = String::new();
    writeln!(
        code,
        "pub(crate) const PARAMS_MARK_OPTIONS: &[Param] = &[{}];\n",
        items.join(", ")
    )
    .unwrap();
    writeln!(
        code,
        "pub(crate) fn apply_all_mark_options(mut plot: {plot_builder}, mut layer: {layer_builder}, args: &[Option<Arg>]) -> Result<({plot_builder}, {layer_builder}), ArgError> {{\n    for (index, arg) in args.iter().enumerate() {{\n        let Some(arg) = arg else {{\n            continue;\n        }};\n        let within = |error: ArgError| error.within(PARAMS_MARK_OPTIONS[index].name);\n        match index {{\n{}\n            _ => unreachable!(\"an option of a mark call\"),\n        }}\n    }}\n    Ok((plot, layer))\n}}",
        arms.join("\n")
    )
    .unwrap();
    Ok(code)
}
