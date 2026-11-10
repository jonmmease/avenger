//! Generates the prost messages of package `avenger.chart.v1` from the files in
//! `proto/avenger/chart/v1/` with the vendored `protoc`, so that a build needs no system `protoc`.
//!
//! It also reads the descriptors of the messages and generates these files:
//!
//! - `walk.rs`: impls of `VisitValues` for each message that holds a `ValueRef`, of `VisitTables`
//!   for each message that holds a `TableRef`, of `VisitEnums` for each message that holds an enum
//!   field, and of `VisitRequired` for each message that holds a field or a oneof that the options
//!   of `annotation.proto` require, directly or through their fields, and `enum_error`, which
//!   checks that the value of an enum field is a value that its enum declares.
//! - `channels.rs`: `takes_scale`, from the `channel_scale` option of the values of `Channel`.
//! - `marks.rs`: for each mark, the member of the `mark` oneof of a layer, its channels and their
//!   encodings, and for each `mark` oneof, an impl of `MarkMessage`.
//! - `merge.rs`: an impl of `MergeFields` for each message that the specs of a shared scale merge
//!   field by field: the axes, the legends, the options of the scale types, and the fits.
//! - `builders.rs` and `prelude.rs`: the builders of the messages, as `codegen/builders.rs`
//!   describes.
//!
//! It fails when a field of a mark breaks a law of the format: an `Encoding` field has the name of
//! a `Channel` value, lowercase and without the prefix, and the number 100 plus that value, and
//! every other field, a property, has a number below 100.

use std::{collections::BTreeSet, fmt::Write, path::PathBuf};

use prost::Message;

#[path = "codegen/annotations.rs"]
mod annotations;
#[path = "codegen/builders.rs"]
mod builders;
#[path = "codegen/compositions.rs"]
mod compositions;
#[path = "codegen/constructors.rs"]
mod constructors;
#[path = "codegen/mark_functions.rs"]
mod mark_functions;
#[path = "codegen/model.rs"]
mod model;
#[path = "codegen/options.rs"]
mod options;
#[path = "codegen/parameters.rs"]
mod parameters;
use prost_types::{
    field_descriptor_proto::{Label, Type},
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorSet,
};

const PROTOS: [&str; 26] = [
    "annotation",
    "attribution",
    "axis",
    "cartesian",
    "channel",
    "chart",
    "element",
    "encoding",
    "facet",
    "geo",
    "tool",
    "legend",
    "mark",
    "option",
    "part",
    "partition",
    "plot",
    "polar",
    "resolve",
    "route",
    "scale",
    "selection",
    "side",
    "style",
    "value",
    "widget",
];

const PACKAGE: &str = ".avenger.chart.v1";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto");
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    let descriptors = out.join("descriptors.bin");
    let files = PROTOS.map(|name| format!("proto/avenger/chart/v1/{name}.proto"));
    prost_build::Config::new()
        .protoc_executable(protoc_bin_vendored::protoc_bin_path()?)
        .file_descriptor_set_path(&descriptors)
        // Every `bytes` field is a `bytes::Bytes`, which decodes as a slice of its input, so the
        // Arrow payloads of a chart decode without a copy.
        .bytes(["."])
        // Each message implements `prost::Name`, whose full name a fragment records.
        .enable_type_names()
        .compile_protos(
            &files,
            &[PathBuf::from("proto"), protoc_bin_vendored::include_path()?],
        )?;

    let mut set = FileDescriptorSet::decode(std::fs::read(&descriptors)?.as_slice())?;
    // The set also holds the files that the package imports, such as `descriptor.proto`.
    set.file
        .retain(|file| format!(".{}", file.package()) == PACKAGE);
    let messages = Messages::new(&set);
    let mut walk = String::new();
    // Every message with a builder visits its values, as an empty walk when it holds none, so a
    // fragment of any message orders its values.
    for (target, visitor, method, every) in [
        ("ValueRef", "VisitValues", "visit_values", true),
        ("TableRef", "VisitTables", "visit_tables", false),
    ] {
        messages.visitors(
            &mut walk,
            &format!("{PACKAGE}.{target}"),
            visitor,
            method,
            every,
        );
    }
    messages.enum_visitors(&mut walk);
    enum_error(&mut walk, &set);
    let annotations = annotations::Annotations::read(&std::fs::read(&descriptors)?)?;
    check_annotations(&annotations)?;
    let tree = options::option_tree(&messages);
    options::check(&messages, &tree)?;
    let model = model::Model::new(
        &messages,
        &annotations,
        &model::parameter_messages(&tree),
        &options::excluded(&messages),
    )?;
    // The messages with constructors: those of the tree with fields, except the marks and their
    // layer, whose mark functions take their parameters, and the composition nodes, whose
    // compositions take them.
    let mut functions_own = options::marks(&messages);
    functions_own
        .extend((options::COMPOSITION_NODES.iter()).map(|(node, _)| format!("{PACKAGE}.{node}")));
    functions_own.insert(format!("{PACKAGE}.{}", options::MARK_LAYER.0));
    let constructors: Vec<&model::MessageModel> = (model.messages.iter())
        .filter(|message| tree.contains(&message.name) && !functions_own.contains(&message.name))
        .filter(|message| !message.message.field.is_empty())
        .collect();
    model::check_names(&constructors)?;
    let constructor_names: Vec<String> = constructors.iter().map(|m| m.name.clone()).collect();
    annotations::required_visitors(&mut walk, &set, &messages, &annotations)?;
    std::fs::write(out.join("walk.rs"), walk)?;
    std::fs::write(out.join("channels.rs"), annotations::channels(&annotations))?;
    std::fs::write(out.join("marks.rs"), marks(&set, &messages)?)?;
    std::fs::write(out.join("merge.rs"), messages.merges())?;
    std::fs::write(
        out.join("builders.rs"),
        builders::builders(&set, &messages, &annotations),
    )?;
    std::fs::write(out.join("prelude.rs"), builders::prelude(&set, &messages))?;
    if std::env::var_os("CARGO_FEATURE_FUNCTIONS").is_some() {
        let comments = builders::Comments::new(&set);
        std::fs::write(
            out.join("parameters.rs"),
            parameters::parameters(&set, &model, &comments)?,
        )?;
        std::fs::write(
            out.join("constructors.rs"),
            constructors::constructors(&model, &comments, &constructor_names)?,
        )?;
        std::fs::write(
            out.join("compositions.rs"),
            compositions::compositions(&model)?,
        )?;
        std::fs::write(
            out.join("mark_options.rs"),
            mark_functions::mark_options(&model)?,
        )?;
        std::fs::write(
            out.join("mark_functions.rs"),
            mark_functions::mark_functions(&set, &messages, &annotations)?,
        )?;
    }
    Ok(())
}

/// The kinds of coordinate systems, by the prefix of the names of their layer messages.
const KINDS: [(&str, &str); 3] = [
    ("Cartesian", "Cartesian"),
    ("Polar", "Polar"),
    ("Geo", "Geo"),
];

/// How a mark holds the encoding of a channel: in a field of its own, or as a member of a oneof
/// of the mark, as `PolarTextMark.turn` holds `angle`.
enum Holder {
    Field(String),
    Oneof { field: String, path: String },
}

/// The code of `marks.rs`, after a check of the laws of the fields of each mark.
fn marks(set: &FileDescriptorSet, messages: &Messages) -> Result<String, String> {
    let channels: Vec<(String, i32)> = (set.file.iter())
        .flat_map(|file| &file.enum_type)
        .find(|enumeration| enumeration.name() == "Channel")
        .ok_or("the package has a `Channel` enum")?
        .value
        .iter()
        .filter(|value| value.number() != 0)
        .map(|value| {
            let name = value
                .name()
                .strip_prefix("CHANNEL_")
                .unwrap_or(value.name());
            (name.to_lowercase(), value.number())
        })
        .collect();
    // The layers: the messages with a `mark` oneof.
    let layers: Vec<(&String, &String, &DescriptorProto, usize)> = (messages.all.iter())
        .filter_map(|(name, path, message)| {
            let oneof = (message.oneof_decl.iter()).position(|oneof| oneof.name() == "mark")?;
            Some((name, path, *message, oneof))
        })
        .collect();
    if layers.is_empty() {
        return Err("the package has layers, messages with a `mark` oneof".to_string());
    }

    let mut code = String::new();
    for (_, layer_path, layer, oneof) in layers {
        let &(_, kind) = KINDS
            .iter()
            .find(|(prefix, _)| layer.name().starts_with(prefix))
            .ok_or_else(|| format!("`{}` names its coordinate system", layer.name()))?;
        let module = format!(
            "{}::{}",
            layer_path.rsplit_once("::").expect("a nested path").0,
            snake(layer.name())
        );
        let members: Vec<&FieldDescriptorProto> = (layer.field.iter())
            .filter(|field| field.oneof_index == Some(oneof as i32))
            .collect();
        let mut arms = Vec::new();
        for member in members {
            let (_, path, mark) = messages
                .get(member.type_name())
                .ok_or_else(|| format!("`{}.{}` is a message", layer.name(), member.name()))?;
            let mut taken: Vec<(String, Holder)> = Vec::new();
            for field in &mark.field {
                let at = format!("`{}.{}`", mark.name(), field.name());
                if field.r#type() == Type::Message
                    && field.type_name() == format!("{PACKAGE}.Encoding")
                {
                    let (name, value) = (channels.iter())
                        .find(|(name, _)| name == field.name())
                        .ok_or_else(|| {
                            format!("the encoding field {at} has the name of a channel")
                        })?;
                    if field.number() != 100 + value {
                        return Err(format!(
                            "the encoding field {at} has the number {}, 100 plus its channel's value",
                            100 + value
                        ));
                    }
                    let holder = match field.oneof_index {
                        Some(index) if !field.proto3_optional() => {
                            let oneof = mark.oneof_decl[index as usize].name();
                            Holder::Oneof {
                                field: ident(oneof),
                                path: format!(
                                    "{}::{}::{}",
                                    path.rsplit_once("::").expect("a nested path").0,
                                    snake(mark.name()),
                                    camel(oneof)
                                ),
                            }
                        }
                        _ => Holder::Field(ident(field.name())),
                    };
                    taken.push((name.clone(), holder));
                } else if field.number() >= 100 {
                    return Err(format!("the property {at} has a number below 100"));
                }
            }
            write_mark(&mut code, path, &taken);
            let variant = camel(member.name());
            writeln!(
                code,
                "impl From<{path}> for {module}::Mark {{\n    fn from(mark: {path}) -> Self {{\n        Self::{variant}(mark)\n    }}\n}}\n"
            )
            .unwrap();
            arms.push((variant, member.name().to_string(), path.clone()));
        }

        // One match over the members of the `mark` oneof for each method of `MarkMessage`.
        let method =
            |code: &mut String, signature: &str, arm: &dyn Fn(&str, &str, &str) -> String| {
                writeln!(code, "    fn {signature} {{\n        match self {{").unwrap();
                for (variant, name, path) in &arms {
                    writeln!(code, "            {},", arm(variant, name, path)).unwrap();
                }
                code.push_str("        }\n    }\n");
            };
        writeln!(code, "impl MarkMessage for {module}::Mark {{").unwrap();
        writeln!(
            code,
            "    fn kind(&self) -> CoordKind {{\n        CoordKind::{kind}\n    }}"
        )
        .unwrap();
        method(
            &mut code,
            "mark_type(&self) -> MarkType",
            &|variant, _, _| format!("Self::{variant}(_) => MarkType::{variant}"),
        );
        method(
            &mut code,
            "name(&self) -> &'static str",
            &|variant, name, _| format!("Self::{variant}(_) => \"{name}\""),
        );
        method(
            &mut code,
            "channels(&self) -> &'static [Channel]",
            &|variant, _, path| format!("Self::{variant}(_) => {path}::CHANNELS"),
        );
        method(
            &mut code,
            "encoding(&self, channel: Channel) -> Option<&Encoding>",
            &|variant, _, _| format!("Self::{variant}(mark) => mark.encoding(channel)"),
        );
        method(
            &mut code,
            "encoding_mut(&mut self, channel: Channel) -> Option<&mut Encoding>",
            &|variant, _, _| format!("Self::{variant}(mark) => mark.encoding_mut(channel)"),
        );
        method(
            &mut code,
            "encoding_slot(&mut self, channel: Channel) -> Option<&mut Option<Encoding>>",
            &|variant, _, _| format!("Self::{variant}(mark) => mark.encoding_slot(channel)"),
        );
        code.push_str("}\n\n");
    }

    writeln!(code, "impl Channel {{").unwrap();
    writeln!(
        code,
        "    /// The name of the field of the channel in a mark, such as `stroke_dash`."
    )
    .unwrap();
    writeln!(
        code,
        "    pub fn field_name(self) -> &'static str {{\n        match self {{"
    )
    .unwrap();
    writeln!(code, "            Self::Unspecified => \"\",").unwrap();
    for (name, _) in &channels {
        writeln!(code, "            Self::{} => \"{name}\",", camel(name)).unwrap();
    }
    writeln!(code, "        }}\n    }}\n}}").unwrap();
    Ok(code)
}

/// The channels and the encodings of the mark at `path`.
fn write_mark(code: &mut String, path: &str, taken: &[(String, Holder)]) {
    let variants: Vec<String> = taken.iter().map(|(name, _)| camel(name)).collect();
    writeln!(code, "impl {path} {{").unwrap();
    writeln!(
        code,
        "    /// The channels that the mark takes, in the order of `Channel`."
    )
    .unwrap();
    writeln!(
        code,
        "    pub const CHANNELS: &'static [Channel] = &[{}];",
        (variants.iter())
            .map(|variant| format!("Channel::{variant}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
    .unwrap();
    for (signature, reference) in [
        (
            "encoding(&self, channel: Channel) -> Option<&Encoding>",
            "&",
        ),
        (
            "encoding_mut(&mut self, channel: Channel) -> Option<&mut Encoding>",
            "&mut ",
        ),
    ] {
        let doc = match reference {
            "&" => "The encoding of `channel`, or `None` when the mark does not encode it.",
            _ => {
                "The encoding of `channel`, to change, or `None` when the mark does not encode it."
            }
        };
        writeln!(
            code,
            "    /// {doc}\n    pub fn {signature} {{\n        match channel {{"
        )
        .unwrap();
        for ((_, holder), variant) in taken.iter().zip(&variants) {
            let arm = match holder {
                Holder::Field(field) => match reference {
                    "&" => format!("self.{field}.as_ref()"),
                    _ => format!("self.{field}.as_mut()"),
                },
                Holder::Oneof { field, path } => format!(
                    "match {reference}self.{field} {{ Some({path}::{variant}(encoding)) => Some(encoding), _ => None }}"
                ),
            };
            writeln!(code, "            Channel::{variant} => {arm},").unwrap();
        }
        writeln!(code, "            _ => None,\n        }}\n    }}").unwrap();
    }
    writeln!(
        code,
        "    /// The field of the encoding of `channel`, to set, or `None` when the mark does not take the\n    /// channel in a field of its own.\n    pub fn encoding_slot(&mut self, channel: Channel) -> Option<&mut Option<Encoding>> {{\n        match channel {{"
    )
    .unwrap();
    for ((_, holder), variant) in taken.iter().zip(&variants) {
        if let Holder::Field(field) = holder {
            writeln!(
                code,
                "            Channel::{variant} => Some(&mut self.{field}),"
            )
            .unwrap();
        }
    }
    writeln!(code, "            _ => None,\n        }}\n    }}").unwrap();
    code.push_str("}\n\n");
}

/// Fails unless the options of `annotation.proto` are on the fields that hold them, one of each
/// kind, so a change to how the options decode cannot drop them all.
fn check_annotations(annotations: &annotations::Annotations) -> Result<(), String> {
    let field = |message: &str, field: &str| (format!("{PACKAGE}.{message}"), field.to_string());
    let checks = [
        (
            annotations
                .required
                .contains(&field("ConditionalValue", "predicate")),
            "`required` on `ConditionalValue.predicate`",
        ),
        (
            annotations.required_oneof.contains(&field("Node", "node")),
            "`required_oneof` on `Node.node`",
        ),
        (
            annotations.lift.contains(&field("Scale", "domain")),
            "`lift` on `Scale.domain`",
        ),
        (
            annotations.channel_scale.contains_key("CHANNEL_X"),
            "`channel_scale` on `CHANNEL_X`",
        ),
        (
            annotations.positions.contains("CHANNEL_X"),
            "`position` on `CHANNEL_X`",
        ),
    ];
    match checks.iter().find(|(found, _)| !found) {
        Some((_, option)) => Err(format!("the descriptors hold the option {option}")),
        None => Ok(()),
    }
}

/// The code of `enum_error`, which takes the full name of an enum and a value of a field of it,
/// and returns why the value is not valid, or `None` for a value that the enum declares.
fn enum_error(code: &mut String, set: &FileDescriptorSet) {
    let enums: Vec<(String, &EnumDescriptorProto)> = (set.file.iter())
        .flat_map(|file| &file.enum_type)
        .map(|enumeration| (format!("{PACKAGE}.{}", enumeration.name()), enumeration))
        .collect();
    code.push_str(
        "/// Why `value` is not a valid value of a field of the enum named `name`, or `None`.\n",
    );
    code.push_str("pub(crate) fn enum_error(name: &str, value: i32) -> Option<&'static str> {\n");
    code.push_str("    match name {\n");
    for (name, enumeration) in &enums {
        let mut declared: Vec<i32> = (enumeration.value.iter())
            .map(|value| value.number())
            .collect();
        declared.sort_unstable();
        // The declared values as patterns, with each run of consecutive values as a range.
        let mut numbers: Vec<String> = Vec::new();
        for run in declared.chunk_by(|a, b| b - a == 1) {
            numbers.push(match run {
                [one] => one.to_string(),
                [first, .., last] => format!("{first}..={last}"),
                [] => unreachable!("a chunk is not empty"),
            });
        }
        let short = enumeration.name();
        writeln!(code, "        \"{name}\" => match value {{").unwrap();
        writeln!(code, "            {} => None,", numbers.join(" | ")).unwrap();
        writeln!(
            code,
            "            _ => Some(\"is not a value of `{short}`\"),\n        }},"
        )
        .unwrap();
    }
    code.push_str("        _ => None,\n    }\n}\n\n");
}

/// The messages of the package, each with its full name and its Rust path.
struct Messages<'a> {
    all: Vec<(String, String, &'a DescriptorProto)>,
}

impl<'a> Messages<'a> {
    fn new(set: &'a FileDescriptorSet) -> Self {
        let mut all = Vec::new();
        for file in &set.file {
            for message in &file.message_type {
                Self::collect(&mut all, PACKAGE, "crate::proto", message);
            }
        }
        Self { all }
    }

    fn collect(
        all: &mut Vec<(String, String, &'a DescriptorProto)>,
        scope: &str,
        module: &str,
        message: &'a DescriptorProto,
    ) {
        let name = message.name();
        all.push((
            format!("{scope}.{name}"),
            format!("{module}::{name}"),
            message,
        ));
        for nested in &message.nested_type {
            Self::collect(
                all,
                &format!("{scope}.{name}"),
                &format!("{module}::{}", snake(name)),
                nested,
            );
        }
    }

    fn get(&self, name: &str) -> Option<&(String, String, &'a DescriptorProto)> {
        self.all.iter().find(|(full, _, _)| full == name)
    }

    /// The messages that hold a `target`, directly or through their fields.
    fn holders(&self, target: &str) -> BTreeSet<String> {
        let mut holders = BTreeSet::new();
        loop {
            let before = holders.len();
            for (name, _, message) in &self.all {
                if message
                    .field
                    .iter()
                    .any(|field| field.type_name() == target || holders.contains(field.type_name()))
                {
                    holders.insert(name.clone());
                }
            }
            if holders.len() == before {
                return holders;
            }
        }
    }

    /// The messages that the specs of a shared scale merge field by field: the axes, the legends,
    /// the messages of the scale types, and the fits, with the messages that they hold.
    fn merged(&self) -> BTreeSet<String> {
        let mut merged: BTreeSet<String> = ["Axis", "Legend", "DomainFit", "Widget"]
            .iter()
            .map(|name| format!("{PACKAGE}.{name}"))
            .collect();
        if let Some((_, _, spec)) = self.get(&format!("{PACKAGE}.Scale")) {
            for field in &spec.field {
                // `merge.rs` merges the options of custom scales by name, because they are keyed.
                if field.oneof_index.is_some()
                    && field.r#type() == Type::Message
                    && field.type_name() != format!("{PACKAGE}.CustomScale")
                {
                    merged.insert(field.type_name().to_string());
                }
            }
        }
        loop {
            let before = merged.len();
            for (name, _, message) in &self.all {
                if merged.contains(name) {
                    for field in &message.field {
                        if field.r#type() == Type::Message
                            && field.type_name() != format!("{PACKAGE}.ValueRef")
                        {
                            merged.insert(field.type_name().to_string());
                        }
                    }
                }
            }
            if merged.len() == before {
                return merged;
            }
        }
    }

    /// An impl of `MergeFields` for each message of `merged`. A field that one message sets and
    /// the other does not takes the set value, and a field that both set must be equal: an
    /// `optional` field is set when it is `Some`, a plain string when it is not empty, an enum when
    /// it is not its zero value, a message when it is `Some`, whose fields merge in turn, a oneof
    /// when it is `Some`, whose members must be of one kind, and a repeated field when it is not
    /// empty, except that the `classes` of two messages join. A `ValueRef` is equal to another that
    /// refers to an equal value.
    fn merges(&self) -> String {
        let merged = self.merged();
        let value_ref = format!("{PACKAGE}.ValueRef");
        let mut code = String::new();
        for (name, path, message) in &self.all {
            if !merged.contains(name) {
                continue;
            }
            let module = path.rsplit_once("::").expect("a nested path").0.to_string()
                + "::"
                + &snake(message.name());
            let mut body = String::new();
            let mut oneofs_done = BTreeSet::new();
            for field in &message.field {
                let field_ident = ident(field.name());
                let at = format!("&format!(\"{{path}}.{}\")", field.name());
                match field.oneof_index {
                    Some(oneof) if !field.proto3_optional() => {
                        if !oneofs_done.insert(oneof) {
                            continue;
                        }
                        let oneof_name = message.oneof_decl[oneof as usize].name();
                        let oneof_ident = ident(oneof_name);
                        let members: Vec<_> = (message.field.iter())
                            .filter(|member| member.oneof_index == Some(oneof))
                            .collect();
                        writeln!(
                            body,
                            "        match (&mut self.{oneof_ident}, &other.{oneof_ident}) {{"
                        )
                        .unwrap();
                        body.push_str("            (_, None) => {}\n");
                        writeln!(
                            body,
                            "            (merged @ None, Some(other)) => *merged = Some(other.clone()),"
                        )
                        .unwrap();
                        for member in members {
                            let variant = format!(
                                "{module}::{}::{}",
                                camel(oneof_name),
                                camel(member.name())
                            );
                            let member_at = format!("&format!(\"{{path}}.{}\")", member.name());
                            let arm = if member.r#type() == Type::Message
                                && member.type_name() == value_ref
                            {
                                format!("if !same(*merged, *other) {{ return Err(differ({member_at})); }}")
                            } else if member.r#type() == Type::Message
                                && merged.contains(member.type_name())
                            {
                                format!("merged.merge_fields(other, same, {member_at})?")
                            } else {
                                format!("if merged != other {{ return Err(differ({member_at})); }}")
                            };
                            writeln!(
                                body,
                                "            (Some({variant}(merged)), Some({variant}(other))) => {{ {arm} }}"
                            )
                            .unwrap();
                        }
                        writeln!(
                            body,
                            "            _ => return Err(differ(&format!(\"{{path}}.{oneof_name}\"))),\n        }}"
                        )
                        .unwrap();
                    }
                    _ => {
                        let call = if field.label() == Label::Repeated && field.name() == "classes"
                        {
                            "classes".to_string()
                        } else if field.label() == Label::Repeated {
                            "repeated".to_string()
                        } else if field.r#type() == Type::Message && field.type_name() == value_ref
                        {
                            "reference".to_string()
                        } else if field.r#type() == Type::Message
                            && merged.contains(field.type_name())
                        {
                            writeln!(
                                body,
                                "        match (&mut self.{field_ident}, &other.{field_ident}) {{\n            (_, None) => {{}}\n            (merged @ None, Some(other)) => *merged = Some(other.clone()),\n            (Some(merged), Some(other)) => merged.merge_fields(other, same, {at})?,\n        }}"
                            )
                            .unwrap();
                            continue;
                        } else if field.proto3_optional() || field.r#type() == Type::Message {
                            "optional".to_string()
                        } else if field.r#type() == Type::String {
                            "text".to_string()
                        } else if field.r#type() == Type::Enum {
                            "enumeration".to_string()
                        } else {
                            "plain".to_string()
                        };
                        let extra = if call == "reference" { ", same" } else { "" };
                        writeln!(
                            body,
                            "        {call}(&mut self.{field_ident}, &other.{field_ident}{extra}, {at})?;"
                        )
                        .unwrap();
                    }
                }
            }
            // prost derives `Copy` for some messages and oneofs, by their fields, so the clones of
            // those values are copies.
            code.push_str("#[allow(clippy::clone_on_copy)]\n");
            writeln!(code, "impl MergeFields for {path} {{").unwrap();
            writeln!(
                code,
                "    fn merge_fields(&mut self, other: &Self, same: &SameValue, path: &str) -> Merged {{"
            )
            .unwrap();
            if body.is_empty() {
                code.push_str("        let _ = (other, same, path);\n");
            } else if !body.contains("same") {
                code.push_str("        let _ = same;\n");
            }
            code.push_str(&body);
            code.push_str("        Ok(())\n    }\n}\n\n");
        }
        code
    }

    /// The messages that hold an enum field, directly or through their fields.
    fn enum_holders(&self) -> BTreeSet<String> {
        let mut holders = BTreeSet::new();
        loop {
            let before = holders.len();
            for (name, _, message) in &self.all {
                if (message.field.iter()).any(|field| {
                    field.r#type() == Type::Enum || holders.contains(field.type_name())
                }) {
                    holders.insert(name.clone());
                }
            }
            if holders.len() == before {
                return holders;
            }
        }
    }

    /// An impl of `VisitEnums` for each message that holds an enum field. Its method calls its
    /// function with the path of each enum field from the message, the full name of the field's
    /// enum, and the field's value.
    fn enum_visitors(&self, code: &mut String) {
        let holders = self.enum_holders();
        for (name, path, message) in &self.all {
            if !holders.contains(name) {
                continue;
            }
            let module = path.rsplit_once("::").expect("a nested path").0.to_string()
                + "::"
                + &snake(message.name());
            // The call for `item` at the path `at`: the function for an enum, and the item's own
            // walk for a message.
            let call = |field: &FieldDescriptorProto, item: &str, at: &str| match field.r#type() {
                Type::Enum => format!("f(&{at}, \"{}\", *{item})", field.type_name()),
                _ => format!("{item}.visit_enums(&{at}, f)"),
            };
            let mut body = String::new();
            let mut oneofs_done = BTreeSet::new();
            for field in &message.field {
                let relevant = |field: &FieldDescriptorProto| {
                    field.r#type() == Type::Enum || holders.contains(field.type_name())
                };
                let field_ident = ident(field.name());
                let at = format!("format!(\"{{path}}.{}\")", field.name());
                match field.oneof_index {
                    Some(oneof) if !field.proto3_optional() => {
                        if !oneofs_done.insert(oneof) {
                            continue;
                        }
                        let members: Vec<_> = (message.field.iter())
                            .filter(|member| member.oneof_index == Some(oneof) && relevant(member))
                            .collect();
                        if members.is_empty() {
                            continue;
                        }
                        let oneof_name = message.oneof_decl[oneof as usize].name();
                        let arms: Vec<(String, String)> = (members.iter())
                            .map(|member| {
                                let at = format!("format!(\"{{path}}.{}\")", member.name());
                                let pattern = format!(
                                    "Some({module}::{}::{}(item))",
                                    camel(oneof_name),
                                    camel(member.name())
                                );
                                (pattern, call(member, "item", &at))
                            })
                            .collect();
                        let oneof_field = ident(oneof_name);
                        match arms.as_slice() {
                            [(pattern, item)] => writeln!(
                                body,
                                "        if let {pattern} = &self.{oneof_field} {{ {item}; }}"
                            )
                            .unwrap(),
                            _ => {
                                writeln!(body, "        match &self.{oneof_field} {{").unwrap();
                                for (pattern, item) in &arms {
                                    writeln!(body, "            {pattern} => {item},").unwrap();
                                }
                                body.push_str("            _ => {}\n        }\n");
                            }
                        }
                    }
                    _ if !relevant(field) => {}
                    _ if field.label() == Label::Repeated => {
                        let at = format!("format!(\"{{path}}.{}[{{index}}]\")", field.name());
                        writeln!(
                            body,
                            "        for (index, item) in self.{field_ident}.iter().enumerate() {{ {}; }}",
                            call(field, "item", &at)
                        )
                        .unwrap();
                    }
                    _ if field.r#type() == Type::Enum && !field.proto3_optional() => {
                        writeln!(
                            body,
                            "        f(&{at}, \"{}\", self.{field_ident});",
                            field.type_name(),
                        )
                        .unwrap();
                    }
                    _ => {
                        writeln!(
                            body,
                            "        if let Some(item) = &self.{field_ident} {{ {}; }}",
                            call(field, "item", &at)
                        )
                        .unwrap();
                    }
                }
            }
            writeln!(code, "impl VisitEnums for {path} {{").unwrap();
            writeln!(
                code,
                "    fn visit_enums(&self, path: &str, f: &mut dyn FnMut(&str, &'static str, i32)) {{"
            )
            .unwrap();
            code.push_str(&body);
            code.push_str("    }\n}\n\n");
        }
    }

    /// An impl of `visitor` for each message that holds a `target`, with a method `{method}` that
    /// reads each `target` and a method `{method}_mut` that changes it. Each calls its function
    /// with each `target` in the order that the `.proto` files declare the fields, where a oneof
    /// takes the place of its first member.
    fn visitors(&self, code: &mut String, target: &str, visitor: &str, method: &str, every: bool) {
        let holders = self.holders(target);
        let (_, target_path, _) = self.get(target).expect("the target is a message");
        for (name, path, message) in &self.all {
            if !holders.contains(name) {
                if every && !builders::DATA.contains(&message.name()) {
                    writeln!(
                        code,
                        "impl {visitor} for {path} {{\n    fn {method}(&self, _: &mut dyn FnMut(&{target_path})) {{}}\n    fn {method}_mut(&mut self, _: &mut dyn FnMut(&mut {target_path})) {{}}\n}}\n"
                    )
                    .unwrap();
                }
                continue;
            }
            writeln!(code, "impl {visitor} for {path} {{").unwrap();
            for (suffix, reference) in [("", "&"), ("_mut", "&mut ")] {
                let visit = |item: &str, type_name: &str| match type_name == target {
                    true => format!("f({item})"),
                    false => format!("{item}.{method}{suffix}(f)"),
                };
                let module = path.rsplit_once("::").expect("a nested path").0.to_string()
                    + "::"
                    + &snake(message.name());
                let mut body = String::new();
                let mut oneofs_done = BTreeSet::new();
                for field in &message.field {
                    let relevant =
                        |type_name: &str| type_name == target || holders.contains(type_name);
                    let field_ident = ident(field.name());
                    match field.oneof_index {
                        Some(oneof) if !field.proto3_optional() => {
                            if !oneofs_done.insert(oneof) {
                                continue;
                            }
                            let members: Vec<_> = (message.field.iter())
                                .filter(|member| {
                                    member.oneof_index == Some(oneof)
                                        && relevant(member.type_name())
                                })
                                .collect();
                            if members.is_empty() {
                                continue;
                            }
                            let oneof_name = message.oneof_decl[oneof as usize].name();
                            let oneof_field = ident(oneof_name);
                            let arms: Vec<_> = (members.iter())
                                .map(|member| {
                                    let pattern = format!(
                                        "Some({module}::{}::{}(item))",
                                        camel(oneof_name),
                                        camel(member.name())
                                    );
                                    (pattern, visit("item", member.type_name()))
                                })
                                .collect();
                            match arms.as_slice() {
                                [(pattern, item)] => writeln!(
                                    body,
                                    "        if let {pattern} = {reference}self.{oneof_field} {{ {item}; }}"
                                )
                                .unwrap(),
                                _ => {
                                    writeln!(body, "        match {reference}self.{oneof_field} {{")
                                        .unwrap();
                                    for (pattern, item) in &arms {
                                        writeln!(body, "            {pattern} => {item},").unwrap();
                                    }
                                    body.push_str("            _ => {}\n        }\n");
                                }
                            }
                        }
                        _ if !relevant(field.type_name()) => {}
                        _ if field.label() == Label::Repeated => {
                            let item = visit("item", field.type_name());
                            writeln!(
                                body,
                                "        for item in {reference}self.{field_ident} {{ {item}; }}"
                            )
                            .unwrap();
                        }
                        _ => {
                            let item = visit("item", field.type_name());
                            writeln!(
                                body,
                                "        if let Some(item) = {reference}self.{field_ident} {{ {item}; }}"
                            )
                            .unwrap();
                        }
                    }
                }
                writeln!(
                    code,
                    "    fn {method}{suffix}(&{} self, f: &mut dyn FnMut({reference}{target_path})) {{",
                    if suffix.is_empty() { "" } else { "mut" }
                )
                .unwrap();
                code.push_str(&body);
                code.push_str("    }\n");
            }
            code.push_str("}\n\n");
        }
    }
}

/// The Rust identifier of a field or a oneof, as prost names it.
fn ident(name: &str) -> String {
    let name = snake(name);
    match name.as_str() {
        // The keywords that prost escapes as raw identifiers.
        "as" | "async" | "await" | "box" | "break" | "const" | "continue" | "do" | "dyn"
        | "else" | "enum" | "extern" | "false" | "final" | "fn" | "for" | "if" | "impl" | "in"
        | "let" | "loop" | "macro" | "match" | "mod" | "move" | "mut" | "pub" | "ref"
        | "return" | "static" | "struct" | "trait" | "true" | "try" | "type" | "unsafe" | "use"
        | "where" | "while" | "yield" => format!("r#{name}"),
        _ => name,
    }
}

/// `name` in snake case, as prost names modules and fields.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `name` in upper camel case, as prost names oneof enums and their variants.
fn camel(name: &str) -> String {
    name.split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}
